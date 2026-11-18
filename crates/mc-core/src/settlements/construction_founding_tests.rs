use super::*;
use crate::journal::{JournalEntry, JournalWriter, Replay};
use crate::settlements::allocator::tests::villager;
use crate::settlements::construction::tests::{complete, fixture, fund};
use crate::settlements::{households, persistence};
use crate::{CoreConfig, dimensions::Dimension};

fn flat_site(state: &mut TickState, expedition: &FoundingExpedition) {
    let bounds = expedition.bounds();
    let wanted = ((bounds.min[0] - 2).div_euclid(16)..=(bounds.max[0] + 2).div_euclid(16))
        .flat_map(|x| {
            ((bounds.min[2] - 2).div_euclid(16)..=(bounds.max[2] + 2).div_euclid(16))
                .map(move |z| (crate::boundary::ChunkPos { x, z }, 0))
        })
        .collect();
    state.world.ensure_blocking(&wanted);
    for x in bounds.min[0] - 2..=bounds.max[0] + 2 {
        for z in bounds.min[2] - 2..=bounds.max[2] + 2 {
            state
                .world
                .set_block(x, bounds.min[1], z, state.registry.states.stone);
            for y in bounds.min[1] + 1..=bounds.max[1] + 3 {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    state.world.fold_overlay();
}

fn prepared(stages: usize) -> (TickState, SettlementId, FoundingExpedition) {
    prepared_at(stages, [96, 200, 0])
}

fn prepared_at(
    stages: usize,
    destination: [i32; 3],
) -> (TickState, SettlementId, FoundingExpedition) {
    let (mut state, parent, builder) = fixture();
    // Establish culture through actual paid construction, not a test flag.
    super::super::tests::start(&mut state, parent, "spanish_plaza", 1, None);
    fund(&mut state, "spanish_plaza", 1);
    complete(&mut state, parent, builder);
    let mut migrants = Vec::new();
    for index in 0..23 {
        let slot = villager(&mut state, [2.5, 201.0, 2.5]);
        let entity = state.entities.body(slot).id;
        crate::settlements::bind(&mut state, parent, entity).unwrap();
        state.entities.row_mut(slot).controls.no_ai = true;
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .food_level = 12;
        if index < 3 {
            migrants.push(entity);
            let diamond = state
                .registry
                .items
                .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
                .unwrap();
            state
                .entities
                .row_mut(slot)
                .villager
                .as_mut()
                .unwrap()
                .inventory[7] = Some(
                mc_data::ItemStack::new(diamond, index + 1)
                    .with_custom_name(format!("Founding heirloom {index}")),
            );
        }
    }
    let family = households::form(
        &mut state,
        parent,
        &migrants,
        households::HouseholdOrigin::Migration,
    )
    .unwrap();
    let expedition = FoundingExpedition {
        destination,
        household: family,
    };
    flat_site(&mut state, &expedition);
    state.settlements.get_mut(parent).unwrap().founding = Some(expedition.clone());
    for (name, origin) in expedition.sites().into_iter().take(stages) {
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
        fund(&mut state, name, 1);
        // Each completed outpost releases its worker. Model the next native
        // allocation, as fixture() does for the initial paid project.
        let villager = state.entities.row_mut(builder).villager.as_mut().unwrap();
        villager.data.profession = Profession::Builder;
        villager.activity = crate::villager::Activity::Work;
        complete(&mut state, parent, builder);
    }
    state.entities.row_mut(builder).controls.no_ai = true;
    (state, parent, expedition)
}

fn members(state: &TickState) -> serde_json::Value {
    let mut rows: Vec<_> = state.entities.slots().filter_map(|slot| {
        let row = state.entities.row(slot);
        let v = row.villager.as_ref()?;
        let settlement = v.settlement?;
        Some(serde_json::json!({"uuid": row.uuid.to_string(), "id": state.entities.body(slot).id.0,
            "settlement": settlement.0, "household": v.household, "inventory": v.inventory,
            "home": v.home, "job_site": v.job_site}))
    }).collect();
    rows.sort_by(|a, b| a["uuid"].as_str().cmp(&b["uuid"].as_str()));
    serde_json::json!(rows)
}

fn civil_image(state: &TickState) -> serde_json::Value {
    serde_json::json!({"records": state.settlements.records().collect::<Vec<_>>(),
        "sequence": state.settlement_sequence, "members": members(state), "plugins": state.plugins})
}

fn assert_member_index(state: &TickState) {
    for record in state.settlements.records() {
        let native: std::collections::BTreeSet<_> = state
            .entities
            .slots()
            .filter_map(|slot| {
                let body = state.entities.body(slot);
                (body.alive
                    && state
                        .entities
                        .row(slot)
                        .villager
                        .as_ref()
                        .is_some_and(|v| v.settlement == Some(record.id)))
                .then_some(body.id)
            })
            .collect();
        assert_eq!(
            state
                .settlements
                .member_ids(record.id)
                .collect::<std::collections::BTreeSet<_>>(),
            native
        );
        assert_eq!(state.settlements.member_count(record.id), native.len());
    }
}
#[test]
fn an_outpost_builder_outside_the_civil_radius_still_places_paid_cells() {
    let (mut state, parent, _) = prepared_at(0, [200, 200, 0]);
    // The shared fixture creates the original builder before the migrants.
    let builder = state.entities.slots().next().unwrap();
    state.entities.row_mut(builder).controls.no_ai = false;
    let v = state.entities.row_mut(builder).villager.as_mut().unwrap();
    v.data.profession = Profession::Builder;
    v.activity = crate::villager::Activity::Work;
    fund(&mut state, "spanish_house", 1);
    assert!(plan(&mut state, parent));
    let project = state
        .settlements
        .get(parent)
        .unwrap()
        .projects
        .first()
        .cloned()
        .unwrap();
    let (item, _) = executor::next_material(&state, parent, &project.structure)
        .unwrap()
        .unwrap();
    let uuid = state.entities.row(builder).uuid;
    // The executor only hauls for the project's bound builder (P2-5).
    if let Some(mut record) = state.settlements.get_mut(parent) {
        super::super::bind_builder(&mut record, &project.structure, Some((uuid, None)));
    }
    executor::fetch(&mut state, uuid, &project.structure, [1, 201, 1], item).unwrap();
    state
        .world
        .ensure_blocking(&rustc_hash::FxHashMap::from_iter([(
            crate::boundary::ChunkPos { x: 11, z: 0 },
            0,
        )]));
    for x in 188..=189 {
        state
            .world
            .set_block(x, 200, 2, state.registry.states.stone);
        for y in 201..=203 {
            state.world.set_block(x, y, 2, state.registry.states.air);
        }
    }
    state.world.fold_overlay();
    let position = [188.85, 201.0, 2.5];
    state.entities.set_pos(builder, position);
    assert!(
        !state
            .settlements
            .get(parent)
            .unwrap()
            .contains([188, 201, 2])
    );
    let next = executor::next_cell(&state, parent, &project.structure)
        .unwrap()
        .unwrap();
    assert_eq!(
        crate::villager::building::wait_reason(&state, builder, &project, next),
        "placement_not_advancing"
    );
    assert_eq!(
        executor::status(&state, parent, &project.structure)
            .unwrap()
            .completed_work_units,
        0
    );
    // A real construction admission, not a direct executor call or navigation aid.
    state.tick = 4;
    super::super::tick(&mut state);
    let status = executor::status(&state, parent, &project.structure).unwrap();
    assert_eq!(
        status.completed_work_units, 1,
        "civil bounds cannot hide an in-reach member builder"
    );
    assert!(
        !status.consumed.is_empty(),
        "the placed cell was paid from native carry"
    );
    assert_eq!(state.entities.pos(builder), position);
}

#[test]
fn admission_requires_fed_household_and_loaded_buildable_outpost() {
    let (mut state, parent, mut expedition) = prepared(0);
    let migrants = households::members(&state, parent, &expedition.household);
    {
        let record = state.settlements.get_mut(parent).unwrap();
        record.founding = None;
        record.construction_cursor = 0;
    }
    // Binding the population expands the native radius to 96. Prepare the
    // actual outside-radius candidate, not the old 48-radius fixture's lot.
    let record = state.settlements.get(parent).unwrap();
    expedition.destination = [
        record.anchor[0] + i32::try_from(record.radius).unwrap() + 40,
        200,
        record.anchor[2],
    ];
    flat_site(&mut state, &expedition);
    for entity in &migrants {
        let slot = state.entities.slot_of(*entity).unwrap();
        let villager = state.entities.row_mut(slot).villager.as_mut().unwrap();
        villager.food_level = 0;
        villager.inventory.fill(None);
    }
    admit(&mut state, parent);
    assert!(state.settlements.get(parent).unwrap().founding.is_none());
    for entity in migrants {
        let slot = state.entities.slot_of(entity).unwrap();
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .food_level = 12;
    }
    state
        .settlements
        .get_mut(parent)
        .unwrap()
        .construction_cursor = 0;
    assert_eq!(state.settlements.member_count(parent), FOUNDING_POPULATION);
    assert!(household_ready(&state, parent, &expedition.household, true));
    admit(&mut state, parent);
    let admitted = state
        .settlements
        .get(parent)
        .unwrap()
        .founding
        .as_ref()
        .unwrap();
    assert_eq!(admitted.destination, expedition.destination);
    assert_eq!(admitted.household, expedition.household);
    validate_claims(&state).unwrap();
}

#[test]
fn old_checkpoint_and_wal_rows_default_founding_to_none() {
    use mc_world::native::NativeCheckpoint;
    let (mut state, parent, _) = fixture();
    state.tick = 1;
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    checkpoint(&mut state, &config);
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let mut image = store.read_checkpoint().unwrap();
    let key = persistence::row_key(parent);
    let mut row: serde_json::Value = serde_json::from_slice(&image.globals[&key].payload).unwrap();
    row["value"].as_object_mut().unwrap().remove("founding");
    image.globals.insert(
        key,
        crate::native_checkpoint::record(image.tick, serde_json::to_vec(&row).unwrap()),
    );
    let manifest = serde_json::from_slice(&image.globals["checkpoint"].payload).unwrap();
    crate::native_checkpoint::refresh_metadata(&mut image.globals, &manifest).unwrap();
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick: image.tick,
                previous_tick: image.tick,
                chunks: &[],
                entities: &image.entities,
                players: &image.players,
                globals: &image.globals,
            },
            &state.registry.blocks,
            &state.registry.items,
        )
        .unwrap();
    drop(store);
    let restored = crate::core::recover_dimension_state(&config, Dimension::Overworld).unwrap();
    assert!(restored.settlements.get(parent).unwrap().founding.is_none());
    assert_eq!(members(&restored), members(&state));
    assert_member_index(&restored);

    let tick = image.tick + 1;
    let mut record = state.settlements.get(parent).unwrap().clone();
    record.name = "legacy_wal_town".into();
    record.revision += 1;
    let entry = JournalEntry::Settlement {
        image: Box::new(crate::settlements::RecordImage {
            id: parent,
            value: Some(record),
        }),
        high_water: parent.0,
        tick,
    };
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    writer
        .append(tick, tick, std::slice::from_ref(&entry))
        .unwrap();
    writer.flush().unwrap();
    drop(writer);
    let mut old: serde_json::Value = serde_json::to_value(entry).unwrap();
    old["Settlement"]["image"]["value"]
        .as_object_mut()
        .unwrap()
        .remove("founding");
    let mut payload = serde_json::to_vec(&old).unwrap();
    payload.push(b'\n');
    // Retain the production writer's version/tick framing; replace only this
    // isolated fixture's legacy JSON payload and its length/CRC.
    let path = directory
        .path()
        .join("journal")
        .join(format!("batch-{tick:020}-{tick:020}.bin"));
    let mut frame = std::fs::read(&path).unwrap();
    frame.truncate(32);
    frame[24..28].copy_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
    frame[28..32].copy_from_slice(&crc32fast::hash(&payload).to_le_bytes());
    frame.extend_from_slice(&payload);
    std::fs::write(path, frame).unwrap();
    let restored = crate::core::recover_dimension_state(&config, Dimension::Overworld).unwrap();
    let record = restored.settlements.get(parent).unwrap();
    assert!(record.founding.is_none());
    assert_eq!(
        record.name, "legacy_wal_town",
        "production recovery consumed the old WAL row"
    );
    assert_eq!(members(&restored), members(&state));
    assert_member_index(&restored);
}

#[test]
fn paid_outpost_handover_preserves_bodies_items_and_inherits_committed_style() {
    let (mut state, parent, expedition) = prepared(4);
    assert_eq!(state.settlements.records().count(), 1);
    assert_eq!(
        state.settlements.get(parent).unwrap().buildings.len(),
        1,
        "outpost beds are not allocated to the parent"
    );
    let migrant_ids = households::members(&state, parent, &expedition.household);
    let before: Vec<_> = migrant_ids
        .iter()
        .map(|id| {
            let slot = state.entities.slot_of(*id).unwrap();
            (
                state.entities.row(slot).uuid,
                state.entities.pos(slot),
                state
                    .entities
                    .row(slot)
                    .villager
                    .as_ref()
                    .unwrap()
                    .inventory
                    .clone(),
            )
        })
        .collect();
    let population = state.entities.slots().count();
    assert!(finish(&mut state, parent, &expedition));
    let child = state.settlements.find("colony_2").unwrap().id;
    assert_eq!(
        state.entities.slots().count(),
        population,
        "no replacement bodies"
    );
    assert!(state.settlements.get(parent).unwrap().founding.is_none());
    assert_eq!(state.settlements.member_count(parent), 21);
    assert_eq!(state.settlements.member_count(child), 3);
    let town = state.settlements.get(child).unwrap();
    assert_eq!(town.owner, state.settlements.get(parent).unwrap().owner);
    assert_eq!(town.buildings.len(), 4);
    assert_eq!(
        catalog(&state, town)
            .find(|blueprint| blueprint.kind == BuildingKind::House)
            .unwrap()
            .id,
        "spanish_house"
    );
    for (name, origin) in expedition.sites() {
        assert!(!executor::built_at(&state, parent, name, origin));
        assert!(executor::built_at(&state, child, name, origin));
    }
    for building in town.buildings.values() {
        assert_eq!(building.id.0 >> 32, child.0);
        assert_eq!(building.settlement, child);
        assert!(placement(&state, child, building).is_some());
    }
    for (id, (uuid, position, inventory)) in migrant_ids.iter().zip(before) {
        let slot = state.entities.slot_of(*id).unwrap();
        let row = state.entities.row(slot);
        assert_eq!(row.uuid, uuid);
        assert_eq!(
            state.entities.pos(slot),
            position,
            "no teleport on founding"
        );
        assert_eq!(row.villager.as_ref().unwrap().inventory, inventory);
        assert_eq!(row.villager.as_ref().unwrap().settlement, Some(child));
        assert_eq!(
            row.villager.as_ref().unwrap().household.as_deref(),
            Some(expedition.household.as_str())
        );
    }
    crate::plugin_structures::validate(&state).unwrap();
    persistence::finish_recovery(&mut state).unwrap();
}

#[test]
fn incomplete_or_broken_outpost_cannot_partially_publish_a_daughter() {
    let (mut state, parent, expedition) = prepared(3);
    let before = civil_image(&state);
    assert!(!finish(&mut state, parent, &expedition));
    assert_eq!(civil_image(&state), before);
    let (mut state, parent, expedition) = prepared(4);
    state.world.set_block(96, 200, 0, state.registry.states.air);
    let before = civil_image(&state);
    assert!(!finish(&mut state, parent, &expedition));
    assert_eq!(civil_image(&state), before);
}

fn queue_unfunded_order(state: &mut TickState, parent: SettlementId, blueprint: &str) {
    state
        .block_entities
        .get_mut(&(1, 201, 1))
        .unwrap()
        .slots
        .fill(mc_data::ItemStack::EMPTY);
    super::super::request_build(state, parent, blueprint).unwrap();
    assert!(
        state
            .settlements
            .get(parent)
            .unwrap()
            .orders
            .first()
            .is_some()
    );
    assert!(state.settlements.get(parent).unwrap().projects.is_empty());
}

#[test]
fn pending_order_does_not_block_completed_founding_handover() {
    let (mut state, parent, expedition) = prepared(4);
    queue_unfunded_order(&mut state, parent, "house");
    super::super::plan(&mut state, parent);
    let child = state
        .settlements
        .find("colony_2")
        .expect("completed expedition publishes its daughter")
        .id;
    let record = state.settlements.get(parent).unwrap();
    assert!(record.founding.is_none());
    assert!(record.projects.is_empty());
    assert_eq!(record.orders[0].blueprint, "house");
    assert_eq!(state.settlements.member_count(child), 3);
    for (name, origin) in expedition.sites() {
        assert!(!executor::built_at(&state, parent, name, origin));
        assert!(executor::built_at(&state, child, name, origin));
    }
}

#[test]
fn pending_order_does_not_block_lost_founding_household_cancellation() {
    let (mut state, parent, expedition) = prepared(0);
    queue_unfunded_order(&mut state, parent, "house");
    for member in households::members(&state, parent, &expedition.household)
        .into_iter()
        .take(2)
    {
        households::detach(&mut state, member);
    }
    assert!(state.settlements.get(parent).unwrap().founding.is_some());
    super::super::plan(&mut state, parent);
    let record = state.settlements.get(parent).unwrap();
    assert!(
        record.founding.is_none(),
        "lost family releases the expedition claim"
    );
    assert!(record.projects.is_empty());
    assert_eq!(record.orders[0].blueprint, "house");
    assert_eq!(state.settlements.records().count(), 1);
}

#[test]
fn pending_order_still_blocks_new_founding_construction() {
    let (mut state, parent, expedition) = prepared(0);
    queue_unfunded_order(&mut state, parent, "farm");
    let name = expedition.sites()[0].0;
    fund(&mut state, name, 1);
    let bill = &blueprints::get(&state.registry, name)
        .unwrap()
        .level(1)
        .unwrap()
        .materials;
    assert!(
        funded_variant(
            &state,
            parent,
            bill,
            &executor::source_images(&state, parent)
        )
        .is_some()
    );
    super::super::plan(&mut state, parent);
    let record = state.settlements.get(parent).unwrap();
    assert_eq!(
        record.founding.as_ref().unwrap().destination,
        expedition.destination
    );
    assert!(
        record.projects.is_empty(),
        "even funded new outpost work waits"
    );
    assert_eq!(record.orders[0].blueprint, "farm");
    assert_eq!(state.settlements.records().count(), 1);
}

#[test]
fn recovery_refuses_overlapping_or_missing_household_claims_and_retirement_cancels() {
    let (mut state, parent, expedition) = prepared(0);
    assert!(validate_claims(&state).is_ok());
    let other = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "intruder".into(),
            "another-owner".into(),
            [512, 200, 0],
            32,
        )
        .unwrap();
    // Recovery still refuses a corrupt persisted image; real create/adopt
    // consumers are covered separately and now refuse before publication.
    state.settlements.get_mut(other).unwrap().anchor = expedition.anchor();
    assert!(validate_claims(&state).is_err());
    assert!(state.settlements.remove(other));
    for entity in households::members(&state, parent, &expedition.household) {
        households::detach(&mut state, entity);
    }
    households::finish(&mut state);
    assert!(state.settlements.get(parent).unwrap().founding.is_none());
    assert!(
        !state
            .settlements
            .get(parent)
            .unwrap()
            .households
            .contains_key(&expedition.household)
    );
    state.settlements.get_mut(parent).unwrap().founding = Some(expedition);
    assert!(crate::settlements::validate_record(state.settlements.get(parent).unwrap()).is_err());
}

