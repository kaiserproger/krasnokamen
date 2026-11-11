//! Block log: grief → rollback → restore through the real tick, inspector,
//! retention and table/WAL persistence.

use std::sync::Arc;

use super::*;
use crate::boundary::{ClickMode, DigAction, Hand, PlayerInput, TickOutput};
use crate::registry::CoreRegistry;
use crate::tick::{BlockEntityState, run_tick};
use crate::world::ChunkWorld;

const Y: i32 = 200;
const GRIEFER: ConnectionId = ConnectionId(1);
const ADMIN: ConnectionId = ConnectionId(2);

fn test_state() -> TickState {
    let registry = Arc::new(CoreRegistry::load(None).expect("registry loads"));
    let terrain = mc_worldgen::TerrainGenerator::new(42, Arc::clone(&registry.blocks));
    let spawn = terrain.locate_safe_spawn().expect("test terrain spawn");
    let world_spawn = [spawn.block_x, spawn.surface_y + 1, spawn.block_z];
    let generator: Arc<dyn mc_world::ChunkGenerator> = Arc::new(terrain);
    let world = ChunkWorld::new(
        42,
        generator,
        &registry,
        mc_world::BlockStateId(registry.states.air),
        world_spawn,
        mc_world::OVERWORLD_GEOMETRY,
    );
    let mut state = TickState::new(registry, world, 2, 512);
    state.block_log = BlockLog::new(BlockLogSettings::default());
    state
}

fn item(state: &TickState, name: &str, count: i32) -> ItemStack {
    let id = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(name).expect("id"))
        .expect("item");
    ItemStack::new(id, count)
}

fn block(state: &TickState, name: &str) -> u32 {
    let id = mc_data::Identifier::parse(name).expect("id");
    state.registry.blocks.block(&id).expect("block").default.0
}

type Drains = Vec<(ConnectionId, Vec<PlayerChange>)>;

struct Rig {
    state: TickState,
    griefer: EntityId,
    /// Origin cell on the platform (one above the stone floor).
    at: BlockPos,
}

impl Rig {
    fn new() -> Self {
        let mut state = test_state();
        let _ = run_tick(
            &mut state,
            &[
                PlayerInput::Join {
                    connection: GRIEFER,
                    uuid: uuid::Uuid::new_v4(),
                    name: "Griefer".to_owned(),
                    permission_level: 0,
                },
                PlayerInput::Join {
                    connection: ADMIN,
                    uuid: uuid::Uuid::new_v4(),
                    name: "Admin".to_owned(),
                    permission_level: 4,
                },
            ],
        );
        let want = state.desired_chunks();
        state.world.ensure_blocking(&want);
        let griefer = state.connections.entity_of(GRIEFER).expect("joined");
        let slot = state.entities.slot_of(griefer).expect("slot");
        let spawn = state.entities.pos(slot);
        let at = BlockPos {
            x: spawn[0].floor() as i32,
            y: Y + 1,
            z: spawn[2].floor() as i32,
        };
        let stone = state.registry.states.stone;
        for dx in -6..=6 {
            for dz in -4..=4 {
                state.world.set_block(at.x + dx, Y, at.z + dz, stone);
            }
        }
        state.world.fold_overlay();
        state.entities.set_pos(
            slot,
            [
                f64::from(at.x) + 0.5,
                f64::from(Y + 1),
                f64::from(at.z) + 2.5,
            ],
        );
        let admin = state.connections.entity_of(ADMIN).expect("joined");
        let admin_slot = state.entities.slot_of(admin).expect("slot");
        state.entities.set_pos(
            admin_slot,
            [
                f64::from(at.x) + 0.5,
                f64::from(Y + 1),
                f64::from(at.z) - 3.5,
            ],
        );
        Self { state, griefer, at }
    }

    fn cell(&self, dx: i32) -> BlockPos {
        BlockPos {
            x: self.at.x + dx,
            ..self.at
        }
    }

    /// One tick plus its journal, which seals the block log like production.
    fn tick(
        &mut self,
        inputs: &[PlayerInput],
    ) -> (TickOutput, Drains, Vec<crate::journal::JournalEntry>) {
        let (output, drains, _) = run_tick(&mut self.state, inputs);
        let entries = crate::core::journal_entries_for_tick(&mut self.state, &output);
        (output, drains, entries)
    }

    fn chest(&mut self, pos: BlockPos, items: &[ItemStack]) {
        let chest = block(&self.state, "minecraft:chest");
        self.state.world.set_block(pos.x, pos.y, pos.z, chest);
        self.state.world.fold_overlay();
        let mut slots = vec![ItemStack::EMPTY; 27];
        slots[..items.len()].clone_from_slice(items);
        self.state.block_entities.insert(
            (pos.x, pos.y, pos.z),
            BlockEntityState {
                slots,
                furnace: None,
            },
        );
    }

