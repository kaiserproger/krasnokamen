//! Construction, rule and plan wirers, and the LOD/GPU entry points.

use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum WorldgenMode {
    #[default]
    VanillaLike,
    TellusLike,
}

impl WorldgenMode {
    #[must_use]
    pub const fn contract_name(self) -> &'static str {
        match self {
            Self::VanillaLike => "vanilla_like",
            Self::TellusLike => "tellus_like",
        }
    }
}

impl TerrainGenerator {
    /// Build a generator from a seed plus a block registry.
    ///
    /// # Panics
    ///
    /// Panics if the registry is missing required vanilla terrain blocks. Use
    /// [`TerrainGenerator::try_with_biome_rules`] for fallible startup validation.
    #[must_use]
    pub fn new(seed: i64, registry: Arc<BlockRegistry>) -> Self {
        Self::try_with_biome_rules(seed, registry, BiomeRules::vanilla_overworld())
            .unwrap_or_else(|err| panic!("{err}"))
    }

    pub fn try_with_biome_rules(
        seed: i64,
        registry: Arc<BlockRegistry>,
        biomes: BiomeRules,
    ) -> Result<Self, TerrainGeneratorError> {
        let stone = try_resolve_block(registry.as_ref(), "minecraft:stone")?;
        let ores = OreRules::krasnokamen_default(registry.as_ref(), &biomes, stone);
        Self::try_with_rules(seed, registry, biomes, ores)
    }

    /// Compile the local vanilla router and expanded overworld biome preset.
    /// Missing or invalid vanilla data is a startup error.
    pub fn load_vanilla_worldgen(&mut self, data_root: &std::path::Path) -> Result<(), String> {
        let climate = VanillaClimate::new(self.seed as u64, data_root)?;
        let density = VanillaDensity::from_runtime(climate.runtime())?;
        density.validate_aquifer_fields()?;
        let structures = crate::overworld_structures::OverworldStructures::load(
            data_root,
            self.seed,
            &self.blocks,
        )?;
        self.vanilla_worldgen = Some((climate, density));
        self.overworld_structures = Some(structures);
        self.diagnostic_cache = DiagnosticCache::default();
        Ok(())
    }

    pub(super) fn vanilla(&self) -> Option<&(VanillaClimate, VanillaDensity)> {
        self.vanilla_worldgen.as_ref()
    }

    /// The unzoomed noise biome used by chunk palettes and structure gates.
    #[must_use]
    pub fn noise_biome(&self, world_x: i32, y: i32, world_z: i32) -> Identifier {
        if let Some((climate, _)) = self.vanilla() {
            return Identifier::parse(climate.sample(world_x, y, world_z).biome)
                .expect("validated vanilla biome identifier");
        }
        self.base_biome(world_x, world_z)
    }
    /// Build a generator with explicit biome and ore rules.
    ///
    /// # Panics
    ///
    /// Panics if the registry is missing required vanilla terrain blocks. Use
    /// [`TerrainGenerator::try_with_rules`] for fallible startup validation.
    #[must_use]
    pub fn with_rules(
        seed: i64,
        registry: Arc<BlockRegistry>,
        biomes: BiomeRules,
        ores: OreRules,
    ) -> Self {
        Self::try_with_rules(seed, registry, biomes, ores).unwrap_or_else(|err| panic!("{err}"))
    }

    /// Build a generator with explicit biome and ore rules, returning startup
    /// validation errors instead of panicking.
    ///
    /// Required terrain blocks fail construction when absent. Optional surface,
    /// fluid, and decoration blocks keep Krasnokamen' existing fallback behavior so
    /// reduced test registries can still exercise generation.
    ///
    /// # Errors
    ///
    /// Returns [`TerrainGeneratorError::MissingRequiredBlock`] when the block
    /// registry is missing `minecraft:air`, `minecraft:bedrock`,
    /// `minecraft:stone`, `minecraft:dirt`, `minecraft:grass_block`, or
    /// `minecraft:iron_ore`.
    pub fn try_with_rules(
        seed: i64,
        registry: Arc<BlockRegistry>,
        biomes: BiomeRules,
        ores: OreRules,
    ) -> Result<Self, TerrainGeneratorError> {
        let air = try_resolve_block(registry.as_ref(), "minecraft:air")?;
        let stone = try_resolve_block(registry.as_ref(), "minecraft:stone")?;
        Ok(Self {
            seed,
            geometry: OVERWORLD_GEOMETRY,
            air,
            bedrock: try_resolve_block(registry.as_ref(), "minecraft:bedrock")?,
            stone,
            dirt: try_resolve_block(registry.as_ref(), "minecraft:dirt")?,
            grass_block: try_resolve_block(registry.as_ref(), "minecraft:grass_block")?,
            sand: resolve_block_or(registry.as_ref(), "minecraft:sand", stone),
            red_sand: resolve_block_or(registry.as_ref(), "minecraft:red_sand", stone),
            gravel: resolve_block_or(registry.as_ref(), "minecraft:gravel", stone),
            podzol: resolve_block_or(registry.as_ref(), "minecraft:podzol", stone),
            snow_block: resolve_block_or(registry.as_ref(), "minecraft:snow_block", stone),
            deepslate: resolve_block_or(registry.as_ref(), "minecraft:deepslate", stone),
            water: resolve_block_or(registry.as_ref(), "minecraft:water", air),
            cave_lava: resolve_block_or(registry.as_ref(), "minecraft:lava", air),
            terracotta_layers: [
                "minecraft:terracotta",
                "minecraft:orange_terracotta",
                "minecraft:terracotta",
                "minecraft:yellow_terracotta",
                "minecraft:brown_terracotta",
                "minecraft:terracotta",
                "minecraft:red_terracotta",
                "minecraft:white_terracotta",
            ]
            .map(|name| resolve_block_or(registry.as_ref(), name, stone)),
            biomes,
            ores,
            geological_ores: None,
            ore_generation_profile: "vanilla",
            village_plans: None,
            overworld_structures: None,
            jigsaw_sources: Vec::new(),
            decorations: DecorationBlocks::new(registry.as_ref()),
            tree_rules: HashMap::new(),
            clay_rule: ClayRule::default(),
            vanilla_worldgen: None,
            diagnostic_cache: DiagnosticCache::default(),
            blocks: registry,
            #[cfg(feature = "gpu")]
            gpu: None,
            solid_cache: solid_cache::SolidCache::default(),
        })
    }

