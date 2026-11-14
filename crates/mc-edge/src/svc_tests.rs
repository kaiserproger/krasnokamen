//! Vectors were produced by a JDK program that mirrors SVC 26.1 `Secret`
//! (`AES/GCM/NoPadding`, 12-byte IV, 128-bit tag) and the `toBytes` methods of
//! `MicPacket`, `PlayerSoundPacket`, `AuthenticatePacket`, `SecretPacket`,
//! `PlayerState` over a `FriendlyByteBuf`-compatible writer.
use super::*;

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
fn secret() -> Secret {
    Secret(std::array::from_fn(|i| i as u8))
}
fn iv() -> [u8; 12] {
    std::array::from_fn(|i| 0xa0 + i as u8)
}
fn player() -> Uuid {
    Uuid::from_u128(0x0102030405060708_090a0b0c0d0e0f10)
}

const MIC_PLAIN: &str = "01050102030405112233445566778801";
const MIC_DATAGRAM: &str = "ff0102030405060708090a0b0c0d0e0f102ca0a1a2a3a4a5a6a7a8a9aaabab8339b97d8d361ba84bf155206538610d418df85f4e0240e655f4ef66eb078f";
const SOUND_PLAIN: &str = "020102030405060708090a0b0c0d0e0f100102030405060708090a0b0c0d0e0f10050102030405000000000000004d4240000001";
const SOUND_DATAGRAM: &str = "ff50a0a1a2a3a4a5a6a7a8a9aaaba8873ab87a8c350d8271bf0b4a1fbe6f43ef210b87123ba82d197b904e8081a78aa644330ceee96d7fdee2c4afdb4492fc544c4874f45fec266b8abdd551b384605e40dc";
const AUTH_PLAIN: &str = "050102030405060708090a0b0c0d0e0f10000102030405060708090a0b0c0d0e0f";
const SECRET_PACKET: &str = "000102030405060708090a0b0c0d0e0f00005f860102030405060708090a0b0c0d0e0f1000000004fb4048000000000000000003e8010000";
const STATE: &str = "00010102030405060708090a0b0c0d0e0f1005537465766500";

#[test]
fn decrypts_a_sdk_mic_datagram() {
    let bytes = unhex(MIC_DATAGRAM);
    let datagram = parse_datagram(&bytes).unwrap();
    assert_eq!(datagram.player, player());
    let plain = secret().decrypt(datagram.payload).unwrap();
    assert_eq!(plain, unhex(MIC_PLAIN));
    assert_eq!(
        parse_udp(&plain),
        Some(Udp::Mic {
            data: &[1, 2, 3, 4, 5],
            sequence: 0x1122334455667788,
            whispering: true,
        })
    );
}

#[test]
fn encryption_matches_the_sdk_byte_for_byte() {
    let sealed = secret().encrypt_with_iv(&iv(), &unhex(MIC_PLAIN)).unwrap();
    let datagram = parse_datagram(&unhex(MIC_DATAGRAM))
        .unwrap()
        .payload
        .to_vec();
    assert_eq!(sealed, datagram);
}

#[test]
fn player_sound_datagram_matches_the_sdk() {
    let plain = player_sound(player(), &[1, 2, 3, 4, 5], 77, true, 48.0);
    assert_eq!(plain, unhex(SOUND_PLAIN));
    let sealed = secret().encrypt_with_iv(&iv(), &plain).unwrap();
    let mut datagram = vec![MAGIC];
    put_byte_array(&mut datagram, &sealed);
    assert_eq!(datagram, unhex(SOUND_DATAGRAM));
}

#[test]
fn seal_datagram_round_trips_through_the_client_view() {
    let wire = seal_datagram(&secret(), &keep_alive()).unwrap();
    // Server datagrams carry no player id: `0xFF, VarInt len, iv||ct||tag`.
    let mut reader = Reader(&wire[1..]);
    assert_eq!(wire[0], MAGIC);
    let sealed = reader.byte_array(MAX_DATAGRAM_PAYLOAD).unwrap();
    assert!(reader.is_empty());
    assert_eq!(secret().decrypt(sealed).unwrap(), keep_alive());
    // Random IV: two seals of one message differ.
    assert_ne!(wire, seal_datagram(&secret(), &keep_alive()).unwrap());
}

