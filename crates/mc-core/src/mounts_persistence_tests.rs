use super::*;
use crate::journal::{
    EntityStateImage, JournalWriter, Replay, read_complete_batches_after, replay_batches,
};
use crate::mounts::tests::{fixture, hold, slot, spawn, use_entity};
use crate::tick::run_tick;
#[test]
fn actual_region_checkpoint_and_wal_tail_restore_tame_owner_collar_and_equipment() {
    let (mut state, actor, pos) = fixture();
    let wolf = spawn(&mut state, "wolf", pos);
    let w = slot(&state, wolf);
    hold(&mut state, actor, "bone", 64);
    for _ in 0..32 {
        run_tick(&mut state, &[use_entity(wolf)]);
        if state.entities.row(w).entity_life.tamed {
            break;
        }
    }
    assert!(state.entities.row(w).entity_life.tamed);
    hold(&mut state, actor, "wolf_armor", 1);
    run_tick(&mut state, &[use_entity(wolf)]);
    let (_, regions, _) = checkpoint_data_for(&state, state.tick);
    let dir = std::env::temp_dir().join(format!(
        "krasnokamen-mount-restart-{}",
        uuid::Uuid::new_v4()
    ));
    mc_world::anvil::entity_regions::write_entity_regions(
        &dir,
        &state.registry.items,
        &regions,
        state.tick as u32,
    )
    .unwrap();
    let read =
        mc_world::anvil::entity_regions::read_entity_regions(&dir, &state.registry.items).unwrap();
    let native = read
        .values()
        .flat_map(|chunk| &chunk.entities)
        .find(|e| e.id == "minecraft:wolf")
        .unwrap();
    let image = EntityStateImage::from_record(&state.registry, native).unwrap();
    assert_eq!(
        image.entity_life.taming,
        state.entities.row(w).entity_life.taming
    );
    let mut checkpoint = Replay::default();
    checkpoint.entities.insert(
        wolf.0,
        crate::journal::ReplayEntity {
            uuid: native.uuid.to_string(),
            type_id: crate::mounts::type_id(state.entities.body(w)).unwrap(),
            pos: native.pos,
            vel: velocity_from_vanilla_motion(native.motion),
            yaw: native.yaw,
            pitch: native.pitch,
            ground: native.on_ground,
            health: native.health,
            alive: true,
            age: image.age,
            breed_cooldown: image.breed_cooldown,
            pickup_delay: 0,
            item: None,
            appearance: appearance_from_vanilla_entity(native),
            active_effects: Default::default(),
            villager: None,
            projectile: None,
            state: image,
            tick: state.tick,
        },
    );
    hold(&mut state, actor, "blue_dye", 1);
    let (out, _, _) = run_tick(&mut state, &[use_entity(wolf)]);
    let entries = journal_entries_for_tick(&mut state, &out);
    let mut writer = JournalWriter::open(&dir, 1).unwrap();
    writer.append(1, state.tick, &entries).unwrap();
    writer.flush().unwrap();
    drop(writer);
    let batches = read_complete_batches_after(&dir.join("journal"), 0).unwrap();
    let replay = replay_batches(checkpoint, &batches);
    let (mut restored, _, _) = fixture();
    restore_entity(&mut restored, wolf.0, &replay.entities[&wolf.0]).unwrap();
    let recovered = restored.entities.row(slot(&restored, wolf));
    assert_eq!(
        recovered.entity_life.taming,
        state.entities.row(w).entity_life.taming
    );
    assert_eq!(
        recovered.entity_life.sitting,
        state.entities.row(w).entity_life.sitting
    );
    assert_eq!(
        recovered.loot.attributes,
        state.entities.row(w).loot.attributes
    );
    assert_eq!(
        recovered.loot.body_armor,
        state.entities.row(w).loot.body_armor
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn happy_ghast_admission_leave_and_reconnect_survive_wal_recovery() {
    use crate::boundary::{ConnectionId, Hand, PlayerInput};
    for leaving in [1, 2] {
        let (mut state, actor, pos) = fixture();
        let first_uuid = state.entities.row(slot(&state, actor)).uuid;
        let second_uuid = uuid::Uuid::new_v4();
        let ghast = spawn(&mut state, "happy_ghast", pos);
        let ghast_slot = slot(&state, ghast);
        hold(&mut state, actor, "red_harness", 1);
        run_tick(&mut state, &[use_entity(ghast)]);
        run_tick(
            &mut state,
            &[PlayerInput::Join {
                connection: ConnectionId(2),
                uuid: second_uuid,
                name: "second".into(),
                permission_level: 0,
            }],
        );
        let second = state.connections.entity_of(ConnectionId(2)).unwrap();
        for (connection, passenger) in [(ConnectionId(1), actor), (ConnectionId(2), second)] {
            let passenger_slot = slot(&state, passenger);
            state
                .entities
                .set_pos(passenger_slot, state.entities.pos(ghast_slot));
            run_tick(
                &mut state,
                &[PlayerInput::InteractEntity {
                    connection,
                    target: ghast,
                    hand: Hand::Main,
                    location: [0.0; 3],
                }],
            );
        }
        assert_eq!(
            state.entities.row(ghast_slot).entity_life.mount.riders,
            vec![first_uuid, second_uuid]
        );
        let (baseline, _, _) = run_tick(&mut state, &[]);
        let initial = journal_entries_for_tick(&mut state, &baseline);
        let (output, _, _) = run_tick(
            &mut state,
            &[PlayerInput::Leave {
                connection: ConnectionId(leaving),
            }],
        );
        let entries = journal_entries_for_tick(&mut state, &output);
        let expected = if leaving == 1 {
            second_uuid
        } else {
            first_uuid
        };
        assert_eq!(
            state.entities.row(ghast_slot).entity_life.mount.riders,
            vec![expected]
        );
        let dir = tempfile::tempdir().unwrap();
        let mut writer = JournalWriter::open(dir.path(), 1).unwrap();
        writer.append(1, baseline.tick, &initial).unwrap();
        writer
            .append(baseline.tick + 1, output.tick, &entries)
            .unwrap();
        writer.flush().unwrap();
        drop(writer);
        let batches = read_complete_batches_after(&dir.path().join("journal"), 0).unwrap();
        let replay = replay_batches(Replay::default(), &batches);
        let record = &replay.entities[&ghast.0];
        assert_eq!(record.state.entity_life.mount.riders, vec![expected]);
        let (mut restored, _, _) = fixture();
        restore_entity(&mut restored, ghast.0, record).unwrap();
        let restored_slot = slot(&restored, ghast);
        assert_eq!(
            restored
                .entities
                .row(restored_slot)
                .entity_life
                .mount
                .riders,
            vec![expected]
        );
        let uuid = if leaving == 1 {
            first_uuid
        } else {
            second_uuid
        };
        run_tick(
            &mut restored,
            &[PlayerInput::Join {
                connection: ConnectionId(20),
                uuid,
                name: "returning".into(),
                permission_level: 0,
            }],
        );
        let rejoined = restored.connections.entity_of(ConnectionId(20)).unwrap();
        assert!(crate::mounts::vehicle(&restored, rejoined).is_none());
        assert_eq!(
            restored
                .entities
                .row(restored_slot)
                .entity_life
                .mount
                .riders,
            vec![expected]
        );
        run_tick(&mut restored, &[]);
        assert_eq!(
            restored
                .entities
                .row(restored_slot)
                .entity_life
                .mount
                .riders,
            vec![expected],
            "a recovered passenger need not reconnect on the first tick"
        );
        run_tick(
            &mut restored,
            &[PlayerInput::Join {
                connection: ConnectionId(21),
                uuid: expected,
                name: "retained".into(),
                permission_level: 0,
            }],
        );
        let retained = restored.connections.entity_of(ConnectionId(21)).unwrap();
        assert_eq!(
            crate::mounts::vehicle(&restored, retained),
            Some(restored_slot)
        );
        run_tick(
            &mut restored,
            &[PlayerInput::Leave {
                connection: ConnectionId(21),
            }],
        );
        assert!(
            restored
                .entities
                .row(restored_slot)
                .entity_life
                .mount
                .riders
                .is_empty()
        );
    }
}
