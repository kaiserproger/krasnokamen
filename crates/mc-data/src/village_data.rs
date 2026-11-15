//! Vanilla worldgen structure registries, resolved by reference.
//!
//! Four JSON registries, each loaded by identifier:
//!
//! - `worldgen/structure_set/*.json` — [`StructureSetSpec`]: the placement plus
//!   the weighted structures the set names.
//! - `worldgen/structure/*.json` — [`StructureSpec`]: the `minecraft:jigsaw`
//!   settings a village needs (start pool, depth, start height, heightmap
//!   projection, terrain adaptation, centre distance, biome tag, step, spawn
//!   overrides).
//! - `worldgen/template_pool/*.json` — [`TemplatePoolSpec`]: weighted elements
//!   of every element type, each naming its piece or placed feature, its
//!   projection and its processor list reference, plus the fallback chain.
//! - `worldgen/processor_list/*.json` — [`ProcessorListSpec`]: `rule`,
//!   `block_age`, `gravity`, `jigsaw_replacement` and `protected_blocks`
//!   processors with their rule fields.
//!
//! Only entries the caller names are read; the vanilla registries are never
//! enumerated, so registries Krasnokamen does not implement cannot fail a load.
//! One exception is deliberate and reads no codec:
//! [`VillageDataLoader::structures_in_step`] lists `worldgen/structure/*.json`
//! to recover the registry order vanilla's feature-placement seeding indexes
//! into, and takes only each entry's `step` string.
//! Anything a loaded entry reaches that is unsupported or malformed fails
//! closed, naming both the type id and the entry that referenced it. Referrers
//! are a within-load notion: a nested entry (a rule test inside an inline
//! processor list, a processor inside a list, an element inside a pool) names
//! the entry that carries it, and a top-level load names itself, since the
//! loader is handed one id at a time and the caller resolves references itself.
//! Required and optional fields follow the 26.1.2 codecs
//! (`net/minecraft/world/level/levelgen/structure/**`): nothing vanilla
//! requires is defaulted, and nothing vanilla defaults is required. Values that
//! vanilla's codec rejects are rejected here too.
//!
//! Piece NBT is deliberately *not* read here: `mc-worldgen`'s structure loader
//! owns the template (palette, blocks, jigsaw blocks) and the jigsaw walk. This
//! module stops at the ids the JSON names, and the engine follows them.

use std::path::{Path, PathBuf};

use serde_json::Value;
use thiserror::Error;

use crate::vanilla_feature_closure::BlockStateSpec;
use crate::{Identifier, ResourcePathError, read_json_resource};

/// How a reference is named in error messages, e.g. "template pool".
type EntryKind = &'static str;

const STRUCTURE_SET: EntryKind = "structure set";
const STRUCTURE_PLACEMENT: EntryKind = "structure placement";
const EXCLUSION_ZONE: EntryKind = "exclusion zone";
const STRUCTURE: EntryKind = "structure";
const HEIGHT_PROVIDER: EntryKind = "height provider";
const SPAWN_OVERRIDE: EntryKind = "structure spawn override";
const TEMPLATE_POOL: EntryKind = "template pool";
const POOL_ELEMENT: EntryKind = "template pool element";
const PROCESSOR_LIST: EntryKind = "processor list";
const PROCESSOR: EntryKind = "structure processor";
const RULE_TEST: EntryKind = "rule test";
const POS_RULE_TEST: EntryKind = "pos rule test";
const BLOCK_STATE: EntryKind = "block state";

/// `VerticalAnchor` bounds: `DimensionType.MIN_Y`..=`DimensionType.MAX_Y`.
const MIN_Y: i32 = -2032;
const MAX_Y: i32 = 2031;
/// `DimensionType.Y_SIZE`, the default and maximum vertical reach.
const Y_SIZE: i32 = MAX_Y - MIN_Y + 1;
/// `JigsawStructure.MAX_TOTAL_STRUCTURE_RANGE`.
const MAX_STRUCTURE_RANGE: i32 = 128;

#[derive(Debug, Error)]
pub enum VillageDataError {
    #[error("village data io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("village data parse error at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid identifier {value:?} referenced by {referrer}")]
    InvalidIdentifier { referrer: Identifier, value: String },
    #[error(transparent)]
    ResourcePath(#[from] ResourcePathError),
    #[error("missing {kind} {id} referenced by {referrer} at {path}")]
    MissingEntry {
        kind: EntryKind,
        id: Identifier,
        referrer: Identifier,
        path: PathBuf,
    },
    #[error("non-minecraft {kind} {id} referenced by {referrer} is outside the vanilla cache")]
    UnsupportedNamespace {
        kind: EntryKind,
        id: Identifier,
        referrer: Identifier,
    },
    #[error("structure entry file {path} does not name a valid identifier")]
    InvalidStructureFile { path: PathBuf },
    #[error("unsupported {kind} type {type_id} referenced by {referrer}")]
    UnsupportedType {
        kind: EntryKind,
        type_id: Identifier,
        referrer: Identifier,
    },
    #[error("{kind} {entry} has unsupported {field}: {value}")]
    UnsupportedField {
        kind: EntryKind,
        entry: Identifier,
        field: &'static str,
        value: String,
    },
    #[error("{kind} {entry} is missing field {field}")]
    MissingField {
        kind: EntryKind,
        entry: Identifier,
        field: &'static str,
    },
    #[error("{kind} {entry} has invalid value for {field}: {value}")]
    InvalidField {
        kind: EntryKind,
        entry: Identifier,
        field: &'static str,
        value: String,
    },
}

/// One weighted structure of a structure set, as `StructureSelectionEntry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureSetEntry {
    pub structure: Identifier,
    pub weight: i32,
}

/// `minecraft:random_spread`, the only placement type villages use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RandomSpreadType {
    Linear,
    Triangular,
}

/// `FrequencyReductionMethod`; `default` halves nothing and is the fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrequencyReductionMethod {
    Default,
    LegacyType1,
    LegacyType2,
    LegacyType3,
}

/// `StructurePlacement.ExclusionZone`: another set whose nearby placements
/// forbid this one. The named set is loaded by reference by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExclusionZone {
    pub other_set: Identifier,
    pub chunk_count: i32,
}

/// The base `StructurePlacement` fields every placement carries.
#[derive(Debug, Clone, PartialEq)]
pub struct RandomSpreadPlacement {
    pub spacing: i32,
    pub separation: i32,
    /// `ExtraCodecs.NON_NEGATIVE_INT`, required by the codec.
    pub salt: i32,
    /// `spread_type`, `linear` by default.
    pub spread_type: RandomSpreadType,
    /// `locate_offset`, zero by default and bounded to ±16 per axis.
    pub locate_offset: [i32; 3],
    /// `frequency_reduction_method`, `default` by default.
    pub frequency_reduction_method: FrequencyReductionMethod,
    /// `frequency`, `1.0` by default.
    pub frequency: f32,
    /// `exclusion_zone`, absent by default.
    pub exclusion_zone: Option<ExclusionZone>,
    /// Minimum horizontal block distance from the world origin, for Moog's
    /// advanced spread. Vanilla random spread has no such restriction.
    pub min_distance_from_world_origin: Option<i32>,
}

/// How a structure set scatters its structures. `minecraft:concentric_rings`
/// (strongholds) and anything else is not implemented and fails closed.
#[derive(Debug, Clone, PartialEq)]
pub enum PlacementSpec {
    RandomSpread(RandomSpreadPlacement),
}

/// A structure set: placement plus the weighted structures it names.
#[derive(Debug, Clone, PartialEq)]
pub struct StructureSetSpec {
    pub id: Identifier,
    pub placement: PlacementSpec,
    pub structures: Vec<StructureSetEntry>,
}

/// `biomes`, a homogeneous holder set: a `#`-prefixed tag, a single biome, or a
/// list of biomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BiomeSet {
    Tag(Identifier),
    Biomes(Vec<Identifier>),
}

/// `VerticalAnchor`: an absolute Y, or an offset from the dimension's floor or
/// ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAnchor {
    Absolute(i32),
    AboveBottom(i32),
    BelowTop(i32),
}

/// A `HeightProvider`; the providers whose sampling Krasnokamen needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeightProviderSpec {
    Constant(VerticalAnchor),
    Uniform {
        min_inclusive: VerticalAnchor,
        max_inclusive: VerticalAnchor,
    },
    Trapezoid {
        min_inclusive: VerticalAnchor,
        max_inclusive: VerticalAnchor,
        plateau: i32,
    },
}

/// `Heightmap.Types`, as named by `project_start_to_heightmap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeightmapType {
    WorldSurfaceWg,
    WorldSurface,
    OceanFloorWg,
    OceanFloor,
    MotionBlocking,
    MotionBlockingNoLeaves,
}

/// `GenerationStep.Decoration`, the generation step a structure claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureStep {
    RawGeneration,
    Lakes,
    LocalModifications,
    UndergroundStructures,
    SurfaceStructures,
    Strongholds,
    UndergroundOres,
    UndergroundDecoration,
    FluidSprings,
    VegetalDecoration,
    TopLayerModification,
}

impl StructureStep {
    /// `GenerationStep.Decoration.ordinal()`, the step index vanilla's
    /// `ChunkGenerator.applyBiomeDecoration` seeds structure placement with
    /// (`WorldgenRandom.setFeatureSeed(seed, index, step)` adds `10000 * step`).
    /// The variants are declared in the enum's own order.
    #[must_use]
    pub const fn decoration_ordinal(self) -> i32 {
        match self {
            Self::RawGeneration => 0,
            Self::Lakes => 1,
            Self::LocalModifications => 2,
            Self::UndergroundStructures => 3,
            Self::SurfaceStructures => 4,
            Self::Strongholds => 5,
            Self::UndergroundOres => 6,
            Self::UndergroundDecoration => 7,
            Self::FluidSprings => 8,
            Self::VegetalDecoration => 9,
            Self::TopLayerModification => 10,
        }
    }
}

