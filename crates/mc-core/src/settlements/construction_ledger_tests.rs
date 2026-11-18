//! The per-record structure ledger under real settlement construction: every
//! settlement writer replays from the WAL to the live cache, a stale advance
//! is refused, and a checkpoint plus WAL tail recovers the ledger together
//! with the blocks and custody it describes.

use super::*;
use crate::plugin_structures::Replica;
use crate::settlements::construction::tests::{attempt, fixture_in, open_plan, start};
use std::collections::BTreeMap;

const OWNER: &str = "@native-structures";

fn structure(state: &TickState, id: SettlementId) -> Option<String> {
    state
        .settlements
        .get(id)
        .unwrap()
        .projects
        .first()
        .map(|project| project.structure.clone())
}

fn snapshot(
    state: &TickState,
    id: SettlementId,
    handle: &str,
) -> mc_script::ScriptStructureSnapshot {
    executor::status(state, id, handle).unwrap()
}

fn record_key(handle: &str) -> String {
    format!("structure/{handle}")
}

/// One attempt per WAL boundary until the project leaves the settlement; the
/// replica must equal the live cache after each, and only the project's record
/// and the meta key may be journaled.
fn build_checked(
    state: &mut TickState,
    replica: &mut Replica,
    id: SettlementId,
    builder: crate::entities::EntitySlot,
    handle: &str,
    mut on_attempt: impl FnMut(&mut TickState, usize),
) -> Vec<mc_script::ScriptStructureSnapshot> {
    let key = record_key(handle);
    let mut seen = vec![snapshot(state, id, handle)];
    for index in 0..4096 {
        if structure(state, id).is_none() {
            return seen;
        }
        on_attempt(state, index);
        attempt(state, id, builder);
        let boundary = replica.step(state);
        replica.assert_matches(state);
        for (journaled, present) in &boundary.keys {
            assert!(*present, "{journaled} removed");
            assert!(journaled == "ledger" || *journaled == key, "{journaled}");
        }
        if boundary.keys.contains_key(&key) {
            let now = snapshot(state, id, handle);
            let last = seen.last().unwrap();
            assert!(now.revision > last.revision);
            assert!(now.watermark >= last.watermark);
            assert!(now.watermark <= now.revision);
            seen.push(now);
        }
    }
    panic!(
        "construction did not finish: {:?}",
        snapshot(state, id, handle)
    );
}

