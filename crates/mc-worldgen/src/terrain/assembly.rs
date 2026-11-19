//! Chunk assembly: `ChunkGenerator` wiring and `generate_with_solids`.

use super::height::HeightContext;
use super::structures::JigsawHeightQuery;

use super::*;

impl ChunkGenerator for TerrainGenerator {
    fn village_sites_in_region(
        &self,
        min_chunk: (i32, i32),
        max_chunk: (i32, i32),
    ) -> Vec<mc_world::GeneratedVillageSite> {
        TerrainGenerator::village_sites_in_region(self, min_chunk, max_chunk)
    }

    fn structure_sites_in_region(
        &self,
        min: (i32, i32),
        max: (i32, i32),
    ) -> Vec<mc_world::chunk::GeneratedStructureSite> {
        let mut sites = self
            .overworld_structures
            .as_ref()
            .map_or_else(Vec::new, |structures| {
                structures.sites(
                    min,
                    max,
                    |x, z| self.world_surface_y(self.surface_height(x, z)),
                    |x, z| self.surface_height(x, z),
                    |x, y, z| self.noise_biome(x, y, z),
                )
            });
        let surface = JigsawHeightQuery(self);
        let biome = |x, y, z| self.noise_biome(x, y, z);
        for source in &self.jigsaw_sources {
            sites.extend(source.structure_sites_in_region(min, max, &surface, &biome));
        }
        sites
    }

    fn structure_candidates_in_ring(
        &self,
        targets: &[Identifier],
        centre_chunk: (i32, i32),
        ring: u32,
    ) -> Vec<ChunkPos> {
        let mut candidates = self
            .overworld_structures
            .as_ref()
            .map_or_else(Vec::new, |structures| {
                structures.structure_candidates_in_ring(targets, centre_chunk, ring)
            });
        for source in self.village_plans.iter().chain(self.jigsaw_sources.iter()) {
            candidates.extend(source.structure_candidates_in_ring(targets, centre_chunk, ring));
        }
        candidates
    }

    fn structure_sites_at(
        &self,
        targets: &[Identifier],
        start_chunk: ChunkPos,
    ) -> Vec<mc_world::chunk::GeneratedStructureSite> {
        let mut sites = self
            .overworld_structures
            .as_ref()
            .map_or_else(Vec::new, |structures| {
                structures.structure_sites_at(
                    targets,
                    start_chunk,
                    |x, z| self.world_surface_y(self.surface_height(x, z)),
                    |x, z| self.surface_height(x, z),
                    |x, y, z| self.noise_biome(x, y, z),
                )
            });
        let surface = JigsawHeightQuery(self);
        let biome = |x, y, z| self.noise_biome(x, y, z);
        for source in self.village_plans.iter().chain(self.jigsaw_sources.iter()) {
            sites.extend(source.structure_sites_at(targets, start_chunk, &surface, &biome));
        }
        sites
    }

    fn noise_biome(&self, world_x: i32, y: i32, world_z: i32) -> Option<Identifier> {
        Some(TerrainGenerator::noise_biome(self, world_x, y, world_z))
    }

    fn surface_height(&self, world_x: i32, world_z: i32) -> Option<i32> {
        Some(Self::surface_height(self, world_x, world_z))
    }

    fn lod_column(&self, world_x: i32, world_z: i32) -> Option<(i32, Identifier)> {
        Some(Self::lod_column(self, world_x, world_z))
    }

    fn lod_program(&self) -> Result<Option<Vec<u8>>, String> {
        Self::lod_program(self)
    }

    fn generate(&self, pos: ChunkPos) -> Chunk {
        #[cfg(feature = "gpu")]
        if let Some(grid) = self.gpu.as_ref().and_then(|gpu| gpu.solids_around(pos)) {
            return self.generate_with_solids(pos, Some(&grid));
        }
        match self.vanilla() {
            Some((_, density)) => {
                let grid = self
                    .solid_cache
                    .around(pos, |pos| chunk_solids(density, pos));
                self.generate_with_solids(pos, Some(&grid))
            }
            None => self.generate_with_solids(pos, None),
        }
    }
}

/// Which blocks of the chunk at `pos` the density makes solid, on the CPU.
pub(super) fn chunk_solids(density: &VanillaDensity, pos: ChunkPos) -> ChunkSolids {
    let mut values = vec![0.0; density.height() as usize * 256];
    density.fill_density_chunk(
        world_block_coordinate(pos.x, 0),
        world_block_coordinate(pos.z, 0),
        &mut values,
    );
    ChunkSolids::from_densities(&values, density.height() as usize)
}

