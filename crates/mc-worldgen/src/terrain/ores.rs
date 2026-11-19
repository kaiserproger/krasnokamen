//! Ore placement: column cache, anchor cells, veins and exposure culling.

use super::height::HeightContext;

use super::*;

pub(super) struct OreColumnCache<'a, 'h> {
    min_x: i64,
    min_z: i64,
    side: usize,
    columns: Vec<Option<ColumnPlan>>,
    /// The generation-local height context: ONE shared native sampler plus
    /// the village-plan memo, owned by the caller. The previous
    /// `native_heights: Option<SurfaceHeightSampler>` + `plans` fields lived
    /// here; they moved into the context so tree, ore and structure readers
    /// share the same sampler and the same plan lookup instead of each
    /// building their own.
    heights: &'a mut HeightContext<'h>,
    cave_min_y: i32,
    cave_layers: usize,
    cave_samples: Vec<u8>,
    raw_cave_samples: Vec<u8>,
    /// Full-density cave answers outside the sampled layers, per cell: ore
    /// veins and their air-exposure scans ask the same cells many times.
    dense_caves: std::collections::HashMap<(i32, i32, i32), bool>,
}

impl OreColumnCache<'_, '_> {
    fn column_index(&self, world_x: i32, world_z: i32) -> Option<usize> {
        let local_x = i64::from(world_x).checked_sub(self.min_x)?;
        let local_z = i64::from(world_z).checked_sub(self.min_z)?;
        let local_x = usize::try_from(local_x).ok()?;
        let local_z = usize::try_from(local_z).ok()?;
        if local_x >= self.side || local_z >= self.side {
            return None;
        }
        Some(local_z * self.side + local_x)
    }

    fn get_or_plan(&mut self, world_x: i32, world_z: i32) -> Option<&ColumnPlan> {
        let index = self.column_index(world_x, world_z)?;
        if self.columns[index].is_none() {
            // The halo reaches into neighbouring chunks, so the plan is the
            // column's own chunk's set — never this chunk's, and never a
            // single column's filtered slice of it (`plan_column` applies the
            // set per column anyway). The context memoizes the lookup per
            // chunk and serves SOURCE heights from one shared sampler, so
            // halo columns cost one plan lookup per chunk and one sampler
            // query per column however many ore cells read them.
            let column = self.heights.halo_column(world_x, world_z);
            self.columns[index] = Some(column);
        }
        self.columns[index].as_ref()
    }

    fn cave_sample_index(&self, world_x: i32, sample_y: i32, world_z: i32) -> Option<usize> {
        let column_index = self.column_index(world_x, world_z)?;
        let delta_y = sample_y.saturating_sub(self.cave_min_y);
        if delta_y < 0 || delta_y % CAVE_VERTICAL_SAMPLE_STEP != 0 {
            return None;
        }
        let layer = usize::try_from(delta_y / CAVE_VERTICAL_SAMPLE_STEP).ok()?;
        if layer >= self.cave_layers {
            return None;
        }
        Some(layer * self.side * self.side + column_index)
    }

    pub(super) fn record_raw_cave(
        &mut self,
        world_x: i32,
        sample_y: i32,
        world_z: i32,
        cave: bool,
    ) {
        if let Some(index) = self.cave_sample_index(world_x, sample_y, world_z) {
            self.raw_cave_samples[index] = if cave { 2 } else { 1 };
        }
    }

    fn get_or_raw_cave(
        &mut self,
        generator: &TerrainGenerator,
        world_x: i32,
        sample_y: i32,
        world_z: i32,
    ) -> bool {
        let Some(index) = self.cave_sample_index(world_x, sample_y, world_z) else {
            return generator
                .density_router()
                .raw_cave(world_x, sample_y, world_z);
        };
        match self.raw_cave_samples[index] {
            1 => false,
            2 => true,
            _ => {
                let cave = generator
                    .density_router()
                    .raw_cave(world_x, sample_y, world_z);
                self.raw_cave_samples[index] = if cave { 2 } else { 1 };
                cave
            }
        }
    }