/// `TerrainAdjustment`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainAdaptation {
    None,
    Bury,
    BeardThin,
    BeardBox,
    Encapsulate,
}

/// `LiquidSettings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiquidSettings {
    IgnoreWaterlogging,
    ApplyWaterlogging,
}

/// `MobCategory`, the spawn override key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MobCategory {
    Monster,
    Creature,
    Ambient,
    Axolotls,
    UndergroundWaterCreature,
    WaterCreature,
    WaterAmbient,
    Misc,
}

/// `StructureSpawnOverride.BoundingBoxType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnBoundingBox {
    Piece,
    Full,
}

/// One `SpawnerData` of a weighted spawn list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightedSpawn {
    pub weight: i32,
    pub entity_type: Identifier,
    pub min_count: i32,
    pub max_count: i32,
}

/// One `MobCategory`'s spawn override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnOverrideEntry {
    pub category: MobCategory,
    pub bounding_box: SpawnBoundingBox,
    pub spawns: Vec<WeightedSpawn>,
}

/// `JigsawStructure.MaxDistance`: `horizontal` and `vertical`, written as one
/// integer when they are equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxDistance {
    pub horizontal: i32,
    pub vertical: i32,
}

/// `DimensionPadding`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DimensionPadding {
    pub bottom: i32,
    pub top: i32,
}

/// Extra overworld start rules from Moog's Structure Lib.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoogStartChecks {
    pub cannot_spawn_in_liquid: bool,
    /// Chunk radius and maximum first-free-height difference.
    pub terrain_check: Option<(i32, i32)>,
    pub biome_radius: Option<i32>,
    pub min_y: Option<i32>,
    pub max_y: Option<i32>,
    pub average_land: bool,
}


/// A jigsaw structure and the settings needed to plan it.
#[derive(Debug, Clone, PartialEq)]
pub struct StructureSpec {
    pub id: Identifier,
    /// The namespaced structure codec ID.
    pub structure_type: Identifier,
    /// Custom start checks; absent for vanilla jigsaws.
    pub moog_start_checks: Option<MoogStartChecks>,
    pub biomes: BiomeSet,
    /// `spawn_overrides`, keyed by mob category; required by the codec.
    pub spawn_overrides: Vec<SpawnOverrideEntry>,
    /// `step`, required by the codec.
    pub step: StructureStep,
    /// `terrain_adaptation`, `none` by default.
    pub terrain_adaptation: TerrainAdaptation,
    pub start_pool: Identifier,
    /// `start_jigsaw_name`, absent by default.
    pub start_jigsaw_name: Option<Identifier>,
    /// `size`, the maximum jigsaw depth.
    pub size: i32,
    pub start_height: HeightProviderSpec,
    /// `use_expansion_hack`, required by the codec.
    pub use_expansion_hack: bool,
    pub project_start_to_heightmap: Option<HeightmapType>,
    pub max_distance_from_center: MaxDistance,
    /// `dimension_padding`, zero by default.
    pub dimension_padding: DimensionPadding,
    /// `liquid_settings`, `apply_waterlogging` by default.
    pub liquid_settings: LiquidSettings,
    pub pool_aliases: Vec<PoolAliasSpec>,
}

/// Pool substitutions sampled once per structure, including correlated groups.
#[derive(Debug, Clone, PartialEq)]
pub enum PoolAliasSpec {
    Direct {
        alias: Identifier,
        target: Identifier,
    },
    Random {
        alias: Identifier,
        targets: Vec<(i32, Identifier)>,
    },
    RandomGroup {
        groups: Vec<(i32, Vec<PoolAliasSpec>)>,
    },
}

/// `StructureTemplatePool.Projection`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    Rigid,
    TerrainMatching,
}

/// The `processors` field of a single pool element: a processor list reference
/// or an inline list. Required by the codec, so an element always carries one.
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessorRef {
    List(Identifier),
    Inline(Vec<StructureProcessorSpec>),
}

/// The settings `single_pool_element` and `legacy_single_pool_element` share.
/// The two are distinct element types because vanilla resolves their jigsaw
/// blocks differently, so the spec keeps them apart.
#[derive(Debug, Clone, PartialEq)]
pub struct SingleElementSpec {
    pub location: Identifier,
    pub projection: Projection,
    pub processors: ProcessorRef,
    /// `override_liquid_settings`, absent by default.
    pub override_liquid_settings: Option<LiquidSettings>,
}

/// One pool element.
#[derive(Debug, Clone, PartialEq)]
pub enum PoolElementSpec {
    /// `empty_pool_element`; vanilla fixes its projection to `terrain_matching`
    /// and the codec carries no fields.
    Empty,
    Single(SingleElementSpec),
    LegacySingle(SingleElementSpec),
    /// `list_pool_element`, whose own elements are parsed recursively.
    List {
        elements: Vec<PoolElementSpec>,
        projection: Projection,
    },
    /// `feature_pool_element`; the placed feature is loaded by reference by the
    /// feature closure, so only its id is carried here.
    Feature {
        feature: Identifier,
        projection: Projection,
    },
}

/// One weighted element of a template pool.
#[derive(Debug, Clone, PartialEq)]
pub struct PoolElementEntry {
    pub weight: i32,
    pub element: PoolElementSpec,
}

/// A template pool: its fallback and its weighted elements.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplatePoolSpec {
    pub id: Identifier,
    pub fallback: Identifier,
    pub elements: Vec<PoolElementEntry>,
}

/// `RuleTest`: what a rule matches against.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleTestSpec {
    AlwaysTrue,
    BlockMatch {
        block: Identifier,
    },
    BlockStateMatch {
        state: BlockStateSpec,
    },
    /// `tag`, a block tag id written without its `#`.
    TagMatch {
        tag: Identifier,
    },
    RandomBlockMatch {
        block: Identifier,
        probability: f32,
    },
    RandomBlockStateMatch {
        state: BlockStateSpec,
        probability: f32,
    },
}

/// `Direction.Axis`, as used by `axis_aligned_linear_pos`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

/// `PosRuleTest`: what a rule matches against in template/world space.
#[derive(Debug, Clone, PartialEq)]
pub enum PosRuleTestSpec {
    AlwaysTrue,
    LinearPos {
        min_chance: f32,
        max_chance: f32,
        min_dist: i32,
        max_dist: i32,
    },
    AxisAlignedLinearPos {
        min_chance: f32,
        max_chance: f32,
        min_dist: i32,
        max_dist: i32,
        axis: Axis,
    },
}

/// One `ProcessorRule`: first matching rule of a `minecraft:rule` processor
/// wins, and `output_state` is the outcome it writes.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessorRuleSpec {
    pub input_predicate: RuleTestSpec,
    pub location_predicate: RuleTestSpec,
    /// `position_predicate`, `always_true` by default.
    pub position_predicate: PosRuleTestSpec,
    pub output_state: BlockStateSpec,
    pub append_loot: Option<Identifier>,
}

/// Mob choice and base-spawner settings for a Moog spawner processor.
#[derive(Debug, Clone, PartialEq)]
pub struct MoogSpawnerSpec {
    pub mobs: Vec<(Identifier, f64)>,
    /// Delay, minimum/maximum delay, count, nearby limit, player range, range.
    pub fields: [i16; 7],
}

/// Processor types supported by the shared jigsaw template pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum StructureProcessorSpec {
    MoogSpawner(MoogSpawnerSpec),
    BlockRot {
        integrity: f32,
        rottable_blocks: Option<Identifier>,
    },
    Capped {
        delegate: Box<StructureProcessorSpec>,
        limit: i32,
    },
    Rule {
        rules: Vec<ProcessorRuleSpec>,
    },
    BlockAge {
        mossiness: f32,
    },
    Gravity {
        heightmap: HeightmapType,
        offset: i32,
    },
    JigsawReplacement,
    /// `protected_blocks`; `cannot_replace` is the block tag id with its `#`
    /// stripped, since the codec demands the `#` in the stored form.
    ProtectedBlocks {
        cannot_replace: Identifier,
    },
}

/// A processor list, in application order.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessorListSpec {
    pub id: Identifier,
    pub processors: Vec<StructureProcessorSpec>,
}

/// Reads the four worldgen structure registries out of a vanilla cache's
/// `data/minecraft/worldgen` directory.
pub struct VillageDataLoader {
    worldgen_dir: PathBuf,
}

impl VillageDataLoader {
    #[must_use]
    pub fn new(worldgen_dir: impl Into<PathBuf>) -> Self {
        Self {
            worldgen_dir: worldgen_dir.into(),
        }
    }

    /// `structure_set/<id>.json`: placement and the weighted structure list.
    pub fn load_structure_set(
        &self,
        id: &Identifier,
    ) -> Result<StructureSetSpec, VillageDataError> {
        let value = self.read_entry(STRUCTURE_SET, "structure_set", id, id)?;
        let placement = parse_placement(id, field_of(&value, "placement", STRUCTURE_SET, id)?)?;
        let entries = field_array(&value, "structures", STRUCTURE_SET, id)?;
        let mut structures = Vec::with_capacity(entries.len());
        for entry in entries {
            structures.push(StructureSetEntry {
                structure: parse_id(id, field_str(entry, "structure", STRUCTURE_SET, id)?)?,
                // `ExtraCodecs.POSITIVE_INT`
                weight: ranged_i32_field(entry, "weight", STRUCTURE_SET, id, 1, i32::MAX)?,
            });
        }
        Ok(StructureSetSpec {
            id: id.clone(),
            placement,
            structures,
        })
    }

