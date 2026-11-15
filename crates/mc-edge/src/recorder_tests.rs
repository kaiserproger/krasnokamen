use std::sync::Arc;

use mc_core::boundary::{ChangeSet, ConnectionId, EntityId, TickOutput, TickTimes, WorldSnapshot};
use mc_protocol::frame::{Compression, try_decode_frame};
use mc_protocol::packets::{CustomPayload, Packet, play::ClientboundCustomPayload};

use super::*;
use crate::fanout::{AddressedPacket, Priority, publish};
use crate::plan::ViewerState;

fn output(tick: u64, wall: u64) -> TickOutput {
    TickOutput {
        tick,
        snapshot: Arc::new(WorldSnapshot {
            tick,
            ..WorldSnapshot::default()
        }),
        changes: Arc::new(ChangeSet::default()),
        times: TickTimes {
            wall_start_unix_ns: wall,
            ..TickTimes::default()
        },
    }
}

fn viewer(connection: u64, recorder: bool) -> ViewerState {
    let mut viewer = ViewerState::new(
        ConnectionId(connection),
        EntityId(connection as i32),
        [8.0, 64.0, 8.0],
    );
    viewer.recorder = recorder;
    viewer
}

/// `(tick, wall_ns)` of a `krasnokamen:tick` frame, `None` for any other frame.
fn marker(frame: &bytes::Bytes) -> Option<(u64, u64)> {
    let mut bytes = bytes::BytesMut::from(&frame[..]);
    let mut frame = try_decode_frame(&mut bytes, Compression::Threshold(256))
        .unwrap()
        .unwrap();
    assert!(bytes.is_empty(), "one frame per marker item");
    if frame.id != ClientboundCustomPayload::ID {
        return None;
    }
    let CustomPayload::Unknown { channel, payload } =
        ClientboundCustomPayload::decode(&mut frame.body)
            .unwrap()
            .payload
    else {
        return None;
    };
    (channel.to_string() == TICK_MARKER_CHANNEL).then(|| {
        assert_eq!(payload.len(), 16);
        (
            u64::from_be_bytes(payload[..8].try_into().unwrap()),
            u64::from_be_bytes(payload[8..].try_into().unwrap()),
        )
    })
}

fn markers_for(packets: &[AddressedPacket], connection: u64) -> Vec<(u64, u64)> {
    packets
        .iter()
        .filter(|packet| packet.connection == ConnectionId(connection))
        .filter_map(|packet| marker(&packet.data))
        .collect()
}

#[test]
fn marker_frame_layout_is_tick_then_wall_big_endian() {
    let frame = tick_marker_frame(0x0102_0304_0506_0708, 0x1112_1314_1516_1718);
    assert_eq!(
        marker(&frame),
        Some((0x0102_0304_0506_0708, 0x1112_1314_1516_1718))
    );
}

/// One marker per published tick, first among the recorder's world frames,
/// carrying the core tick and its wall start; a player gets none.
#[test]
fn every_publish_marks_recorders_only_first_on_the_world_queue() {
    let mut recorder = viewer(1, true);
    let mut player = viewer(2, false);
    let mut seen = Vec::new();
    for tick in [40, 41, 43] {
        let wall = 1_800_000_000_000_000_000 + tick * 50_000_000;
        let packets = publish(
            &output(tick, wall),
            [&mut recorder, &mut player].into_iter(),
            4,
        );
        let first_world = packets
            .iter()
            .find(|packet| {
                packet.connection == ConnectionId(1) && packet.priority == Priority::BestEffort
            })
            .expect("recorder world frame");
        assert_eq!(marker(&first_world.data), Some((tick, wall)));
        assert_eq!(markers_for(&packets, 1).len(), 1);
        assert!(markers_for(&packets, 2).is_empty(), "players get no marker");
        seen.extend(markers_for(&packets, 1).into_iter().map(|(tick, _)| tick));
    }
    assert_eq!(seen, [40, 41, 43], "skipped ticks are gaps, never invented");
}

