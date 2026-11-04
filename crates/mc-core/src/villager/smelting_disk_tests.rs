use super::*;
use crate::CoreConfig;
use crate::dimensions::Dimension;

fn checkpoint(state: &mut TickState, config: &CoreConfig) {
    use crate::native_checkpoint::{encode_entities, encode_players, record, refresh_metadata};
    use mc_world::native::NativeCheckpoint;
    let store = crate::native_checkpoint::tests::fixture_store(config).unwrap();
    let tick = state.tick;
    let (players, captured, block_entities) = crate::core::checkpoint_data_for(state, tick);
    let entities =
        encode_entities(Dimension::Overworld, &captured, &state.registry.items, tick).unwrap();
    let players = encode_players(players.iter(), &state.registry.items, tick).unwrap();
    let capture = state.world.capture_dirty(&block_entities);
    let written = capture.written();
    let mut image = store.read_checkpoint().unwrap();
    let mut manifest: crate::journal::CheckpointManifest =
        serde_json::from_slice(&image.globals["checkpoint"].payload).unwrap();
    manifest.tick = tick;
    for child in manifest.dimensions.values_mut() {
        child.tick = tick;
    }
    image
        .globals
        .extend(crate::settlements::persistence::checkpoint_rows(state, tick).unwrap());
    image.globals.insert(
        "plugins".into(),
        record(tick, serde_json::to_vec(&state.plugins).unwrap()),
    );
    image.globals.insert(
        "checkpoint".into(),
        record(tick, serde_json::to_vec(&manifest).unwrap()),
    );
    refresh_metadata(&mut image.globals, &manifest).unwrap();
    for global in image.globals.values_mut() {
        global.captured_tick = tick;
    }
    store
        .commit_checkpoint_stream(
            NativeCheckpoint {
                tick,
                previous_tick: image.tick,
                chunks: &[],
                entities: &[],
                players: &players,
                globals: &image.globals,
            },
            |write| {
                capture
                    .visit_native(|snapshot| {
                        write(snapshot)
                            .map_err(|error| crate::config::CoreError::Storage(error.to_string()))
                    })
                    .map_err(|error| mc_world::native::NativeError::Source(error.to_string()))
            },
            entities.iter().map(Ok),
            &state.registry.blocks,
            &state.registry.items,
        )
        .unwrap();
    state.world.mark_flushed(&written);
}

fn activate(state: &mut TickState) {
    let chunk = crate::boundary::ChunkPos { x: 0, z: 0 };
    state
        .world
        .ensure_blocking(&[(chunk, 0)].into_iter().collect());
    state.tickets.insert("smelt-restart".into(), (chunk, 0));
    state.refresh_active_chunks(0);
}

fn disk_setup(config: &CoreConfig) -> (TickState, EntityId, EntityId) {
    let mut native = crate::core::recover_dimension_state(config, Dimension::Overworld).unwrap();
    crate::tick::run_tick(&mut native, &[]);
    let (mut state, worker, sheep) = setup();
    state.world = native.world;
    activate(&mut state);
    for x in 0..16 {
        for z in 0..16 {
            for y in 190..206 {
                state.world.set_block(
                    x,
                    y,
                    z,
                    if y <= 200 {
                        state.registry.states.stone
                    } else {
                        state.registry.states.air
                    },
                );
            }
        }
    }
    state.world.set_block(
        CHEST.0,
        CHEST.1,
        CHEST.2,
        mining_tests::block(&state, "chest"),
    );
    state.world.set_block(
        FURNACE.0,
        FURNACE.1,
        FURNACE.2,
        mining_tests::block(&state, "furnace"),
    );
    state.world.fold_overlay();
    (state, worker, sheep)
}