    /// `structure/<id>.json`: the settings a `minecraft:jigsaw` structure runs
    /// with.
    pub fn load_structure(&self, id: &Identifier) -> Result<StructureSpec, VillageDataError> {
        let value = self.read_entry(STRUCTURE, "structure", id, id)?;
        parse_structure(id, &value)
    }

    /// `template_pool/<id>.json`: the fallback and the weighted elements.
    pub fn load_template_pool(
        &self,
        id: &Identifier,
    ) -> Result<TemplatePoolSpec, VillageDataError> {
        let value = self.read_entry(TEMPLATE_POOL, "template_pool", id, id)?;
        parse_template_pool(id, &value)
    }

    /// `processor_list/<id>.json`: the processors in application order.
    pub fn load_processor_list(
        &self,
        id: &Identifier,
    ) -> Result<ProcessorListSpec, VillageDataError> {
        let value = self.read_entry(PROCESSOR_LIST, "processor_list", id, id)?;
        parse_processor_list(id, &value)
    }

    /// The ids of every `structure/*.json` entry whose `step` is `step`, in
    /// vanilla's registry order.
    ///
    /// The one place this module reads a registry *directory* instead of an
    /// entry a caller named, and it reads exactly one string out of each file:
    /// the `step`. Vanilla registers a registry's entries in identifier order —
    /// `ResourceManagerRegistryLoadTask.load` registers
    /// `loadedEntries.entrySet().stream().sorted(Entry.comparingByKey())`, and
    /// `Identifier.compareTo` compares the path first and the namespace second —
    /// and `ChunkGenerator.applyBiomeDecoration` seeds the random its structure
    /// placement runs on from `(index of the structure within its step, step
    /// ordinal)`, an index nothing else in the village data names.
    /// No other field of a structure entry is parsed here, so the structure
    /// types this engine does not implement contribute their step and nothing
    /// more; a file vanilla's registry load would reject is rejected here too.
    ///
    /// # Errors
    ///
    /// [`VillageDataError`] when the structure directory cannot be read, a file
    /// is not JSON, an entry does not carry a valid `step`, or a file name is
    /// not a valid identifier.
    pub fn structures_in_step(
        &self,
        step: StructureStep,
    ) -> Result<Vec<Identifier>, VillageDataError> {
        let ids = self.registry_entries("structure")?;
        let mut in_step = Vec::new();
        for id in ids {
            let value = self.read_entry(STRUCTURE, "structure", &id, &id)?;
            if enum_field(&value, "step", STRUCTURE, &id, STRUCTURE_STEPS)? == step {
                in_step.push(id);
            }
        }
        // Vanilla's order is `Identifier.compareTo`: **path first**, then
        // namespace, both by UTF-16 code unit (the registry load sorts the
        // loaded entries by that key before registering them). Identifiers are
        // ASCII, so byte order is that order — but the derived `Ord` on
        // `Identifier` compares the joined `namespace:path` string, where `:`
        // sorts after a digit and so orders `minecraft:z` before `a0:x`.
        in_step.sort_by(|first, second| {
            (first.path(), first.namespace()).cmp(&(second.path(), second.namespace()))
        });
        Ok(in_step)
    }

    /// List the available structure-set identifiers in deterministic registry order.
    ///
    /// # Errors
    ///
    /// Returns an error if a registry directory cannot be read or a resource
    /// path is not a valid identifier.
    pub fn structure_sets(&self) -> Result<Vec<Identifier>, VillageDataError> {
        self.registry_entries("structure_set")
    }

    fn registry_entries(&self, registry: &str) -> Result<Vec<Identifier>, VillageDataError> {
        let root = self.worldgen_dir.join(registry);
        let mut ids = Vec::new();
        collect_structure_files(&root, "minecraft", "", &mut ids)?;
        if let Some(data) = self.worldgen_dir.parent().and_then(Path::parent) {
            let entries = std::fs::read_dir(data).map_err(|source| VillageDataError::Io {
                path: data.to_path_buf(),
                source,
            })?;
            for entry in entries {
                let entry = entry.map_err(|source| VillageDataError::Io {
                    path: data.to_path_buf(),
                    source,
                })?;
                let path = entry.path();
                let Some(namespace) = entry.file_name().to_str().map(str::to_owned) else {
                    return Err(VillageDataError::InvalidStructureFile { path });
                };
                let file_type = entry.file_type().map_err(|source| VillageDataError::Io {
                    path: path.clone(),
                    source,
                })?;
                if namespace == "minecraft" || !file_type.is_dir() {
                    continue;
                }
                let directory = path.join("worldgen").join(registry);
                if directory.is_dir() {
                    collect_structure_files(&directory, &namespace, "", &mut ids)?;
                }
            }
        }
        ids.sort_by(|first, second| {
            (first.path(), first.namespace()).cmp(&(second.path(), second.namespace()))
        });
        Ok(ids)
    }

    fn read_entry(
        &self,
        kind: EntryKind,
        dir: &str,
        id: &Identifier,
        referrer: &Identifier,
    ) -> Result<Value, VillageDataError> {
        let namespace_dir;
        let worldgen_dir = if id.namespace() == "minecraft" {
            &self.worldgen_dir
        } else {
            let data = self.worldgen_dir.parent().and_then(Path::parent).ok_or_else(|| {
                VillageDataError::UnsupportedNamespace {
                    kind,
                    id: id.clone(),
                    referrer: referrer.clone(),
                }
            })?;
            namespace_dir = data.join(id.namespace()).join("worldgen");
            &namespace_dir
        };
        if id.namespace() != "minecraft" {
            let root = worldgen_dir.join(dir);
            match std::fs::metadata(&root) {
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                    return Err(VillageDataError::MissingEntry {
                        kind,
                        id: id.clone(),
                        referrer: referrer.clone(),
                        path: root.join(format!("{}.json", id.path())),
                    });
                }
                Err(source) => return Err(VillageDataError::Io { path: root, source }),
                Ok(_) => {}
            }
        }
        let io_error = |path, source| VillageDataError::Io { path, source };
        let parse_error = |path, source| VillageDataError::Parse { path, source };
        match crate::open_worldgen_entry(worldgen_dir, dir, id)? {
            crate::WorldgenEntry::Present(opened) => {
                read_json_resource(opened, &io_error, &parse_error)
            }
            crate::WorldgenEntry::Missing(path) => Err(VillageDataError::MissingEntry {
                kind,
                id: id.clone(),
                referrer: referrer.clone(),
                path,
            }),
        }
    }
}

/// Collect `<namespace>:<relative path>` ids of every `*.json` under a
/// registry directory, recursing into subdirectories (vanilla's registry
/// resource listing is recursive).
fn collect_structure_files(
    root: &std::path::Path,
    namespace: &str,
    prefix: &str,
    ids: &mut Vec<Identifier>,
) -> Result<(), VillageDataError> {
    let entries = std::fs::read_dir(root).map_err(|source| VillageDataError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| VillageDataError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return Err(VillageDataError::InvalidStructureFile { path });
        };
        if path.is_dir() {
            collect_structure_files(&path, namespace, &format!("{prefix}{name}/"), ids)?;
            continue;
        }
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        let id = Identifier::parse(format!("{namespace}:{prefix}{stem}"))
            .map_err(|_| VillageDataError::InvalidStructureFile { path: path.clone() })?;
        ids.push(id);
    }
    Ok(())
}

fn parse_placement(
    referrer: &Identifier,
    value: &Value,
) -> Result<PlacementSpec, VillageDataError> {
    let type_id = parse_id(
        referrer,
        field_str(value, "type", STRUCTURE_PLACEMENT, referrer)?,
    )?;
    let moog = type_id.as_str() == "moogs_structures:advanced_random_spread";
    if type_id.as_str() != "minecraft:random_spread" && !moog {
        return Err(VillageDataError::UnsupportedType {
            kind: STRUCTURE_PLACEMENT,
            type_id,
            referrer: referrer.clone(),
        });
    }
    if moog {
        for field in ["super_exclusion_zone", "enhanced_exclusion_zone", "when_replacing"] {
            if let Some(unsupported) = value.get(field) {
                return Err(VillageDataError::UnsupportedField {
                    kind: STRUCTURE_PLACEMENT,
                    entry: referrer.clone(),
                    field,
                    value: unsupported.to_string(),
                });
            }
        }
    }
    let mut placement = RandomSpreadPlacement {
        spacing: ranged_i32_field(value, "spacing", STRUCTURE_PLACEMENT, referrer, 0, 4096)?,
        separation: ranged_i32_field(value, "separation", STRUCTURE_PLACEMENT, referrer, 0, 4096)?,
        salt: ranged_i32_field(value, "salt", STRUCTURE_PLACEMENT, referrer, 0, i32::MAX)?,
        spread_type: optional_enum_field(
            value,
            "spread_type",
            STRUCTURE_PLACEMENT,
            referrer,
            RANDOM_SPREAD_TYPES,
            RandomSpreadType::Linear,
        )?,
        locate_offset: parse_locate_offset(referrer, value)?,
        frequency_reduction_method: optional_enum_field(
            value,
            "frequency_reduction_method",
            STRUCTURE_PLACEMENT,
            referrer,
            FREQUENCY_REDUCTION_METHODS,
            FrequencyReductionMethod::Default,
        )?,
        frequency: optional_ranged_f32_field(
            value,
            "frequency",
            STRUCTURE_PLACEMENT,
            referrer,
            0.0,
            1.0,
            1.0,
        )?,
        exclusion_zone: parse_exclusion_zone(referrer, value.get("exclusion_zone"))?,
        min_distance_from_world_origin: if moog {
            Some(optional_ranged_i32_field(
                value,
                "min_distance_from_world_origin",
                STRUCTURE_PLACEMENT,
                referrer,
                0,
                i32::MAX,
                0,
            )?)
        } else {
            None
        },
    };
    // `RandomSpreadStructurePlacement.validate`
    if placement.spacing <= placement.separation {
        return Err(VillageDataError::InvalidField {
            kind: STRUCTURE_PLACEMENT,
            entry: referrer.clone(),
            field: "spacing",
            value: format!(
                "{} is not larger than separation {}",
                placement.spacing, placement.separation
            ),
        });
    }
    if moog {
        // Moog's default configuration scales both grid dimensions by 1.65,
        // rounds to the nearest chunk and keeps separation below spacing.
        let scale = |value: i32| (value * 165 + 50) / 100;
        placement.spacing = scale(placement.spacing).max(1);
        placement.separation = scale(placement.separation).min(placement.spacing - 1);
    }
    Ok(PlacementSpec::RandomSpread(placement))
}

