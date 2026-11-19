//! Column plans, base layers, surface materials and biome sections.

use super::*;

/// Number of dirt cells between grass cap and stone.
const DIRT_DEPTH: i32 = 3;

const DEEPSLATE_TOP_Y: i32 = 8;
const DEEPSLATE_SOLID_Y: i32 = 0;

/// Palette index of `biome` in a biome section, appended on first use.
fn palette_slot(palette: &mut Vec<Identifier>, biome: Identifier) -> usize {
    palette
        .iter()
        .position(|entry| entry == &biome)
        .unwrap_or_else(|| {
            palette.push(biome);
            palette.len() - 1
        })
}

impl TerrainGenerator {
    pub(super) fn plan_column(
        &self,
        pos: ChunkPos,
        lx: u8,
        lz: u8,
        plans: Option<&VillagePlanSet>,
    ) -> ColumnPlan {
        let wx = world_block_coordinate(pos.x, lx);
        let wz = world_block_coordinate(pos.z, lz);
        let diagnostic = self.cached_diagnostic_sample(wx, wz);
        self.plan_column_from_diagnostic(pos, lx, lz, plans, diagnostic)
    }

    pub(super) fn plan_column_from_diagnostic(
        &self,
        pos: ChunkPos,
        lx: u8,
        lz: u8,
        plans: Option<&VillagePlanSet>,
        diagnostic: TerrainDiagnosticSample,
    ) -> ColumnPlan {
        let wx = world_block_coordinate(pos.x, lx);
        let wz = world_block_coordinate(pos.z, lz);
        let height = self.village_surface_y(wx, wz, diagnostic.surface_y, plans);
        let biome = diagnostic.biome;
        let vegetation_density = diagnostic.vegetation_density.unwrap_or(-1.0);
        let (sea_level, water_enabled) = (SEA_LEVEL, true);
        let (mut surface, mut fill) = self.surface_materials(&biome);
        if self.biomes.mountain.contains(&biome) && height >= sea_level + 112 {
            surface = self.snow_block;
            fill = self.stone;
        }
        // Vanilla river beds are sand with gravel bars; without them the bed
        // reads as one flat sand sheet.
        if self.biomes.is_river(&biome) && self.river_gravel_bar(wx, wz) {
            surface = self.gravel;
            fill = self.gravel;
        }
        let top_non_air = if self.vanilla().is_some() {
            height.max(sea_level - 1).min(self.geometry.max_y() - 1)
        } else if water_enabled && (height < sea_level || self.biomes.is_river(&biome)) {
            let inclusive_top = checked_y_offset(self.geometry.max_y(), -1).unwrap_or(height);
            sea_level.clamp(height, inclusive_top)
        } else {
            height
        };
        let minimum_fill_y =
            checked_y_offset(self.geometry.min_y(), 1).unwrap_or(self.geometry.min_y());
        ColumnPlan {
            lx,
            lz,
            wx,
            wz,
            height,
            top_non_air,
            dirt_start: checked_y_offset(height, -DIRT_DEPTH)
                .unwrap_or(minimum_fill_y)
                .max(minimum_fill_y),
            biome,
            surface,
            fill,
            vegetation_density,
            hash: feature_hash(self.seed, wx, height, wz, 0xDEC0_0001),
        }
    }

    pub(super) fn fill_column(&self, chunk: &mut Chunk, plan: &ColumnPlan) {
        let min_y = self.geometry.min_y();
        let _ = chunk.set_block(plan.lx, min_y, plan.lz, self.bedrock);
        let Some(first_fill_y) = checked_y_offset(min_y, 1) else {
            return;
        };
        for y in first_fill_y..plan.dirt_start {
            let _ = chunk.set_block(
                plan.lx,
                y,
                plan.lz,
                self.base_stone_for_y(plan.lx, y, plan.lz, chunk.pos),
            );
        }
        for y in plan.dirt_start..plan.height {
            let _ = chunk.set_block(plan.lx, y, plan.lz, plan.fill);
        }
        let _ = chunk.set_block(plan.lx, plan.height, plan.lz, plan.surface);
        if plan.top_non_air > plan.height {
            let Some(first_water_y) = checked_y_offset(plan.height, 1) else {
                return;
            };
            for y in first_water_y..=plan.top_non_air {
                let _ = chunk.set_block(plan.lx, y, plan.lz, self.water);
            }
        }
    }

    pub(super) fn surface_materials(&self, biome: &Identifier) -> (BlockStateId, BlockStateId) {
        let path = biome.path();
        if path == "stony_shore" {
            return (self.gravel, self.stone);
        }
        if self.biomes.is_surface_water(biome) || self.biomes.is_beach_or_shore(biome) {
            return (self.sand, self.sand);
        }
        if path.contains("badlands") {
            return (self.red_sand, self.terracotta_layers[0]);
        }
        if path == "desert" {
            return (self.sand, self.sand);
        }
        if path == "mangrove_swamp"
            && let Some(mud) = self.decorations.mud
        {
            return (mud, self.dirt);
        }
        if self.biomes.mountain.contains(biome) || path.contains("stony") {
            return (self.gravel, self.stone);
        }
        if self.biomes.cold.contains(biome) || path.contains("snow") || path.contains("frozen") {
            return (self.snow_block, self.dirt);
        }
        if self.biomes.jungle.contains(biome) {
            return (self.grass_block, self.dirt);
        }
        if path.contains("taiga") {
            return (self.podzol, self.dirt);
        }
        (self.grass_block, self.dirt)
    }

