//! `[network] chunks_per_tick_per_player` paces a real Play connection
//! through the listener, connection pipeline, pump and the client's ACKs.

use std::time::Duration;

use super::*;
use mc_protocol::packets::Packet;
use mc_protocol::packets::play::{
    ClientboundChunkBatchFinished, ClientboundChunkBatchStart, LevelChunkWithLight, LoginPlay,
};
use mc_vanilla::wire_client::Client;

const WAIT: Duration = Duration::from_secs(120);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configured_chunk_rate_caps_every_batch_on_a_real_connection() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(super::leave_tests::server_with(dir.path(), |cfg| {
        cfg.network.chunks_per_tick_per_player = 3;
    }));
    let mut ticks = server.hub.subscribe();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let pump = crate::server::spawn_pump(Arc::clone(&server)).unwrap();
    let accepted = tokio::spawn({
        let server = Arc::clone(&server);
        async move {
            let (socket, _) = listener.accept().await.unwrap();
            let slot = server.pre_play.try_acquire().expect("pre-play slot");
            let _ = handle_connection(socket, server, slot).await;
        }
    });
    let mut client = Client::connect(address).await.unwrap();
    client.drive_login(address, "Paced").await.unwrap();
    client.drive_configuration().await.unwrap();
    client
        .wait_for_frame_id_with_timeout(LoginPlay::ID, WAIT)
        .await
        .expect("the connection reaches Play");
    // The client acknowledges a batch only when it reads its end, so nothing
    // after the first batch is sent until the world holds many ready chunks:
    // from then on the batch size is set by the pacing, not by generation.
    super::leave_tests::wait_snapshot(&mut ticks, WAIT, |snapshot| snapshot.chunks.len() >= 49)
        .await;
    // Every ACK asks for 8 chunks per tick, so a cap of 3 is what binds.
    let mut batches = Vec::new();
    let mut open: Option<usize> = None;
    while batches.len() < 4 {
        let frame = client.read_frame_with_timeout(WAIT).await.unwrap();
        if frame.id == ClientboundChunkBatchStart::ID {
            open = Some(0);
        } else if frame.id == LevelChunkWithLight::ID {
            if let Some(count) = open.as_mut() {
                *count += 1;
            }
        } else if frame.id == ClientboundChunkBatchFinished::ID {
            batches.push(open.take().expect("a batch finishes after it starts"));
        }
    }
    assert!(batches.iter().all(|&count| count <= 3), "{batches:?}");
    assert_eq!(&batches[1..], [3, 3, 3], "{batches:?}");
    drop(client);
    server.hub.shutdown();
    accepted.abort();
    pump.abort();
}