/// The marker takes a world slot like any frame: a full queue plans
/// nothing that tick, marker included, and the reservation never lets the
/// plan exceed the free slots.
#[test]
fn marker_reserves_its_world_slot() {
    let mut recorder = viewer(1, true);
    recorder.world_credit = 0;
    let packets = publish(&output(7, 1), [&mut recorder].into_iter(), 4);
    assert!(markers_for(&packets, 1).is_empty());
    for credit in [1, 3] {
        recorder.world_credit = credit;
        let packets = publish(&output(8, 2), [&mut recorder].into_iter(), 4);
        assert_eq!(markers_for(&packets, 1), [(8, 2)]);
        assert!(
            packets
                .iter()
                .filter(|packet| packet.priority != Priority::Reliable)
                .count()
                <= credit
        );
    }
}

/// Deferred entity work is summed and maximised over recorders only, and a
/// publish resets each viewer's count, even one it does not plan.
#[test]
fn recorder_tick_totals_recorders_only() {
    let mut viewers = [viewer(1, true), viewer(2, true), viewer(3, false)];
    for (viewer, deferred) in viewers.iter_mut().zip([4, 9, 100]) {
        viewer.deferred_updates = deferred;
    }
    assert_eq!(
        RecorderTick::of(&viewers),
        Some(RecorderTick {
            connections: 2,
            deferred_updates_sum: 13,
            deferred_updates_max: 9,
        })
    );
    assert_eq!(RecorderTick::of(&viewers[2..]), None);
    // No snapshot of this dimension: only the reset can zero the count.
    viewers[2].dimension = mc_core::dimensions::Dimension::Nether;
    publish(&output(5, 1), viewers.iter_mut(), 4);
    assert!(viewers.iter().all(|viewer| viewer.deferred_updates == 0));
}

#[test]
fn recorder_section_parses_and_matches_names_case_insensitively() {
    let config = crate::config::EdgeConfig::from_toml(
        "[recorder]\nplayers = [\"Camera\"]\nworld_queue_bytes = 4096\n",
        None,
    )
    .unwrap();
    assert!(config.recorder.is_recorder("camera"));
    assert!(config.recorder.is_recorder("CAMERA"));
    assert!(!config.recorder.is_recorder("Alex"));
    assert_eq!(config.recorder.world_queue_bytes, 4096);
    let default = crate::config::EdgeConfig::from_toml("", None).unwrap();
    assert!(!default.recorder.is_recorder("Camera"));
    assert_eq!(default.recorder.world_queue_bytes, 16 << 20);
    assert!(
        crate::config::EdgeConfig::from_toml("[recorder]\nworld_queue_bytes = 0\n", None).is_err()
    );
}

/// Recorder mode needs a listed name and a direct loopback peer, unless
/// `allow_remote` is set; a proxied or remote client using the name is a
/// player.
#[test]
fn recorder_mode_is_loopback_only_unless_allow_remote() {
    let loopback = Some(std::net::IpAddr::from([127, 0, 0, 1]));
    let remote = Some(std::net::IpAddr::from([203, 0, 113, 7]));
    let local = crate::config::EdgeConfig::from_toml("[recorder]\nplayers = [\"Camera\"]\n", None)
        .unwrap()
        .recorder;
    assert!(!local.allow_remote);
    assert!(local.grants("camera", loopback, false));
    assert!(local.grants(
        "Camera",
        Some(std::net::IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1])),
        false
    ));
    assert!(!local.grants("Camera", remote, false));
    // A dual-stack `::` listener sees an IPv4 loopback client as mapped.
    let mapped = |ip: [u8; 4]| {
        Some(std::net::IpAddr::V6(
            std::net::Ipv4Addr::from(ip).to_ipv6_mapped(),
        ))
    };
    assert!(local.grants("Camera", mapped([127, 0, 0, 1]), false));
    assert!(!local.grants("Camera", mapped([203, 0, 113, 7]), false));
    assert!(!local.grants("Camera", loopback, true));
    assert!(!local.grants("Camera", None, false));
    assert!(!local.grants("Alex", loopback, false));
    let open = crate::config::EdgeConfig::from_toml(
        "[recorder]\nplayers = [\"Camera\"]\nallow_remote = true\n",
        None,
    )
    .unwrap()
    .recorder;
    assert!(open.grants("Camera", remote, false));
    assert!(open.grants("Camera", remote, true));
    assert!(!open.grants("Alex", remote, false));
}
