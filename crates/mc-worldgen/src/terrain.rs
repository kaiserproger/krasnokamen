//! Overworld chunk assembly.
//!
//! The density router owns continents, erosion, mountain ridges, rivers,
//! climate, and caves. This module resolves block palettes and assembles
//! chunks from that deterministic field.
//! Vertical base layers:
//!
//! - `y = geometry.min_y()` → bedrock
//! - `geometry.min_y() < y < height - 3` → stone
//! - `height - 3 ≤ y < height` → dirt
//! - `y = height` → grass_block
//! - `y > height` → air
//!
//! Output is deterministic in `(seed, world_x, world_z)` and independent of
//! chunk generation order.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use mc_data::Identifier;
use mc_world::chunk::{Chunk, ChunkGeometry, ChunkPos, Heightmap, OVERWORLD_GEOMETRY};
use mc_world::{
    BIOME_DIM, BIOME_VOLUME, BiomeSection, BlockRegistry, BlockStateId, ChunkGenerator,
    PackedBitArray,
};

use crate::noise::fbm_2d;
use crate::vanilla_climate::VanillaClimate;
use crate::vanilla_density::{SurfaceHeightSampler, VanillaDensity};
use crate::village::plan_source::{VillagePlanSet, VillagePlanSource};

mod biome_routing;
mod biome_rules;
mod gameplay_rules;
mod geological_ores;
#[cfg(feature = "gpu")]
mod gpu_service;
mod ore_rules;
mod ore_veins;
mod overworld;
mod sediments;
mod solid_cache;
mod solids;
mod trees;

mod assembly;
mod caves;
mod config;
mod decorations;
mod height;
mod ores;
mod sampling;
mod spawn;
mod structures;
mod surface;

pub use biome_rules::BiomeRules;
pub use gameplay_rules::{ClayRule, TreeRule};
use geological_ores::GeologicalOreRules;
pub use ore_rules::{
    BiomeScope, MAX_ORE_RULES, MAX_ORE_WORK_UNITS_PER_CHUNK, OreRule, OreRules, OreRulesError,
    OreSpacing, YRange,
};
use ore_rules::{
    MAX_ORE_ANCHORS_PER_CELL, MAX_ORE_VEIN_SIZE, ORE_ANCHOR_CELL_EDGE, ORE_ANCHOR_CELL_VOLUME,
};
use overworld::{OverworldRouter, TerrainSample};
pub use solids::{ChunkSolids, SolidGrid};

#[cfg(test)]
use assembly::chunk_solids;
pub use config::WorldgenMode;
#[cfg(test)]
pub(crate) use height::HeightContext;
#[cfg(feature = "worldgen-counters")]
pub use height::{HeightContextStats, height_context_stats, reset_height_context_stats};
pub use spawn::SpawnLocation;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TerrainGeneratorError {
    #[error("block registry missing required terrain block {name}")]
    MissingRequiredBlock { name: &'static str },
}

pub const SEA_LEVEL: i32 = 63;

const ORE_VEIN_RADIUS: i32 = 4;
const ORE_COLUMN_HALO: i32 = ORE_VEIN_RADIUS * 2 + 1;
const ORE_DIRECTIONS: [[i8; 3]; 6] = [
    [-1, 0, 0],
    [1, 0, 0],
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
];
const CAVE_SURFACE_CLEARANCE: i32 = 32;
const CAVE_VERTICAL_SAMPLE_STEP: i32 = 2;

const DIAGNOSTIC_CACHE_SHARDS: usize = 64;
const DIAGNOSTIC_CACHE_ENTRIES_PER_SHARD: usize = 2_048;

/// Read-only view of the production column plan for diagnostics and tooling.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainDiagnosticSample {
    pub surface_y: i32,
    pub biome: Identifier,
    /// The production vegetation density for biomes that support land vegetation.
    pub vegetation_density: Option<f64>,
}

/// Derived column facts, with bounded FIFO eviction rather than whole-shard
/// invalidation. Sampling remains outside the shard lock.
struct DiagnosticCache {
    shards: [Mutex<DiagnosticCacheShard>; DIAGNOSTIC_CACHE_SHARDS],
}

#[derive(Default)]
struct DiagnosticCacheShard {
    entries: HashMap<(i32, i32), TerrainDiagnosticSample>,
    order: VecDeque<(i32, i32)>,
}

impl Default for DiagnosticCache {
    fn default() -> Self {
        Self {
            shards: std::array::from_fn(|_| Mutex::new(DiagnosticCacheShard::default())),
        }
    }
}