#[test]
fn settlement_ledger_writes_replay_from_the_wal_after_every_transition() {
    let (mut state, id, builder) = fixture();
    open_plan(&mut state);
    let mut replica = Replica::capture(&mut state);
    start(&mut state, id, "house", 1, None);
    let handle = structure(&state, id).unwrap();
    let key = record_key(&handle);
    let boundary = replica.step(&mut state);
    assert!(boundary.keys[&key] && boundary.keys["ledger"]);
    replica.assert_matches(&state);
    assert_eq!(
        snapshot(&state, id, &handle).watermark,
        0,
        "a new record counts no advance"
    );
    fund(&mut state, "house", 1);

    // An already-processed cell is not a pending-cell certificate. Its loss
    // queues a paid repair at completion without stopping unrelated work.
    let mut emptied = None;
    let seen = build_checked(
        &mut state,
        &mut replica,
        id,
        builder,
        &handle,
        |state, _| {
            if emptied.is_some() {
                return;
            }
            let record = state
                .plugins
                .get_domain(OWNER, &record_key(&handle))
                .unwrap()
                .clone();
            let snapshot = &record["snapshot"];
            let index = snapshot["current_stage_index"].as_u64().unwrap() as usize;
            if snapshot["stages"][index]["stage"] != "build"
                || snapshot["completed_work_units"].as_u64().unwrap() < 2
            {
                return;
            }
            let cell = &record["cells"][index][0];
            let at = [0, 1, 2].map(|axis| cell["at"][axis].as_i64().unwrap() as i32);
            // The repair needs one more block than the funded blueprint.
            let placed = state
                .registry
                .blocks
                .by_id(mc_world::BlockStateId(
                    cell["state"].as_u64().unwrap() as u32
                ))
                .unwrap()
                .block
                .id
                .clone();
            let item = state.registry.items.id_of(&placed).unwrap();
            let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
            *slots.iter_mut().find(|stack| stack.is_empty()).unwrap() =
                mc_data::ItemStack::new(item, 1);
            state
                .world
                .set_block(at[0], at[1], at[2], state.registry.states.air);
            emptied = Some(at);
        },
    );
    assert!(emptied.is_some(), "a placed build cell was emptied");
    assert!(
        seen.iter()
            .all(|s| s.pause_reason.as_deref() != Some("footprint_changed")),
        "completed-cell damage does not pause a pending portion"
    );
    assert!(
        seen.iter()
            .any(|s| s.stages.iter().any(|stage| stage.stage == "repair")),
        "recover queued a repair stage"
    );
    let done = seen.last().unwrap();
    assert_eq!(done.state, mc_script::ScriptStructureState::Committed);
    let advances = seen
        .windows(2)
        .filter(|pair| pair[1].watermark > pair[0].watermark)
        .count() as u64;
    assert_eq!(done.watermark, advances, "one watermark step per advance");

    // A committed structure whose placed cell is emptied reopens for repair:
    // a new revision, the same watermark. `recover` is the admission the tick
    // uses and the entry `reopen_emptied` is reached from, so the ledger write
    // is the one the game performs; the direct call would also pin this test to
    // that private step.
    let record = state.plugins.get_domain(OWNER, &key).unwrap().clone();
    let cell = record["cells"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|stage| stage.as_array().unwrap())
        .find(|cell| cell["state"].as_u64().unwrap() != u64::from(state.registry.states.air))
        .unwrap();
    let at = [0, 1, 2].map(|axis| cell["at"][axis].as_i64().unwrap() as i32);
    state
        .world
        .set_block(at[0], at[1], at[2], state.registry.states.air);
    assert!(executor::recover(&mut state, id, &handle).unwrap());
    let boundary = replica.step(&mut state);
    assert_eq!(
        boundary.keys.keys().collect::<Vec<_>>(),
        ["ledger", key.as_str()]
    );
    replica.assert_matches(&state);
    let reopened = snapshot(&state, id, &handle);
    assert!(reopened.revision > done.revision);
    assert_eq!(reopened.watermark, done.watermark);

    // Bulk cancel writes every record of the settlement, none other.
    executor::cancel(&mut state, id).unwrap();
    let boundary = replica.step(&mut state);
    assert_eq!(
        boundary.keys.keys().collect::<Vec<_>>(),
        ["ledger", key.as_str()]
    );
    replica.assert_matches(&state);
    let cancelled = snapshot(&state, id, &handle);
    assert_eq!(cancelled.state, mc_script::ScriptStructureState::Cancelled);
    assert_eq!(cancelled.watermark, done.watermark);
    crate::plugin_structures::validate(&state).unwrap();
}

#[test]
fn founding_transfer_and_bulk_cancel_journal_exactly_the_records_they_rewrite() {
    let (mut state, parent, expedition) = prepared(4);
    let mut replica = Replica::capture(&mut state);
    let before: BTreeMap<String, mc_script::ScriptStructureSnapshot> = state
        .plugins
        .domain_entries(OWNER)
        .filter_map(|(key, value)| {
            let handle = key.strip_prefix("structure/")?;
            Some((
                handle.to_owned(),
                serde_json::from_value(value["snapshot"].clone()).unwrap(),
            ))
        })
        .collect();
    assert!(
        finish(&mut state, parent, &expedition),
        "the expedition founds its town"
    );
    let child = state.settlements.find("colony_2").unwrap().id;
    let boundary = replica.step(&mut state);
    replica.assert_matches(&state);
    let transferred: Vec<_> = before
        .keys()
        .filter(|handle| executor::status(&state, child, handle).is_ok())
        .cloned()
        .collect();
    assert_eq!(
        transferred.len(),
        4,
        "the four outpost structures change owner"
    );
    let mut expected: Vec<String> = transferred
        .iter()
        .map(|handle| record_key(handle))
        .collect();
    expected.push("ledger".into());
    expected.sort();
    assert_eq!(boundary.keys.keys().cloned().collect::<Vec<_>>(), expected);
    for handle in &transferred {
        let now = snapshot(&state, child, handle);
        assert!(now.revision > before[handle].revision);
        assert_eq!(now.watermark, before[handle].watermark);
    }

    executor::cancel(&mut state, child).unwrap();
    let boundary = replica.step(&mut state);
    replica.assert_matches(&state);
    let mut expected: Vec<String> = transferred
        .iter()
        .map(|handle| record_key(handle))
        .collect();
    expected.push("ledger".into());
    expected.sort();
    assert_eq!(boundary.keys.keys().cloned().collect::<Vec<_>>(), expected);
    crate::plugin_structures::validate(&state).unwrap();
}

