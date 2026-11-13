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
/// Least drive, in blocks per second (steering drives are physics
/// velocities), of a body walking out from under rock: the formation's
/// walking step of 0.079 blocks per tick (`FormationRuntime::destination`).
const WALK_OUT: f64 = 0.079 / mc_physics::TICK_SECONDS;
/// Deepest drop a body walks into, in blocks; deeper holes are steered around
/// and never staged in (vanilla mobs avoid falls over about three blocks).
pub(crate) const MAX_DROP: f64 = 2.0;
/// Cells above head height always read for a terrain roof (cave mouths and
/// overhangs on edited terrain, whose heightmaps are stale).
const ROOF_BAND: i32 = 4;
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
/// headroom) and the cell two above the feet over the body's own footprint
/// (a jump first lifts the body where it stands: a canopy over a terrace
/// edge stops it), ground lies within [`MAX_DROP`] blocks below the feet
/// (a cave opening or sinkhole is an obstacle; water is no ground), and the
/// cell has no more rock overhead than the body's own column (from open sky
/// a cave mouth or overhang is an obstacle: fugitives walked down a ravine
/// ramp into a cave and vanished underground; a body already under rock
/// never goes deeper). Slots sit on block boundaries, so one body often
/// straddles two columns. Unknown blocks are obstacles.
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
    let mut rise = None;
    let mut here = None;
    for (i, lateral) in [0.0, edge, -edge].into_iter().enumerate() {
        let x = (ahead[0] + right[0] * lateral).floor() as i32;
        let z = (ahead[1] + right[1] * lateral).floor() as i32;
        if probed[..i].contains(&(x, z)) {
            continue;
        }
        probed[i] = (x, z);
        let Some(step) = footing(world, registry, x, z, feet) else {
            return false;
        };
        if step && !*rise.get_or_insert_with(|| free_above(world, registry, pos, half_width, feet + 2)) {
            return false;
        }
        let above = cover(world, registry, x, z, feet + 2);
        if above > 0
            && above
                > *here.get_or_insert_with(|| {
                    cover(world, registry, pos[0].floor() as i32, pos[2].floor() as i32, feet + 2)
                })
        {
            return false;
        }
    }
    true
}

/// Whether a body with its feet at `feet` can stand in column (x, z): no
/// block at head height, a step of at most one block with room above it, and
/// ground within [`MAX_DROP`] blocks below (water is no ground). `Some(step)`
/// when it can. Unknown blocks are obstacles.
fn footing(world: &ChunkWorld, registry: &CoreRegistry, x: i32, z: i32, feet: i32) -> Option<bool> {
    let solid = |y| {
        world
            .physics_block_at(x, y, z)
            .is_none_or(|state| registry.materials.classify(state) == BlockMaterial::Solid)
    };
    let step = solid(feet);
    if solid(feet + 1) || (step && solid(feet + 2)) {
        return None;
    }
    // On level ground the cell under the feet is solid: one read.
    if !solid(feet - 1) && (2..=MAX_DROP as i32 + 1).all(|depth| !solid(feet - depth)) {
        return None;
    }
    Some(step)
}

/// Terrain blocks (not trees) over column (x, z) at or above `y`: 0 under
/// open sky, more in a cave or under an overhang. The column's
/// motion-blocking heightmap bounds the scan on generated terrain; block
/// edits do not refresh heightmaps (`fold_overlay` writes
/// `Chunk::set_block`), so the [`ROOF_BAND`] cells from `y` up are always
/// read.
fn cover(world: &ChunkWorld, registry: &CoreRegistry, x: i32, z: i32, y: i32) -> usize {
    let Some(chunk) = world.chunk(mc_world::ChunkPos {
        x: x.div_euclid(16),
        z: z.div_euclid(16),
    }) else {
        return 0;
    };
    let (lx, lz) = (x.rem_euclid(16) as u8, z.rem_euclid(16) as u8);
    let top = chunk
        .heightmaps
        .get("MOTION_BLOCKING")
        .map_or(y, |map| chunk.geometry().min_y() + map.get(lx, lz) as i32)
        .max(y + ROOF_BAND);
    (y..top)
        .filter(|&y| {
            chunk.get_block(lx, y, lz).is_some_and(|state| {
                !tree(registry, state.0) && registry.materials.classify(state.0) == BlockMaterial::Solid
            })
        })
        .count()
}

/// Whether terrain rock (a cave roof or an overhang, not a tree) lies over a
/// body's head: [`steer`] then walks it out toward open sky whatever its
/// slot, flight or crowd says.
pub(crate) fn under_rock(world: &ChunkWorld, registry: &CoreRegistry, pos: [f64; 3]) -> bool {
    let feet = (pos[1] + CONTACT).floor() as i32;
    cover(
        world,
        registry,
        pos[0].floor() as i32,
        pos[2].floor() as i32,
        feet + 2,
    ) > 0
}

