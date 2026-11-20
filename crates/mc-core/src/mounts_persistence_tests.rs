use super::*;
use crate::journal::{EntityStateImage, JournalWriter, Replay, replay_batches};
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
    let entries = journal_entries_for_tick(&mut state, &out).unwrap();
    let mut writer = JournalWriter::open_test(&dir, 1).unwrap();
    writer.append(1, state.tick, &entries).unwrap();
    writer.flush().unwrap();
    drop(writer);
    let batches =
        crate::journal::read_complete_batches_after_test(&dir.join("journal"), 0).unwrap();
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
        let initial = journal_entries_for_tick(&mut state, &baseline).unwrap();
        let (output, _, _) = run_tick(
            &mut state,
            &[PlayerInput::Leave {
                connection: ConnectionId(leaving),
            }],
        );
        let entries = journal_entries_for_tick(&mut state, &output).unwrap();
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
        let mut writer = JournalWriter::open_test(dir.path(), 1).unwrap();
        writer.append(1, baseline.tick, &initial).unwrap();
        writer
            .append(baseline.tick + 1, output.tick, &entries)
            .unwrap();
        writer.flush().unwrap();
        drop(writer);
        let batches =
            crate::journal::read_complete_batches_after_test(&dir.path().join("journal"), 0)
                .unwrap();
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
    let mut writer = JournalWriter::open_test(&config.world_dir, 1).unwrap();
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

/// Produce the actual clean-stop case, then reopen without the passenger.
/// Every probe below runs before anybody issues a new Ride.
fn reopen_mount_without_passenger(
    config: &CoreConfig,
) -> (TickState, crate::boundary::EntityId, uuid::Uuid) {
    use crate::boundary::{ConnectionId, PlayerInput};
    let uuid = uuid::Uuid::from_u128(0x3333);
    let (mut state, actor, pos) = native_mounts_fixture(config, uuid);
    let horse = spawn(&mut state, "horse", pos);
    let horse_slot = slot(&state, horse);
    state.entities.row_mut(horse_slot).entity_life.tamed = true;
    let horse_uuid = state.entities.row(horse_slot).uuid;
    hold(&mut state, actor, "saddle", 1);
    run_tick(&mut state, &[use_entity(horse)]);
    hold(&mut state, actor, "saddle", 0);
    run_tick(&mut state, &[use_entity(horse)]);
    assert_eq!(crate::mounts::vehicle(&state, actor), Some(horse_slot));
    save_mounts_checkpoint(config, &mut state);
    drop(state);
    let core = Core::open(config.clone()).unwrap();
    let mut receiver = core.subscribe();
    assert!(core.submit(PlayerInput::Join {
        connection: ConnectionId(10),
        uuid,
        name: "rider".into(),
        permission_level: 0,
    }));
    let mut seated = false;
    for _ in 0..100 {
        let output = receiver.blocking_recv().unwrap();
        core.try_drain_outbox();
        seated =
            output.snapshot.entities.iter().any(|entity| {
                entity.uuid == horse_uuid && !entity.appearance.passengers.is_empty()
            });
        if seated {
            break;
        }
    }
    core.shutdown();
    drop(core);
    assert!(seated, "the real Core was stopped with its player riding");
    let reopened = Core::open(config.clone()).unwrap();
    let mut receiver = reopened.subscribe();
    receiver.blocking_recv().unwrap();
    reopened.shutdown();
    drop(reopened);
    let mut state =
        recover_dimension_state(config, crate::dimensions::Dimension::Overworld).unwrap();
    for dimension in [
        crate::dimensions::Dimension::Nether,
        crate::dimensions::Dimension::End,
    ] {
        state.other_dimensions.insert(
            dimension,
            Box::new(recover_dimension_state(config, dimension).unwrap()),
        );
    }
    let horse_slot = state.entities.slot_of_uuid(horse_uuid).unwrap();
    let horse = state.entities.body(horse_slot).id;
    assert_eq!(
        state.entities.row(horse_slot).entity_life.mount.riders,
        [uuid]
    );
    assert!(state.entities.slot_of_uuid(uuid).is_none());
    state.world.ensure_blocking(
        &[(crate::boundary::ChunkPos { x: 0, z: 0 }, 0)]
            .into_iter()
            .collect(),
    );
    (state, horse, uuid)
}

fn reconnect_pending_mount_rider(
    state: &mut TickState,
    horse: crate::boundary::EntityId,
    uuid: uuid::Uuid,
) {
    let connection = crate::boundary::ConnectionId(20);
    run_tick(
        state,
        &[crate::boundary::PlayerInput::Join {
            connection,
            uuid,
            name: "rider".into(),
            permission_level: 0,
        }],
    );
    let actor = state.connections.entity_of(connection).unwrap();
    assert_eq!(
        crate::mounts::vehicle(state, actor),
        Some(slot(state, horse))
    );
    assert!(crate::mounts::occupied(
        &state.entities,
        &state.entities.row(slot(state, horse)).entity_life.mount
    ));
}

