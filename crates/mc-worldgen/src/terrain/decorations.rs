//! Tree, plant, cactus, sugar-cane and kelp placement.

use super::height::HeightContext;

use super::*;

/// Columns per plains tree outside groves: vanilla `trees_plains` places one
/// tree with 5% chance per chunk (256 columns / 0.05).
const GRASSLAND_SPARSE_TREE_SPACING: u64 = 5120;

impl TerrainGenerator {
    pub(super) fn apply_decorations(
        &self,
        chunk: &mut Chunk,
        columns: &[ColumnPlan; 256],
        heights: &mut HeightContext<'_>,
        village: Option<&VillagePlanSet>,
    ) {
        let mut touched = [None; 256];
        let mut leaves = Vec::new();
        for lz in 0..16u8 {
            for lx in 0..16u8 {
                let idx = lz as usize * 16 + lx as usize;
                let plan = &columns[idx];
                let height = plan.height;
                let Some(decoration_limit) = checked_y_offset(height, 8) else {
                    continue;
                };
                let Some(base_y) = checked_y_offset(height, 1) else {
                    continue;
                };
                if height <= self.geometry.min_y() || decoration_limit >= self.geometry.max_y() {
                    continue;
                }
                let biome = &plan.biome;
                let surface = plan.surface;
                let h = plan.hash;

                if chunk.get_block(lx, height, lz) != Some(surface) {
                    continue;
                }

                let tree_spacing = self.tree_spacing_for_biome(biome);
                let base_tree = self
                    .base_tree_spacing(biome)
                    .is_some_and(|spacing| h.is_multiple_of(spacing));
                if (base_tree
                    || (tree_spacing.is_some_and(|spacing| h.is_multiple_of(spacing))
                        && self.tree_density_allows(plan)))
                    && self.tree_site_is_stable(plan, heights)
                    && self.place_tree(
                        chunk,
                        plan,
                        self.tree_blocks_for_biome(biome),
                        &mut touched,
                        &mut leaves,
                        village,
                    )
                {
                    continue;
                }
                if self.biomes.hot_dry.contains(biome)
                    && surface == self.sand
                    && h.is_multiple_of(47)
                    && self.place_cactus(chunk, lx, base_y, lz, &mut touched)
                {
                    continue;
                }
                if (self.biomes.beach.contains(biome)
                    || self.biomes.river.contains(biome)
                    || self.biomes.swamp.contains(biome))
                    && h.is_multiple_of(29)
                    && self.place_sugar_cane(chunk, lx, base_y, lz, &mut touched)
                {
                    continue;
                }
                if (self.biomes.grassland.contains(biome)
                    || self.biomes.temperate_forest.contains(biome)
                    || self.biomes.jungle.contains(biome)
                    || self.biomes.swamp.contains(biome)
                    || Self::is_savanna(biome))
                    && (surface == self.grass_block || surface == self.podzol)
                {
                    let (grass_spacing, dandelion_spacing, poppy_spacing) =
                        self.plant_spacing_for_biome(biome);
                    let plant = if Self::is_savanna(biome) {
                        if h.is_multiple_of(grass_spacing) {
                            self.decorations.short_grass
                        } else {
                            None
                        }
                    } else if h.is_multiple_of(1021) {
                        self.decorations.pumpkin
                    } else if self.biomes.swamp.contains(biome) {
                        if h.is_multiple_of(dandelion_spacing) {
                            self.decorations.blue_orchid
                        } else if h.is_multiple_of(grass_spacing) {
                            self.decorations.short_grass
                        } else {
                            None
                        }
                    } else if h.is_multiple_of(dandelion_spacing) {
                        self.decorations.dandelion
                    } else if h.is_multiple_of(poppy_spacing) {
                        self.decorations.poppy
                    } else if h.is_multiple_of(grass_spacing) {
                        self.decorations.short_grass
                    } else {
                        None
                    };
                    if let Some(plant) = plant
                        && chunk.get_block(lx, base_y, lz) == Some(self.air)
                    {
                        self.place_single(chunk, lx, base_y, lz, plant, &mut touched);
                    }
                }
                if self.biomes.is_ocean(biome)
                    && checked_y_offset(height, 2)
                        .is_some_and(|minimum_top| plan.top_non_air > minimum_top)
                {
                    if h.is_multiple_of(31) && self.place_kelp_column(chunk, plan, h, &mut touched)
                    {
                        continue;
                    }
                    if h.is_multiple_of(17)
                        && let Some(seagrass) = self.decorations.seagrass
                        && chunk.get_block(lx, base_y, lz) == Some(self.water)
                    {
                        self.place_single(chunk, lx, base_y, lz, seagrass, &mut touched);
                    }
                }
            }
        }
        self.initialize_leaf_distances(chunk, &mut leaves);
        self.apply_sediments(chunk, columns);
        for lz in 0..16u8 {
            for lx in 0..16u8 {
                if let Some(top) = touched[lz as usize * 16 + lx as usize] {
                    self.refresh_known_top_column(chunk, lx, lz, top);
                }
            }
        }
    }

