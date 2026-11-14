use super::*;
use mc_world::{ChunkGenerator, ChunkPos};
use std::sync::Arc;

fn registry() -> BlockRegistry {
    BlockRegistry::from_report(&mc_data::blocks::krasnokamen_required_blocks_report()).unwrap()
}
fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla")
}
fn fill_box(chunk: &mut Chunk, min_y: i32, max_y: i32, state: BlockStateId) {
    for y in min_y..=max_y {
        for lx in 0..16u8 {
            for lz in 0..16u8 {
                assert!(chunk.set_block(lx, y, lz, state).is_some());
            }
        }
    }
}

#[test]
fn archaeology_pyramid_generated_with_loot() {
    let r = registry();
    let (templates, _) = catalog::templates(&root(), "desert_pyramid", &r).unwrap();
    let mut chunk = Chunk::empty(
        ChunkPos { x: 0, z: 0 },
        state(&r, "minecraft:stone").unwrap(),
        id("minecraft:desert"),
    );
    paste_template(
        &mut chunk,
        &templates[0],
        BlockPos { x: 0, y: 64, z: 0 },
        42,
    );
    let found = chunk.block_entities.iter().any(|(pos, bytes)| {
        let block = chunk.get_block(pos.x as u8, pos.y, pos.z as u8);
        let tag = mc_nbt::read_network(&mut bytes.as_slice()).unwrap();
        block == Some(state(&r, "minecraft:suspicious_sand").unwrap())
            && matches!(tag, Tag::Compound(fields) if fields.iter().any(|(key, value)| key == "LootTable" && *value == Tag::String("minecraft:archaeology/desert_pyramid".into())) && fields.iter().any(|(key, value)| key == "LootTableSeed" && matches!(value, Tag::Long(_))))
    });
    assert!(
        found,
        "pyramid must generate suspicious sand with archaeology loot and seed"
    );
}

#[test]
fn archaeology_ocean_ruins_generate_seeded_loot() {
    let r = registry();
    for (family, material, table) in [
        (
            "ocean_ruin_warm",
            "minecraft:suspicious_sand",
            "minecraft:archaeology/ocean_ruin_warm",
        ),
        (
            "ocean_ruin_cold",
            "minecraft:suspicious_gravel",
            "minecraft:archaeology/ocean_ruin_cold",
        ),
    ] {
        let (templates, _) = catalog::templates(&root(), family, &r).unwrap();
        let mut found = false;
        for template in &templates {
            let mut chunk = Chunk::empty(
                ChunkPos { x: 0, z: 0 },
                state(&r, "minecraft:stone").unwrap(),
                id("minecraft:ocean"),
            );
            paste_template(&mut chunk, template, BlockPos { x: 0, y: 64, z: 0 }, 42);
            found |= chunk.block_entities.iter().any(|(pos, bytes)| {
                let tag = mc_nbt::read_network(&mut bytes.as_slice()).unwrap();
                chunk.get_block(pos.x as u8, pos.y, pos.z as u8) == Some(state(&r, material).unwrap())
                    && matches!(tag, Tag::Compound(fields) if fields.iter().any(|(key, value)| key == "LootTable" && *value == Tag::String(table.into())) && fields.iter().any(|(key, value)| key == "LootTableSeed" && matches!(value, Tag::Long(_))))
            });
        }
        assert!(found, "{family}");
    }
}

#[test]
fn generated_fully_eyed_stronghold_activates_portal() {
    let registry = registry();
    let original = stronghold(&registry).unwrap();
    let eyes = stronghold_eye_states(&registry).unwrap();
    let blocks = original
        .blocks()
        .iter()
        .map(|block| {
            let mut block = *block;
            if let Some((_, filled)) = eyes.iter().find(|(empty, _)| *empty == block.state) {
                block.state = *filled;
            }
            block
        })
        .collect();
    let template = StructureTemplate::new(original.size(), blocks)
        .with_chests(original.chests().to_vec())
        .with_block_entities(original.block_entities().to_vec())
        .with_entities(original.entities().to_vec());
    let structures = OverworldStructures {
        seed: 42,
        strongholds: vec![(0, 0)],
        stronghold: template,
        stronghold_eye_states: eyes,
        stronghold_portal: state(&registry, "minecraft:end_portal").unwrap(),
        air: vec![state(&registry, "minecraft:air").unwrap()],
        ground: HashSet::new(),
        filler: state(&registry, "minecraft:stone").unwrap(),
        families: Vec::new(),
        features: Vec::new(),
        reach_before: [3, 2],
        reach_after: [0, 0],
    };
    let mut chunk = Chunk::empty(
        ChunkPos { x: 0, z: 0 },
        state(&registry, "minecraft:stone").unwrap(),
        id("minecraft:plains"),
    );
    structures.paste(
        &mut chunk,
        |_, _| 32,
        |_, _| 32,
        |_, _, _| id("minecraft:plains"),
    );
    let portal = state(&registry, "minecraft:end_portal").unwrap();
    for x in 6..=8 {
        for z in 6..=8 {
            assert_eq!(chunk.get_block(x, 3, z), Some(portal), "portal at {x},{z}");
        }
    }
}