#[test]
fn cancelling_one_prepared_structure_journals_only_its_record() {
    let (mut state, parent, expedition) = prepared(3);
    let (name, origin) = expedition.sites()[3];
    assert!(begin(
        &mut state,
        Project {
            settlement: parent,
            blueprint: name.into(),
            level: 1,
            origin,
            rotation: 0,
            building: None,
            structure: String::new(),
            ordered: false,
            builder: None,
            order_seq: None,
        },
        "oak",
        0
    ));
    let handle = structure(&state, parent).unwrap();
    let mut replica = Replica::capture(&mut state);
    let prepared = snapshot(&state, parent, &handle);
    assert_eq!(prepared.watermark, 0);
    executor::cancel_structure(&mut state, parent, &handle).unwrap();
    let boundary = replica.step(&mut state);
    let key = record_key(&handle);
    assert_eq!(
        boundary.keys.keys().collect::<Vec<_>>(),
        ["ledger", key.as_str()]
    );
    replica.assert_matches(&state);
    let cancelled = snapshot(&state, parent, &handle);
    assert!(cancelled.revision > prepared.revision);
    assert_eq!(cancelled.watermark, 0);
}

/// The settlement path calls the structure transaction with its current
/// snapshot; an advance with an older revision is refused without a debit or
/// a placement.
#[test]
fn a_settlement_advance_with_an_old_revision_changes_nothing() {
    let (mut state, id, builder) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    fund(&mut state, "house", 1);
    let handle = structure(&state, id).unwrap();
    let old = snapshot(&state, id, &handle);
    while snapshot(&state, id, &handle).watermark == 0 {
        attempt(&mut state, id, builder);
    }
    let advanced = snapshot(&state, id, &handle);
    let uuid = state.entities.row(builder).uuid;
    let stage = advanced.stages[advanced.current_stage_index as usize]
        .stage
        .clone();
    let before = (
        state
            .plugins
            .domain_entries(OWNER)
            .map(|(k, v)| (k.to_owned(), v.clone()))
            .collect::<BTreeMap<_, _>>(),
        serde_json::to_value(
            &state
                .entities
                .row(builder)
                .villager
                .as_ref()
                .unwrap()
                .inventory,
        )
        .unwrap(),
        serde_json::to_value(&state.block_entities[&(1, 201, 1)].slots).unwrap(),
        state.plugin_block_writes.len(),
    );
    let request = mc_script::ScriptOperationRequest::try_new(
        "stale-settlement-advance",
        mc_script::ScriptOperation::WorldStructure {
            operation: mc_script::ScriptWorldStructureOperation::AdvanceStructure {
                operation_id: format!("settlement-build-{}", old.revision),
                structure_id: handle.clone(),
                stage,
                reservation_ref: uuid.to_string(),
                expected_revision: old.revision,
                work_units: 1,
            },
        },
    )
    .unwrap();
    let outcome = crate::plugin_structures::execute(&mut state, &executor::owner(id), &request);
    assert_eq!(
        outcome.failure(),
        Some(mc_script::ScriptOperationFailure::StaleRevision)
    );
    let after = (
        state
            .plugins
            .domain_entries(OWNER)
            .map(|(k, v)| (k.to_owned(), v.clone()))
            .collect::<BTreeMap<_, _>>(),
        serde_json::to_value(
            &state
                .entities
                .row(builder)
                .villager
                .as_ref()
                .unwrap()
                .inventory,
        )
        .unwrap(),
        serde_json::to_value(&state.block_entities[&(1, 201, 1)].slots).unwrap(),
        state.plugin_block_writes.len(),
    );
    assert_eq!(after, before);
    assert_eq!(snapshot(&state, id, &handle).watermark, advanced.watermark);
}