    /// Wire the village plan source: vanilla villages assembled by the jigsaw
    /// solver, placed through [`crate::village::piece`], with their
    /// `terrain_adaptation = beard_thin` applied as [`crate::village::beard`]'s
    /// column-height analogue.
    ///
    /// Until this is called the generator places no villages, and every path
    /// stays byte-identical to a build without village generation.
    #[must_use]
    pub fn with_village_plans(mut self, source: Arc<VillagePlanSource>) -> Self {
        self.village_plans = Some(source);
        self
    }

    #[must_use]
    pub fn with_jigsaw_source(mut self, source: Arc<VillagePlanSource>) -> Self {
        self.jigsaw_sources.push(source);
        self
    }

    /// The village plan source, when the generator was given one.
    #[must_use]
    pub fn village_plan_source(&self) -> Option<&Arc<VillagePlanSource>> {
        self.village_plans.as_ref()
    }

    /// Explicit standalone vanilla plugin deposit profile. Native Earth never
    /// calls this path: its ores belong to the immutable geological base.
    pub fn try_with_realistic_deposits(
        mut self,
        registry: &BlockRegistry,
    ) -> Result<Self, TerrainGeneratorError> {
        self.ores = OreRules::new(Vec::new()).expect("empty ore rules are valid");
        self.geological_ores = Some(GeologicalOreRules::try_new(registry)?);
        self.ore_generation_profile = "realistic_deposits";
        Ok(self)
    }

    #[must_use]
    pub fn with_realistic_deposits(self, registry: &BlockRegistry) -> Self {
        self.try_with_realistic_deposits(registry)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    #[must_use]
    pub const fn ore_generation_profile(&self) -> &'static str {
        self.ore_generation_profile
    }

    #[must_use]
    pub fn with_geometry(mut self, geometry: ChunkGeometry) -> Self {
        self.geometry = geometry;
        self.diagnostic_cache = DiagnosticCache::default();
        self
    }

    /// Far-terrain LOD summary of a never-generated column: the router's
    /// cave-free surface on the column's lattice corner and the biome there.
    /// Villages, carvers and decoration are not applied; nothing is cached.
    #[must_use]
    pub fn lod_column(&self, world_x: i32, world_z: i32) -> (i32, Identifier) {
        let clamp = |y: i32| y.clamp(self.geometry.min_y(), self.geometry.max_y() - 1);
        if let Some((climate, density)) = self.vanilla()
            && let Some(y) = density.lod_surface_height(world_x, world_z)
        {
            let y = clamp(y);
            let biome = Identifier::parse(climate.sample(world_x, y, world_z).biome)
                .expect("validated vanilla biome identifier");
            return (y, biome);
        }
        let sample = self.diagnostic_sample_uncached(world_x, world_z, None);
        (clamp(sample.surface_y), sample.biome)
    }

    /// The far-terrain generation program native clients may run for
    /// never-generated columns (see [`crate::vanilla_density::program`]);
    /// `Ok(None)` without vanilla content or the LOD surface shape. It
    /// reveals the seed's terrain noise: send it only on operator opt-in.
    pub fn lod_program(&self) -> Result<Option<Vec<u8>>, String> {
        match self.vanilla() {
            Some((climate, density)) => crate::vanilla_density::program::encode(density, climate),
            None => Ok(None),
        }
    }

    /// Evaluate chunk density on the GPU in batches shared by every thread
    /// generating chunks (call after [`Self::load_vanilla_worldgen`]).
    /// Returns the adapter's name; errors leave generation on the CPU.
    pub fn enable_gpu(&mut self) -> Result<String, String> {
        #[cfg(feature = "gpu")]
        {
            let (_, density) = self
                .vanilla()
                .ok_or("GPU worldgen needs vanilla worldgen content")?;
            let gpu = crate::vanilla_density::gpu::GpuDensity::new(density)?;
            let adapter = gpu.adapter().to_owned();
            self.gpu = Some(
                gpu_service::GpuSolidService::spawn(gpu)
                    .map_err(|error| format!("worldgen GPU thread: {error}"))?,
            );
            Ok(adapter)
        }
        #[cfg(not(feature = "gpu"))]
        Err("this build has no GPU worldgen (feature `gpu`)".into())
    }
}