#[test]
fn wrong_secret_or_tampering_does_not_decrypt() {
    let bytes = unhex(MIC_DATAGRAM);
    let payload = parse_datagram(&bytes).unwrap().payload;
    assert!(Secret([9; 16]).decrypt(payload).is_none());
    let mut tampered = payload.to_vec();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert!(secret().decrypt(&tampered).is_none());
    assert!(secret().decrypt(&payload[..8]).is_none());
}

#[test]
fn authenticate_and_control_messages_parse() {
    assert_eq!(
        parse_udp(&unhex(AUTH_PLAIN)),
        Some(Udp::Authenticate {
            player: player(),
            secret: secret(),
        })
    );
    assert_eq!(parse_udp(&[0x9]), Some(Udp::ConnectionCheck));
    assert_eq!(parse_udp(&[0x8]), Some(Udp::KeepAlive));
    assert_eq!(parse_udp(&[0x7, 0]), Some(Udp::Ignored));
    assert_eq!(parse_udp(&[0x6]), Some(Udp::Ignored));
    assert_eq!(parse_udp(&[0x63]), None);
    assert_eq!(parse_udp(&[]), None);
    assert_eq!(authenticate_ack(), vec![0x6]);
    assert_eq!(connection_check_ack(), vec![0xA]);
}

#[test]
fn mic_packet_limits() {
    // Empty data is the end-of-speech marker and is valid.
    let mut end = vec![0x1, 0];
    end.extend(5i64.to_be_bytes());
    end.push(0);
    assert_eq!(
        parse_udp(&end),
        Some(Udp::Mic {
            data: &[],
            sequence: 5,
            whispering: false
        })
    );
    let mut long = vec![0x1];
    put_var_int(&mut long, MAX_OPUS_PAYLOAD as u32 + 1);
    long.extend(vec![0; MAX_OPUS_PAYLOAD + 1]);
    long.extend([0; 9]);
    assert_eq!(parse_udp(&long), None);
    // Truncated and bad boolean.
    assert_eq!(parse_udp(&unhex(MIC_PLAIN)[..10]), None);
    let mut bad = unhex(MIC_PLAIN);
    *bad.last_mut().unwrap() = 2;
    assert_eq!(parse_udp(&bad), None);
}

#[test]
fn datagram_framing_rejects_garbage() {
    assert_eq!(parse_datagram(&[]), None);
    assert_eq!(parse_datagram(&[0x00; 40]), None, "magic");
    let mut short = vec![MAGIC];
    short.extend(player().as_bytes());
    assert_eq!(parse_datagram(&short), None, "no length");
    short.push(5);
    short.extend([0; 3]);
    assert_eq!(parse_datagram(&short), None, "truncated payload");
    let mut huge = vec![MAGIC];
    huge.extend(player().as_bytes());
    put_var_int(&mut huge, MAX_DATAGRAM_PAYLOAD as u32 + 1);
    huge.extend(vec![0; MAX_DATAGRAM_PAYLOAD + 1]);
    assert_eq!(parse_datagram(&huge), None, "oversized");
}

#[test]
fn unauthenticated_ping_is_echoed_raw() {
    let mut wire = vec![MAGIC];
    wire.extend(PING_V1.as_bytes());
    let body: Vec<u8> = (0..24).collect();
    put_byte_array(&mut wire, &body);
    let datagram = parse_datagram(&wire).unwrap();
    assert_eq!(datagram.player, PING_V1);
    assert_eq!(ping_reply(datagram.payload), Some(body));
    assert_eq!(ping_reply(&[1, 2]), None);
}

