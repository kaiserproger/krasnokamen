//! Real native transaction plus published WAL, without shutdown checkpointing the tail.
use super::*;

#[test]
fn native_frozen_item_clock_survives_checkpoint_tail_recovery_and_frozen_open() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = fixture(directory.path());
    drill.state.tick_control.set_frozen(true);
    let item = drill
        .state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap();
    let id = crate::tick::spawn_item_entity(
        &mut drill.state,
        [34.5, 200.0, 40.5],
        mc_data::ItemStack::new(item, 1),
    );
    let slot = drill.state.entities.slot_of(id).unwrap();
    drill.state.entities.row_mut(slot).age = 5999;
    let uuid = drill.state.entities.row(slot).uuid.to_string();
    drill.run(&[]).unwrap();
    drill.checkpoint();
    let checkpoint_tick = drill.manifest.tick;
    let game_time = drill.state.tick_control.game_ticks(drill.state.tick);
    for _ in 0..17 {
        drill.run(&[]).unwrap();
    }
    drill.writer.as_mut().unwrap().flush().unwrap();
    assert!(drill.state.tick > checkpoint_tick);
    assert_eq!(
        drill.state.tick_control.game_ticks(drill.state.tick),
        game_time
    );
    let config = drill.config.clone();
    let mut restored = drill.crash();
    let slot = restored.state.entities.slot_of(id).unwrap();
    assert_eq!(restored.state.entities.row(slot).age, 5999);
    assert_eq!(
        restored
            .state
            .entities
            .row(slot)
            .item
            .as_ref()
            .unwrap()
            .full_stack()
            .count,
        1
    );
    restored.state.tick_control.set_frozen(true);
    restored.run(&[]).unwrap();
    assert_eq!(restored.state.entities.row(slot).age, 5999);
    restored.checkpoint();
    drop(restored);

    let mut frozen_config = config.clone();
    frozen_config.start_frozen = true;
    let core = Core::open(frozen_config).unwrap();
    let mut ticks = core.subscribe();
    for _ in 0..3 {
        assert_eq!(ticks.blocking_recv().unwrap().snapshot.game_time, game_time);
    }
    core.shutdown();
    drop(core);
    let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
    let item = recovered.dimensions[&crate::dimensions::Dimension::Overworld]
        .entities
        .values()
        .find(|record| record.uuid == uuid)
        .unwrap();
    assert_eq!(item.age, 5999);
    assert_eq!(item.item.as_ref().unwrap().count, 1);
    drop(recovered);

    let mut resumed = recovered_world(&config);
    resumed
        .world
        .ensure_blocking(&[(CHUNK, 0)].into_iter().collect());
    resumed
        .tickets
        .insert("frozen-item-restart".into(), (CHUNK, 0));
    resumed.tick_control.set_frozen(false);
    let (output, _, _) = crate::tick::run_tick(&mut resumed, &[]);
    assert!(resumed.entities.slot_of(id).is_none());
    assert!(output.changes.entities_removed.contains(&id));
}