fn parse_locate_offset(referrer: &Identifier, value: &Value) -> Result<[i32; 3], VillageDataError> {
    // `Vec3i.offsetCodec(16)`, absent by default.
    let Some(offset) = value.get("locate_offset") else {
        return Ok([0, 0, 0]);
    };
    let entries = offset
        .as_array()
        .ok_or_else(|| VillageDataError::InvalidField {
            kind: STRUCTURE_PLACEMENT,
            entry: referrer.clone(),
            field: "locate_offset",
            value: offset.to_string(),
        })?;
    let [x, y, z] = entries.as_slice() else {
        return Err(VillageDataError::InvalidField {
            kind: STRUCTURE_PLACEMENT,
            entry: referrer.clone(),
            field: "locate_offset",
            value: offset.to_string(),
        });
    };
    let mut parsed = [0; 3];
    for (slot, raw) in parsed.iter_mut().zip([x, y, z]) {
        let coordinate = raw.as_i64().and_then(|raw| i32::try_from(raw).ok());
        // `offsetCodec(16)` accepts strictly less than 16 per axis.
        match coordinate {
            Some(coordinate) if (-15..=15).contains(&coordinate) => *slot = coordinate,
            _ => {
                return Err(VillageDataError::InvalidField {
                    kind: STRUCTURE_PLACEMENT,
                    entry: referrer.clone(),
                    field: "locate_offset",
                    value: offset.to_string(),
                });
            }
        }
    }
    Ok(parsed)
}

fn parse_exclusion_zone(
    referrer: &Identifier,
    value: Option<&Value>,
) -> Result<Option<ExclusionZone>, VillageDataError> {
    let Some(value) = value else {
        return Ok(None);
    };
    Ok(Some(ExclusionZone {
        other_set: parse_id(
            referrer,
            field_str(value, "other_set", EXCLUSION_ZONE, referrer)?,
        )?,
        chunk_count: ranged_i32_field(value, "chunk_count", EXCLUSION_ZONE, referrer, 1, 16)?,
    }))
}

fn parse_moog_start_checks(
    id: &Identifier,
    value: &Value,
) -> Result<MoogStartChecks, VillageDataError> {
    let cannot_spawn_in_liquid = value
        .get("cannot_spawn_in_liquid")
        .map(|_| bool_field(value, "cannot_spawn_in_liquid", STRUCTURE, id))
        .transpose()?
        .unwrap_or(false);
    let terrain_check = match (
        value.get("terrain_height_radius_check"),
        value.get("allowed_terrain_height_range"),
    ) {
        (None, None) => None,
        (Some(_), Some(_)) => Some((
            ranged_i32_field(value, "terrain_height_radius_check", STRUCTURE, id, 1, 100)?,
            ranged_i32_field(value, "allowed_terrain_height_range", STRUCTURE, id, 1, 1000)?,
        )),
        _ => {
            return Err(VillageDataError::InvalidField {
                kind: STRUCTURE,
                entry: id.clone(),
                field: "terrain_height_radius_check",
                value: "terrain radius and allowed range must be specified together".to_owned(),
            });
        }
    };
    let biome_radius = value.get("valid_biome_radius_check")
        .map(|_| ranged_i32_field(value, "valid_biome_radius_check", STRUCTURE, id, 1, 100))
        .transpose()?;
    let (min_y, max_y) = if let Some(allowance) = value.get("y_allowance") {
        if !allowance.is_object() {
            return Err(VillageDataError::InvalidField {
                kind: STRUCTURE, entry: id.clone(), field: "y_allowance",
                value: allowance.to_string(),
            });
        }
        if allowance.as_object().is_some_and(|object| {
            object.keys().any(|key| key != "min_y_allowed" && key != "max_y_allowed")
        }) {
            return Err(VillageDataError::InvalidField {
                kind: STRUCTURE, entry: id.clone(), field: "y_allowance",
                value: "unsupported y_allowance field".to_owned(),
            });
        }
        let min = allowance.get("min_y_allowed")
            .map(|_| i32_field(allowance, "min_y_allowed", STRUCTURE, id)).transpose()?;
        let max = allowance.get("max_y_allowed")
            .map(|_| i32_field(allowance, "max_y_allowed", STRUCTURE, id)).transpose()?;
        if matches!((min, max), (Some(min), Some(max)) if min > max) {
            return Err(VillageDataError::InvalidField {
                kind: STRUCTURE, entry: id.clone(), field: "y_allowance",
                value: "minimum exceeds maximum".to_owned(),
            });
        }
        (min, max)
    } else {
        (None, None)
    };
    let average_land = match value.get("burying_type") {
        None => false,
        Some(Value::String(burying)) if burying == "AVERAGE_LAND" => true,
        Some(unsupported) => {
            return Err(VillageDataError::InvalidField {
                kind: STRUCTURE, entry: id.clone(), field: "burying_type",
                value: format!("unsupported Moog burial: {unsupported}"),
            });
        }
    };
    Ok(MoogStartChecks {
        cannot_spawn_in_liquid,
        terrain_check,
        biome_radius,
        min_y,
        max_y,
        average_land,
    })
}

fn parse_structure(id: &Identifier, value: &Value) -> Result<StructureSpec, VillageDataError> {
    let structure_type = parse_id(id, field_str(value, "type", STRUCTURE, id)?)?;
    let is_moog = structure_type.as_str()
        == "moogs_structures:moogs_structures_generic_jigsaw_structure";
    if !crate::worldgen_structure_types::supports_structure_type(&structure_type) {
        return Err(VillageDataError::UnsupportedType {
            kind: STRUCTURE,
            type_id: structure_type,
            referrer: id.clone(),
        });
    }
    let moog_start_checks = if is_moog {
        for field in [
            "land_search_direction",
            "pools_that_ignore_boundaries",
        ] {
            if let Some(unsupported) = value.get(field) {
                return Err(VillageDataError::InvalidField {
                    kind: STRUCTURE,
                    entry: id.clone(),
                    field,
                    value: format!("unsupported Moog rule: {unsupported}"),
                });
            }
        }
        Some(parse_moog_start_checks(id, value)?)
    } else {
        None
    };
    let terrain_adaptation = optional_enum_field(
        value,
        "terrain_adaptation",
        STRUCTURE,
        id,
        TERRAIN_ADAPTATIONS,
        TerrainAdaptation::None,
    )?;
    let max_distance_from_center = match value.get("max_distance_from_center") {
        None if is_moog => MaxDistance { horizontal: 80, vertical: 80 },
        _ => parse_max_distance(
            id,
            field_of(value, "max_distance_from_center", STRUCTURE, id)?,
        )?,
    };
    // `JigsawStructure.verifyRange`
    let edge = match terrain_adaptation {
        TerrainAdaptation::None => 0,
        TerrainAdaptation::Bury
        | TerrainAdaptation::BeardThin
        | TerrainAdaptation::BeardBox
        | TerrainAdaptation::Encapsulate => 12,
    };
    if max_distance_from_center.horizontal + edge > MAX_STRUCTURE_RANGE {
        return Err(VillageDataError::InvalidField {
            kind: STRUCTURE,
            entry: id.clone(),
            field: "max_distance_from_center",
            value: format!(
                "{} plus terrain adaptation {edge} exceeds {MAX_STRUCTURE_RANGE}",
                max_distance_from_center.horizontal
            ),
        });
    }
    Ok(StructureSpec {
        id: id.clone(),
        structure_type,
        moog_start_checks,
        pool_aliases: value
            .get("pool_aliases")
            .map(|aliases| {
                aliases
                    .as_array()
                    .ok_or_else(|| VillageDataError::InvalidField {
                        kind: STRUCTURE,
                        entry: id.clone(),
                        field: "pool_aliases",
                        value: aliases.to_string(),
                    })?
                    .iter()
                    .map(|alias| parse_pool_alias(id, alias))
                    .collect()
            })
            .transpose()?
            .unwrap_or_default(),
        biomes: parse_biome_set(id, field_of(value, "biomes", STRUCTURE, id)?)?,
        spawn_overrides: parse_spawn_overrides(
            id,
            field_of(value, "spawn_overrides", STRUCTURE, id)?,
        )?,
        step: enum_field(value, "step", STRUCTURE, id, STRUCTURE_STEPS)?,
        terrain_adaptation,
        start_pool: parse_id(id, field_str(value, "start_pool", STRUCTURE, id)?)?,
        start_jigsaw_name: parse_optional_id(value, "start_jigsaw_name", STRUCTURE, id)?,
        // `Codec.intRange(0, 20)`
        size: ranged_i32_field(value, "size", STRUCTURE, id, 0, if is_moog { 30 } else { 20 })?,
        start_height: parse_height_provider(id, field_of(value, "start_height", STRUCTURE, id)?)?,
        use_expansion_hack: if is_moog {
            value
                .get("use_bounding_box_hack")
                .map(|_| bool_field(value, "use_bounding_box_hack", STRUCTURE, id))
                .transpose()?
                .unwrap_or(false)
        } else {
            bool_field(value, "use_expansion_hack", STRUCTURE, id)?
        },
        project_start_to_heightmap: parse_optional_enum_field(
            value,
            "project_start_to_heightmap",
            STRUCTURE,
            id,
            HEIGHTMAP_TYPES,
        )?,
        max_distance_from_center,
        dimension_padding: parse_dimension_padding(id, value.get("dimension_padding"))?,
        liquid_settings: optional_enum_field(
            value,
            "liquid_settings",
            STRUCTURE,
            id,
            LIQUID_SETTINGS,
            if is_moog { LiquidSettings::IgnoreWaterlogging } else { LiquidSettings::ApplyWaterlogging },
        )?,
    })
}