/// Ledger keys, the blocks of every planned cell, the stock and the builder's
/// carry: what recovery must bring back together.
fn construction_image(state: &TickState, id: SettlementId) -> serde_json::Value {
    let ledger: BTreeMap<String, serde_json::Value> = state
        .plugins
        .domain_entries(OWNER)
        .map(|(key, value)| (key.to_owned(), value.clone()))
        .collect();
    let mut blocks = BTreeMap::new();
    for record in ledger
        .iter()
        .filter(|(key, _)| key.starts_with("structure/"))
        .map(|(_, record)| record)
    {
        for cell in record["cells"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|stage| stage.as_array().unwrap())
        {
            let at = [0, 1, 2].map(|axis| cell["at"][axis].as_i64().unwrap() as i32);
            blocks.insert(format!("{at:?}"), state.world.block_at(at[0], at[1], at[2]));
        }
    }
    let carry: BTreeMap<String, serde_json::Value> = state
        .entities
        .slots()
        .filter_map(|slot| {
            let row = state.entities.row(slot);
            let villager = row.villager.as_ref()?;
            (villager.settlement == Some(id)).then(|| {
                (
                    row.uuid.to_string(),
                    serde_json::to_value(&villager.inventory).unwrap(),
                )
            })
        })
        .collect();
    serde_json::json!({
        "ledger": ledger,
        "blocks": blocks,
        "stock": state.block_entities[&(1, 201, 1)].slots,
        "carry": carry,
        "projects": state.settlements.get(id).unwrap().projects,
    })
}

fn copy_world(from: &std::path::Path, to: &std::path::Path) {
    for entry in walkdir(from) {
        let target = to.join(entry.strip_prefix(from).unwrap());
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(&entry, &target).unwrap();
    }
}

fn walkdir(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}

/// The world recovered from `directory` (a fresh one when it is empty), with
/// the fixture chunks resident.
fn reopen(directory: &std::path::Path) -> Result<TickState, crate::CoreError> {
    let config = CoreConfig::new(directory.to_path_buf(), 42);
    let mut state = crate::core::recover_dimension_state(&config, Dimension::Overworld)?;
    state
        .world
        .ensure_blocking(&rustc_hash::FxHashMap::from_iter((-1..=1).flat_map(|x| {
            (-1..=1).map(move |z| (crate::boundary::ChunkPos { x, z }, 0))
        })));
    // The stock chest lives in a checkpointed chunk: hydrate it as a tick would.
    crate::tick::sync_chunk_block_entities(&mut state);
    // The structure startup fence follows physical dimension recovery.
    crate::plugin_structures::validate(&state)?;
    Ok(state)
}

/// The builder settlement on a world backed by the native store in
/// `directory`, so a checkpoint persists its chunks and the chest.
fn native_fixture(
    directory: &std::path::Path,
) -> (TickState, SettlementId, crate::entities::EntitySlot) {
    let mut state = reopen(directory).unwrap();
    for x in 0..16 {
        for z in 0..16 {
            state
                .world
                .set_block(x, 200, z, state.registry.states.stone);
        }
    }
    state.tickets.insert(
        "ledger-fixture".into(),
        (crate::boundary::ChunkPos { x: 0, z: 0 }, 0),
    );
    state.refresh_active_chunks(state.tick);
    fixture_in(state)
}

/// One builder attempt and the tick after it, journaled as one batch that
/// covers the ticks the attempt skipped.
fn journaled_attempt(
    state: &mut TickState,
    id: SettlementId,
    builder: crate::entities::EntitySlot,
    writer: &mut JournalWriter,
) {
    let from = state.tick + 1;
    attempt(state, id, builder);
    let (output, _, _) = crate::tick::run_tick(state, &[]);
    let entries = crate::core::journal_entries_for_tick(state, &output).unwrap();
    writer.append(from, output.tick, &entries).unwrap();
    writer.flush().unwrap();
}

