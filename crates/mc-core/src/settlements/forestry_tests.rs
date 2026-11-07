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
        cache.ready.insert(pos, chunk);
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