fn parse_pool_alias(id: &Identifier, value: &Value) -> Result<PoolAliasSpec, VillageDataError> {
    let type_id = parse_id(id, field_str(value, "type", STRUCTURE, id)?)?;
    let weight = |entry: &Value| ranged_i32_field(entry, "weight", STRUCTURE, id, 1, i32::MAX);
    Ok(match type_id.path() {
        "direct" => PoolAliasSpec::Direct {
            alias: parse_id(id, field_str(value, "alias", STRUCTURE, id)?)?,
            target: parse_id(id, field_str(value, "target", STRUCTURE, id)?)?,
        },
        "random" => PoolAliasSpec::Random {
            alias: parse_id(id, field_str(value, "alias", STRUCTURE, id)?)?,
            targets: field_array(value, "targets", STRUCTURE, id)?
                .iter()
                .map(|entry| {
                    Ok((
                        weight(entry)?,
                        parse_id(id, field_str(entry, "data", STRUCTURE, id)?)?,
                    ))
                })
                .collect::<Result<_, VillageDataError>>()?,
        },
        "random_group" => PoolAliasSpec::RandomGroup {
            groups: field_array(value, "groups", STRUCTURE, id)?
                .iter()
                .map(|entry| {
                    Ok((
                        weight(entry)?,
                        field_array(entry, "data", STRUCTURE, id)?
                            .iter()
                            .map(|alias| parse_pool_alias(id, alias))
                            .collect::<Result<_, _>>()?,
                    ))
                })
                .collect::<Result<_, VillageDataError>>()?,
        },
        _ => {
            return Err(VillageDataError::UnsupportedType {
                kind: STRUCTURE,
                type_id,
                referrer: id.clone(),
            });
        }
    })
}

fn parse_biome_set(id: &Identifier, value: &Value) -> Result<BiomeSet, VillageDataError> {
    match value {
        // A `#`-prefixed string is a tag; a bare identifier is a one-entry list.
        Value::String(raw) => match raw.strip_prefix('#') {
            Some(tag) => Ok(BiomeSet::Tag(parse_id(id, tag)?)),
            None => Ok(BiomeSet::Biomes(vec![parse_id(id, raw)?])),
        },
        Value::Array(entries) => {
            let mut biomes = Vec::with_capacity(entries.len());
            for entry in entries {
                let Some(raw) = entry.as_str() else {
                    return Err(VillageDataError::InvalidField {
                        kind: STRUCTURE,
                        entry: id.clone(),
                        field: "biomes",
                        value: entry.to_string(),
                    });
                };
                biomes.push(parse_id(id, raw)?);
            }
            Ok(BiomeSet::Biomes(biomes))
        }
        other => Err(VillageDataError::InvalidField {
            kind: STRUCTURE,
            entry: id.clone(),
            field: "biomes",
            value: other.to_string(),
        }),
    }
}

fn parse_spawn_overrides(
    id: &Identifier,
    value: &Value,
) -> Result<Vec<SpawnOverrideEntry>, VillageDataError> {
    let Some(object) = value.as_object() else {
        return Err(VillageDataError::InvalidField {
            kind: STRUCTURE,
            entry: id.clone(),
            field: "spawn_overrides",
            value: value.to_string(),
        });
    };
    let mut overrides = Vec::with_capacity(object.len());
    for (category, override_value) in object {
        let Some(category) = mob_category(category) else {
            return Err(VillageDataError::InvalidField {
                kind: SPAWN_OVERRIDE,
                entry: id.clone(),
                field: "spawn_overrides",
                value: category.clone(),
            });
        };
        overrides.push(SpawnOverrideEntry {
            category,
            bounding_box: enum_field(
                override_value,
                "bounding_box",
                SPAWN_OVERRIDE,
                id,
                SPAWN_BOUNDING_BOXES,
            )?,
            spawns: parse_weighted_spawns(
                id,
                field_array(override_value, "spawns", SPAWN_OVERRIDE, id)?,
            )?,
        });
    }
    Ok(overrides)
}

fn parse_weighted_spawns(
    id: &Identifier,
    entries: &[Value],
) -> Result<Vec<WeightedSpawn>, VillageDataError> {
    let mut spawns = Vec::with_capacity(entries.len());
    for entry in entries {
        let data = field_of(entry, "data", SPAWN_OVERRIDE, id)?;
        let min_count = ranged_i32_field(data, "minCount", SPAWN_OVERRIDE, id, 1, i32::MAX)?;
        let max_count = ranged_i32_field(data, "maxCount", SPAWN_OVERRIDE, id, 1, i32::MAX)?;
        if min_count > max_count {
            return Err(VillageDataError::InvalidField {
                kind: SPAWN_OVERRIDE,
                entry: id.clone(),
                field: "minCount",
                value: format!("{min_count} is larger than maxCount {max_count}"),
            });
        }
        spawns.push(WeightedSpawn {
            weight: ranged_i32_field(entry, "weight", SPAWN_OVERRIDE, id, 0, i32::MAX)?,
            entity_type: parse_id(id, field_str(data, "type", SPAWN_OVERRIDE, id)?)?,
            min_count,
            max_count,
        });
    }
    Ok(spawns)
}

fn parse_max_distance(id: &Identifier, value: &Value) -> Result<MaxDistance, VillageDataError> {
    // `Codec.either(full, horizontal only)`: one integer means both axes.
    if let Some(horizontal) = value.as_i64().and_then(|raw| i32::try_from(raw).ok()) {
        checked_range(
            id,
            horizontal,
            "max_distance_from_center",
            1,
            MAX_STRUCTURE_RANGE,
        )?;
        return Ok(MaxDistance {
            horizontal,
            vertical: horizontal,
        });
    }
    let horizontal = ranged_i32_field(value, "horizontal", STRUCTURE, id, 1, MAX_STRUCTURE_RANGE)?;
    let vertical = optional_ranged_i32_field(value, "vertical", STRUCTURE, id, 1, Y_SIZE, Y_SIZE)?;
    Ok(MaxDistance {
        horizontal,
        vertical,
    })
}

fn parse_dimension_padding(
    id: &Identifier,
    value: Option<&Value>,
) -> Result<DimensionPadding, VillageDataError> {
    // `Codec.either(non-negative int, {bottom, top})`, both zero by default.
    let Some(value) = value else {
        return Ok(DimensionPadding { bottom: 0, top: 0 });
    };
    if let Some(both) = value.as_i64().and_then(|raw| i32::try_from(raw).ok()) {
        checked_range(id, both, "dimension_padding", 0, i32::MAX)?;
        return Ok(DimensionPadding {
            bottom: both,
            top: both,
        });
    }
    Ok(DimensionPadding {
        bottom: optional_ranged_i32_field(value, "bottom", STRUCTURE, id, 0, i32::MAX, 0)?,
        top: optional_ranged_i32_field(value, "top", STRUCTURE, id, 0, i32::MAX, 0)?,
    })
}

fn parse_height_provider(
    id: &Identifier,
    value: &Value,
) -> Result<HeightProviderSpec, VillageDataError> {
    // `HeightProvider.CODEC` is an either: a bare `VerticalAnchor` is the
    // constant provider, anything else dispatches on `type`.
    let Some(object) = value.as_object() else {
        return Err(VillageDataError::InvalidField {
            kind: HEIGHT_PROVIDER,
            entry: id.clone(),
            field: "start_height",
            value: value.to_string(),
        });
    };
    if !object.contains_key("type") {
        return Ok(HeightProviderSpec::Constant(parse_vertical_anchor(
            id,
            "start_height",
            value,
        )?));
    }
    let type_id = parse_id(id, field_str(value, "type", HEIGHT_PROVIDER, id)?)?;
    Ok(match type_id.path() {
        "constant" => HeightProviderSpec::Constant(parse_vertical_anchor(
            id,
            "start_height",
            field_of(value, "value", HEIGHT_PROVIDER, id)?,
        )?),
        "uniform" => HeightProviderSpec::Uniform {
            min_inclusive: parse_vertical_anchor(
                id,
                "start_height",
                field_of(value, "min_inclusive", HEIGHT_PROVIDER, id)?,
            )?,
            max_inclusive: parse_vertical_anchor(
                id,
                "start_height",
                field_of(value, "max_inclusive", HEIGHT_PROVIDER, id)?,
            )?,
        },
        "trapezoid" => HeightProviderSpec::Trapezoid {
            min_inclusive: parse_vertical_anchor(
                id,
                "start_height",
                field_of(value, "min_inclusive", HEIGHT_PROVIDER, id)?,
            )?,
            max_inclusive: parse_vertical_anchor(
                id,
                "start_height",
                field_of(value, "max_inclusive", HEIGHT_PROVIDER, id)?,
            )?,
            plateau: optional_i32_field(value, "plateau", HEIGHT_PROVIDER, id, 0)?,
        },
        _ => {
            return Err(VillageDataError::UnsupportedType {
                kind: HEIGHT_PROVIDER,
                type_id,
                referrer: id.clone(),
            });
        }
    })
}