#[test]
fn native_game_clock_overworld_only_transition_controls_nether_and_end_items() {
    use crate::dimensions::Dimension;
    for dimension in [Dimension::Nether, Dimension::End] {
        let directory = tempfile::tempdir().unwrap();
        let mut drill = fixture(directory.path());
        let item = drill
            .state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
            .unwrap();
        let child = drill.state.other_dimensions.get_mut(&dimension).unwrap();
        child
            .world
            .ensure_blocking(&[(CHUNK, 0)].into_iter().collect());
        child.tickets.insert("native-game-clock".into(), (CHUNK, 0));
        child.entities.set_next_wire_id(1000);
        let id = crate::tick::spawn_item_entity(
            child,
            [34.5, 200.0, 40.5],
            mc_data::ItemStack::new(item, 1),
        );
        let slot = child.entities.slot_of(id).unwrap();
        child.entities.row_mut(slot).age = 5998;
        let uuid = child.entities.row(slot).uuid.to_string();
        drill.run(&[]).unwrap();
        let child = &drill.state.other_dimensions[&dimension];
        assert_eq!(
            child.entities.row(child.entities.slot_of(id).unwrap()).age,
            5999
        );
        drill.checkpoint();
        drill.state.tick_control.set_frozen(true);
        let mut transitions = 0;
        for _ in 0..17 {
            let mut batch = drill.capture_tick(&[]);
            for entry in &mut batch.entries {
                if let JournalEntry::Climate { value } = entry {
                    let value = *value;
                    *entry = JournalEntry::Dimension {
                        dimension: Dimension::Overworld,
                        entries: vec![JournalEntry::Climate { value }],
                    };
                    transitions += 1;
                }
            }
            drill.append(&batch).unwrap();
        }
        assert_eq!(transitions, 1);
        drill.writer.as_mut().unwrap().flush().unwrap();
        let config = drill.config.clone();
        drop(drill); // Published tail, no final checkpoint.
        let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
        let record = recovered.dimensions[&dimension]
            .entities
            .values()
            .find(|record| record.uuid == uuid)
            .unwrap();
        assert_eq!(record.age, 5999);
        assert_eq!(record.item.as_ref().unwrap().count, 1);
        drop(recovered);
        let mut resumed = recovered_world(&config);
        let child = resumed.other_dimensions.get_mut(&dimension).unwrap();
        child
            .world
            .ensure_blocking(&[(CHUNK, 0)].into_iter().collect());
        child.tickets.insert("native-game-clock".into(), (CHUNK, 0));
        resumed.tick_control.set_frozen(false);
        crate::tick::run_tick(&mut resumed, &[]);
        assert!(
            resumed.other_dimensions[&dimension]
                .entities
                .slot_of(id)
                .is_none()
        );
    }
}

#[test]
fn native_debt_game_clock_active_checkpoint_and_tail_keep_the_global_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = fixture(directory.path());
    drill.state.tick_control.set_frozen(true);
    for _ in 0..3 {
        drill.run(&[]).unwrap();
    }
    drill.checkpoint();
    assert!(drill.manifest.debt.iter().any(|image| image.active));
    for image in drill.manifest.debt.iter().filter(|image| image.active) {
        assert_eq!(image.held_anchor, Some(3));
    }
    for _ in 0..2 {
        drill.run(&[]).unwrap();
    }
    drill.writer.as_mut().unwrap().flush().unwrap();
    let tick = drill.state.tick;
    let config = drill.config.clone();
    drop(drill);
    let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
    let root = &recovered.dimensions[&crate::dimensions::Dimension::Overworld];
    assert_eq!(root.recovered_held_ticks, 5);
    for image in root.sim_debt.values().filter(|image| image.active) {
        assert_eq!((image.last_sim, image.held_anchor), (tick, Some(5)));
    }
    let encoded = crate::native_checkpoint::encode_checkpoint(&recovered).unwrap();
    let manifest: CheckpointManifest =
        serde_json::from_slice(&encoded.globals["checkpoint"].payload).unwrap();
    for image in manifest.debt.iter().filter(|image| image.active) {
        assert_eq!((image.last_sim, image.held_anchor), (tick, Some(5)));
    }
}

#[test]
fn native_debt_game_clock_old_held_writer_crash_refuses_new_open() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = fixture(directory.path());
    drill.state.tick_control.set_frozen(true);
    drill.run(&[]).unwrap();
    drill.checkpoint();
    let tick = drill.state.tick + 1;
    // This is the old writer's exact serialized shape: no held anchor and,
    // because its held climate did not change, no Climate entry in the tail.
    drill
        .writer
        .as_mut()
        .unwrap()
        .append(
            tick,
            tick,
            &[JournalEntry::SimDebt {
                image: crate::journal::SimDebtImage {
                    chunk: [CHUNK.x, CHUNK.z],
                    last_sim: 0,
                    held_anchor: None,
                    remaining: 350,
                    active: false,
                    tick,
                },
            }],
        )
        .unwrap();
    drill.writer.as_mut().unwrap().flush().unwrap();
    let config = drill.config.clone();
    let checkpoint = drill.manifest.tick;
    drop(drill);
    let error = crate::native_checkpoint::recover_offline(&config)
        .err()
        .expect("old writer must refuse");
    assert!(
        error
            .to_string()
            .contains("anchorless simulation-debt WAL image"),
        "{error}"
    );
    let error = Core::open(config.clone()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("anchorless simulation-debt WAL image"),
        "{error}"
    );
    assert_eq!(read_manifest(&config).unwrap().tick, checkpoint);
}