#[test]
fn plugin_channel_handshake_bodies() {
    assert_eq!(
        parse_request_secret(&[0, 0, 0, 20]),
        Some(COMPATIBILITY_VERSION)
    );
    assert_eq!(parse_request_secret(&[0, 0, 0, 20, 0]), None);
    assert_eq!(parse_request_secret(&[0, 0]), None);
    assert_eq!(parse_update_state(&[1]), Some(true));
    assert_eq!(parse_update_state(&[0]), Some(false));
    assert_eq!(parse_update_state(&[2]), None);
    let packet = secret_packet(&SecretInfo {
        secret: &secret(),
        port: 24454,
        player: player(),
        distance: 48.0,
        voice_host: "",
    });
    assert_eq!(packet, unhex(SECRET_PACKET));
    let named = secret_packet(&SecretInfo {
        secret: &secret(),
        port: 24454,
        player: player(),
        distance: 48.0,
        voice_host: "voice.example",
    });
    assert_eq!(named.len(), packet.len() + "voice.example".len());
}

#[test]
fn state_packets_match_the_sdk() {
    let state = State {
        uuid: player(),
        name: "Steve".into(),
        disabled: false,
        disconnected: true,
        group: None,
    };
    assert_eq!(state_packet(&state), unhex(STATE));
    let list = states_packet(&[state.clone(), state]);
    assert_eq!(&list[..4], &[0, 0, 0, 2]);
    assert_eq!(&list[4..4 + STATE.len() / 2], &unhex(STATE)[..]);
    assert_eq!(states_packet(&[]), vec![0, 0, 0, 0]);
    assert_eq!(remove_state_packet(player()), player().as_bytes().to_vec());
}

#[test]
fn register_body_lists_serverbound_channels() {
    let body = String::from_utf8(register_body()).unwrap();
    let channels: Vec<&str> = body.split('\0').collect();
    assert!(channels.contains(&channel::REQUEST_SECRET));
    assert!(channels.contains(&channel::UPDATE_STATE));
    assert!(channels.iter().all(|c| c.starts_with("voicechat:")));
}

#[test]
fn secrets_are_random() {
    assert_ne!(Secret::random().unwrap(), Secret::random().unwrap());
}

#[test]
fn var_int_round_trips() {
    for value in [0u32, 1, 127, 128, 300, 2048, 1_000_000, u32::MAX] {
        let mut out = Vec::new();
        put_var_int(&mut out, value);
        let mut reader = Reader(&out);
        assert_eq!(reader.var_int(), Some(value));
        assert!(reader.is_empty());
    }
    assert_eq!(Reader(&[0x80; 6]).var_int(), None);
}

#[test]
fn group_packets_match_release_layout_and_reject_invalid_inputs() {
    let group = parse_create_group(b"\x04test\x01\x06secret\x00\x02").unwrap();
    assert_eq!(group.name, "test");
    assert_eq!(group.password.as_deref(), Some("secret"));
    assert_eq!(group.kind, 2);
    let mut joined = group.id.as_bytes().to_vec();
    joined.extend(b"\x01\x06secret");
    assert_eq!(
        parse_join_group(&joined),
        Some((group.id, Some("secret".into())))
    );
    let mut expected = group.id.as_bytes().to_vec();
    expected.extend(b"\x04test\x01\x00\x00\x00\x02");
    assert_eq!(add_group(&group), expected);
    assert_eq!(joined_group(None, true), vec![0, 1]);
    assert!(parse_create_group(b"\x01 \x00\x00\x00").is_none());
    assert!(parse_create_group(b"\x04test\x01\x00\x00\x00").is_none());
    joined.push(0);
    assert!(parse_join_group(&joined).is_none());
    let sound = group_sound(player(), &[7], 9);
    assert_eq!(sound[0], 3);
    assert_eq!(&sound[1..17], player().as_bytes());
    assert_eq!(&sound[17..33], player().as_bytes());
    assert_eq!(&sound[33..], &[1, 7, 0, 0, 0, 0, 0, 0, 0, 9, 0]);
}