    pub(super) fn initialize_leaf_distances(
        &self,
        chunk: &mut Chunk,
        leaves: &mut Vec<(u8, i32, u8)>,
    ) {
        let palettes = &self.decorations.leaf_distances;
        let neighbours = |(x, y, z): (u8, i32, u8)| {
            [
                (-1, 0, 0),
                (1, 0, 0),
                (0, -1, 0),
                (0, 1, 0),
                (0, 0, -1),
                (0, 0, 1),
            ]
            .into_iter()
            .filter_map(move |(dx, dy, dz)| {
                let x = x.checked_add_signed(dx)?;
                let y = y.checked_add(dy)?;
                let z = z.checked_add_signed(dz)?;
                (x < 16 && z < 16).then_some((x, y, z))
            })
        };
        // Resolve the complete generated leaf graph before publishing the chunk.
        // Persistent remains false; later log removal uses ordinary scheduled decay.
        leaves.retain(|&(x, y, z)| {
            let state = chunk
                .get_block(x, y, z)
                .expect("generated leaf is in the chunk");
            let Some(palette) = palettes
                .iter()
                .flatten()
                .find(|palette| palette[6] == state)
            else {
                return false;
            };
            let supported = neighbours((x, y, z)).any(|(nx, ny, nz)| {
                chunk.get_block(nx, ny, nz).is_some_and(|state| {
                    self.decorations
                        .leaf_supports
                        .get(state.0 as usize)
                        .copied()
                        .unwrap_or(false)
                })
            });
            if supported {
                chunk.set_block(x, y, z, palette[0]);
            }
            supported
        });
        let mut start = 0;
        for distance in 2..=6 {
            let end = leaves.len();
            for index in start..end {
                for (x, y, z) in neighbours(leaves[index]) {
                    let Some(state) = chunk.get_block(x, y, z) else {
                        continue;
                    };
                    if let Some(palette) = palettes
                        .iter()
                        .flatten()
                        .find(|palette| palette[6] == state)
                    {
                        chunk.set_block(x, y, z, palette[distance - 1]);
                        leaves.push((x, y, z));
                    }
                }
            }
            start = end;
        }
    }

    pub(super) fn tree_site_is_stable(
        &self,
        plan: &ColumnPlan,
        heights: &mut HeightContext<'_>,
    ) -> bool {
        // Keep the trunk and nearby canopy footprint off sharp terrain ledges.
        // Neighbour heights come from the generation-local context — one shared
        // sampler + one plan memo — instead of 25 independent `surface_height`
        // queries that each re-run the density query and the village lookup.
        for dz in -2..=2 {
            for dx in -2..=2 {
                let Some(wx) = plan.wx.checked_add(dx) else {
                    return false;
                };
                let Some(wz) = plan.wz.checked_add(dz) else {
                    return false;
                };
                let neighbour = heights.adjusted_height(wx, wz);
                if (neighbour - plan.height).abs() > 1 {
                    return false;
                }
            }
        }
        true
    }