#[test]
fn generated_stronghold_eye_patterns_vary_between_sites_in_one_world() {
    let registry = registry();
    let template = stronghold(&registry).unwrap();
    let structures = OverworldStructures {
        seed: 42,
        strongholds: (0..16).map(|x| (x * 10, 0)).collect(),
        stronghold: template.clone(),
        stronghold_eye_states: stronghold_eye_states(&registry).unwrap(),
        stronghold_portal: state(&registry, "minecraft:end_portal").unwrap(),
        air: vec![state(&registry, "minecraft:air").unwrap()],
        ground: HashSet::new(),
        filler: state(&registry, "minecraft:stone").unwrap(),
        families: Vec::new(),
        features: Vec::new(),
        reach_before: [3, 2],
        reach_after: [0, 0],
    };
    let mut patterns = HashSet::new();
    for x in 0..16 {
        let mut chunk = Chunk::empty(
            ChunkPos { x: x * 10, z: 0 },
            state(&registry, "minecraft:stone").unwrap(),
            id("minecraft:plains"),
        );
        structures.paste(
            &mut chunk,
            |_, _| 32,
            |_, _| 32,
            |_, _, _| id("minecraft:plains"),
        );
        let pattern: Vec<_> = template
            .blocks()
            .iter()
            .filter(|block| {
                registry.by_id(block.state).unwrap().block.id == id("minecraft:end_portal_frame")
            })
            .map(|block| {
                let [x, y, z] = block.pos;
                let state = chunk.get_block(x as u8, y, z as u8).unwrap();
                registry
                    .by_id(state)
                    .unwrap()
                    .properties
                    .iter()
                    .any(|(k, v)| k == "eye" && v == "true")
            })
            .collect();
        patterns.insert(pattern);
    }
    assert!(
        patterns.len() > 1,
        "strongholds in one world must not share a single eye pattern"
    );
}

#[test]
fn generated_stronghold_frame_eyes_are_independent_and_seeded() {
    let registry = registry();
    let template = stronghold(&registry).unwrap();
    let mut eyes = 0;
    let mut empty_rooms = 0;
    let mut mixed_rooms = 0;
    for seed in 0..256 {
        let structures = OverworldStructures {
            seed,
            strongholds: vec![(0, 0)],
            stronghold: template.clone(),
            stronghold_eye_states: stronghold_eye_states(&registry).unwrap(),
            stronghold_portal: state(&registry, "minecraft:end_portal").unwrap(),
            air: vec![state(&registry, "minecraft:air").unwrap()],
            ground: HashSet::new(),
            filler: state(&registry, "minecraft:stone").unwrap(),
            families: Vec::new(),
            features: Vec::new(),
            reach_before: [3, 2],
            reach_after: [0, 0],
        };
        let generate = || {
            let mut chunk = Chunk::empty(
                ChunkPos { x: 0, z: 0 },
                state(&registry, "minecraft:stone").unwrap(),
                id("minecraft:plains"),
            );
            structures.paste(
                &mut chunk,
                |_, _| 32,
                |_, _| 32,
                |_, _, _| id("minecraft:plains"),
            );
            chunk
        };
        let chunk = generate();
        let repeat = generate();
        let mut room_eyes = 0;
        for block in template.blocks().iter().filter(|block| {
            registry.by_id(block.state).unwrap().block.id == id("minecraft:end_portal_frame")
        }) {
            let [x, y, z] = block.pos;
            let actual = chunk.get_block(x as u8, y, z as u8).unwrap();
            assert_eq!(repeat.get_block(x as u8, y, z as u8), Some(actual));
            let actual = registry.by_id(actual).unwrap();
            let original = registry.by_id(block.state).unwrap();
            assert_eq!(actual.block.id, original.block.id);
            assert_eq!(
                actual.properties.iter().find(|(k, _)| k == "facing"),
                original.properties.iter().find(|(k, _)| k == "facing")
            );
            if actual
                .properties
                .iter()
                .any(|(k, v)| k == "eye" && v == "true")
            {
                room_eyes += 1;
            }
        }
        eyes += room_eyes;
        empty_rooms += usize::from(room_eyes == 0);
        mixed_rooms += usize::from((1..12).contains(&room_eyes));
    }
    assert!(
        (215..=400).contains(&eyes),
        "expected approximately 10% of 3072 eyes, got {eyes}"
    );
    assert!(
        empty_rooms > 20,
        "no forced guaranteed eyes: {empty_rooms} empty rooms"
    );
    assert!(
        mixed_rooms > 100,
        "frame outcomes must vary independently: {mixed_rooms} mixed rooms"
    );
}