#[test]
fn mounts_absent_claim_after_core_restart_does_not_suppress_ai_or_breeding() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.view_distance = 0;
    config.simulation_distance = 0;
    config.checkpoint_every_ticks = u64::MAX;
    let (mut state, horse, uuid) = reopen_mount_without_passenger(&config);
    let horse_slot = slot(&state, horse);
    assert!(!crate::taming::suppress_ai(
        &state.entities,
        state.entities.row(horse_slot)
    ));
    assert!(crate::entity_life::can_breed(
        &state,
        state.entities.body(horse_slot),
        state.entities.row(horse_slot)
    ));
    reconnect_pending_mount_rider(&mut state, horse, uuid);
    assert!(crate::taming::suppress_ai(
        &state.entities,
        state.entities.row(horse_slot)
    ));
    assert!(!crate::entity_life::can_breed(
        &state,
        state.entities.body(horse_slot),
        state.entities.row(horse_slot)
    ));
}

#[test]
fn mounts_absent_claim_after_core_restart_does_not_block_leash_follow() {
    use crate::boundary::{ConnectionId, PlayerInput};
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.view_distance = 0;
    config.simulation_distance = 0;
    config.checkpoint_every_ticks = u64::MAX;
    let (mut state, horse, uuid) = reopen_mount_without_passenger(&config);
    let horse_slot = slot(&state, horse);
    let at = state.entities.pos(horse_slot);
    let holder_uuid = uuid::Uuid::from_u128(0x4444);
    run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection: ConnectionId(2),
            uuid: holder_uuid,
            name: "holder".into(),
            permission_level: 0,
        }],
    );
    let holder = state.connections.entity_of(ConnectionId(2)).unwrap();
    state
        .entities
        .set_pos(slot(&state, holder), [at[0] + 4.0, at[1], at[2]]);
    state
        .entities
        .set_leash(horse_slot, Some(crate::leash::Holder::Entity(holder_uuid)));
    state.entities.set_vel(horse_slot, [0.0; 3]);
    crate::leash::tick(&mut state, &mut crate::boundary::ChangeSet::default());
    assert!(
        state.entities.vel(horse_slot)[0] > 0.0,
        "the leash owner must pull an absent-claim vehicle"
    );
    let before = state.entities.pos(horse_slot);
    run_tick(&mut state, &[]);
    assert!(
        state.entities.pos(horse_slot)[0] > before[0],
        "native movement must admit leash follow too"
    );
    reconnect_pending_mount_rider(&mut state, horse, uuid);
    crate::leash::tick(&mut state, &mut crate::boundary::ChangeSet::default());
    assert_eq!(
        state.entities.vel(horse_slot)[0],
        0.0,
        "a live rider blocks leash following"
    );
}

#[test]
fn mounts_absent_claim_after_core_restart_does_not_block_portal_and_expires_on_travel() {
    use crate::dimensions::Dimension;
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.view_distance = 0;
    config.simulation_distance = 0;
    config.checkpoint_every_ticks = u64::MAX;
    let (mut state, horse, _) = reopen_mount_without_passenger(&config);
    let horse_slot = slot(&state, horse);
    let at = state.entities.pos(horse_slot);
    let cell = at.map(|coordinate| coordinate.floor() as i32);
    let portal = state
        .registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:end_portal").unwrap())
        .unwrap()
        .default
        .0;
    state.world.set_block(cell[0], cell[1], cell[2], portal);
    state.world.fold_overlay();
    state.entities.set_pos(
        horse_slot,
        [
            f64::from(cell[0]) + 0.5,
            f64::from(cell[1]) + 0.5,
            f64::from(cell[2]) + 0.5,
        ],
    );
    let want = crate::world::chunks_in_radius(crate::boundary::ChunkPos { x: 6, z: 0 }, 1)
        .into_iter()
        .map(|pos| (pos, 0))
        .collect();
    state
        .other_dimensions
        .get_mut(&Dimension::End)
        .unwrap()
        .world
        .ensure_blocking(&want);
    state.tick += 1;
    for world in state.other_dimensions.values_mut() {
        world.tick = state.tick;
    }
    let mut changes = std::collections::BTreeMap::new();
    crate::portals::travel::finish(&mut state, &mut changes);
    assert_eq!(
        crate::dimensions::travel::locate(&state, horse).unwrap().0,
        Dimension::End
    );
    let end = &state.other_dimensions[&Dimension::End];
    let horse_uuid = end.entities.row(slot(end, horse)).uuid;
    assert!(
        end.entities
            .row(slot(end, horse))
            .entity_life
            .mount
            .riders
            .is_empty(),
        "dimension travel ends pending claims"
    );
    save_mounts_checkpoint(&config, &mut state);
    drop(state);
    let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
    let mut owners = Vec::new();
    for (dimension, replay) in &recovered.dimensions {
        crate::mounts::persistence::validate_replay(replay).unwrap();
        if let Some(horse) = replay
            .entities
            .values()
            .find(|entity| entity.uuid == horse_uuid.to_string())
        {
            owners.push(*dimension);
            assert!(horse.state.entity_life.mount.riders.is_empty());
        }
    }
    assert_eq!(owners, [Dimension::End]);
}
