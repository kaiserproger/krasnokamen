use super::*;
use crate::settlements::allocator::tests::block;
use crate::settlements::construction::tests::{attempt, complete, fixture, fund, open_plan};
use crate::settlements::{SettlementId, blueprints, construction, layout};
use std::sync::Arc;

struct ForestGenerator {
    air: u32,
    stone: u32,
    chest: u32,
    log: u32,
    leaves: u32,
    roots: Vec<[i32; 3]>,
}

impl mc_world::ChunkGenerator for ForestGenerator {
    fn generate(&self, pos: ChunkPos) -> Chunk {
        let mut chunk = Chunk::empty_with_geometry(
            pos,
            BlockStateId(self.air),
            mc_data::Identifier::parse("minecraft:plains").unwrap(),
            mc_world::OVERWORLD_GEOMETRY,
        );
        for x in 0..16 {
            for z in 0..16 {
                chunk.set_block(x, 200, z, BlockStateId(self.stone));
            }
        }
        let mut set = |at: [i32; 3], block| {
            if at[0].div_euclid(16) == pos.x && at[2].div_euclid(16) == pos.z {
                chunk.set_block(
                    at[0].rem_euclid(16) as u8,
                    at[1],
                    at[2].rem_euclid(16) as u8,
                    BlockStateId(block),
                );
            }
        };
        set([1, 201, 1], self.chest);
        for &[x, y, z] in &self.roots {
            for above in y..y + 4 {
                set([x, above, z], self.log);
            }
            for dx in -1..=1 {
                for dz in -1..=1 {
                    set([x + dx, y + 4, z + dz], self.leaves);
                }
            }
        }
        chunk
    }
}

fn forest() -> (
    TickState,
    SettlementId,
    crate::entities::EntitySlot,
    construction::Project,
    [i32; 3],
    [i32; 3],
) {
    let (mut state, id, builder) = fixture();
    open_plan(&mut state);
    let record = state.settlements.get(id).unwrap();
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let mut network = layout::Network::walk(&state, record);
    let lot = network
        .frontage()
        .into_iter()
        .find_map(|(at, normal)| {
            let lot = network.lot(blueprint, at, normal)?;
            let size = blueprint.footprint(blueprints::Rotation::from_quarter_turns(lot.rotation));
            construction::site_free(&state, record, lot.origin, size).then_some(lot)
        })
        .unwrap();
    let project = construction::Project {
        settlement: id,
        blueprint: "storage".into(),
        level: 1,
        origin: lot.origin,
        rotation: lot.rotation,
        building: None,
        structure: String::new(),
        ordered: true,
        borrowed_builder: None,
    };
    let root = [lot.origin[0] + 2, 201, lot.origin[2] + 2];
    let corridor = network.construction_access(&project).unwrap();
    let size = blueprint.footprint(blueprints::Rotation::from_quarter_turns(lot.rotation));
    let access = corridor
        .into_iter()
        .rev()
        .find(|at| {
            (!(lot.origin[0]..lot.origin[0] + i32::from(size[0])).contains(&at[0])
                || !(lot.origin[2]..lot.origin[2] + i32::from(size[2])).contains(&at[2]))
                && (at[0], at[2]) != (1, 1)
        })
        .map(|[x, y, z]| [x, y + 1, z])
        .unwrap();
    grown_world(&mut state, vec![root, access], "birch");
    (state, id, builder, project, root, access)
}

/// A grown-tree image whose own deterministic generator agrees with it, so
/// the planted-root veto (not a generator mismatch) must provide protection.
pub(crate) fn grown_world(state: &mut TickState, roots: Vec<[i32; 3]>, stem: &str) {
    let generator = Arc::new(ForestGenerator {
        air: state.registry.states.air,
        stone: state.registry.states.stone,
        chest: block(state, "minecraft:chest", &[]),
        log: block(state, &format!("minecraft:{stem}_log"), &[]),
        leaves: block(state, &format!("minecraft:{stem}_leaves"), &[]),
        roots,
    });
    state.world = crate::world::ChunkWorld::new(
        42,
        generator,
        &state.registry,
        BlockStateId(state.registry.states.air),
        [8, 201, 8],
        mc_world::OVERWORLD_GEOMETRY,
    );
    state
        .world
        .ensure_blocking(&rustc_hash::FxHashMap::from_iter(
            (-1..=1).flat_map(|x| (-1..=1).map(move |z| (ChunkPos { x, z }, 0))),
        ));
    state.settlements.forest = Default::default();
    for x in -1..=1 {
        for z in -1..=1 {
            warm(state, [x * 16, 201, z * 16]);
        }
    }
}

