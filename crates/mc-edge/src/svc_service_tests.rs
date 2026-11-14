//! In-process end to end: the SVC handshake over plugin channels, an encrypted
//! UDP client (as the mod writes it), and bridging to a native listener.
use mc_core::boundary::EntityId;
use tokio::{net::UdpSocket, sync::mpsc};

use super::*;
use crate::{
    screen::Pose,
    server::{ConnectionHandles, EdgeServer},
};

fn server(root: &std::path::Path) -> Arc<EdgeServer> {
    let mut config = mc_core::CoreConfig::new(root.join("world"), 42);
    config.vanilla_data_dir =
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    config.view_distance = 0;
    config.simulation_distance = 0;
    let core = Arc::new(mc_core::Core::open(config).unwrap());
    let mut cfg = crate::config::EdgeConfig {
        server_dir: root.join("server"),
        ..Default::default()
    };
    std::fs::create_dir_all(&cfg.server_dir).unwrap();
    cfg.data.world_dir = root.join("world");
    cfg.data.vanilla_data_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    cfg.auth.online_mode = false;
    cfg.voice = VoiceConfig {
        svc: true,
        svc_port: 0,
        svc_bind_address: "127.0.0.1".into(),
        ..VoiceConfig::default()
    };
    let data = crate::server::load_vanilla_data(&cfg).unwrap();
    let tags = crate::server::load_tags(&cfg, &data).unwrap();
    Arc::new(EdgeServer::new(core, data, tags, cfg).unwrap())
}

fn connect(server: &EdgeServer) -> (ConnectionId, mpsc::Receiver<bytes::Bytes>) {
    let id = server.connections.joining_id();
    let (reliable_tx, reliable_rx) = mpsc::channel(512);
    let (world_tx, _) = crate::server::world_queue(8, crate::server::WORLD_QUEUE_BYTES);
    server
        .connections
        .register(
            id,
            ConnectionHandles {
                reliable_tx,
                world_tx,
            },
        )
        .expect("test connection admitted");
    (id, reliable_rx)
}

/// `(channel, payload)` of the next framed custom payload.
async fn next_payload(rx: &mut mpsc::Receiver<bytes::Bytes>) -> (String, Vec<u8>) {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut writer = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (reader, _) = listener.accept().await.unwrap();
    let framed = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("a payload within 5 s")
        .unwrap();
    writer.write_all(&framed).await.unwrap();
    let mut connection = crate::connection::Connection::new(reader);
    connection.set_compression(mc_protocol::frame::Compression::Threshold(256));
    let frame = connection.read_frame_play().await.unwrap();
    let mut body = &frame.body[..];
    let packet =
        <mc_protocol::packets::play::ClientboundCustomPayload as Packet>::decode(&mut body)
            .unwrap();
    match packet.payload {
        CustomPayload::Unknown { channel, payload } => (channel.as_str().to_owned(), payload),
        other => panic!("unexpected payload {other:?}"),
    }
}

struct Client {
    id: ConnectionId,
    uuid: Uuid,
    secret: Secret,
    socket: UdpSocket,
    _rx: mpsc::Receiver<bytes::Bytes>,
    handle: Connection,
}

async fn svc_client(server: &Arc<EdgeServer>, name: &str, entity: i32) -> Client {
    let (id, mut rx) = connect(server);
    let uuid = crate::crypto::offline_uuid(name);
    let handle = Connection::start(Arc::clone(server), id, EntityId(entity), uuid, name.into())
        .expect("svc enabled");
    let (channel, body) = next_payload(&mut rx).await;
    assert_eq!(channel, "minecraft:register");
    assert!(
        String::from_utf8(body)
            .unwrap()
            .contains(channel::REQUEST_SECRET)
    );
    handle.receive(
        channel::REQUEST_SECRET,
        &svc::COMPATIBILITY_VERSION.to_be_bytes(),
    );
    let (channel, _) = next_payload(&mut rx).await;
    assert_eq!(channel, channel::STATES);
    let (channel, _) = next_payload(&mut rx).await;
    assert_eq!(channel, channel::STATE);
    let (channel, secret) = next_payload(&mut rx).await;
    assert_eq!(channel, channel::SECRET);
    let port = u16::try_from(i32::from_be_bytes(secret[16..20].try_into().unwrap())).unwrap();
    assert_eq!(port, server.voice.svc.public_port());
    assert_eq!(&secret[20..36], uuid.as_bytes());
    let secret = Secret(secret[..16].try_into().unwrap());
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.connect(("127.0.0.1", port)).await.unwrap();
    let client = Client {
        id,
        uuid,
        secret,
        socket,
        _rx: rx,
        handle,
    };
    // Authenticate, then validate the connection, exactly like the mod.
    let mut auth = vec![0x5];
    auth.extend(uuid.as_bytes());
    auth.extend(client.secret.0);
    client.send(&auth).await;
    assert_eq!(client.recv().await, svc::authenticate_ack());
    client.send(&[0x9]).await;
    assert_eq!(client.recv().await, svc::connection_check_ack());
    client
}

