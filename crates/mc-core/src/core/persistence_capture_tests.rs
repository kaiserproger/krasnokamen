//! D65's same-stage byte oracle. The runner invokes generation and recovery in
//! separate processes, and always supplies the BASE world to both recoveries.
use super::*;
use crate::boundary::{ChangeSet, WorldSnapshot};
use crate::dimensions::Dimension;
use crate::native_checkpoint::tests::fixture_store;
use crate::settlements::{SettlementId, buildings::BuildingId};
use mc_world::native::NativeCheckpointImage;
use serde_json::{Value, json};
use std::path::Path;

const K: u64 = 1;
const T: u64 = 3;

fn config(path: &Path) -> CoreConfig {
    let mut config = CoreConfig::new(path.to_owned(), 42);
    config.vanilla_data_dir =
        Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    config.checkpoint_every_ticks = u64::MAX;
    config.journal_fsync_every_batches = 1;
    config.view_distance = 1;
    config.simulation_distance = 0;
    config
}

fn registry() -> Arc<CoreRegistry> {
    let mut registry = CoreRegistry::load(Some(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"),
    ))
    .unwrap();
    crate::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    Arc::new(registry)
}

fn output(state: &mut TickState, tick: u64) -> TickOutput {
    state.tick = tick;
    state.wall_clock = || 1_700_000_000_000;
    let dimensions = state
        .other_dimensions
        .iter_mut()
        .map(|(&dimension, child)| (dimension, output(child, tick)))
        .collect();
    TickOutput {
        tick,
        snapshot: Arc::new(WorldSnapshot {
            tick,
            dimension: state.dimension,
            dimensions,
            ..Default::default()
        }),
        changes: Arc::new(ChangeSet::default()),
        times: TickTimes::default(),
    }
}

fn capture(
    state: &mut TickState,
    writer: &mut JournalWriter,
    manifest: &mut CheckpointManifest,
    path: &Path,
) {
    let (tx, rx) = journal_work_channel();
    let (done, _ack) = std::sync::mpsc::channel();
    assert!(start_checkpoint(state, &tx, &done, state.tick));
    let JournalWork::Checkpoint(job) = rx.recv().unwrap() else {
        panic!("checkpoint not captured")
    };
    write_checkpoint(path, &job, &state.plugins, writer, manifest).unwrap();
}

fn dump(path: &Path, image: NativeCheckpointImage) {
    let mut rows = serde_json::Map::new();
    for (key, record) in image.globals {
        rows.insert(format!("global/{key}"), json!({"version": record.version, "captured_tick": record.captured_tick, "payload": record.payload}));
    }
    assert!(image.players.is_empty());
    for entity in image.entities {
        let mut payload = Vec::new();
        mc_nbt::write_named(
            &mut payload,
            "",
            entity.payload.as_ref().expect("live NBT row"),
        )
        .unwrap();
        rows.insert(
            format!("entity/{}", uuid::Uuid::from_bytes(entity.identity)),
            json!({
                "version": 1, "captured_tick": entity.journal_lsn, "payload": payload,
                "dimension": entity.dimension, "position": [entity.position.x, entity.position.z],
                "generated_slot": entity.generated_slot,
            }),
        );
    }
    std::fs::write(path, serde_json::to_vec(&rows).unwrap()).unwrap();
}

fn town(state: &mut TickState, name: &str, anchor: [i32; 3]) -> SettlementId {
    state
        .settlements
        .create(
            &mut state.settlement_sequence,
            name.into(),
            "00000000-0000-4000-8000-000000000065".into(),
            anchor,
            48,
        )
        .unwrap()
}

fn resident(state: &mut TickState) -> crate::entities::EntitySlot {
    let kind = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:villager").unwrap())
        .unwrap();
    let id = state.spawn_mob(kind, [2.5, 201.0, 2.5], 20.0);
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.row_mut(slot).villager = Some(Box::default());
    slot
}