    fn dig(&mut self, pos: BlockPos) {
        crate::tick::mining_tests::ready_to_finish(&mut self.state, GRIEFER, pos);
        self.tick(&[PlayerInput::Dig {
            connection: GRIEFER,
            position: pos,
            action: DigAction::Finish,
            sequence: 1,
        }]);
    }

    fn use_on(&mut self, connection: ConnectionId, pos: BlockPos) -> Drains {
        self.tick(&[PlayerInput::UseItemOn {
            connection,
            hand: Hand::Main,
            position: pos,
            face: 1,
            cursor: [0.5, 1.0, 0.5],
            sequence: 2,
        }])
        .1
    }

    fn click(&mut self, slot: i16) {
        let menu = self.state.players[&self.griefer]
            .open
            .as_ref()
            .expect("open menu");
        let (window, state_id) = (menu.window, menu.state_id);
        self.tick(&[PlayerInput::ContainerClick {
            connection: GRIEFER,
            window,
            state_id,
            slot,
            button: 0,
            mode: ClickMode::Pickup,
        }]);
    }

    fn command(&mut self, command: &str) -> Vec<String> {
        let (_, drains, _) = self.tick(&[PlayerInput::Command {
            connection: ADMIN,
            command: command.to_owned(),
            permission_level: 4,
        }]);
        chat(&drains, ADMIN)
    }

    fn block_at(&self, pos: BlockPos) -> Option<u32> {
        self.state.world.block_at(pos.x, pos.y, pos.z)
    }

    fn storage(&self, pos: BlockPos) -> Option<Vec<ItemStack>> {
        self.state
            .block_entities
            .get(&(pos.x, pos.y, pos.z))
            .map(|record| record.slots.clone())
    }
}