#[test]
fn native_debt_game_clock_legacy_zero_epoch_admits_its_entire_old_tail() {
    for kind in ["running_zero", "without_game_clock", "without_climate"] {
        let directory = tempfile::tempdir().unwrap();
        let drill = fixture(directory.path());
        let config = drill.config.clone();
        let mut manifest = drill.manifest.clone();
        drop(drill);
        let climate = manifest.climate.as_mut().unwrap();
        climate.sim_debt_anchored_since = None;
        if kind == "without_game_clock" {
            let mut json = serde_json::to_value(*climate).unwrap();
            json.as_object_mut().unwrap().remove("game_clock");
            *climate = serde_json::from_value(json).unwrap();
        } else if kind == "without_climate" {
            manifest.climate = None;
        }
        crate::native_checkpoint::tests::write_manifest(&config, &manifest).unwrap();
        let tick = manifest.tick + 1;
        let mut writer =
            JournalWriter::open_recovered(&config.world_dir, 1, manifest.tick, manifest.tick)
                .unwrap();
        writer
            .append(
                tick,
                tick,
                &[JournalEntry::SimDebt {
                    image: crate::journal::SimDebtImage {
                        chunk: [CHUNK.x, CHUNK.z],
                        last_sim: manifest.tick,
                        held_anchor: None,
                        remaining: 0,
                        active: false,
                        tick,
                    },
                }],
            )
            .unwrap();
        writer.flush().unwrap();
        drop(writer);
        let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
        let root = &recovered.dimensions[&crate::dimensions::Dimension::Overworld];
        assert_eq!(root.sim_debt_anchored_since, Some(tick + 1), "{kind}");
        assert_eq!(
            root.sim_debt[&[CHUNK.x, CHUNK.z]].held_anchor,
            Some(0),
            "{kind}"
        );
        drop(recovered);
        let core = Core::open(config).unwrap();
        core.shutdown();
    }
}

#[test]
fn native_debt_game_clock_unflagged_held_or_nonzero_running_refuses_upgrade() {
    for resume in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut drill = fixture(directory.path());
        drill.state.tick_control.set_frozen(true);
        drill.run(&[]).unwrap();
        if resume {
            drill.state.tick_control.set_frozen(false);
            drill.run(&[]).unwrap();
        }
        drill.checkpoint();
        let config = drill.config.clone();
        let mut manifest = drill.manifest.clone();
        drop(drill);
        manifest.climate.as_mut().unwrap().sim_debt_anchored_since = None;
        crate::native_checkpoint::tests::write_manifest(&config, &manifest).unwrap();
        let error = crate::native_checkpoint::recover_offline(&config)
            .err()
            .expect("legacy held epoch must refuse");
        assert!(
            error
                .to_string()
                .contains("world was frozen by an older binary"),
            "{error}"
        );
        assert_eq!(read_manifest(&config).unwrap().climate, manifest.climate);
    }
}

