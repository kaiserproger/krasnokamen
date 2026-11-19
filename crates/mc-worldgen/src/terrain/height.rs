//! Column height inputs and the generation-local height cache.

#[cfg(feature = "worldgen-counters")]
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

#[cfg(feature = "worldgen-counters")]
static HEIGHT_REQUESTS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "worldgen-counters")]
static HEIGHT_UNIQUE_KEYS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "worldgen-counters")]
static HEIGHT_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "worldgen-counters")]
static HEIGHT_COLUMN_SEARCHES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "worldgen-counters")]
static HEIGHT_COLUMN_ITERATIONS: AtomicU64 = AtomicU64::new(0);

/// Snapshot of the generation-local height-context cache counters.
///
/// Only populated with the `worldgen-counters` feature; without it every
/// increment below compiles out and collection costs nothing.
///
/// Field semantics: `base_height_requests` counts every `source_height`
/// entry; `unique_height_keys` counts distinct columns ever inserted (one
/// insert each — NOT counting `for_chunk` seeding, which inserts without a
/// request); `height_cache_hits` counts map hits on the request path;
/// `column_iterations_from_height_queries` counts top-down Y-steps of the
/// column search (`max_y - 1 - raw`, off by one vs successful samples by
/// construction — a diagnostic shape metric, NOT noise evaluations: memo
/// and Min/Max change actual evaluations independently; use DensityCounters
/// for those); `actual_density_column_searches` counts the searches.
#[cfg(feature = "worldgen-counters")]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HeightContextStats {
    pub source_hits: u64,
    pub source_misses: u64,
    pub plan_hits: u64,
    pub plan_misses: u64,
    pub base_height_requests: u64,
    pub unique_height_keys: u64,
    pub height_cache_hits: u64,
    pub actual_density_column_searches: u64,
    pub column_iterations_from_height_queries: u64,
}

/// Read the height-context counters accumulated so far.
#[cfg(feature = "worldgen-counters")]
#[must_use]
pub fn height_context_stats() -> HeightContextStats {
    HeightContextStats {
        source_hits: HEIGHT_REQUESTS.load(Ordering::Relaxed),
        source_misses: HEIGHT_UNIQUE_KEYS.load(Ordering::Relaxed),
        plan_hits: HEIGHT_CACHE_HITS.load(Ordering::Relaxed),
        plan_misses: HEIGHT_COLUMN_SEARCHES.load(Ordering::Relaxed),
        base_height_requests: HEIGHT_REQUESTS.load(Ordering::Relaxed),
        unique_height_keys: HEIGHT_UNIQUE_KEYS.load(Ordering::Relaxed),
        height_cache_hits: HEIGHT_CACHE_HITS.load(Ordering::Relaxed),
        actual_density_column_searches: HEIGHT_COLUMN_SEARCHES.load(Ordering::Relaxed),
        column_iterations_from_height_queries: HEIGHT_COLUMN_ITERATIONS.load(Ordering::Relaxed),
    }
}

/// Zero the height-context counters, e.g. between benchmark rounds.
#[cfg(feature = "worldgen-counters")]
pub fn reset_height_context_stats() {
    HEIGHT_REQUESTS.store(0, Ordering::Relaxed);
    HEIGHT_UNIQUE_KEYS.store(0, Ordering::Relaxed);
    HEIGHT_CACHE_HITS.store(0, Ordering::Relaxed);
    HEIGHT_COLUMN_SEARCHES.store(0, Ordering::Relaxed);
    HEIGHT_COLUMN_ITERATIONS.store(0, Ordering::Relaxed);
}