#[test]
fn stronghold_portal_room_has_twelve_inward_frames_spawner_and_loot() {
    let registry = registry();
    let template = stronghold(&registry).unwrap();
    let frame = id("minecraft:end_portal_frame");
    let frames: Vec<_> = template
        .blocks()
        .iter()
        .filter(|b| registry.by_id(b.state).unwrap().block.id == frame)
        .collect();
    assert_eq!(frames.len(), 12);
    for block in frames {
        let state = registry.by_id(block.state).unwrap();
        let facing = state
            .properties
            .iter()
            .find(|(k, _)| k == "facing")
            .unwrap()
            .1
            .as_str();
        assert_eq!(
            facing,
            if block.pos[0] == 5 {
                "east"
            } else if block.pos[0] == 9 {
                "west"
            } else if block.pos[2] == 5 {
                "south"
            } else {
                "north"
            }
        );
    }
    assert_eq!(template.chests().len(), 2);
    assert_eq!(template.block_entities().len(), 1);
    let pos = BlockPos {
        x: -49,
        y: -20,
        z: 70,
    };
    let bytes = template.block_entities()[0].at_world_position(pos);
    let nbt = mc_nbt::read_network(&mut bytes.as_slice()).unwrap();
    let Tag::Compound(fields) = nbt else {
        panic!("compound")
    };
    assert!(fields.contains(&("x".into(), Tag::Int(-49))));
    assert!(fields.contains(&(
        "SpawnData".into(),
        Tag::Compound(vec![(
            "entity".into(),
            Tag::Compound(vec![(
                "id".into(),
                Tag::String("minecraft:silverfish".into())
            )])
        )])
    )));
}

#[test]
fn stronghold_clipped_paste_matches_whole_plan_and_preserves_entities() {
    let registry = registry();
    let template = stronghold(&registry).unwrap();
    let origin = BlockPos {
        x: -16,
        y: -30,
        z: -32,
    };
    let mut chunks = BTreeMap::new();
    for x in -1..=2 {
        for z in -2..=0 {
            let mut chunk = Chunk::empty(
                ChunkPos { x, z },
                state(&registry, "minecraft:stone").unwrap(),
                id("minecraft:plains"),
            );
            paste_template(&mut chunk, &template, origin, 42);
            chunks.insert((x, z), chunk);
        }
    }
    for block in template.blocks() {
        let (x, y, z) = (
            origin.x + block.pos[0],
            origin.y + block.pos[1],
            origin.z + block.pos[2],
        );
        let chunk = &chunks[&(x.div_euclid(16), z.div_euclid(16))];
        assert_eq!(
            chunk.get_block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8),
            Some(block.state)
        );
    }
    assert_eq!(chunks.values().map(|c| c.chests.len()).sum::<usize>(), 2);
    assert_eq!(
        chunks
            .values()
            .map(|c| c.block_entities.len())
            .sum::<usize>(),
        1
    );
}

#[test]
fn vanilla_catalog_loads_and_located_stronghold_generates_portal() {
    if !root().exists() {
        eprintln!("SKIP local vanilla assets unavailable");
        return;
    }
    let registry = Arc::new(registry());
    let mut generator = crate::TerrainGenerator::new(42, Arc::clone(&registry));
    generator.load_vanilla_worldgen(&root()).unwrap();
    let structures = generator.structure_sites_in_region((-180, -180), (180, 180));
    let site = structures
        .iter()
        .find(|s| s.structure == id("minecraft:stronghold"))
        .unwrap();
    let chunk = generator.generate(ChunkPos {
        x: site.start_chunk.0,
        z: site.start_chunk.1,
    });
    let frame = registry
        .by_id(chunk.get_block(6, site.position.y + 3, 5).unwrap())
        .unwrap();
    assert_eq!(frame.block.id, id("minecraft:end_portal_frame"));
    assert!(
        frame
            .properties
            .iter()
            .any(|(key, value)| key == "facing" && value == "south")
    );
    assert!(chunk.block_entities.contains_key(&BlockPos {
        x: site.position.x + 7,
        y: site.position.y + 1,
        z: site.position.z + 14
    }));
    assert!(
        mc_world::structure_starts::read(&chunk.extras, chunk.pos, false)
            .unwrap()
            .iter()
            .any(|saved| saved == site)
    );
}

#[test]
fn mineshaft_has_walkable_crossing_rails_cave_spiders_and_loot() {
    let registry = registry();
    let template = mineshaft(&registry).unwrap();
    assert!(template.chests().is_empty());
    assert!(
        template
            .entities()
            .iter()
            .any(|entity| entity.entity_type == "minecraft:chest_minecart")
    );
    assert_eq!(template.block_entities().len(), 1);
    assert!(template.blocks().iter().any(|b| b.pos==[24,1,24] && b.state==state(&registry,"minecraft:rail").unwrap()));
}