impl DiagnosticCache {
    fn shard_index(world_x: i32, world_z: i32) -> usize {
        let mixed = (world_x as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (world_z as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        (mixed as usize) & (DIAGNOSTIC_CACHE_SHARDS - 1)
    }

    fn get(&self, world_x: i32, world_z: i32) -> Option<TerrainDiagnosticSample> {
        let shard = self.shards[Self::shard_index(world_x, world_z)]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        shard.entries.get(&(world_x, world_z)).cloned()
    }

    fn insert(
        &self,
        world_x: i32,
        world_z: i32,
        sample: TerrainDiagnosticSample,
    ) -> TerrainDiagnosticSample {
        let mut shard = self.shards[Self::shard_index(world_x, world_z)]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = shard.entries.get(&(world_x, world_z)) {
            return existing.clone();
        }
        if shard.entries.len() == DIAGNOSTIC_CACHE_ENTRIES_PER_SHARD
            && let Some(evicted) = shard.order.pop_front()
        {
            shard.entries.remove(&evicted);
        }
        shard.entries.insert((world_x, world_z), sample.clone());
        shard.order.push_back((world_x, world_z));
        sample
    }
}

/// Vanilla-like terrain generator. Holds the resolved state ids of every block
/// state it emits, so `generate` is allocation-free after the `Chunk::empty`
/// allocation it returns.
pub struct TerrainGenerator {
    seed: i64,
    geometry: ChunkGeometry,
    air: BlockStateId,
    bedrock: BlockStateId,
    stone: BlockStateId,
    dirt: BlockStateId,
    grass_block: BlockStateId,
    sand: BlockStateId,
    red_sand: BlockStateId,
    gravel: BlockStateId,
    podzol: BlockStateId,
    snow_block: BlockStateId,
    deepslate: BlockStateId,
    water: BlockStateId,
    cave_lava: BlockStateId,
    terracotta_layers: [BlockStateId; 8],
    biomes: BiomeRules,
    ores: OreRules,
    geological_ores: Option<GeologicalOreRules>,
    ore_generation_profile: &'static str,
    /// Native village plans. `None` leaves terrain routing unadapted and places
    /// no village pieces.
    village_plans: Option<Arc<VillagePlanSource>>,
    overworld_structures: Option<crate::overworld_structures::OverworldStructures>,
    /// Other jigsaw structures retain their own placement grids and biome gates.
    jigsaw_sources: Vec<Arc<VillagePlanSource>>,
    decorations: DecorationBlocks,
    tree_rules: HashMap<Identifier, TreeRule>,
    clay_rule: ClayRule,
    vanilla_worldgen: Option<(VanillaClimate, VanillaDensity)>,
    diagnostic_cache: DiagnosticCache,
    /// Block registry the generator was built with; spawn clearance reads
    /// fluid states from it.
    blocks: Arc<BlockRegistry>,
    /// Batched GPU density (`[server] worldgen_gpu`); `None` evaluates it on
    /// the CPU.
    #[cfg(feature = "gpu")]
    gpu: Option<gpu_service::GpuSolidService>,
    /// Chunk solids shared between the generations that read them.
    solid_cache: solid_cache::SolidCache,
}

#[derive(Clone)]
struct ColumnPlan {
    lx: u8,
    lz: u8,
    wx: i32,
    wz: i32,
    height: i32,
    top_non_air: i32,
    dirt_start: i32,
    biome: Identifier,
    surface: BlockStateId,
    fill: BlockStateId,
    vegetation_density: f64,
    hash: u64,
}

#[derive(Clone)]
struct DecorationBlocks {
    oak_log: Option<BlockStateId>,
    oak_leaves: Option<BlockStateId>,
    forest_log: Option<BlockStateId>,
    forest_leaves: Option<BlockStateId>,
    cold_log: Option<BlockStateId>,
    cold_leaves: Option<BlockStateId>,
    jungle_log: Option<BlockStateId>,
    jungle_leaves: Option<BlockStateId>,
    acacia_log: Option<BlockStateId>,
    acacia_leaves: Option<BlockStateId>,
    mangrove_log: Option<BlockStateId>,
    mangrove_leaves: Option<BlockStateId>,
    mangrove_roots: Option<BlockStateId>,
    wet_mangrove_roots: Option<BlockStateId>,
    muddy_mangrove_roots: Option<BlockStateId>,
    mud: Option<BlockStateId>,
    blue_orchid: Option<BlockStateId>,
    short_grass: Option<BlockStateId>,
    dandelion: Option<BlockStateId>,
    poppy: Option<BlockStateId>,
    pumpkin: Option<BlockStateId>,
    sugar_cane: Option<BlockStateId>,
    cactus: Option<BlockStateId>,
    seagrass: Option<BlockStateId>,
    clay: Option<BlockStateId>,
    kelp_plant: Option<BlockStateId>,
    kelp: Option<BlockStateId>,
    leaf_distances: [Option<[BlockStateId; 7]>; 6],
    leaf_supports: Vec<bool>,
}

impl DecorationBlocks {
    fn new(registry: &BlockRegistry) -> Self {
        let mut decorations = Self {
            oak_log: optional_block(registry, "minecraft:oak_log"),
            oak_leaves: optional_generated_leaves(registry, "minecraft:oak_leaves"),
            forest_log: optional_block(registry, "minecraft:birch_log")
                .or_else(|| optional_block(registry, "minecraft:oak_log")),
            forest_leaves: optional_generated_leaves(registry, "minecraft:birch_leaves")
                .or_else(|| optional_generated_leaves(registry, "minecraft:oak_leaves")),
            cold_log: optional_block(registry, "minecraft:spruce_log")
                .or_else(|| optional_block(registry, "minecraft:oak_log")),
            cold_leaves: optional_generated_leaves(registry, "minecraft:spruce_leaves")
                .or_else(|| optional_generated_leaves(registry, "minecraft:oak_leaves")),
            jungle_log: optional_block(registry, "minecraft:jungle_log")
                .or_else(|| optional_block(registry, "minecraft:oak_log")),
            jungle_leaves: optional_generated_leaves(registry, "minecraft:jungle_leaves")
                .or_else(|| optional_generated_leaves(registry, "minecraft:oak_leaves")),
            acacia_log: optional_block(registry, "minecraft:acacia_log")
                .or_else(|| optional_block(registry, "minecraft:oak_log")),
            acacia_leaves: optional_generated_leaves(registry, "minecraft:acacia_leaves")
                .or_else(|| optional_generated_leaves(registry, "minecraft:oak_leaves")),
            mangrove_log: optional_block(registry, "minecraft:mangrove_log"),
            mangrove_leaves: optional_generated_leaves(registry, "minecraft:mangrove_leaves"),
            mangrove_roots: optional_block(registry, "minecraft:mangrove_roots"),
            wet_mangrove_roots: None,
            muddy_mangrove_roots: optional_block(registry, "minecraft:muddy_mangrove_roots"),
            mud: optional_block(registry, "minecraft:mud"),
            blue_orchid: optional_block(registry, "minecraft:blue_orchid"),
            short_grass: optional_block(registry, "minecraft:short_grass"),
            dandelion: optional_block(registry, "minecraft:dandelion"),
            poppy: optional_block(registry, "minecraft:poppy"),
            pumpkin: optional_block(registry, "minecraft:pumpkin"),
            sugar_cane: optional_block(registry, "minecraft:sugar_cane"),
            cactus: optional_block(registry, "minecraft:cactus"),
            seagrass: optional_block(registry, "minecraft:seagrass"),
            clay: optional_block(registry, "minecraft:clay"),
            kelp_plant: optional_block(registry, "minecraft:kelp_plant"),
            kelp: optional_block(registry, "minecraft:kelp"),
            leaf_distances: [None; 6],
            leaf_supports: registry
                .states()
                .map(|state| {
                    mc_world::plant_rules_26_1_2::leaf_distance_from_state(registry, state.id) == 0
                })
                .collect(),
        };
        decorations.wet_mangrove_roots = decorations.mangrove_roots.and_then(|root| {
            let state = registry.by_id(root)?;
            let mut properties = state.properties.clone();
            properties
                .iter_mut()
                .find(|(name, _)| name == "waterlogged")?
                .1 = "true".to_owned();
            registry.by_name_and_props(&state.block.id, &properties)
        });
        decorations.leaf_distances = [
            decorations.oak_leaves,
            decorations.forest_leaves,
            decorations.cold_leaves,
            decorations.jungle_leaves,
            decorations.acacia_leaves,
            decorations.mangrove_leaves,
        ]
        .map(|leaf| {
            let leaf = leaf?;
            let state = registry.by_id(leaf)?;
            let mut properties = state.properties.clone();
            let property = properties.iter().position(|(key, _)| key == "distance")?;
            let mut distances = [leaf; 7];
            for (index, distance) in distances.iter_mut().enumerate() {
                properties[property].1 = (index + 1).to_string();
                *distance = registry.by_name_and_props(&state.block.id, &properties)?;
            }
            Some(distances)
        });
        decorations
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeKind {
    Oak,
    Birch,
    Spruce,
    Jungle,
    JungleBush,
    Acacia,
    Mangrove,
}

#[derive(Debug, Clone, Copy)]
struct TreeBlocks {
    kind: TreeKind,
    log: BlockStateId,
    leaves: BlockStateId,
}

#[derive(Debug, Clone, Copy)]
struct TreeLeafOffset {
    relative_y: i32,
    dx: i8,
    dz: i8,
    radius: i8,
}

fn tree_canopy_radius(kind: TreeKind, relative_y: i32) -> Option<i8> {
    match (kind, relative_y) {
        (TreeKind::Oak, -2 | -1) => Some(2),
        (TreeKind::Oak, 0 | 1) => Some(1),
        (TreeKind::Birch, -2 | -1) => Some(2),
        (TreeKind::Birch, 0) => Some(1),
        (TreeKind::Birch, 1) => Some(0),
        (TreeKind::Spruce, -4) => Some(1),
        (TreeKind::Spruce, -3 | -2) => Some(2),
        (TreeKind::Spruce, -1 | 0) => Some(1),
        (TreeKind::Spruce, 1) => Some(0),
        (TreeKind::Jungle, -2..=0) => Some(2),
        (TreeKind::Jungle, 1) => Some(1),
        (TreeKind::JungleBush, 0) => Some(2),
        (TreeKind::JungleBush, 1) => Some(1),
        (TreeKind::JungleBush, 2) => Some(0),
        (TreeKind::Acacia, -1 | 0) => Some(2),
        (TreeKind::Acacia, 1) => Some(1),
        (TreeKind::Mangrove, -3..=-1) => Some(2),
        (TreeKind::Mangrove, 0) => Some(1),
        (TreeKind::Mangrove, 1) => Some(0),
        _ => None,
    }
}

fn try_resolve_block(
    registry: &BlockRegistry,
    name: &'static str,
) -> Result<BlockStateId, TerrainGeneratorError> {
    let id = Identifier::parse(name).expect("static identifier");
    registry
        .block(&id)
        .map(|b| b.default)
        .ok_or(TerrainGeneratorError::MissingRequiredBlock { name })
}

fn resolve_block_or(registry: &BlockRegistry, name: &str, fallback: BlockStateId) -> BlockStateId {
    let id = Identifier::parse(name).expect("static identifier");
    registry.block(&id).map(|b| b.default).unwrap_or(fallback)
}

fn optional_block(registry: &BlockRegistry, name: &str) -> Option<BlockStateId> {
    let id = Identifier::parse(name).expect("static identifier");
    registry.block(&id).map(|b| b.default)
}

fn optional_generated_leaves(registry: &BlockRegistry, name: &str) -> Option<BlockStateId> {
    let id = Identifier::parse(name).expect("static identifier");
    registry
        .by_name_and_props(
            &id,
            &[
                ("distance".to_string(), "7".to_string()),
                ("persistent".to_string(), "false".to_string()),
                ("waterlogged".to_string(), "false".to_string()),
            ],
        )
        .or_else(|| registry.block(&id).map(|block| block.default))
}

fn checked_y_offset(y: i32, offset: i32) -> Option<i32> {
    i32::try_from(i64::from(y) + i64::from(offset)).ok()
}

fn heightmap_value_for_top(geometry: ChunkGeometry, top: i32) -> Option<u32> {
    let value = i64::from(top) + 1 - i64::from(geometry.min_y());
    let value = u32::try_from(value).ok()?;
    (value <= geometry.height() as u32).then_some(value)
}

fn world_block_coordinate(chunk: i32, local: u8) -> i32 {
    let coordinate = i64::from(chunk) * 16 + i64::from(local);
    i32::try_from(coordinate).expect("chunk lies outside the supported i32 block-coordinate range")
}

fn feature_hash(seed: i64, x: i32, y: i32, z: i32, salt: u64) -> u64 {
    let mut h = seed as u64 ^ salt;
    h ^= (x as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h = h.rotate_left(17);
    h ^= (y as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h = h.rotate_left(23);
    h ^= (z as i64 as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h.wrapping_mul(0x94D0_49BB_1331_11EB) ^ (h >> 31)
}

#[cfg(test)]
#[path = "terrain/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "terrain/deepslate_tests.rs"]
mod deepslate_tests;

#[cfg(test)]
#[path = "terrain/coastal_tests.rs"]
mod coastal_tests;

#[cfg(test)]
#[path = "terrain/village_water_tests.rs"]
mod village_water_tests;

#[cfg(test)]
#[path = "terrain/village_seed_tests.rs"]
mod village_seed_tests;

#[cfg(test)]
#[path = "terrain/profile_tests.rs"]
mod profile_tests;

#[cfg(test)]
#[path = "terrain/environment_tests.rs"]
mod environment_tests;

#[cfg(test)]
#[path = "terrain/modern_jigsaw_tests.rs"]
mod modern_jigsaw_tests;
