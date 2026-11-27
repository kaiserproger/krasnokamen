//! Java-style hostile attempts: one per eligible chunk per tick, including
//! unsuccessful vertical samples. Habitat checks do not retry failed candidates.
use super::*;

pub(super) fn run(
    state: &mut TickState,
    players: &[[f64; 3]],
    rng: &mut Rng,
    changes: &mut ChangeSet,
) {
    let category = MobCategory::Monster;
    let cap = category.max_instances_per_chunk() as usize;
    let global_cap = cap * state.spawn_census.eligible.len() / 289;
    // Stable player/chunk order. A chunk inside an earlier observer's square
    // was visited there (eligibility and the 128-block check do not depend on
    // the observer), so a visited set replaces re-testing every earlier player
    // for each of the 289 chunks of every observer.
    let mut visited: FxHashSet<(i32, i32)> = FxHashSet::default();
    visited.reserve(state.spawn_census.eligible.len());
    for player in players {
        let cell = chunk_of(*player);
        for x in cell.0 - 8..=cell.0 + 8 {
            for z in cell.1 - 8..=cell.1 + 8 {
                let room = global_cap.saturating_sub(state.spawn_census.global[0]);
                if room == 0 {
                    return;
                }
                if !state.spawn_census.eligible.contains(&(x, z))
                    || !visited.insert((x, z))
                    || !players.iter().any(|p| {
                        (p[0] - f64::from(x * 16 + 8)).powi(2)
                            + (p[2] - f64::from(z * 16 + 8)).powi(2)
                            < SPAWN_MAX * SPAWN_MAX
                    })
                {
                    continue;
                }
                let column = [x * 16 + rng.below(16) as i32, z * 16 + rng.below(16) as i32];
                spawn_group_in_column(state, players, column, category, room, rng, changes);
            }
        }
    }
}

/// Topmost non-air block of the column below `max_y`, `None` for a missing
/// chunk or an all-air column. Same answer as walking `block_at` down from
/// the top, but a section whose palette holds only air is skipped whole (the
/// sky above the terrain is most of the 384 blocks) and the chunk is looked
/// up once. Pending overlay edits take the exact per-block walk.
fn column_top(state: &TickState, x: i32, z: i32, max_y: i32) -> Option<i32> {
    let materials = &state.registry.materials;
    let is_air = |id: u32| materials.classify(id) == BlockMaterial::Air;
    let min_y = state.dimension.geometry().min_y();
    let mut top = max_y - 1;
    if !state.world.overlay_is_empty() {
        loop {
            if !is_air(state.world.block_at(x, top, z)?) {
                return Some(top);
            }
            top -= 1;
            if top < min_y {
                return None;
            }
        }
    }
    let chunk = state.world.chunk(mc_world::ChunkPos {
        x: x.div_euclid(16),
        z: z.div_euclid(16),
    })?;
    let (lx, lz) = (x.rem_euclid(16) as u8, z.rem_euclid(16) as u8);
    while top >= min_y {
        let (index, sy) = chunk.geometry().world_y_to_section(top)?;
        let section = &chunk.sections[index];
        let base = top - i32::from(sy);
        let all_air = section.palette().map_or_else(
            || is_air(section.get(0, 0, 0).0),
            |palette| palette.iter().all(|state| is_air(state.0)),
        );
        if !all_air {
            for y in (base..=top).rev() {
                if !is_air(section.get(lx, (y - base) as u8, lz).0) {
                    return Some(y);
                }
            }
        }
        top = base - 1;
    }
    None
}

pub(super) fn height(state: &TickState, x: i32, z: i32, rng: &mut Rng) -> Option<i32> {
    let geometry = state.dimension.geometry();
    // Include solid blocks and unsupported air in the sample space, rather than
    // reservoir-sampling only successful floors (which floods exposed surfaces).
    let max_y = if state.dimension == crate::dimensions::Dimension::Nether {
        128
    } else {
        geometry.max_y()
    };
    let top = column_top(state, x, z, max_y)?;
    let ceiling = (top + 1).min(max_y - 1);
    Some(geometry.min_y() + rng.below((ceiling - geometry.min_y() + 1) as u64) as i32)
}

fn slime_chunk(seed: i64, x: i32, z: i32) -> bool {
    let seed = seed
        .wrapping_add(i64::from(x.wrapping_mul(x).wrapping_mul(0x4c1906)))
        .wrapping_add(i64::from(x.wrapping_mul(0x5ac0db)))
        .wrapping_add(i64::from(z.wrapping_mul(z)).wrapping_mul(0x4307a7))
        .wrapping_add(i64::from(z.wrapping_mul(0x5f24f)))
        ^ 0x3ad8025f;
    mc_inventory::enchanting::LegacyRandom::new(seed).next_int(10) == 0
}

pub(super) fn slime_allowed(
    state: &TickState,
    pos: [i32; 3],
    brightness: u8,
    rng: &mut Rng,
) -> bool {
    let [x, y, z] = pos;
    if y < 40 && slime_chunk(state.world.seed(), x.div_euclid(16), z.div_euclid(16)) {
        return rng.below(10) == 0;
    }
    if !(51..=69).contains(&y)
        || !state.world.biome_at(x, y, z).is_some_and(|biome| {
            matches!(
                biome.as_str(),
                "minecraft:swamp" | "minecraft:mangrove_swamp"
            )
        })
    {
        return false;
    }
    const MOON: [f64; 8] = [1.0, 0.75, 0.5, 0.25, 0.0, 0.25, 0.5, 0.75];
    let moon = MOON[state.day_time.div_euclid(24_000).rem_euclid(8) as usize];
    rng.below(2) == 0 && rng.unit() < moon && u64::from(brightness) <= rng.below(8)
}