#[inline]
fn count_height_request() {
    #[cfg(feature = "worldgen-counters")]
    HEIGHT_REQUESTS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
fn count_height_unique_key() {
    #[cfg(feature = "worldgen-counters")]
    HEIGHT_UNIQUE_KEYS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
fn count_height_cache_hit() {
    #[cfg(feature = "worldgen-counters")]
    HEIGHT_CACHE_HITS.fetch_add(1, Ordering::Relaxed);
}

#[inline]
fn count_height_column_search(column_iterations: u64) {
    #[cfg(feature = "worldgen-counters")]
    {
        HEIGHT_COLUMN_SEARCHES.fetch_add(1, Ordering::Relaxed);
        HEIGHT_COLUMN_ITERATIONS.fetch_add(column_iterations, Ordering::Relaxed);
    }
    #[cfg(not(feature = "worldgen-counters"))]
    let _ = column_iterations;
}

/// Generation-local height context: the chunk's own source heights plus one
/// shared [`SurfaceHeightSampler`] + one village-plan memo the decorations
/// (trees), ore-halo reads and structure writes consult instead of re-running
/// the full height + plan pipeline per neighbour column.
///
/// Cache key discipline: `source_heights` holds each column's SOURCE
/// (pre-village) surface — clamped to `geometry.min_y()..=max_y()-1` — so a
/// village plan can still move the column later without a second SOURCE
/// evaluation. Single village adjustment only: [`HeightContext::adjusted_height`]
/// reads exactly one [`VillagePlanSet`] (the chunk's set for generation-local
/// columns, the column's own chunk's set for halo readers) and applies it once;
/// callers must not pre-adjust or post-adjust.
///
/// Order-independence: every entry is a pure function of `(seed, world_x,
/// world_z)` — SOURCE through the density query, the plan set through the
/// village lookup keyed by chunk, and the final height through the beard
/// analogue — so visitation order only decides hits vs misses, never values.
pub(crate) struct HeightContext<'h> {
    generator: &'h TerrainGenerator,
    geometry_min_y: i32,
    geometry_max_y: i32,
    source_heights: HashMap<(i32, i32), i32>,
    native_sampler: Option<SurfaceHeightSampler<'h>>,
    plans: HashMap<(i32, i32), Option<VillagePlanSet>>,
    /// Precomputed solids, with the density's min y.
    solids: Option<(&'h SolidGrid, i32)>,
}

impl<'h> HeightContext<'h> {
    /// Build a context seeded with the central chunk's ready SOURCE heights.
    /// `source_columns` are the chunk's own entries straight from the
    /// diagnostic cache the fill-density pass inserted (the same entries
    /// `plan_column` planned from); the context re-applies the village set
    /// per read so planned and unplanned readers agree.
    pub(crate) fn for_chunk(
        generator: &'h TerrainGenerator,
        pos: ChunkPos,
        source_columns: &[(i32, i32, i32)],
        native_sampler: Option<SurfaceHeightSampler<'h>>,
    ) -> Self {
        let mut source_heights = HashMap::with_capacity(source_columns.len().max(256));
        for &(wx, wz, base) in source_columns {
            source_heights.insert((wx, wz), base);
        }
        let _ = pos;
        Self {
            generator,
            geometry_min_y: generator.geometry.min_y(),
            geometry_max_y: generator.geometry.max_y(),
            source_heights,
            native_sampler,
            plans: HashMap::new(),
            solids: None,
        }
    }

    /// Answer SOURCE heights and cave checks of the chunks `solids` holds
    /// from them instead of the halo sampler (same answers). `min_y` is the
    /// density's.
    pub(crate) fn with_solids(mut self, solids: &'h SolidGrid, min_y: i32) -> Self {
        self.solids = Some((solids, min_y));
        self
    }

    /// Whether native terrain density leaves one block empty (`<= 0`), from
    /// held solids or the shared halo sampler; `None` without vanilla density.
    pub(super) fn native_empty(&mut self, world_x: i32, y: i32, world_z: i32) -> Option<bool> {
        if let Some(solid) = self
            .solids
            .and_then(|(solids, min_y)| solids.solid(world_x, y, world_z, min_y))
        {
            return Some(!solid);
        }
        self.native_sampler
            .as_mut()
            .map(|sampler| sampler.density(world_x, y, world_z) <= 0.0)
    }

    /// The SOURCE (pre-village) surface for one column: ready central-chunk
    /// values first, then the shared sampler, then the per-column query.
    fn source_height(&mut self, world_x: i32, world_z: i32) -> i32 {
        count_height_request();
        if let Some(&height) = self.source_heights.get(&(world_x, world_z)) {
            count_height_cache_hit();
            return height;
        }
        // ONE shared sampler: lazily computed halo columns retain lattice
        // corners across columns instead of one sampler per column.
        let held = self
            .solids
            .and_then(|(solids, min_y)| solids.surface_height(world_x, world_z, min_y));
        let raw = if let Some(held) = held.filter(|_| self.native_sampler.is_some()) {
            held
        } else if let Some(sampler) = self.native_sampler.as_mut() {
            sampler.surface_height(world_x, world_z)
        } else {
            self.generator.base_surface_y(world_x, world_z)
        };
        // `base_surface_y` already clamps vanilla heights to the geometry;
        // clamp the sampler path identically so every stored value is final.
        let clamped = raw.clamp(self.geometry_min_y, self.geometry_max_y - 1);
        // The column search walks top-down one `sample_terrain` per Y until
        // the first solid: `column_iterations = max_y - 1 - raw` by
        // construction (off by one vs successful samples — the successful
        // sample yields 0 further steps). Diagnostic shape metric, NOT a
        // noise-evaluation count (see DensityCounters for actual evaluations).
        // The own-router path is a single closed-form column query, counted
        // as one search of one iteration.
        let column_iterations = if self.native_sampler.is_some() {
            (self.geometry_max_y - 1 - raw.min(self.geometry_max_y - 1)).max(0) as u64
        } else {
            1
        };
        count_height_column_search(column_iterations);
        count_height_unique_key();
        self.source_heights.insert((world_x, world_z), clamped);
        clamped
    }

    /// The chunk's plan set, memoized: one lookup per chunk reached.
    /// (No height counters: the plan memo is village work, not a height
    /// query; the required five counters live on the SOURCE path above.)
    fn plans_for_chunk(&mut self, chunk: ChunkPos) -> Option<VillagePlanSet> {
        if let Some(hit) = self.plans.get(&(chunk.x, chunk.z)) {
            return hit.clone();
        }
        let set = self.generator.village_plans_for_chunk(chunk);
        self.plans.insert((chunk.x, chunk.z), set.clone());
        set
    }

    /// Final (village-adjusted) height: cached SOURCE + single adjustment.
    pub(crate) fn adjusted_height(&mut self, world_x: i32, world_z: i32) -> i32 {
        let base = self.source_height(world_x, world_z);
        let chunk = ChunkPos {
            x: world_x.div_euclid(16),
            z: world_z.div_euclid(16),
        };
        let plans = self.plans_for_chunk(chunk);
        self.generator
            .village_surface_y(world_x, world_z, base, plans.as_ref())
    }

    /// Column plan for halo readers that need more than a height: SOURCE
    /// diagnostic (cached by the generator; a miss takes its height from
    /// this context's shared sampler) + this context's single plan set.
    pub(super) fn halo_column(&mut self, world_x: i32, world_z: i32) -> ColumnPlan {
        let chunk = ChunkPos {
            x: world_x.div_euclid(16),
            z: world_z.div_euclid(16),
        };
        let plans = self.plans_for_chunk(chunk);
        // An independent column query walks the whole height with a fresh
        // sampler (~0.1 ms); the shared lattice answers the same height
        // (`vanilla_halo_surface_heights_match_independent_queries`) for a
        // fraction. Halo planning was 89 of 104 ms of a fresh chunk.
        let diagnostic = match self.generator.diagnostic_cache.get(world_x, world_z) {
            Some(sample) => sample,
            None => {
                let height = self
                    .native_sampler
                    .is_some()
                    .then(|| self.source_height(world_x, world_z));
                let sample = self
                    .generator
                    .diagnostic_sample_uncached(world_x, world_z, height);
                self.generator
                    .diagnostic_cache
                    .insert(world_x, world_z, sample)
            }
        };
        let lx = world_x.rem_euclid(16) as u8;
        let lz = world_z.rem_euclid(16) as u8;
        self.generator
            .plan_column_from_diagnostic(chunk, lx, lz, plans.as_ref(), diagnostic)
    }

    /// WORLD_SURFACE_WG after terrain adaptation, including water. Gravity
    /// must follow the fitted terrain, not restore dirt across an entrance cut.
    pub(crate) fn structure_surface(&mut self, world_x: i32, world_z: i32) -> i32 {
        let base = self.source_height(world_x, world_z);
        let chunk = ChunkPos {
            x: world_x.div_euclid(16),
            z: world_z.div_euclid(16),
        };
        let plans = self
            .plans
            .entry((chunk.x, chunk.z))
            .or_insert_with(|| self.generator.village_plans_for_chunk(chunk));
        let solid_y = self
            .generator
            .village_surface_y(world_x, world_z, base, plans.as_ref());
        self.generator.world_surface_y(solid_y)
    }
    /// Map entries held (seeded + lazily resolved). Test hook only.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.source_heights.len()
    }
}
