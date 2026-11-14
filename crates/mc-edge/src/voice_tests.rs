use std::time::Duration;

use super::*;

const A: ConnectionId = ConnectionId(1);
const B: ConnectionId = ConnectionId(2);
const C: ConnectionId = ConnectionId(3);

fn pose(dimension: u8, x: f64) -> Pose {
    Pose::from_feet(dimension, [x, 64.0, 0.0], 0.0, 0.0)
}

/// A, B, C registered and listening at the given x offsets in dimension 0.
fn relay(xs: [f64; 3]) -> (Relay, Instant) {
    let now = Instant::now();
    let mut relay = Relay::new(&VoiceConfig::default());
    for (id, x) in [A, B, C].into_iter().zip(xs) {
        relay.register(
            id,
            EntityId(id.0 as i32),
            Uuid::from_u128(id.0.into()),
            format!("p{}", id.0),
            now,
        );
        relay.handle(id, Message::Listen(true), now);
        relay.poses.insert(id, pose(0, x));
    }
    (relay, now)
}
fn say(relay: &mut Relay, from: ConnectionId, now: Instant) -> Option<Delivery> {
    relay.handle(
        from,
        Message::Audio {
            seq: 7,
            opus: &[1, 2, 3],
        },
        now,
    )
}
fn sorted(delivery: Delivery) -> Vec<u64> {
    let mut to: Vec<u64> = delivery.native.iter().map(|id| id.0).collect();
    to.sort();
    to
}

#[test]
fn relays_only_within_range_and_never_to_the_speaker() {
    let (mut relay, now) = relay([0.0, 48.0, 48.5]);
    let delivery = say(&mut relay, A, now).unwrap();
    assert_eq!(sorted(delivery), vec![2]);
}

#[test]
fn other_dimension_is_not_relayed() {
    let (mut relay, now) = relay([0.0, 1.0, 2.0]);
    relay.poses.insert(B, pose(1, 1.0));
    assert_eq!(sorted(say(&mut relay, A, now).unwrap()), vec![3]);
}

#[test]
fn only_listeners_with_a_pose_hear() {
    let (mut relay, now) = relay([0.0, 1.0, 2.0]);
    relay.handle(B, Message::Listen(false), now);
    relay.poses.remove(&C);
    assert!(say(&mut relay, A, now).is_none());
    relay.handle(B, Message::Listen(true), now);
    assert_eq!(sorted(say(&mut relay, A, now).unwrap()), vec![2]);
}

#[test]
fn speaker_without_pose_or_not_registered_is_dropped() {
    let (mut relay, now) = relay([0.0, 1.0, 2.0]);
    relay.poses.remove(&A);
    assert!(say(&mut relay, A, now).is_none());
    assert!(say(&mut relay, ConnectionId(9), now).is_none());
}

#[test]
fn audio_payload_carries_speaker_eye_and_seq_verbatim() {
    let (mut relay, now) = relay([10.0, 11.0, 200.0]);
    let delivery = say(&mut relay, A, now).unwrap();
    let payload = audio(
        delivery.speaker,
        delivery.eye,
        delivery.sequence as u32,
        &[1, 2, 3],
    );
    assert_eq!(&payload[..2], &[1, 0]);
    assert_eq!(&payload[2..18], Uuid::from_u128(1).as_bytes());
    assert_eq!(
        f32::from_be_bytes(payload[18..22].try_into().unwrap()),
        10.0
    );
    assert_eq!(
        f32::from_be_bytes(payload[22..26].try_into().unwrap()),
        65.62
    );
    assert_eq!(f32::from_be_bytes(payload[26..30].try_into().unwrap()), 0.0);
    assert_eq!(u32::from_be_bytes(payload[30..34].try_into().unwrap()), 7);
    assert_eq!(&payload[34..], &[1, 2, 3]);
}

#[test]
fn packet_rate_limit_drops_overload_then_refills() {
    let (mut relay, now) = relay([0.0, 1.0, 2.0]);
    let sent = (0..40)
        .filter(|_| say(&mut relay, A, now).is_some())
        .count();
    assert_eq!(sent, 20);
    assert!(say(&mut relay, A, now + Duration::from_millis(100)).is_some());
}

#[test]
fn byte_rate_limit_drops_overload() {
    let (mut relay, now) = relay([0.0, 1.0, 2.0]);
    let big = [0u8; MAX_OPUS_BYTES];
    let sent = (0..20)
        .filter(|_| {
            relay
                .handle(A, Message::Audio { seq: 0, opus: &big }, now)
                .is_some()
        })
        .count();
    assert_eq!(sent, 16);
    // A different speaker has its own budget.
    assert!(
        relay
            .handle(B, Message::Audio { seq: 0, opus: &big }, now)
            .is_some()
    );
}

