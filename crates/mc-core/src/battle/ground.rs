//! Natural relief for the staged battle. Staging stands every body on the
//! collision top of its own footprint (never in a tree, a wall or water), and
//! steering turns a drive aside from obstacles a jump cannot clear. One-block
//! steps and slopes need nothing here: the living-entity physics kernel jumps
//! a blocked grounded body when the raised body clears the step.
//! Reads resident chunks only; a missing chunk is unknown, never ground.

use crate::registry::CoreRegistry;
use crate::world::ChunkWorld;
use mc_physics::{BlockCollisionBox, BlockMaterial};

/// Free height a mounted pair needs above its support: horse plus rider.
pub(crate) const MOUNTED_CLEARANCE: f64 = 3.0;
/// Ring searched for dry, clear ground when a slot's own footprint is a trunk,
/// a wall or water. Wide enough for a pond; a lake fails setup's start check.
pub(crate) const SEARCH_RADIUS: i32 = 8;
/// Probe distance past the body edge for obstacle steering, in blocks.
const LOOKAHEAD: f64 = 0.45;
/// Lateral reach, in blocks, of the search for an obstacle's nearer end.
const AROUND: i32 = 8;
/// Deepest drop a body walks into, in blocks; deeper holes are steered around
/// and never staged in (vanilla mobs avoid falls over about three blocks).
const MAX_DROP: f64 = 2.0;
/// Contact tolerance for the staging census: physics rests bodies exactly on
/// collision tops, so anything beyond this is a real gap or overlap.
const CONTACT: f64 = 1e-3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Placement {
    /// The slot's own chunk is not resident yet.
    Wait,
    At([f64; 3]),
    /// No dry, clear footprint within [`SEARCH_RADIUS`].
    Nowhere,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Census {
    pub bodies: usize,
    pub inside: usize,
    pub floating: usize,
    pub wet: usize,
}

fn boxes(registry: &CoreRegistry, state: u32) -> &[BlockCollisionBox] {
    registry
        .collision_boxes
        .get(state as usize)
        .map_or(&[BlockCollisionBox::FULL_BLOCK][..], |boxes| &boxes[..])
}

/// Trunks, canopies and stems are scenery to stand under, never ground.
fn tree(registry: &CoreRegistry, state: u32) -> bool {
    registry
        .blocks
        .by_id(mc_world::BlockStateId(state))
        .is_some_and(|state| {
            let path = state.block.id.path();
            ["_leaves", "_log", "_wood", "_stem", "_hyphae"]
                .iter()
                .any(|suffix| path.ends_with(suffix))
                || matches!(path, "mangrove_roots" | "bamboo" | "cactus")
        })
}

/// Top of the highest non-tree collision block of a column, `-inf` when the
/// column has no ground (a bottomless opening), or `Err` when fluid lies on
/// it. `None`: the column's chunk is not resident.
fn surface(world: &ChunkWorld, registry: &CoreRegistry, x: i32, z: i32) -> Option<Result<f64, ()>> {
    let chunk = world.chunk(mc_world::ChunkPos {
        x: x.div_euclid(16),
        z: z.div_euclid(16),
    })?;
    let geometry = chunk.geometry();
    let (lx, lz) = (x.rem_euclid(16) as u8, z.rem_euclid(16) as u8);
    // The heightmap is a start hint; a margin covers later edits above it.
    let start = chunk
        .heightmaps
        .get("MOTION_BLOCKING")
        .map_or(geometry.max_y(), |map| {
            (geometry.min_y() + map.get(lx, lz) as i32 + 16).min(geometry.max_y())
        });
    for y in (geometry.min_y()..start).rev() {
        let state = chunk.get_block(lx, y, lz)?.0;
        if registry.materials.classify(state).is_fluid() {
            return Some(Err(()));
        }
        if tree(registry, state) {
            continue;
        }
        let top = boxes(registry, state)
            .iter()
            .map(|b| b.as_blocks()[4])
            .fold(f64::NEG_INFINITY, f64::max);
        if top.is_finite() {
            return Some(Ok(f64::from(y) + top));
        }
    }
    Some(Ok(f64::NEG_INFINITY))
}

fn footprint(pos: [f64; 3], half_width: f64) -> (std::ops::RangeInclusive<i32>, std::ops::RangeInclusive<i32>) {
    let span = |centre: f64| {
        (centre - half_width + CONTACT).floor() as i32..=(centre + half_width - CONTACT).floor() as i32
    };
    (span(pos[0]), span(pos[2]))
}

/// Whether a body box at `pos` overlaps any collision box or fluid.
/// `None`: part of it lies in a chunk that is not resident.
fn clear(
    world: &ChunkWorld,
    registry: &CoreRegistry,
    pos: [f64; 3],
    half_width: f64,
    height: f64,
) -> Option<bool> {
    let min = [pos[0] - half_width + CONTACT, pos[1] + CONTACT, pos[2] - half_width + CONTACT];
    let max = [pos[0] + half_width - CONTACT, pos[1] + height, pos[2] + half_width - CONTACT];
    let (xs, zs) = footprint(pos, half_width);
    for x in xs {
        for z in zs.clone() {
            for y in min[1].floor() as i32..=max[1].floor() as i32 {
                world.chunk(mc_world::ChunkPos {
                    x: x.div_euclid(16),
                    z: z.div_euclid(16),
                })?;
                let state = world.physics_block_at(x, y, z)?;
                if registry.materials.classify(state).is_fluid() {
                    return Some(false);
                }
                let cell = [f64::from(x), f64::from(y), f64::from(z)];
                for b in boxes(registry, state) {
                    let b = b.as_blocks();
                    if (0..3).all(|axis| cell[axis] + b[axis] < max[axis] && cell[axis] + b[axis + 3] > min[axis]) {
                        return Some(false);
                    }
                }
            }
        }
    }
    Some(true)
}

/// Support under a footprint: the highest dry column surface it covers.
fn support(world: &ChunkWorld, registry: &CoreRegistry, pos: [f64; 3], half_width: f64) -> Option<Result<f64, ()>> {
    let (xs, zs) = footprint(pos, half_width);
    let mut top = f64::NEG_INFINITY;
    for x in xs {
        for z in zs.clone() {
            match surface(world, registry, x, z)? {
                Ok(y) => top = top.max(y),
                Err(()) => return Some(Err(())),
            }
        }
    }
    Some(Ok(top))
}

/// A drop edge or the floor of a hole (cave opening, sinkhole): a column
/// beside the footprint lies more than [`MAX_DROP`] blocks above or below the
/// support, or the support lies more than that below the median ground eight
/// blocks out (wide holes). Unknown and wet columns do not count.
fn pit(world: &ChunkWorld, registry: &CoreRegistry, pos: [f64; 3], half_width: f64) -> bool {
    let (xs, zs) = footprint(pos, half_width);
    for x in xs.start() - 1..=xs.end() + 1 {
        for z in zs.start() - 1..=zs.end() + 1 {
            if let Some(Ok(top)) = surface(world, registry, x, z)
                && (top - pos[1]).abs() > MAX_DROP
            {
                return true;
            }
        }
    }
    let (x, z) = (pos[0].floor() as i32, pos[2].floor() as i32);
    let mut ring = [0.0; 8];
    let mut known = 0;
    for (dx, dz) in [(-8, -8), (0, -8), (8, -8), (-8, 0), (8, 0), (-8, 8), (0, 8), (8, 8)] {
        if let Some(Ok(top)) = surface(world, registry, x + dx, z + dz) {
            ring[known] = top;
            known += 1;
        }
    }
    let ring = &mut ring[..known];
    ring.sort_by(f64::total_cmp);
    known >= 5 && pos[1] + MAX_DROP < ring[known / 2]
}

/// Ground placement for one staged body: its own footprint first, then the
/// nearest dry, clear footprint that is not a pit, by whole-block offsets in
/// deterministic ring order. Columns of chunks that are not resident are
/// skipped.
pub(crate) fn place(
    world: &ChunkWorld,
    registry: &CoreRegistry,
    at: [f64; 3],
    half_width: f64,
    height: f64,
) -> Placement {
    if !world.has_chunk(
        (at[0].floor() as i32).div_euclid(16),
        (at[2].floor() as i32).div_euclid(16),
    ) {
        return Placement::Wait;
    }
    for radius in 0..=SEARCH_RADIUS {
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                if dx.abs().max(dz.abs()) != radius {
                    continue;
                }
                let x = at[0] + f64::from(dx);
                let z = at[2] + f64::from(dz);
                let Some(Ok(y)) = support(world, registry, [x, 0.0, z], half_width) else {
                    continue;
                };
                if !y.is_finite() {
                    continue;
                }
                let pos = [x, y, z];
                if pit(world, registry, pos, half_width) {
                    continue;
                }
                if clear(world, registry, pos, half_width, height) == Some(true) {
                    return Placement::At(pos);
                }
            }
        }
    }
    Placement::Nowhere
}