    pub(super) fn is_savanna(biome: &Identifier) -> bool {
        biome.path().contains("savanna")
    }

    pub(super) fn is_cold_forest(biome: &Identifier) -> bool {
        biome.path().contains("taiga") || biome.path() == "grove"
    }

    pub(super) fn tree_density_allows(&self, plan: &ColumnPlan) -> bool {
        if let Some(rule) = self.tree_rules.get(&plan.biome) {
            return plan.vegetation_density >= rule.density_threshold;
        }
        let threshold = if self.biomes.jungle.contains(&plan.biome) {
            -0.55
        } else if self.biomes.temperate_forest.contains(&plan.biome) {
            -0.25
        } else if self.biomes.swamp.contains(&plan.biome) {
            // Swamp trees root in wet soil wherever the tightened swamp band
            // applies; previously swamp had no branch here at all, which made
            // every swamp column barren by construction.
            -0.45
        } else if Self::is_cold_forest(&plan.biome) || Self::is_savanna(&plan.biome) {
            -0.05
        } else if self.biomes.grassland.contains(&plan.biome) {
            0.0
        } else {
            return false;
        };
        plan.vegetation_density >= threshold
    }

    /// Columns per tree a biome carries everywhere, before the vegetation
    /// field's denser groves (vanilla vegetation is per biome, never barren
    /// regions): plains `trees_plains` one tree in ~20 chunks, savanna
    /// `trees_savanna` ~1.1 per chunk, swamp 2 per chunk; forests, taiga and
    /// jungle keep their full spacing everywhere.
    fn base_tree_spacing(&self, biome: &Identifier) -> Option<u64> {
        if self.biomes.grassland.contains(biome) {
            Some(GRASSLAND_SPARSE_TREE_SPACING)
        } else if Self::is_savanna(biome) {
            Some(233)
        } else if self.biomes.swamp.contains(biome) {
            Some(127)
        } else if self.biomes.jungle.contains(biome)
            || self.biomes.temperate_forest.contains(biome)
            || Self::is_cold_forest(biome)
        {
            self.tree_spacing_for_biome(biome)
        } else {
            None
        }
    }

    pub(super) fn tree_spacing_for_biome(&self, biome: &Identifier) -> Option<u64> {
        if let Some(rule) = self.tree_rules.get(biome) {
            return Some(rule.spacing);
        }
        if self.biomes.jungle.contains(biome) {
            // Vanilla jungle vegetation makes about 50 attempts per chunk,
            // mixing trees with a low bush layer rather than sparse trunks.
            Some(5)
        } else if self.biomes.temperate_forest.contains(biome) {
            Some(37)
        } else if self.biomes.swamp.contains(biome) {
            Some(31)
        } else if Self::is_cold_forest(biome) {
            Some(47)
        } else if Self::is_savanna(biome) {
            Some(113)
        } else if self.biomes.grassland.contains(biome) {
            Some(173)
        } else {
            None
        }
    }

    fn plant_spacing_for_biome(&self, biome: &Identifier) -> (u64, u64, u64) {
        if Self::is_savanna(biome) {
            (31, 127, 137)
        } else if self.biomes.jungle.contains(biome) {
            (13, 89, 97)
        } else if self.biomes.swamp.contains(biome) {
            (15, 71, 79)
        } else if self.biomes.temperate_forest.contains(biome) {
            (29, 127, 137)
        } else {
            // Common lowlands at 1/19 grass read as featureless lawns from
            // any distance; 1/15 keeps the existing moderate-cover test cap.
            (15, 61, 67)
        }
    }

