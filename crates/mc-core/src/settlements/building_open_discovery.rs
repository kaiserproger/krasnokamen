//! Position-based discovery for open work areas, straight walls and gate frames.
use super::{Refusal, ScriptWorldBounds, TickState, floor_below, full, solid};
use crate::settlements::buildings::{self, BuildingKind};

fn contains(bounds: ScriptWorldBounds, at: [i32; 3]) -> bool {
    (0..3).all(|axis| bounds.min[axis] <= at[axis] && at[axis] <= bounds.max[axis])
}

fn occupied(state: &TickState, at: [i32; 3]) -> bool {
    state.settlements.records().any(|record| {
        record
            .buildings
            .values()
            .any(|building| contains(building.bounds, at))
    })
}

fn offset(value: i32, delta: i32) -> Result<i32, Refusal> {
    value.checked_add(delta).ok_or(Refusal::TooLarge)
}

fn range(value: i32, radius: i32) -> Result<std::ops::RangeInclusive<i32>, Refusal> {
    Ok(offset(value, -radius)?..=offset(value, radius)?)
}

pub(super) fn find(
    state: &TickState,
    feet: [i32; 3],
    kind: BuildingKind,
) -> Result<ScriptWorldBounds, Refusal> {
    for record in state.settlements.records() {
        for building in record.buildings.values() {
            if contains(building.bounds, feet) {
                return if building.kind == kind {
                    Ok(building.bounds)
                } else {
                    Err(Refusal::Occupied)
                };
            }
        }
    }
    let floor = floor_below(state, feet)?;
    let bounds = match kind {
        BuildingKind::Wall => wall(state, feet, floor)?,
        BuildingKind::Gate => gate(state, feet, floor)?,
        _ => workplace(state, feet, floor, kind)?,
    };
    if state.settlements.records().any(|record| {
        record.buildings.values().any(|building| {
            (0..3).all(|axis| {
                bounds.min[axis] <= building.bounds.max[axis]
                    && building.bounds.min[axis] <= bounds.max[axis]
            })
        })
    }) {
        return Err(Refusal::Occupied);
    }
    Ok(bounds)
}

fn workplace(
    state: &TickState,
    feet: [i32; 3],
    floor: i32,
    kind: BuildingKind,
) -> Result<ScriptWorldBounds, Refusal> {
    let mut min = [feet[0], floor, feet[2]];
    let mut max = [feet[0], offset(floor, 3)?, feet[2]];
    let mut found = false;
    for x in range(feet[0], 4)? {
        for z in range(feet[2], 4)? {
            for y in offset(floor, 1)?..=offset(floor, 4)? {
                let at = [x, y, z];
                let id = state.world.block_at(x, y, z).ok_or(Refusal::Unloaded)?;
                let matches = if kind == BuildingKind::Storage {
                    state
                        .registry
                        .blocks
                        .by_id(mc_world::BlockStateId(id))
                        .is_some_and(|definition| {
                            matches!(
                                definition.block.id.path(),
                                "chest" | "trapped_chest" | "barrel"
                            )
                        })
                } else {
                    buildings::workstation_matches(&state.registry, kind, id)
                };
                if matches && !occupied(state, at) {
                    found = true;
                    for axis in [0, 2] {
                        min[axis] = min[axis].min(at[axis]);
                        max[axis] = max[axis].max(at[axis]);
                    }
                    max[1] = max[1].max(y);
                }
            }
        }
    }
    if !found {
        return Err(Refusal::NoWorkPoint);
    }
    for axis in [0, 2] {
        min[axis] = offset(min[axis], -1)?;
        max[axis] = offset(max[axis], 1)?;
    }
    ScriptWorldBounds::new(min, max).map_err(|_| Refusal::TooLarge)
}

fn free_full(state: &TickState, at: [i32; 3]) -> Result<bool, Refusal> {
    Ok(full(state, at)? && !occupied(state, at))
}

fn line_length(
    state: &TickState,
    seed: [i32; 3],
    axis: usize,
    direction: i32,
) -> Result<i32, Refusal> {
    for distance in 1..=64 {
        let mut at = seed;
        at[axis] = offset(at[axis], direction * distance)?;
        if !free_full(state, at)? {
            return Ok(distance - 1);
        }
    }
    Err(Refusal::TooLarge)
}