/// Classify one standing body for the staging census.
pub(crate) fn audit(
    world: &ChunkWorld,
    registry: &CoreRegistry,
    pos: [f64; 3],
    half_width: f64,
    height: f64,
    census: &mut Census,
) {
    census.bodies += 1;
    let (xs, zs) = footprint(pos, half_width);
    let mut inside = false;
    let mut wet = false;
    let mut rest = false;
    for x in xs {
        for z in zs.clone() {
            let cell_xz = [f64::from(x), f64::from(z)];
            let low = (pos[1] - CONTACT).floor() as i32;
            for y in low..=(pos[1] + height).floor() as i32 {
                let Some(state) = world.physics_block_at(x, y, z) else {
                    continue;
                };
                wet |= y > low && registry.materials.classify(state) == BlockMaterial::Water;
                for b in boxes(registry, state) {
                    let b = b.as_blocks();
                    let overlaps_xz = cell_xz[0] + b[0] < pos[0] + half_width - CONTACT
                        && cell_xz[0] + b[3] > pos[0] - half_width + CONTACT
                        && cell_xz[1] + b[2] < pos[2] + half_width - CONTACT
                        && cell_xz[1] + b[5] > pos[2] - half_width + CONTACT;
                    if !overlaps_xz {
                        continue;
                    }
                    let (bottom, top) = (f64::from(y) + b[1], f64::from(y) + b[4]);
                    rest |= (top - pos[1]).abs() <= CONTACT;
                    inside |= bottom < pos[1] + height - CONTACT && top > pos[1] + CONTACT;
                }
            }
        }
    }
    census.inside += usize::from(inside);
    census.wet += usize::from(wet);
    census.floating += usize::from(!inside && !rest);
}

