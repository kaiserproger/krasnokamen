//! Density-router and vanilla climate sampling: heights, biomes, diagnostics.

use super::*;

/// Scale and coverage of gravel bars on river beds.
const RIVER_GRAVEL_SCALE: f64 = 34.0;
const RIVER_GRAVEL_THRESHOLD: f64 = 0.30;
const RIVER_GRAVEL_SALT: i64 = 0x5247_5256;

const VEGETATION_REGION_SCALE: f64 = 192.0;

impl TerrainGenerator {
    /// Sample the terrain height for an absolute world `(x, z)`.
    /// Public so tests + spawn-position picking can use the same
    /// function the generator does.
    ///
    /// A configured village plan source applies the column-height analogue
    /// here, on the same on-demand plan lookup
    /// ([`VillagePlanSource::plans_for_column`]) the chunk fill reads its own
    /// chunk's plans from, so this query and the generated terrain agree.
    #[must_use]
    pub fn surface_height(&self, world_x: i32, world_z: i32) -> i32 {
        let base = self.base_surface_y(world_x, world_z);
        match self.village_plans_for_column(world_x, world_z) {
            Some(plans) => self.village_surface_y(world_x, world_z, base, Some(&plans)),
            None => base,
        }
    }

    /// The router's surface for one column, before the village analogue.
    pub(super) fn base_surface_y(&self, world_x: i32, world_z: i32) -> i32 {
        if let Some((_, density)) = self.vanilla() {
            return density
                .surface_height(world_x, world_z)
                .clamp(self.geometry.min_y(), self.geometry.max_y() - 1);
        }
        self.density_router().sample(world_x, world_z).surface_y
    }

    /// The plan-free WORLD_SURFACE_WG top block includes water; the density
    /// surface alone is the ocean/river floor. Match the fill lane's water top.
    pub(super) fn world_surface_y(&self, solid_y: i32) -> i32 {
        let water_top = if self.vanilla().is_some() {
            SEA_LEVEL - 1
        } else {
            SEA_LEVEL
        };
        solid_y.max(water_top).min(self.geometry.max_y() - 1)
    }

    /// WORLD_SURFACE_WG's first free height, used by jigsaw assembly.
    pub(super) fn first_free_height(&self, world_x: i32, world_z: i32) -> i32 {
        self.world_surface_y(self.base_surface_y(world_x, world_z))
            .saturating_add(1)
    }

    /// The terrain's own biome at a column, before any village is applied.
    ///
    /// This is the biome the structure gate reads, and it deliberately does not
    /// go through [`TerrainGenerator::surface_height`] or
    /// [`TerrainGenerator::diagnostic_sample`]: both consult the plan lookup,
    /// which asks this question, so a plan-aware path here would recurse.
    #[must_use]
    pub fn base_biome(&self, world_x: i32, world_z: i32) -> Identifier {
        self.cached_diagnostic_sample(world_x, world_z).biome
    }

    /// Sample the same terrain, biome, and vegetation decisions used to plan a
    /// generated column, without constructing or mutating a chunk.
    ///
    /// A configured village plan source moves `surface_y` exactly where it moves
    /// a generated column's surface: [`TerrainGenerator::surface_height`],
    /// [`TerrainGenerator::plan_column`] and this sample all read one plan per
    /// chunk ([`VillagePlanSource::plans_for_column`] here, the chunk's own plans
    /// inside generation), so they cannot disagree.
    #[must_use]
    pub fn diagnostic_sample(&self, world_x: i32, world_z: i32) -> TerrainDiagnosticSample {
        let sample = self.cached_diagnostic_sample(world_x, world_z);
        let plans = self.village_plans_for_column(world_x, world_z);
        TerrainDiagnosticSample {
            surface_y: self.village_surface_y(world_x, world_z, sample.surface_y, plans.as_ref()),
            ..sample
        }
    }

    /// The terrain's own decisions for a column, memoized, before the village
    /// analogue. The fill path reads this and applies its chunk's plan to the
    /// result, so the analogue is applied once per consumer, not twice.
    pub(super) fn cached_diagnostic_sample(
        &self,
        world_x: i32,
        world_z: i32,
    ) -> TerrainDiagnosticSample {
        if let Some(sample) = self.diagnostic_cache.get(world_x, world_z) {
            return sample;
        }
        let sample = self.diagnostic_sample_uncached(world_x, world_z, None);
        self.diagnostic_cache.insert(world_x, world_z, sample)
    }