fn wall(state: &TickState, feet: [i32; 3], floor: i32) -> Result<ScriptWorldBounds, Refusal> {
    let mut nearest = None;
    for x in range(feet[0], 2)? {
        for z in range(feet[2], 2)? {
            let at = [x, offset(floor, 1)?, z];
            if free_full(state, at)? {
                let distance = (x - feet[0]).pow(2) + (z - feet[2]).pow(2);
                if nearest.is_none_or(|(known, _)| distance < known) {
                    nearest = Some((distance, at));
                }
            }
        }
    }
    let (_, seed) = nearest.ok_or(Refusal::NoWall)?;
    let x = [
        line_length(state, seed, 0, -1)?,
        line_length(state, seed, 0, 1)?,
    ];
    let z = [
        line_length(state, seed, 2, -1)?,
        line_length(state, seed, 2, 1)?,
    ];
    let (axis, lengths) = if x[0] + x[1] >= z[0] + z[1] {
        (0, x)
    } else {
        (2, z)
    };
    if lengths[0] + lengths[1] + 1 > 64 {
        return Err(Refusal::TooLarge);
    }
    let mut min = seed;
    let mut max = seed;
    min[axis] = offset(seed[axis], -lengths[0])?;
    max[axis] = offset(seed[axis], lengths[1])?;
    min[1] = floor;
    for coordinate in min[axis]..=max[axis] {
        let mut at = seed;
        at[axis] = coordinate;
        for height in 1..=64 {
            at[1] = offset(floor, height)?;
            if !free_full(state, at)? {
                break;
            }
            if height == 64 {
                return Err(Refusal::TooLarge);
            }
            max[1] = max[1].max(at[1]);
        }
    }
    ScriptWorldBounds::new(min, max).map_err(|_| Refusal::TooLarge)
}

fn door(state: &TickState, at: [i32; 3]) -> Result<bool, Refusal> {
    let id = state
        .world
        .block_at(at[0], at[1], at[2])
        .ok_or(Refusal::Unloaded)?;
    Ok(state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(id))
        .is_some_and(|definition| definition.block.id.path().ends_with("_door"))
        && !occupied(state, at))
}

fn gate(state: &TickState, feet: [i32; 3], floor: i32) -> Result<ScriptWorldBounds, Refusal> {
    let mut nearest = None;
    for x in range(feet[0], 4)? {
        for z in range(feet[2], 4)? {
            let at = [x, offset(floor, 1)?, z];
            if door(state, at)? {
                let distance = (x - feet[0]).pow(2) + (z - feet[2]).pow(2);
                if nearest.is_none_or(|(known, _)| distance < known) {
                    nearest = Some((distance, at));
                }
            }
        }
    }
    let (_, seed) = nearest.ok_or(Refusal::NoGate)?;
    let index = |at: [i32; 3]| -> Option<usize> {
        let x = i64::from(at[0]) - i64::from(seed[0]) + 4;
        let z = i64::from(at[2]) - i64::from(seed[2]) + 4;
        let y = i64::from(at[1]) - i64::from(floor) - 1;
        ((0..9).contains(&x) && (0..9).contains(&z) && (0..8).contains(&y))
            .then(|| (x as usize * 9 + z as usize) * 8 + y as usize)
    };
    let mut seen = [false; 9 * 9 * 8];
    let mut cells = Vec::with_capacity(seen.len());
    cells.push(seed);
    seen[index(seed).unwrap()] = true;
    let mut min = seed;
    let mut max = seed;
    let mut cursor = 0;
    while cursor < cells.len() {
        let at = cells[cursor];
        cursor += 1;
        for axis in 0..3 {
            for direction in [-1, 1] {
                let mut next = at;
                next[axis] = offset(next[axis], direction)?;
                if next[1] <= floor {
                    continue;
                }
                let bit = index(next);
                if bit.is_some_and(|bit| seen[bit]) {
                    continue;
                }
                if let Some(bit) = bit {
                    seen[bit] = true;
                }
                if !solid(state, next)? || occupied(state, next) {
                    continue;
                }
                let _ = bit.ok_or(Refusal::TooLarge)?;
                for axis in 0..3 {
                    min[axis] = min[axis].min(next[axis]);
                    max[axis] = max[axis].max(next[axis]);
                }
                cells.push(next);
            }
        }
    }
    min[1] = floor;
    ScriptWorldBounds::new(min, max).map_err(|_| Refusal::TooLarge)
}
