//! Explicit, bounded discovery of the room containing a player. No world mutation.
use mc_script::ScriptWorldBounds;

use crate::tick::TickState;

const MAX_AXIS: i32 = 64;
const MAX_VOLUME: i32 = 32768;
const GRID_SIDE: usize = 127;

#[path = "building_open_discovery.rs"]
mod open;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Refusal {
    OpenAir,
    NotEnclosed,
    TooLarge,
    Unloaded,
    NoFloor,
    NoWorkPoint,
    NoWall,
    NoGate,
    Occupied,
}

fn solid(state: &TickState, at: [i32; 3]) -> Result<bool, Refusal> {
    let id = state
        .world
        .block_at(at[0], at[1], at[2])
        .ok_or(Refusal::Unloaded)?;
    // Doors, including open doors, have collision and delimit rooms.
    Ok(!state.registry.collision_boxes[id as usize].is_empty())
}

fn probes(state: &TickState, feet: [i32; 3], y: i32) -> Result<(), Refusal> {
    let mut distances = [0; 4];
    for (index, [dx, dz]) in [[-1, 0], [1, 0], [0, -1], [0, 1]].into_iter().enumerate() {
        for distance in 1..MAX_AXIS {
            let Some(x) = feet[0].checked_add(dx * distance) else {
                break;
            };
            let Some(z) = feet[2].checked_add(dz * distance) else {
                break;
            };
            match solid(state, [x, y, z]) {
                Ok(true) => {
                    distances[index] = distance;
                    break;
                }
                Ok(false) => {}
                Err(_) => break,
            }
        }
    }
    let closed = distances.iter().filter(|distance| **distance != 0).count();
    if closed != 4 {
        return Err(if closed == 0 {
            Refusal::OpenAir
        } else {
            Refusal::NotEnclosed
        });
    }
    if distances[0] + distances[1] + 1 > MAX_AXIS || distances[2] + distances[3] + 1 > MAX_AXIS {
        return Err(Refusal::TooLarge);
    }
    Ok(())
}

fn footprint(state: &TickState, feet: [i32; 3], y: i32) -> Result<([i32; 2], [i32; 2]), Refusal> {
    probes(state, feet, y)?;
    if solid(state, [feet[0], y, feet[2]])? {
        return Err(Refusal::NotEnclosed);
    }
    let mut seen = [0_u64; (GRID_SIDE * GRID_SIDE).div_ceil(64)];
    let index = |x: i32, z: i32| -> Result<usize, Refusal> {
        let dx = i64::from(x) - i64::from(feet[0]) + 63;
        let dz = i64::from(z) - i64::from(feet[2]) + 63;
        if !(0..127).contains(&dx) || !(0..127).contains(&dz) {
            return Err(Refusal::TooLarge);
        }
        Ok(dx as usize * GRID_SIDE + dz as usize)
    };
    let mut cells = Vec::with_capacity(62 * 62);
    cells.push([feet[0], feet[2]]);
    let first = index(feet[0], feet[2])?;
    seen[first / 64] |= 1 << (first % 64);
    let mut min = [feet[0], feet[2]];
    let mut max = min;
    let mut cursor = 0;
    while cursor < cells.len() {
        let [x, z] = cells[cursor];
        cursor += 1;
        for [dx, dz] in [[-1, 0], [1, 0], [0, -1], [0, 1]] {
            let x = x.checked_add(dx).ok_or(Refusal::TooLarge)?;
            let z = z.checked_add(dz).ok_or(Refusal::TooLarge)?;
            let bit = index(x, z)?;
            if seen[bit / 64] & (1 << (bit % 64)) != 0 {
                continue;
            }
            seen[bit / 64] |= 1 << (bit % 64);
            if solid(state, [x, y, z])? {
                continue;
            }
            min[0] = min[0].min(x);
            min[1] = min[1].min(z);
            max[0] = max[0].max(x);
            max[1] = max[1].max(z);
            if i64::from(max[0]) - i64::from(min[0]) + 3 > i64::from(MAX_AXIS)
                || i64::from(max[1]) - i64::from(min[1]) + 3 > i64::from(MAX_AXIS)
            {
                return Err(Refusal::TooLarge);
            }
            cells.push([x, z]);
        }
    }
    // A bounding box must not borrow an isolated room's stations or beds.
    for x in min[0]..=max[0] {
        for z in min[1]..=max[1] {
            let bit = index(x, z)?;
            if seen[bit / 64] & (1 << (bit % 64)) == 0 && !solid(state, [x, y, z])? {
                return Err(Refusal::NotEnclosed);
            }
        }
    }
    Ok((min, max))
}

