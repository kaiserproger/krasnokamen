//! Real redb checkpoint + tick-WAL recovery of migration transit custody at
//! the three cuts (departure, in transit, arrival). Every reopen goes through
//! the production recovery path (`recover_dimension_state` → `finish_recovery`
//! → `validate_custody`); afterwards each member UUID is in exactly one place.
//! The departure runs in a tick whose batch is appended, or not appended for a
//! crash before publication; arrival runs in the settlements tick wiring.
use super::*;
use crate::CoreConfig;
use crate::boundary::ChunkPos;
use crate::journal::JournalWriter;
use crate::settlements::households::{HouseholdOrigin, form};
use crate::settlements::persistence;
use crate::villager::VillagerProfession as Profession;
use std::path::Path;

/// Standable cells on the test floor.
const Y: i32 = 201;
const SOURCE: [i32; 3] = [8, Y, 8];
const DESTINATION: [i32; 3] = [72, Y, 8];

/// One generic ticket simulating chunks x 0..=4, z −2..=2 (blocks x 0..=79):
/// both anchors simulate, the exit and entry edges lie at x 79 and x 0.
fn area(state: &mut TickState) {
    state
        .tickets
        .insert("transit-disk".into(), (ChunkPos { x: 2, z: 0 }, 2));
    let wanted = state.desired_chunks();
    state.world.ensure_blocking(&wanted);
    let tick = state.tick;
    state.refresh_active_chunks(tick);
}

/// Stone at y 200 under air along the bearing row (x 0..80, z 0..16).
fn floor(state: &mut TickState) {
    let stone = mc_data::Identifier::parse("minecraft:stone").unwrap();
    let stone = state.registry.blocks.block(&stone).unwrap().default.0;
    let air = state.registry.states.air;
    for x in 0..80 {
        for z in 0..16 {
            state.world.set_block(x, Y - 1, z, stone);
            for y in Y..=Y + 3 {
                state.world.set_block(x, y, z, air);
            }
        }
    }
    state.world.fold_overlay();
}