/// Commit `state` as the world's durable native checkpoint; later WAL frames
/// replay on top of it through production recovery.
pub(in crate::settlements::construction) fn checkpoint(state: &mut TickState, config: &CoreConfig) {
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
    manifest.climate = Some(crate::weather::ClimateImage::capture(state));
    manifest.world_spawn = Some(state.world.world_spawn());
    for child in manifest.dimensions.values_mut() {
        child.tick = tick;
    }
    image
        .globals
        .extend(persistence::checkpoint_rows(state, tick).unwrap());
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
                            .map_err(|error| crate::CoreError::Storage(error.to_string()))
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

#[test]
fn founding_sigkill_publishes_all_or_nothing_at_native_wal_boundary() {
    use std::io::{BufRead, Write};
    if let Ok(stage) = std::env::var("MC_SPAIN_FOUNDING_CRASH_STAGE") {
        let directory =
            std::path::PathBuf::from(std::env::var_os("MC_SPAIN_FOUNDING_CRASH_DIR").unwrap());
        let config = CoreConfig::new(directory.clone(), 42);
        let (mut state, parent, expedition) = prepared(4);
        // One frozen actual tick establishes the production entity/WAL baseline.
        // The next actual tick is exactly a native founding planning admission.
        state.tick = (state.tick / PLAN_CADENCE + 1) * PLAN_CADENCE - 1;
        state.tick_control.set_frozen(true);
        let (baseline, _, _) = crate::tick::run_tick(&mut state, &[]);
        crate::core::journal_entries_for_tick(&mut state, &baseline).unwrap();
        assert!(state.settlements.get(parent).unwrap().founding.is_some());
        checkpoint(&mut state, &config);
        std::fs::write(
            directory.join("before.json"),
            serde_json::to_vec(&civil_image(&state)).unwrap(),
        )
        .unwrap();
        state.tick_control.set_frozen(false);
        let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
        let entries = crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
        assert!(
            state.settlements.get(parent).unwrap().founding.is_none(),
            "actual planning tick must found"
        );
        let child = state.settlements.find("colony_2").unwrap().id;
        let replay = crate::journal::apply_batch_images(
            Replay::default(),
            &[(output.tick, output.tick, entries.clone())],
        );
        assert_eq!(
            replay
                .entities
                .values()
                .filter(|row| row
                    .villager
                    .as_ref()
                    .is_some_and(|v| v.settlement == Some(child)))
                .count(),
            3
        );
        assert!(replay.settlements[&parent.0].founding.is_none());
        assert_eq!(replay.settlements[&child.0].buildings.len(), 4);
        for (name, origin) in expedition.sites() {
            assert!(executor::built_at(&state, child, name, origin));
        }
        std::fs::write(
            directory.join("after.json"),
            serde_json::to_vec(&civil_image(&state)).unwrap(),
        )
        .unwrap();
        if stage == "unpublished" {
            *crate::journal::WAL_WRITE_OBSERVER.lock() = Some((
                directory.join("journal"),
                Box::new(|file, frame| {
                    file.write_all(&frame[..frame.len() / 2]).unwrap();
                    file.sync_all().unwrap();
                    println!("READY unpublished");
                    std::io::stdout().flush().unwrap();
                    loop {
                        std::thread::park();
                    }
                }),
            ));
        }
        let mut writer = JournalWriter::open(&directory, 1).unwrap();
        writer.append(output.tick, output.tick, &entries).unwrap();
        writer.flush().unwrap();
        println!("READY published");
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    }
    struct CrashChild(std::process::Child);
    impl Drop for CrashChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for stage in ["unpublished", "published"] {
        let directory = tempfile::tempdir().unwrap();
        let mut child = CrashChild(
            std::process::Command::new(std::env::current_exe().unwrap())
                .arg("founding_sigkill_publishes_all_or_nothing_at_native_wal_boundary")
                .arg("--nocapture")
                .env("MC_SPAIN_FOUNDING_CRASH_STAGE", stage)
                .env("MC_SPAIN_FOUNDING_CRASH_DIR", directory.path())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let stdout = child.0.stdout.take().unwrap();
        let (sender, ready) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let observed = (|| {
                for line in std::io::BufReader::new(stdout).lines() {
                    let line = line.map_err(|error| error.to_string())?;
                    if line.trim() == format!("READY {stage}") {
                        return Ok(());
                    }
                }
                Err(format!("founding crash child exited before {stage}"))
            })();
            let _ = sender.send(observed);
        });
        ready
            .recv_timeout(std::time::Duration::from_secs(90))
            .expect("founding crash child did not reach its WAL boundary")
            .unwrap();
        reader.join().unwrap();
        child.0.kill().unwrap();
        let status = child.0.wait().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(status.signal(), Some(9));
        }
        let config = CoreConfig::new(directory.path().to_path_buf(), 42);
        let restored = crate::core::recover_dimension_state(&config, Dimension::Overworld).unwrap();
        let expected = if stage == "published" {
            "after.json"
        } else {
            "before.json"
        };
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.path().join(expected)).unwrap())
                .unwrap();
        assert_eq!(
            civil_image(&restored),
            expected,
            "{stage}: records, memberships and ledger publish together"
        );
        crate::plugin_structures::validate(&restored).unwrap();
    }
}