/// Enclosed kinds require a room; open kinds also have bounded site discovery.
pub(super) fn find_for_kind(
    state: &TickState,
    feet: [i32; 3],
    kind: super::buildings::BuildingKind,
) -> Result<ScriptWorldBounds, Refusal> {
    use super::buildings::BuildingKind;
    match find(state, feet) {
        Ok(bounds) => Ok(bounds),
        Err(reason @ (Refusal::OpenAir | Refusal::NotEnclosed))
            if matches!(
                kind,
                BuildingKind::House | BuildingKind::Barracks | BuildingKind::Tower
            ) =>
        {
            Err(reason)
        }
        Err(Refusal::OpenAir | Refusal::NotEnclosed) => open::find(state, feet, kind),
        Err(reason) => Err(reason),
    }
}

fn full(state: &TickState, at: [i32; 3]) -> Result<bool, Refusal> {
    let id = state
        .world
        .block_at(at[0], at[1], at[2])
        .ok_or(Refusal::Unloaded)?;
    Ok(state.registry.collision_boxes[id as usize]
        .iter()
        .any(|shape| {
            shape.coordinates() == mc_physics::BlockCollisionBox::FULL_BLOCK.coordinates()
        }))
}

fn floor_below(state: &TickState, feet: [i32; 3]) -> Result<i32, Refusal> {
    for depth in 1..=8 {
        let y = feet[1].checked_sub(depth).ok_or(Refusal::TooLarge)?;
        if full(state, [feet[0], y, feet[2]])? {
            return Ok(y);
        }
    }
    Err(Refusal::NoFloor)
}

pub(super) fn find(state: &TickState, feet: [i32; 3]) -> Result<ScriptWorldBounds, Refusal> {
    let head = feet[1].checked_add(1).ok_or(Refusal::TooLarge)?;
    let (min, max) = footprint(state, feet, head)?;
    let floor = floor_below(state, feet)?;
    let x0 = min[0].checked_sub(1).ok_or(Refusal::TooLarge)?;
    let z0 = min[1].checked_sub(1).ok_or(Refusal::TooLarge)?;
    let x1 = max[0].checked_add(1).ok_or(Refusal::TooLarge)?;
    let z1 = max[1].checked_add(1).ok_or(Refusal::TooLarge)?;
    let area = (x1 - x0 + 1) * (z1 - z0 + 1);
    let top = floor
        .checked_add((MAX_VOLUME / area).min(MAX_AXIS) - 1)
        .ok_or(Refusal::TooLarge)?;
    let mut roof = None;
    for x in min[0]..=max[0] {
        for z in min[1]..=max[1] {
            for y in head.saturating_add(1)..=top {
                if solid(state, [x, y, z])? {
                    roof = Some(roof.map_or(y, |old: i32| old.max(y)));
                    break;
                }
            }
        }
    }
    let ceiling = if let Some(roof) = roof {
        roof
    } else {
        let mut wall_top = head;
        for x in x0..=x1 {
            for z in z0..=z1 {
                if x != x0 && x != x1 && z != z0 && z != z1 {
                    continue;
                }
                for y in head..=top {
                    if !solid(state, [x, y, z])? {
                        break;
                    }
                    wall_top = wall_top.max(y);
                }
            }
        }
        wall_top
            .checked_add(1)
            .filter(|y| *y <= top)
            .ok_or(Refusal::TooLarge)?
    };
    ScriptWorldBounds::new([x0, floor, z0], [x1, ceiling, z1]).map_err(|_| Refusal::TooLarge)
}

#[cfg(test)]
#[path = "building_discovery_tests.rs"]
pub(crate) mod tests;