fn cursors(state: &mut TickState, slot: crate::entities::EntitySlot, town: SettlementId) {
    let v = state.entities.row_mut(slot).villager.as_mut().unwrap();
    v.settlement = Some(town);
    v.household = Some(format!("{}-1", town.0));
    v.data.profession = crate::villager::VillagerProfession::Soldier;
    v.mining_progress.lane = 7;
    v.mining_progress.cell = 3;
    v.lumber_progress.cursor = 19;
    v.smelting = Some(crate::villager::smelting::Task {
        furnace: [4, 201, 4],
        phase: crate::villager::smelting::Phase::Deliver,
        carried: Default::default(),
    });
    v.soldier = Some(crate::settlements::army::Soldier {
        barracks: BuildingId((town.0 << 32) | 1),
        order: crate::settlements::army::Order::Guard([8, 201, 8]),
        next_attack: 27,
    });
    crate::villager::persistence::validate_state(v).unwrap();
}

fn rich_record(state: &mut TickState, id: SettlementId, worker: uuid::Uuid) {
    use crate::settlements::{
        buildings::*, construction::OwnerOrder, districts::DistrictPlan, households::*,
    };
    let item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:oak_log").unwrap())
        .unwrap();
    let building_ref =
        json!({"kind":"structure", "id":"bytecheck-store", "purpose":"krasnokamen:warehouse"});
    let fence = json!({"revision":3, "snapshot_hash":"0".repeat(64)});
    let container = |position| json!({"building":building_ref, "dimension":"minecraft:overworld", "position":position});
    let record = state.settlements.get_mut(id).unwrap();
    record.owner_name = Some("Byte Oracle".into());
    record.household_sequence = 1;
    let family = Household {
        id: format!("{}-1", id.0),
        revision: 2,
        origin: HouseholdOrigin::Adoption,
        home: Some([2, 201, 2]),
        needs: Some("food".into()),
        policy: Some("stay".into()),
    };
    record.households.insert(family.id.clone(), family);
    record.building_sequence = 1;
    let barracks = BuildingId((id.0 << 32) | 1);
    record.buildings.insert(
        barracks,
        Building {
            id: barracks,
            settlement: id,
            kind: BuildingKind::Barracks,
            level: 1,
            bounds: mc_script::ScriptWorldBounds::new([8, 201, 8], [10, 204, 10]).unwrap(),
            work_points: vec![[9, 201, 9]],
            revision: 2,
            natural_zone: false,
        },
    );
    record.district = Some(DistrictPlan {
        owner: "bytecheck".into(),
        branch: "estate".into(),
        level: 1,
        buildings: [
            "krasnokamen:manor_hall",
            "krasnokamen:farm",
            "krasnokamen:warehouse",
        ]
        .into_iter()
        .enumerate()
        .map(|(i, purpose)| {
            mc_script::ScriptSettlementBuildingRef::new(
                "structure".into(),
                format!("bytecheck-{i}"),
                purpose.into(),
            )
        })
        .collect(),
    });
    record.planted_trees.insert([12, 201, 12]);
    record.order_seq = 2;
    record.orders.push(OwnerOrder {
        seq: 2,
        blueprint: "house".into(),
        level: 1,
        building: None,
    });
    // A cancelled, unfinished job retains the schema without claiming output
    // custody that only a final job's delivery record can own.
    let mut economy = serde_json::to_value(&record.economy).unwrap();
    economy["production"]["sequence"] = json!(1);
    economy["production"]["jobs"] = json!({"bytecheck-job":{
        "id":"bytecheck-job", "owner":"bytecheck", "worker":worker,
        "recipe":"krasnokamen:planks", "recipe_revision":"bytecheck-v1", "duration":20,
        "count":1, "done":0, "station":[4,201,4], "stand_at":[4,201,5],
        "building":building_ref, "dimension":"minecraft:overworld", "accepted_tick":1,
        "next_due":21, "paused":"paused", "cancelled":true, "receipts":[]
    }});
    economy["commerce"]["employments"] = json!({"1":{
        "worker":worker, "treasury":{"Player":{"uuid":"00000000-0000-4000-8000-000000000066"}},
        "wage":2, "period_ticks":20, "next_due":21, "budget":20, "earned":4, "paid":2,
        "active":false, "attempt":1, "settled":{"op":"1#1","amount":2}
    }});
    economy["caravan"]["shipments"] = json!({"bytecheck-shipment":{
        "id":"bytecheck-shipment", "owner":"bytecheck", "settlement":id,
        "source":container([1,201,1]), "destination":container([6,201,6]),
        "carrier":worker, "manifest":mc_data::ItemStack::new(item,2), "quantity":2,
        "waypoints":[[1,201,2],[6,201,5]], "epoch":1, "max_ticks":100,
        "game_deadline":101, "paid_mandate":null, "source_fence":fence,
        "destination_fence":fence, "carrier_fence":fence, "status":"cancelled",
        "blocked":null, "cursor":1, "ticks":3, "attempt":1, "loaded":false,
        "carry":[], "carrier_live_fence":null, "load":null, "unload":null, "delivery":null
    }});
    record.economy = serde_json::from_value(economy).unwrap();
    crate::settlements::validate_record(record).unwrap();
}