#[test]
fn vanilla_shipwreck_markers_become_loot_not_authoring_blocks() {
    if !root().exists() {
        eprintln!("SKIP local vanilla assets unavailable");
        return;
    }
    let registry = registry();
    let template = StructureTemplate::from_nbt_file(
        root().join("data/minecraft/structure/shipwreck/with_mast.nbt"),
        &registry,
    )
    .unwrap();
    let template = clean_template(template, "shipwreck", &registry).unwrap();
    assert!(!template.blocks().iter().any(|block| {
        registry.by_id(block.state).unwrap().block.id == id("minecraft:structure_block")
    }));
    assert!(
        template
            .chests()
            .iter()
            .all(|chest| chest.loot_table.is_some())
    );
    assert!(
        template
            .chests()
            .iter()
            .any(|chest| chest.loot_table == Some(id("minecraft:chests/shipwreck_map")))
    );
}

#[test]
fn classic_catalog_contains_every_family_with_interactive_contents() {
    if !root().exists() {
        eprintln!("SKIP local vanilla assets unavailable");
        return;
    }
    let registry = registry();
    for name in [
        "mineshaft",
        "mineshaft_mesa",
        "igloo",
        "shipwreck",
        "shipwreck_beached",
        "ocean_ruin_warm",
        "ocean_ruin_cold",
        "pillager_outpost",
        "desert_pyramid",
        "jungle_pyramid",
        "swamp_hut",
        "buried_treasure",
        "monument",
        "mansion",
        "ruined_portal",
    ] {
        let (templates, _) = catalog::templates(&root(), name, &registry).unwrap();
        assert!(!templates.is_empty(), "{name}");
        for t in templates {
            assert!(!t.blocks().is_empty(), "{name}");
            assert!(
                t.blocks()
                    .iter()
                    .all(|b| registry.by_id(b.state).unwrap().block.id
                        != id("minecraft:structure_block")),
                "{name}"
            );
            assert!(t.chests().iter().all(|c| c.loot_table.is_some()), "{name}");
            if name == "mineshaft" || name == "mineshaft_mesa" {
                assert!(
                    t.entities()
                        .iter()
                        .any(|entity| entity.entity_type == "minecraft:chest_minecart"),
                    "{name}"
                );
            }
        }
    }
    let (temples, _) = catalog::templates(&root(), "jungle_pyramid", &registry).unwrap();
    assert_eq!(temples[0].block_entities().len(), 2);
    for entity in temples[0].block_entities() {
        assert!(
            entity
                .nbt
                .contains(&("id".into(), Tag::String("minecraft:dispenser".into())))
        );
        assert!(entity.nbt.iter().any(|(key, value)| key == "Items"
            && matches!(value,Tag::List(list) if !list.elements.is_empty())));
    }
    let (monuments, _) = catalog::templates(&root(), "monument", &registry).unwrap();
    assert_eq!(
        monuments[0]
            .entities()
            .iter()
            .filter(|e| e.entity_type == "minecraft:elder_guardian" && e.persistent)
            .count(),
        3
    );
    let (igloos, _) = catalog::templates(&root(), "igloo", &registry).unwrap();
    assert_eq!(igloos.len(), 2);
    assert!(
        igloos[1].bounds().0[1] < 0,
        "basement participates in saved bounds"
    );
}

#[test]
fn generated_shipwreck_map_chest_preserves_unopened_table_and_seed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    assert!(root.exists(), "local vanilla assets required");
    let registry = registry();
    let (templates, _) = catalog::templates(&root, "shipwreck", &registry).unwrap();
    let (template, chest) = templates
        .iter()
        .find_map(|template| {
            template
                .chests()
                .iter()
                .find(|chest| {
                    chest
                        .loot_table
                        .as_ref()
                        .is_some_and(|id| id.as_str() == "minecraft:chests/shipwreck_map")
                })
                .map(|chest| (template, chest))
        })
        .expect("shipwreck map chest asset");
    let position = BlockPos {
        x: chest.pos[0],
        y: 64 + chest.pos[1],
        z: chest.pos[2],
    };
    let mut chunk = Chunk::empty(
        ChunkPos {
            x: position.x.div_euclid(16),
            z: position.z.div_euclid(16),
        },
        state(&registry, "minecraft:air").unwrap(),
        id("minecraft:ocean"),
    );
    paste_template(&mut chunk, template, BlockPos { x: 0, y: 64, z: 0 }, 42);
    let generated = &chunk.chests[&position];
    assert_eq!(
        generated.loot_table,
        Some(id("minecraft:chests/shipwreck_map"))
    );
    assert_eq!(
        generated.loot_seed,
        (hash(42, position.x, position.z) ^ position.y as u64) as i64
    );
    assert!(
        generated.slots.iter().all(|slot| slot.count == 0),
        "unopened, not prematurely rolled without map context"
    );
}