    pub(super) fn diagnostic_sample_uncached(
        &self,
        world_x: i32,
        world_z: i32,
        native_height: Option<i32>,
    ) -> TerrainDiagnosticSample {
        // The vanilla branch never reads the own-router sample: `surface_y` is
        // overwritten from the native density, `moisture` from the native
        // climate, and `biome` from `noise_biome`. The router call below is
        // therefore dead work in exactly this branch — and skipping it is
        // side-effect-free by construction: `OverworldRouter::sample` and
        // `fbm_2d` are pure functions of `(seed, x, z)` (no statics, cells,
        // locks or atomics anywhere in `overworld/*` or `noise.rs`), so no
        // memo, counter or global observes the call.
        if let Some((climate, density)) = self.vanilla() {
            let surface_y = native_height
                .unwrap_or_else(|| density.surface_height(world_x, world_z))
                .clamp(self.geometry.min_y(), self.geometry.max_y() - 1);
            let native = climate.sample(world_x, surface_y, world_z);
            let biome =
                Identifier::parse(native.biome).expect("validated vanilla biome identifier");
            let vegetation_density = self
                .biome_supports_land_vegetation(&biome)
                .then(|| self.vegetation_density(world_x, world_z, f64::from(native.climate[1])));
            return TerrainDiagnosticSample {
                surface_y,
                biome,
                vegetation_density,
            };
        }
        let sample = self.density_router().sample(world_x, world_z);
        let biome = self.vanilla_biome_for(world_x, world_z, sample.surface_y, sample);
        let surface_y = sample.surface_y;
        let vegetation_density = self
            .biome_supports_land_vegetation(&biome)
            .then(|| self.vegetation_density(world_x, world_z, sample.moisture));
        TerrainDiagnosticSample {
            surface_y,
            biome,
            vegetation_density,
        }
    }

    pub(super) fn density_router(&self) -> OverworldRouter {
        OverworldRouter::new(self.seed, self.geometry)
    }

    pub(super) fn routing_sample(&self, x: i32, z: i32) -> TerrainSample {
        if let Some((climate, density)) = self.vanilla() {
            let surface_y = density
                .surface_height(x, z)
                .clamp(self.geometry.min_y(), self.geometry.max_y() - 1);
            let sample = climate.sample(x, surface_y, z).climate;
            return TerrainSample {
                surface_y,
                temperature: f64::from(sample[0]),
                moisture: f64::from(sample[1]),
                continentalness: f64::from(sample[2]),
                erosion: f64::from(sample[3]),
                ridges: f64::from(sample[5].abs()),
                river: f64::from(sample[5].abs()),
                wetland: 0.0,
                climate_domain: 0.0,
            };
        }
        self.density_router().sample(x, z)
    }

    #[cfg(test)]
    pub(super) fn biome_for_cell(
        &self,
        world_x: i32,
        y: i32,
        world_z: i32,
        surface_height: i32,
    ) -> Identifier {
        if self.vanilla().is_some() {
            return self.noise_biome(world_x, y, world_z);
        }
        if i64::from(y) < i64::from(surface_height) - 24 && y < 32 {
            return self
                .biomes
                .pick(&self.biomes.cave, world_x, world_z, self.seed, 0x554E_4447);
        }
        self.biome_for(world_x, world_z, surface_height)
    }

    pub(super) fn vegetation_density(&self, world_x: i32, world_z: i32, moisture: f64) -> f64 {
        let regional = fbm_2d(
            f64::from(world_x) / VEGETATION_REGION_SCALE,
            f64::from(world_z) / VEGETATION_REGION_SCALE,
            self.seed ^ 0x5645_4745,
            2,
            0.5,
        );
        (regional * 0.72 + moisture * 0.28).clamp(-1.0, 1.0)
    }

    /// Gravel bars along a river bed: a coarse two-octave field so bars read as
    /// patches rather than per-block speckle.
    pub(super) fn river_gravel_bar(&self, world_x: i32, world_z: i32) -> bool {
        fbm_2d(
            f64::from(world_x) / RIVER_GRAVEL_SCALE,
            f64::from(world_z) / RIVER_GRAVEL_SCALE,
            self.seed ^ RIVER_GRAVEL_SALT,
            2,
            0.5,
        ) > RIVER_GRAVEL_THRESHOLD
    }

    pub(super) fn biome_supports_land_vegetation(&self, biome: &Identifier) -> bool {
        self.biomes.grassland.contains(biome)
            || self.biomes.temperate_forest.contains(biome)
            || self.biomes.jungle.contains(biome)
            || self.biomes.swamp.contains(biome)
            || Self::is_cold_forest(biome)
            || Self::is_savanna(biome)
    }
}
