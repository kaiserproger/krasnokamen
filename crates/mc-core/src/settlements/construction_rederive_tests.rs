//! Executor-half regressions for the D27 in-territory re-derivation of
//! preparation cells (PrepRederive). Every case drives the real
//! `settlement_builds::advance` path (stale check, addressed debit, one
//! custody commit, block write, cursor) on a world that carries the real
//! server registry, so loot authority is the vanilla tables, never the
//! lightweight registry. Assertions are exact: chest stock, builder carry,
//! dropped item entities, world blocks and the plugin-domain bytes.

use super::*;
use crate::journal::JournalWriter;
use mc_data::ItemStack;
use mc_script::{ScriptOperationFailure as Failure, ScriptStructureState};
use std::path::Path;
use std::sync::Arc;

const Y: i32 = 201;
const Z: i32 = 4;

/// Planned cell: portion, x (y and z are fixed), target block name, material.
type Spec = (
    executor::PortionKind,
    i32,
    &'static str,
    Option<&'static str>,
);

struct Scene {
    state: TickState,
    id: SettlementId,
    builder: uuid::Uuid,
    handle: String,
}

fn registry() -> Arc<crate::registry::CoreRegistry> {
    Arc::new(
        crate::registry::CoreRegistry::load(Some(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"),
        ))
        .unwrap(),
    )
}

fn named(state: &TickState, name: &str) -> u32 {
    crate::settlements::allocator::tests::block(state, name, &[])
}

fn item(state: &TickState, name: &str) -> u32 {
    state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(name).unwrap())
        .unwrap()
}

/// The settlement on `state` (a native-store world when `dir` is given) with
/// one admitted preparation structure over `specs`. `pre` is the world at
/// admission; `roots` are registered planted-tree roots.
fn scene(dir: Option<&Path>, pre: &[(i32, &str)], roots: &[i32], specs: &[Spec]) -> Scene {
    let (mut state, id, slot) = match dir {
        Some(dir) => native_fixture(dir),
        None => crate::settlements::construction::tests::fixture(),
    };
    state.registry = registry();
    let chest = named(&state, "minecraft:chest");
    state.world.set_block(1, Y, 1, chest);
    if dir.is_some() {
        state
            .world
            .mark_block_entity_dirty(crate::boundary::BlockPos { x: 1, y: Y, z: 1 });
    }
    let uuid = state.entities.row(slot).uuid;
    for (x, name) in pre {
        let block = named(&state, name);
        state.world.set_block(*x, Y, Z, block);
    }
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .extend(roots.iter().map(|x| [*x, Y, Z]));
    let min = specs.iter().map(|spec| spec.1).min().unwrap();
    let max = specs.iter().map(|spec| spec.1).max().unwrap();
    let bounds = mc_script::ScriptWorldBounds::new([min, Y, Z], [max, Y, Z]).unwrap();
    let cells = specs
        .iter()
        .map(|(portion, x, target, resource)| executor::PlannedCell {
            portion: *portion,
            at: [*x, Y, Z],
            state: named(&state, target),
            quantity: u32::from(resource.is_some()),
            resource: resource.map(str::to_owned),
        })
        .collect();
    let handle = executor::prepare(&mut state, id, "house".into(), bounds, bounds, cells).unwrap();
    state.settlements.get_mut(id).unwrap().projects = vec![Project {
        settlement: id,
        blueprint: "house".into(),
        level: 1,
        origin: bounds.min,
        rotation: 0,
        building: None,
        structure: handle.clone(),
        ordered: false,
        builder: Some((uuid, None)),
        order_seq: None,
    }];
    for (index, name) in ["minecraft:dirt", "minecraft:oak_log"]
        .into_iter()
        .enumerate()
    {
        let id = item(&state, name);
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .inventory[index] = Some(ItemStack::new(id, 16));
    }
    Scene {
        state,
        id,
        builder: uuid,
        handle,
    }
}

fn set(scene: &mut Scene, x: i32, name: &str) {
    let block = named(&scene.state, name);
    scene.state.world.set_block(x, Y, Z, block);
}

fn at(scene: &Scene, x: i32) -> Option<u32> {
    scene.state.world.block_at(x, Y, Z)
}

fn step(scene: &mut Scene) -> Result<bool, Failure> {
    executor::advance(&mut scene.state, scene.id, &scene.handle, scene.builder)
}