#[test]
fn review_create_command_cannot_publish_inside_an_expedition_claim() {
    let (mut state, parent, expedition) = prepared(0);
    let at = expedition.anchor().map(|value| f64::from(value) + 0.5);
    let executor = villager(&mut state, at);
    let executor = state.entities.body(executor).id;
    let sequence = state.settlement_sequence;
    crate::command_execution::execute(
        &mut state,
        crate::command_console::CONNECTION,
        executor,
        &crate::commands::Command::Settlement(
            crate::settlements::commands::SettlementCommand::Create {
                name: "intruder".into(),
                radius: 32,
            },
        ),
        &mut crate::boundary::ChangeSet::default(),
    );
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    checkpoint(&mut state, &config);
    let restored = crate::core::recover_dimension_state(&config, Dimension::Overworld).unwrap();
    assert!(restored.settlements.find("intruder").is_none());
    assert_eq!(
        state.settlement_sequence, sequence,
        "refusal consumes no identity"
    );
    assert!(restored.settlements.get(parent).unwrap().founding.is_some());
}

#[test]
fn review_command_certificate_cannot_outlive_a_cancelled_expedition_claim() {
    let (mut state, parent, expedition) = prepared_at(0, [192, 200, 0]);
    let executor = state.entities.slots().next().unwrap();
    let executor = state.entities.body(executor).id;
    let (name, origin) = expedition.sites()[0];
    // Ordinary certification accepts physical, player-authored geometry; it
    // must not gain civil authority merely because an expedition reserves it.
    let rotated = blueprints::get(&state.registry, name)
        .unwrap()
        .level(1)
        .unwrap()
        .rotated(Rotation(0));
    for (offset, id) in &rotated.blocks {
        let [x, y, z] = rotated.world(origin, *offset);
        state.world.set_block(x, y, z, id.0);
    }
    let bounds = rotated.bounds(origin);
    state.world.fold_overlay();
    let result = crate::settlements::building_commands::certify(
        &mut state,
        executor,
        Some("owner"),
        false,
        "house",
        bounds.min,
        bounds.max,
    );
    for entity in households::members(&state, parent, &expedition.household) {
        households::detach(&mut state, entity);
    }
    households::finish(&mut state);
    assert!(state.settlements.get(parent).unwrap().founding.is_none());
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    checkpoint(&mut state, &config);
    let restored = crate::core::recover_dimension_state(&config, Dimension::Overworld).unwrap();
    assert!(
        result.is_err(),
        "generic certification cannot borrow an outpost claim"
    );
    assert_eq!(restored.settlements.get(parent).unwrap().buildings.len(), 1);
}

