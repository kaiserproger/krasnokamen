//! Village and jigsaw pieces, plan lookups and their column queries.

use super::height::HeightContext;

use super::*;

/// Plan-free columns used when admitting and projecting jigsaw starts.
pub(super) struct JigsawHeightQuery<'a>(pub(super) &'a TerrainGenerator);

impl crate::village::height_query::HeightQuery for JigsawHeightQuery<'_> {
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.0.first_free_height(x, z)
    }

    fn ocean_floor(&self, x: i32, z: i32) -> Option<i32> {
        Some(self.0.base_surface_y(x, z).saturating_add(1))
    }

    fn top_is_liquid(&self, x: i32, z: i32) -> Option<bool> {
        let ground = self.0.base_surface_y(x, z);
        Some(self.0.world_surface_y(ground) > ground)
    }

    fn sea_level(&self) -> Option<i32> {
        Some(SEA_LEVEL)
    }
}

impl TerrainGenerator {
    /// Every vanilla village whose start chunk lies in the inclusive chunk
    /// rectangle `min_chunk..=max_chunk`, as the plans this generator generates
    /// them from.
    ///
    /// The two column queries the plan enumeration reads are this generator's
    /// own plan-free ones ([`TerrainGenerator::first_free_height`] and
    /// [`TerrainGenerator::base_biome`]), so a listed village is the village a
    /// later chunk fill will actually place, and listing one generates nothing.
    /// A generator with no village plans lists nothing.
    #[must_use]
    pub fn village_sites_in_region(
        &self,
        min_chunk: (i32, i32),
        max_chunk: (i32, i32),
    ) -> Vec<mc_world::GeneratedVillageSite> {
        let Some(source) = self.village_plans.as_ref() else {
            return Vec::new();
        };
        let free_height = |x, z| self.first_free_height(x, z);
        let biome_at = |x, y, z| self.noise_biome(x, y, z);
        source.sites_in_region(min_chunk, max_chunk, &free_height, &biome_at)
    }

    /// The plans the chunk being filled reads its pieces and beard columns
    /// from: a single on-demand lookup per generated chunk.
    pub(super) fn village_plans_for_chunk(&self, pos: ChunkPos) -> Option<VillagePlanSet> {
        let source = self.village_plans.as_ref()?;
        let free_height = |x, z| self.first_free_height(x, z);
        let biome_at = |x, y, z| self.noise_biome(x, y, z);
        source.plans_for_chunk(pos.x, pos.z, &free_height, &biome_at)
    }

    /// The plans covering one column, for callers that hold a column rather than
    /// the chunk being filled. The chunk's own set filtered to the column, so a
    /// column and its chunk cannot disagree.
    pub(super) fn village_plans_for_column(
        &self,
        world_x: i32,
        world_z: i32,
    ) -> Option<VillagePlanSet> {
        let source = self.village_plans.as_ref()?;
        let free_height = |x, z| self.first_free_height(x, z);
        let biome_at = |x, y, z| self.noise_biome(x, y, z);
        source.plans_for_column(world_x, world_z, &free_height, &biome_at)
    }

    /// The surface a column is planned at: the terrain's own surface, moved by
    /// the column-height analogue of every plan that reaches the column.
    pub(super) fn village_surface_y(
        &self,
        world_x: i32,
        world_z: i32,
        base: i32,
        plans: Option<&VillagePlanSet>,
    ) -> i32 {
        match plans {
            Some(plans) => plans.adjusted_surface_y(
                world_x,
                world_z,
                base,
                self.geometry.min_y(),
                self.geometry.max_y(),
            ),
            None => base,
        }
    }

    /// Write every plan's piece blocks into this chunk and refresh the
    /// heightmaps of the columns they touched.
    ///
    /// Pieces are clipped to the chunk's box, so a village spanning several
    /// chunks writes each chunk's part of it when that chunk is filled, and a
    /// chunk two villages reach writes both.
    ///
    /// A piece that cannot be placed is a registry mismatch, and
    /// [`VillagePlanSource::new`] refuses such a registry at startup rather than
    /// dropping blocks here. Chunk generation has no way to report a failure, so
    /// the invariant is enforced where it can be, once, before a world uses the
    /// village — and reaching the failure here means the village would silently
    /// lose blocks, which is worse than stopping.
    pub(super) fn apply_village_pieces(
        &self,
        chunk: &mut Chunk,
        plans: &VillagePlanSet,
        heights: &mut HeightContext<'_>,
    ) {
        let Some(source) = self.village_plans.as_ref() else {
            return;
        };
        // The piece writer's surface is the terrain's own (SOURCE) surface —
        // the `WORLD_SURFACE_WG` heightmap the `terrain_matching` projection's
        // gravity processor reads — served from the shared context so repeated
        // piece-column queries do not re-run the density query per column.
        let heights_cell = RefCell::new(heights);
        let surface = |x: i32, z: i32| heights_cell.borrow_mut().structure_surface(x, z);
        let mut touched = [false; 256];
        source
            .write_plans(chunk, plans, &mut touched, &surface, self.air)
            .expect("the plan source was validated against this registry at construction");
        for lz in 0..16u8 {
            for lx in 0..16u8 {
                if touched[lz as usize * 16 + lx as usize] {
                    self.refresh_structure_column(chunk, lx, lz);
                }
            }
        }
    }

    pub(super) fn apply_jigsaw_structures(&self, chunk: &mut Chunk) {
        let surface = |x, z| self.first_free_height(x, z);
        let query = JigsawHeightQuery(self);
        let biome = |x, y, z| self.noise_biome(x, y, z);
        let mut touched = [false; 256];
        for source in &self.jigsaw_sources {
            let Some(plans) = source.plans_for_chunk(chunk.pos.x, chunk.pos.z, &query, &biome)
            else {
                continue;
            };
            source
                .write_plans(chunk, &plans, &mut touched, &surface, self.air)
                .expect("jigsaw source validated at construction");
        }
        for z in 0..16u8 {
            for x in 0..16u8 {
                if touched[z as usize * 16 + x as usize] {
                    self.refresh_structure_column(chunk, x, z);
                }
            }
        }
    }
}