#[test]
fn disabled_relays_nothing_and_reports_state() {
    let now = Instant::now();
    let mut relay = Relay::new(&VoiceConfig {
        enabled: false,
        ..VoiceConfig::default()
    });
    let state = relay.register(A, EntityId(1), Uuid::nil(), "a".into(), now);
    assert_eq!(&state[..], &[1, 1, 0, 0, 48]);
    relay.register(B, EntityId(2), Uuid::nil(), "b".into(), now);
    relay.handle(B, Message::Listen(true), now);
    relay.poses.insert(A, pose(0, 0.0));
    relay.poses.insert(B, pose(0, 1.0));
    assert!(say(&mut relay, A, now).is_none());
}

#[test]
fn unregistered_connection_stops_hearing() {
    let (mut relay, now) = relay([0.0, 1.0, 2.0]);
    relay.unregister(B);
    assert_eq!(sorted(say(&mut relay, A, now).unwrap()), vec![3]);
}

#[test]
fn decode_validates_shape() {
    assert_eq!(
        decode(&[1, 0, 0, 0, 0, 9, 5, 6]),
        Some(Message::Audio {
            seq: 9,
            opus: &[5, 6]
        })
    );
    assert_eq!(decode(&[1, 1, 1]), Some(Message::Listen(true)));
    assert_eq!(decode(&[1, 1, 0]), Some(Message::Listen(false)));
    assert_eq!(decode(&[1, 1, 2]), None);
    assert_eq!(decode(&[2, 0, 0, 0, 0, 9, 5]), None, "version");
    assert_eq!(decode(&[1, 0, 0, 0, 0, 9]), None, "empty opus");
    let mut long = vec![1, 0, 0, 0, 0, 9];
    long.extend([0; MAX_OPUS_BYTES + 1]);
    assert_eq!(decode(&long), None, "oversized opus");
    assert_eq!(decode(&[1, 9]), None, "unknown kind");
    assert_eq!(decode(&[1]), None);
}

#[test]
fn default_config_parses_and_validates() {
    let config: VoiceConfig = toml::from_str("").unwrap();
    assert!(config.enabled);
    assert_eq!(config.range_blocks, 48);
    assert!(!config.svc);
    assert!(
        VoiceConfig {
            enabled: false,
            ..VoiceConfig::default()
        }
        .validate()
        .is_ok()
    );
    assert_eq!(config.svc_port, 24454);
    let bad_range = VoiceConfig {
        range_blocks: 0,
        ..VoiceConfig::default()
    };
    assert!(bad_range.validate().is_err());
    let svc = VoiceConfig {
        svc: true,
        ..VoiceConfig::default()
    };
    assert!(svc.validate().is_ok());
    let bad_bind = VoiceConfig {
        svc_bind_address: "not an ip".into(),
        ..svc.clone()
    };
    assert!(bad_bind.validate().is_err());
    let disabled = VoiceConfig {
        enabled: false,
        ..svc
    };
    assert!(disabled.validate().is_err());
}

fn with_svc(xs: [f64; 3]) -> (Relay, Instant) {
    // A native, B and C Simple Voice Chat; all listening.
    let now = Instant::now();
    let mut relay = Relay::new(&VoiceConfig::default());
    relay.register(A, EntityId(1), Uuid::from_u128(1), "a".into(), now);
    relay.handle(A, Message::Listen(true), now);
    for id in [B, C] {
        relay.register_svc(
            id,
            EntityId(id.0 as i32),
            Uuid::from_u128(id.0.into()),
            format!("p{}", id.0),
            now,
        );
        relay.set_svc_connected(id, true);
    }
    for (id, x) in [A, B, C].into_iter().zip(xs) {
        relay.poses.insert(id, pose(0, x));
    }
    (relay, now)
}

#[test]
fn native_speech_reaches_svc_listeners_in_range_with_the_native_sequence() {
    let (mut relay, now) = with_svc([0.0, 10.0, 100.0]);
    let delivery = say(&mut relay, A, now).unwrap();
    assert!(delivery.native.is_empty());
    assert_eq!(delivery.svc, vec![B]);
    assert_eq!(delivery.sequence, 7);
    assert_eq!(delivery.speaker, Uuid::from_u128(1));
    assert!(!delivery.whispering);
    assert_eq!(delivery.range, 48.0);
}

#[test]
fn svc_speech_reaches_native_and_svc_listeners() {
    let (mut relay, now) = with_svc([0.0, 10.0, 20.0]);
    let delivery = relay.audio(B, 9, &[1, 2], false, now).unwrap();
    assert_eq!(delivery.native, vec![A]);
    assert_eq!(delivery.svc, vec![C]);
    assert_eq!(delivery.speaker, Uuid::from_u128(2));
}

#[test]
fn svc_end_of_speech_goes_only_to_svc_listeners() {
    let (mut relay, now) = with_svc([0.0, 10.0, 20.0]);
    let delivery = relay.audio(B, 10, &[], false, now).unwrap();
    assert!(delivery.native.is_empty());
    assert_eq!(delivery.svc, vec![C]);
}