fn status(scene: &Scene) -> mc_script::ScriptStructureSnapshot {
    executor::status(&scene.state, scene.id, &scene.handle).unwrap()
}

fn stock(state: &TickState, name: &str) -> i32 {
    let wanted = item(state, name);
    state.block_entities[&(1, Y, 1)]
        .slots
        .iter()
        .filter(|stack| stack.item_id == wanted)
        .map(|stack| stack.count)
        .sum()
}

fn total_stock(state: &TickState) -> i32 {
    state.block_entities[&(1, Y, 1)]
        .slots
        .iter()
        .map(|stack| stack.count)
        .sum()
}

fn carried(scene: &Scene, name: &str) -> i32 {
    let wanted = item(&scene.state, name);
    let slot = scene.state.entities.slot_of_uuid(scene.builder).unwrap();
    scene
        .state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .inventory
        .iter()
        .flatten()
        .filter(|stack| stack.item_id == wanted)
        .map(|stack| stack.count)
        .sum()
}

/// Everything a refused cell must leave alone: the whole plugin domain, every
/// chest, every carry, the row of cells, planted roots and dropped entities.
fn image(scene: &Scene) -> serde_json::Value {
    let state = &scene.state;
    let carry: std::collections::BTreeMap<String, serde_json::Value> = state
        .entities
        .slots()
        .filter_map(|slot| {
            let row = state.entities.row(slot);
            let villager = row.villager.as_ref()?;
            Some((
                row.uuid.to_string(),
                serde_json::to_value(&villager.inventory).unwrap(),
            ))
        })
        .collect();
    let chests: std::collections::BTreeMap<String, serde_json::Value> = state
        .block_entities
        .iter()
        .map(|(key, entity)| {
            (
                format!("{key:?}"),
                serde_json::to_value(&entity.slots).unwrap(),
            )
        })
        .collect();
    let blocks: Vec<Option<u32>> = (0..16).map(|x| state.world.block_at(x, Y, Z)).collect();
    serde_json::json!({
        "plugins": serde_json::to_value(&state.plugins).unwrap(),
        "chests": chests,
        "carry": carry,
        "blocks": blocks,
        "trees": state.settlements.get(scene.id).unwrap().planted_trees,
        "revision": state.settlements.get(scene.id).unwrap().revision,
        "entities": state.entities.slots().count(),
    })
}

fn expect_refusal(scene: &mut Scene, failure: Failure, label: &str) {
    let before = image(scene);
    assert_eq!(step(scene), Err(failure), "{label}");
    assert_eq!(image(scene), before, "{label}: a refusal publishes nothing");
    let snapshot = status(scene);
    assert_eq!(snapshot.pause_reason, None, "{label}: no persisted pause");
    assert_ne!(snapshot.state, ScriptStructureState::Paused, "{label}");
}

use executor::PortionKind::{Build, Cut, Fill, Saplings, Trees};

// --- 1. a block changed after admission inside territory -------------------

/// (block that appears after admission, canonical drop, count): hand-droppable.
const CHANGED_HAND: [(&str, Option<&str>, i32); 4] = [
    ("minecraft:oak_planks", Some("minecraft:oak_planks"), 1),
    ("minecraft:dirt", Some("minecraft:dirt"), 1),
    ("minecraft:grass_block", Some("minecraft:dirt"), 1),
    ("minecraft:glass", None, 0),
];

/// The same, for blocks whose vanilla loot needs a suitable tool (D27 ruling:
/// settlements roll with their canonical extraction tool, no silk or fortune).
const CHANGED_TOOL: [(&str, Option<&str>, i32); 2] = [
    ("minecraft:stone", Some("minecraft:cobblestone"), 1),
    ("minecraft:cobblestone", Some("minecraft:cobblestone"), 1),
];