fn parse_vertical_anchor(
    id: &Identifier,
    field: &'static str,
    value: &Value,
) -> Result<VerticalAnchor, VillageDataError> {
    let invalid = || VillageDataError::InvalidField {
        kind: HEIGHT_PROVIDER,
        entry: id.clone(),
        field,
        value: value.to_string(),
    };
    // `Codec.xor` of the three anchor codecs: exactly one key, in range.
    let Some(object) = value.as_object() else {
        return Err(invalid());
    };
    let mut keys = object.iter();
    let Some((name, raw)) = keys.next() else {
        return Err(invalid());
    };
    if keys.next().is_some() {
        return Err(invalid());
    }
    let Some(offset) = raw.as_i64().and_then(|raw| i32::try_from(raw).ok()) else {
        return Err(invalid());
    };
    if !(MIN_Y..=MAX_Y).contains(&offset) {
        return Err(invalid());
    }
    match name.as_str() {
        "absolute" => Ok(VerticalAnchor::Absolute(offset)),
        "above_bottom" => Ok(VerticalAnchor::AboveBottom(offset)),
        "below_top" => Ok(VerticalAnchor::BelowTop(offset)),
        _ => Err(invalid()),
    }
}

fn parse_template_pool(
    id: &Identifier,
    value: &Value,
) -> Result<TemplatePoolSpec, VillageDataError> {
    let fallback = parse_id(id, field_str(value, "fallback", TEMPLATE_POOL, id)?)?;
    let entries = field_array(value, "elements", TEMPLATE_POOL, id)?;
    let mut elements = Vec::with_capacity(entries.len());
    for entry in entries {
        elements.push(PoolElementEntry {
            // `Codec.intRange(1, 150)`
            weight: ranged_i32_field(entry, "weight", TEMPLATE_POOL, id, 1, 150)?,
            element: parse_pool_element(id, field_of(entry, "element", TEMPLATE_POOL, id)?)?,
        });
    }
    Ok(TemplatePoolSpec {
        id: id.clone(),
        fallback,
        elements,
    })
}

fn parse_pool_element(
    referrer: &Identifier,
    value: &Value,
) -> Result<PoolElementSpec, VillageDataError> {
    let type_id = parse_id(
        referrer,
        field_str(value, "element_type", POOL_ELEMENT, referrer)?,
    )?;
    if type_id.as_str() == "moogs_structures:versioned_single_pool_element" {
        let mut single = parse_single_element(referrer, value)?;
        let fallback = field_str(value, "location", POOL_ELEMENT, referrer)?;
        let release = crate::protocol_versions::SUPPORTED
            .last()
            .expect("supported protocol window is nonempty")
            .release;
        let location = crate::versioned_structure_pool::select(
            value.get("locations"),
            fallback,
            release,
        )
        .map_err(|()| VillageDataError::InvalidField {
            kind: POOL_ELEMENT,
            entry: referrer.clone(),
            field: "locations",
            value: value.get("locations").map_or_else(String::new, Value::to_string),
        })?;
        single.location = parse_id(referrer, location)?;
        return Ok(PoolElementSpec::Single(single));
    }
    Ok(match type_id.path() {
        // A unit codec: the element carries no fields of its own.
        "empty_pool_element" => PoolElementSpec::Empty,
        "single_pool_element" | "legacy_single_pool_element" => {
            let single = parse_single_element(referrer, value)?;
            if type_id.path() == "single_pool_element" {
                PoolElementSpec::Single(single)
            } else {
                PoolElementSpec::LegacySingle(single)
            }
        }
        "list_pool_element" => {
            let elements = field_array(value, "elements", POOL_ELEMENT, referrer)?;
            if elements.is_empty() {
                // Vanilla refuses an empty list element outright.
                return Err(VillageDataError::InvalidField {
                    kind: POOL_ELEMENT,
                    entry: referrer.clone(),
                    field: "elements",
                    value: "empty list pool element".to_owned(),
                });
            }
            let mut parsed = Vec::with_capacity(elements.len());
            for element in elements {
                parsed.push(parse_pool_element(referrer, element)?);
            }
            PoolElementSpec::List {
                elements: parsed,
                projection: enum_field(value, "projection", POOL_ELEMENT, referrer, PROJECTIONS)?,
            }
        }
        "feature_pool_element" => PoolElementSpec::Feature {
            feature: parse_id(
                referrer,
                field_str(value, "feature", POOL_ELEMENT, referrer)?,
            )?,
            projection: enum_field(value, "projection", POOL_ELEMENT, referrer, PROJECTIONS)?,
        },
        _ => {
            return Err(VillageDataError::UnsupportedType {
                kind: POOL_ELEMENT,
                type_id,
                referrer: referrer.clone(),
            });
        }
    })
}

fn parse_single_element(
    referrer: &Identifier,
    value: &Value,
) -> Result<SingleElementSpec, VillageDataError> {
    Ok(SingleElementSpec {
        location: parse_id(
            referrer,
            field_str(value, "location", POOL_ELEMENT, referrer)?,
        )?,
        projection: enum_field(value, "projection", POOL_ELEMENT, referrer, PROJECTIONS)?,
        processors: parse_processor_ref(
            referrer,
            field_of(value, "processors", POOL_ELEMENT, referrer)?,
        )?,
        override_liquid_settings: parse_optional_enum_field(
            value,
            "override_liquid_settings",
            POOL_ELEMENT,
            referrer,
            LIQUID_SETTINGS,
        )?,
    })
}

fn parse_processor_ref(
    referrer: &Identifier,
    value: &Value,
) -> Result<ProcessorRef, VillageDataError> {
    // `StructureProcessorType.LIST_CODEC`: a list reference, a bare processor
    // array, or `{"processors": [...]}`.
    match value {
        Value::String(raw) => Ok(ProcessorRef::List(parse_id(referrer, raw)?)),
        Value::Array(entries) => Ok(ProcessorRef::Inline(parse_processors(referrer, entries)?)),
        Value::Object(object) if object.contains_key("processors") => {
            Ok(ProcessorRef::Inline(parse_processors(
                referrer,
                field_array(value, "processors", PROCESSOR_LIST, referrer)?,
            )?))
        }
        other => Err(VillageDataError::InvalidField {
            kind: PROCESSOR_LIST,
            entry: referrer.clone(),
            field: "processors",
            value: other.to_string(),
        }),
    }
}

fn parse_processors(
    referrer: &Identifier,
    entries: &[Value],
) -> Result<Vec<StructureProcessorSpec>, VillageDataError> {
    let mut processors = Vec::with_capacity(entries.len());
    for entry in entries {
        processors.push(parse_processor(referrer, entry)?);
    }
    Ok(processors)
}

fn parse_processor_list(
    id: &Identifier,
    value: &Value,
) -> Result<ProcessorListSpec, VillageDataError> {
    // A list file holds either `{"processors": [...]}` or a bare array.
    let entries = match value {
        Value::Array(entries) => entries,
        Value::Object(_) => field_array(value, "processors", PROCESSOR_LIST, id)?,
        other => {
            return Err(VillageDataError::InvalidField {
                kind: PROCESSOR_LIST,
                entry: id.clone(),
                field: "processors",
                value: other.to_string(),
            });
        }
    };
    Ok(ProcessorListSpec {
        id: id.clone(),
        processors: parse_processors(id, entries)?,
    })
}

fn parse_moog_spawner(
    referrer: &Identifier,
    value: &Value,
) -> Result<MoogSpawnerSpec, VillageDataError> {
    let invalid = |field, detail: String| VillageDataError::InvalidField {
        kind: PROCESSOR,
        entry: referrer.clone(),
        field,
        value: detail,
    };
    let allowed = [
        "processor_type", "weighted_entities", "delay", "min_spawn_delay",
        "max_spawn_delay", "spawn_count", "max_nearby_entities",
        "required_player_range", "spawn_range",
    ];
    if let Some(object) = value.as_object() {
        for key in object.keys() {
            if !allowed.contains(&key.as_str()) {
                return Err(invalid("processor_type", format!("unsupported Moog spawner field {key}")));
            }
        }
    }
    let entries = field_array(value, "weighted_entities", PROCESSOR, referrer)?;
    let mut mobs = Vec::with_capacity(entries.len());
    let mut total = 0.0;
    for entry in entries {
        let object = entry.as_object().ok_or_else(|| invalid(
            "weighted_entities", "expected an entity object".to_owned(),
        ))?;
        if object.keys().any(|key| key != "entity" && key != "weight") {
            return Err(invalid("weighted_entities", "unsupported entity fields".to_owned()));
        }
        let entity = parse_id(referrer, field_str(entry, "entity", PROCESSOR, referrer)?)?;
        let weight = entry.get("weight").and_then(Value::as_f64)
            .filter(|weight| weight.is_finite() && *weight > 0.0)
            .ok_or_else(|| invalid("weighted_entities", "weight must be positive and finite".to_owned()))?;
        total += weight;
        mobs.push((entity, weight));
    }
    if mobs.is_empty() || !total.is_finite() {
        return Err(invalid("weighted_entities", "empty or overflowing mob weights".to_owned()));
    }
    let mut fields = [0; 7];
    for (slot, (key, default)) in fields.iter_mut().zip([
        ("delay", 20), ("min_spawn_delay", 200), ("max_spawn_delay", 800),
        ("spawn_count", 4), ("max_nearby_entities", 6),
        ("required_player_range", 16), ("spawn_range", 4),
    ]) {
        let raw = optional_i32_field(value, key, PROCESSOR, referrer, default)?;
        *slot = i16::try_from(raw).map_err(|_| invalid(key, raw.to_string()))?;
    }
    Ok(MoogSpawnerSpec { mobs, fields })
}