// Wait on the actual regeneration result, never wall-clock sleeps or polling.
fn warm(state: &TickState, at: [i32; 3]) {
    let mut cache = state.settlements.forest.lock().unwrap();
    let started = std::time::Instant::now();
    while cache.block(state, at).is_none() {
        let (pos, chunk, micros) = cache.worker.as_ref().unwrap().1.recv().unwrap();
        cache.pending.remove(&pos);
        let evidence = chunk.expect("trusted forest fixture");
        cache.bytes += evidence.bytes();
        cache.ready.insert(pos, evidence);
        cache.order.push_back(pos);
        eprintln!("forest evidence chunk ({}, {}): {micros} us", pos.x, pos.z);
    }
    eprintln!(
        "forest lot evidence wait: {} us",
        started.elapsed().as_micros()
    );
}

fn count(state: &TickState, name: &str) -> i32 {
    let item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(name).unwrap())
        .unwrap();
    state.block_entities[&(1, 201, 1)]
        .slots
        .iter()
        .filter(|stack| stack.item_id == item)
        .map(|stack| stack.count)
        .sum()
}

#[test]
fn natural_forest_lot_and_access_are_felled_into_storage_before_construction() {
    let (mut state, id, builder, expected, root, access) = forest();
    assert!(clearable(&state, root));
    fund(&mut state, "storage", 1);
    let before = state.block_entities[&(1, 201, 1)].slots.clone();
    let log = state.world.block_at(root[0], root[1], root[2]).unwrap();
    construction::request_build(&mut state, id, "storage").unwrap();
    let project = state.settlements.get(id).unwrap().project.as_ref().unwrap();
    assert_eq!(project.origin, expected.origin);
    assert_eq!(
        state.block_entities[&(1, 201, 1)].slots,
        before,
        "admission does not debit"
    );
    assert_eq!(
        state.world.block_at(root[0], root[1], root[2]),
        Some(log),
        "admission never clears"
    );
    let old = count(&state, "minecraft:birch_log");
    for _ in 0..4096 {
        let project = state.settlements.get(id).unwrap().project.as_ref().unwrap();
        if crate::plugin_structures::settlement_builds::next_material(
            &state,
            id,
            &project.structure,
        )
        .unwrap()
        .is_some()
        {
            break;
        }
        let next =
            crate::plugin_structures::settlement_builds::next_cell(&state, id, &project.structure)
                .unwrap()
                .unwrap();
        let previous = state.world.block_at(next[0], next[1], next[2]).unwrap();
        let mut expected_stock: std::collections::BTreeMap<u32, i32> =
            std::collections::BTreeMap::new();
        for stack in state.block_entities[&(1, 201, 1)]
            .slots
            .iter()
            .filter(|stack| !stack.is_empty())
        {
            *expected_stock.entry(stack.item_id).or_default() += stack.count;
        }
        if tree(&state, previous) {
            state.tick += 20;
            for stack in drops(&state, next, previous).unwrap() {
                *expected_stock.entry(stack.item_id).or_default() += stack.count;
            }
            state.tick -= 20;
        }
        attempt(&mut state, id, builder);
        let mut actual_stock: std::collections::BTreeMap<u32, i32> =
            std::collections::BTreeMap::new();
        for stack in state.block_entities[&(1, 201, 1)]
            .slots
            .iter()
            .filter(|stack| !stack.is_empty())
        {
            *actual_stock.entry(stack.item_id).or_default() += stack.count;
        }
        assert_eq!(
            actual_stock, expected_stock,
            "every vanilla drop conserved at {next:?}"
        );
    }
    assert_eq!(
        count(&state, "minecraft:birch_log") - old,
        8,
        "both four-log trunks enter real storage"
    );
    for [x, y, z] in [root, access] {
        for above in y..y + 4 {
            assert_eq!(
                state.world.block_at(x, above, z),
                Some(state.registry.states.air)
            );
        }
    }
    complete(&mut state, id, builder);
    assert!(state.settlements.get(id).unwrap().project.is_none());
    let building = state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .values()
        .find(|building| building.kind == crate::settlements::buildings::BuildingKind::Storage)
        .unwrap();
    assert!(
        construction::upgrade_available(&state, id, building.id),
        "the authored lot survives a larger preparation fence"
    );
    assert!(
        !crate::plugin_structures::settlement_builds::area_reserved(
            &state,
            mc_script::ScriptWorldBounds::new(access, access).unwrap()
        ),
        "finished clearing does not permanently reserve the corridor"
    );
}