#[test]
fn igloo_basement_has_continuous_ladder_and_pastes_north_of_start() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    let registry = registry();
    let mut structures = OverworldStructures::load(&root, 42, &registry).unwrap();
    structures.strongholds.clear();
    structures.features.clear();
    structures
        .families
        .retain(|family| family.id.path() == "igloo");
    let family = &mut structures.families[0];
    let basement = family.templates.remove(1);
    for y in -22..0 {
        let block = basement
            .blocks()
            .iter()
            .find(|block| block.pos == [3, y, 5])
            .expect("continuous basement-to-trapdoor shaft");
        assert_eq!(
            registry.by_id(block.state).unwrap().block.id.path(),
            "ladder"
        );
    }
    let (cx, cz) = family.placement.potential_chunk(42, 0, 0);
    family.templates = vec![basement.clone()];
    assert!(family.biomes.contains(&id("minecraft:snowy_plains")));
    let mut chunk = Chunk::empty(
        ChunkPos { x: cx, z: cz - 1 },
        state(&registry, "minecraft:air").unwrap(),
        id("minecraft:snowy_plains"),
    );
    structures.paste(
        &mut chunk,
        |_, _| 64,
        |_, _| 64,
        |_, _, _| id("minecraft:snowy_plains"),
    );
    let mut checked = 0;
    for block in basement.blocks().iter().filter(|block| block.pos[2] < 0) {
        assert_eq!(
            chunk.get_block(
                block.pos[0] as u8,
                65 + block.pos[1],
                (16 + block.pos[2]) as u8
            ),
            Some(block.state)
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "basement crosses the negative-Z chunk boundary"
    );
}

#[test]
fn outpost_exclusion_rejects_nearby_village_placement_before_terrain_sampling() {
    let registry = registry();
    let mut structures = OverworldStructures::load(&root(), 42, &registry).unwrap();
    structures.strongholds.clear();
    structures
        .families
        .retain(|family| family.id.path() == "pillager_outpost");
    let family = &mut structures.families[0];
    assert_eq!(
        family.exclusion.unwrap().1,
        10,
        "local vanilla exclusion radius"
    );
    let grid = RandomSpreadPlacement {
        spacing: 100,
        separation: 99,
        salt: 1,
        triangular: false,
    };
    family.placement = grid;
    family.frequency = 1.0;
    family.exclusion = Some((grid, 10));
    assert!(
        structures
            .sites(
                (0, 0),
                (0, 0),
                |_, _| panic!("excluded start must not sample terrain"),
                |_, _| panic!("excluded start must not sample terrain"),
                |_, _, _| id("minecraft:plains")
            )
            .is_empty()
    );
    structures.families[0].exclusion = None;
    assert_eq!(
        structures
            .sites(
                (0, 0),
                (0, 0),
                |_, _| 64,
                |_, _| 64,
                |_, _, _| id("minecraft:plains")
            )
            .len(),
        1
    );
    assert!(near_placement(42, grid, 10, -90, 190));

    assert!(!near_placement(42, grid, 10, -89, 189));
}
#[test]
fn ocean_monument_start_is_underwater() {
    let registry = registry();
    let mut structures = OverworldStructures::load(&root(), 42, &registry).unwrap();
    structures.strongholds.clear();
    structures
        .families
        .retain(|family| family.id.path() == "monument");
    let family = &mut structures.families[0];
    family.frequency = 1.0;
    family.exclusion = None;
    family.biomes.insert(id("minecraft:deep_ocean"));
    let (x, z) = family.placement.potential_chunk(42, 0, 0);
    let sites = structures.sites(
        (x, z),
        (x, z),
        |_, _| 62,
        |_, _| 51,
        |_, _, _| id("minecraft:deep_ocean"),
    );
    assert_eq!(sites.len(), 1);
    assert!(
        sites[0].position.y + 16 < 63,
        "monument roof must be underwater"
    );
}