fn parse_processor(
    referrer: &Identifier,
    value: &Value,
) -> Result<StructureProcessorSpec, VillageDataError> {
    let type_id = parse_id(
        referrer,
        field_str(value, "processor_type", PROCESSOR, referrer)?,
    )?;
    if type_id.as_str() == "moogs_structures:spawner_randomizing_processor" {
        return parse_moog_spawner(referrer, value).map(StructureProcessorSpec::MoogSpawner);
    }
    Ok(match type_id.path() {
        "block_rot" => StructureProcessorSpec::BlockRot {
            integrity: optional_ranged_f32_field(
                value,
                "integrity",
                PROCESSOR,
                referrer,
                0.0,
                1.0,
                1.0,
            )?,
            rottable_blocks: value
                .get("rottable_blocks")
                .map(|tag| parse_hashed_tag(referrer, tag))
                .transpose()?,
        },
        "capped" => {
            let delegate = parse_processor(
                referrer,
                field_of(value, "delegate", PROCESSOR, referrer)?,
            )?;
            if matches!(delegate, StructureProcessorSpec::MoogSpawner(_)) {
                return Err(VillageDataError::InvalidField {
                    kind: PROCESSOR,
                    entry: referrer.clone(),
                    field: "delegate",
                    value: "capped Moog spawner processors are unsupported".to_owned(),
                });
            }
            StructureProcessorSpec::Capped {
                delegate: Box::new(delegate),
                limit: ranged_i32_field(value, "limit", PROCESSOR, referrer, 0, i32::MAX)?,
            }
        }
        "rule" => {
            let entries = field_array(value, "rules", PROCESSOR, referrer)?;
            let mut rules = Vec::with_capacity(entries.len());
            for entry in entries {
                rules.push(parse_processor_rule(referrer, entry)?);
            }
            StructureProcessorSpec::Rule { rules }
        }
        "block_age" => StructureProcessorSpec::BlockAge {
            mossiness: f32_field(value, "mossiness", PROCESSOR, referrer)?,
        },
        "gravity" => StructureProcessorSpec::Gravity {
            // `orElse` on both fields: `world_surface_wg` and 0.
            heightmap: optional_enum_field(
                value,
                "heightmap",
                PROCESSOR,
                referrer,
                HEIGHTMAP_TYPES,
                HeightmapType::WorldSurfaceWg,
            )?,
            offset: optional_i32_field(value, "offset", PROCESSOR, referrer, 0)?,
        },
        "jigsaw_replacement" => StructureProcessorSpec::JigsawReplacement,
        "protected_blocks" => StructureProcessorSpec::ProtectedBlocks {
            cannot_replace: parse_hashed_tag(
                referrer,
                field_of(value, "value", PROCESSOR, referrer)?,
            )?,
        },
        _ => {
            return Err(VillageDataError::UnsupportedType {
                kind: PROCESSOR,
                type_id,
                referrer: referrer.clone(),
            });
        }
    })
}

fn parse_processor_rule(
    referrer: &Identifier,
    value: &Value,
) -> Result<ProcessorRuleSpec, VillageDataError> {
    let mut append_loot = None;
    if let Some(modifier) = value.get("block_entity_modifier") {
        let type_id = parse_id(referrer, field_str(modifier, "type", PROCESSOR, referrer)?)?;
        if type_id.path() == "append_loot" {
            append_loot = Some(parse_id(
                referrer,
                field_str(modifier, "loot_table", PROCESSOR, referrer)?,
            )?);
        } else if type_id.path() != "passthrough" {
            return Err(VillageDataError::UnsupportedType {
                kind: PROCESSOR,
                type_id,
                referrer: referrer.clone(),
            });
        }
    }
    Ok(ProcessorRuleSpec {
        append_loot,
        input_predicate: parse_rule_test(
            referrer,
            field_of(value, "input_predicate", PROCESSOR, referrer)?,
        )?,
        location_predicate: parse_rule_test(
            referrer,
            field_of(value, "location_predicate", PROCESSOR, referrer)?,
        )?,
        position_predicate: parse_pos_rule_test(value.get("position_predicate"), referrer)?,
        output_state: parse_block_state(
            referrer,
            field_of(value, "output_state", PROCESSOR, referrer)?,
        )?,
    })
}

fn parse_rule_test(referrer: &Identifier, value: &Value) -> Result<RuleTestSpec, VillageDataError> {
    let type_id = parse_id(
        referrer,
        field_str(value, "predicate_type", RULE_TEST, referrer)?,
    )?;
    Ok(match type_id.path() {
        "always_true" => RuleTestSpec::AlwaysTrue,
        "block_match" => RuleTestSpec::BlockMatch {
            block: parse_id(referrer, field_str(value, "block", RULE_TEST, referrer)?)?,
        },
        "blockstate_match" => RuleTestSpec::BlockStateMatch {
            state: parse_block_state(
                referrer,
                field_of(value, "block_state", RULE_TEST, referrer)?,
            )?,
        },
        "tag_match" => RuleTestSpec::TagMatch {
            tag: parse_id(referrer, field_str(value, "tag", RULE_TEST, referrer)?)?,
        },
        "random_block_match" => RuleTestSpec::RandomBlockMatch {
            block: parse_id(referrer, field_str(value, "block", RULE_TEST, referrer)?)?,
            probability: f32_field(value, "probability", RULE_TEST, referrer)?,
        },
        "random_blockstate_match" => RuleTestSpec::RandomBlockStateMatch {
            state: parse_block_state(
                referrer,
                field_of(value, "block_state", RULE_TEST, referrer)?,
            )?,
            probability: f32_field(value, "probability", RULE_TEST, referrer)?,
        },
        _ => {
            return Err(VillageDataError::UnsupportedType {
                kind: RULE_TEST,
                type_id,
                referrer: referrer.clone(),
            });
        }
    })
}

fn parse_pos_rule_test(
    value: Option<&Value>,
    referrer: &Identifier,
) -> Result<PosRuleTestSpec, VillageDataError> {
    // `PosRuleTest.CODEC`, `always_true` by default.
    let Some(value) = value else {
        return Ok(PosRuleTestSpec::AlwaysTrue);
    };
    let type_id = parse_id(
        referrer,
        field_str(value, "predicate_type", POS_RULE_TEST, referrer)?,
    )?;
    let chances = |value: &Value| -> Result<(f32, f32, i32, i32), VillageDataError> {
        Ok((
            optional_f32_field(value, "min_chance", POS_RULE_TEST, referrer, 0.0)?,
            optional_f32_field(value, "max_chance", POS_RULE_TEST, referrer, 0.0)?,
            optional_i32_field(value, "min_dist", POS_RULE_TEST, referrer, 0)?,
            optional_i32_field(value, "max_dist", POS_RULE_TEST, referrer, 0)?,
        ))
    };
    Ok(match type_id.path() {
        "always_true" => PosRuleTestSpec::AlwaysTrue,
        "linear_pos" => {
            let (min_chance, max_chance, min_dist, max_dist) = chances(value)?;
            PosRuleTestSpec::LinearPos {
                min_chance,
                max_chance,
                min_dist,
                max_dist,
            }
        }
        "axis_aligned_linear_pos" => {
            let (min_chance, max_chance, min_dist, max_dist) = chances(value)?;
            PosRuleTestSpec::AxisAlignedLinearPos {
                min_chance,
                max_chance,
                min_dist,
                max_dist,
                axis: optional_enum_field(value, "axis", POS_RULE_TEST, referrer, AXES, Axis::Y)?,
            }
        }
        _ => {
            return Err(VillageDataError::UnsupportedType {
                kind: POS_RULE_TEST,
                type_id,
                referrer: referrer.clone(),
            });
        }
    })
}

/// `{"Name": ..., "Properties": {...}}`, the JSON form of a block state.
fn parse_block_state(
    referrer: &Identifier,
    value: &Value,
) -> Result<BlockStateSpec, VillageDataError> {
    let name = field_str(value, "Name", BLOCK_STATE, referrer)?;
    let mut properties = Vec::new();
    if let Some(values) = value.get("Properties") {
        let Some(values) = values.as_object() else {
            return Err(VillageDataError::InvalidField {
                kind: BLOCK_STATE,
                entry: referrer.clone(),
                field: "Properties",
                value: values.to_string(),
            });
        };
        for (key, value) in values {
            let Some(value) = value.as_str() else {
                return Err(VillageDataError::InvalidField {
                    kind: BLOCK_STATE,
                    entry: referrer.clone(),
                    field: "Properties",
                    value: format!("{key}={value}"),
                });
            };
            properties.push((key.clone(), value.to_owned()));
        }
    }
    Ok(BlockStateSpec {
        block: parse_id(referrer, name)?,
        properties,
    })
}

/// `TagKey.hashedCodec`: the stored form carries the `#`.
fn parse_hashed_tag(referrer: &Identifier, value: &Value) -> Result<Identifier, VillageDataError> {
    let Some(raw) = value.as_str() else {
        return Err(VillageDataError::InvalidField {
            kind: PROCESSOR,
            entry: referrer.clone(),
            field: "value",
            value: value.to_string(),
        });
    };
    let Some(tag) = raw.strip_prefix('#') else {
        return Err(VillageDataError::InvalidField {
            kind: PROCESSOR,
            entry: referrer.clone(),
            field: "value",
            value: raw.to_owned(),
        });
    };
    parse_id(referrer, tag)
}

