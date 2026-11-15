//! Plan-free Nether queries for the shared Moog jigsaw solver.
//!
//! The landscape occupies Y 0..=127 even though chunks have 256 writable
//! layers. The bedrock roof is a heightmap surface, not land for a structure:
//! HIGHEST_LAND searches below it, from generation-top minus 40, for a solid
//! block with air three blocks above. It returns the occupied block's Y, not
//! a first-free height. No suitable land is `None`, never a sea-level fallback.
//!
//! Behaviour reference only (no copied implementation): Horizon's public
//! `worldgen/structure/piece_limited.rs`, `nether` and `land_in`. The source
//! adapter and persistence remain the shared village plan/native-store path.

use super::{NETHER_CEILING_Y, NETHER_LAVA_LEVEL, NETHER_MIN_Y, NetherGenerator};
use crate::village::height_query::HeightQuery;

/// The unmodified landscape, without ores, decorations or structure writes.
pub(super) struct NetherMoogQuery<'a> {
    pub(super) generator: &'a NetherGenerator,
}

impl HeightQuery for NetherMoogQuery<'_> {
    fn world_surface(&self, _x: i32, _z: i32) -> i32 {
        // fill_landscape always writes bedrock at the roof, and nothing above.
        NETHER_CEILING_Y + 1
    }

    fn ocean_floor(&self, x: i32, z: i32) -> Option<i32> {
        // The same roof is solid, so both heightmaps include it.
        Some(self.world_surface(x, z))
    }

    fn top_is_liquid(&self, _x: i32, _z: i32) -> Option<bool> {
        Some(false)
    }

    fn sea_level(&self) -> Option<i32> {
        Some(NETHER_LAVA_LEVEL)
    }
}

impl NetherMoogQuery<'_> {
    /// The generator's biome source is columnar; Y is intentionally ignored
    /// by `NetherGenerator::noise_biome` too.
    pub(super) fn biome_at(&self, x: i32, _y: i32, z: i32) -> mc_data::Identifier {
        self.generator.biomes[self.generator.biome_index(x, z)].clone()
    }
}

/// Find HIGHEST_LAND in the real noise column within `[min_y, max_y)`.
///
/// The search excludes the sea and the top 40 generation layers, matching the
/// referenced land rule. Chunk storage above the generated roof must not move
/// that search band. The solid ground and the air-at-Y+3 check use the same
/// interpolated density as `fill_landscape`; lava is not air or valid ground.
/// No extra contiguous-headroom condition is invented here.
pub(super) fn highest_land(
    generator: &NetherGenerator,
    x: i32,
    z: i32,
    min_y: i32,
    max_y: i32,
) -> Option<i32> {
    let bottom = min_y.max(NETHER_MIN_Y).max(NETHER_LAVA_LEVEL + 1);
    let top = max_y.min(NETHER_CEILING_Y + 1).checked_sub(40)?;
    if bottom > top {
        return None;
    }
    let column = generator.density_column(x, z);
    (bottom..=top)
        .rev()
        .find(|&y| NetherGenerator::solid(&column, y) && !NetherGenerator::solid(&column, y + 3))
}

#[cfg(test)]
#[path = "moog_jigsaw_tests.rs"]
mod tests;