/// From a body under rock (pushed or knocked into a cave mouth), the way to
/// the first open-sky column it can stand in (no wall, no drop: see
/// [`footing`]) on the nearest ring within [`AROUND`] blocks; `None` under
/// open sky or with no such opening near.
fn exit(world: &ChunkWorld, registry: &CoreRegistry, pos: [f64; 3]) -> Option<[f64; 2]> {
    if !under_rock(world, registry, pos) {
        return None;
    }
    let feet = (pos[1] + CONTACT).floor() as i32;
    let (x, z) = (pos[0].floor() as i32, pos[2].floor() as i32);
    (1..=AROUND).find_map(|radius| {
        (-radius..=radius)
            .flat_map(|dz| (-radius..=radius).map(move |dx| (dx, dz)))
            .filter(|(dx, dz)| dx.abs().max(dz.abs()) == radius)
            .find(|&(dx, dz)| {
                cover(world, registry, x + dx, z + dz, feet + 2) == 0
                    && footing(world, registry, x + dx, z + dz, feet).is_some()
            })
            .map(|(dx, dz)| {
                [
                    f64::from(x + dx) + 0.5 - pos[0],
                    f64::from(z + dz) + 0.5 - pos[2],
                ]
            })
    })
}

/// Whether the cells at height `y` over a body's footprint hold no collision
/// block. Unknown blocks are obstacles.
fn free_above(world: &ChunkWorld, registry: &CoreRegistry, pos: [f64; 3], half_width: f64, y: i32) -> bool {
    let (xs, zs) = footprint(pos, half_width);
    xs.into_iter().all(|x| {
        zs.clone().all(|z| {
            world
                .physics_block_at(x, y, z)
                .is_some_and(|state| registry.materials.classify(state) != BlockMaterial::Solid)
        })
    })
}

/// Drop a drive component that presses into a trunk or wall touching the
/// body's side at head height: physics refuses a step-up jump whose raised
/// sweep is clipped on either axis, so a horse grazing a trunk never climbed
/// the terrace ahead. A drive that touches nothing is returned exactly.
fn slide(world: &ChunkWorld, registry: &CoreRegistry, pos: [f64; 3], half_width: f64, mut drive: [f64; 2]) -> [f64; 2] {
    const TOUCH: f64 = 0.05;
    let feet = (pos[1] + CONTACT).floor() as i32;
    let (xs, zs) = footprint(pos, half_width);
    for (component, centre) in [(0, pos[0]), (1, pos[2])] {
        if drive[component] == 0.0 {
            continue;
        }
        let cell = (centre + drive[component].signum() * (half_width + TOUCH)).floor() as i32;
        let across = if component == 0 { zs.clone() } else { xs.clone() };
        let touching = across.into_iter().any(|other| {
            let (x, z) = if component == 0 { (cell, other) } else { (other, cell) };
            world
                .physics_block_at(x, feet + 1, z)
                .is_some_and(|state| registry.materials.classify(state) == BlockMaterial::Solid)
        });
        if touching {
            drive[component] = 0.0;
        }
    }
    drive
}

/// Steer a horizontal drive around a trunk or a wall of two or more blocks.
/// `heading` is where the body is going without crowd pushes or lateral
/// dressing (its formation slot). When the heading or the drive is blocked,
/// the heading turns 45 then 90 degrees toward the obstacle's nearer end
/// (the first lateral offset up to [`AROUND`] blocks whose way ahead is
/// crossable and that the body can walk to along the obstacle), `side` on a
/// tie or when neither end is near. A stable heading picks the side, so
/// bodies at one wall split at its middle and file outward, and lateral
/// dressing cannot pull a detouring body back behind the wall. When both
/// ways are crossable and nothing touches the body's side the drive is
/// returned exactly, so flat ground and one-block steps are unchanged.
pub(crate) fn deflect(
    world: &ChunkWorld,
    registry: &CoreRegistry,
    pos: [f64; 3],
    half_width: f64,
    drive: [f64; 2],
    heading: [f64; 2],
    side: f64,
) -> [f64; 2] {
    steer(world, registry, pos, half_width, drive, heading, side).unwrap_or_else(|boxed| boxed)
}

/// [`deflect`] for a body on foot. Boxed in (no way ahead and no turn of up
/// to 90 degrees crossable) it stands instead of keeping its drive: physics
/// walked that drive into the cave mouths and over the edges the rules had
/// just refused (r7: a blue pile at a sinkhole's tunnel mouth stepped in and
/// out across it to the end of the take). Under rock it keeps the way out.
pub(crate) fn deflect_foot(
    world: &ChunkWorld,
    registry: &CoreRegistry,
    pos: [f64; 3],
    half_width: f64,
    drive: [f64; 2],
    heading: [f64; 2],
    side: f64,
) -> [f64; 2] {
    match steer(world, registry, pos, half_width, drive, heading, side) {
        Ok(out) => out,
        Err(out) if exit(world, registry, pos).is_some() => out,
        Err(_) => [0.0; 2],
    }
}

/// Ticks a mounted body boxed in by terrain keeps its way back out.
const ESCAPE_TICKS: u64 = 60;

/// A mounted body's way back out of a terrain pocket and the tick it ends.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Escape {
    way: [f64; 2],
    until: u64,
}

