use super::*;
use crate::boundary::{Hand, PlayerInput};
use crate::registry::CoreRegistry;
use mc_data::Identifier;
use mc_world::{BlockPos, Chunk, ChunkGenerator, ChunkPos};

const LOOT: &str = r#"{"type":"minecraft:chest","pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:iron_ingot","modifier":[{"type":"minecraft:set_count","count":7}]}]}]}"#;

fn pack(root: &Path, name: &str, table: &str, text: &str) {
    let folder = root.join(name);
    std::fs::create_dir_all(folder.join("data/demo/loot_table")).unwrap();
    std::fs::write(
        folder.join("pack.mcmeta"),
        r#"{"pack":{"pack_format":121,"description":"authored container test"}}"#,
    )
    .unwrap();
    std::fs::write(
        folder.join(format!("data/demo/loot_table/{table}.json")),
        text,
    )
    .unwrap();
}

struct BarrelWorld {
    base: Arc<dyn ChunkGenerator>,
    barrel: u32,
    table: Identifier,
}
impl ChunkGenerator for BarrelWorld {
    fn generate(&self, pos: ChunkPos) -> Chunk {
        let mut chunk = self.base.generate(pos);
        if pos == (ChunkPos { x: 0, z: 0 }) {
            chunk.set_block(4, 201, 4, mc_world::BlockStateId(self.barrel));
            let position = BlockPos { x: 4, y: 201, z: 4 };
            chunk.chests.insert(
                position,
                mc_world::ChestBlockEntity {
                    loot_table: Some(self.table.clone()),
                    loot_seed: 17,
                    ..Default::default()
                },
            );
            let tag = Tag::Compound(vec![
                ("id".into(), Tag::String("minecraft:barrel".into())),
                ("x".into(), Tag::Int(4)),
                ("y".into(), Tag::Int(201)),
                ("z".into(), Tag::Int(4)),
                ("LootTable".into(), Tag::String(self.table.to_string())),
                ("LootTableSeed".into(), Tag::Long(17)),
            ]);
            let mut bytes = Vec::new();
            mc_nbt::write_network(&mut bytes, &tag).unwrap();
            chunk.block_entities.insert(position, bytes);
        }
        chunk
    }
}
fn barrel(registry: CoreRegistry, table: &str) -> TickState {
    let registry = Arc::new(registry);
    let base = crate::horde_bench::flat_world(&registry);
    let generator = Arc::new(BarrelWorld {
        base: base.generator(),
        barrel: registry
            .blocks
            .block(&Identifier::parse("minecraft:barrel").unwrap())
            .unwrap()
            .default
            .0,
        table: Identifier::parse(table).unwrap(),
    });
    let mut world = crate::world::ChunkWorld::new(
        42,
        generator,
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [4, 202, 4],
        mc_world::OVERWORLD_GEOMETRY,
    );
    world.ensure_blocking(&[(ChunkPos { x: 0, z: 0 }, 0)].into_iter().collect());
    TickState::new(registry, world, 2, 512)
}
fn open(state: &mut TickState) -> bool {
    crate::maps::loot::before_open(
        state,
        ConnectionId(1),
        &PlayerInput::UseItemOn {
            connection: ConnectionId(1),
            hand: Hand::Main,
            position: crate::boundary::BlockPos { x: 4, y: 201, z: 4 },
            face: 1,
            cursor: [0.5; 3],
            sequence: 1,
        },
        &[(4, 201, 4)],
    )
}

#[test]
fn atm_container_pack_barrel_rolls_once_and_unknown_table_warns_without_consuming() {
    let directory = tempfile::tempdir().unwrap();
    pack(directory.path(), "a", "crystal", LOOT);
    let packs = PackSet::load(directory.path(), Selection::default()).unwrap();
    let base = CoreRegistry::load(None).unwrap();
    let mut registry = base.prepare_content();
    packs.install_content(&mut registry).unwrap();
    assert!(
        base.chest_loot
            .get(&Identifier::parse("demo:crystal").unwrap())
            .is_none()
    );
    let mut state = barrel(registry, "demo:crystal");
    assert!(!state.block_entities.contains_key(&(4, 201, 4)));
    assert!(!open(&mut state));
    let slots = state.block_entities[&(4, 201, 4)].slots.clone();
    assert_eq!(slots.iter().map(|stack| stack.count).sum::<i32>(), 7);
    assert!(!open(&mut state));
    assert_eq!(state.block_entities[&(4, 201, 4)].slots, slots);
    assert_eq!(state.opaque_changes.len(), 1);
    let tag =
        mc_nbt::read_network(&mut state.opaque_changes[0].1.as_ref().unwrap().as_slice()).unwrap();
    let Tag::Compound(fields) = tag else {
        panic!("barrel compound")
    };
    assert_eq!(
        mc_data::item_nbt::field(&fields, "id"),
        Some(&Tag::String("minecraft:barrel".into()))
    );
    assert!(mc_data::item_nbt::field(&fields, "LootTable").is_none());

    let mut missing = barrel(base, "demo:absent");
    assert!(open(&mut missing));
    assert!(missing.block_entities.is_empty());
    assert!(missing.opaque_changes.is_empty());
    let chest = &missing.world.chunk(ChunkPos { x: 0, z: 0 }).unwrap().chests
        [&BlockPos { x: 4, y: 201, z: 4 }];
    assert_eq!(chest.loot_table.as_ref().unwrap().as_str(), "demo:absent");
    assert_eq!(chest.loot_seed, 17);
    assert!(missing.outbox[&ConnectionId(1)].iter().any(|change| matches!(change,
        PlayerChange::Chat { message } if message.contains("demo:absent") && message.contains("retained"))));
}