#[test]
fn review_future_daughter_radius_fences_a_second_real_admission() {
    let (mut state, parent, expedition) = prepared(4);
    let other = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "neighbor".into(),
            "owner".into(),
            [272, 201, 0],
            96,
        )
        .unwrap();
    let mut family = Vec::new();
    for index in 0..24 {
        let slot = villager(&mut state, [272.5, 201.0, 0.5]);
        let entity = state.entities.body(slot).id;
        crate::settlements::bind(&mut state, other, entity).unwrap();
        state.entities.row_mut(slot).controls.no_ai = true;
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .food_level = 12;
        if index < 3 {
            family.push(entity);
        }
    }
    let household = households::form(
        &mut state,
        other,
        &family,
        households::HouseholdOrigin::Migration,
    )
    .unwrap();
    let candidate = FoundingExpedition {
        destination: [136, 200, 0],
        household,
    };
    assert!(
        !overlaps(candidate.bounds(), expedition.bounds()),
        "rectangles are disjoint"
    );
    flat_site(&mut state, &candidate);
    state
        .settlements
        .get_mut(other)
        .unwrap()
        .construction_cursor = 2;
    admit(&mut state, other);
    assert!(finish(&mut state, parent, &expedition));
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    checkpoint(&mut state, &config);
    let restored = crate::core::recover_dimension_state(&config, Dimension::Overworld).unwrap();
    assert!(restored.settlements.get(other).unwrap().founding.is_none());
    assert_eq!(restored.settlements.records().count(), 3);
}