/// The checkpoint the core commits: this tick's rows, the settlement and
/// transit rows the WAL deleted dropped, and (unless `keep_bodies`) the
/// removed entity identities tombstoned exactly as the journal thread does.
fn checkpoint(state: &mut TickState, config: &CoreConfig, keep_bodies: bool) {
    use crate::native_checkpoint::{
        encode_entities, encode_players, entity_tombstone, record, refresh_metadata,
    };
    use mc_world::native::NativeCheckpoint;
    let store = crate::native_checkpoint::tests::fixture_store(config).unwrap();
    let tick = state.tick;
    let (players, captured, block_entities) = crate::core::checkpoint_data_for(state, tick);
    let mut entities =
        encode_entities(Dimension::Overworld, &captured, &state.registry.items, tick).unwrap();
    if !keep_bodies {
        let gone = crate::core::checkpoint_tombstones(state);
        entities.extend(gone.into_iter().map(|uuid| entity_tombstone(uuid, tick)));
    }
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
    let rows = persistence::checkpoint_rows(state, tick).unwrap();
    image.globals.retain(|key, _| {
        !(key.starts_with("settlement/") || key.starts_with("transit/")) || rows.contains_key(key)
    });
    image.globals.extend(rows);
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

/// Transit rows of the durable checkpoint image.
fn stored_rows(config: &CoreConfig) -> usize {
    let store = crate::native_checkpoint::tests::fixture_store(config).unwrap();
    let image = store.read_checkpoint().unwrap();
    image
        .globals
        .keys()
        .filter(|key| key.starts_with("transit/"))
        .count()
}

/// One tick. `act` runs before the batch is captured, inside the same tick
/// output; `publish = false` is a crash before the batch is appended.
fn step(
    state: &mut TickState,
    writer: &mut JournalWriter,
    publish: bool,
    act: impl FnOnce(&mut TickState, &mut ChangeSet),
) {
    let (mut output, _, _) = crate::tick::run_tick(state, &[]);
    let changes = std::sync::Arc::get_mut(&mut output.changes).expect("sole tick output");
    act(state, changes);
    let entries = crate::core::journal_entries_for_tick(state, &output).unwrap();
    if publish {
        writer.append(output.tick, output.tick, &entries).unwrap();
    }
}

fn reopen(state: TickState, mut writer: JournalWriter, config: &CoreConfig) -> TickState {
    writer.flush().unwrap();
    drop(writer);
    // Close the last old world's store Arc before reopening it.
    drop(state);
    let mut state = crate::core::recover_dimension_state(config, Dimension::Overworld).unwrap();
    area(&mut state);
    state
}

fn places(state: &TickState, uuid: uuid::Uuid) -> usize {
    let text = uuid.to_string();
    usize::from(state.entities.slot_of_uuid(uuid).is_some())
        + state
            .settlements
            .transit
            .records
            .values()
            .filter(|record| record.members.iter().any(|member| member.uuid == text))
            .count()
}

struct Town {
    state: TickState,
    writer: JournalWriter,
    source: SettlementId,
    destination: SettlementId,
    family: String,
    /// The librarian's, then the child's.
    uuids: [uuid::Uuid; 2],
    exit: BlockPos,
}

/// Two radius-96 towns 64 blocks apart on a real native-store world; a
/// traded librarian and a child (no AI) form a household of the source and
/// stand at the simulated edge east of it. Checkpointed; the WAL is open.
fn town(config: &CoreConfig, directory: &Path) -> Town {
    let mut state = crate::core::recover_dimension_state(config, Dimension::Overworld).unwrap();
    // Tick zero is the database's creation commit, never a second checkpoint.
    let _ = crate::tick::run_tick(&mut state, &[]);
    area(&mut state);
    floor(&mut state);
    let create = |state: &mut TickState, name: &str, anchor| {
        state
            .settlements
            .create(
                &mut state.settlement_sequence,
                name.into(),
                "owner".into(),
                anchor,
                96,
            )
            .unwrap()
    };
    let source = create(&mut state, "source", SOURCE);
    let destination = create(&mut state, "destination", DESTINATION);
    let exit = edge_point(&state, SOURCE, bearing(SOURCE, DESTINATION)).unwrap();
    assert_eq!((exit.x, exit.y, exit.z), (79, Y, 8));
    let villager = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:villager").unwrap())
        .unwrap();
    let book = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:book").unwrap())
        .unwrap();
    let mut members = [EntityId(0); 2];
    for (index, member) in members.iter_mut().enumerate() {
        let id = state.spawn_mob(villager, [8.5 + index as f64, f64::from(Y), 8.5], 20.0);
        let slot = state.entities.slot_of(id).unwrap();
        let row = state.entities.row_mut(slot);
        row.controls.no_ai = true;
        let mut v = crate::villager::VillagerState::default();
        if index == 0 {
            v.data.profession = Profession::Librarian;
            v.data.level = 2;
            v.xp = 12;
            v.inventory[0] = Some(mc_data::ItemStack::new(book, 3));
        } else {
            v.age = -12_000;
        }
        row.villager = Some(Box::new(v));
        crate::villager::sync_appearance(&mut state, slot);
        crate::settlements::bind(&mut state, source, id).unwrap();
        *member = id;
    }
    let family = form(&mut state, source, &members, HouseholdOrigin::Adoption).unwrap();
    let centre = [
        f64::from(exit.x) + 0.5,
        f64::from(Y),
        f64::from(exit.z) + 0.5,
    ];
    for id in members {
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.set_pos(slot, centre);
    }
    let uuids = members.map(|id| state.entities.row(state.entities.slot_of(id).unwrap()).uuid);
    checkpoint(&mut state, config, false);
    let writer = JournalWriter::open(directory, 1).unwrap();
    Town {
        state,
        writer,
        source,
        destination,
        family,
        uuids,
        exit,
    }
}

fn depart_in(t: &mut Town, publish: bool) {
    let (source, destination, exit) = (t.source, t.destination, t.exit);
    let family = t.family.clone();
    step(&mut t.state, &mut t.writer, publish, |state, changes| {
        let toward = bearing(SOURCE, DESTINATION);
        let unit = Unit::Household(&family);
        depart(state, changes, source, unit, destination, exit, toward).unwrap();
    });
}

/// The record's member images, as stored.
fn images(state: &TickState) -> Vec<serde_json::Value> {
    let record = state.settlements.transit.records.values().next().unwrap();
    serde_json::to_value(&record.members)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn departure_cut_recovers_entirely_before_or_entirely_after() {
    for publish in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let config = CoreConfig::new(directory.path().to_path_buf(), 42);
        let mut t = town(&config, directory.path());
        step(&mut t.state, &mut t.writer, true, |_, _| {});
        depart_in(&mut t, publish);
        let (uuids, family, source) = (t.uuids, t.family.clone(), t.source);
        let state = reopen(t.state, t.writer, &config);
        for uuid in uuids {
            assert_eq!(places(&state, uuid), 1, "published {publish}");
            assert_eq!(state.entities.slot_of_uuid(uuid).is_some(), !publish);
        }
        assert_eq!(state.settlements.transit.records.len(), usize::from(publish));
        let source = state.settlements.get(source).unwrap();
        assert_eq!(source.households.contains_key(&family), !publish);
    }
}

#[test]
fn in_transit_cut_recovers_the_record_from_its_checkpoint_row() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut t = town(&config, directory.path());
    depart_in(&mut t, true);
    let stored = images(&t.state);
    checkpoint(&mut t.state, &config, false);
    assert_eq!(stored_rows(&config), 1);
    for _ in 0..3 {
        step(&mut t.state, &mut t.writer, true, |_, _| {});
    }
    let uuids = t.uuids;
    let state = reopen(t.state, t.writer, &config);
    assert_eq!(state.settlements.transit.records.len(), 1);
    assert_eq!(images(&state), stored, "member images round-trip exactly");
    for uuid in uuids {
        assert_eq!(places(&state, uuid), 1);
        assert!(state.entities.slot_of_uuid(uuid).is_none());
    }
}