/// A step ahead is crossable when, across the body's whole width just past
/// its leading edge, the cell at head height above the feet is free, if the
/// feet cell holds a step the cell above the step too (vanilla jump
/// headroom), and ground lies within [`MAX_DROP`] blocks below the feet (a
/// cave opening or sinkhole is an obstacle; water is no ground). Slots sit on
/// block boundaries, so one body often straddles two columns. Unknown blocks
/// are obstacles.
fn crossable(
    world: &ChunkWorld,
    registry: &CoreRegistry,
    pos: [f64; 3],
    half_width: f64,
    dir: [f64; 2],
) -> bool {
    let reach = half_width + LOOKAHEAD;
    let ahead = [pos[0] + dir[0] * reach, pos[2] + dir[1] * reach];
    let right = [dir[1], -dir[0]];
    let edge = half_width - CONTACT;
    let feet = (pos[1] + CONTACT).floor() as i32;
    let mut probed = [(i32::MIN, i32::MIN); 3];
    for (i, lateral) in [0.0, edge, -edge].into_iter().enumerate() {
        let x = (ahead[0] + right[0] * lateral).floor() as i32;
        let z = (ahead[1] + right[1] * lateral).floor() as i32;
        if probed[..i].contains(&(x, z)) {
            continue;
        }
        probed[i] = (x, z);
        let solid = |y| {
            world
                .physics_block_at(x, y, z)
                .is_none_or(|state| registry.materials.classify(state) == BlockMaterial::Solid)
        };
        if solid(feet + 1) || (solid(feet) && solid(feet + 2)) {
            return false;
        }
        // On level ground the cell under the feet is solid: one read.
        if !solid(feet - 1) && (2..=MAX_DROP as i32 + 1).all(|depth| !solid(feet - depth)) {
            return false;
        }
    }
    true
}