fn chat(drains: &Drains, connection: ConnectionId) -> Vec<String> {
    drains
        .iter()
        .filter(|(target, _)| *target == connection)
        .flat_map(|(_, queue)| queue.iter())
        .filter_map(|change| match change {
            PlayerChange::Chat { message } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn restored_sensor_retains_its_frequency_and_active_deadline() {
    let mut rig = Rig::new();
    let pos = rig.cell(0);
    let name = mc_data::Identifier::parse("minecraft:sculk_sensor").unwrap();
    let default = rig.state.registry.blocks.block(&name).unwrap().default;
    let mut props = rig
        .state
        .registry
        .blocks
        .by_id(default)
        .unwrap()
        .properties
        .clone();
    for (key, value) in &mut props {
        if key == "sculk_sensor_phase" {
            *value = "active".into();
        } else if key == "power" {
            *value = "10".into();
        }
    }
    let active = rig
        .state
        .registry
        .blocks
        .by_name_and_props(&name, &props)
        .unwrap()
        .0;
    let image = mc_nbt::Tag::Compound(vec![
        (
            "id".into(),
            mc_nbt::Tag::String("minecraft:sculk_sensor".into()),
        ),
        ("last_vibration_frequency".into(), mc_nbt::Tag::Int(7)),
        (
            "solaris:phase_until".into(),
            mc_nbt::Tag::Long((rig.state.tick + 20) as i64),
        ),
    ]);
    let mut nbt = Vec::new();
    mc_nbt::write_network(&mut nbt, &image).unwrap();
    rig.state.block_log.restores.push((
        pos,
        EntityImage {
            slots: None,
            nbt: Some(nbt),
        },
    ));
    rig.state.write_block(pos, active);
    crate::dimensions::commands::commit(&mut rig.state, &mut Default::default());
    assert_eq!(crate::sculk::comparator(&rig.state, pos), Some(7));
    rig.state.tick += 1;
    crate::sculk::run(&mut rig.state, &mut Vec::new());
    assert_eq!(crate::sculk::comparator(&rig.state, pos), Some(7));
}

#[test]
fn sealed_record_exposes_committing_tick() {
    let mut rig = Rig::new();
    let pos = rig.cell(0);
    let stone = rig.state.registry.states.stone;
    rig.state.world.set_block(pos.x, pos.y, pos.z, stone);
    rig.state.world.fold_overlay();
    rig.dig(pos);
    let sealed = rig.state.block_log.records.back().expect("break record");
    assert_eq!(sealed.tick, Some(rig.state.tick));
    assert!(
        describe(&rig.state, sealed, sealed.time).contains(&format!("[tick={}]", rig.state.tick))
    );
    let mut old = sealed.clone();
    old.tick = None;
    assert!(describe(&rig.state, &old, old.time).contains("[tick=unknown]"));
}

#[test]
fn grief_rollback_restores_blocks_and_chests_exactly_and_restore_reapplies() {
    let mut rig = Rig::new();
    let (stone_cell, dirt_cell, chest_cell, broken_cell) =
        (rig.cell(-2), rig.cell(-1), rig.cell(0), rig.cell(2));
    let stone = rig.state.registry.states.stone;
    let air = rig.state.registry.states.air;
    rig.state
        .world
        .set_block(stone_cell.x, stone_cell.y, stone_cell.z, stone);
    let loot = [
        item(&rig.state, "minecraft:diamond", 64),
        item(&rig.state, "minecraft:iron_ingot", 32),
    ];
    rig.chest(chest_cell, &loot);
    let gold = [item(&rig.state, "minecraft:gold_ingot", 16)];
    rig.chest(broken_cell, &gold);
    let chest_before = rig.storage(chest_cell).expect("chest");
    let broken_before = rig.storage(broken_cell).expect("chest");
    let chest_block = rig.block_at(broken_cell).expect("chest block");
    rig.state
        .players
        .get_mut(&rig.griefer)
        .expect("griefer")
        .inventory
        .slots[0] = item(&rig.state, "minecraft:dirt", 1);

    // Grief: break stone, place dirt, steal two stacks, break a full chest.
    rig.dig(stone_cell);
    assert_eq!(rig.block_at(stone_cell), Some(air));
    rig.use_on(GRIEFER, BlockPos { y: Y, ..dirt_cell });
    let dirt = block(&rig.state, "minecraft:dirt");
    assert_eq!(rig.block_at(dirt_cell), Some(dirt));
    rig.use_on(GRIEFER, chest_cell);
    rig.click(0);
    rig.click(54);
    rig.click(1);
    rig.click(55);
    let stolen = rig.storage(chest_cell).expect("chest");
    assert!(stolen[0].is_empty() && stolen[1].is_empty());
    rig.dig(broken_cell);
    assert_eq!(rig.block_at(broken_cell), Some(air));
    assert!(rig.storage(broken_cell).is_none());

    let lookup = rig.command("co lookup u:griefer t:1h");
    assert!(lookup[0].contains("5 result(s)"), "{lookup:?}");
    assert!(
        lookup
            .iter()
            .any(|line| line.contains("Griefer broke stone")),
        "{lookup:?}"
    );
    assert!(
        lookup
            .iter()
            .any(|line| line.contains("Griefer placed dirt")),
        "{lookup:?}"
    );
    assert!(
        lookup.iter().any(|line| line.contains("took 64 diamond")),
        "{lookup:?}"
    );
    assert!(
        lookup
            .iter()
            .any(|line| line.contains("took 32 iron_ingot")),
        "{lookup:?}"
    );

    let reply = rig.command("co rollback u:griefer t:1h");
    assert_eq!(
        reply,
        ["Rollback complete: 5 change(s) rolled back, 0 skipped (changed since)"]
    );
    assert_eq!(rig.block_at(stone_cell), Some(stone));
    assert_eq!(rig.block_at(dirt_cell), Some(air));
    assert_eq!(rig.storage(chest_cell), Some(chest_before.clone()));
    assert_eq!(rig.block_at(broken_cell), Some(chest_block));
    assert_eq!(rig.storage(broken_cell), Some(broken_before));
    assert!(
        rig.state
            .block_log
            .records
            .iter()
            .all(|record| record.rolled_back)
    );
    // A second rollback finds nothing left to revert.
    assert_eq!(
        rig.command("co rollback u:griefer t:1h"),
        ["Rollback complete: 0 change(s) rolled back, 0 skipped (changed since)"]
    );

    let reply = rig.command("co restore u:griefer t:1h");
    assert_eq!(
        reply,
        ["Restore complete: 5 change(s) restored, 0 skipped (changed since)"]
    );
    assert_eq!(rig.block_at(stone_cell), Some(air));
    assert_eq!(rig.block_at(dirt_cell), Some(dirt));
    assert_eq!(rig.storage(chest_cell), Some(stolen));
    assert_eq!(rig.block_at(broken_cell), Some(air));
    assert!(rig.storage(broken_cell).is_none());
}

#[test]
fn rollback_skips_cells_someone_else_changed_since() {
    let mut rig = Rig::new();
    let cell = rig.cell(-2);
    let stone = rig.state.registry.states.stone;
    rig.state.world.set_block(cell.x, cell.y, cell.z, stone);
    rig.state.world.fold_overlay();
    rig.dig(cell);
    // Another source fills the hole afterwards; the rollback must not erase it.
    let planks = block(&rig.state, "minecraft:oak_planks");
    rig.state.world.set_block(cell.x, cell.y, cell.z, planks);
    rig.state.world.fold_overlay();
    assert_eq!(
        rig.command("co rollback u:Griefer t:1h"),
        ["Rollback complete: 0 change(s) rolled back, 1 skipped (changed since)"]
    );
    assert_eq!(rig.block_at(cell), Some(planks));
}

#[test]
fn inspector_prints_history_and_keeps_the_block() {
    let mut rig = Rig::new();
    let cell = rig.cell(-2);
    let stone = rig.state.registry.states.stone;
    rig.state.world.set_block(cell.x, cell.y, cell.z, stone);
    rig.state.world.fold_overlay();
    rig.dig(cell);
    rig.state.world.set_block(cell.x, cell.y, cell.z, stone);
    rig.state.world.fold_overlay();
    assert_eq!(
        rig.command("co inspect"),
        ["Inspector enabled: click a block to see its history"]
    );
    let (_, drains, _) = rig.tick(&[PlayerInput::Dig {
        connection: ADMIN,
        position: cell,
        action: DigAction::Start,
        sequence: 3,
    }]);
    let lines = chat(&drains, ADMIN);
    assert!(
        lines[0].contains(&format!("{} {} {}", cell.x, cell.y, cell.z)),
        "{lines:?}"
    );
    assert!(lines[1].contains("Griefer broke stone"), "{lines:?}");
    assert_eq!(rig.block_at(cell), Some(stone));
    assert_eq!(rig.command("co i"), ["Inspector disabled"]);
}

#[test]
fn parse_accepts_coreprotect_parameters() {
    let command = parse(&["rollback", "u:Alice,#fire", "t:1h30m", "r:12"]).expect("parses");
    assert_eq!(command.users, ["alice", "#fire"]);
    assert_eq!(command.since_secs, Some(5_400));
    assert_eq!(command.radius, Some(12));
    assert_eq!(parse(&["l", "t:2.5d", "3"]).expect("lookup").page, 3);
    assert!(
        parse(&["rollback", "u:Alice"]).is_err(),
        "rollback needs t:"
    );
    assert!(parse(&["lookup", "t:soon"]).is_err());
    assert!(parse(&["inspect", "u:Alice"]).is_err());
    assert_eq!(duration("1w"), Some(604_800));
}

fn record(seq: u64, time: u64) -> Record {
    Record {
        seq,
        tick: Some(123),
        time,
        actor: Arc::from("Alice"),
        pos: [1, 64, seq as i32],
        change: Change::Block {
            before: 1,
            after: 0,
            entity: None,
        },
        rolled_back: false,
    }
}

#[test]
fn retention_prunes_by_count_then_age() {
    let mut log = BlockLog::new(BlockLogSettings {
        enabled: true,
        retention_secs: 100,
        max_records: 3,
    });
    log.records
        .extend((0..5).map(|seq| record(seq, 1_000 + seq)));
    log.next_seq = 5;
    log.evict(1_000);
    assert_eq!(
        log.records.iter().map(|r| r.seq).collect::<Vec<_>>(),
        [2, 3, 4]
    );
    log.evict(1_103);
    assert_eq!(
        log.records.iter().map(|r| r.seq).collect::<Vec<_>>(),
        [3, 4]
    );
    assert_eq!(log.floor(), 3);
    log.evict(2_000);
    assert_eq!(
        log.floor(),
        5,
        "an empty log prunes everything below next_seq"
    );
}

fn required_blocks() -> mc_world::BlockRegistry {
    mc_world::BlockRegistry::from_report(&mc_data::blocks::solaris_required_blocks_report())
        .expect("required blocks")
}

fn id(name: &str) -> mc_data::Identifier {
    mc_data::Identifier::parse(name).expect("identifier")
}

#[test]
fn table_rows_then_wal_tail_rebuild_the_log() {
    let directory = tempfile::tempdir().expect("tempdir");
    let store =
        mc_world::native::NativeStore::create(&directory.path().join("state.redb"), [7; 32])
            .expect("store");
    let blocks = required_blocks();
    let items = mc_data::items::solaris_required_items();
    let now = unix_now();
    let stone = |seq| Record {
        change: Change::Block {
            before: blocks
                .block(&id("minecraft:stone"))
                .expect("stone")
                .default
                .0,
            after: blocks.block(&id("minecraft:air")).expect("air").default.0,
            entity: None,
        },
        ..record(seq, now)
    };
    let mut backlog = Backlog::default();
    backlog.absorb(Persist {
        deltas: vec![
            (
                Dimension::Overworld,
                Delta {
                    records: vec![stone(0), stone(1)],
                    marks: Vec::new(),
                },
            ),
            (
                Dimension::Nether,
                Delta {
                    records: vec![stone(0)],
                    marks: Vec::new(),
                },
            ),
        ],
        floors: vec![(Dimension::Overworld, 0), (Dimension::Nether, 0)],
    });
    backlog.write(&store, &blocks, &items).expect("write");
    // A later checkpoint marks record 1 and prunes record 0.
    backlog.absorb(Persist {
        deltas: vec![(
            Dimension::Overworld,
            Delta {
                records: Vec::new(),
                marks: vec![(1, true)],
            },
        )],
        floors: vec![(Dimension::Overworld, 1), (Dimension::Nether, 0)],
    });
    backlog.write(&store, &blocks, &items).expect("write");
    drop(store);
    let store = mc_world::native::NativeStore::open(&directory.path().join("state.redb"), [7; 32])
        .expect("reopen");
    let rows = load(&store.checkpoint_reader().expect("reader"), &blocks, &items).expect("rows");
    assert_eq!(rows[&Dimension::Nether].records, [stone(0)]);
    let overworld = rows[&Dimension::Overworld].clone();
    assert_eq!(
        overworld.records.iter().map(|r| r.seq).collect::<Vec<_>>(),
        [1]
    );
    assert_eq!(overworld.marks, [(1, true)]);
    // The WAL tail after that checkpoint restores record 1 and adds record 2.
    let tail = Delta {
        records: vec![stone(2)],
        marks: vec![(1, false), (7, true)],
    };
    let log = BlockLog::restore(BlockLogSettings::default(), vec![overworld, tail]);
    assert_eq!(
        log.records.iter().map(|r| r.seq).collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(!log.records[0].rolled_back);
    assert_eq!(
        log.next_seq, 8,
        "a mark of an evicted record still reserves its seq"
    );
}

/// A native row as written before records carried their committing tick.
fn old_format_row(
    blocks: &mc_world::BlockRegistry,
    items: &mc_data::items::ItemRegistry,
) -> Vec<u8> {
    let record = Record {
        change: Change::Block {
            before: blocks
                .block(&id("minecraft:stone"))
                .expect("stone")
                .default
                .0,
            after: blocks.block(&id("minecraft:air")).expect("air").default.0,
            entity: None,
        },
        ..record(1, 99)
    };
    let bytes = encode_record(&record, blocks, items).expect("encode");
    let (_, mut root) = mc_nbt::read_named(&mut bytes.as_slice()).expect("row nbt");
    let mc_nbt::Tag::Compound(fields) = &mut root else {
        panic!("block-log row is a compound");
    };
    fields.retain(|(name, _)| name != "tick");
    let mut old = Vec::new();
    mc_nbt::write_named(&mut old, "", &root).expect("write row");
    old
}

#[test]
fn block_log_tick_survives_wal_and_old_records_are_unknown() {
    let directory = tempfile::tempdir().expect("tempdir");
    let batch = |tick: u64| {
        directory
            .path()
            .join(format!("journal/batch-{tick:020}-{tick:020}.bin"))
    };
    let entry = crate::journal::JournalEntry::BlockLog {
        value: Box::new(Delta {
            records: vec![record(1, 99)],
            marks: Vec::new(),
        }),
    };
    let mut writer = crate::journal::JournalWriter::open(directory.path(), 1).expect("wal");
    writer
        .append(123, 123, std::slice::from_ref(&entry))
        .expect("append");
    let replayed = crate::journal::read_batch_file(&batch(123)).expect("new batch");
    assert_eq!(replayed, std::slice::from_ref(&entry));
    let mut old = serde_json::to_value(&entry).expect("entry json");
    old["BlockLog"]["value"]["records"][0]
        .as_object_mut()
        .expect("record object")
        .remove("tick");
    let old: crate::journal::JournalEntry = serde_json::from_value(old).expect("old entry");
    writer
        .append(124, 124, std::slice::from_ref(&old))
        .expect("append");
    let old_disk = crate::journal::read_batch_file(&batch(124)).expect("old batch");
    let crate::journal::JournalEntry::BlockLog { value } = &old_disk[0] else {
        panic!("block-log entry");
    };
    assert_eq!(value.records[0].tick, None);

    let blocks = required_blocks();
    let items = mc_data::items::solaris_required_items();
    let old_row = old_format_row(&blocks, &items);
    assert_eq!(
        decode_record(&old_row, 1, &blocks, &items)
            .expect("old row decodes")
            .tick,
        None
    );
    let path = directory.path().join("old-state.redb");
    let store = mc_world::native::NativeStore::create(&path, [7; 32]).expect("store");
    store
        .write_block_log(&[(key(RECORD_ROW, Dimension::Overworld, 1), old_row)], &[])
        .expect("write old row");
    drop(store);
    let store = mc_world::native::NativeStore::open(&path, [7; 32]).expect("reopen");
    let rows = load(&store.checkpoint_reader().expect("reader"), &blocks, &items).expect("rows");
    assert_eq!(rows[&Dimension::Overworld].records[0].tick, None);
}

#[test]
#[ignore = "child process of replayed_wal_tail_survives_the_checkpoint_that_prunes_it"]
fn block_log_crash_child() {
    use std::io::Write;
    let world = std::env::var_os("SOLARIS_BLOCK_LOG_CRASH_DIR").expect("world directory");
    let tick: u64 = std::env::var("SOLARIS_BLOCK_LOG_CRASH_TICK")
        .expect("tick")
        .parse()
        .expect("tick number");
    let registry = CoreRegistry::load(None).expect("registry");
    let mut old = record(2, unix_now());
    old.tick = None;
    old.change = Change::Block {
        before: registry.states.stone,
        after: registry.states.air,
        entity: None,
    };
    let mut new = old.clone();
    new.seq = 3;
    new.tick = Some(tick);
    let entry = crate::journal::JournalEntry::BlockLog {
        value: Box::new(Delta {
            records: vec![old, new],
            marks: Vec::new(),
        }),
    };
    let mut wal =
        crate::journal::JournalWriter::open(std::path::Path::new(&world), 1).expect("wal");
    wal.append(tick, tick, &[entry]).expect("append");
    wal.flush().expect("flush");
    println!("BLOCK_LOG_WAL_READY");
    std::io::stdout().flush().expect("stdout");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).expect("stdin");
    panic!("parent must kill child before any graceful shutdown");
}

/// Crash -> replay -> checkpoint (prunes the WAL) -> reopen: the replayed
/// block-log tail reaches the native table before its WAL batch is pruned.
#[test]
fn replayed_wal_tail_survives_the_checkpoint_that_prunes_it() {
    use std::io::BufRead;
    let world = tempfile::tempdir().expect("tempdir");
    let mut config = crate::CoreConfig::new(world.path().to_path_buf(), 42);
    config.view_distance = 1;
    config.simulation_distance = 1;
    let initial = crate::Core::open(config.clone()).expect("fresh world");
    initial.shutdown();
    drop(initial);
    let base = crate::native_checkpoint::tests::read_manifest(&config)
        .expect("manifest")
        .tick;
    let registry = CoreRegistry::load(None).expect("registry");
    // An existing world's table still holds a row from before tick capture.
    let store = crate::native_checkpoint::tests::fixture_store(&config).expect("store");
    store
        .write_block_log(
            &[(
                key(RECORD_ROW, Dimension::Overworld, 1),
                old_format_row(&registry.blocks, &registry.items),
            )],
            &[],
        )
        .expect("write old row");
    drop(store);

    let mut child = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args([
            "--exact",
            "block_log::tests::block_log_crash_child",
            "--ignored",
            "--nocapture",
        ])
        .env("SOLARIS_BLOCK_LOG_CRASH_DIR", world.path())
        .env("SOLARIS_BLOCK_LOG_CRASH_TICK", (base + 1).to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn crash child");
    let mut reader = std::io::BufReader::new(child.stdout.take().expect("child stdout"));
    let mut ready = false;
    let mut line = String::new();
    while reader.read_line(&mut line).expect("child output") != 0 {
        if line.contains("BLOCK_LOG_WAL_READY") {
            ready = true;
            break;
        }
        line.clear();
    }
    if !ready {
        let status = child.wait().expect("child status");
        panic!("child failed before durable WAL: {status}");
    }
    child.kill().expect("kill child");
    assert!(!child.wait().expect("child status").success());

    let assert_rows = |stage: &str| {
        let store = crate::native_checkpoint::tests::fixture_store(&config).expect("store");
        let rows = load(
            &store.checkpoint_reader().expect("reader"),
            &registry.blocks,
            &registry.items,
        )
        .expect("rows");
        let records = &rows
            .get(&Dimension::Overworld)
            .unwrap_or_else(|| panic!("{stage}: no overworld block-log rows"))
            .records;
        let tick = |seq| {
            records
                .iter()
                .find(|record| record.seq == seq)
                .unwrap_or_else(|| panic!("{stage}: replayed record {seq} missing"))
                .tick
        };
        assert_eq!(tick(2), None, "{stage}");
        assert_eq!(tick(3), Some(base + 1), "{stage}");
    };
    let replayed = crate::Core::open(config.clone()).expect("world with old row and WAL opens");
    replayed.shutdown();
    drop(replayed);
    let batch = world.path().join(format!(
        "journal/batch-{:020}-{:020}.bin",
        base + 1,
        base + 1
    ));
    assert!(!batch.exists(), "the checkpoint pruned the replayed batch");
    assert_rows("after the pruning checkpoint");
    let reopened = crate::Core::open(config.clone()).expect("world reopens after WAL prune");
    reopened.shutdown();
    drop(reopened);
    assert_rows("after reopen");
}

/// Rows name blocks and items, so a game version that renumbers runtime ids
/// still reads them back as the same blocks and items.
#[test]
fn table_rows_survive_runtime_id_reordering() {
    let report = mc_data::blocks::solaris_required_blocks_report();
    let old = mc_world::BlockRegistry::from_report(&report).expect("blocks");
    let mut next = 0;
    let renumbered: Vec<_> = report
        .iter()
        .rev()
        .map(|block| {
            let mut block = block.clone();
            for state in &mut block.states {
                state.id = next;
                next += 1;
            }
            block
        })
        .collect();
    let new = mc_world::BlockRegistry::from_report(&renumbered).expect("renumbered blocks");
    let items = |ids: [u32; 2]| {
        mc_data::items::ItemRegistry::from_report(&[
            mc_data::items::ItemReport {
                id: id("minecraft:diamond"),
                protocol_id: ids[0],
            },
            mc_data::items::ItemReport {
                id: id("minecraft:gold_ingot"),
                protocol_id: ids[1],
            },
        ])
    };
    let (old_items, new_items) = (items([1, 2]), items([9, 4]));
    let chest = |blocks: &mc_world::BlockRegistry| {
        blocks
            .by_name_and_props(
                &id("minecraft:chest"),
                &[
                    ("facing".into(), "east".into()),
                    ("type".into(), "left".into()),
                    ("waterlogged".into(), "true".into()),
                ],
            )
            .expect("east chest")
            .0
    };
    let air = |blocks: &mc_world::BlockRegistry| {
        blocks.block(&id("minecraft:air")).expect("air").default.0
    };
    assert_ne!(chest(&old), chest(&new));
    let broken = Record {
        change: Change::Block {
            before: chest(&old),
            after: air(&old),
            entity: Some(EntityImage {
                slots: Some(vec![
                    ItemStack::EMPTY,
                    ItemStack::new(1, 64),
                    ItemStack::EMPTY,
                ]),
                nbt: Some(vec![10, 0, 0]),
            }),
        },
        rolled_back: true,
        ..record(3, 1_000)
    };
    let stolen = Record {
        change: Change::Container {
            slots: vec![(5, ItemStack::new(2, 16), ItemStack::EMPTY)],
        },
        ..record(4, 1_001)
    };
    let bytes = encode_record(&broken, &old, &old_items).expect("encode");
    let read = decode_record(&bytes, 3, &new, &new_items).expect("decode");
    assert_eq!(
        read,
        Record {
            change: Change::Block {
                before: chest(&new),
                after: air(&new),
                entity: Some(EntityImage {
                    slots: Some(vec![
                        ItemStack::EMPTY,
                        ItemStack::new(9, 64),
                        ItemStack::EMPTY
                    ]),
                    nbt: Some(vec![10, 0, 0]),
                }),
            },
            ..broken
        }
    );
    let bytes = encode_record(&stolen, &old, &old_items).expect("encode");
    let read = decode_record(&bytes, 4, &new, &new_items).expect("decode");
    assert_eq!(
        read.change,
        Change::Container {
            slots: vec![(5, ItemStack::new(4, 16), ItemStack::EMPTY)],
        }
    );
    // Like the native block dictionary, the stored property set must be exact.
    let chest_with = |properties: &[(&str, &str)]| {
        mc_nbt::Tag::Compound(vec![
            ("Name".into(), mc_nbt::Tag::String("minecraft:chest".into())),
            (
                "Properties".into(),
                mc_nbt::Tag::Compound(
                    properties
                        .iter()
                        .map(|(key, value)| ((*key).into(), mc_nbt::Tag::String((*value).into())))
                        .collect(),
                ),
            ),
        ])
    };
    let full = [
        ("facing", "east"),
        ("type", "left"),
        ("waterlogged", "true"),
    ];
    assert_eq!(state_from(&new, &chest_with(&full)), Ok(chest(&new)));
    for stored in [
        chest_with(&full[..2]),
        chest_with(&[
            ("facing", "east"),
            ("type", "gone"),
            ("waterlogged", "true"),
        ]),
        chest_with(&[full[0], full[1], full[2], ("removed", "true")]),
    ] {
        let error = state_from(&new, &stored).expect_err("inexact chest state");
        assert!(error.contains("minecraft:chest"), "{error}");
    }
}

/// A checkpointed row naming a block or item this version lacks fails the
/// load with the name, dimension and sequence instead of dropping history.
#[test]
fn table_row_with_unknown_name_fails_load() {
    let directory = tempfile::tempdir().expect("tempdir");
    let store =
        mc_world::native::NativeStore::create(&directory.path().join("state.redb"), [7; 32])
            .expect("store");
    let report = mc_data::blocks::solaris_required_blocks_report();
    let blocks = mc_world::BlockRegistry::from_report(&report).expect("blocks");
    let items = mc_data::items::solaris_required_items();
    let diamond = items.id_of(&id("minecraft:diamond")).expect("diamond");
    let chest = blocks
        .block(&id("minecraft:chest"))
        .expect("chest")
        .default
        .0;
    let air = blocks.block(&id("minecraft:air")).expect("air").default.0;
    let mut backlog = Backlog::default();
    backlog.absorb(Persist {
        deltas: vec![(
            Dimension::Nether,
            Delta {
                records: vec![
                    Record {
                        change: Change::Block {
                            before: chest,
                            after: air,
                            entity: None,
                        },
                        ..record(3, 1_000)
                    },
                    Record {
                        change: Change::Container {
                            slots: vec![(0, ItemStack::new(diamond, 1), ItemStack::EMPTY)],
                        },
                        ..record(4, 1_000)
                    },
                ],
                marks: Vec::new(),
            },
        )],
        floors: vec![(Dimension::Nether, 0)],
    });
    backlog.write(&store, &blocks, &items).expect("write");
    let reader = || store.checkpoint_reader().expect("reader");
    assert_eq!(
        load(&reader(), &blocks, &items).expect("known rows")[&Dimension::Nether]
            .records
            .len(),
        2
    );
    let mut next = 0;
    let without_chest: Vec<_> = report
        .iter()
        .filter(|block| block.id != id("minecraft:chest"))
        .map(|block| {
            let mut block = block.clone();
            for state in &mut block.states {
                state.id = next;
                next += 1;
            }
            block
        })
        .collect();
    let without_chest =
        mc_world::BlockRegistry::from_report(&without_chest).expect("blocks without chest");
    let without_diamond =
        mc_data::items::ItemRegistry::from_report(&[mc_data::items::ItemReport {
            id: id("minecraft:stick"),
            protocol_id: 0,
        }]);
    for (blocks, items, name, seq) in [
        (&without_chest, &items, "minecraft:chest", "seq 3"),
        (&blocks, &without_diamond, "minecraft:diamond", "seq 4"),
    ] {
        let error = load(&reader(), blocks, items)
            .expect_err("unknown name fails closed")
            .to_string();
        for part in [name, "Nether", seq] {
            assert!(error.contains(part), "{error} lacks {part}");
        }
    }
}

/// `cargo test -p mc-core --release --lib block_log_capture_cost -- --ignored --nocapture`
#[test]
#[ignore = "timing; run under mem-run.sh -p in release"]
fn block_log_capture_cost() {
    let mut rig = Rig::new();
    let actor = rig.state.block_log.intern("Griefer");
    let stone = rig.state.registry.states.stone;
    let cells: Vec<BlockPos> = (-6..=6)
        .flat_map(|dx| (-4..=4).map(move |dz| (dx, dz)))
        .flat_map(|(dx, dz)| (0..8).map(move |dy| (dx, dy, dz)))
        .map(|(dx, dy, dz)| BlockPos {
            x: rig.at.x + dx,
            y: Y + 1 + dy,
            z: rig.at.z + dz,
        })
        .collect();
    let rounds = 200;
    let mut entries = Vec::new();
    let mut noted = std::time::Duration::ZERO;
    let mut sealed = std::time::Duration::ZERO;
    for round in 0..rounds {
        let next = if round % 2 == 0 {
            stone
        } else {
            rig.state.registry.states.air
        };
        let start = std::time::Instant::now();
        for &pos in &cells {
            note(&mut rig.state, pos, &actor);
        }
        noted += start.elapsed();
        for &pos in &cells {
            rig.state.world.set_block(pos.x, pos.y, pos.z, next);
        }
        rig.state.world.fold_overlay();
        let start = std::time::Instant::now();
        journal(&mut rig.state, 123, &mut entries);
        sealed += start.elapsed();
        entries.clear();
        rig.state.block_log.unpersisted.clear();
    }
    let changes = (rounds * cells.len()) as f64;
    let record = rig.state.block_log.records.back().expect("record");
    eprintln!(
        "block_log cost: {changes} changes; capture {:.0} ns/change; seal {:.0} ns/change; WAL JSON {} bytes/record",
        noted.as_nanos() as f64 / changes,
        sealed.as_nanos() as f64 / changes,
        serde_json::to_vec(record).expect("json").len(),
    );
}