#[test]
fn review_cancelled_history_does_not_poison_a_rebuilt_outpost_handover() {
    let (mut state, parent, expedition) = prepared(0);
    let (name, origin) = expedition.sites()[0];
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
    cancel(&mut state, parent);
    state.settlements.get_mut(parent).unwrap().founding = Some(expedition.clone());
    let builder = state.entities.slots().next().unwrap();
    for (name, origin) in expedition.sites() {
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
        fund(&mut state, name, 1);
        let v = state.entities.row_mut(builder).villager.as_mut().unwrap();
        v.data.profession = Profession::Builder;
        v.activity = crate::villager::Activity::Work;
        complete(&mut state, parent, builder);
    }
    assert!(
        finish(&mut state, parent, &expedition),
        "cancelled receipt is history, not live provenance"
    );
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    checkpoint(&mut state, &config);
    let restored = crate::core::recover_dimension_state(&config, Dimension::Overworld).unwrap();
    crate::plugin_structures::validate(&restored).unwrap();
    assert_eq!(restored.settlements.records().count(), 2);
}

#[test]
fn review_population_radius_growth_cannot_steal_a_founding_claim() {
    let (mut state, parent, _) = prepared(0);
    let other = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "growing_neighbor".into(),
            "other".into(),
            [168, 201, 8],
            16,
        )
        .unwrap();
    for _ in 0..24 {
        let slot = villager(&mut state, [168.5, 201.0, 8.5]);
        let entity = state.entities.body(slot).id;
        crate::settlements::bind(&mut state, other, entity).unwrap();
    }
    state.settlements.grow_radii();
    assert_eq!(
        state.settlements.get(other).unwrap().radius,
        44,
        "unobstructed growth remains allowed"
    );
    assert!(state.settlements.get(parent).unwrap().founding.is_some());
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    checkpoint(&mut state, &config);
    crate::core::recover_dimension_state(&config, Dimension::Overworld).unwrap();
}

#[path = "construction_ledger_tests.rs"]
mod ledger;