    fn place_cactus(
        &self,
        chunk: &mut Chunk,
        lx: u8,
        base_y: i32,
        lz: u8,
        touched: &mut [Option<i32>; 256],
    ) -> bool {
        let Some(cactus) = self.decorations.cactus else {
            return false;
        };
        let height =
            1 + (feature_hash(self.seed, lx as i32, base_y, lz as i32, 0xCA_C7) % 3) as i32;
        let Some(top_exclusive) = checked_y_offset(base_y, height) else {
            return false;
        };
        for y in base_y..top_exclusive {
            if chunk.get_block(lx, y, lz) != Some(self.air) {
                return false;
            }
        }
        for y in base_y..top_exclusive {
            self.place_single(chunk, lx, y, lz, cactus, touched);
        }
        true
    }

    fn place_sugar_cane(
        &self,
        chunk: &mut Chunk,
        lx: u8,
        base_y: i32,
        lz: u8,
        touched: &mut [Option<i32>; 256],
    ) -> bool {
        let Some(sugar_cane) = self.decorations.sugar_cane else {
            return false;
        };
        let Some(below_y) = checked_y_offset(base_y, -1) else {
            return false;
        };
        if chunk.get_block(lx, base_y, lz) != Some(self.air)
            || !self.has_adjacent_water(chunk, lx, below_y, lz)
        {
            return false;
        }
        self.place_single(chunk, lx, base_y, lz, sugar_cane, touched);
        if let Some(above_y) = checked_y_offset(base_y, 1)
            && chunk.get_block(lx, above_y, lz) == Some(self.air)
        {
            self.place_single(chunk, lx, above_y, lz, sugar_cane, touched);
        }
        true
    }

    fn has_adjacent_water(&self, chunk: &Chunk, lx: u8, y: i32, lz: u8) -> bool {
        [(1i8, 0i8), (-1, 0), (0, 1), (0, -1)]
            .into_iter()
            .any(|(dx, dz)| {
                let x = lx as i8 + dx;
                let z = lz as i8 + dz;
                if !(0..16).contains(&x) || !(0..16).contains(&z) {
                    return false;
                }
                let x = x as u8;
                let z = z as u8;
                chunk.get_block(x, y, z) == Some(self.water)
                    || checked_y_offset(y, 1).and_then(|above_y| chunk.get_block(x, above_y, z))
                        == Some(self.water)
            })
    }

    fn place_kelp_column(
        &self,
        chunk: &mut Chunk,
        plan: &ColumnPlan,
        hash: u64,
        touched: &mut [Option<i32>; 256],
    ) -> bool {
        let lx = plan.lx;
        let lz = plan.lz;
        let Some(base_y) = checked_y_offset(plan.height, 1) else {
            return false;
        };
        let water_top = plan.top_non_air;
        let Some(kelp) = self.decorations.kelp else {
            return false;
        };
        let stem = self.decorations.kelp_plant.unwrap_or(kelp);
        let available = i64::from(water_top) - i64::from(base_y) + 1;
        if available < 2 {
            return false;
        }
        let height = i64::from(2 + (hash % 5) as i32).min(available);
        let Ok(top_y) = i32::try_from(i64::from(base_y) + height - 1) else {
            return false;
        };
        for y in base_y..=top_y {
            if chunk.get_block(lx, y, lz) != Some(self.water) {
                return false;
            }
        }
        for y in base_y..top_y {
            self.place_single(chunk, lx, y, lz, stem, touched);
        }
        self.place_single(chunk, lx, top_y, lz, kelp, touched);
        true
    }

    pub(super) fn place_single(
        &self,
        chunk: &mut Chunk,
        lx: u8,
        y: i32,
        lz: u8,
        state: BlockStateId,
        touched: &mut [Option<i32>; 256],
    ) {
        if chunk.set_block(lx, y, lz, state).is_some() {
            let touched = &mut touched[lz as usize * 16 + lx as usize];
            *touched = Some(touched.map_or(y, |top| top.max(y)));
        }
    }
}
