use super::*;
use mc_world::{BlockRegistry, Chunk, ChunkGenerator, ChunkPos};
use std::sync::Arc;

fn generator(seed: i64) -> NetherGenerator {
    let registry = Arc::new(
        BlockRegistry::from_report(&mc_data::blocks::krasnokamen_required_blocks_report()).unwrap(),
    );
    NetherGenerator::new(seed, registry)
}

fn landscape(generator: &NetherGenerator, pos: ChunkPos) -> Chunk {
    let mut chunk = Chunk::empty_with_geometry(
        pos,
        generator.air,
        generator.biomes[0].clone(),
        generator.geometry,
    );
    generator.fill_landscape(&mut chunk);
    chunk
}

// Independent oracle over actual written blocks, not density-column queries.
fn land_from_blocks(
    generator: &NetherGenerator,
    chunk: &Chunk,
    x: u8,
    z: u8,
    min_y: i32,
    max_y: i32,
) -> Option<i32> {
    let start = min_y.max(33);
    let end = max_y.min(128).checked_sub(40)?;
    (start..=end).rev().find(|&y| {
        let block = chunk.get_block(x, y, z).unwrap();
        block != generator.air
            && block != generator.lava
            && chunk.get_block(x, y + 3, z) == Some(generator.air)
    })
}

#[test]
fn highest_land_matches_written_nether_columns_not_roof_or_storage_top() {
    let generator = generator(1234);
    let mut found = 0;
    let mut absent = 0;
    for pos in [ChunkPos { x: 0, z: 0 }, ChunkPos { x: -9, z: 13 }] {
        let chunk = landscape(&generator, pos);
        for x in 0..16u8 {
            for z in 0..16u8 {
                let wx = pos.x * 16 + i32::from(x);
                let wz = pos.z * 16 + i32::from(z);
                let expected = land_from_blocks(&generator, &chunk, x, z, 0, 128);
                assert_eq!(highest_land(&generator, wx, wz, 0, 128), expected);
                assert_eq!(highest_land(&generator, wx, wz, 0, 256), expected);
                if let Some(y) = expected {
                    found += 1;
                    assert!((33..=88).contains(&y));
                    assert_eq!(highest_land(&generator, wx, wz, y, 128), Some(y));
                    assert_eq!(highest_land(&generator, wx, wz, y + 1, 128), None);
                } else {
                    absent += 1;
                }
            }
        }
    }
    assert!(
        found > 0,
        "fixture must exercise actual land, not only refusals"
    );
    assert!(
        absent > 0,
        "fixture must exercise refusal when no land exists"
    );
}

#[test]
fn highest_land_honours_generation_bounds_without_fallback() {
    let generator = generator(-8745);
    let pos = ChunkPos { x: -3, z: -2 };
    let chunk = landscape(&generator, pos);
    for x in 0..16u8 {
        for z in 0..16u8 {
            let wx = pos.x * 16 + i32::from(x);
            let wz = pos.z * 16 + i32::from(z);
            for (min_y, max_y) in [(0, 96), (60, 128), (89, 256), (128, 0)] {
                assert_eq!(
                    highest_land(&generator, wx, wz, min_y, max_y),
                    land_from_blocks(&generator, &chunk, x, z, min_y, max_y),
                    "bounds {min_y}..{max_y}, column {wx},{wz}"
                );
            }
            assert_eq!(highest_land(&generator, wx, wz, i32::MAX, i32::MIN), None);
        }
    }
}

#[test]
fn height_and_fluid_queries_match_real_plan_free_roof() {
    let generator = generator(741);
    let query = NetherMoogQuery {
        generator: &generator,
    };
    let pos = ChunkPos { x: -2, z: 3 };
    let chunk = landscape(&generator, pos);
    for x in 0..16u8 {
        for z in 0..16u8 {
            let wx = pos.x * 16 + i32::from(x);
            let wz = pos.z * 16 + i32::from(z);
            let top = (0..256)
                .rev()
                .find(|&y| chunk.get_block(x, y, z) != Some(generator.air))
                .unwrap();
            assert_eq!(chunk.get_block(x, top, z), Some(generator.bedrock));
            assert_eq!(query.world_surface(wx, wz), top + 1);
            assert_eq!(query.ocean_floor(wx, wz), Some(top + 1));
            assert_eq!(
                query.top_is_liquid(wx, wz),
                Some(chunk.get_block(x, top, z) == Some(generator.lava))
            );
        }
    }
    assert_eq!(query.sea_level(), Some(32));
}

#[test]
fn biome_query_uses_the_real_generator_source_including_negative_columns() {
    let generator = generator(1234);
    let query = NetherMoogQuery {
        generator: &generator,
    };
    let mut biomes = std::collections::BTreeSet::new();
    for x in -12..=12 {
        for z in -12..=12 {
            for y in [0, 64, 127, 255] {
                let biome = query.biome_at(x * 128, y, z * 128);
                assert_eq!(
                    Some(biome.clone()),
                    generator.noise_biome(x * 128, y, z * 128)
                );
                biomes.insert(biome);
            }
        }
    }
    assert_eq!(biomes.len(), 5);
}
