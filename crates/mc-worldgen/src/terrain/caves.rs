//! Carver application: cave column bounds, sampled layers and cave cells.

use super::ores::OreColumnCache;

use super::*;

impl TerrainGenerator {
    pub(super) fn apply_caves(&self, chunk: &mut Chunk, columns: &[ColumnPlan; 256]) {
        self.apply_caves_with_cache(chunk, columns, None);
    }

    pub(super) fn apply_caves_with_cache(
        &self,
        chunk: &mut Chunk,
        columns: &[ColumnPlan; 256],
        mut ore_cache: Option<&mut OreColumnCache>,
    ) {
        if self.vanilla().is_some() {
            return;
        }
        const RAW_SIDE: usize = 18;
        let cave_min_y = self.geometry.min_y().saturating_add(8);
        let cave_max_y = columns
            .iter()
            .filter_map(|plan| self.cave_y_bounds(plan).map(|(_, max_y)| max_y))
            .max()
            .unwrap_or(cave_min_y.saturating_sub(1))
            .min(31);
        if cave_max_y < cave_min_y {
            return;
        }

        let chunk_min_x = i64::from(chunk.pos.x) * 16;
        let chunk_min_z = i64::from(chunk.pos.z) * 16;
        let router = self.density_router();
        let raw_min_x = chunk_min_x - 1;
        let raw_min_z = chunk_min_z - 1;
        let raw_max_x = chunk_min_x + 16;
        let raw_max_z = chunk_min_z + 16;
        let raw_grid_bounds = i32::try_from(raw_min_x)
            .ok()
            .zip(i32::try_from(raw_min_z).ok())
            .zip(i32::try_from(raw_max_x).ok())
            .zip(i32::try_from(raw_max_z).ok());
        let mut raw = [0_u8; RAW_SIDE * RAW_SIDE];
        for sample_y in (cave_min_y..=cave_max_y).step_by(CAVE_VERTICAL_SAMPLE_STEP as usize) {
            if let Some((((raw_min_x, raw_min_z), _), _)) = raw_grid_bounds {
                router.fill_raw_cave_layer(raw_min_x, raw_min_z, RAW_SIDE, sample_y, &mut raw);
            } else {
                raw.fill(1);
                for raw_z in 0..RAW_SIDE {
                    for raw_x in 0..RAW_SIDE {
                        let world_x = chunk_min_x + raw_x as i64 - 1;
                        let world_z = chunk_min_z + raw_z as i64 - 1;
                        let cave = i32::try_from(world_x)
                            .ok()
                            .zip(i32::try_from(world_z).ok())
                            .is_some_and(|(world_x, world_z)| {
                                router.raw_cave(world_x, sample_y, world_z)
                            });
                        raw[raw_z * RAW_SIDE + raw_x] = if cave { 2 } else { 1 };
                    }
                }
            }

            for plan in columns {
                let Some((plan_min_y, plan_max_y)) = self.cave_y_bounds(plan) else {
                    continue;
                };
                let cave_limit = plan.height.saturating_sub(CAVE_SURFACE_CLEARANCE).min(32);
                if sample_y < plan_min_y || sample_y > plan_max_y || sample_y >= cave_limit {
                    continue;
                }
                let raw_x = usize::from(plan.lx) + 1;
                let raw_z = usize::from(plan.lz) + 1;
                let center = raw_z * RAW_SIDE + raw_x;
                if raw[center] != 2 {
                    continue;
                }
                let connected = raw[raw_z * RAW_SIDE + raw_x - 1] == 2
                    || raw[raw_z * RAW_SIDE + raw_x + 1] == 2
                    || raw[(raw_z - 1) * RAW_SIDE + raw_x] == 2
                    || raw[(raw_z + 1) * RAW_SIDE + raw_x] == 2;
                if !connected {
                    continue;
                }
                for y in sample_y..sample_y.saturating_add(CAVE_VERTICAL_SAMPLE_STEP) {
                    if y > plan_max_y || y >= self.geometry.max_y() {
                        break;
                    }
                    let _ = chunk.set_block(plan.lx, y, plan.lz, self.air);
                }
            }
            if let Some(cache) = ore_cache.as_deref_mut() {
                for raw_z in 0..RAW_SIDE {
                    for raw_x in 0..RAW_SIDE {
                        let state = raw[raw_z * RAW_SIDE + raw_x];
                        if state == 0 {
                            continue;
                        }
                        let world_x = chunk_min_x + raw_x as i64 - 1;
                        let world_z = chunk_min_z + raw_z as i64 - 1;
                        if let (Ok(world_x), Ok(world_z)) =
                            (i32::try_from(world_x), i32::try_from(world_z))
                        {
                            cache.record_raw_cave(world_x, sample_y, world_z, state == 2);
                        }
                    }
                }
            }
        }
    }

    pub(super) fn cave_y_bounds(&self, plan: &ColumnPlan) -> Option<(i32, i32)> {
        let cave_min_y = i64::from(self.geometry.min_y()) + 8;
        let cave_max_y = (i64::from(plan.height) - i64::from(CAVE_SURFACE_CLEARANCE))
            .min(i64::from(plan.dirt_start) - 1);
        if cave_max_y < cave_min_y {
            return None;
        }
        Some((
            i32::try_from(cave_min_y).ok()?,
            i32::try_from(cave_max_y).ok()?,
        ))
    }

    pub(super) fn is_cave_cell(&self, x: i32, y: i32, z: i32, surface_y: i32) -> bool {
        if let Some((_, density)) = self.vanilla() {
            return y < surface_y && density.density(x, y, z) <= 0.0;
        }
        self.density_router().is_cave(x, y, z, surface_y)
    }
}