#[test]
fn checkpoint_reopen_preserves_ore_mid_cook_and_ingots_in_delivery_carry() {
    for in_carry in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let config = CoreConfig::new(directory.path().to_path_buf(), 42);
        let (mut state, worker, sheep) = disk_setup(&config);
        let original = counts(&state);
        let raw = mining::item_id(&state, "raw_iron").unwrap();
        let iron = mining::item_id(&state, "iron_ingot").unwrap();
        let mut reached = false;
        for _ in 0..2_000 {
            crate::tick::run_tick(&mut state, &[]);
            let v = state
                .entities
                .row(state.entities.slot_of(worker).unwrap())
                .villager
                .as_ref()
                .unwrap();
            let f = &state.block_entities[&FURNACE];
            reached = if in_carry {
                v.smelting
                    .is_some_and(|task| task.phase == crate::villager::smelting::Phase::Deliver)
                    && v.inventory
                        .iter()
                        .flatten()
                        .any(|s| s.item_id == iron && s.count > 0)
            } else {
                f.slots[0].item_id == raw
                    && f.slots[0].count > 0
                    && f.furnace.as_ref().unwrap().cook_progress > 0
            };
            if reached {
                break;
            }
        }
        assert!(
            reached,
            "native scenario must reach restart phase carry={in_carry}"
        );
        assert_eq!(
            shear_yield(&state, sheep),
            None,
            "restart precedes the wool conversion"
        );
        let before = counts(&state);
        let before_furnace = state.block_entities[&FURNACE].clone();
        let worker_slot = state.entities.slot_of(worker).unwrap();
        let before_worker = crate::villager::persistence::journal_image(
            state.entities.row(worker_slot).villager.as_ref().unwrap(),
        );
        let uuid = state.entities.row(worker_slot).uuid;
        let registry = std::sync::Arc::clone(&state.registry);
        checkpoint(&mut state, &config);
        drop(state);
        let mut state =
            crate::core::recover_dimension_state(&config, Dimension::Overworld).unwrap();
        state.registry = registry;
        activate(&mut state);
        crate::tick::sync_chunk_block_entities(&mut state);
        assert_eq!(
            counts(&state),
            before,
            "no stock loss/duplication at checkpoint reopen"
        );
        assert_eq!(state.block_entities[&FURNACE].slots, before_furnace.slots);
        assert_eq!(
            state.block_entities[&FURNACE].furnace,
            before_furnace.furnace
        );
        let slot = state.entities.slot_of_uuid(uuid).unwrap();
        let restored = state.entities.row(slot).villager.as_ref().unwrap();
        assert!(crate::villager::persistence::journal_eq(
            restored,
            &before_worker
        ));
        let mut generated_wool = None;
        for _ in 0..3_000 {
            crate::tick::run_tick(&mut state, &[]);
            if generated_wool.is_none() {
                generated_wool = shear_yield(&state, sheep);
            }
            if stored_wool(&state) > 0 {
                break;
            }
        }
        let wool = generated_wool.expect("native shearing creates a lot after reopening");
        assert_eq!(
            stored_wool(&state),
            wool,
            "restart preserves the whole sheared lot"
        );
        assert!(
            wool > 0,
            "reopened tender must deliver and allow shepherd to finish"
        );
        let mut expected = original;
        for (name, delta) in [
            ("raw_iron", -2),
            ("coal", -1),
            ("shears", 1),
            ("white_wool", wool),
        ] {
            *expected
                .entry(mining::item_id(&state, name).unwrap())
                .or_default() += delta;
        }
        expected.retain(|_, count| *count > 0);
        assert_eq!(
            counts(&state),
            expected,
            "restart conservation includes all carry, drops and furnace"
        );
        assert_eq!(
            crate::tick::wool_of(
                &state
                    .entities
                    .row(state.entities.slot_of(sheep).unwrap())
                    .appearance
            ),
            0x10
        );
    }
}

#[test]
fn furnace_phase_round_trips_nbt_and_changes_the_worker_journal() {
    use crate::villager::{persistence, smelting};
    let (state, worker, _) = setup();
    let slot = state.entities.slot_of(worker).unwrap();
    let mut resident =
        persistence::journal_image(state.entities.row(slot).villager.as_ref().unwrap());
    let before = persistence::journal_image(&resident);
    resident.smelting = Some(smelting::Task {
        furnace: [FURNACE.0, FURNACE.1, FURNACE.2],
        phase: smelting::Phase::Deliver,
        carried: Default::default(),
    });
    assert!(!persistence::journal_eq(&resident, &before));
    let fields =
        persistence::villager_to_fields(&state.registry.items, &resident, state.tick).unwrap();
    let loaded =
        persistence::villager_from_fields(&state.registry.items, &fields, state.tick).unwrap();
    assert_eq!(loaded.smelting, resident.smelting);
    assert!(persistence::journal_eq(&loaded, &resident));
}

#[path = "smelting_provisioning_disk_tests.rs"]
mod provisioning;