impl Client {
    async fn send(&self, plain: &[u8]) {
        self.socket
            .send(&svc::client_datagram(self.uuid, &self.secret, plain))
            .await
            .unwrap();
    }
    async fn recv(&self) -> Vec<u8> {
        let mut buffer = [0u8; 4096];
        let len = tokio::time::timeout(Duration::from_secs(5), self.socket.recv(&mut buffer))
            .await
            .expect("a datagram within 5 s")
            .unwrap();
        let bytes = &buffer[..len];
        assert_eq!(bytes[0], svc::MAGIC);
        let mut reader = svc::Reader::new(&bytes[1..]);
        let sealed = reader.byte_array(svc::MAX_DATAGRAM_PAYLOAD).unwrap();
        self.secret
            .decrypt(sealed)
            .expect("authentic server datagram")
    }
    async fn recv_audio(&self) -> Vec<u8> {
        loop {
            let plain = self.recv().await;
            if plain[0] == 0x2 {
                return plain;
            }
        }
    }
    fn mic(data: &[u8], sequence: i64) -> Vec<u8> {
        let mut out = vec![0x1, data.len() as u8];
        out.extend(data);
        out.extend(sequence.to_be_bytes());
        out.push(0);
        out
    }
}

fn place(server: &EdgeServer, poses: &[(ConnectionId, f64)]) {
    server.voice.relay.lock().set_poses(
        poses
            .iter()
            .map(|(id, x)| (*id, Pose::from_feet(0, [*x, 64.0, 0.0], 0.0, 0.0)))
            .collect(),
    );
}

#[tokio::test]
async fn svc_players_and_native_players_hear_each_other_through_the_bridge() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let _udp = spawn(Arc::clone(&server)).await.expect("udp socket");

    let alice = svc_client(&server, "Alice", 1).await;
    let bob = svc_client(&server, "Bob", 2).await;

    // A native client Carol, listening.
    let (carol, mut carol_rx) = connect(&server);
    let carol_uuid = crate::crypto::offline_uuid("Carol");
    let carol_handle = crate::voice_service::Connection::start(
        Arc::clone(&server),
        carol,
        EntityId(3),
        carol_uuid,
        "Carol".into(),
    );
    let (channel, state) = next_payload(&mut carol_rx).await;
    assert_eq!(channel, "krasnokamen:voice");
    assert_eq!(state, vec![1, 1, 1, 0, 48]);
    carol_handle.receive(&[1, 1, 1]);

    // Bob learned about Carol and Alice's UDP validation: states arrive as plugin messages.
    place(&server, &[(alice.id, 0.0), (bob.id, 10.0), (carol, 20.0)]);

    // SVC -> SVC and SVC -> native, same Opus bytes.
    alice.send(&Client::mic(&[9, 8, 7], 41)).await;
    let sound = bob.recv_audio().await;
    assert_eq!(&sound[1..17], alice.uuid.as_bytes(), "channel id");
    assert_eq!(&sound[17..33], alice.uuid.as_bytes(), "sender");
    assert_eq!(&sound[33..37], &[3, 9, 8, 7]);
    assert_eq!(i64::from_be_bytes(sound[37..45].try_into().unwrap()), 41);
    assert_eq!(f32::from_be_bytes(sound[45..49].try_into().unwrap()), 48.0);
    let (channel, audio) = loop {
        let (channel, payload) = next_payload(&mut carol_rx).await;
        if channel == "krasnokamen:voice" {
            break (channel, payload);
        }
    };
    assert_eq!(channel, "krasnokamen:voice");
    assert_eq!(&audio[..2], &[1, 0]);
    assert_eq!(&audio[2..18], alice.uuid.as_bytes());
    assert_eq!(u32::from_be_bytes(audio[30..34].try_into().unwrap()), 41);
    assert_eq!(&audio[34..], &[9, 8, 7]);

    // Native -> SVC: Carol's packet reaches Alice (20 away) and Bob (10 away).
    let mut native = vec![1, 0, 0, 0, 0, 5];
    native.extend([4, 5, 6]);
    carol_handle.receive(&native);
    for listener in [&alice, &bob] {
        let sound = listener.recv_audio().await;
        assert_eq!(&sound[17..33], carol_uuid.as_bytes());
        assert_eq!(&sound[33..37], &[3, 4, 5, 6]);
        assert_eq!(i64::from_be_bytes(sound[37..45].try_into().unwrap()), 5);
    }

    // Silence from the native speaker ends the burst for SVC listeners.
    let ended = server
        .voice
        .relay
        .lock()
        .flush_idle(Instant::now() + Duration::from_millis(300));
    assert_eq!(ended.len(), 1);
    crate::voice_service::dispatch(&server, &ended[0], &[]);
    let end = bob.recv_audio().await;
    assert_eq!(&end[33..34], &[0], "empty opus = end of speech");
    assert_eq!(i64::from_be_bytes(end[34..42].try_into().unwrap()), 6);

    // Out of range: nothing for a far listener, and a forged datagram is dropped.
    place(&server, &[(alice.id, 0.0), (bob.id, 500.0), (carol, 20.0)]);
    alice.send(&Client::mic(&[1], 42)).await;
    let far = tokio::time::timeout(Duration::from_millis(300), bob.recv_audio()).await;
    assert!(far.is_err(), "no audio beyond 48 blocks");
    let forged = svc::client_datagram(alice.uuid, &Secret([7; 16]), &Client::mic(&[1], 43));
    alice.socket.send(&forged).await.unwrap();
    place(&server, &[(alice.id, 0.0), (bob.id, 10.0), (carol, 20.0)]);
    alice.send(&Client::mic(&[2], 44)).await;
    let sound = bob.recv_audio().await;
    assert_eq!(
        i64::from_be_bytes(sound[35..43].try_into().unwrap()),
        44,
        "forged packet 43 was not relayed"
    );

    // Disabled SVC clients hear nothing; a leaving one is forgotten.
    bob.handle.receive(channel::UPDATE_STATE, &[1]);
    alice.send(&Client::mic(&[3], 45)).await;
    let muted = tokio::time::timeout(Duration::from_millis(300), bob.recv_audio()).await;
    assert!(muted.is_err(), "disabled client is not sent audio");
    let bob_id = bob.id;
    drop(bob);
    assert!(!server.voice.relay.lock().is_svc(bob_id));
}