#[test]
fn updated_leaf_distance_remains_clearable_without_ignoring_other_properties() {
    let (mut state, id, builder, _, root, _) = forest();
    let at = [root[0], root[1] + 4, root[2]];
    let original = state.world.block_at(at[0], at[1], at[2]).unwrap();
    let pos = crate::boundary::BlockPos {
        x: at[0],
        y: at[1],
        z: at[2],
    };
    crate::leaves::schedule(&mut state, pos, original);
    let active = state.world.chunks().map(|chunk| chunk.pos).collect();
    state.world.set_active_chunks(active, state.tick);
    for _ in 0..4 {
        crate::tick::run_tick(&mut state, &[]);
    }
    let updated = state.world.block_at(at[0], at[1], at[2]).unwrap();
    assert_ne!(
        original, updated,
        "the real scheduled leaf tick changed distance"
    );
    assert!(clearable(&state, at));
    let persistent =
        crate::farming::with_prop(&state.registry.blocks, updated, "persistent", "true").unwrap();
    state.world.set_block(at[0], at[1], at[2], persistent);
    assert!(
        !clearable(&state, at),
        "persistent player leaves are not natural"
    );
    state.world.set_block(at[0], at[1], at[2], updated);
    fund(&mut state, "storage", 1);
    construction::request_build(&mut state, id, "storage").unwrap();
    complete(&mut state, id, builder);
    assert!(state.settlements.get(id).unwrap().project.is_none());
}

#[test]
fn vanished_planted_tree_releases_old_crown_after_replant_elsewhere() {
    let (mut state, id, _, _, root, _) = forest();
    let log = state.world.block_at(root[0], root[1], root[2]).unwrap();
    for y in root[1]..root[1] + 4 {
        state
            .world
            .set_block(root[0], y, root[2], state.registry.states.air);
    }
    let sapling = block(&state, "minecraft:birch_sapling", &[]);
    state.world.set_block(root[0], root[1], root[2], sapling);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    assert!(planted(&state, root), "the recorded sapling is protected");
    for y in root[1]..root[1] + 4 {
        state.world.set_block(root[0], y, root[2], log);
    }
    let crown = [root[0] + 1, root[1] + 4, root[2]];
    assert!(!clearable(&state, crown));
    for y in root[1]..root[1] + 4 {
        state
            .world
            .set_block(root[0], y, root[2], state.registry.states.air);
    }
    state.tick = 20;
    retire_missing(&mut state);
    assert!(
        !state
            .settlements
            .get(id)
            .unwrap()
            .planted_trees
            .contains(&root)
    );
    let new_root = [30, 201, 30];
    let sapling = block(&state, "minecraft:birch_sapling", &[]);
    state
        .world
        .set_block(new_root[0], new_root[1], new_root[2], sapling);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(new_root);
    assert!(planted(&state, new_root));
    assert!(clearable(&state, crown));
}

#[test]
fn planted_tree_does_not_reserve_a_separate_nearby_natural_trunk() {
    let (mut state, id, _, _, _, _) = forest();
    let root = [0, 201, 0];
    let neighbor = [6, 201, 0];
    grown_world(&mut state, vec![root, neighbor], "birch");
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    assert!(!clearable(&state, root));
    assert!(clearable(&state, neighbor));
}

#[test]
fn forest_evidence_evicts_before_exceeding_payload_budget() {
    let (state, _, _, _, _, _) = forest();
    let (requests, _queued) = mpsc::sync_channel(2);
    let (results, received) = mpsc::sync_channel(2);
    let first = ChunkPos { x: 0, z: 0 };
    let second = ChunkPos { x: 1, z: 0 };
    let evidence = || Evidence {
        min_y: -64,
        max_y: 320,
        cells: Vec::with_capacity(CACHE_BYTES / 16 + 16),
    };
    results.send((first, Some(evidence()), 0)).unwrap();
    results.send((second, Some(evidence()), 0)).unwrap();
    let mut cache = Cache {
        worker: Some((requests, received)),
        trusted: Some(true),
        ..Default::default()
    };
    assert_eq!(
        cache.block(&state, [16, 80, 0]),
        Some(state.registry.states.air)
    );
    assert!(!cache.ready.contains_key(&first));
    assert!(cache.ready.contains_key(&second));
    assert!(cache.bytes <= CACHE_BYTES);
}