fn changed_cases(table: &[(&str, Option<&str>, i32)]) -> Vec<String> {
    let mut failures = Vec::new();
    for history in [false, true] {
        for (appeared, drop, count) in table {
            let (drop, count) = (*drop, *count);
            let mut s = scene(
                None,
                &[],
                &[],
                &[
                    (Build, 5, "minecraft:oak_log", Some("minecraft:oak_log")),
                    (Build, 6, "minecraft:oak_log", Some("minecraft:oak_log")),
                ],
            );
            set(&mut s, 5, appeared);
            if history {
                // D27: inside the territory the block's history is not consulted.
                s.state.block_log =
                    crate::settlements::forestry::tests::player_history(&s.state, [5, Y, Z], 0);
                assert!(s.state.block_log.player_touched([5, Y, Z]));
            }
            let log = named(&s.state, "minecraft:oak_log");
            let label = format!("{appeared} history={history}");
            let result = (|| -> Result<(), String> {
                step(&mut s).map_err(|e| format!("{label}: first cell {e:?}"))?;
                if at(&s, 5) != Some(log) {
                    return Err(format!("{label}: changed cell not replaced"));
                }
                let credited = drop.map_or(0, |name| stock(&s.state, name));
                if credited != count || total_stock(&s.state) != count {
                    return Err(format!(
                        "{label}: credited {credited} of {drop:?}, total {}",
                        total_stock(&s.state)
                    ));
                }
                step(&mut s).map_err(|e| format!("{label}: second cell {e:?}"))?;
                if total_stock(&s.state) != count {
                    return Err(format!("{label}: the unchanged cell paid"));
                }
                if status(&s).state != ScriptStructureState::Committed
                    || carried(&s, "minecraft:oak_log") != 14
                {
                    return Err(format!("{label}: bill or completion wrong"));
                }
                let finished = image(&s);
                if step(&mut s).is_ok() || image(&s) != finished {
                    return Err(format!("{label}: finished work paid or moved again"));
                }
                Ok(())
            })();
            if let Err(message) = result {
                failures.push(message);
            }
        }
    }
    failures
}