#[test]
fn whispering_halves_the_range() {
    let (mut relay, now) = with_svc([0.0, 10.0, 40.0]);
    let delivery = relay.audio(B, 1, &[1], true, now).unwrap();
    assert!(delivery.whispering);
    assert_eq!(delivery.range, 24.0);
    assert_eq!(
        delivery.native,
        vec![A],
        "10 blocks away: inside the whisper range"
    );
    assert!(
        delivery.svc.is_empty(),
        "C is 30 blocks away: inside the normal range, outside the whisper range"
    );
}

#[test]
fn disabled_or_unconnected_svc_players_hear_nothing() {
    let (mut relay, now) = with_svc([0.0, 10.0, 20.0]);
    relay.set_svc_disabled(B, true);
    relay.set_svc_connected(C, false);
    assert!(say(&mut relay, A, now).is_none());
    relay.set_svc_disabled(B, false);
    assert_eq!(say(&mut relay, A, now).unwrap().svc, vec![B]);
}

#[test]
fn native_burst_ends_after_a_gap_for_svc_listeners() {
    let (mut relay, now) = with_svc([0.0, 10.0, 20.0]);
    say(&mut relay, A, now).unwrap();
    assert!(
        relay
            .flush_idle(now + Duration::from_millis(100))
            .is_empty()
    );
    let ended = relay.flush_idle(now + Duration::from_millis(200));
    assert_eq!(ended.len(), 1);
    assert!(ended[0].native.is_empty());
    assert_eq!(ended[0].svc.len(), 2);
    assert_eq!(ended[0].sequence, 8, "end marker follows the last sequence");
    assert_eq!(ended[0].speaker, Uuid::from_u128(1));
    assert!(
        relay.flush_idle(now + Duration::from_secs(5)).is_empty(),
        "ends once"
    );
}

#[test]
fn svc_speakers_are_not_flushed_and_states_reflect_connection() {
    let (mut relay, now) = with_svc([0.0, 10.0, 20.0]);
    relay.audio(B, 1, &[1], false, now).unwrap();
    assert!(relay.flush_idle(now + Duration::from_secs(1)).is_empty());
    relay.set_svc_connected(C, false);
    relay.set_svc_disabled(C, true);
    let state = relay.state(C).unwrap();
    assert!(state.disabled && state.disconnected);
    assert_eq!(state.name, "p3");
    let native = relay.state(A).unwrap();
    assert!(!native.disabled && !native.disconnected);
    assert_eq!(relay.states().len(), 3);
    assert!(relay.is_svc(B) && !relay.is_svc(A));
    assert_eq!(relay.unregister(B), Some(Uuid::from_u128(2)));
    assert_eq!(relay.svc_connections(), vec![C]);
}

#[test]
fn svc_speaker_shares_the_rate_limit() {
    let (mut relay, now) = with_svc([0.0, 10.0, 20.0]);
    let sent = (0..40)
        .filter(|seq| relay.audio(B, *seq, &[1], false, now).is_some())
        .count();
    assert_eq!(sent, 20);
}

#[test]
fn svc_groups_route_privately_and_reap_after_last_member() {
    let (mut relay, now) = with_svc([0.0, 1.0, 500.0]);
    let id = Uuid::new_v4();
    assert!(relay.create_group(
        B,
        crate::svc::Group {
            id,
            name: "private".into(),
            password: Some("secret".into()),
            kind: 0,
        }
    ));
    assert_eq!(relay.join_group(C, id, Some("wrong")), Some(false));
    assert!(relay.audio(B, 1, &[1], false, now).is_none());
    assert_eq!(relay.join_group(C, id, Some("secret")), Some(true));
    let delivery = relay.audio(B, 2, &[1], false, now).unwrap();
    assert_eq!(delivery.group_svc, vec![C]);
    assert!(delivery.native.is_empty() && delivery.svc.is_empty());
    let reverse = relay.audio(C, 3, &[1], false, now).unwrap();
    assert_eq!(reverse.group_svc, vec![B]);
    assert!(reverse.native.is_empty() && reverse.svc.is_empty());
    assert_eq!(relay.state(B).unwrap().group, Some(id));
    assert!(relay.leave_group(B));
    assert!(relay.reap_groups().is_empty());
    relay.unregister(C);
    assert_eq!(relay.reap_groups(), vec![id]);
}

#[test]
fn svc_group_kinds_enforce_open_and_isolated_proximity() {
    let (mut relay, now) = with_svc([0.0, 1.0, 2.0]);
    let id = Uuid::new_v4();
    assert!(relay.create_group(
        B,
        crate::svc::Group {
            id,
            name: "open".into(),
            password: None,
            kind: 1,
        }
    ));
    let delivery = relay.audio(B, 1, &[1], false, now).unwrap();
    assert_eq!(delivery.native, vec![A]);
    assert_eq!(delivery.svc, vec![C]);
    relay.groups.get_mut(&id).unwrap().kind = 2;
    assert_eq!(say(&mut relay, A, now).unwrap().svc, vec![C]);
    assert!(relay.audio(B, 2, &[1], false, now).is_none());
}