    fn get_or_cave(
        &mut self,
        generator: &TerrainGenerator,
        world_x: i32,
        sample_y: i32,
        world_z: i32,
        surface_y: i32,
    ) -> bool {
        let Some(index) = self.cave_sample_index(world_x, sample_y, world_z) else {
            // Above the sampled cave layers: the full-density test, through
            // the halo sampler's shared lattice corners (same values).
            let heights = &mut *self.heights;
            return *self
                .dense_caves
                .entry((world_x, sample_y, world_z))
                .or_insert_with(|| {
                    sample_y < surface_y
                        && heights
                            .native_empty(world_x, sample_y, world_z)
                            .unwrap_or_else(|| {
                                generator.is_cave_cell(world_x, sample_y, world_z, surface_y)
                            })
                });
        };
        match self.cave_samples[index] {
            1 => false,
            2 => true,
            _ => {
                let cave = sample_y > generator.geometry.min_y().saturating_add(10)
                    && sample_y < surface_y.saturating_sub(CAVE_SURFACE_CLEARANCE).min(32)
                    && self.get_or_raw_cave(generator, world_x, sample_y, world_z)
                    && [(-1, 0), (1, 0), (0, -1), (0, 1)]
                        .into_iter()
                        .any(|(dx, dz)| {
                            world_x
                                .checked_add(dx)
                                .zip(world_z.checked_add(dz))
                                .is_some_and(|(x, z)| {
                                    self.get_or_raw_cave(generator, x, sample_y, z)
                                })
                        });
                self.cave_samples[index] = if cave { 2 } else { 1 };
                cave
            }
        }
    }
}

impl TerrainGenerator {
    pub(super) fn ore_column_cache<'a, 'h>(
        &self,
        chunk: &Chunk,
        chunk_columns: &[ColumnPlan; 256],
        heights: &'a mut HeightContext<'h>,
    ) -> OreColumnCache<'a, 'h> {
        let min_x_i64 = i64::from(chunk.pos.x) * 16 - i64::from(ORE_COLUMN_HALO);
        let min_z_i64 = i64::from(chunk.pos.z) * 16 - i64::from(ORE_COLUMN_HALO);
        let side = usize::try_from(16 + ORE_COLUMN_HALO * 2)
            .expect("bounded ore column cache side fits usize");
        let min_x = min_x_i64;
        let min_z = min_z_i64;
        let cave_min_y = self.geometry.min_y().saturating_add(8);
        let cave_max_y = self.geometry.max_y().saturating_sub(1).min(31);
        let cave_layers = if cave_max_y < cave_min_y {
            0
        } else {
            usize::try_from((cave_max_y - cave_min_y) / CAVE_VERTICAL_SAMPLE_STEP + 1)
                .expect("bounded cave layer count fits usize")
        };
        let mut cache = OreColumnCache {
            min_x,
            min_z,
            side,
            columns: vec![None; side * side],
            heights,
            cave_min_y,
            cave_layers,
            cave_samples: vec![0; side * side * cave_layers],
            raw_cave_samples: vec![0; side * side * cave_layers],
            dense_caves: std::collections::HashMap::new(),
        };
        let center_offset = usize::try_from(ORE_COLUMN_HALO).expect("ore halo fits usize");
        for plan in chunk_columns {
            let local_x = center_offset + usize::from(plan.lx);
            let local_z = center_offset + usize::from(plan.lz);
            cache.columns[local_z * side + local_x] = Some(plan.clone());
        }
        cache
    }