#[test]
fn floating_surface_structure_rests_on_a_support_column() {
    // WORLDGEN-RUINED-PORTAL-FLOATS-ON-SLOPE: a surface template that hangs over
    // air gets a support column of the column's TERRAIN material, so the
    // netherrack platform no longer floats above the ground below its footprint
    // and no template block is copied into the gap.
    let registry = registry();
    let mut structures = OverworldStructures::load(&root(), 42, &registry).unwrap();
    structures.strongholds.clear();
    structures
        .families
        .retain(|family| family.id.path().starts_with("ruined_portal"));
    let family = &mut structures.families[0];
    family.frequency = 1.0;
    family.exclusion = None;
    family.biomes.insert(id("minecraft:plains"));
    let (x, z) = family.placement.potential_chunk(42, 0, 0);
    let surface = |_: i32, _: i32| 64;
    let biome = |_, _, _| id("minecraft:plains");
    let sites = structures.sites((x, z), (x, z), surface, |_, _| 51, biome);
    assert_eq!(sites.len(), 1);
    let origin_y = sites[0].position.y;
    let stone = state(&registry, "minecraft:stone").unwrap();
    let dirt = state(&registry, "minecraft:dirt").unwrap();
    let air = state(&registry, "minecraft:air").unwrap();
    let mut chunk = Chunk::empty(ChunkPos { x, z }, stone, id("minecraft:plains"));
    // Air above a dirt ground at y=62: the ruin pasted above it floats.
    for y in 63..=origin_y + 40 {
        for lx in 0..16 {
            for lz in 0..16 {
                assert!(chunk.set_block(lx, y, lz, air).is_some());
            }
        }
    }
    for lx in 0..16u8 {
        for lz in 0..16u8 {
            assert!(chunk.set_block(lx, 62, lz, dirt).is_some());
        }
    }
    structures.paste(&mut chunk, surface, |_, _| 51, biome);
    let family = &structures.families[0];
    let template = &family.templates
        [(hash(structures.seed ^ 0x53545255, x, z) as usize) % family.templates.len()];
    let template_states: HashSet<BlockStateId> =
        template.blocks().iter().map(|block| block.state).collect();
    assert!(
        !template_states.contains(&dirt),
        "the fixture needs a template without dirt"
    );
    let mut lowest = BTreeMap::new();
    for block in template.blocks() {
        if block.state == air {
            continue;
        }
        lowest
            .entry((block.pos[0], block.pos[2]))
            .and_modify(|base: &mut i32| *base = (*base).min(block.pos[1]))
            .or_insert(block.pos[1]);
    }
    let mut columns = 0;
    for ((lx, lz), base) in lowest {
        if !(0..16).contains(&lx) || !(0..16).contains(&lz) {
            continue;
        }
        columns += 1;
        let (lx, lz) = (lx as u8, lz as u8);
        let base_y = origin_y + base;
        let support = chunk.get_block(lx, base_y - 1, lz);
        assert!(
            support.is_some_and(|id| id != air),
            "column {lx},{lz} floats above air"
        );
        let support = support.unwrap();
        assert_eq!(
            support, dirt,
            "column {lx},{lz} continues the terrain under the gap"
        );
        assert!(
            !template_states.contains(&support),
            "column {lx},{lz} copies a template block into the support column"
        );
        assert!(
            !registry.by_id(support).unwrap().has_fluid(),
            "column {lx},{lz} fills the gap with a fluid"
        );
        for y in 63..base_y {
            assert_eq!(
                chunk.get_block(lx, y, lz),
                Some(dirt),
                "column {lx},{lz} gap cell {y} is not terrain material"
            );
        }
    }
    assert!(columns > 0, "the template has footprint columns");
}