#[test]
fn native_debt_game_clock_running_nonzero_epoch_capture_load_capture_is_byte_identical() {
    let directory = tempfile::tempdir().unwrap();
    // Online -> offline player clocks deliberately normalize on recovery.
    // Exercise byte stability of a running world without that session change.
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.view_distance = 0;
    config.simulation_distance = 0;
    let mut drill = Drill::open(config);
    drill
        .state
        .world
        .ensure_blocking(&[(CHUNK, 0)].into_iter().collect());
    drill.state.tickets.insert("byte-clock".into(), (CHUNK, 0));
    drill.state.tick_control.set_frozen(true);
    let item = drill
        .state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap();
    crate::tick::spawn_item_entity(
        &mut drill.state,
        [38.5, 200.0, 40.5],
        mc_data::ItemStack::new(item, 1),
    );
    for _ in 0..3 {
        drill.run(&[]).unwrap();
    }
    drill.state.tick_control.set_frozen(false);
    drill.run(&[]).unwrap();
    drill.checkpoint();
    let config = drill.config.clone();
    let before = fixture_store(&config).unwrap().read_checkpoint().unwrap();
    drop(drill);
    let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
    let after = crate::native_checkpoint::encode_checkpoint(&recovered).unwrap();
    let registry = Arc::clone(&recovered.registry);
    drop(recovered);
    let store = fixture_store(&config).unwrap();
    store
        .commit_checkpoint(
            mc_world::native::NativeCheckpoint {
                tick: after.tick,
                previous_tick: after.tick,
                chunks: &[],
                entities: &after.entities,
                players: &after.players,
                globals: &after.globals,
            },
            &registry.blocks,
            &registry.items,
        )
        .unwrap();
    let after = store.read_checkpoint().unwrap();
    assert_eq!(before.tick, after.tick);
    assert!(before.globals.keys().eq(after.globals.keys()));
    for (key, record) in &before.globals {
        assert_eq!(record, &after.globals[key], "native domain {key}");
    }
    assert_eq!(before.players, after.players);
    assert_eq!(before.entities.len(), after.entities.len());
    for old in &before.entities {
        let new = after
            .entities
            .iter()
            .find(|new| new.identity == old.identity)
            .unwrap();
        assert_eq!(old.dimension, new.dimension);
        assert_eq!(old.position, new.position);
        assert_eq!(old.generated_slot, new.generated_slot);
        assert_eq!(old.journal_lsn, new.journal_lsn);
        let encode = |payload: &Option<mc_nbt::Tag>| {
            let mut bytes = Vec::new();
            if let Some(payload) = payload {
                mc_nbt::write_named(&mut bytes, "", payload).unwrap();
            }
            bytes
        };
        assert_eq!(encode(&old.payload), encode(&new.payload));
    }
}

fn debt_dimension(
    state: &mut TickState,
    dimension: crate::dimensions::Dimension,
) -> &mut TickState {
    if dimension == crate::dimensions::Dimension::Overworld {
        state
    } else {
        state.other_dimensions.get_mut(&dimension).unwrap()
    }
}

