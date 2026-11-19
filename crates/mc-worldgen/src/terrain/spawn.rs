//! Deterministic natural spawn search on terrain this generator produces.

use super::*;

const SPAWN_SEARCH_STEP_BLOCKS: i32 = 64;
const SPAWN_SEARCH_MAX_RADIUS_BLOCKS: i32 = 8_192;
const SPAWN_SITE_SAMPLE_RADIUS_BLOCKS: i32 = 8;
const SPAWN_SITE_MAX_RELIEF: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnLocation {
    pub block_x: i32,
    pub surface_y: i32,
    pub block_z: i32,
}

impl SpawnLocation {
    #[must_use]
    pub const fn chunk(self) -> ChunkPos {
        ChunkPos {
            x: self.block_x.div_euclid(16),
            z: self.block_z.div_euclid(16),
        }
    }
}

impl TerrainGenerator {
    /// Find a deterministic natural spawn centre without modifying terrain.
    ///
    /// The search walks expanding 64-block rings around the origin and accepts
    /// only dry, low-relief inland terrain away from river centres and strong
    /// mountain ridges. Each routing-level candidate is then checked against the
    /// chunk this generator actually produces for it (features, decoration and
    /// village pieces included): a dry, non-fluid support block at `surface_y`
    /// and air in the two body blocks above. A candidate that fails is skipped
    /// for the next one within the same bounded search.
    #[must_use]
    pub fn locate_safe_spawn(&self) -> Option<SpawnLocation> {
        self.spawn_candidates()
            .find(|spawn| self.spawn_is_clear(*spawn))
    }

    fn spawn_is_clear(&self, spawn: SpawnLocation) -> bool {
        let chunk = ChunkGenerator::generate(self, spawn.chunk());
        let lx = spawn.block_x.rem_euclid(16) as u8;
        let lz = spawn.block_z.rem_euclid(16) as u8;
        chunk.get_block(lx, spawn.surface_y, lz).is_some_and(|id| {
            id != self.air
                && self
                    .blocks
                    .by_id(id)
                    .is_some_and(|state| !state.has_fluid())
        }) && chunk.get_block(lx, spawn.surface_y + 1, lz) == Some(self.air)
            && chunk.get_block(lx, spawn.surface_y + 2, lz) == Some(self.air)
    }

    /// Routing-level spawn candidates in search order: the origin, then each
    /// expanding 64-block ring.
    fn spawn_candidates(&self) -> impl Iterator<Item = SpawnLocation> + '_ {
        let max_ring = SPAWN_SEARCH_MAX_RADIUS_BLOCKS / SPAWN_SEARCH_STEP_BLOCKS;
        let rings = (1..=max_ring).flat_map(|ring| {
            let radius = ring * SPAWN_SEARCH_STEP_BLOCKS;
            let rows = (-ring..=ring).flat_map(move |offset| {
                let coordinate = offset * SPAWN_SEARCH_STEP_BLOCKS;
                [(coordinate, -radius), (coordinate, radius)]
            });
            let columns = ((-ring + 1)..ring).flat_map(move |offset| {
                let coordinate = offset * SPAWN_SEARCH_STEP_BLOCKS;
                [(-radius, coordinate), (radius, coordinate)]
            });
            rows.chain(columns)
        });
        std::iter::once((0, 0))
            .chain(rings)
            .filter_map(|(x, z)| self.spawn_candidate(x, z))
    }

    fn spawn_candidate(&self, block_x: i32, block_z: i32) -> Option<SpawnLocation> {
        let centre = self.routing_sample(block_x, block_z);
        let sea_level = SEA_LEVEL;
        if centre.surface_y < sea_level + 4
            || centre.continentalness < 0.02
            || centre.river < 0.08
            || centre.ridges > 0.24
        {
            return None;
        }

        let mut minimum = centre.surface_y;
        let mut maximum = centre.surface_y;
        for dz in [
            -SPAWN_SITE_SAMPLE_RADIUS_BLOCKS,
            0,
            SPAWN_SITE_SAMPLE_RADIUS_BLOCKS,
        ] {
            for dx in [
                -SPAWN_SITE_SAMPLE_RADIUS_BLOCKS,
                0,
                SPAWN_SITE_SAMPLE_RADIUS_BLOCKS,
            ] {
                let sample =
                    self.routing_sample(block_x.checked_add(dx)?, block_z.checked_add(dz)?);
                if sample.surface_y < sea_level + 3 || sample.river < 0.04 {
                    return None;
                }
                minimum = minimum.min(sample.surface_y);
                maximum = maximum.max(sample.surface_y);
            }
        }
        if maximum - minimum > SPAWN_SITE_MAX_RELIEF {
            return None;
        }
        Some(SpawnLocation {
            block_x,
            surface_y: centre.surface_y,
            block_z,
        })
    }
}