    pub(super) fn apply_ores_with_cache(
        &self,
        chunk: &mut Chunk,
        column_cache: &mut OreColumnCache<'_, '_>,
    ) {
        debug_assert!(self.geological_ores.is_none());
        let chunk_min_x = i64::from(chunk.pos.x) * 16;
        let chunk_min_z = i64::from(chunk.pos.z) * 16;
        // Re-evaluate nearby anchors so a vein crossing a chunk edge is derived
        // identically no matter which side is generated first.
        let radius = i64::from(ORE_VEIN_RADIUS);
        let cell_edge = i64::from(ORE_ANCHOR_CELL_EDGE);
        let min_cell_x = (chunk_min_x - radius).div_euclid(cell_edge);
        let max_cell_x = (chunk_min_x + 15 + radius).div_euclid(cell_edge);
        let min_cell_z = (chunk_min_z - radius).div_euclid(cell_edge);
        let max_cell_z = (chunk_min_z + 15 + radius).div_euclid(cell_edge);

        for (rule_index, rule) in self.ores.rules().iter().enumerate() {
            let Some(first_stone_y) = checked_y_offset(self.geometry.min_y(), 1) else {
                continue;
            };
            let Some(inclusive_top) = checked_y_offset(self.geometry.max_y(), -1) else {
                continue;
            };
            let min_y = rule.y.min.max(first_stone_y);
            let max_y = rule.y.max.min(inclusive_top);
            if min_y > max_y {
                continue;
            }
            let vein_size = usize::try_from(rule.size)
                .unwrap_or(MAX_ORE_VEIN_SIZE)
                .clamp(1, MAX_ORE_VEIN_SIZE);
            let min_cell_y = min_y.div_euclid(ORE_ANCHOR_CELL_EDGE);
            let max_cell_y = max_y.div_euclid(ORE_ANCHOR_CELL_EDGE);
            let rule_salt = 0x0AE0_0000_u64 ^ rule_index as u64;

            for cell_y in min_cell_y..=max_cell_y {
                let sample_y = i64::from(cell_y) * i64::from(ORE_ANCHOR_CELL_EDGE)
                    + i64::from(ORE_ANCHOR_CELL_EDGE / 2);
                let Ok(sample_y) = i32::try_from(sample_y) else {
                    continue;
                };
                let spacing = rule.spacing.at_y(sample_y, rule.y).max(1);
                let denominator = spacing.saturating_mul(vein_size as u64).max(1);
                for cell_z in min_cell_z..=max_cell_z {
                    for cell_x in min_cell_x..=max_cell_x {
                        let cell_x_i32 =
                            i32::try_from(cell_x).expect("valid chunk halo cell x fits i32");
                        let cell_z_i32 =
                            i32::try_from(cell_z).expect("valid chunk halo cell z fits i32");
                        let cell_hash =
                            feature_hash(self.seed, cell_x_i32, cell_y, cell_z_i32, rule_salt);
                        let guaranteed = ORE_ANCHOR_CELL_VOLUME / denominator;
                        let remainder = ORE_ANCHOR_CELL_VOLUME % denominator;
                        let extra = usize::from(
                            remainder != 0 && cell_hash.rotate_left(29) % denominator < remainder,
                        );
                        let anchor_count = usize::try_from(guaranteed)
                            .unwrap_or(MAX_ORE_ANCHORS_PER_CELL)
                            .saturating_add(extra)
                            .min(MAX_ORE_ANCHORS_PER_CELL);
                        if anchor_count == 0 {
                            continue;
                        }

                        let start = (cell_hash & 63) as usize;
                        let step = (((cell_hash >> 6) & 31) as usize) * 2 + 1;
                        for slot in 0..anchor_count {
                            let cell_index = (start + slot * step) & 63;
                            let anchor_x = cell_x * cell_edge + (cell_index & 3) as i64;
                            let anchor_z = cell_z * cell_edge + ((cell_index >> 2) & 3) as i64;
                            let anchor_y =
                                i64::from(cell_y) * cell_edge + ((cell_index >> 4) & 3) as i64;
                            let (Ok(anchor_x), Ok(anchor_y), Ok(anchor_z)) = (
                                i32::try_from(anchor_x),
                                i32::try_from(anchor_y),
                                i32::try_from(anchor_z),
                            ) else {
                                continue;
                            };
                            if !(min_y..=max_y).contains(&anchor_y) {
                                continue;
                            }
                            if !rule.biomes.is_any() {
                                let matches = if let Some((climate, _)) = self.vanilla() {
                                    rule.biomes.matches_name(
                                        climate.sample(anchor_x, anchor_y, anchor_z).biome,
                                    )
                                } else {
                                    column_cache
                                        .get_or_plan(anchor_x, anchor_z)
                                        .is_some_and(|column| rule.biomes.matches(&column.biome))
                                };
                                if !matches {
                                    continue;
                                }
                            }
                            let vein_hash = feature_hash(
                                self.seed,
                                anchor_x,
                                anchor_y,
                                anchor_z,
                                rule_salt ^ slot as u64,
                            );
                            self.place_ore_vein(
                                chunk,
                                column_cache,
                                rule,
                                [anchor_x, anchor_y, anchor_z],
                                vein_hash,
                                vein_size,
                            );
                        }
                    }
                }
            }
        }
    }