#[test]
fn changed_after_admission_inside_territory_credits_the_canonical_drop_exactly_once() {
    let failures = changed_cases(&CHANGED_HAND);
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn changed_tool_requiring_blocks_credit_the_drop_of_the_canonical_tool() {
    let failures = changed_cases(&CHANGED_TOOL);
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Three cells edited after admission and before the checkpoint; the live
/// run and every WAL restart (before and after each published batch) must end
/// with the same stock: each edit credited once, none lost, none doubled.
fn restart_credits_once(edits: [&str; 3], expect: &[(&str, i32)]) {
    let work = tempfile::tempdir().unwrap();
    let live = work.path().join("live");
    std::fs::create_dir_all(&live).unwrap();
    let config = CoreConfig::new(live.clone(), 42);
    let mut s = scene(
        Some(&live),
        &[],
        &[],
        &[
            (Build, 5, "minecraft:oak_log", Some("minecraft:oak_log")),
            (Build, 6, "minecraft:oak_log", Some("minecraft:oak_log")),
            (Build, 7, "minecraft:oak_log", Some("minecraft:oak_log")),
        ],
    );
    // The world changes after admission, before the durable checkpoint.
    for (x, name) in [5, 6, 7].into_iter().zip(edits) {
        set(&mut s, x, name);
    }
    s.state.tick_control.set_frozen(true);
    let (output, _, _) = crate::tick::run_tick(&mut s.state, &[]);
    crate::core::journal_entries_for_tick(&mut s.state, &output).unwrap();
    checkpoint(&mut s.state, &config);
    let mut writer = JournalWriter::open(&live, 1).unwrap();
    let mut copies = Vec::new();
    for index in 0..3 {
        let before = work.path().join(format!("before-{index}"));
        copy_world(&live, &before);
        step(&mut s).unwrap();
        journal(&mut s.state, &mut writer);
        let after = work.path().join(format!("after-{index}"));
        copy_world(&live, &after);
        copies.push((before, after));
    }
    drop(writer);
    let totals = |state: &TickState| -> Vec<i32> {
        let mut all: Vec<i32> = expect.iter().map(|(name, _)| stock(state, name)).collect();
        all.push(total_stock(state));
        all
    };
    let mut wanted: Vec<i32> = expect.iter().map(|(_, count)| *count).collect();
    wanted.push(expect.iter().map(|(_, count)| count).sum());
    assert_eq!(totals(&s.state), wanted, "live run");
    for (index, (before, after)) in copies.iter().enumerate() {
        for (label, dir) in [("unpublished", before), ("published", after)] {
            let mut restored = restore(&s, dir);
            let mut reached = status(&restored).watermark;
            let published = label == "published";
            assert_eq!(
                reached,
                index as u64 + u64::from(published),
                "{label} {index}"
            );
            while status(&restored).state != ScriptStructureState::Committed {
                step(&mut restored).unwrap();
                assert!(status(&restored).watermark > reached);
                reached = status(&restored).watermark;
            }
            assert_eq!(
                totals(&restored.state),
                wanted,
                "{label} {index}: no second credit, none lost"
            );
            assert_eq!(
                carried(&restored, "minecraft:oak_log"),
                13,
                "{label} {index}"
            );
            crate::plugin_structures::validate(&restored.state).unwrap();
        }
    }
}

#[test]
fn an_edited_cell_is_credited_once_across_checkpoint_wal_replay_and_restart() {
    restart_credits_once(
        [
            "minecraft:oak_planks",
            "minecraft:dirt",
            "minecraft:grass_block",
        ],
        &[("minecraft:oak_planks", 1), ("minecraft:dirt", 2)],
    );
}

#[test]
fn an_edited_tool_requiring_cell_is_credited_once_across_checkpoint_wal_replay_and_restart() {
    restart_credits_once(
        [
            "minecraft:stone",
            "minecraft:oak_planks",
            "minecraft:cobblestone",
        ],
        &[("minecraft:cobblestone", 2), ("minecraft:oak_planks", 1)],
    );
}

// --- 2. pre-existing non-terrain block / terrain under a placement ---------

/// (block present at admission and unchanged, canonical credit).
fn unchanged_under_placement(table: &[(&str, Option<&str>)]) -> Vec<String> {
    let mut failures = Vec::new();
    for (present, drop) in table {
        let (present, drop) = (*present, *drop);
        // The tested block is the second cell of the portion, so its evidence
        // is read at the right offset behind the cursor.
        let mut s = scene(
            None,
            &[(6, present)],
            &[],
            &[
                (Build, 5, "minecraft:oak_log", Some("minecraft:oak_log")),
                (Build, 6, "minecraft:oak_log", Some("minecraft:oak_log")),
                (Build, 7, "minecraft:oak_log", Some("minecraft:oak_log")),
            ],
        );
        let log = named(&s.state, "minecraft:oak_log");
        let label = format!("{present} under a placement");
        let result = (|| -> Result<(), String> {
            step(&mut s).map_err(|e| format!("{label}: first {e:?}"))?;
            if total_stock(&s.state) != 0 {
                return Err(format!("{label}: an air placement paid"));
            }
            step(&mut s).map_err(|e| format!("{label}: {e:?}"))?;
            if at(&s, 6) != Some(log) {
                return Err(format!("{label}: not replaced"));
            }
            let want = i32::from(drop.is_some());
            if drop.map_or(0, |name| stock(&s.state, name)) != want || total_stock(&s.state) != want
            {
                return Err(format!(
                    "{label}: expected {drop:?} x{want}, total stock {}",
                    total_stock(&s.state)
                ));
            }
            step(&mut s).map_err(|e| format!("{label}: third {e:?}"))?;
            if total_stock(&s.state) != want {
                return Err(format!("{label}: third cell paid"));
            }
            let done = image(&s);
            if step(&mut s).is_ok() || image(&s) != done {
                return Err(format!("{label}: paid twice"));
            }
            Ok(())
        })();
        if let Err(message) = result {
            failures.push(message);
        }
    }
    failures
}

#[test]
fn placement_over_a_preexisting_non_terrain_block_credits_one_drop() {
    let failures = unchanged_under_placement(&[
        ("minecraft:oak_planks", Some("minecraft:oak_planks")),
        ("minecraft:cobblestone", Some("minecraft:cobblestone")),
    ]);
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn placement_over_preexisting_terrain_credits_nothing() {
    let failures =
        unchanged_under_placement(&[("minecraft:dirt", None), ("minecraft:grass_block", None)]);
    assert!(failures.is_empty(), "{failures:#?}");
}

// --- 3. air pays zero, finished work never twice, a refusal publishes nothing

#[test]
fn already_air_cells_pay_zero_and_a_cleared_cell_is_never_recredited() {
    // Cut cell 5 is air at admission; cut cell 6 was stone and is mined away
    // by someone else before the builder arrives; cell 7 is stone and pays.
    let mut s = scene(
        None,
        &[(6, "minecraft:stone"), (7, "minecraft:stone")],
        &[],
        &[
            (Cut, 5, "minecraft:air", None),
            (Cut, 6, "minecraft:air", None),
            (Cut, 7, "minecraft:air", None),
            (Build, 8, "minecraft:oak_log", Some("minecraft:oak_log")),
        ],
    );
    set(&mut s, 6, "minecraft:air");
    let air = s.state.registry.states.air;
    step(&mut s).unwrap();
    assert_eq!(total_stock(&s.state), 0, "an air cell pays zero");
    step(&mut s).unwrap();
    assert_eq!(
        total_stock(&s.state),
        0,
        "a cell emptied after admission pays zero"
    );
    assert_eq!(at(&s, 6), Some(air));
    step(&mut s).unwrap();
    assert_eq!(stock(&s.state, "minecraft:cobblestone"), 1);
    assert_eq!(total_stock(&s.state), 1);
    // A player places stone back into the finished cut cells: the cursor is
    // past them, so neither payment nor rewrite happens again.
    set(&mut s, 5, "minecraft:stone");
    set(&mut s, 7, "minecraft:stone");
    let stone = named(&s.state, "minecraft:stone");
    step(&mut s).unwrap();
    assert_eq!(total_stock(&s.state), 1, "finished work is not paid twice");
    assert_eq!(at(&s, 5), Some(stone));
    assert_eq!(at(&s, 7), Some(stone));
    assert_eq!(status(&s).state, ScriptStructureState::Committed);
    let done = image(&s);
    assert!(step(&mut s).is_err());
    assert_eq!(image(&s), done);
}

fn one_log_cell() -> Scene {
    scene(
        None,
        &[],
        &[],
        &[(Build, 5, "minecraft:oak_log", Some("minecraft:oak_log"))],
    )
}

#[test]
fn a_refused_cell_publishes_nothing_when_storage_is_full() {
    // The live drop (planks) has nowhere to go: the block is not destroyed.
    let mut s = one_log_cell();
    set(&mut s, 5, "minecraft:oak_planks");
    let sticks = ItemStack::new(item(&s.state, "minecraft:stick"), 64);
    for slot in &mut s.state.block_entities.get_mut(&(1, Y, 1)).unwrap().slots {
        *slot = sticks.clone();
    }
    expect_refusal(&mut s, Failure::InsufficientItems, "full storage");
    assert_eq!(at(&s, 5), Some(named(&s.state, "minecraft:oak_planks")));
    // Room again: the very same cell then proceeds with exactly one credit.
    for slot in &mut s.state.block_entities.get_mut(&(1, Y, 1)).unwrap().slots {
        *slot = ItemStack::EMPTY;
    }
    step(&mut s).unwrap();
    assert_eq!(stock(&s.state, "minecraft:oak_planks"), 1);
    assert_eq!(total_stock(&s.state), 1);
}

#[test]
fn a_refused_cell_publishes_nothing_when_the_material_is_missing() {
    // The credit must not be published without the debit.
    let mut s = one_log_cell();
    set(&mut s, 5, "minecraft:oak_planks");
    let slot = s.state.entities.slot_of_uuid(s.builder).unwrap();
    s.state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[1] = None;
    expect_refusal(&mut s, Failure::InsufficientItems, "missing material");
    assert_eq!(total_stock(&s.state), 0);
}

#[test]
fn a_refused_cell_publishes_nothing_when_water_appeared_in_an_air_target() {
    let mut s = scene(
        None,
        &[],
        &[],
        &[
            (Cut, 5, "minecraft:air", None),
            (Build, 6, "minecraft:oak_log", Some("minecraft:oak_log")),
        ],
    );
    set(&mut s, 5, "minecraft:water");
    expect_refusal(&mut s, Failure::Blocked, "fluid in an air target");
    assert_eq!(total_stock(&s.state), 0);
}

// --- 4. portion order through a restart in the middle of a portion ---------

const ORDER: [(&str, i32); 9] = [
    ("saplings", 4),
    ("trees", 5),
    ("trees", 6),
    ("cut", 7),
    ("cut", 8),
    ("fill", 9),
    ("fill", 10),
    ("build", 11),
    ("build", 12),
];

fn ordered_scene(dir: Option<&Path>) -> Scene {
    scene(
        dir,
        &[
            (4, "minecraft:oak_sapling"),
            (5, "minecraft:oak_log"),
            (6, "minecraft:oak_log"),
            (7, "minecraft:stone"),
            (8, "minecraft:stone"),
        ],
        &[4, 5],
        &[
            (Saplings, 4, "minecraft:air", None),
            (Trees, 5, "minecraft:air", None),
            (Trees, 6, "minecraft:air", None),
            (Cut, 7, "minecraft:air", None),
            (Cut, 8, "minecraft:air", None),
            (Fill, 9, "minecraft:dirt", Some("minecraft:dirt")),
            (Fill, 10, "minecraft:dirt", Some("minecraft:dirt")),
            (Build, 11, "minecraft:oak_log", Some("minecraft:oak_log")),
            (Build, 12, "minecraft:oak_log", Some("minecraft:oak_log")),
        ],
    )
}

/// Advance one cell and prove it was the next one in portion order: exactly
/// that cell changed, every later portion's cell is untouched, and the stage
/// reported by the ledger is the stage the cell belongs to.
fn advance_in_order(s: &mut Scene, index: usize) {
    let (stage, x) = ORDER[index];
    let snapshot = status(s);
    assert_eq!(
        snapshot.stages[snapshot.current_stage_index as usize].stage, stage,
        "step {index}"
    );
    assert_eq!(
        executor::next_cell(&s.state, s.id, &s.handle).unwrap(),
        Some([x, Y, Z])
    );
    let before: Vec<_> = (4..=12).map(|x| at(s, x)).collect();
    step(s).unwrap();
    for (other, was) in (4..=12).zip(before) {
        assert_eq!(
            at(s, other) != was,
            other == x,
            "step {index} touched x={other}"
        );
    }
}

fn outcome(s: &Scene) -> serde_json::Value {
    let snapshot = status(s);
    serde_json::json!({
        "blocks": (4..=12).map(|x| at(s, x)).collect::<Vec<_>>(),
        "log": stock(&s.state, "minecraft:oak_log"),
        "cobblestone": stock(&s.state, "minecraft:cobblestone"),
        "total": total_stock(&s.state),
        "trees": s.state.settlements.get(s.id).unwrap().planted_trees,
        "consumed": snapshot.consumed.iter()
            .map(|m| (m.resource.clone(), m.quantity)).collect::<Vec<_>>(),
        "watermark": snapshot.watermark,
    })
}

#[test]
fn preparation_portions_hold_their_order_through_restarts_mid_portion_and_repair() {
    // The planner hands cells in portion order; a misordered plan is refused.
    {
        let mut s = ordered_scene(None);
        let bounds = mc_script::ScriptWorldBounds::new([14, Y, Z], [15, Y, Z]).unwrap();
        let cell = |portion, x| executor::PlannedCell {
            portion,
            at: [x, Y, Z],
            state: s.state.registry.states.air,
            quantity: 0,
            resource: None,
        };
        let id = s.id;
        let cells = vec![cell(Cut, 14), cell(Saplings, 15)];
        assert_eq!(
            executor::prepare(&mut s.state, id, "house".into(), bounds, bounds, cells).err(),
            Some(Failure::InvalidRequest)
        );
    }
    let work = tempfile::tempdir().unwrap();
    let live = work.path().join("live");
    std::fs::create_dir_all(&live).unwrap();
    let config = CoreConfig::new(live.clone(), 42);
    let mut s = ordered_scene(Some(&live));
    assert_eq!(
        status(&s)
            .stages
            .iter()
            .map(|stage| stage.stage.as_str())
            .collect::<Vec<_>>(),
        ["saplings", "trees", "cut", "fill", "build"]
    );
    s.state.tick_control.set_frozen(true);
    let (output, _, _) = crate::tick::run_tick(&mut s.state, &[]);
    crate::core::journal_entries_for_tick(&mut s.state, &output).unwrap();
    checkpoint(&mut s.state, &config);
    let mut writer = JournalWriter::open(&live, 1).unwrap();
    // Restart points: after step 4 (cut, one cell done), after step 6 (fill,
    // one cell done).
    let mut saved = Vec::new();
    for index in 0..ORDER.len() {
        advance_in_order(&mut s, index);
        journal(&mut s.state, &mut writer);
        if index == 3 || index == 5 {
            let copy = work.path().join(format!("restart-{index}"));
            copy_world(&live, &copy);
            saved.push((index, copy));
        }
    }
    assert_eq!(status(&s).state, ScriptStructureState::Committed);
    let finished = outcome(&s);
    // A cleared root retires on the ordinary retire pass once its trunk column
    // is empty, never cell by cell inside the debit (D18/N6). Journaled ticks
    // reach the pass's cadence so the WAL stays contiguous.
    while !s.state.tick.is_multiple_of(20) {
        journal(&mut s.state, &mut writer);
    }
    crate::settlements::forestry::retire_missing(&mut s.state);
    assert!(
        s.state
            .settlements
            .get(s.id)
            .unwrap()
            .planted_trees
            .is_empty()
    );
    assert_eq!(stock(&s.state, "minecraft:cobblestone"), 2);
    for (index, copy) in &saved {
        let mut restored = restore(&s, copy);
        let snapshot = status(&restored);
        let (stage, _) = ORDER[index + 1];
        assert_eq!(
            snapshot.stages[snapshot.current_stage_index as usize].stage, stage,
            "the cursor survives a restart in the middle of a portion"
        );
        crate::plugin_structures::validate(&restored.state).unwrap();
        for next in index + 1..ORDER.len() {
            advance_in_order(&mut restored, next);
        }
        assert_eq!(outcome(&restored), finished, "restart after step {index}");
    }

    // Repair is derived last: break a filled and a built cell, recover, and
    // restart with the repair stage half done.
    set(&mut s, 10, "minecraft:air");
    set(&mut s, 12, "minecraft:air");
    assert!(executor::recover(&mut s.state, s.id, &s.handle).unwrap());
    journal(&mut s.state, &mut writer);
    let names: Vec<_> = status(&s)
        .stages
        .iter()
        .map(|stage| stage.stage.clone())
        .collect();
    assert_eq!(
        names,
        ["saplings", "trees", "cut", "fill", "build", "repair"]
    );
    step(&mut s).unwrap();
    journal(&mut s.state, &mut writer);
    drop(writer);
    let copy = work.path().join("repair");
    copy_world(&live, &copy);
    let mut restored = restore(&s, &copy);
    let names: Vec<_> = status(&restored)
        .stages
        .iter()
        .map(|stage| stage.stage.clone())
        .collect();
    assert_eq!(
        names,
        ["saplings", "trees", "cut", "fill", "build", "repair"]
    );
    assert_eq!(status(&restored).current_stage_index, 5);
    crate::plugin_structures::validate(&restored.state).unwrap();
    step(&mut restored).unwrap();
    assert_eq!(status(&restored).state, ScriptStructureState::Committed);
    let dirt = named(&restored.state, "minecraft:dirt");
    let log = named(&restored.state, "minecraft:oak_log");
    assert_eq!(
        (at(&restored, 10), at(&restored, 12)),
        (Some(dirt), Some(log))
    );
}

#[test]
fn a_ledger_record_with_misordered_portions_is_refused_at_startup() {
    let s = ordered_scene(None);
    let key = format!("structure/{}", s.handle);
    let valid = s.state.plugins.get_domain(OWNER, &key).unwrap().clone();
    crate::plugin_structures::validate(&s.state).unwrap();
    let mut bad = valid.clone();
    // Swap fill and build in stages, cells and portions together: every
    // alignment still holds, only the portion order is wrong.
    bad["snapshot"]["stages"].as_array_mut().unwrap().swap(3, 4);
    bad["cells"].as_array_mut().unwrap().swap(3, 4);
    bad["portions"].as_array_mut().unwrap().swap(3, 4);
    let mut s = s;
    s.state.plugins.set_domain(OWNER, &key, bad);
    assert!(matches!(
        crate::plugin_structures::validate(&s.state),
        Err(crate::CoreError::Recovery(_))
    ));
    s.state.plugins.set_domain(OWNER, &key, valid);
    crate::plugin_structures::validate(&s.state).unwrap();
}

// --- 5. outside territory ---------------------------------------------------

#[test]
fn outside_territory_natural_drift_proceeds_but_player_history_blocks_without_a_pause() {
    let build = [
        (Build, 5, "minecraft:oak_log", Some("minecraft:oak_log")),
        (Build, 6, "minecraft:oak_log", Some("minecraft:oak_log")),
    ];
    // A founding site lies outside the parent's territory.
    let mut s = scene(None, &[], &[], &build);
    s.state.settlements.get_mut(s.id).unwrap().radius = 1;
    assert!(!s.state.settlements.get(s.id).unwrap().contains([5, Y, Z]));
    set(&mut s, 5, "minecraft:grass_block");
    let log = named(&s.state, "minecraft:oak_log");
    step(&mut s).unwrap();
    assert_eq!(at(&s, 5), Some(log));
    assert_eq!(
        stock(&s.state, "minecraft:dirt"),
        1,
        "natural drift is cleared and credited"
    );
    assert_eq!(total_stock(&s.state), 1);

    let mut s = scene(None, &[], &[], &build);
    s.state.settlements.get_mut(s.id).unwrap().radius = 1;
    set(&mut s, 5, "minecraft:grass_block");
    let history = crate::settlements::forestry::tests::player_history(&s.state, [5, Y, Z], 0);
    let clean = std::mem::replace(&mut s.state.block_log, history);
    assert!(s.state.block_log.player_touched([5, Y, Z]));
    let grass = named(&s.state, "minecraft:grass_block");
    for attempt in 0..3 {
        expect_refusal(
            &mut s,
            Failure::Blocked,
            &format!("player history, retry {attempt}"),
        );
        assert_eq!(at(&s, 5), Some(grass));
    }
    // Nothing was stored, so lifting the veto resumes with no recovery step.
    s.state.block_log = clean;
    step(&mut s).unwrap();
    assert_eq!(at(&s, 5), Some(log));
    assert_eq!(stock(&s.state, "minecraft:dirt"), 1);

    // The same history block inside the territory is the village's property.
    let mut s = scene(None, &[], &[], &build);
    set(&mut s, 5, "minecraft:grass_block");
    s.state.block_log = crate::settlements::forestry::tests::player_history(&s.state, [5, Y, Z], 0);
    step(&mut s).unwrap();
    assert_eq!(at(&s, 5), Some(log));
    assert_eq!(stock(&s.state, "minecraft:dirt"), 1);
}

// --- 6. a non-empty container in a placement cell ---------------------------

fn put_chest(s: &mut Scene, x: i32, contents: Option<&str>) {
    set(s, x, "minecraft:chest");
    let mut slots = vec![ItemStack::EMPTY; 27];
    if let Some(name) = contents {
        slots[0] = ItemStack::new(item(&s.state, name), 5);
    }
    s.state.block_entities.insert(
        (x, Y, Z),
        crate::tick::BlockEntityState {
            slots,
            furnace: None,
        },
    );
}

#[test]
fn a_non_empty_container_in_a_placement_cell_still_refuses() {
    let build = [
        (Build, 5, "minecraft:oak_log", Some("minecraft:oak_log")),
        (Build, 6, "minecraft:oak_log", Some("minecraft:oak_log")),
    ];
    // The chest block is already there at admission (unchanged), or appears
    // after admission (changed).
    for after_admission in [false, true] {
        let pre: &[(i32, &str)] = if after_admission {
            &[]
        } else {
            &[(5, "minecraft:chest")]
        };
        let mut s = scene(None, pre, &[], &build);
        put_chest(&mut s, 5, Some("minecraft:diamond"));
        let chest = named(&s.state, "minecraft:chest");
        let label = format!("after_admission={after_admission}");
        expect_refusal(&mut s, Failure::Blocked, &label);
        assert_eq!(at(&s, 5), Some(chest), "{label}");
        assert_eq!(
            s.state.block_entities[&(5, Y, Z)].slots[0].count,
            5,
            "{label}: the contents stay"
        );
        // Emptied, the cell is the village's and the placement proceeds.
        s.state.block_entities.get_mut(&(5, Y, Z)).unwrap().slots[0] = ItemStack::EMPTY;
        step(&mut s).unwrap();
        assert_eq!(
            at(&s, 5),
            Some(named(&s.state, "minecraft:oak_log")),
            "{label}"
        );
        assert_eq!(stock(&s.state, "minecraft:diamond"), 0, "{label}");
        // Changed and unchanged empty containers both yield their canonical
        // block drop exactly once under the placement.
        assert_eq!(stock(&s.state, "minecraft:chest"), 1);
        assert_eq!(total_stock(&s.state), 1);
    }
}

// --- shared native helpers ---------------------------------------------------

fn journal(state: &mut TickState, writer: &mut JournalWriter) {
    let (output, _, _) = crate::tick::run_tick(state, &[]);
    let entries = crate::core::journal_entries_for_tick(state, &output).unwrap();
    writer.append(output.tick, output.tick, &entries).unwrap();
    writer.flush().unwrap();
}

/// The world recovered from a copy of the native store in `dir`, carrying the
/// real server registry like production does.
fn restore(like: &Scene, dir: &Path) -> Scene {
    let mut state = reopen(dir).unwrap();
    state.registry = registry();
    Scene {
        state,
        id: like.id,
        builder: like.builder,
        handle: like.handle.clone(),
    }
}
