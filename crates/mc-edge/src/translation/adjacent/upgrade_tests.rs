use super::super::{
    play,
    tables::{Formats, Tables},
    test_session,
};
use super::*;

fn roundtrip(name: &str, old: &[u8]) {
    let target_name = clientbound_name(name, old).unwrap();
    let upgraded = clientbound(name, Cow::Borrowed(old)).unwrap();
    let mut down = test_session(Tables {
        formats: Formats {
            upper: 777,
            lower: 776,
        },
        ..Tables::default()
    });
    let restored = play::clientbound_body(&mut down, target_name, &upgraded)
        .unwrap()
        .unwrap();
    assert_eq!(restored.as_ref(), old, "{name}");
}

#[test]
fn upgrade_movement_and_animation_layouts_have_measured_downgrade_roundtrips() {
    for action in [0, 2, 3, 4, 5] {
        roundtrip("minecraft:animate", &[7, action]);
    }
    for rotation in [false, true] {
        let mut body = vec![7];
        for delta in [1, -2, 3] {
            body.write_i16(delta);
        }
        if rotation {
            body.extend([4, 5]);
        }
        body.push(1);
        roundtrip(
            if rotation {
                "minecraft:move_entity_pos_rot"
            } else {
                "minecraft:move_entity_pos"
            },
            &body,
        );
    }
    roundtrip("minecraft:move_entity_rot", &[7, 4, 5, 1]);
    let mut sync = vec![7];
    for position in [1.0, 2.0, 3.0] {
        sync.write_f64(position);
    }
    for _ in 0..3 {
        sync.write_f64(0.0);
    }
    sync.write_f32(10.0);
    sync.write_f32(20.0);
    sync.push(1);
    roundtrip("minecraft:entity_position_sync", &sync);
    assert_eq!(
        clientbound("minecraft:animate", Cow::Borrowed(&[7, 0]))
            .unwrap()
            .as_ref(),
        &[7, 0, 1, 6]
    );
}

#[test]
fn particle_upgrade_repeats_the_old_scalar_speed_and_uses_the_measured_default() {
    let mut body = vec![0, 1];
    for value in [1.0, 2.0, 3.0] {
        body.write_f64(value);
    }
    for value in [4.0, 5.0, 6.0] {
        body.write_f32(value);
    }
    body.write_f32(0.5);
    body.write_i32(12);
    body.extend([9]); // already name-paired, payload-free ParticleOptions
    let out = clientbound("minecraft:level_particles", Cow::Owned(body)).unwrap();
    let mut r = out.as_ref();
    assert_eq!(r.read_varint().unwrap(), 9);
    assert!(!r.read_bool().unwrap());
    assert!(r.read_bool().unwrap());
    for value in [1.0, 2.0, 3.0] {
        assert_eq!(r.read_f64().unwrap(), value);
    }
    for value in [4.0, 5.0, 6.0] {
        assert_eq!(r.read_f32().unwrap(), value);
    }
    for _ in 0..3 {
        assert_eq!(r.read_f32().unwrap(), 0.5);
    }
    assert_eq!(r.read_varint().unwrap(), 12);
    assert_eq!(r.read_varint().unwrap(), 0);
    assert!(r.is_empty());
}

#[test]
fn team_upgrade_reorders_parameters_without_changing_them() {
    let mut body = Vec::new();
    body.write_string("team", 32767).unwrap();
    body.push(0);
    mc_nbt::write_network_value(&mut body, &mc_nbt::Tag::String("Team".into())).unwrap();
    body.extend([3, 0, 0, 21]); // options, visibility, collision, reset colour
    for value in ["Prefix", "Suffix"] {
        mc_nbt::write_network_value(&mut body, &mc_nbt::Tag::String(value.into())).unwrap();
    }
    body.push(0); // no players
    roundtrip("minecraft:set_player_team", &body);
}

#[test]
fn newer_inputs_are_explicitly_downgraded_and_unrepresentable_operations_error() {
    assert_eq!(serverbound("minecraft:punch", &[]).unwrap().unwrap(), [0]);
    assert_eq!(
        serverbound("minecraft:player_action", &[2, 7])
            .unwrap()
            .unwrap(),
        [1, 7]
    );
    assert!(
        serverbound("minecraft:player_action", &[1])
            .unwrap_err()
            .to_string()
            .contains("CHANGE_DESTROY_DIRECTION")
    );
    let mut sign = vec![0; 8];
    for line in ["a", "b", "c", "d"] {
        sign.write_string(line, 384).unwrap();
    }
    sign.push(1);
    let old = serverbound("minecraft:sign_update", &sign)
        .unwrap()
        .unwrap();
    let mut down = test_session(Tables::default());
    let restored = play::serverbound_body(&mut down, "minecraft:sign_update", &old)
        .unwrap()
        .unwrap();
    assert_eq!(restored.as_ref(), sign);
}