impl TerrainGenerator {
    /// [`ChunkGenerator::generate`] reading which blocks the density makes
    /// solid from `solids` (computed elsewhere, e.g. the GPU worldgen
    /// experiment) for this chunk and any held neighbour; anything not held
    /// is computed here.
    pub fn generate_with_solids(&self, pos: ChunkPos, solids: Option<&SolidGrid>) -> Chunk {
        #[cfg(test)]
        let mut stage_clock = profile_tests::StageClock::new();
        let mut chunk =
            Chunk::empty_with_geometry(pos, self.air, self.biomes.default.clone(), self.geometry);
        chunk
            .heightmaps
            .insert("MOTION_BLOCKING".into(), Heightmap::zeroed());
        chunk
            .heightmaps
            .insert("WORLD_SURFACE".into(), Heightmap::zeroed());
        #[cfg(test)]
        stage_clock.end(5);
        let native_solids = self.vanilla().map(|(climate, noise)| {
            let min_x = world_block_coordinate(pos.x, 0);
            let min_z = world_block_coordinate(pos.z, 0);
            let values = match solids.and_then(|solids| solids.chunk(pos)) {
                Some(held) => std::borrow::Cow::Borrowed(held),
                None => std::borrow::Cow::Owned(chunk_solids(noise, pos)),
            };
            // Density offsets inside the geometry: the top search's range.
            let low = (self.geometry.min_y() - noise.min_y()).max(0);
            let high = (self.geometry.max_y() - noise.min_y()).clamp(0, noise.height());
            let density = noise;
            #[cfg(test)]
            stage_clock.end(0);
            // Block columns grouped by biome quart: one column sampler then
            // reuses each quart's Y-free climate fields for its 16 columns.
            let mut sampler = climate.column_sampler();
            let mut parsed: Vec<(&str, Identifier)> = Vec::new();
            for order in 0..256 {
                let (quart, inner) = (order / 16, order % 16);
                let idx = (quart / 4 * 4 + inner / 4) * 16 + quart % 4 * 4 + inner % 4;
                let x = min_x + (idx % 16) as i32;
                let z = min_z + (idx / 16) as i32;
                let top = values
                    .top_below(idx, high as usize)
                    .filter(|offset| *offset as i32 >= low)
                    .map_or(self.geometry.min_y(), |offset| {
                        density.min_y() + offset as i32
                    });
                let native = sampler.sample(x, top, z);
                let biome = match parsed.iter().find(|(name, _)| *name == native.biome) {
                    Some((_, biome)) => biome.clone(),
                    None => {
                        let biome = Identifier::parse(native.biome)
                            .expect("validated vanilla biome identifier");
                        parsed.push((native.biome, biome.clone()));
                        biome
                    }
                };
                let vegetation_density = self
                    .biome_supports_land_vegetation(&biome)
                    .then(|| self.vegetation_density(x, z, f64::from(native.climate[1])));
                self.diagnostic_cache.insert(
                    x,
                    z,
                    TerrainDiagnosticSample {
                        surface_y: top,
                        biome,
                        vegetation_density,
                    },
                );
            }
            values
        });
        #[cfg(test)]
        stage_clock.end(6);
        let village = self.village_plans_for_chunk(pos);
        #[cfg(test)]
        stage_clock.end(4);
        let columns = std::array::from_fn(|idx| {
            let lx = (idx % 16) as u8;
            let lz = (idx / 16) as u8;
            self.plan_column(pos, lx, lz, village.as_ref())
        });
        // The generation-local height context: ready central-chunk SOURCE
        // heights (the diagnostic cache's own entries the columns planned
        // from, so no second derivation can disagree) plus ONE shared native
        // halo sampler for lazily reached neighbours plus the village-plan
        // memo. Tree, ore-halo and structure readers consult it instead of
        // re-running the full height + plan pipeline per column.
        //
        // The context's sampler covers exactly the columns a chunk fill can
        // read: the halo's reach is `ORE_COLUMN_HALO` (ore veins) inside the
        // ±2 tree-stability footprint; the sampler bound is the union.
        let source_columns: [(i32, i32, i32); 256] = std::array::from_fn(|idx| {
            let plan = &columns[idx];
            // The diagnostic cache already holds this column's SOURCE surface
            // (the fill-density pass inserted it above); `plan_column` read
            // the same entry, so no second derivation can disagree.
            let base = self
                .diagnostic_cache
                .get(plan.wx, plan.wz)
                .map(|sample| sample.surface_y)
                .unwrap_or_else(|| self.base_surface_y(plan.wx, plan.wz));
            (plan.wx, plan.wz, base)
        });
        let mut heights = HeightContext::for_chunk(
            self,
            pos,
            &source_columns,
            self.vanilla().map(|(_, density)| {
                let min_x = world_block_coordinate(pos.x, 0) as i64 - i64::from(ORE_COLUMN_HALO);
                let min_z = world_block_coordinate(pos.z, 0) as i64 - i64::from(ORE_COLUMN_HALO);
                let clamp =
                    |value: i64| value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
                density.surface_height_sampler(
                    clamp(min_x),
                    clamp(min_z),
                    clamp(min_x + 16 + i64::from(ORE_COLUMN_HALO) * 2 - 1),
                    clamp(min_z + 16 + i64::from(ORE_COLUMN_HALO) * 2 - 1),
                )
            }),
        );
        if let (Some(solids), Some((_, density))) = (solids, self.vanilla()) {
            heights = heights.with_solids(solids, density.min_y());
        }

        // Apply the same CPU aquifer decisions to CPU and GPU solid grids.
        let mut aquifer = self.vanilla().map(|(_, density)| {
            crate::vanilla_density::aquifer::Aquifer::new(density, self.seed as u64)
        });

        for plan in &columns {
            if let (Some(values), Some((_, density))) = (&native_solids, self.vanilla()) {
                let idx = usize::from(plan.lz) * 16 + usize::from(plan.lx);
                let badlands = plan.biome.path().contains("badlands");
                for y in self.geometry.min_y()..=plan.top_non_air {
                    let offset = y - density.min_y();
                    let solid = offset >= 0
                        && offset < density.height()
                        && values.solid(idx, offset as usize);
                    let state = if y == self.geometry.min_y() {
                        self.bedrock
                    } else if y == plan.height {
                        plan.surface
                    } else if y > plan.height {
                        self.water
                    } else if solid || y >= plan.dirt_start {
                        if badlands && y >= 64 {
                            let band =
                                (y.div_euclid(3) + self.seed.rem_euclid(8) as i32).rem_euclid(8);
                            self.terracotta_layers[band as usize]
                        } else if y >= plan.dirt_start {
                            plan.fill
                        } else {
                            self.base_stone_for_y(plan.lx, y, plan.lz, pos)
                        }
                    } else {
                        use crate::vanilla_density::aquifer::Fluid;
                        match aquifer
                            .as_mut()
                            .expect("vanilla aquifer")
                            .substance(plan.wx, y, plan.wz)
                        {
                            Fluid::Air => self.air,
                            Fluid::Water => self.water,
                            Fluid::Lava => self.cave_lava,
                            Fluid::Solid => self.base_stone_for_y(plan.lx, y, plan.lz, pos),
                        }
                    };
                    let _ = chunk.set_block(plan.lx, y, plan.lz, state);
                }
            } else {
                self.fill_column(&mut chunk, plan);
            }
            // Heightmap value: Y of the first air cell above the
            // Heightmaps store offsets from this dimension's minimum Y.
            let Some(world_surface) = heightmap_value_for_top(self.geometry, plan.top_non_air)
            else {
                continue;
            };
            let Some(motion_blocking) = heightmap_value_for_top(self.geometry, plan.height) else {
                continue;
            };
            if let Some(mb) = chunk.heightmaps.get_mut("MOTION_BLOCKING") {
                mb.set(plan.lx, plan.lz, motion_blocking);
            }
            if let Some(ws) = chunk.heightmaps.get_mut("WORLD_SURFACE") {
                ws.set(plan.lx, plan.lz, world_surface);
            }
            chunk.highest_opaque.set(plan.lx, plan.lz, motion_blocking);
        }
        #[cfg(test)]
        stage_clock.end(1);
        if self.geological_ores.is_some() {
            self.apply_caves(&mut chunk, &columns);
            #[cfg(test)]
            stage_clock.end(2);
            if let Some(geological_ores) = &self.geological_ores {
                geological_ores.apply(self, &mut chunk);
            }
        } else {
            #[cfg(test)]
            stage_clock.end(3);
            {
                let mut ore_cache = self.ore_column_cache(&chunk, &columns, &mut heights);
                self.apply_caves_with_cache(&mut chunk, &columns, Some(&mut ore_cache));
                #[cfg(test)]
                stage_clock.end(2);
                self.apply_ores_with_cache(&mut chunk, &mut ore_cache);
            }
        }
        #[cfg(test)]
        stage_clock.end(3);
        self.assign_biomes(&mut chunk, &columns);
        #[cfg(test)]
        stage_clock.end(6);
        if let Some(plans) = village.as_ref() {
            {
                self.apply_village_pieces(&mut chunk, plans, &mut heights);
            }
        }
        self.apply_jigsaw_structures(&mut chunk);
        #[cfg(test)]
        stage_clock.end(4);
        {
            self.apply_decorations(&mut chunk, &columns, &mut heights, village.as_ref());
        }
        #[cfg(test)]
        stage_clock.end(7);
        if let Some(structures) = &self.overworld_structures {
            let heights_cell = std::cell::RefCell::new(&mut heights);
            let touched = structures.paste(
                &mut chunk,
                |x, z| heights_cell.borrow_mut().structure_surface(x, z),
                |x, z| heights_cell.borrow_mut().adjusted_height(x, z),
                |x, y, z| self.noise_biome(x, y, z),
            );
            for (index, changed) in touched.into_iter().enumerate() {
                if changed {
                    self.refresh_structure_column(
                        &mut chunk,
                        (index % 16) as u8,
                        (index / 16) as u8,
                    );
                }
            }
        }
        #[cfg(test)]
        stage_clock.end(8);
        chunk.status = "minecraft:full".into();
        chunk.mark_dirty();
        #[cfg(test)]
        stage_clock.end(5);
        chunk
    }
}