#[test]
fn ruined_portal_support_columns_never_copy_template_material() {
    // Review of 81a2b6bb8: the fill kept the last visited template block of a
    // column, so `portal_6`'s netherrack-over-gold column grew four gold blocks
    // under the platform, and other columns copied chests or lava. Every 26.3
    // ruined portal template the generator loads is walked here over an air gap:
    // the support must be terrain material, never a template block, fluid or
    // block entity.
    let registry = registry();
    let structures = OverworldStructures::load(&root(), 42, &registry).unwrap();
    let air = state(&registry, "minecraft:air").unwrap();
    let stone = state(&registry, "minecraft:stone").unwrap();
    let dirt = state(&registry, "minecraft:dirt").unwrap();
    let netherrack = state(&registry, "minecraft:netherrack").unwrap();
    let gold = state(&registry, "minecraft:gold_block").unwrap();
    let directory = root().join("data/minecraft/structure/ruined_portal");
    let mut paths: Vec<_> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("nbt"))
        .collect();
    paths.sort();
    let (loaded, _) = catalog::templates(&root(), "ruined_portal", &registry).unwrap();
    assert_eq!(
        paths.len(),
        loaded.len(),
        "the test walks exactly the templates the generator loads"
    );
    let mut reviewed_shape: Vec<String> = Vec::new();
    for path in &paths {
        let name = path.file_stem().and_then(|stem| stem.to_str()).unwrap();
        let template = clean_template(
            StructureTemplate::from_nbt_file(path, &registry).unwrap(),
            "ruined_portal",
            &registry,
        )
        .unwrap();
        let (min, max) = template.bounds();
        // Centre every footprint column inside chunk (0, 0).
        let origin = BlockPos {
            x: (16 - (max[0] - min[0] + 1)) / 2 - min[0],
            y: 100,
            z: (16 - (max[2] - min[2] + 1)) / 2 - min[2],
        };
        let template_states: HashSet<BlockStateId> =
            template.blocks().iter().map(|block| block.state).collect();
        assert!(
            !template_states.contains(&dirt),
            "{name} template has no dirt, so a dirt support proves the fill is not template material"
        );
        let mut lowest: BTreeMap<(i32, i32), i32> = BTreeMap::new();
        for block in template.blocks() {
            if block.state == air {
                continue;
            }
            lowest
                .entry((block.pos[0], block.pos[2]))
                .and_modify(|base: &mut i32| *base = (*base).min(block.pos[1]))
                .or_insert(block.pos[1]);
        }
        // A dirt floor exactly four cells under every column: the deepest gap
        // the rule may fill still lands on natural terrain.
        let mut chunk = Chunk::empty(ChunkPos { x: 0, z: 0 }, stone, id("minecraft:plains"));
        fill_box(&mut chunk, 0, origin.y + max[1] + 1, air);
        for (&(lx, lz), &base) in &lowest {
            let (x, z) = ((origin.x + lx) as u8, (origin.z + lz) as u8);
            assert!(chunk.set_block(x, origin.y + base - 5, z, dirt).is_some());
        }
        structures.support_columns(&mut chunk, &template, origin);
        let lowest_y = origin.y + lowest.values().copied().min().unwrap();
        for (&(lx, lz), &base) in &lowest {
            let (x, z) = ((origin.x + lx) as u8, (origin.z + lz) as u8);
            let base_y = origin.y + base;
            for y in base_y - 4..base_y {
                assert_eq!(
                    chunk.get_block(x, y, z),
                    Some(dirt),
                    "{name} column {lx},{lz} must continue the terrain under the gap"
                );
            }
        }
        for y in 61..lowest_y {
            for x in 0..16u8 {
                for z in 0..16u8 {
                    let cell = chunk.get_block(x, y, z).unwrap();
                    if cell == air {
                        continue;
                    }
                    assert!(
                        !template_states.contains(&cell) || cell == stone,
                        "{name} copied {} into its support column at {x},{y},{z}",
                        registry.by_id(cell).unwrap().block.id.path()
                    );
                    assert!(
                        cell == dirt || cell == stone,
                        "{name} wrote {} under its template at {x},{y},{z}",
                        registry.by_id(cell).unwrap().block.id.path()
                    );
                    assert!(
                        !registry.by_id(cell).unwrap().has_fluid(),
                        "{name} filled the gap with a fluid at {x},{y},{z}"
                    );
                }
            }
        }
        // No ground within the bound: the dimension filler, never a template block.
        let mut bare = Chunk::empty(ChunkPos { x: 0, z: 0 }, air, id("minecraft:plains"));
        structures.support_columns(&mut bare, &template, origin);
        for (&(lx, lz), &base) in &lowest {
            let (x, z) = ((origin.x + lx) as u8, (origin.z + lz) as u8);
            let base_y = origin.y + base;
            for y in base_y - 4..base_y {
                assert_eq!(
                    bare.get_block(x, y, z),
                    Some(stone),
                    "{name} column {lx},{lz} must use the dimension filler over a chasm"
                );
            }
        }
        for (&(lx, lz), &base) in &lowest {
            let lowest_state = template
                .blocks()
                .iter()
                .find(|block| block.pos == [lx, base, lz])
                .map(|block| block.state);
            let gold_above = template.blocks().iter().any(|block| {
                block.pos[0] == lx
                    && block.pos[2] == lz
                    && block.pos[1] > base
                    && block.state == gold
            });
            if lowest_state == Some(netherrack) && gold_above {
                reviewed_shape.push(name.to_owned());
            }
        }
    }
    assert!(
        reviewed_shape.iter().any(|name| name == "portal_6"),
        "the reviewed netherrack-over-gold column is covered: {reviewed_shape:?}"
    );

    // End-to-end: pasting `portal_6` into a chunk with a terrain ground under it
    // writes terrain support, not the template's own netherrack or gold.
    let mut structures = OverworldStructures::load(&root(), 42, &registry).unwrap();
    structures.strongholds.clear();
    structures
        .families
        .retain(|family| family.id.path() == "ruined_portal");
    assert_eq!(structures.families.len(), 1, "one overworld ruined portal");
    let portal_6 = clean_template(
        StructureTemplate::from_nbt_file(directory.join("portal_6.nbt"), &registry).unwrap(),
        "ruined_portal",
        &registry,
    )
    .unwrap();
    let family = &mut structures.families[0];
    family.frequency = 1.0;
    family.exclusion = None;
    family.biomes.insert(id("minecraft:plains"));
    family.templates = vec![portal_6.clone()];
    let (x, z) = family.placement.potential_chunk(42, 0, 0);
    let surface = |_: i32, _: i32| 64;
    let biome = |_, _, _| id("minecraft:plains");
    let sites = structures.sites((x, z), (x, z), surface, |_, _| 51, biome);
    assert_eq!(sites.len(), 1);
    let origin_y = sites[0].position.y;
    let mut chunk = Chunk::empty(ChunkPos { x, z }, stone, id("minecraft:plains"));
    for y in 61..=origin_y + 20 {
        for lx in 0..16u8 {
            for lz in 0..16u8 {
                assert!(chunk.set_block(lx, y, lz, air).is_some());
            }
        }
    }
    for lx in 0..16u8 {
        for lz in 0..16u8 {
            assert!(chunk.set_block(lx, 60, lz, dirt).is_some());
        }
    }
    structures.paste(&mut chunk, surface, |_, _| 51, biome);
    let mut lowest_y = i32::MAX;
    for block in portal_6.blocks() {
        if block.state != air {
            lowest_y = lowest_y.min(origin_y + block.pos[1]);
        }
    }
    let pasted_states: HashSet<BlockStateId> =
        portal_6.blocks().iter().map(|block| block.state).collect();
    let mut support_cells = 0;
    for y in 61..lowest_y {
        for lx in 0..16u8 {
            for lz in 0..16u8 {
                let cell = chunk.get_block(lx, y, lz).unwrap();
                if cell == air {
                    continue;
                }
                assert!(
                    !pasted_states.contains(&cell) || cell == stone,
                    "the pasted ruin copied {} into its support column at {lx},{y},{lz}",
                    registry.by_id(cell).unwrap().block.id.path()
                );
                assert!(
                    !registry.by_id(cell).unwrap().has_fluid(),
                    "the pasted ruin filled its support column with a fluid at {lx},{y},{lz}"
                );
                support_cells += usize::from(cell == dirt);
            }
        }
    }
    assert!(
        support_cells > 0,
        "portal_6 rests on a terrain support column"
    );
    assert!(
        !chunk.block_entities.keys().any(|pos| pos.y < lowest_y),
        "no block entity under the portal"
    );
    assert!(
        !chunk.chests.keys().any(|pos| pos.y < lowest_y),
        "no chest under the portal"
    );
}

