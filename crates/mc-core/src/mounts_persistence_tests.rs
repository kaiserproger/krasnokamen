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

fn native_mounts_fixture(
    config: &CoreConfig,
    uuid: uuid::Uuid,
) -> (TickState, crate::boundary::EntityId, [f64; 3]) {
    use crate::dimensions::Dimension;
    let mut state = recover_dimension_state(config, Dimension::Overworld).unwrap();
    for dimension in [Dimension::Nether, Dimension::End] {
        state.other_dimensions.insert(
            dimension,
            Box::new(recover_dimension_state(config, dimension).unwrap()),
        );
    }
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    run_tick(
        &mut state,
        &[crate::boundary::PlayerInput::Join {
            connection: crate::boundary::ConnectionId(1),
            uuid,
            name: "rider".into(),
            permission_level: 0,
        }],
    );
    state.world.ensure_blocking(&state.desired_chunks());
    let actor = state
        .connections
        .entity_of(crate::boundary::ConnectionId(1))
        .unwrap();
    let pos = [8.5, 200.0, 8.5];
    for x in 5..=11 {
        for z in 5..=11 {
            state
                .world
                .set_block(x, 199, z, state.registry.states.stone);
            for y in 200..208 {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    state.world.fold_overlay();
    state.entities.set_pos(slot(&state, actor), pos);
    state.entities.set_ground(slot(&state, actor), true);
    (state, actor, pos)
}

fn save_mounts_checkpoint(config: &CoreConfig, state: &mut TickState) {
    let (work_tx, work_rx) = journal_work_channel();
    let (done_tx, _) = std::sync::mpsc::channel();
    assert!(start_checkpoint(state, &work_tx, &done_tx, state.tick));
    let JournalWork::Checkpoint(job) = work_rx.recv().unwrap() else {
        panic!("expected native checkpoint capture");
    };
    let mut manifest = crate::native_checkpoint::tests::read_manifest(config).unwrap();
    let mut writer = JournalWriter::open(&config.world_dir, 1).unwrap();
    writer.append(manifest.tick + 1, state.tick, &[]).unwrap();
    write_checkpoint(
        &config.world_dir,
        &job,
        &state.plugins,
        &mut writer,
        &mut manifest,
    )
    .unwrap();
}

#[test]
fn mounts_new_ride_reconciles_other_vehicle_claim_and_native_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.view_distance = 0;
    config.simulation_distance = 0;
    let uuid = uuid::Uuid::from_u128(0x1234);
    let (mut state, actor, pos) = native_mounts_fixture(&config, uuid);
    let old = spawn(&mut state, "pig", [pos[0] + 512.0, pos[1], pos[2]]);
    let target = spawn(&mut state, "pig", pos);
    let (old_slot, target_slot) = (slot(&state, old), slot(&state, target));
    state.entities.row_mut(old_slot).controls.no_ai = true;
    state.entities.row_mut(target_slot).controls.no_ai = true;
    hold(&mut state, actor, "saddle", 1);
    run_tick(&mut state, &[use_entity(target)]);
    hold(&mut state, actor, "carrot_on_a_stick", 1);
    // Reconstruct the old vehicle's durable claim without a restart-local
    // reverse link. Live Ride must reconcile that authority itself.
    state.entities.set_riders(old_slot, vec![uuid]);
    assert!(crate::mounts::vehicle(&state, actor).is_none());
    assert!(!state.world.is_active(
        ((pos[0] + 512.0) as i32).div_euclid(16),
        (pos[2] as i32).div_euclid(16),
    ));
    run_tick(&mut state, &[use_entity(target)]);
    assert_eq!(
        state.entities.row(target_slot).entity_life.mount.riders,
        [uuid]
    );
    assert!(
        state
            .entities
            .row(old_slot)
            .entity_life
            .mount
            .riders
            .is_empty(),
        "a new Ride must remove the stale claim even from an inactive vehicle"
    );
    save_mounts_checkpoint(&config, &mut state);
    drop(state);
    let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
    let replay = &recovered.dimensions[&crate::dimensions::Dimension::Overworld];
    crate::mounts::persistence::validate_replay(replay).unwrap();
    assert!(
        replay.entities[&old.0]
            .state
            .entity_life
            .mount
            .riders
            .is_empty()
    );
    assert_eq!(
        replay.entities[&target.0].state.entity_life.mount.riders,
        [uuid]
    );
}

#[test]
fn mounts_plugin_unlock_restores_pending_seat_without_waiting_for_active_tick() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.view_distance = 0;
    config.simulation_distance = 0;
    let (mut state, first, pos) = native_mounts_fixture(&config, uuid::Uuid::from_u128(1));
    let target = spawn(&mut state, "pig", pos);
    hold(&mut state, first, "saddle", 1);
    run_tick(&mut state, &[use_entity(target)]);
    let uuid = uuid::Uuid::from_u128(0x5678);
    let far = [pos[0] + 512.0, pos[1], pos[2]];
    let pig = spawn(&mut state, "pig", far);
    let pig_slot = slot(&state, pig);
    state.entities.set_riders(pig_slot, vec![uuid]);
    state.require_plugin_login = true;
    let connection = crate::boundary::ConnectionId(2);
    run_tick(
        &mut state,
        &[crate::boundary::PlayerInput::Join {
            connection,
            uuid,
            name: "authenticated".into(),
            permission_level: 0,
        }],
    );
    let actor = state.connections.entity_of(connection).unwrap();
    assert!(state.session_locked(connection));
    assert!(crate::mounts::vehicle(&state, actor).is_none());
    assert!(!state.world.is_active(
        (far[0] as i32).div_euclid(16),
        (far[2] as i32).div_euclid(16),
    ));
    crate::tick::unlock_session(&mut state, connection);
    assert!(!state.session_locked(connection));
    assert_eq!(state.entities.row(slot(&state, actor)).uuid, uuid);
    assert_eq!(
        crate::mounts::vehicle(&state, actor),
        Some(pig_slot),
        "unlock restores the real UUID and its vehicle before another input"
    );
    assert_eq!(state.entities.pos(slot(&state, actor))[0], far[0]);
    // The authenticated passenger may deliberately leave A and take B. The
    // resulting native checkpoint must never retain that UUID on both.
    crate::mounts::dismount(&mut state, actor);
    let actor_slot = slot(&state, actor);
    state.entities.set_pos(actor_slot, pos);
    state.entities.set_ground(actor_slot, true);
    hold(&mut state, actor, "carrot_on_a_stick", 1);
    run_tick(
        &mut state,
        &[crate::boundary::PlayerInput::InteractEntity {
            connection,
            target,
            hand: crate::boundary::Hand::Main,
            location: [0.0; 3],
        }],
    );
    assert_eq!(
        crate::mounts::vehicle(&state, actor),
        Some(slot(&state, target))
    );
    assert!(
        state
            .entities
            .row(pig_slot)
            .entity_life
            .mount
            .riders
            .is_empty()
    );
    save_mounts_checkpoint(&config, &mut state);
    drop(state);
    let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
    let replay = &recovered.dimensions[&crate::dimensions::Dimension::Overworld];
    crate::mounts::persistence::validate_replay(replay).unwrap();
    assert!(
        replay.entities[&pig.0]
            .state
            .entity_life
            .mount
            .riders
            .is_empty()
    );
    assert_eq!(
        replay.entities[&target.0].state.entity_life.mount.riders,
        [uuid]
    );
}

#[test]
fn mounts_clean_core_shutdown_and_reopen_keep_the_pending_rider() {
    use crate::boundary::{ConnectionId, PlayerInput};
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.view_distance = 0;
    config.simulation_distance = 0;
    config.checkpoint_every_ticks = u64::MAX;
    let uuid = uuid::Uuid::from_u128(0x9abc);
    let (mut state, actor, pos) = native_mounts_fixture(&config, uuid);
    let pig = spawn(&mut state, "pig", pos);
    let pig_slot = slot(&state, pig);
    state.entities.row_mut(pig_slot).controls.no_ai = true;
    let pig_uuid = state.entities.row(pig_slot).uuid;
    hold(&mut state, actor, "saddle", 1);
    run_tick(&mut state, &[use_entity(pig)]);
    hold(&mut state, actor, "carrot_on_a_stick", 1);
    run_tick(&mut state, &[use_entity(pig)]);
    assert_eq!(crate::mounts::vehicle(&state, actor), Some(pig_slot));
    save_mounts_checkpoint(&config, &mut state);
    drop(state);
    let core = Core::open(config.clone()).unwrap();
    let mut receiver = core.subscribe();
    assert!(core.submit(PlayerInput::Join {
        connection: ConnectionId(20),
        uuid,
        name: "rider".into(),
        permission_level: 0,
    }));
    let mut seated = false;
    for _ in 0..100 {
        let output = receiver.blocking_recv().unwrap();
        core.try_drain_outbox();
        seated = output
            .snapshot
            .entities
            .iter()
            .any(|entity| entity.uuid == pig_uuid && !entity.appearance.passengers.is_empty());
        if seated {
            break;
        }
    }
    core.shutdown();
    drop(core);
    assert!(
        seated,
        "the real Core must have a live seated player before shutdown"
    );
    let reopened = Core::open(config.clone()).unwrap();
    reopened.shutdown();
    drop(reopened);
    let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
    let pig = recovered.dimensions[&crate::dimensions::Dimension::Overworld]
        .entities
        .values()
        .find(|entity| entity.uuid == pig_uuid.to_string())
        .unwrap();
    assert_eq!(pig.state.entity_life.mount.riders, [uuid]);
    println!("clean Core::shutdown without Leave retains the rider UUID after reopen");
}