    pub(super) fn assign_biomes(&self, chunk: &mut Chunk, columns: &[ColumnPlan; 256]) {
        let quart_ys = chunk.biomes.len() * BIOME_DIM;
        let quart_y =
            |q: usize| i32::try_from(i64::from(self.geometry.min_y()) + q as i64 * 4 + 2).ok();
        // Each quart column's whole height in one pass, so the column
        // sampler computes its Y-free climate fields once per chunk instead
        // of once per section. Indexed `(cz * 4 + cx) * quart_ys + q`.
        let sampled = self.vanilla().map(|(climate, _)| {
            let mut sampler = climate.column_sampler();
            let mut sampled = Vec::with_capacity(BIOME_DIM * BIOME_DIM * quart_ys);
            for column in 0..BIOME_DIM * BIOME_DIM {
                let (cz, cx) = (column / BIOME_DIM, column % BIOME_DIM);
                let plan = &columns[(cz * 4 + 2) * 16 + cx * 4 + 2];
                for q in 0..quart_ys {
                    sampled.push(quart_y(q).map(|y| sampler.sample(plan.wx, y, plan.wz).biome));
                }
            }
            sampled
        });
        for (section_idx, section) in chunk.biomes.iter_mut().enumerate() {
            let mut palette: Vec<Identifier> = Vec::with_capacity(4);
            // Raw biome names already resolved to a palette slot.
            let mut named: Vec<(&str, usize)> = Vec::with_capacity(4);
            let mut indices = PackedBitArray::zeroed(6, BIOME_VOLUME);
            for cy in 0..BIOME_DIM {
                let q = section_idx * BIOME_DIM + cy;
                let Some(y) = quart_y(q) else {
                    continue;
                };
                for cz in 0..BIOME_DIM {
                    for cx in 0..BIOME_DIM {
                        let lx = cx * 4 + 2;
                        let lz = cz * 4 + 2;
                        let column = &columns[lz * 16 + lx];
                        let palette_idx = if let Some(sampled) = &sampled {
                            let name = sampled[(cz * BIOME_DIM + cx) * quart_ys + q]
                                .expect("same checked biome Y");
                            match named.iter().find(|(seen, _)| *seen == name) {
                                Some(&(_, slot)) => slot,
                                None => {
                                    let biome = Identifier::parse(name)
                                        .expect("validated vanilla biome identifier");
                                    let slot = palette_slot(&mut palette, biome);
                                    named.push((name, slot));
                                    slot
                                }
                            }
                        } else if i64::from(y) < i64::from(column.height) - 24 && y < 32 {
                            palette_slot(
                                &mut palette,
                                self.biomes.pick(
                                    &self.biomes.cave,
                                    column.wx,
                                    column.wz,
                                    self.seed,
                                    0x554E_4447,
                                ),
                            )
                        } else {
                            palette_slot(&mut palette, column.biome.clone())
                        };
                        let idx = (cy * BIOME_DIM + cz) * BIOME_DIM + cx;
                        indices.set(idx, palette_idx as u32);
                    }
                }
            }
            if palette.len() == 1 {
                *section = BiomeSection::filled(palette.pop().unwrap());
            } else {
                *section = BiomeSection::from_indirect(palette, indices);
            }
        }
    }

    pub(super) fn refresh_known_top_column(&self, chunk: &mut Chunk, lx: u8, lz: u8, top: i32) {
        let current = chunk
            .heightmaps
            .get("MOTION_BLOCKING")
            .map(|heightmap| {
                i64::from(heightmap.get(lx, lz)) + i64::from(self.geometry.min_y()) - 1
            })
            .unwrap_or(i64::from(self.geometry.min_y()));
        let top = i64::from(top).max(current);
        let Ok(top) = i32::try_from(top) else {
            return;
        };
        let Some(value) = heightmap_value_for_top(self.geometry, top) else {
            return;
        };
        if let Some(mb) = chunk.heightmaps.get_mut("MOTION_BLOCKING") {
            mb.set(lx, lz, value);
        }
        if let Some(ws) = chunk.heightmaps.get_mut("WORLD_SURFACE") {
            ws.set(lx, lz, value);
        }
        chunk.highest_opaque.set(lx, lz, value);
    }

    pub(super) fn refresh_structure_column(&self, chunk: &mut Chunk, lx: u8, lz: u8) {
        let top = (self.geometry.min_y()..self.geometry.max_y())
            .rev()
            .find(|&y| {
                chunk
                    .get_block(lx, y, lz)
                    .is_some_and(|state| state != self.air)
            })
            .unwrap_or(self.geometry.min_y());
        let Some(value) = heightmap_value_for_top(self.geometry, top) else {
            return;
        };
        if let Some(mb) = chunk.heightmaps.get_mut("MOTION_BLOCKING") {
            mb.set(lx, lz, value);
        }
        if let Some(ws) = chunk.heightmaps.get_mut("WORLD_SURFACE") {
            ws.set(lx, lz, value);
        }
        chunk.highest_opaque.set(lx, lz, value);
    }

    pub(super) fn base_stone_for_y(&self, lx: u8, y: i32, lz: u8, pos: ChunkPos) -> BlockStateId {
        if y <= DEEPSLATE_SOLID_Y {
            return self.deepslate;
        }
        if y >= DEEPSLATE_TOP_Y {
            return self.stone;
        }
        let wx = world_block_coordinate(pos.x, lx);
        let wz = world_block_coordinate(pos.z, lz);
        // Vanilla's vertical_gradient is certain at Y=0 and zero at Y=8.
        // Keep the existing coordinate hash, but use a linear probability.
        let deepslate_chance = (DEEPSLATE_TOP_Y - y) as u64;
        let span = (DEEPSLATE_TOP_Y - DEEPSLATE_SOLID_Y) as u64;
        if feature_hash(self.seed, wx, y, wz, 0xD33F) % span < deepslate_chance {
            self.deepslate
        } else {
            self.stone
        }
    }
}