/// [`deflect`] for a mounted body. A horse is boxed in where every way
/// ahead and to the sides is refused: the dead end of a gully it ran up (in
/// the G1 rehearsals 20-40 fleeing riders stood pressed into one gully end
/// from t 90 to the end of the take). It then turns back (135 degrees, then
/// 180) and keeps that way for [`ESCAPE_TICKS`], steered round obstacles as
/// usual, so it climbs out instead of turning straight back in; boxed in
/// again, it turns back again. `escape` holds the way between ticks.
#[allow(clippy::too_many_arguments)]
pub(crate) fn deflect_mounted(
    world: &ChunkWorld,
    registry: &CoreRegistry,
    pos: [f64; 3],
    half_width: f64,
    drive: [f64; 2],
    heading: [f64; 2],
    side: f64,
    escape: &mut Option<Escape>,
    tick: u64,
) -> [f64; 2] {
    let speed = mc_physics::mth::length2(drive[0], drive[1]);
    if let Some(Escape { way, until }) = *escape {
        if tick < until
            && speed >= 1e-9
            && let Ok(out) = steer(world, registry, pos, half_width, way.map(|v| v * speed), way, side)
        {
            return out;
        }
        *escape = None;
    }
    let boxed = match steer(world, registry, pos, half_width, drive, heading, side) {
        Ok(out) => return out,
        Err(boxed) if speed < 1e-9 => return boxed,
        Err(boxed) => boxed,
    };
    let length = mc_physics::mth::length2(heading[0], heading[1]);
    let dir = if length < 1e-9 {
        drive.map(|v| v / speed)
    } else {
        heading.map(|v| v / length)
    };
    let axis = if dir[0].abs() > dir[1].abs() {
        [dir[0].signum(), 0.0]
    } else {
        [0.0, dir[1].signum()]
    };
    for (cos, sin) in [(-std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2), (-1.0, 0.0)] {
        for turn in [side, -side] {
            let s = sin * turn;
            let way = [axis[0] * cos - axis[1] * s, axis[0] * s + axis[1] * cos];
            if crossable(world, registry, pos, half_width, way) {
                *escape = Some(Escape {
                    way,
                    until: tick + ESCAPE_TICKS,
                });
                return slide(world, registry, pos, half_width, way.map(|v| v * speed));
            }
        }
    }
    boxed
}

/// [`deflect`]'s turn, or `Err` with the drive to keep when the body is
/// boxed in (no way ahead, no turn of up to 90 degrees crossable).
fn steer(
    world: &ChunkWorld,
    registry: &CoreRegistry,
    pos: [f64; 3],
    half_width: f64,
    drive: [f64; 2],
    heading: [f64; 2],
    side: f64,
) -> Result<[f64; 2], [f64; 2]> {
    // A body under rock walks out toward open sky, at least at walking pace
    // (a body holding its slot has almost no drive), whatever its slot,
    // flight or crowd says; `crossable` never lets it under more rock.
    let speed = mc_physics::mth::length2(drive[0], drive[1]);
    let (drive, heading, speed) = match exit(world, registry, pos) {
        Some(out) => {
            let walk = speed.max(WALK_OUT);
            let length = mc_physics::mth::length2(out[0], out[1]);
            (out.map(|v| v / length * walk), out, walk)
        }
        None if speed < 1e-9 => return Ok(drive),
        None => (drive, heading, speed),
    };
    let length = mc_physics::mth::length2(heading[0], heading[1]);
    let dir = if length < 1e-9 {
        drive.map(|v| v / speed)
    } else {
        heading.map(|v| v / length)
    };
    if crossable(world, registry, pos, half_width, dir) {
        if crossable(world, registry, pos, half_width, drive.map(|v| v / speed)) {
            return Ok(slide(world, registry, pos, half_width, drive));
        }
        // Crowd pushes into an obstacle: go the clear way instead.
        return Ok(slide(world, registry, pos, half_width, dir.map(|v| v * speed)));
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
    // A side counts only while the body can walk along the obstacle to the
    // probe: a dead end (a canopy over a terrace edge, another trunk) closes
    // it, so a body is not sent toward an end it cannot reach and back.
    let mut open = [true; 2];
    let side = (1..=AROUND)
        .find_map(|offset| {
            [side, -side].into_iter().enumerate().find_map(|(i, turn)| {
                let lateral = [left[0] * turn, left[1] * turn];
                let from = f64::from(offset - 1);
                let at = [pos[0] + lateral[0] * from, pos[1], pos[2] + lateral[1] * from];
                open[i] = open[i] && crossable(world, registry, at, half_width, lateral);
                let probe = [at[0] + lateral[0], pos[1], at[2] + lateral[1]];
                (open[i] && crossable(world, registry, probe, half_width, axis)).then_some(turn)
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
                return Ok(slide(world, registry, pos, half_width, turned.map(|v| v * speed)));
            }
        }
    }
    Err(drive)
}

#[cfg(test)]
#[path = "ground_tests.rs"]
mod tests;