/// A real checkpoint and WAL tail mid-construction: a crash before the last
/// batch is published recovers the previous tick, after it the last tick, a
/// damaged batch refuses recovery; construction then continues, and a later
/// checkpoint with the tail pruned reopens to the same image.
#[test]
fn settlement_ledger_recovers_from_checkpoint_and_wal_tail_with_its_physical_effects() {
    let work = tempfile::tempdir().unwrap();
    let live_dir = work.path().join("live");
    std::fs::create_dir_all(&live_dir).unwrap();
    let config = CoreConfig::new(live_dir.clone(), 42);
    let (mut state, id, builder) = native_fixture(&live_dir);
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    fund(&mut state, "house", 1);
    // The fixture funds the chest in memory; make it durable as a write would.
    state
        .world
        .mark_block_entity_dirty(crate::boundary::BlockPos { x: 1, y: 201, z: 1 });
    let handle = structure(&state, id).unwrap();
    for _ in 0..8 {
        attempt(&mut state, id, builder);
    }
    state.tick_control.set_frozen(true);
    let (baseline, _, _) = crate::tick::run_tick(&mut state, &[]);
    crate::core::journal_entries_for_tick(&mut state, &baseline).unwrap();
    checkpoint(&mut state, &config);
    let at_checkpoint = snapshot(&state, id, &handle).watermark;

    let mut writer = JournalWriter::open(&live_dir, 1).unwrap();
    let mut images = Vec::new();
    for _ in 0..24 {
        journaled_attempt(&mut state, id, builder, &mut writer);
        images.push(construction_image(&state, id));
    }
    let unpublished = work.path().join("unpublished");
    copy_world(&live_dir, &unpublished);
    journaled_attempt(&mut state, id, builder, &mut writer);
    images.push(construction_image(&state, id));
    drop(writer);
    let published = work.path().join("published");
    copy_world(&live_dir, &published);
    let damaged = work.path().join("damaged");
    copy_world(&live_dir, &damaged);
    assert!(
        snapshot(&state, id, &handle).watermark > at_checkpoint,
        "the WAL tail carries advances after the checkpoint"
    );

    let restored = reopen(&unpublished).unwrap();
    assert_eq!(
        construction_image(&restored, id),
        images[images.len() - 2],
        "unpublished"
    );
    let mut restored = reopen(&published).unwrap();
    assert_eq!(
        construction_image(&restored, id),
        images[images.len() - 1],
        "published"
    );
    crate::plugin_structures::validate(&restored).unwrap();

    let last = walkdir(&damaged.join("journal"))
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "bin"))
        .max()
        .unwrap();
    let mut bytes = std::fs::read(&last).unwrap();
    let middle = 32 + (bytes.len() - 32) / 2;
    bytes[middle] ^= 0x20;
    std::fs::write(&last, bytes).unwrap();
    assert!(
        reopen(&damaged).is_err(),
        "a damaged published batch refuses recovery"
    );

    // Construction continues on the recovered world.
    let restored_builder = restored
        .entities
        .slots()
        .find(|slot| {
            restored
                .entities
                .row(*slot)
                .villager
                .as_ref()
                .is_some_and(|v| v.data.profession == crate::villager::VillagerProfession::Builder)
        })
        .unwrap();
    let recovered = snapshot(&restored, id, &handle);
    for _ in 0..12 {
        attempt(&mut restored, id, restored_builder);
    }
    let continued = snapshot(&restored, id, &handle);
    assert!(
        continued.watermark > recovered.watermark,
        "building continues"
    );
    assert!(continued.revision > recovered.revision);

    // A checkpoint of the recovered world with its WAL tail pruned reopens to
    // the same construction image.
    restored.tick_control.set_frozen(true);
    let (output, _, _) = crate::tick::run_tick(&mut restored, &[]);
    crate::core::journal_entries_for_tick(&mut restored, &output).unwrap();
    checkpoint(&mut restored, &CoreConfig::new(published.clone(), 42));
    let tick = restored.tick;
    for batch in walkdir(&published.join("journal")) {
        if batch.extension().is_some_and(|ext| ext == "bin") {
            std::fs::remove_file(batch).unwrap();
        }
    }
    let image = construction_image(&restored, id);
    let reopened = reopen(&published).unwrap();
    assert_eq!(reopened.tick, tick);
    assert_eq!(
        construction_image(&reopened, id),
        image,
        "after checkpoint and prune"
    );
    crate::plugin_structures::validate(&reopened).unwrap();
}