#[test]
fn owner_order_waits_for_delayed_forest_evidence_then_starts() {
    let (mut state, id, _, project, _, _) = forest();
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let size = blueprint.footprint(blueprints::Rotation::from_quarter_turns(project.rotation));
    let anchor = state.settlements.get(id).unwrap().anchor;
    for x in -16..32 {
        for z in -16..32 {
            let selected = (project.origin[0]..project.origin[0] + i32::from(size[0])).contains(&x)
                && (project.origin[2]..project.origin[2] + i32::from(size[2])).contains(&z);
            if !selected && layout::cell(anchor, x, z) == layout::Cell::Lot && (x, z) != (1, 1) {
                for y in 201..=216 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    fund(&mut state, "storage", 1);
    let (requests, queued) = mpsc::sync_channel(QUEUED_CHUNKS);
    let (results, received) = mpsc::sync_channel(QUEUED_CHUNKS);
    state.settlements.forest = Default::default();
    state.settlements.forest.lock().unwrap().worker = Some((requests, received));
    assert!(matches!(
        construction::request_build(&mut state, id, "storage").unwrap(),
        construction::OrderOutcome::Queued { .. }
    ));
    assert!(state.settlements.get(id).unwrap().pending_order.is_some());
    let active = state.world.chunks().map(|chunk| chunk.pos).collect();
    state.world.set_active_chunks(active, state.tick);
    let generator = state.world.generator();
    for _ in 0..512 {
        while let Ok(pos) = queued.try_recv() {
            let evidence = Evidence::from_chunk(
                &generator.generate(pos),
                &tree_states(&state.registry.blocks),
            );
            results.send((pos, Some(evidence), 0)).unwrap();
        }
        state.tick += 4;
        construction::tick(&mut state);
        if state.settlements.get(id).unwrap().project.is_some() {
            break;
        }
    }
    let record = state.settlements.get(id).unwrap();
    assert!(record.pending_order.is_none());
    assert_eq!(record.project.as_ref().unwrap().origin, project.origin);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "manual cold/warm worldgen cache measurement"]
fn measure_seed24_forest_cache() {
    let directory = tempfile::tempdir().unwrap();
    let config = crate::CoreConfig::new(directory.path().to_path_buf(), 24);
    crate::Core::open(config.clone()).unwrap().shutdown();
    let state =
        crate::core::recover_dimension_state(&config, crate::dimensions::Dimension::Overworld)
            .unwrap();
    let rss = || -> u64 {
        std::fs::read_to_string("/proc/self/status")
            .unwrap()
            .lines()
            .find_map(|line| {
                line.strip_prefix("VmRSS:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
            .unwrap()
            * 1024
    };
    let before = rss();
    let started = std::time::Instant::now();
    for x in 7..11 {
        for z in -15..-11 {
            warm(&state, [x * 16, 80, z * 16]);
        }
    }
    let cold = started.elapsed();
    let after = rss();
    let started = std::time::Instant::now();
    for x in 7..11 {
        for z in -15..-11 {
            warm(&state, [x * 16, 80, z * 16]);
        }
    }
    let cache_bytes = state.settlements.forest.lock().unwrap().bytes;
    eprintln!(
        "seed24 16 chunks: cold={cold:?} warm={:?} rss_delta={} bytes evidence={cache_bytes} bytes",
        started.elapsed(),
        after.saturating_sub(before)
    );
}

#[test]
fn real_core_reopen_trusts_native_contract_and_rejects_imported_chunk() {
    let directory = tempfile::tempdir().unwrap();
    let config = crate::CoreConfig::new(directory.path().to_path_buf(), 24);
    crate::Core::open(config.clone()).unwrap().shutdown();
    crate::Core::open(config.clone()).unwrap().shutdown();
    let state =
        crate::core::recover_dimension_state(&config, crate::dimensions::Dimension::Overworld)
            .unwrap();
    assert!(
        generator_known(&state),
        "native level.dat epoch must not disable clearing"
    );
    let source = state.world.chunk_source();
    let kinds = tree_states(&state.registry.blocks);
    let mut sites = Vec::new();
    for x in 7..11 {
        for z in -15..-11 {
            let pos = ChunkPos { x, z };
            let chunk = source.generator.generate(pos);
            let evidence = Evidence::from_chunk(&chunk, &kinds);
            if let Some(&(index, _)) = evidence.cells.first() {
                let at = [
                    x * 16 + ((index % 256) / 16) as i32,
                    evidence.min_y + (index / 256) as i32,
                    z * 16 + (index % 16) as i32,
                ];
                sites.push((pos, chunk, at));
                if sites.len() == 2 {
                    break;
                }
            }
        }
        if sites.len() == 2 {
            break;
        }
    }
    let (native_pos, _, native_at) = sites.pop().expect("second natural tree chunk");
    let (pos, imported, imported_at) = sites.pop().expect("imported natural tree chunk");
    source
        .native
        .as_ref()
        .unwrap()
        .import_chunk(0, &imported, &state.registry.blocks, &state.registry.items)
        .unwrap();
    assert!(
        !source
            .native
            .as_ref()
            .unwrap()
            .generator_owns_baseline(0, pos)
            .unwrap()
    );
    drop(source);
    drop(state);
    crate::Core::open(config.clone()).unwrap().shutdown();
    let mut state =
        crate::core::recover_dimension_state(&config, crate::dimensions::Dimension::Overworld)
            .unwrap();
    assert!(
        !state
            .world
            .chunk_source()
            .native
            .as_ref()
            .unwrap()
            .generator_owns_baseline(0, pos)
            .unwrap()
    );
    state
        .world
        .ensure_blocking(&[(pos, 0), (native_pos, 0)].into_iter().collect());
    assert!(!clearable(&state, imported_at));
    {
        let mut cache = state.settlements.forest.lock().unwrap();
        let (received, evidence, _) = cache.worker.as_ref().unwrap().1.recv().unwrap();
        assert_eq!(received, pos);
        assert!(
            evidence.is_none(),
            "imported baseline cannot authorize clearing"
        );
        cache.pending.remove(&received);
        cache.denied.insert(received);
    }
    warm(&state, native_at);
    assert!(
        clearable(&state, native_at),
        "real native world tree is clearable after reopen"
    );
    let record = state.native_globals.get_mut("generation").unwrap();
    let mut contract: serde_json::Value = serde_json::from_slice(&record.payload).unwrap();
    contract["base"]["revision"] = serde_json::json!(mc_worldgen::WORLDGEN_REVISION + 1);
    record.payload = serde_json::to_vec(&contract).unwrap();
    assert!(
        !generator_known(&state),
        "mismatched generation contract fails closed"
    );
}
#[test]
fn player_log_under_natural_canopy_is_protected_with_empty_or_expired_history() {
    let (mut state, _, _, _, root, _) = forest();
    let at = [root[0] + 1, root[1] + 3, root[2]];
    let log = block(&state, "minecraft:birch_log", &[]);
    state.world.set_block(at[0], at[1], at[2], log);
    assert!(
        !clearable(&state, at),
        "worldgen had air beneath this canopy"
    );
    state.block_log.evict(u64::MAX);
    assert!(
        !clearable(&state, at),
        "pruning cannot turn a nonmatching player block natural"
    );
    assert_eq!(state.world.block_at(at[0], at[1], at[2]), Some(log));
}

#[test]
fn identical_replacement_after_history_expiry_is_felled_and_conserved() {
    let (mut state, id, builder, _, root, _) = forest();
    let log = state.world.block_at(root[0], root[1], root[2]).unwrap();
    state
        .world
        .set_block(root[0], root[1], root[2], state.registry.states.air);
    state.world.set_block(root[0], root[1], root[2], log);
    state.block_log = crate::block_log::BlockLog::restore(
        crate::block_log::Settings {
            retention_secs: 1,
            ..Default::default()
        },
        vec![crate::block_log::Delta {
            records: vec![crate::block_log::Record {
                seq: 0,
                time: 0,
                actor: Arc::from("player"),
                pos: root,
                change: crate::block_log::Change::Block {
                    before: state.registry.states.air,
                    after: log,
                    entity: None,
                },
                rolled_back: false,
            }],
            marks: Vec::new(),
        }],
    );
    assert!(!state.block_log.player_touched(root));
    assert!(
        clearable(&state, root),
        "owner-approved indistinguishable historical replacement"
    );
    fund(&mut state, "storage", 1);
    construction::request_build(&mut state, id, "storage").unwrap();
    let before = count(&state, "minecraft:birch_log");
    for _ in 0..100 {
        if count(&state, "minecraft:birch_log") - before == 8 {
            break;
        }
        attempt(&mut state, id, builder);
    }
    assert_eq!(count(&state, "minecraft:birch_log") - before, 8);
    assert_eq!(
        state.world.block_at(root[0], root[1], root[2]),
        Some(state.registry.states.air)
    );
}

#[test]
fn retained_player_placement_vetoes_an_identical_worldgen_log() {
    let (mut state, _, _, _, root, _) = forest();
    let log = state.world.block_at(root[0], root[1], root[2]).unwrap();
    state.block_log = crate::block_log::BlockLog::restore(
        crate::block_log::Settings {
            retention_secs: 0,
            ..Default::default()
        },
        vec![crate::block_log::Delta {
            records: vec![crate::block_log::Record {
                seq: 0,
                time: 0,
                actor: Arc::from("player"),
                pos: root,
                change: crate::block_log::Change::Block {
                    before: state.registry.states.air,
                    after: log,
                    entity: None,
                },
                rolled_back: false,
            }],
            marks: Vec::new(),
        }],
    );
    assert!(!clearable(&state, root));
}

#[test]
fn grown_lumberjack_tree_stays_protected_after_settlement_checkpoint_recovery() {
    let (mut state, id, _, _, root, _) = forest();
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    let rows = crate::settlements::persistence::checkpoint_rows(&state, 1).unwrap();
    let (mut restored, _, _, _, grown, _) = forest();
    restored.settlements = Default::default();
    crate::settlements::persistence::hydrate_dimension(
        &mut restored,
        &rows,
        &Default::default(),
        id.0,
    )
    .unwrap();
    assert_eq!(root, grown);
    warm(&restored, grown);
    assert!(!clearable(&restored, grown));
    assert!(!clearable(
        &restored,
        [grown[0] + 1, grown[1] + 4, grown[2]]
    ));
}

#[test]
fn imported_world_without_matching_generator_identity_clears_nothing() {
    let (mut state, _, _, _, root, _) = forest();
    let templates =
        mc_nbt::Tag::Compound(vec![("level.dat".into(), mc_nbt::Tag::Compound(vec![]))]);
    state.native_globals.insert(
        "level_metadata".into(),
        crate::native_checkpoint::tag_record(0, &templates).unwrap(),
    );
    state.settlements.forest = Default::default();
    assert!(!generator_known(&state));
    assert!(!clearable(&state, root));
    assert!(
        state.settlements.forest.lock().unwrap().worker.is_none(),
        "unknown generator never regenerates as evidence"
    );
}

#[test]
fn full_storage_refuses_felling_without_losing_or_duplicating_the_log() {
    let (mut state, id, builder, _, _, _) = forest();
    fund(&mut state, "storage", 1);
    construction::request_build(&mut state, id, "storage").unwrap();
    let handle = state
        .settlements
        .get(id)
        .unwrap()
        .project
        .as_ref()
        .unwrap()
        .structure
        .clone();
    let (next, previous) = (0..64)
        .find_map(|_| {
            let next = crate::plugin_structures::settlement_builds::next_cell(&state, id, &handle)
                .unwrap()
                .unwrap();
            let previous = state.world.block_at(next[0], next[1], next[2]).unwrap();
            if state
                .registry
                .blocks
                .by_id(BlockStateId(previous))
                .unwrap()
                .block
                .id
                .path()
                .ends_with("_log")
            {
                Some((next, previous))
            } else {
                attempt(&mut state, id, builder);
                None
            }
        })
        .expect("the clearance contains a log with guaranteed vanilla loot");
    let stone = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap();
    state
        .block_entities
        .get_mut(&(1, 201, 1))
        .unwrap()
        .slots
        .fill(mc_data::ItemStack::new(stone, 64));
    let before = state.block_entities[&(1, 201, 1)].slots.clone();
    let uuid = state.entities.row(builder).uuid;
    assert_eq!(
        crate::plugin_structures::settlement_builds::advance(&mut state, id, &handle, uuid),
        Err(mc_script::ScriptOperationFailure::InsufficientItems)
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, before);
    assert_eq!(
        state.world.block_at(next[0], next[1], next[2]),
        Some(previous)
    );
    state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots[26] = mc_data::ItemStack::EMPTY;
    crate::plugin_structures::settlement_builds::advance(&mut state, id, &handle, uuid).unwrap();
    assert_eq!(
        state.world.block_at(next[0], next[1], next[2]),
        Some(state.registry.states.air)
    );
    assert_eq!(count(&state, "minecraft:birch_log"), 1);
}

#[test]
fn protected_crown_above_access_headroom_stays_while_the_building_finishes() {
    let (mut state, id, builder, _, _, access) = forest();
    let at = [access[0], access[1] + 4, access[2]];
    let leaves = state.world.block_at(at[0], at[1], at[2]).unwrap();
    state.block_log = crate::block_log::BlockLog::restore(
        crate::block_log::Settings {
            retention_secs: 0,
            ..Default::default()
        },
        vec![crate::block_log::Delta {
            records: vec![crate::block_log::Record {
                seq: 0,
                time: 0,
                actor: Arc::from("player"),
                pos: at,
                change: crate::block_log::Change::Block {
                    before: state.registry.states.air,
                    after: leaves,
                    entity: None,
                },
                rolled_back: false,
            }],
            marks: Vec::new(),
        }],
    );
    assert!(!clearable(&state, at));
    fund(&mut state, "storage", 1);
    construction::request_build(&mut state, id, "storage").unwrap();
    assert!(state.settlements.get(id).unwrap().project.is_some());
    complete(&mut state, id, builder);
    assert_eq!(state.world.block_at(at[0], at[1], at[2]), Some(leaves));
}

#[test]
fn disabled_block_history_protects_natural_forest_near_a_live_player() {
    let (mut state, _, slot, _, root, _) = forest();
    let entity = state.entities.body(slot).id;
    let connection = crate::boundary::ConnectionId(999);
    state
        .entities
        .set_kind(slot, crate::entities::EntityKind::Player { connection });
    state.entities.row_mut(slot).villager = None;
    state.players.insert(
        entity,
        crate::player::PlayerState::new(connection, crate::commands::GameMode::Survival),
    );
    state.entities.set_pos(slot, root.map(f64::from));
    assert!(!state.block_log.enabled());
    assert!(!clearable(&state, root));
    state.entities.set_pos(
        slot,
        [
            f64::from(root[0]) + 33.0,
            f64::from(root[1]),
            f64::from(root[2]),
        ],
    );
    assert!(clearable(&state, root));
}

fn checkpoint_forest(state: &mut TickState, config: &crate::CoreConfig) {
    use crate::native_checkpoint::{encode_entities, encode_players, record, refresh_metadata};
    use mc_world::native::NativeCheckpoint;
    let store = crate::native_checkpoint::tests::fixture_store(config).unwrap();
    let tick = state.tick;
    let (players, captured, block_entities) = crate::core::checkpoint_data_for(state, tick);
    let entities = encode_entities(
        crate::dimensions::Dimension::Overworld,
        &captured,
        &state.registry.items,
        tick,
    )
    .unwrap();
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

// Drive only navigation; construction itself advances in run_tick.
fn guide_builder(state: &mut TickState, slot: crate::entities::EntitySlot) {
    let mut villager = state.entities.row_mut(slot).villager.take().unwrap();
    let target =
        crate::villager::building::work(state, slot, &mut villager, &mut Default::default());
    state.entities.row_mut(slot).villager = Some(villager);
    if let Some(target) = target {
        let pos = target.pos;
        let z = if target.exact { pos.z } else { pos.z + 1 };
        let y = if state
            .world
            .block_at(pos.x, pos.y, z)
            .is_some_and(|id| id != state.registry.states.air)
        {
            pos.y + 1
        } else {
            pos.y
        };
        state.entities.set_pos(
            slot,
            [f64::from(pos.x) + 0.5, f64::from(y), f64::from(z) + 0.5],
        );
    }
}

#[test]
fn forest_work_recovers_checkpoint_and_sealed_wal_without_duplicate_drops() {
    let directory = tempfile::tempdir().unwrap();
    let config = crate::CoreConfig::new(directory.path().to_path_buf(), 42);
    let (mut state, id, builder, _, _, access) = forest();
    let generator = state.world.generator();
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    state.world = crate::world::ChunkWorld::new_native(
        42,
        store,
        0,
        Arc::clone(&generator),
        &state.registry,
        BlockStateId(state.registry.states.air),
        [8, 201, 8],
        mc_world::OVERWORLD_GEOMETRY,
    );
    let wanted = rustc_hash::FxHashMap::from_iter(
        (-1..=1).flat_map(|x| (-1..=1).map(move |z| (ChunkPos { x, z }, 0))),
    );
    state.world.ensure_blocking(&wanted);
    fund(&mut state, "storage", 1);
    construction::request_build(&mut state, id, "storage").unwrap();
    let builder_uuid = state.entities.row(builder).uuid;
    state.entities.row_mut(builder).controls.no_ai = true;
    let baseline = count(&state, "minecraft:birch_log");
    let chest = state.world.block_at(1, 201, 1).unwrap();
    state.world.set_block(1, 201, 1, state.registry.states.air);
    state.world.set_block(1, 201, 1, chest);
    state
        .world
        .mark_block_entity_dirty(crate::boundary::BlockPos { x: 1, y: 201, z: 1 });
    for pos in wanted.keys().copied() {
        state
            .tickets
            .insert(format!("forest-{}-{}", pos.x, pos.z), (pos, 0));
    }
    state.refresh_active_chunks(state.tick);
    let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
    let _ = crate::core::journal_entries_for_tick(&mut state, &output);
    checkpoint_forest(&mut state, &config);
    let mut writer = crate::journal::JournalWriter::open(directory.path(), 1).unwrap();
    let mut previous_tick = state.tick;
    for _ in 0..2000 {
        if count(&state, "minecraft:birch_log") >= baseline + 2 {
            break;
        }
        guide_builder(&mut state, builder);
        let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
        let entries = crate::core::journal_entries_for_tick(&mut state, &output);
        writer
            .append(previous_tick + 1, output.tick, &entries)
            .unwrap();
        previous_tick = output.tick;
    }
    let checkpoint_stock = count(&state, "minecraft:birch_log");
    assert!(checkpoint_stock >= baseline + 2 && checkpoint_stock < baseline + 8);
    writer.flush().unwrap();
    checkpoint_forest(&mut state, &config);
    let checkpoint_tick = state.tick;
    let checkpoint_cursor = crate::plugin_structures::settlement_builds::status(
        &state,
        id,
        &state
            .settlements
            .get(id)
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .structure,
    )
    .unwrap()
    .completed_work_units;
    let mut unsealed = None;
    for _ in 0..2000 {
        guide_builder(&mut state, builder);
        let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
        let entries = crate::core::journal_entries_for_tick(&mut state, &output);
        if count(&state, "minecraft:birch_log") > checkpoint_stock {
            unsealed = Some((output.tick, entries));
            break;
        }
        writer
            .append(previous_tick + 1, output.tick, &entries)
            .unwrap();
        previous_tick = output.tick;
    }
    let (tail_tick, entries) = unsealed.expect("another physical log was felled");
    let mut before =
        crate::core::recover_dimension_state(&config, crate::dimensions::Dimension::Overworld)
            .unwrap();
    let wanted = rustc_hash::FxHashMap::from_iter(
        (-1..=1).flat_map(|x| (-1..=1).map(move |z| (ChunkPos { x, z }, 0))),
    );
    before.world.ensure_blocking(&wanted);
    crate::tick::sync_chunk_block_entities(&mut before);
    assert!(
        before.block_entities.contains_key(&(1, 201, 1)),
        "chest block={:?} loaded={}",
        before.world.block_at(1, 201, 1),
        before.world.has_chunk(0, 0)
    );
    assert_eq!(
        count(&before, "minecraft:birch_log"),
        checkpoint_stock,
        "unsealed cut cannot credit storage"
    );
    assert!(before.tick >= checkpoint_tick && before.tick < tail_tick);
    let felled: Vec<_> = entries
        .iter()
        .filter_map(|entry| match entry {
            crate::journal::JournalEntry::Block { pos, state }
                if *state == before.registry.states.air
                    && before
                        .world
                        .block_at(pos[0], pos[1], pos[2])
                        .is_some_and(|id| tree(&before, id)) =>
            {
                Some(*pos)
            }
            _ => None,
        })
        .collect();
    assert!(
        !felled.is_empty(),
        "the unsealed batch contains the physical tree cut"
    );
    let before_project = before
        .settlements
        .get(id)
        .unwrap()
        .project
        .as_ref()
        .unwrap();
    let before_cursor =
        crate::plugin_structures::settlement_builds::status(&before, id, &before_project.structure)
            .unwrap()
            .completed_work_units;
    drop(before);
    writer
        .append(previous_tick + 1, tail_tick, &entries)
        .unwrap();
    writer.flush().unwrap();
    let expected_stock = count(&state, "minecraft:birch_log");
    let expected_cursor = crate::plugin_structures::settlement_builds::status(
        &state,
        id,
        &state
            .settlements
            .get(id)
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .structure,
    )
    .unwrap()
    .completed_work_units;
    assert!(expected_cursor > checkpoint_cursor);
    assert!(
        before_cursor < expected_cursor,
        "the unsealed cut cannot advance the cursor"
    );
    drop(writer);
    drop(state);
    let mut restored =
        crate::core::recover_dimension_state(&config, crate::dimensions::Dimension::Overworld)
            .unwrap();
    let wanted = rustc_hash::FxHashMap::from_iter(
        (-1..=1).flat_map(|x| (-1..=1).map(move |z| (ChunkPos { x, z }, 0))),
    );
    restored.world.ensure_blocking(&wanted);
    crate::tick::sync_chunk_block_entities(&mut restored);
    for pos in wanted.keys().copied() {
        restored
            .tickets
            .insert(format!("forest-{}-{}", pos.x, pos.z), (pos, 0));
    }
    restored.refresh_active_chunks(restored.tick);
    // The disk fixture uses the synthetic generator above; production recovery
    // recreates the configured real generator. Supply that fixture's exact
    // evidence while exercising the real checkpoint and WAL owners.
    let mut cache = restored.settlements.forest.lock().unwrap();
    cache.trusted = Some(true);
    let kinds = tree_states(&restored.registry.blocks);
    for pos in wanted.keys().copied() {
        let evidence = Evidence::from_chunk(&generator.generate(pos), &kinds);
        cache.bytes += evidence.bytes();
        cache.ready.insert(pos, evidence);
        cache.order.push_back(pos);
    }
    drop(cache);
    assert_eq!(count(&restored, "minecraft:birch_log"), expected_stock);
    for pos in felled {
        assert_eq!(
            restored.world.block_at(pos[0], pos[1], pos[2]),
            Some(restored.registry.states.air)
        );
    }
    let project = restored
        .settlements
        .get(id)
        .unwrap()
        .project
        .as_ref()
        .unwrap();
    assert_eq!(
        crate::plugin_structures::settlement_builds::status(&restored, id, &project.structure)
            .unwrap()
            .completed_work_units,
        expected_cursor
    );
    let builder = restored.entities.slot_of_uuid(builder_uuid).unwrap();
    for _ in 0..10_000 {
        if restored.settlements.get(id).unwrap().project.is_none() {
            break;
        }
        if count(&restored, "minecraft:birch_log") == baseline + 8 {
            break;
        }
        guide_builder(&mut restored, builder);
        crate::tick::run_tick(&mut restored, &[]);
    }
    assert_eq!(count(&restored, "minecraft:birch_log"), baseline + 8);
    for _ in 0..36_000 {
        if restored.settlements.get(id).unwrap().project.is_none() {
            break;
        }
        guide_builder(&mut restored, builder);
        crate::tick::run_tick(&mut restored, &[]);
    }
    assert!(
        restored.settlements.get(id).unwrap().project.is_none(),
        "status={:?} day={} builder={:?}",
        construction::project_status(&restored, id),
        restored.day_time,
        restored.entities.row(builder).villager
    );
    assert_eq!(
        restored.world.block_at(access[0], access[1], access[2]),
        Some(restored.registry.states.air)
    );
}