const RANDOM_SPREAD_TYPES: &[(&str, RandomSpreadType)] = &[
    ("linear", RandomSpreadType::Linear),
    ("triangular", RandomSpreadType::Triangular),
];

const FREQUENCY_REDUCTION_METHODS: &[(&str, FrequencyReductionMethod)] = &[
    ("default", FrequencyReductionMethod::Default),
    ("legacy_type_1", FrequencyReductionMethod::LegacyType1),
    ("legacy_type_2", FrequencyReductionMethod::LegacyType2),
    ("legacy_type_3", FrequencyReductionMethod::LegacyType3),
];

const TERRAIN_ADAPTATIONS: &[(&str, TerrainAdaptation)] = &[
    ("none", TerrainAdaptation::None),
    ("bury", TerrainAdaptation::Bury),
    ("beard_thin", TerrainAdaptation::BeardThin),
    ("beard_box", TerrainAdaptation::BeardBox),
    ("encapsulate", TerrainAdaptation::Encapsulate),
];

const STRUCTURE_STEPS: &[(&str, StructureStep)] = &[
    ("raw_generation", StructureStep::RawGeneration),
    ("lakes", StructureStep::Lakes),
    ("local_modifications", StructureStep::LocalModifications),
    (
        "underground_structures",
        StructureStep::UndergroundStructures,
    ),
    ("surface_structures", StructureStep::SurfaceStructures),
    ("strongholds", StructureStep::Strongholds),
    ("underground_ores", StructureStep::UndergroundOres),
    (
        "underground_decoration",
        StructureStep::UndergroundDecoration,
    ),
    ("fluid_springs", StructureStep::FluidSprings),
    ("vegetal_decoration", StructureStep::VegetalDecoration),
    (
        "top_layer_modification",
        StructureStep::TopLayerModification,
    ),
];

const HEIGHTMAP_TYPES: &[(&str, HeightmapType)] = &[
    ("WORLD_SURFACE_WG", HeightmapType::WorldSurfaceWg),
    ("WORLD_SURFACE", HeightmapType::WorldSurface),
    ("OCEAN_FLOOR_WG", HeightmapType::OceanFloorWg),
    ("OCEAN_FLOOR", HeightmapType::OceanFloor),
    ("MOTION_BLOCKING", HeightmapType::MotionBlocking),
    (
        "MOTION_BLOCKING_NO_LEAVES",
        HeightmapType::MotionBlockingNoLeaves,
    ),
];

const LIQUID_SETTINGS: &[(&str, LiquidSettings)] = &[
    ("ignore_waterlogging", LiquidSettings::IgnoreWaterlogging),
    ("apply_waterlogging", LiquidSettings::ApplyWaterlogging),
];

const SPAWN_BOUNDING_BOXES: &[(&str, SpawnBoundingBox)] = &[
    ("piece", SpawnBoundingBox::Piece),
    ("full", SpawnBoundingBox::Full),
];

const PROJECTIONS: &[(&str, Projection)] = &[
    ("rigid", Projection::Rigid),
    ("terrain_matching", Projection::TerrainMatching),
];

const AXES: &[(&str, Axis)] = &[("x", Axis::X), ("y", Axis::Y), ("z", Axis::Z)];

fn mob_category(name: &str) -> Option<MobCategory> {
    Some(match name {
        "monster" => MobCategory::Monster,
        "creature" => MobCategory::Creature,
        "ambient" => MobCategory::Ambient,
        "axolotls" => MobCategory::Axolotls,
        "underground_water_creature" => MobCategory::UndergroundWaterCreature,
        "water_creature" => MobCategory::WaterCreature,
        "water_ambient" => MobCategory::WaterAmbient,
        "misc" => MobCategory::Misc,
        _ => return None,
    })
}

fn parse_id(referrer: &Identifier, value: &str) -> Result<Identifier, VillageDataError> {
    Identifier::parse(value.to_owned()).map_err(|_| VillageDataError::InvalidIdentifier {
        referrer: referrer.clone(),
        value: value.to_owned(),
    })
}

fn parse_optional_id(
    value: &Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
) -> Result<Option<Identifier>, VillageDataError> {
    match value.get(field) {
        None => Ok(None),
        Some(raw) => Ok(Some(parse_id(
            entry,
            raw.as_str().ok_or_else(|| VillageDataError::InvalidField {
                kind,
                entry: entry.clone(),
                field,
                value: raw.to_string(),
            })?,
        )?)),
    }
}

fn field_of<'a>(
    value: &'a Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
) -> Result<&'a Value, VillageDataError> {
    value
        .get(field)
        .ok_or_else(|| VillageDataError::MissingField {
            kind,
            entry: entry.clone(),
            field,
        })
}

fn field_array<'a>(
    value: &'a Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
) -> Result<&'a Vec<Value>, VillageDataError> {
    field_of(value, field, kind, entry)?
        .as_array()
        .ok_or_else(|| VillageDataError::InvalidField {
            kind,
            entry: entry.clone(),
            field,
            value: "not an array".to_owned(),
        })
}

fn field_str<'a>(
    value: &'a Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
) -> Result<&'a str, VillageDataError> {
    field_of(value, field, kind, entry)?
        .as_str()
        .ok_or_else(|| VillageDataError::InvalidField {
            kind,
            entry: entry.clone(),
            field,
            value: "not a string".to_owned(),
        })
}

fn bool_field(
    value: &Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
) -> Result<bool, VillageDataError> {
    field_of(value, field, kind, entry)?
        .as_bool()
        .ok_or_else(|| VillageDataError::InvalidField {
            kind,
            entry: entry.clone(),
            field,
            value: "not a boolean".to_owned(),
        })
}

fn i32_field(
    value: &Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
) -> Result<i32, VillageDataError> {
    let raw = field_of(value, field, kind, entry)?
        .as_i64()
        .ok_or_else(|| VillageDataError::InvalidField {
            kind,
            entry: entry.clone(),
            field,
            value: "not an integer".to_owned(),
        })?;
    i32::try_from(raw).map_err(|_| VillageDataError::InvalidField {
        kind,
        entry: entry.clone(),
        field,
        value: raw.to_string(),
    })
}

fn f32_field(
    value: &Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
) -> Result<f32, VillageDataError> {
    let raw = field_of(value, field, kind, entry)?
        .as_f64()
        .ok_or_else(|| VillageDataError::InvalidField {
            kind,
            entry: entry.clone(),
            field,
            value: "not a number".to_owned(),
        })?;
    Ok(raw as f32)
}

fn optional_i32_field(
    value: &Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
    default: i32,
) -> Result<i32, VillageDataError> {
    match value.get(field) {
        None => Ok(default),
        Some(_) => i32_field(value, field, kind, entry),
    }
}

fn optional_f32_field(
    value: &Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
    default: f32,
) -> Result<f32, VillageDataError> {
    match value.get(field) {
        None => Ok(default),
        Some(_) => f32_field(value, field, kind, entry),
    }
}

fn enum_field<T: Copy>(
    value: &Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
    table: &[(&str, T)],
) -> Result<T, VillageDataError> {
    let raw = field_str(value, field, kind, entry)?;
    table
        .iter()
        .find_map(|(name, parsed)| (*name == raw).then_some(*parsed))
        .ok_or_else(|| VillageDataError::InvalidField {
            kind,
            entry: entry.clone(),
            field,
            value: raw.to_owned(),
        })
}

fn optional_enum_field<T: Copy>(
    value: &Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
    table: &[(&str, T)],
    default: T,
) -> Result<T, VillageDataError> {
    match value.get(field) {
        None => Ok(default),
        Some(_) => enum_field(value, field, kind, entry, table),
    }
}

fn parse_optional_enum_field<T: Copy>(
    value: &Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
    table: &[(&str, T)],
) -> Result<Option<T>, VillageDataError> {
    match value.get(field) {
        None => Ok(None),
        Some(_) => Ok(Some(enum_field(value, field, kind, entry, table)?)),
    }
}

fn checked_range(
    entry: &Identifier,
    parsed: i32,
    field: &'static str,
    min: i32,
    max: i32,
) -> Result<i32, VillageDataError> {
    if (min..=max).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(VillageDataError::InvalidField {
            kind: STRUCTURE,
            entry: entry.clone(),
            field,
            value: format!("{parsed} is outside {min}..={max}"),
        })
    }
}

fn ranged_i32_field(
    value: &Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
    min: i32,
    max: i32,
) -> Result<i32, VillageDataError> {
    let parsed = i32_field(value, field, kind, entry)?;
    if (min..=max).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(VillageDataError::InvalidField {
            kind,
            entry: entry.clone(),
            field,
            value: format!("{parsed} is outside {min}..={max}"),
        })
    }
}

fn optional_ranged_i32_field(
    value: &Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
    min: i32,
    max: i32,
    default: i32,
) -> Result<i32, VillageDataError> {
    match value.get(field) {
        None => Ok(default),
        Some(_) => ranged_i32_field(value, field, kind, entry, min, max),
    }
}

fn optional_ranged_f32_field(
    value: &Value,
    field: &'static str,
    kind: EntryKind,
    entry: &Identifier,
    min: f32,
    max: f32,
    default: f32,
) -> Result<f32, VillageDataError> {
    if value.get(field).is_none() {
        return Ok(default);
    }
    let parsed = f32_field(value, field, kind, entry)?;
    if (min..=max).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(VillageDataError::InvalidField {
            kind,
            entry: entry.clone(),
            field,
            value: format!("{parsed} is outside {min}..={max}"),
        })
    }
}

#[cfg(test)]
#[path = "village_data_tests.rs"]
mod tests;