#[test]
fn preparation_portion_boundary_and_mid_portion_recover_exact_physical_evidence() {
    let work = tempfile::tempdir().unwrap();
    let live = work.path().join("live");
    std::fs::create_dir_all(&live).unwrap();
    let config = CoreConfig::new(live.clone(), 42);
    let (mut state, id, builder) = native_fixture(&live);
    state.registry = std::sync::Arc::new(
        crate::registry::CoreRegistry::load(Some(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"),
        ))
        .unwrap(),
    );
    let registry = std::sync::Arc::clone(&state.registry);
    let uuid = state.entities.row(builder).uuid;
    let log = crate::settlements::allocator::tests::block(&state, "minecraft:oak_log", &[]);
    let air = state.registry.states.air;
    let dirt = state.registry.states.dirt;
    let planks = crate::settlements::allocator::tests::block(&state, "minecraft:oak_planks", &[]);
    state.world.set_block(6, 201, 4, planks);
    let plank_item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:oak_planks").unwrap())
        .unwrap();
    state.world.set_block(3, 201, 4, log);
    state
        .world
        .set_block(4, 201, 4, state.registry.states.stone);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert([3, 201, 4]);
    let bounds = mc_script::ScriptWorldBounds::new([3, 201, 4], [7, 201, 4]).unwrap();
    let cells = [
        (executor::PortionKind::Trees, 3, air, None),
        (executor::PortionKind::Cut, 4, air, None),
        (executor::PortionKind::Fill, 5, dirt, Some("minecraft:dirt")),
        (
            executor::PortionKind::Build,
            6,
            log,
            Some("minecraft:oak_log"),
        ),
        (
            executor::PortionKind::Build,
            7,
            log,
            Some("minecraft:oak_log"),
        ),
    ]
    .into_iter()
    .map(|(portion, x, state, resource)| executor::PlannedCell {
        portion,
        at: [x, 201, 4],
        state,
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
        let item = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap();
        state
            .entities
            .row_mut(builder)
            .villager
            .as_mut()
            .unwrap()
            .inventory[index] = Some(mc_data::ItemStack::new(item, 8));
    }
    state.tick_control.set_frozen(true);
    let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
    crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
    checkpoint(&mut state, &config);
    let mut writer = JournalWriter::open(&live, 1).unwrap();
    for step in 0..4 {
        executor::advance(&mut state, id, &handle, uuid).unwrap();
        let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
        let entries = crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
        writer.append(output.tick, output.tick, &entries).unwrap();
        writer.flush().unwrap();
        if step == 0 || step == 3 {
            let status = snapshot(&state, id, &handle);
            assert_eq!(
                (status.current_stage_index, status.completed_work_units),
                if step == 0 { (1, 0) } else { (3, 1) }
            );
            let saved = work.path().join(format!("restart-{step}"));
            copy_world(&live, &saved);
            let image = construction_image(&state, id);
            for _ in 0..2 {
                let restored = reopen(&saved).unwrap();
                assert_eq!(
                    construction_image(&restored, id),
                    image,
                    "repeat WAL replay at step {step}"
                );
                assert!(
                    restored
                        .settlements
                        .get(id)
                        .unwrap()
                        .planted_trees
                        .is_empty()
                );
                crate::plugin_structures::validate(&restored).unwrap();
            }
        }
    }
    drop(writer);
    let saved = work.path().join("restart-3");
    let mut restored = reopen(&saved).unwrap();
    restored.registry = std::sync::Arc::clone(&registry);
    let dirt_item = restored
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:dirt").unwrap())
        .unwrap();
    let stock_before: i32 = restored.block_entities[&(1, 201, 1)]
        .slots
        .iter()
        .filter(|stack| stack.item_id == dirt_item)
        .map(|stack| stack.count)
        .sum();
    // The still-unprocessed second build cell changes after restart.
    restored.world.set_block(7, 201, 4, dirt);
    assert!(executor::advance(&mut restored, id, &handle, uuid).unwrap());
    assert_eq!(
        restored.block_entities[&(1, 201, 1)]
            .slots
            .iter()
            .filter(|stack| stack.item_id == plank_item)
            .map(|stack| stack.count)
            .sum::<i32>(),
        1,
        "pre-admission planks credit once, not again after repeated WAL replay"
    );
    let committed = snapshot(&restored, id, &handle);
    assert_eq!(committed.state, mc_script::ScriptStructureState::Committed);
    assert_eq!(committed.watermark, 5);
    assert_eq!(
        committed
            .consumed
            .iter()
            .map(|material| (material.resource.as_str(), material.quantity))
            .collect::<Vec<_>>(),
        [("minecraft:dirt", 1), ("minecraft:oak_log", 2)]
    );
    assert_eq!(
        restored.block_entities[&(1, 201, 1)]
            .slots
            .iter()
            .filter(|stack| stack.item_id == dirt_item)
            .map(|stack| stack.count)
            .sum::<i32>(),
        stock_before + 1
    );
    restored.tick_control.set_frozen(true);
    let (output, _, _) = crate::tick::run_tick(&mut restored, &[]);
    let entries = crate::core::journal_entries_for_tick(&mut restored, &output).unwrap();
    {
        let mut writer = JournalWriter::open(&saved, 1).unwrap();
        writer.append(output.tick, output.tick, &entries).unwrap();
    }
    checkpoint(&mut restored, &CoreConfig::new(saved.clone(), 42));
    for batch in walkdir(&saved.join("journal")) {
        if batch
            .extension()
            .is_some_and(|extension| extension == "bin")
        {
            std::fs::remove_file(batch).unwrap();
        }
    }
    let image = construction_image(&restored, id);
    let reopened = reopen(&saved).unwrap();
    assert_eq!(
        construction_image(&reopened, id),
        image,
        "checkpoint survives pruning the replayed WAL"
    );
    crate::plugin_structures::validate(&reopened).unwrap();
}

#[test]
fn startup_refuses_structure_without_portions_but_construction_free_store_opens() {
    let work = tempfile::tempdir().unwrap();
    let live = work.path().join("live");
    std::fs::create_dir_all(&live).unwrap();
    let (mut state, id, _) = native_fixture(&live);
    state.tick_control.set_frozen(true);
    let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
    crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
    checkpoint(&mut state, &CoreConfig::new(live.clone(), 42));
    let clean = work.path().join("construction-free");
    copy_world(&live, &clean);
    assert!(reopen(&clean).is_ok());
    start(&mut state, id, "house", 1, None);
    let handle = structure(&state, id).unwrap();
    let key = record_key(&handle);
    let valid = state.plugins.get_domain(OWNER, &key).unwrap().clone();
    let mut old = valid.clone();
    old.as_object_mut().unwrap().remove("portions");
    state.plugins.set_domain(OWNER, &key, old);
    checkpoint(&mut state, &CoreConfig::new(live.clone(), 42));
    let refused = work.path().join("without-portions");
    copy_world(&live, &refused);
    assert!(matches!(
        reopen(&refused),
        Err(crate::CoreError::Recovery(_))
    ));
    state.plugins.set_domain(OWNER, &key, valid);
    checkpoint(&mut state, &CoreConfig::new(live.clone(), 42));
    let current = work.path().join("with-portions");
    copy_world(&live, &current);
    assert!(
        reopen(&current).is_ok(),
        "the same store opens with required evidence"
    );
}

#[cfg(test)]
#[path = "construction_rederive_tests.rs"]
mod rederive;