/// Steer a horizontal drive around a trunk or a wall of two or more blocks.
/// `heading` is where the body is going without crowd pushes or lateral
/// dressing (its formation slot). When the heading or the drive is blocked,
/// the heading turns 45 then 90 degrees toward the obstacle's nearer end
/// (the first lateral offset up to [`AROUND`] blocks whose way ahead is
/// crossable), `side` on a tie or when neither end is near. A stable heading
/// picks the side, so bodies at one wall split at its middle and file
/// outward, and lateral dressing cannot pull a detouring body back behind
/// the wall. When both ways are crossable the drive is returned exactly, so
/// flat ground and one-block steps are unchanged.
pub(crate) fn deflect(
    world: &ChunkWorld,
    registry: &CoreRegistry,
    pos: [f64; 3],
    half_width: f64,
    drive: [f64; 2],
    heading: [f64; 2],
    side: f64,
) -> [f64; 2] {
    let speed = mc_physics::mth::length2(drive[0], drive[1]);
    if speed < 1e-9 {
        return drive;
    }
    let length = mc_physics::mth::length2(heading[0], heading[1]);
    let dir = if length < 1e-9 {
        drive.map(|v| v / speed)
    } else {
        heading.map(|v| v / length)
    };
    if crossable(world, registry, pos, half_width, dir) {
        if crossable(world, registry, pos, half_width, drive.map(|v| v / speed)) {
            return drive;
        }
        // Crowd pushes into an obstacle: go the clear way instead.
        return dir.map(|v| v * speed);
    }
    // A positive turn rotates toward the left of the heading. Blocks are
    // axis-aligned, so the search and the turns run on the grid axis nearest
    // the heading: probes along an oblique heading's normal drift across the
    // obstacle's row, and a turn of an oblique heading runs into the wall it
    // should follow; either flipped bodies toward its far end, into their
    // neighbours.
    let axis = if dir[0].abs() > dir[1].abs() {
        [dir[0].signum(), 0.0]
    } else {
        [0.0, dir[1].signum()]
    };
    let left = [-axis[1], axis[0]];
    let side = (1..=AROUND)
        .find_map(|offset| {
            [side, -side].into_iter().find(|turn| {
                let shift = f64::from(offset) * turn;
                let probe = [pos[0] + left[0] * shift, pos[1], pos[2] + left[1] * shift];
                crossable(world, registry, probe, half_width, axis)
            })
        })
        .unwrap_or(side);
    for (cos, sin) in [
        (std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2),
        (0.0, 1.0),
    ] {
        for turn in [side, -side] {
            let s = sin * turn;
            let turned = [axis[0] * cos - axis[1] * s, axis[0] * s + axis[1] * cos];
            if crossable(world, registry, pos, half_width, turned) {
                return turned.map(|v| v * speed);
            }
        }
    }
    drive
}

#[cfg(test)]
#[path = "ground_tests.rs"]
mod tests;