#[tokio::test]
async fn svc_group_audio_stays_inside_the_password_protected_group() {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let _udp = spawn(Arc::clone(&server)).await.expect("udp socket");
    let alice = svc_client(&server, "Alice", 1).await;
    let bob = svc_client(&server, "Bob", 2).await;
    let (carol, mut carol_rx) = connect(&server);
    let carol_handle = crate::voice_service::Connection::start(
        Arc::clone(&server),
        carol,
        EntityId(3),
        crate::crypto::offline_uuid("Carol"),
        "Carol".into(),
    );
    next_payload(&mut carol_rx).await;
    carol_handle.receive(&[1, 1, 1]);
    place(&server, &[(alice.id, 0.0), (bob.id, 500.0), (carol, 1.0)]);

    alice
        .handle
        .receive(channel::CREATE_GROUP, b"\x04test\x01\x06secret\x00\x00");
    let group = server
        .voice
        .relay
        .lock()
        .state(alice.id)
        .unwrap()
        .group
        .unwrap();
    let mut join = group.as_bytes().to_vec();
    join.extend(b"\x01\x05wrong");
    bob.handle.receive(channel::SET_GROUP, &join);
    assert_eq!(server.voice.relay.lock().state(bob.id).unwrap().group, None);
    join.truncate(16);
    join.extend(b"\x01\x06secret");
    bob.handle.receive(channel::SET_GROUP, &join);
    assert_eq!(
        server.voice.relay.lock().state(bob.id).unwrap().group,
        Some(group)
    );
    for (speaker, listener) in [(&alice, &bob), (&bob, &alice)] {
        speaker.send(&Client::mic(&[9, 8, 7], 41)).await;
        let sound = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let sound = listener.recv().await;
                if sound[0] == 3 {
                    break sound;
                }
            }
        })
        .await
        .expect("group audio within 10 s");
        assert_eq!(&sound[17..33], speaker.uuid.as_bytes());
        assert_eq!(&sound[33..37], &[3, 9, 8, 7]);
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(300), carol_rx.recv())
            .await
            .is_err(),
        "nearby non-group native listener must not receive private audio"
    );
}