fn structure_advanced(plugins: &Value) -> bool {
    plugins["domains"]["@native-structures"]
        .as_object()
        .unwrap()
        .iter()
        .any(|(key, value)| {
            key.starts_with("structure/")
                && value["snapshot"]["completed_work_units"].as_u64().unwrap() > 0
        })
}

fn assert_rows(image: &NativeCheckpointImage) {
    let towns: Vec<Value> = image
        .globals
        .iter()
        .filter(|(k, _)| k.starts_with("settlement/"))
        .map(|(_, r)| serde_json::from_slice(&r.payload).unwrap())
        .collect();
    assert!(towns.len() >= 2);
    assert!(towns.iter().any(|r| r["dimension"] != towns[0]["dimension"]
        && r["value"]["name"] == towns[0]["value"]["name"]));
    let r = towns
        .iter()
        .find(|r| !r["value"]["households"].as_object().unwrap().is_empty())
        .unwrap();
    for field in [
        "households",
        "buildings",
        "planted_trees",
        "projects",
        "orders",
    ] {
        assert!(
            !r["value"][field].as_array().map_or_else(
                || r["value"][field].as_object().unwrap().is_empty(),
                Vec::is_empty
            ),
            "{field}"
        );
    }
    assert!(r["value"]["district"].is_object());
    assert!(
        !r["value"]["economy"]["production"]["jobs"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert!(
        !r["value"]["economy"]["commerce"]["employments"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert!(
        !r["value"]["economy"]["caravan"]["shipments"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert!(image.globals.contains_key("settlement_sequence"));
    assert_eq!(
        image
            .globals
            .keys()
            .filter(|k| k.starts_with("transit/"))
            .count(),
        2
    );
    let nbt = image
        .entities
        .iter()
        .map(|e| format!("{:?}", e.payload))
        .collect::<String>();
    for key in [
        "KrasnokamenMiningProgress",
        "KrasnokamenLumberProgress",
        "KrasnokamenSmelting",
        "KrasnokamenSoldier",
    ] {
        assert!(nbt.contains(key), "{key}");
    }
    let plugins: Value = serde_json::from_slice(&image.globals["plugins"].payload).unwrap();
    assert!(
        structure_advanced(&plugins),
        "construction must have advanced, not merely admitted"
    );
}

fn assert_tail(entries: &[JournalEntry]) {
    let mut kinds = [false; 5];
    fn visit(entries: &[JournalEntry], kinds: &mut [bool; 5]) {
        for entry in entries {
            match entry {
                JournalEntry::Dimension { entries, .. } => visit(entries, kinds),
                JournalEntry::Settlement { image, .. } => {
                    kinds[usize::from(image.value.is_none())] = true
                }
                JournalEntry::Transit { record, .. } => {
                    kinds[2 + usize::from(record.is_none())] = true
                }
                JournalEntry::Entity {
                    villager: Some(v), ..
                } if v.settlement.is_some()
                    && v.mining_progress.lane == 7
                    && v.lumber_progress.cursor == 19
                    && v.smelting.is_some()
                    && v.soldier.is_some() =>
                {
                    kinds[4] = true
                }
                _ => {}
            }
        }
    }
    visit(entries, &mut kinds);
    assert_eq!(
        kinds, [true; 5],
        "tail must exercise live images, tombstones and all cursors"
    );
}

#[test]
#[ignore = "prepares the immutable shared store for tools/dev/bytecheck.sh"]
fn persistence_byte_seed() {
    let out = PathBuf::from(
        std::env::var_os("CORE_UNLOAD_OUT").expect("bytecheck seed output directory"),
    );
    let config = config(&out.join("world"));
    assert!(!config.world_dir.exists(), "bytecheck seed must be fresh");
    drop(fixture_store(&config).unwrap());
}

#[test]
fn persistence_byte_capture() {
    let temporary = tempfile::tempdir().unwrap();
    let out = std::env::var_os("CORE_UNLOAD_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| temporary.path().to_owned());
    std::fs::create_dir_all(&out).unwrap();
    let world = out.join("world");
    let config = config(&world);
    let store = fixture_store(&config).unwrap();
    let registry = registry();
    let (mut state, mut manifest, _) =
        recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
    state.wall_clock = || 1_700_000_000_000;
    crate::terrain::stage_data_field(
        &mut state.native_globals,
        crate::entity_uuid::SALT_FIELD,
        "00000000000040008000000000000065",
    )
    .unwrap();
    assert!(!crate::entity_uuid::install_salt(&mut state).unwrap());
    output(&mut state, K);
    // The real construction admission and a work advance populate the ledger.
    crate::settlements::construction::tests::open_plan(&mut state);
    let (mut state, id, builder) = crate::settlements::construction::tests::fixture_in(state);
    state.settlements.rename(id, "same".into()).unwrap();
    crate::settlements::construction::tests::fund(&mut state, "house", 1);
    crate::settlements::construction::tests::start(&mut state, id, "house", 1, None);
    for _ in 0..16 {
        crate::settlements::construction::tests::attempt(&mut state, id, builder);
        if structure_advanced(&serde_json::to_value(&state.plugins).unwrap()) {
            break;
        }
    }
    assert!(structure_advanced(
        &serde_json::to_value(&state.plugins).unwrap()
    ));
    assert!(!state.settlements.get(id).unwrap().projects.is_empty());
    // Clear the builder binding before installing a soldier cursor on the body.
    let record = state.settlements.get_mut(id).unwrap();
    for project in &mut record.projects {
        project.builder = None;
    }
    let worker = state.entities.row(builder).uuid;
    rich_record(&mut state, id, worker);
    cursors(&mut state, builder, id);
    let deleted = town(&mut state, "deleted", [512, 201, 512]);
    let nether = state.other_dimensions.get_mut(&Dimension::Nether).unwrap();
    nether.settlement_sequence = state.settlement_sequence;
    town(nether, "same", [8, 64, 8]);
    state.settlement_sequence = nether.settlement_sequence;
    let transit_slot = resident(&mut state);
    let transit_uuid = state.entities.row(transit_slot).uuid;
    let stable_slot = resident(&mut state);
    let stable_uuid = state.entities.row(stable_slot).uuid;
    std::fs::write(
        out.join("stable-transit.json"),
        serde_json::to_vec(&stable_uuid).unwrap(),
    )
    .unwrap();
    // Use the production entity image, not a hand-written ReplayEntity.
    let initial_output = output(&mut state, K);
    let first = journal_entries_for_tick(&mut state, &initial_output).unwrap();
    let replay = crate::journal::apply_batch_images(Replay::default(), &[(K, K, first.clone())]);
    let member = replay.entities[&entity_id_int(state.entities.body(transit_slot).id)].clone();
    let stable_member =
        replay.entities[&entity_id_int(state.entities.body(stable_slot).id)].clone();
    state.entities.remove(transit_slot);
    let stable_slot = state.entities.slot_of_uuid(stable_uuid).unwrap();
    state.entities.remove(stable_slot);
    crate::settlements::transit::bytecheck_put(
        &mut state,
        Some(stable_member.clone()),
        stable_uuid,
    );
    crate::settlements::transit::bytecheck_put(&mut state, Some(member.clone()), transit_uuid);
    let mut writer = JournalWriter::open_test(&world, 1).unwrap();
    // Regather after transit custody changes; one physical batch per tick.
    let gathered = journal_entries_for_tick(&mut state, &initial_output).unwrap();
    writer.append(K, K, &gathered).unwrap();
    capture(&mut state, &mut writer, &mut manifest, &world);
    let image = store.read_checkpoint().unwrap();
    assert_rows(&image);
    dump(&out.join("A.json"), image);
    let mut tail = Vec::new();
    for tick in K + 1..=T {
        let output = output(&mut state, tick);
        if tick == K + 1 {
            state.settlements.get_mut(id).unwrap().owner_name = Some("Tail Owner".into());
            state.settlements.remove(deleted);
            crate::settlements::transit::bytecheck_put(
                &mut state,
                Some(member.clone()),
                transit_uuid,
            );
            crate::settlements::transit::bytecheck_put(
                &mut state,
                Some(stable_member.clone()),
                stable_uuid,
            );
            cursors(&mut state, builder, id);
            state
                .entities
                .row_mut(builder)
                .villager
                .as_mut()
                .unwrap()
                .xp = 1;
        } else {
            crate::settlements::transit::bytecheck_put(&mut state, None, transit_uuid);
        }
        let entries = journal_entries_for_tick(&mut state, &output).unwrap();
        writer.append(tick, tick, &entries).unwrap();
        tail.extend(entries);
    }
    assert_tail(&tail);
    writer.flush().unwrap();
    drop(writer);
    drop(state);
    drop(store);
    let wal = out.join("B");
    std::fs::create_dir(&wal).unwrap();
    for entry in std::fs::read_dir(world.join("journal")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name().to_string_lossy().starts_with("batch-") {
            let name = entry.file_name();
            let text = name.to_str().unwrap();
            let ticks: Vec<u64> = text
                .trim_start_matches("batch-")
                .trim_end_matches(".bin")
                .split('-')
                .map(|v| v.parse().unwrap())
                .collect();
            let (lo, hi) = (ticks[0], ticks[1]);
            let identity = crate::journal::test_wal_identity(&world);
            assert!(crate::journal::read_batch_file(&entry.path(), identity).is_some());
            if hi > K {
                assert_eq!(lo, hi);
                std::fs::copy(entry.path(), wal.join(entry.file_name())).unwrap();
            }
        }
    }
    assert_eq!(std::fs::read_dir(wal).unwrap().count(), (T - K) as usize);
}

#[test]
#[ignore = "requires the base-world copy supplied by tools/dev/bytecheck.sh"]
fn persistence_byte_reopen() {
    let out =
        PathBuf::from(std::env::var_os("CORE_UNLOAD_OUT").expect("bytecheck output directory"));
    let world = out.join("live-input"); // a fresh copy of the BASE input, made by the runner
    let config = config(&world);
    let registry = registry();
    let store = fixture_store(&config).unwrap();
    let (mut state, mut manifest, _) =
        recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
    assert_eq!(state.tick, T);
    assert_eq!(manifest.tick, K);
    let stable_uuid: uuid::Uuid =
        serde_json::from_slice(&std::fs::read(out.join("reopen-transit.json")).unwrap()).unwrap();
    let stable_key = format!("transit/{stable_uuid}");
    let recovered_rows = crate::settlements::persistence::checkpoint_rows(&state, T).unwrap();
    assert!(recovered_rows.contains_key(&stable_key));
    assert_eq!(
        recovered_rows
            .keys()
            .filter(|key| key.starts_with("transit/"))
            .count(),
        1
    );
    let mut writer =
        JournalWriter::open_recovered_test(&world, 1, manifest.tick, state.tick).unwrap();
    capture(&mut state, &mut writer, &mut manifest, &world);
    assert_eq!(manifest.tick, T);
    let image = store.read_checkpoint().unwrap();
    assert_eq!(
        image.globals[&stable_key].payload,
        recovered_rows[&stable_key].payload
    );
    dump(&out.join("C-live.json"), image);
}