    pub(super) fn place_ore_vein(
        &self,
        chunk: &mut Chunk,
        column_cache: &mut OreColumnCache<'_, '_>,
        rule: &OreRule,
        anchor: [i32; 3],
        vein_hash: u64,
        vein_size: usize,
    ) {
        let mut offsets = [[0_i8; 3]; MAX_ORE_VEIN_SIZE];
        let offset_count = ore_veins::ore_offsets(&mut offsets, vein_hash, vein_size);
        let min_x = i64::from(chunk.pos.x) * 16;
        let min_z = i64::from(chunk.pos.z) * 16;
        for (index, &offset) in offsets[..offset_count].iter().enumerate() {
            // Off-chunk cells can never be written here. Reject them before
            // reconstructing their density and exposure halo; the remaining
            // in-chunk cells still query cross-boundary neighbours normally.
            let x = i64::from(anchor[0]) + i64::from(offset[0]);
            let z = i64::from(anchor[2]) + i64::from(offset[2]);
            if !(min_x..min_x + 16).contains(&x) || !(min_z..min_z + 16).contains(&z) {
                continue;
            }
            if self.ore_cell_can_generate(
                column_cache,
                rule,
                anchor,
                offset,
                vein_hash ^ index as u64,
            ) {
                self.place_ore_cell(chunk, rule, anchor, offset);
            }
        }
    }

    pub(super) fn ore_cell_can_generate(
        &self,
        column_cache: &mut OreColumnCache<'_, '_>,
        rule: &OreRule,
        anchor: [i32; 3],
        offset: [i8; 3],
        cell_hash: u64,
    ) -> bool {
        let Some(world_x) = anchor[0].checked_add(i32::from(offset[0])) else {
            return false;
        };
        let Some(world_y) = anchor[1].checked_add(i32::from(offset[1])) else {
            return false;
        };
        let Some(world_z) = anchor[2].checked_add(i32::from(offset[2])) else {
            return false;
        };
        if !(rule.y.min..=rule.y.max).contains(&world_y) {
            return false;
        }
        let Some(base) = self.generated_cell_before_ores(column_cache, world_x, world_y, world_z)
        else {
            return false;
        };
        if base != self.stone && base != self.deepslate {
            return false;
        }
        !self.should_discard_exposed_ore(column_cache, world_x, world_y, world_z, rule, cell_hash)
    }

    pub(super) fn place_ore_cell(
        &self,
        chunk: &mut Chunk,
        rule: &OreRule,
        anchor: [i32; 3],
        offset: [i8; 3],
    ) {
        let world_x = i64::from(anchor[0]) + i64::from(offset[0]);
        let Some(world_y) = anchor[1].checked_add(i32::from(offset[1])) else {
            return;
        };
        let world_z = i64::from(anchor[2]) + i64::from(offset[2]);
        let chunk_min_x = i64::from(chunk.pos.x) * 16;
        let chunk_min_z = i64::from(chunk.pos.z) * 16;
        if !(chunk_min_x..chunk_min_x + 16).contains(&world_x)
            || !(chunk_min_z..chunk_min_z + 16).contains(&world_z)
        {
            return;
        }
        let lx = (world_x - chunk_min_x) as u8;
        let lz = (world_z - chunk_min_z) as u8;
        let Some(base) = chunk.get_block(lx, world_y, lz) else {
            return;
        };
        if base != self.stone && base != self.deepslate {
            return;
        }
        let ore = self.ore_variant(base, rule.normal, rule.deepslate);
        let _ = chunk.set_block(lx, world_y, lz, ore);
    }