#[test]
fn arrival_cut_recovers_the_same_villagers_and_drops_the_row() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut t = town(&config, directory.path());
    depart_in(&mut t, true);
    checkpoint(&mut t.state, &config, false);
    let (uuids, family, destination) = (t.uuids, t.family.clone(), t.destination);
    let mut state = reopen(t.state, t.writer, &config);
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    let mut ticks = 0;
    while !state.settlements.transit.records.is_empty() {
        ticks += 1;
        assert!(ticks <= 200, "no arrival in {ticks} ticks");
        // The explicit call is the settlements tick wiring's; with it wired
        // into run_tick the record is already gone and this is a no-op.
        step(&mut state, &mut writer, true, |state, changes| {
            tick(state, changes);
        });
    }
    assert_eq!(stored_rows(&config), 1, "the WAL alone holds the arrival");
    let mut state = reopen(state, writer, &config);
    assert!(state.settlements.transit.records.is_empty());
    for uuid in uuids {
        assert_eq!(places(&state, uuid), 1);
        let slot = state.entities.slot_of_uuid(uuid).unwrap();
        let pos = state.entities.pos(slot);
        let entry = [0.5, f64::from(Y), 8.5];
        assert!(
            pos.iter().zip(entry).all(|(a, b)| (a - b).abs() < 0.01),
            "entered at the west edge: {pos:?}"
        );
        let v = state.entities.row(slot).villager.as_ref().unwrap();
        assert_eq!(v.settlement, Some(destination));
        assert_eq!(v.household.as_deref(), Some(family.as_str()));
    }
    let librarian = state.entities.slot_of_uuid(uuids[0]).unwrap();
    let librarian = state.entities.row(librarian).villager.as_ref().unwrap();
    assert_eq!(
        (
            librarian.data.profession,
            librarian.data.level,
            librarian.xp
        ),
        (Profession::Librarian, 2, 12)
    );
    assert_eq!(
        librarian.inventory[0].as_ref().map(|stack| stack.count),
        Some(3)
    );
    let child = state.entities.slot_of_uuid(uuids[1]).unwrap();
    assert_eq!(
        state.entities.row(child).villager.as_ref().unwrap().age,
        -12_000
    );
    // The next checkpoint deletes the row the WAL tombstoned.
    checkpoint(&mut state, &config, false);
    assert_eq!(stored_rows(&config), 0);
    let writer = JournalWriter::open(directory.path(), 1).unwrap();
    let state = reopen(state, writer, &config);
    for uuid in uuids {
        assert_eq!(places(&state, uuid), 1);
        assert!(state.entities.slot_of_uuid(uuid).is_some());
    }
}

/// Negative control: a checkpoint that kept the departed bodies besides the
/// transit row (no entity tombstones) refuses to open.
#[test]
fn recovery_refuses_a_member_stored_as_body_and_record() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut t = town(&config, directory.path());
    depart_in(&mut t, true);
    checkpoint(&mut t.state, &config, true);
    let mut writer = t.writer;
    writer.flush().unwrap();
    drop(writer);
    drop(t.state);
    let error = crate::core::recover_dimension_state(&config, Dimension::Overworld)
        .err()
        .expect("recovery refuses")
        .to_string();
    assert!(
        error.contains("both a world entity and in transit record"),
        "{error}"
    );
}