#[test]
fn atm_container_pack_priority_disable_and_failed_reload_preserve_base() {
    let directory = tempfile::tempdir().unwrap();
    pack(directory.path(), "a", "crystal", LOOT);
    pack(
        directory.path(),
        "b",
        "crystal",
        &LOOT.replace("\"count\":7", "\"count\":11"),
    );
    let base = CoreRegistry::load(None).unwrap();
    for (enabled, disabled, count) in [
        (vec!["vanilla", "file/a", "file/b"], vec![], 11),
        (vec!["vanilla", "file/a"], vec!["file/b"], 7),
    ] {
        let selection = Selection {
            enabled: enabled.into_iter().map(str::to_owned).collect(),
            disabled: disabled.into_iter().map(str::to_owned).collect(),
        };
        let packs = PackSet::load(directory.path(), selection).unwrap();
        let mut registry = base.prepare_content();
        packs.install_content(&mut registry).unwrap();
        let mut state = barrel(registry, "demo:crystal");
        assert!(!open(&mut state));
        assert_eq!(
            state.block_entities[&(4, 201, 4)]
                .slots
                .iter()
                .map(|s| s.count)
                .sum::<i32>(),
            count
        );
    }
    pack(
        directory.path(),
        "b",
        "crystal",
        &LOOT.replace("minecraft:set_count", "demo:unsupported"),
    );
    let packs = PackSet::load(directory.path(), Selection::default()).unwrap();
    let error = packs
        .install_content(&mut base.prepare_content())
        .unwrap_err();
    for detail in ["file/b", "demo:crystal", "demo:unsupported", "entries[0]"] {
        assert!(error.contains(detail), "{error}");
    }
    assert!(
        base.chest_loot
            .get(&Identifier::parse("demo:crystal").unwrap())
            .is_none()
    );
}

#[test]
#[ignore = "requires ATM_CARRY_MVS_PACK pointing at locally licensed converted data; no mod bytes in Git"]
fn atm_container_actual_mvs_tables_compile_and_crystal_quantity_reaches_barrel() {
    let source = PathBuf::from(std::env::var_os("ATM_CARRY_MVS_PACK").expect("ATM_CARRY_MVS_PACK"));
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("mvs/data/mvs/loot_table");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(
        directory.path().join("mvs/pack.mcmeta"),
        r#"{"pack":{"pack_format":121,"description":"local evaluation only"}}"#,
    )
    .unwrap();
    let source = source.join("data/mvs/loot_table");
    let mut pending = vec![source.clone()];
    let mut names = Vec::new();
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let relative = path.strip_prefix(&source).unwrap();
            let destination = target.join(relative);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::copy(&path, destination).unwrap();
            names.push(
                relative
                    .with_extension("")
                    .components()
                    .map(|part| part.as_os_str().to_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
    assert_eq!(names.len(), 32);
    let packs = PackSet::load(directory.path(), Selection::default()).unwrap();
    let mut registry = CoreRegistry::load(None).unwrap().prepare_content();
    packs.install_content(&mut registry).unwrap();
    let mut saw_quantity = false;
    for name in names {
        let table = registry
            .chest_loot
            .get(&Identifier::parse(format!("mvs:{name}")).unwrap())
            .unwrap();
        for seed in 0..64 {
            let drops = table
                .roll(
                    &mut mc_data::loot::chest_26_1_2::ChestRng::new(seed),
                    &registry.items,
                    0.0,
                )
                .unwrap();
            if name == "crystal" {
                saw_quantity |= drops.iter().any(|s| s.count > 1);
            }
        }
    }
    assert!(
        saw_quantity,
        "converted crystal modifiers must not disappear"
    );
    let mut state = barrel(registry, "mvs:crystal");
    assert!(!open(&mut state));
    assert!(
        state.block_entities[&(4, 201, 4)]
            .slots
            .iter()
            .any(|s| s.count > 1)
    );
    println!(
        "32 actual converted MVS tables compiled/rolled; crystal modifier quantity observed on first barrel open"
    );
}