    fn should_discard_exposed_ore(
        &self,
        column_cache: &mut OreColumnCache<'_, '_>,
        world_x: i32,
        y: i32,
        world_z: i32,
        rule: &OreRule,
        hash: u64,
    ) -> bool {
        // Cheap gates first: NaN/zero chance never discards, so the six-cell
        // air scan below must not run at all for those rules — its result is
        // unread and each cell would plan a halo column for nothing.
        let chance = rule.discard_chance_on_air_exposure;
        if !chance.is_finite() || chance <= 0.0 {
            return false;
        }
        if chance >= 1.0 {
            return self.generated_cell_touches_air(column_cache, world_x, y, world_z);
        }
        if !self.generated_cell_touches_air(column_cache, world_x, y, world_z) {
            return false;
        }
        let sample = (hash >> 11) as f64 * (1.0 / ((1_u64 << 53) as f64));
        sample < chance
    }

    fn generated_cell_touches_air(
        &self,
        column_cache: &mut OreColumnCache<'_, '_>,
        world_x: i32,
        y: i32,
        world_z: i32,
    ) -> bool {
        ORE_DIRECTIONS.iter().any(|direction| {
            let neighbour = world_x
                .checked_add(i32::from(direction[0]))
                .zip(y.checked_add(i32::from(direction[1])))
                .zip(world_z.checked_add(i32::from(direction[2])));
            let Some(((nx, ny), nz)) = neighbour else {
                return true;
            };
            self.generated_cell_before_ores(column_cache, nx, ny, nz) == Some(self.air)
        })
    }

    fn generated_cell_before_ores(
        &self,
        column_cache: &mut OreColumnCache<'_, '_>,
        world_x: i32,
        y: i32,
        world_z: i32,
    ) -> Option<BlockStateId> {
        if y < self.geometry.min_y() || y >= self.geometry.max_y() {
            return None;
        }
        let (top_non_air, height, dirt_start, fill, surface, cave_bounds) = {
            let plan = column_cache.get_or_plan(world_x, world_z)?;
            (
                plan.top_non_air,
                plan.height,
                plan.dirt_start,
                plan.fill,
                plan.surface,
                self.cave_y_bounds(plan),
            )
        };
        let pos = ChunkPos {
            x: world_x.div_euclid(16),
            z: world_z.div_euclid(16),
        };
        if y > top_non_air {
            return Some(self.air);
        }
        if y > height {
            return Some(self.water);
        }
        if y == self.geometry.min_y() {
            return Some(self.bedrock);
        }

        if let Some((cave_min_y, cave_max_y)) = cave_bounds
            && (cave_min_y..=cave_max_y).contains(&y)
        {
            let step = i64::from(CAVE_VERTICAL_SAMPLE_STEP);
            let sample_y = i64::from(cave_min_y)
                + (i64::from(y) - i64::from(cave_min_y)).div_euclid(step) * step;
            let Ok(sample_y) = i32::try_from(sample_y) else {
                return None;
            };
            if column_cache.get_or_cave(self, world_x, sample_y, world_z, height) {
                return Some(self.air);
            }
        }
        if y < dirt_start {
            return Some(self.base_stone_for_y(
                world_x.rem_euclid(16) as u8,
                y,
                world_z.rem_euclid(16) as u8,
                pos,
            ));
        }
        if y < height {
            return Some(fill);
        }
        Some(surface)
    }

    #[cfg(test)]
    pub(super) fn ore_for(
        &self,
        x: i32,
        y: i32,
        z: i32,
        base: BlockStateId,
        biome: &Identifier,
    ) -> BlockStateId {
        let h = feature_hash(self.seed, x, y, z, 0x0A_E0);
        for rule in self.ores.rules() {
            if rule.matches(h, y, biome) {
                return self.ore_variant(base, rule.normal, rule.deepslate);
            }
        }
        base
    }

    fn ore_variant(
        &self,
        base: BlockStateId,
        stone_ore: BlockStateId,
        deepslate_ore: BlockStateId,
    ) -> BlockStateId {
        if base == self.deepslate {
            deepslate_ore
        } else {
            stone_ore
        }
    }
}
