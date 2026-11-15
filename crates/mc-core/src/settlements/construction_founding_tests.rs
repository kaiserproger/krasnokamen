use super::*;
use crate::journal::{JournalEntry, JournalWriter, Replay};
use crate::settlements::allocator::tests::villager;
use crate::settlements::construction::tests::{complete, fixture, fund};
use crate::settlements::{households, persistence};
use crate::{CoreConfig, dimensions::Dimension};

fn flat_site(state: &mut TickState, expedition: &FoundingExpedition) {
    let bounds = expedition.bounds();
    let wanted = (bounds.min[0].div_euclid(16)..=bounds.max[0].div_euclid(16))
        .flat_map(|x| {
            (bounds.min[2].div_euclid(16)..=bounds.max[2].div_euclid(16))
                .map(move |z| (crate::boundary::ChunkPos { x, z }, 0))
        })
        .collect();
    state.world.ensure_blocking(&wanted);
    for x in bounds.min[0]..=bounds.max[0] {
        for z in bounds.min[2]..=bounds.max[2] {
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
        destination: [96, 200, 0],
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
                borrowed_builder: None
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
    let (state, parent, _) = fixture();
    let record = state.settlements.get(parent).unwrap().clone();
    let mut checkpoint = serde_json::to_value(persistence::Row {
        dimension: Dimension::Overworld,
        value: record.clone(),
    })
    .unwrap();
    checkpoint["value"]
        .as_object_mut()
        .unwrap()
        .remove("founding");
    let decoded: persistence::Row = serde_json::from_value(checkpoint).unwrap();
    assert!(decoded.value.founding.is_none());
    let entry = JournalEntry::Settlement {
        image: Box::new(crate::settlements::RecordImage {
            id: parent,
            value: Some(record),
        }),
        high_water: parent.0,
        tick: 1,
    };
    let mut wal = serde_json::to_value(entry).unwrap();
    wal["Settlement"]["image"]["value"]
        .as_object_mut()
        .unwrap()
        .remove("founding");
    let decoded: JournalEntry = serde_json::from_value(wal).unwrap();
    let replay = crate::journal::apply_batch_images(Replay::default(), &[(1, 1, vec![decoded])]);
    assert!(replay.settlements[&parent.0].founding.is_none());
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
            expedition.anchor(),
            32,
        )
        .unwrap();
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
        let _ = crate::core::journal_entries_for_tick(&mut state, &baseline);
        assert!(state.settlements.get(parent).unwrap().founding.is_some());
        checkpoint(&mut state, &config);
        std::fs::write(
            directory.join("before.json"),
            serde_json::to_vec(&civil_image(&state)).unwrap(),
        )
        .unwrap();
        state.tick_control.set_frozen(false);
        let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
        let entries = crate::core::journal_entries_for_tick(&mut state, &output);
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