#[test]
fn ocean_wrecks_and_ruins_use_seabed_and_beached_wrecks_use_surface() {
    let registry = registry();
    for (name, expected) in [
        ("shipwreck", 52),
        ("ocean_ruin_cold", 52),
        ("ocean_ruin_warm", 52),
        ("shipwreck_beached", 63),
    ] {
        let mut structures = OverworldStructures::load(&root(), 42, &registry).unwrap();
        structures.strongholds.clear();
        structures
            .families
            .retain(|family| family.id.path() == name);
        let family = &mut structures.families[0];
        family.frequency = 1.0;
        family.exclusion = None;
        family.biomes.insert(id("minecraft:ocean"));
        let (x, z) = family.placement.potential_chunk(42, 0, 0);
        let sites = structures.sites(
            (x, z),
            (x, z),
            |_, _| 62,
            |_, _| 51,
            |_, _, _| id("minecraft:ocean"),
        );
        assert_eq!(sites.len(), 1, "{name}");
        assert_eq!(sites[0].position.y, expected, "{name}");
        let mut chunk = Chunk::empty(
            ChunkPos { x, z },
            state(&registry, "minecraft:stone").unwrap(),
            id("minecraft:ocean"),
        );
        structures.paste(
            &mut chunk,
            |_, _| 62,
            |_, _| 51,
            |_, _, _| id("minecraft:ocean"),
        );
        let saved = mc_world::structure_starts::read(&chunk.extras, chunk.pos, false).unwrap();
        assert!(saved.iter().any(|site| site == &sites[0]), "{name}");
    }
}

#[test]
fn mineshaft_places_chest_minecart_on_rail_in_owner_chunk() {
    let registry = registry();
    let template = mineshaft(&registry).unwrap();
    let mut chunk = Chunk::empty(
        ChunkPos { x: 2, z: 1 },
        state(&registry, "minecraft:stone").unwrap(),
        id("minecraft:plains"),
    );
    paste_template(&mut chunk, &template, BlockPos { x: 0, y: 32, z: 0 }, 42);
    let markers = chunk.generated_entities();
    let cart = markers
        .iter()
        .find(|m| m.entity_type == "minecraft:chest_minecart")
        .expect("mineshaft cart");
    assert_eq!(
        chunk.get_block(8, 33, 9),
        Some(state(&registry, "minecraft:rail").unwrap())
    );
    assert_eq!(cart.position, [40.5, 33.0, 25.5]);
    assert!(chunk.chests.is_empty());
}