#[test]
fn native_debt_game_clock_cold_unload_multiple_holds_restart_and_reload_are_exact() {
    use crate::dimensions::Dimension;
    let cold = mc_world::ChunkPos { x: 20, z: 20 };
    for dimension in [Dimension::Overworld, Dimension::Nether, Dimension::End] {
        for checkpoint_after_holds in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let mut drill = fixture(directory.path());
            let target = debt_dimension(&mut drill.state, dimension);
            target
                .world
                .ensure_blocking(&[(cold, 0)].into_iter().collect());
            target.tickets.insert("cold-debt-clock".into(), (cold, 0));
            drill.run(&[]).unwrap();
            let running_at_anchor = drill.state.tick_control.game_ticks(drill.state.tick);
            debt_dimension(&mut drill.state, dimension)
                .tickets
                .remove("cold-debt-clock");
            drill.state.tick_control.set_frozen(true);
            drill.run(&[]).unwrap();
            drill.checkpoint();
            let target = debt_dimension(&mut drill.state, dimension);
            let want = target.desired_chunks();
            target.world.evict_outside(&want);
            assert!(
                !target.world.has_chunk(cold.x, cold.z),
                "{dimension:?}: actual cold unload"
            );
            for _ in 0..2 {
                drill.run(&[]).unwrap();
            }
            drill.state.tick_control.set_frozen(false);
            for _ in 0..2 {
                drill.run(&[]).unwrap();
            }
            drill.state.tick_control.set_frozen(true);
            for _ in 0..2 {
                drill.run(&[]).unwrap();
            }
            drill.state.tick_control.set_frozen(false);
            drill.run(&[]).unwrap();
            drill.state.tick_control.set_frozen(true);
            drill.run(&[]).unwrap();
            if checkpoint_after_holds {
                drill.checkpoint();
            }
            for _ in 0..2 {
                drill.run(&[]).unwrap();
            }
            let owed = drill.state.tick_control.game_ticks(drill.state.tick) - running_at_anchor;
            assert_eq!(owed, 3);
            drill.writer.as_mut().unwrap().flush().unwrap();
            let config = drill.config.clone();
            drop(drill);
            let mut restored = Drill::open(config);
            restored.state.tick_control.set_frozen(true);
            let target = debt_dimension(&mut restored.state, dimension);
            target
                .world
                .ensure_blocking(&[(cold, 0)].into_iter().collect());
            target.tickets.insert("cold-debt-clock".into(), (cold, 0));
            restored.run(&[]).unwrap();
            let held = restored
                .state
                .tick_control
                .held_ticks_at(restored.state.tick);
            let tick = restored.state.tick;
            let target = debt_dimension(&mut restored.state, dimension);
            assert_eq!(
                target.world.simulation_debt(cold),
                owed,
                "{dimension:?}, checkpoint={checkpoint_after_holds}"
            );
            let image = target
                .world
                .debt_checkpoint(tick)
                .into_iter()
                .find(|image| image.chunk == [cold.x, cold.z])
                .unwrap();
            assert_eq!((image.last_sim, image.held_anchor), (tick, Some(held)));
            restored.state.tick_control.set_frozen(false);
            restored.run(&[]).unwrap();
            assert_eq!(
                debt_dimension(&mut restored.state, dimension)
                    .world
                    .simulation_debt(cold),
                0
            );
            restored.checkpoint();
            let target = debt_dimension(&mut restored.state, dimension);
            let mut anchors = target
                .world
                .chunk(cold)
                .unwrap()
                .extras
                .iter()
                .filter(|(key, _)| key == crate::debt::HELD_ANCHOR_KEY);
            assert_eq!(
                anchors.next().map(|(_, tag)| tag),
                Some(&mc_nbt::Tag::Long(held as i64))
            );
            assert!(
                anchors.next().is_none(),
                "two native checkpoints never duplicate the held key"
            );
        }
    }
}

#[test]
fn native_debt_game_clock_manifest_supersedes_old_anchorless_native_extras() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = fixture(directory.path());
    drill.state.tick_control.set_frozen(true);
    for _ in 0..3 {
        drill.run(&[]).unwrap();
    }
    drill.checkpoint();
    let tick = drill.state.tick;
    let base = drill.state.world.chunk(CHUNK).unwrap().clone();
    let mut old = base.clone();
    old.extras
        .retain(|(key, _)| key != crate::debt::HELD_ANCHOR_KEY && key != crate::debt::LAST_SIM_KEY);
    old.extras.push((
        crate::debt::LAST_SIM_KEY.into(),
        mc_nbt::Tag::Long(tick as i64),
    ));
    let store = fixture_store(&drill.config).unwrap();
    let checkpoint = store.read_checkpoint().unwrap();
    store
        .commit_checkpoint(
            mc_world::native::NativeCheckpoint {
                tick,
                previous_tick: tick,
                chunks: &[mc_world::native::NativeChunkSnapshot {
                    dimension: 0,
                    base: &base,
                    effective: &old,
                    dirty_sections: None,
                }],
                entities: &[],
                players: &[],
                globals: &checkpoint.globals,
            },
            &drill.state.registry.blocks,
            &drill.state.registry.items,
        )
        .unwrap();
    drop(store);
    let chest = drill.state.world.block_at(POS.x, POS.y, POS.z).unwrap();
    drill
        .writer
        .as_mut()
        .unwrap()
        .append(
            tick + 1,
            tick + 1,
            &[JournalEntry::Block {
                pos: [POS.x, POS.y, POS.z],
                state: chest,
            }],
        )
        .unwrap();
    drill.writer.as_mut().unwrap().flush().unwrap();
    let mut config = drill.config.clone();
    config.start_frozen = true;
    drop(drill);
    // The WAL block forces native chunk admission during startup. Its old
    // anchorless extras would fail the numerical held-gap check if consulted.
    let core = Core::open(config).unwrap();
    core.shutdown();
}
