//! Shared swept collision, ballistic motion, pickup and loyalty return.
//! One post-move body snapshot owns exact hit geometry. It holds only bodies
//! that meet some shot's frozen query box (plus deployed stands and dragon
//! parts). Large volleys use a two-block XZ counting grid with Y rejection;
//! small/sparse scenes use native chunks. Membership freezes before payload
//! mutations. Pickup/riptide keep live native chunk queries and dense-slot
//! priority.

use std::sync::Arc;

use mc_physics::BlockMaterial;
use uuid::Uuid;

use super::{ARROW_IN_GROUND_INDEX, ProjectileKind, ProjectileState};
use crate::boundary::{EntityDataValue, EntityId};
use crate::entities::{EntityKind, EntitySlot};
use crate::intent::{DamageSource, Footprint, Intent, IntentKind};
use crate::survival::DamageKind;
use crate::tick::TickState;

#[cfg(test)]
#[path = "flight_top100_h02_tests.rs"]
mod top100_h02_tests;

#[derive(Debug)]
pub(super) struct Hittable {
    slot: Option<EntitySlot>,
    id: EntityId,
    uuid: Uuid,
    /// Armor stand: a possible plugin deployment. Hit tests resolve its slot by
    /// id, because a projectile removed earlier in the same pass swap-moves rows.
    deployed_candidate: bool,
    min: [f64; 3],
    max: [f64; 3],
    item: bool,
    /// Frozen mob allegiance; players and vanilla/unaffiliated bodies stay zero.
    battle_faction: u16,
}

#[cfg(test)]
fn hittables(
    state: &TickState,
    include_items: bool,
    cover: Option<&Cover>,
    bodies: &mut Vec<Hittable>,
) {
    hittables_from_slots(state, include_items, cover, bodies, state.entities.slots());
}

fn hittables_from_slots(
    state: &TickState,
    include_items: bool,
    cover: Option<&Cover>,
    bodies: &mut Vec<Hittable>,
    slots: impl Iterator<Item = EntitySlot>,
) {
    bodies.clear();
    let mut stand_types: Vec<(u32, bool)> = Vec::new();
    bodies.extend(slots.filter_map(|slot| {
        let body = state.entities.body(slot);
        if crate::end_fight::is_dragon(state, slot) {
            return None;
        }
        let hittable = body.alive
            && match body.kind {
                EntityKind::Player { .. } => state
                    .players
                    .get(&body.id)
                    .is_some_and(|player| !player.creative() && !player.spectator()),
                EntityKind::Mob { .. } => {
                    body.movement != crate::blueprint::MovementClass::Projectile
                }
                EntityKind::Item { .. } => include_items,
                EntityKind::ExperienceOrb { .. } => false,
            };
        if !hittable {
            return None;
        }
        let pos = state.entities.pos(slot);
        let min = [pos[0] - body.half_width, pos[1], pos[2] - body.half_width];
        let max = [
            pos[0] + body.half_width,
            pos[1] + body.height,
            pos[2] + body.half_width,
        ];
        let deployed_candidate = match body.kind {
            EntityKind::Mob { type_id } => {
                match stand_types.iter().find(|(seen, _)| *seen == type_id) {
                    Some(&(_, stand)) => stand,
                    None => {
                        let stand = crate::armor_stand::is_stand(state, slot);
                        stand_types.push((type_id, stand));
                        stand
                    }
                }
            }
            _ => false,
        };
        if !deployed_candidate && cover.is_some_and(|cover| !cover.reaches(min, max)) {
            return None;
        }
        Some(Hittable {
            slot: Some(slot),
            id: body.id,
            uuid: state.entities.row(slot).uuid,
            item: matches!(body.kind, EntityKind::Item { .. }),
            battle_faction: if matches!(body.kind, EntityKind::Mob { .. }) {
                body.faction
            } else {
                0
            },
            deployed_candidate,
            min,
            max,
        })
    }));
    if let Some(uuid) = state.end_fight.dragon_uuid
        && let Some(slot) = state.entities.slot_of_uuid(uuid)
    {
        let id = state.entities.body(slot).id;
        for offset in 1..=8 {
            let part = EntityId(id.0 + offset);
            if let Some((min, max)) = crate::end_fight::part_bounds(state, part) {
                bodies.push(Hittable {
                    slot: None,
                    id: part,
                    uuid,
                    item: false,
                    battle_faction: 0,
                    deployed_candidate: false,
                    min,
                    max,
                });
            }
        }
    }
}

/// Derived, reusable scratch over the frozen geometry, never a second entity
/// authority. One center entry per body; maximum half extent makes queries a
/// conservative superset even for giants and boundary-straddling bodies.
#[derive(Debug, Default)]
pub(super) struct FineGrid {
    origin: [i32; 2],
    size: [usize; 2],
    half_extent: f64,
    offsets: Vec<usize>,
    cursors: Vec<usize>,
    items: Vec<usize>,
}

impl FineGrid {
    const CELL: i32 = 2;
    const MIN_SHOTS: usize = 16;
    const MAX_CELLS: usize = 1 << 20;

    fn cell(value: f64) -> i32 {
        mc_physics::mth::floor_i32(value).div_euclid(Self::CELL)
    }

    fn center(body: &Hittable) -> [f64; 2] {
        [
            body.min[0] * 0.5 + body.max[0] * 0.5,
            body.min[2] * 0.5 + body.max[2] * 0.5,
        ]
    }

    fn index(&self, center: [f64; 2]) -> usize {
        let x = (i64::from(Self::cell(center[0])) - i64::from(self.origin[0])) as usize;
        let z = (i64::from(Self::cell(center[1])) - i64::from(self.origin[1])) as usize;
        z * self.size[0] + x
    }

    /// `reach`: cells the caller already proved worth spanning (the shots'
    /// cover); a covered snapshot may be sparser than `bodies.len()` implies.
    fn build(&mut self, bodies: &[Hittable], reach: usize) -> bool {
        self.size = [0; 2];
        if bodies.is_empty() {
            return false;
        }
        let mut min = [i32::MAX; 2];
        let mut max = [i32::MIN; 2];
        self.half_extent = 0.0;
        for body in bodies {
            if body.min.iter().chain(&body.max).any(|v| !v.is_finite())
                || (0..3).any(|axis| body.min[axis] > body.max[axis])
            {
                return false;
            }
            for (axis, value) in Self::center(body).into_iter().enumerate() {
                min[axis] = min[axis].min(Self::cell(value));
                max[axis] = max[axis].max(Self::cell(value));
            }
            self.half_extent = self
                .half_extent
                .max((body.max[0] - body.min[0]) * 0.5)
                .max((body.max[2] - body.min[2]) * 0.5);
        }
        if !self.half_extent.is_finite() {
            return false;
        }
        let width = (i64::from(max[0]) - i64::from(min[0]) + 1) as usize;
        let height = (i64::from(max[1]) - i64::from(min[1]) + 1) as usize;
        let Some(cells) = width.checked_mul(height) else {
            return false;
        };
        // Widely separated worlds must not allocate/enumerate empty continents.
        // The ordinary chunk path is still conservative for these sparse scenes.
        if cells > Self::MAX_CELLS || cells > bodies.len().saturating_mul(16).max(4096).max(reach) {
            return false;
        }
        self.origin = min;
        self.size = [width, height];
        self.cursors.clear();
        self.cursors.resize(cells, 0);
        for body in bodies {
            let cell = self.index(Self::center(body));
            self.cursors[cell] += 1;
        }
        self.offsets.clear();
        self.offsets.push(0);
        for count in &mut self.cursors {
            let start = *self.offsets.last().unwrap();
            self.offsets.push(start + *count);
            *count = start;
        }
        self.items.resize(bodies.len(), 0);
        for (index, body) in bodies.iter().enumerate() {
            let cell = self.index(Self::center(body));
            self.items[self.cursors[cell]] = index;
            self.cursors[cell] += 1;
        }
        true
    }

    /// Return test-only work, without adding hot-path counters to release builds.
    fn query(
        &self,
        bodies: &[Hittable],
        start: [f64; 3],
        end: [f64; 3],
        radius: f64,
        out: &mut Vec<usize>,
    ) -> usize {
        let (low, high) = query_box(start, end, radius);
        let mut cells = [[0_i64; 2]; 2];
        for (axis, world_axis) in [0, 2].into_iter().enumerate() {
            cells[axis][0] = (i64::from(Self::cell(low[world_axis] - self.half_extent))
                - i64::from(self.origin[axis]))
            .max(0);
            cells[axis][1] = (i64::from(Self::cell(high[world_axis] + self.half_extent))
                - i64::from(self.origin[axis]))
            .min(self.size[axis] as i64 - 1);
        }
        #[cfg(test)]
        let mut examined = 0;
        for z in cells[1][0]..=cells[1][1] {
            for x in cells[0][0]..=cells[0][1] {
                let cell = z as usize * self.size[0] + x as usize;
                for &index in &self.items[self.offsets[cell]..self.offsets[cell + 1]] {
                    #[cfg(test)]
                    {
                        examined += 1;
                    }
                    let body = &bodies[index];
                    if (0..3)
                        .all(|axis| body.max[axis] >= low[axis] && body.min[axis] <= high[axis])
                    {
                        out.push(index);
                    }
                }
            }
        }
        #[cfg(test)]
        {
            examined
        }
        #[cfg(not(test))]
        {
            0
        }
    }
}

/// The broad-phase box of one swept query: every body a shot can be tested
/// against overlaps it (inclusive bounds, as `FineGrid::query` compares).
fn query_box(start: [f64; 3], end: [f64; 3], radius: f64) -> ([f64; 3], [f64; 3]) {
    (
        std::array::from_fn(|axis| start[axis].min(end[axis]) - radius),
        std::array::from_fn(|axis| start[axis].max(end[axis]) + radius),
    )
}

/// Derived per-pass scratch: XZ cells over this pass's frozen shot query
/// boxes, each holding the vertical span of the boxes that reach it. A body
/// whose box meets no reached cell within that cell's span overlaps no query
/// box, so it can be no shot's candidate on either broad phase. Bounded like
/// `FineGrid`: a sparse or non-finite set of boxes builds no cover and every
/// hittable body is kept.
#[derive(Debug, Default)]
pub(super) struct Cover {
    origin: [i64; 2],
    size: [usize; 2],
    /// `[low, high]` Y per cell; an unreached cell holds an empty span.
    spans: Vec<[f64; 2]>,
}

impl Cover {
    /// Four-block cells: scaling by a power of two is exact, so this is
    /// `floor(value).div_euclid(4)` without the integer division.
    fn cell(value: f64) -> i64 {
        (value * 0.25).floor() as i64
    }

    /// Two-block `FineGrid` cells spanning this built cover plus one cell of
    /// margin on every side for body half widths.
    fn fine_cells(&self) -> usize {
        (self.size[0] + 2).saturating_mul(2) * (self.size[1] + 2).saturating_mul(2)
    }

    fn build(&mut self, boxes: &[([f64; 3], [f64; 3])]) -> bool {
        self.size = [0; 2];
        let mut min = [i64::MAX; 2];
        let mut max = [i64::MIN; 2];
        for (low, high) in boxes {
            if low.iter().chain(high).any(|value| !value.is_finite()) {
                return false;
            }
            for (axis, world_axis) in [0, 2].into_iter().enumerate() {
                min[axis] = min[axis].min(Self::cell(low[world_axis]));
                max[axis] = max[axis].max(Self::cell(high[world_axis]));
            }
        }
        if boxes.is_empty() {
            return false;
        }
        let width = (max[0] - min[0] + 1) as usize;
        let height = (max[1] - min[1] + 1) as usize;
        let Some(cells) = width.checked_mul(height) else {
            return false;
        };
        if cells > FineGrid::MAX_CELLS || cells > boxes.len().saturating_mul(64).max(4096) {
            return false;
        }
        self.origin = min;
        self.size = [width, height];
        self.spans.clear();
        self.spans.resize(cells, [f64::INFINITY, f64::NEG_INFINITY]);
        for (low, high) in boxes {
            for z in Self::cell(low[2])..=Self::cell(high[2]) {
                for x in Self::cell(low[0])..=Self::cell(high[0]) {
                    let index = (z - min[1]) as usize * width + (x - min[0]) as usize;
                    let span = &mut self.spans[index];
                    span[0] = span[0].min(low[1]);
                    span[1] = span[1].max(high[1]);
                }
            }
        }
        true
    }

    /// Whether a body box may overlap some query box. Non-finite bodies are
    /// kept: the broad phases already treat them conservatively.
    fn reaches(&self, min: [f64; 3], max: [f64; 3]) -> bool {
        if min.iter().chain(&max).any(|value| !value.is_finite()) {
            return true;
        }
        let range = |axis: usize, world_axis: usize| {
            (
                (Self::cell(min[world_axis]) - self.origin[axis]).max(0),
                (Self::cell(max[world_axis]) - self.origin[axis]).min(self.size[axis] as i64 - 1),
            )
        };
        let (x0, x1) = range(0, 0);
        let (z0, z1) = range(1, 2);
        (z0..=z1).any(|z| {
            (x0..=x1).any(|x| {
                let span = self.spans[z as usize * self.size[0] + x as usize];
                max[1] >= span[0] && min[1] <= span[1]
            })
        })
    }
}

/// The entity store indexes feet, not bounds. Expand by its retained maximum
/// body width before visiting chunks; callers retain their exact narrow phase.
/// Slot order preserves the exhaustive pickup/riptide order, including swaps.
pub(crate) fn swept_slots(
    state: &TickState,
    start: [f64; 3],
    end: [f64; 3],
    margin: f64,
    slots: &mut Vec<EntitySlot>,
) {
    slots.clear();
    let extent = state.entities.max_half_width() + margin;
    let chunk = |v: f64| mc_physics::mth::floor_i32(v).div_euclid(16);
    let min_x = chunk(start[0].min(end[0]) - extent);
    let max_x = chunk(start[0].max(end[0]) + extent);
    let min_z = chunk(start[2].min(end[2]) - extent);
    let max_z = chunk(start[2].max(end[2]) + extent);
    let cells =
        (i64::from(max_x) - i64::from(min_x) + 1) * (i64::from(max_z) - i64::from(min_z) + 1);
    let indexed =
        state.entities.brain_sensor_chunks().len() + state.entities.villager_chunks().len();
    if cells > indexed as i64 {
        // Long diagonal sweeps/teleports must not enumerate empty world space.
        // These are the same authoritative chunks, not a second spatial index.
        for &pos in state
            .entities
            .brain_sensor_chunks()
            .keys()
            .chain(state.entities.villager_chunks().keys())
        {
            if (min_x..=max_x).contains(&pos.x) && (min_z..=max_z).contains(&pos.z) {
                state
                    .entities
                    .for_each_slot_in_chunk(pos, |slot| slots.push(slot));
            }
        }
    } else {
        for z in min_z..=max_z {
            for x in min_x..=max_x {
                state
                    .entities
                    .for_each_slot_in_chunk(crate::boundary::ChunkPos { x, z }, |slot| {
                        slots.push(slot)
                    });
            }
        }
    }
    slots.sort_unstable_by_key(|slot| slot.0);
    slots.dedup();
}

/// One frozen broad-phase query, before any shot payload can mutate the world.
#[derive(Debug, Clone, Copy)]
struct ShotQuery {
    start: [f64; 3],
    end: [f64; 3],
    radius: f64,
    hook: bool,
}

fn shot_query(state: &TickState, id: EntityId) -> Option<ShotQuery> {
    let slot = state.entities.slot_of(id)?;
    let record = state.entities.row(slot).projectile.as_deref()?;
    if record.stuck || record.kind == ProjectileKind::EyeOfEnder {
        return None;
    }
    let pos = state.entities.pos(slot);
    let velocity = state.entities.vel(slot).map(|v| v / 20.0);
    let radius = crate::physical_attacks::projectile_profile(state, slot)
        .map_or(0.3, |profile| profile.collision_radius);
    if pos.iter().any(|v| !v.is_finite()) || !radius.is_finite() || !(0.0..=128.0).contains(&radius)
    {
        return None;
    }
    let (start, end) = if matches!(record.data, super::ProjectileData::Homing { .. }) {
        (pos.map(|v| v - 128.0), pos.map(|v| v + 128.0))
    } else {
        if velocity.iter().any(|v| !v.is_finite() || v.abs() > 128.0) {
            return None;
        }
        (pos, std::array::from_fn(|axis| pos[axis] + velocity[axis]))
    };
    Some(ShotQuery {
        start,
        end,
        radius,
        hook: record.kind == ProjectileKind::FishingHook,
    })
}

fn query_candidates(
    state: &TickState,
    query: &ShotQuery,
    bodies: &[Hittable],
    exceptional: &[usize],
    grid: Option<&FineGrid>,
    slots: &mut Vec<EntitySlot>,
    candidates: &mut Vec<usize>,
) {
    if let Some(grid) = grid {
        grid.query(bodies, query.start, query.end, query.radius, candidates);
    } else {
        swept_slots(state, query.start, query.end, query.radius, slots);
        candidates.extend(slots.iter().filter_map(|slot| {
            bodies
                .binary_search_by_key(&slot.0, |body| body.slot.map_or(u32::MAX, |slot| slot.0))
                .ok()
        }));
    }
    candidates.extend_from_slice(exceptional);
}

#[cfg(test)]
fn shot_candidates(
    state: &TickState,
    id: EntityId,
    bodies: &[Hittable],
    exceptional: &[usize],
    grid: Option<&FineGrid>,
    slots: &mut Vec<EntitySlot>,
    candidates: &mut Vec<usize>,
) {
    if let Some(query) = shot_query(state, id) {
        query_candidates(state, &query, bodies, exceptional, grid, slots, candidates);
    }
}

pub(crate) fn segment_enters(
    origin: [f64; 3],
    delta: [f64; 3],
    min: [f64; 3],
    max: [f64; 3],
) -> Option<f64> {
    let (mut near, mut far) = (0.0_f64, 1.0_f64);
    for axis in 0..3 {
        if delta[axis].abs() < 1.0e-9 {
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
        } else {
            let a = (min[axis] - origin[axis]) / delta[axis];
            let b = (max[axis] - origin[axis]) / delta[axis];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return None;
            }
        }
    }
    Some(near)
}

/// Exact swept sphere against an axis-aligned box, including rounded corners.
/// Each box-face crossing partitions squared distance into one quadratic.
pub(crate) fn swept_sphere_enters(
    origin: [f64; 3],
    delta: [f64; 3],
    min: [f64; 3],
    max: [f64; 3],
    radius: f64,
) -> Option<f64> {
    if radius == 0.0 {
        return segment_enters(origin, delta, min, max);
    }
    if !radius.is_finite()
        || radius < 0.0
        || origin
            .iter()
            .chain(delta.iter())
            .chain(min.iter())
            .chain(max.iter())
            .any(|v| !v.is_finite())
        || (0..3).any(|axis| min[axis] > max[axis])
    {
        return None;
    }
    let mut cuts = [0.0; 8];
    cuts[1] = 1.0;
    let mut count = 2;
    for axis in 0..3 {
        if delta[axis] == 0.0 {
            continue;
        }
        for face in [min[axis], max[axis]] {
            let crossing = (face - origin[axis]) / delta[axis];
            if crossing > 0.0 && crossing < 1.0 {
                cuts[count] = crossing;
                count += 1;
            }
        }
    }
    cuts[..count].sort_unstable_by(f64::total_cmp);
    for pair in cuts[..count].windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start == end {
            continue;
        }
        let middle = (start + end) * 0.5;
        let (mut quadratic, mut linear, mut constant) = (0.0, 0.0, -radius * radius);
        for axis in 0..3 {
            let at = origin[axis] + delta[axis] * middle;
            let face = if at < min[axis] {
                min[axis]
            } else if at > max[axis] {
                max[axis]
            } else {
                continue;
            };
            let offset = origin[axis] - face;
            quadratic += delta[axis] * delta[axis];
            linear += 2.0 * offset * delta[axis];
            constant += offset * offset;
        }
        if (quadratic * start + linear) * start + constant <= 0.0 {
            return Some(start);
        }
        if quadratic == 0.0 {
            continue;
        }
        let discriminant = linear * linear - 4.0 * quadratic * constant;
        if discriminant < 0.0 {
            continue;
        }
        let entry = (-linear - discriminant.sqrt()) / (2.0 * quadratic);
        if entry >= start && entry <= end {
            return Some(entry);
        }
    }
    None
}

/// Swept ray against the same exact registry shapes as walker collision.
/// Unloaded cells pause flight instead of deleting retained ammunition.
fn first_solid(
    state: &TickState,
    from: [f64; 3],
    step: [f64; 3],
    radius: f64,
) -> Result<Option<(f64, crate::boundary::BlockPos)>, ()> {
    sweep(state, from, step, radius).map(|sweep| sweep.hit)
}

/// One accepted block sweep: the first hit and, when the blocks were read
/// from resident columns, the state of the start cell (`from` floored),
/// which every sweep box contains.
#[derive(Debug)]
struct Sweep {
    hit: Option<(f64, crate::boundary::BlockPos)>,
    start: Option<u32>,
}

// Test-only count of cells whose state a sweep read one by one.
#[cfg(test)]
std::thread_local! {
    static SWEPT_CELLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn take_swept_cells() -> usize {
    SWEPT_CELLS.with(|cells| cells.replace(0))
}

/// The result is a function of the box's cells alone: `Err` when any cell is
/// unloaded, outside the world height or unregistered, otherwise the
/// earliest hit, ties to the first cell in y, z, x order.
///
/// Measured narrow hack (BvArrowPerf receipts). Bottleneck: per-cell reads
/// plus a second chunk-map probe for the water check, once per flying arrow
/// (thousands a tick in battle volleys, mostly over all-air sections).
/// Boundary: overlay-free sweeps; a run of cells inside a single-state
/// section whose state has no collision shape holds nothing to hit and is
/// skipped after one registry check. Fence: the world-read oracle tests
/// `sweep_reads_cells_only_in_sections_that_can_hold_collision_shapes` and
/// `first_solid_column_reads_match_world_reads_before_and_after_folding`
/// (hits, ties, refusals, start cell). Effect, S2 battle micro: sweep and
/// water probe 434 -> 228 profile samples. Removal: drop the `uniform`
/// branch and let `step` read the start cell through `block_at`.
fn sweep(state: &TickState, from: [f64; 3], step: [f64; 3], radius: f64) -> Result<Sweep, ()> {
    if from
        .iter()
        .chain(step.iter())
        .any(|value| !value.is_finite())
        || step.iter().any(|value| value.abs() > 128.0)
        || !radius.is_finite()
        || !(0.0..=128.0).contains(&radius)
    {
        return Err(());
    }
    let end: [f64; 3] = std::array::from_fn(|axis| from[axis] + step[axis]);
    let min: [i32; 3] =
        std::array::from_fn(|axis| (from[axis].min(end[axis]) - radius).floor() as i32);
    let max: [i32; 3] =
        std::array::from_fn(|axis| (from[axis].max(end[axis]) + radius).floor() as i32);
    let below = i32::from(
        state
            .registry
            .max_collision_box_y
            .div_ceil(16)
            .saturating_sub(1),
    );
    let low = min[1].saturating_sub(below);
    let mut first: Option<(f64, crate::boundary::BlockPos)> = None;
    let mut test = |x: i32, y: i32, z: i32, block: u32| -> Result<(), ()> {
        let boxes = state
            .registry
            .collision_boxes
            .get(block as usize)
            .ok_or(())?;
        for collision_box in boxes {
            let local = collision_box.as_blocks();
            let offset = [f64::from(x), f64::from(y), f64::from(z)];
            let lower = std::array::from_fn(|axis| offset[axis] + local[axis]);
            let upper = std::array::from_fn(|axis| offset[axis] + local[axis + 3]);
            if let Some(t) = swept_sphere_enters(from, step, lower, upper, radius)
                && first.is_none_or(|(old, _)| t < old)
            {
                first = Some((t, crate::boundary::BlockPos { x, y, z }));
            }
        }
        Ok(())
    };
    #[cfg(test)]
    let mut read = 0;
    // Unfolded writes keep the overlay-first world read.
    if !state.world.overlay_is_empty() {
        for y in low..=max[1] {
            for z in min[2]..=max[2] {
                for x in min[0]..=max[0] {
                    test(x, y, z, state.world.block_at(x, y, z).ok_or(())?)?;
                    #[cfg(test)]
                    {
                        read += 1;
                    }
                }
            }
        }
        #[cfg(test)]
        SWEPT_CELLS.with(|cells| cells.set(cells.get() + read));
        return Ok(Sweep {
            hit: first,
            start: None,
        });
    }
    // An overlay-free world is read through its resident columns, one map
    // probe per column crossing, one row run per column and section level.
    let start_cell: [i32; 3] = from.map(|value| value.floor() as i32);
    let mut start = None;
    let mut column: Option<(crate::boundary::ChunkPos, &crate::boundary::Chunk)> = None;
    for y in low..=max[1] {
        for z in min[2]..=max[2] {
            let mut x = min[0];
            loop {
                let at = crate::boundary::ChunkPos {
                    x: x.div_euclid(16),
                    z: z.div_euclid(16),
                };
                let chunk = match column {
                    Some((cached, chunk)) if cached == at => chunk,
                    _ => {
                        let chunk = state.world.chunk(at).ok_or(())?;
                        column = Some((at, chunk));
                        chunk
                    }
                };
                let (index, local_y) = chunk.geometry().world_y_to_section(y).ok_or(())?;
                let section = &chunk.sections[index];
                let last = max[0].min(at.x * 16 + 15);
                let start_row =
                    y == start_cell[1] && z == start_cell[2] && (x..=last).contains(&start_cell[0]);
                let uniform = section.palette().is_none().then(|| section.get(0, 0, 0).0);
                if let Some(block) = uniform
                    && state
                        .registry
                        .collision_boxes
                        .get(block as usize)
                        .ok_or(())?
                        .is_empty()
                {
                    if start_row {
                        start = Some(block);
                    }
                } else {
                    let local_z = z.rem_euclid(16) as u8;
                    for x in x..=last {
                        let block = section.get(x.rem_euclid(16) as u8, local_y, local_z).0;
                        if start_row && x == start_cell[0] {
                            start = Some(block);
                        }
                        test(x, y, z, block)?;
                    }
                    #[cfg(test)]
                    {
                        read += (last - x + 1) as usize;
                    }
                }
                if last == max[0] {
                    break;
                }
                x = last + 1;
            }
        }
    }
    #[cfg(test)]
    SWEPT_CELLS.with(|cells| cells.set(cells.get() + read));
    Ok(Sweep { hit: first, start })
}
/// `AbstractArrow.shouldFall`: `noCollision(AABB(pos, pos).inflate(0.06))`,
/// a strict-overlap test against every block collision box. An unloaded or
/// unknown cell keeps the arrow in place. An authored projectile with a
/// collision radius sticks with its centre that radius further from the face
/// (the swept sphere touches first), so its box grows by the same radius.
fn should_fall(state: &TickState, pos: [f64; 3], collision_radius: f64) -> bool {
    let reach = 0.06 + collision_radius;
    if pos.iter().any(|value| !value.is_finite()) {
        return false;
    }
    let lower: [f64; 3] = std::array::from_fn(|axis| pos[axis] - reach);
    let upper: [f64; 3] = std::array::from_fn(|axis| pos[axis] + reach);
    let below = i32::from(
        state
            .registry
            .max_collision_box_y
            .div_ceil(16)
            .saturating_sub(1),
    );
    for y in (lower[1].floor() as i32).saturating_sub(below)..=upper[1].floor() as i32 {
        for z in lower[2].floor() as i32..=upper[2].floor() as i32 {
            for x in lower[0].floor() as i32..=upper[0].floor() as i32 {
                let Some(boxes) = state
                    .world
                    .block_at(x, y, z)
                    .and_then(|block| state.registry.collision_boxes.get(block as usize))
                else {
                    return false;
                };
                let offset = [f64::from(x), f64::from(y), f64::from(z)];
                if boxes.iter().any(|collision_box| {
                    let local = collision_box.as_blocks();
                    (0..3).all(|axis| {
                        offset[axis] + local[axis] < upper[axis]
                            && offset[axis] + local[axis + 3] > lower[axis]
                    })
                }) {
                    return false;
                }
            }
        }
    }
    true
}

/// The synced `IN_GROUND` flag the client uses to stop simulating the arrow.
fn set_in_ground(state: &mut TickState, slot: EntitySlot, in_ground: bool) {
    let extra = &mut Arc::make_mut(&mut state.entities.row_mut(slot).appearance).extra;
    if let Some((_, value)) = extra
        .iter_mut()
        .find(|(index, _)| *index == ARROW_IN_GROUND_INDEX)
    {
        *value = EntityDataValue::Boolean(in_ground);
    } else {
        extra.push((ARROW_IN_GROUND_INDEX, EntityDataValue::Boolean(in_ground)));
    }
}

fn hit_face(
    state: &TickState,
    pos: [f64; 3],
    velocity: [f64; 3],
    hit: crate::boundary::BlockPos,
    time: f64,
) -> Option<[i32; 3]> {
    let id = state.world.block_at(hit.x, hit.y, hit.z)?;
    let boxes = state.registry.collision_boxes.get(id as usize)?;
    let offset = [f64::from(hit.x), f64::from(hit.y), f64::from(hit.z)];
    for collision_box in boxes {
        let bounds = collision_box.as_blocks();
        let lower = std::array::from_fn(|axis| offset[axis] + bounds[axis]);
        let upper = std::array::from_fn(|axis| offset[axis] + bounds[axis + 3]);
        if segment_enters(pos, velocity, lower, upper) != Some(time) {
            continue;
        }
        let mut best = f64::NEG_INFINITY;
        let mut face = [0; 3];
        for axis in 0..3 {
            if velocity[axis].abs() < 1.0e-9 {
                continue;
            }
            let entry = if velocity[axis] > 0.0 {
                lower[axis]
            } else {
                upper[axis]
            };
            let near = (entry - pos[axis]) / velocity[axis];
            if near > best {
                best = near;
                face = [0; 3];
                face[axis] = if velocity[axis] > 0.0 { -1 } else { 1 };
            }
        }
        return Some(face);
    }
    None
}

/// Physical melee shares native block shapes and refuses unloaded segments.
pub(crate) fn clear_segment(state: &TickState, start: [f64; 3], end: [f64; 3]) -> bool {
    let step = std::array::from_fn(|axis| end[axis] - start[axis]);
    matches!(first_solid(state, start, step, 0.0), Ok(None))
}

pub(super) fn run(
    state: &mut TickState,
    intents: &mut Vec<Intent>,
    ids: impl Iterator<Item = EntityId>,
) {
    state.projectiles.retired_crossbows.clear();
    let ids: Vec<_> = ids.collect();
    if ids.is_empty() {
        return;
    }
    let mut clock = std::time::Instant::now();
    let mut lap = || {
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(clock).as_micros() as u64;
        clock = now;
        elapsed
    };
    let queries: Vec<_> = ids.iter().map(|&id| shot_query(state, id)).collect();
    let boxes: Vec<_> = queries
        .iter()
        .flatten()
        .map(|query| query_box(query.start, query.end, query.radius))
        .collect();
    let mut bodies = std::mem::take(&mut state.projectiles.bodies);
    bodies.clear();
    let dense = ids.len() >= FineGrid::MIN_SHOTS;
    let include_items = queries.iter().flatten().any(|query| query.hook);
    let mut cover = std::mem::take(&mut state.projectiles.cover);
    let covered = !boxes.is_empty() && cover.build(&boxes);
    let reach = if covered { cover.fine_cells() } else { 0 };
    let mut slots = Vec::new();
    let mut shot_members = Vec::new();
    let mut shot_ranges = Vec::with_capacity(if dense { 0 } else { ids.len() });
    if dense {
        hittables_from_slots(
            state,
            include_items,
            covered.then_some(&cover),
            &mut bodies,
            state.entities.slots(),
        );
    } else {
        for query in &queries {
            let start = shot_members.len();
            slots.clear();
            if let Some(query) = query {
                swept_slots(state, query.start, query.end, query.radius, &mut slots);
            }
            shot_members.extend_from_slice(&slots);
            shot_ranges.push(start..shot_members.len());
        }
        if queries.iter().any(Option::is_some) {
            let mut selected = state.entities.mob_slots_matching(|type_id| {
                state
                    .registry
                    .blueprint(type_id)
                    .is_some_and(|blueprint| blueprint.name == "minecraft:armor_stand")
            });
            selected.extend_from_slice(&shot_members);
            selected.sort_unstable();
            selected.dedup();
            hittables_from_slots(
                state,
                include_items,
                None,
                &mut bodies,
                selected.into_iter(),
            );
        }
    }
    state.projectiles.cover = cover;
    let hittables_us = lap();
    let mut grid = std::mem::take(&mut state.projectiles.grid);
    let use_grid = ids.len() >= FineGrid::MIN_SHOTS && grid.build(&bodies, reach);
    let exceptional: Vec<_> = bodies
        .iter()
        .enumerate()
        .filter_map(|(index, body)| {
            (body.slot.is_none() || body.deployed_candidate).then_some(index)
        })
        .collect();
    let mut candidates = Vec::new();
    let mut ranges = Vec::with_capacity(ids.len());
    for (index, query) in queries.iter().enumerate() {
        let start = candidates.len();
        if let Some(query) = query {
            if dense {
                if !bodies.is_empty() {
                    query_candidates(
                        state,
                        query,
                        &bodies,
                        &exceptional,
                        use_grid.then_some(&grid),
                        &mut slots,
                        &mut candidates,
                    );
                }
            } else {
                candidates.extend(shot_members[shot_ranges[index].clone()].iter().filter_map(
                    |slot| {
                        bodies
                            .binary_search_by_key(&slot.0, |body| {
                                body.slot.map_or(u32::MAX, |slot| slot.0)
                            })
                            .ok()
                    },
                ));
                candidates.extend_from_slice(&exceptional);
            }
        }
        candidates[start..].sort_unstable();
        // Each query has unique slots, but exceptional stands may also be local.
        let mut end = start;
        for read in start..candidates.len() {
            if end == start || candidates[read] != candidates[end - 1] {
                candidates[end] = candidates[read];
                end += 1;
            }
        }
        candidates.truncate(end);
        ranges.push(start..end);
    }
    let candidates_us = lap();
    let shots = ids.len() as u32;
    run_shots(
        state,
        intents,
        ids.into_iter()
            .zip(ranges)
            .map(|(id, range)| (id, &candidates[range])),
        &bodies,
        &mut slots,
    );
    let post = &mut state.projectiles.post;
    post.hittables_us = hittables_us;
    post.candidates_us = candidates_us;
    post.shots_us = lap();
    post.shots = shots;
    state.projectiles.bodies = bodies;
    state.projectiles.grid = grid;
}

/// Settled arrows need neither a projectile write nor appearance/WAL touch just
/// to advance time. Native admitted entity age already handles active-chunk
/// pauses and restart; pickup and expiry remain ordinary native operations.
fn grounded_arrow(
    state: &mut TickState,
    id: EntityId,
    slot: EntitySlot,
    slots: &mut Vec<EntitySlot>,
) -> bool {
    let row = state.entities.row(slot);
    let Some(record) = row.projectile.as_deref().filter(|record| {
        record.stuck
            && record.grounded_at_age.is_some()
            && matches!(
                record.kind,
                ProjectileKind::Arrow | ProjectileKind::SpectralArrow
            )
    }) else {
        return false;
    };
    let authored = crate::physical_attacks::projectile_profile(state, slot);
    if matches!(record.data, super::ProjectileData::Physical { .. }) && authored.is_none() {
        crate::plugin::fail_native(state);
        return true;
    }
    if should_fall(
        state,
        state.entities.pos(slot),
        authored.map_or(0.0, |profile| profile.collision_radius),
    ) {
        return false;
    }
    if record.ground_age(row.age) >= record.ground_lifetime {
        super::remove(state, id);
        return true;
    }
    if authored.is_some_and(|profile| !profile.recoverable) && record.pickup {
        state
            .entities
            .row_mut(slot)
            .projectile
            .as_mut()
            .unwrap()
            .pickup = false;
        super::touch(state, id);
    }
    let record = state.entities.row(slot).projectile.as_deref().unwrap();
    if !record.pickup || record.stack.is_empty() {
        return true;
    }
    let pos = state.entities.pos(slot);
    swept_slots(state, pos, pos, 1.5, slots);
    if slots
        .iter()
        .any(|&candidate| pickup_candidate(state, record, pos, candidate))
    {
        // Only a real nearby pickup transaction needs owned custody. Idle arrows
        // allocate nothing; reuse the query instead of scanning the cells twice.
        let record = record.clone();
        if pickup_from_slots(state, &record, pos, slots) {
            super::remove(state, id);
        }
    }
    true
}

fn run_shots<'a>(
    state: &mut TickState,
    intents: &mut Vec<Intent>,
    shots: impl Iterator<Item = (EntityId, &'a [usize])>,
    bodies: &[Hittable],
    slots: &mut Vec<EntitySlot>,
) {
    for (id, candidates) in shots {
        let Some(slot) = state.entities.slot_of(id) else {
            continue;
        };
        // A pearl whose owner is offline is saved with the owner: it does not
        // fly, land or age until the owner joins again.
        if state
            .entities
            .row(slot)
            .projectile
            .as_deref()
            .is_some_and(|record| crate::chunk_tickets::parked(state, record))
        {
            continue;
        }
        let ppos = state.entities.pos(slot);
        if !state.world.is_active(
            (ppos[0].floor() as i32).div_euclid(16),
            (ppos[2].floor() as i32).div_euclid(16),
        ) {
            continue;
        }
        if grounded_arrow(state, id, slot, slots) {
            state.projectiles.post.grounded += 1;
            continue;
        }
        let Some(mut boxed) = state.entities.row_mut(slot).projectile.take() else {
            // Vanilla /summon projectile: give it a native retained flight record.
            let EntityKind::Mob { type_id } = state.entities.body(slot).kind else {
                continue;
            };
            let Some(kind) = state
                .registry
                .blueprint(type_id)
                .and_then(|blueprint| ProjectileKind::from_name(&blueprint.name))
            else {
                continue;
            };
            state.entities.row_mut(slot).projectile = Some(Box::new(ProjectileState::new(
                kind,
                None,
                mc_data::ItemStack::EMPTY,
            )));
            super::payloads::initialize(state, id);
            super::touch(state, id);
            continue;
        };
        let record = &mut *boxed;
        record.age = record.age.saturating_add(1);
        let gone = step(state, intents, id, record, bodies, candidates, slots);
        if gone {
            // Removing the row must not discard actual weapon custody before
            // the lethal-hit hooks run during same-tick damage admission.
            if record.piercing > 0
                || (record.fired_from_weapon.is_some()
                    && !state.registry.advancements.definitions.is_empty())
            {
                state.projectiles.retired_crossbows.insert(id, *boxed);
            }
            super::remove(state, id);
        } else if let Some(slot) = state.entities.slot_of(id) {
            super::initialize_appearance(state, slot, record);
            state.entities.row_mut(slot).projectile = Some(boxed);
            super::touch(state, id);
        }
    }
}

/// Attached rockets follow their player's UUID, never a restart-local entity id.
/// Missing/dead players detach; client-side boost does not use terrain collision.
fn attached(state: &mut TickState, id: EntityId, record: &mut ProjectileState) -> bool {
    let Some(target) = record.attached_to else {
        return false;
    };
    let Some(target_slot) = state.entities.slot_of_uuid(target) else {
        record.attached_to = None;
        super::payloads::clear_attachment(state, id);
        return false;
    };
    if !state.entities.body(target_slot).alive {
        record.attached_to = None;
        super::payloads::clear_attachment(state, id);
        return false;
    }
    let Some(slot) = state.entities.slot_of(id) else {
        return true;
    };
    state
        .entities
        .set_pos(slot, state.entities.pos(target_slot));
    // ELYTRA-FIREWORK-BOOST (FireworkRocketEntity.tick, 26.1.2): while
    // attached and the owner is gliding, per axis v += look*0.1 +
    // (look*1.5 - v)*0.5 (look = full 3D look angle; same mth convention as
    // weapon geometry). Input movement recomputes velocity each tick, so the
    // observable effect is the per-tick boost while the rocket burns.
    let gliding = state
        .players
        .get(&state.entities.body(target_slot).id)
        .is_some_and(|p| p.fall_flying && !p.vitals.is_dead());
    if gliding {
        let rot = state.entities.rot(target_slot);
        let (yaw, pitch) = (
            f64::from(rot[0]).to_radians(),
            f64::from(rot[1]).to_radians(),
        );
        let look = [
            -f64::from(mc_physics::mth::sin(yaw)) * f64::from(mc_physics::mth::cos(pitch)),
            -f64::from(mc_physics::mth::sin(pitch)),
            f64::from(mc_physics::mth::cos(yaw)) * f64::from(mc_physics::mth::cos(pitch)),
        ];
        let vel = state.entities.vel(target_slot);
        state.entities.set_vel(
            target_slot,
            std::array::from_fn(|i| vel[i] + look[i] * 0.1 + (look[i] * 1.5 - vel[i]) * 0.5),
        );
        state.entities.set_vel(slot, [0.0; 3]);
    }
    true
}
fn arrow_damage(
    state: &TickState,
    id: EntityId,
    target: EntityId,
    record: &ProjectileState,
    velocity: [f64; 3],
) -> f32 {
    let speed = velocity.iter().map(|v| v * v).sum::<f64>().sqrt();
    let mut amount = if record.kind == ProjectileKind::Trident {
        record.damage + crate::enchantments::target_damage(state, target, &record.stack)
    } else {
        (speed * f64::from(record.damage)).ceil() as f32
    };
    if record.critical {
        let mut rng = crate::natural_spawn::Rng(state.tick ^ id.0 as u64 ^ target.0 as u64);
        amount += (rng.next() % ((amount / 2.0 + 2.0) as u64).max(1)) as f32;
    }
    if record.mob_damage {
        amount = state.difficulty.scale_mob_damage(amount);
    }
    amount
}

fn step(
    state: &mut TickState,
    intents: &mut Vec<Intent>,
    id: EntityId,
    record: &mut ProjectileState,
    bodies: &[Hittable],
    candidates: &[usize],
    slots: &mut Vec<EntitySlot>,
) -> bool {
    if record.kind == ProjectileKind::EyeOfEnder {
        return super::eye::step(state, id, record);
    }
    let is_attached = record.kind == ProjectileKind::Firework
        && record.attached_to.is_some()
        && attached(state, id, record);
    if super::payloads::step(state, intents, id, record) {
        return false;
    }
    if record.kind == ProjectileKind::FishingHook
        && super::fishing::step(state, intents, id, record)
    {
        return false;
    }
    if is_attached {
        return false;
    }
    let Some(slot) = state.entities.slot_of(id) else {
        return true;
    };
    let pos = state.entities.pos(slot);
    let authored = crate::physical_attacks::projectile_profile(state, slot);
    if matches!(record.data, super::ProjectileData::Physical { .. }) && authored.is_none() {
        crate::plugin::fail_native(state);
        return false;
    }
    if let Some(profile) = authored {
        record.pickup &= profile.recoverable;
    }
    if loyalty(state, id, slot, record, pos) {
        return state.entities.slot_of(id).is_none();
    }
    if record.stuck {
        // AbstractArrow.tick: an in-ground arrow whose box inflated by 0.06
        // no longer collides (`shouldFall`) starts falling: in-ground cleared,
        // its zero motion scaled, `life` reset, and no despawn tick.
        if should_fall(
            state,
            pos,
            authored.map_or(0.0, |profile| profile.collision_radius),
        ) {
            record.stuck = false;
            record.age = 0;
            record.grounded_at_age = None;
            state.entities.set_ground(slot, false);
            set_in_ground(state, slot, false);
            return false;
        }
        if record.age >= record.ground_lifetime {
            return true;
        }
        return pickup(state, record, pos, slots);
    }
    // Vanilla ThrowableProjectile has no lifetime: a pearl may hover for ever.
    if record.kind != ProjectileKind::EnderPearl
        && record.age > authored.map_or(1200, |profile| profile.lifetime_ticks)
    {
        return true;
    }
    if let super::ProjectileData::Homing { target } = record.data
        && let Some(target_slot) = state.entities.slot_of_uuid(target)
        && state.entities.body(target_slot).alive
    {
        let mut at = state.entities.pos(target_slot);
        at[1] += state.entities.body(target_slot).height * 0.5;
        let aim = crate::special_mobs::homing_velocity(pos, at);
        let current = state.entities.vel(slot).map(|value| value / 20.0);
        state.entities.set_vel(
            slot,
            std::array::from_fn(|axis| (current[axis] + (aim[axis] - current[axis]) * 0.2) * 20.0),
        );
    }
    let velocity = state.entities.vel(slot).map(|value| value / 20.0);
    let collision_radius = authored.map_or(0.0, |profile| profile.collision_radius);
    let Ok(Sweep {
        hit: block_hit,
        start: start_block,
    }) = sweep(state, pos, velocity, collision_radius)
    else {
        return false;
    };
    let reach = block_hit.map_or(1.0, |(t, _)| t);
    let owner = super::owner_id(state, record);
    let battle_faction = if record.kind.arrow() {
        owner
            .and_then(|owner| state.entities.slot_of_any(owner))
            .map(|slot| state.entities.body(slot))
            .filter(|body| matches!(body.kind, EntityKind::Mob { .. }))
            .map_or(0, |body| body.faction)
    } else {
        0
    };
    if !(record.kind == ProjectileKind::Trident && (!record.hit.is_empty() || record.returning)) {
        loop {
            let victim = candidates
                .iter()
                .map(|&index| &bodies[index])
                .filter(|body| !body.item || record.kind == ProjectileKind::FishingHook)
                .filter(|body| Some(body.uuid) != record.owner && !record.hit.contains(&body.uuid))
                // Battle volleys cross their own ranks just as a scoreboard
                // team with friendlyFire=false does; ordinary arrows and player
                // targets retain the existing collision and scoreboard rules.
                .filter(|body| battle_faction == 0 || body.battle_faction != battle_faction)
                .filter(|body| {
                    owner.is_none_or(|owner| {
                        crate::scoreboard::permits_player_damage(state, owner, body.id)
                    })
                })
                .filter_map(|body| {
                    let candidate = if body.deployed_candidate {
                        state.entities.slot_of_any(body.id)
                    } else {
                        None
                    };
                    let hit = candidate.and_then(|candidate| {
                        crate::plugin_npcs::deployed_projectile_hit(
                            state,
                            candidate,
                            pos,
                            std::array::from_fn(|axis| pos[axis] + velocity[axis]),
                            authored.map_or(state.entities.body(slot).half_width, |profile| {
                                profile.collision_radius
                            }),
                        )
                    });
                    hit.unwrap_or_else(|| {
                        if let Some(profile) = authored {
                            swept_sphere_enters(
                                pos,
                                velocity,
                                body.min,
                                body.max,
                                profile.collision_radius,
                            )
                        } else {
                            segment_enters(
                                pos,
                                velocity,
                                body.min.map(|value| value - 0.3),
                                body.max.map(|value| value + 0.3),
                            )
                        }
                    })
                    .filter(|t| *t <= reach)
                    .map(|t| (t, body, hit.and(candidate)))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
            let Some((t, body, deployed)) = victim else {
                break;
            };
            let impact = std::array::from_fn(|axis| pos[axis] + velocity[axis] * t);
            let physical_impact = if authored.is_some() {
                let physical = state.entities.slot_of_uuid(body.uuid).and_then(|target| {
                    crate::physical_attacks::projectile_hit(state, slot, target, impact)
                });
                let Some(physical) = physical else {
                    record.hit.push(body.uuid);
                    if record.kind == ProjectileKind::Trident {
                        state.entities.set_pos(slot, impact);
                        super::set_flight(state, slot, velocity.map(|value| value * -0.01));
                        return false;
                    }
                    if record.piercing == 0 || record.hit.len() > record.piercing as usize {
                        return true;
                    }
                    continue;
                };
                Some(physical)
            } else {
                None
            };
            if let Some(candidate) = deployed
                && record.kind.arrow()
            {
                let physical = physical_impact.unwrap_or_else(|| {
                    let direct_type_id = match state.entities.body(slot).kind {
                        EntityKind::Mob { type_id } => type_id,
                        _ => 0,
                    };
                    let piece = crate::tick::DamagePiece {
                        amount: arrow_damage(state, id, body.id, record, velocity),
                        kind: DamageKind::Projectile,
                        source: Some(DamageSource::Projectile {
                            projectile: id,
                            direct_type_id,
                            owner: owner.unwrap_or(EntityId(0)),
                            velocity: [velocity[0], velocity[2]],
                            punch: record.punch,
                            flame: record.flame,
                        }),
                        physical: None,
                        custom_armor: false,
                    };
                    let mut native =
                        crate::physical_defense::impact_from_piece(state, candidate, &piece)
                            .expect("native arrow physical impact");
                    native.point = impact;
                    native.collision_radius = state.entities.body(slot).half_width;
                    native
                });
                if let Some(blocked) =
                    crate::plugin_npcs::adjudicate_deployed_projectile(state, candidate, &physical)
                {
                    record.hit.push(body.uuid);
                    if blocked {
                        return true;
                    }
                    continue;
                }
            }
            if record.kind == ProjectileKind::FishingHook {
                super::fishing::impact(state, id, record, Some(body.id), impact);
                return false;
            }
            if record.kind.arrow() && crate::enderman::projectile_hit(state, body.id) {
                record.hit.push(body.uuid);
                state.entities.set_pos(slot, impact);
                super::set_flight(state, slot, velocity.map(|v| v * -0.1));
                record.critical = false;
                return false;
            }
            if !record.kind.arrow() {
                state.entities.set_pos(slot, impact);
                return super::payloads::impact(state, intents, id, record, Some(body.id), impact);
            }
            record.hit.push(body.uuid);
            if let Some(physical) = physical_impact {
                let seq = state.next_seq(id);
                let direct_type_id = match state.entities.body(slot).kind {
                    EntityKind::Mob { type_id } => type_id,
                    _ => 0,
                };
                intents.push(Intent {
                    actor: id,
                    seq,
                    kind: IntentKind::PhysicalDamageEntity {
                        target: body.id,
                        impact: physical,
                        source: Some(DamageSource::Projectile {
                            projectile: id,
                            direct_type_id,
                            owner: owner.unwrap_or(EntityId(0)),
                            velocity: [velocity[0], velocity[2]],
                            punch: record.punch,
                            flame: record.flame,
                        }),
                    },
                    footprint: Footprint {
                        entity_writes: vec![body.id],
                        ..Default::default()
                    },
                });
            } else if record.kind.arrow()
                && crate::item_frame::eject_on_projectile(state, body.id, owner)
            {
                // Vanilla occupied frame: first non-explosive hit ejects the
                // displayed item and leaves the frame attached. The arrow is
                // consumed without frame damage.
                state.projectiles.pending_arrow_removals.insert(id);
                state.entities.set_pos(slot, impact);
                return false;
            } else if super::payloads::breaks_armor_stand(state, body.id)
                && (record.kind.arrow() || record.kind == ProjectileKind::Trident)
            {
                // Vanilla ArmorStand.hurtServer: arrows and tridents always
                // break the stand outright (ALWAYS_KILLS_ARMOR_STANDS tag).
                super::damage(
                    state,
                    intents,
                    id,
                    body.id,
                    1000.0,
                    DamageKind::Projectile,
                    Some(owner.unwrap_or(EntityId(0))),
                    velocity,
                );
            } else {
                let amount = arrow_damage(state, id, body.id, record, velocity);
                super::damage(
                    state,
                    intents,
                    id,
                    body.id,
                    amount,
                    DamageKind::Projectile,
                    Some(owner.unwrap_or(EntityId(0))),
                    velocity,
                );
            }
            super::embedded_projectiles::queue_arrow_hit(state, id, body.id, record);
            if let Some(Intent {
                kind:
                    IntentKind::DamageEntity {
                        source: Some(DamageSource::Projectile { punch, flame, .. }),
                        ..
                    },
                ..
            }) = intents.last_mut()
            {
                *punch = record.punch;
                *flame = record.flame;
            }
            super::payloads::arrow_effects(state, intents, id, record, body.id);
            if record.kind == ProjectileKind::Trident {
                let _ = super::payloads::channeling(state, intents, id, record, impact);
                state.entities.set_pos(slot, impact);
                super::set_flight(state, slot, velocity.map(|v| v * -0.01));
                return false;
            }
            if record.piercing == 0 {
                state.projectiles.pending_arrow_removals.insert(id);
                state.entities.set_pos(slot, impact);
                return false;
            }
            if record.hit.len() > record.piercing as usize {
                return true;
            }
        }
    }
    if let Some((t, block_position)) = block_hit {
        let impact = std::array::from_fn(|axis| pos[axis] + velocity[axis] * t);
        state.entities.set_pos(slot, impact);
        crate::weather::lightning::channel_rod(state, record, block_position);
        let owner = super::owner_id(state, record);
        crate::vibration::emit(
            state,
            impact,
            crate::vibration::GameEvent::ProjectileLand,
            owner,
        );
        let face = hit_face(state, pos, velocity, block_position, t);
        super::block_impact::react(state, slot, record, block_position, impact, face, velocity);
        if record.kind == ProjectileKind::FishingHook {
            super::fishing::impact(state, id, record, None, impact);
            return false;
        }
        if !record.kind.arrow() {
            if matches!(
                record.kind,
                ProjectileKind::Potion | ProjectileKind::LingeringPotion
            ) && super::payloads::is_water_bottle(&record.stack)
                && let Some(face) = face
            {
                super::payloads::water_douse_blocks(state, block_position, face);
            }
            return super::payloads::impact(state, intents, id, record, None, impact);
        }
        // Target blocks charge from projectile strikes (vanilla output).
        let arrows = record.kind.arrow();
        crate::redstone::charge_target(state, block_position, impact, velocity, arrows);
        record.stuck = true;
        record.critical = false;
        record.age = 0;
        if matches!(
            record.kind,
            ProjectileKind::Arrow | ProjectileKind::SpectralArrow
        ) {
            record.grounded_at_age = Some(state.entities.row(slot).age);
        }
        // Vanilla AbstractArrow.onHitBlock retracts the stuck position by
        // signum(deltaMovement) * 0.05 per axis (26.3 javap:
        // setPos(position().subtract(signum.scale(0.05)))), so the entity sits in
        // the cell in front of the block face. The raw impact stays the source
        // for the land vibration and target charging above.
        let retracted = std::array::from_fn(|axis| {
            // Java Math.signum maps zero to zero.
            let signum = if velocity[axis] > 0.0 {
                1.0
            } else if velocity[axis] < 0.0 {
                -1.0
            } else {
                0.0
            };
            impact[axis] - signum * 0.05
        });
        state.entities.set_pos(slot, retracted);
        state.entities.set_vel(slot, [0.0; 3]);
        state.entities.set_ground(slot, true);
        set_in_ground(state, slot, true);
        return false;
    }
    state
        .entities
        .set_pos(slot, std::array::from_fn(|axis| pos[axis] + velocity[axis]));
    if let Some(profile) = authored {
        let mut next = velocity;
        if !state.entities.row(slot).controls.no_gravity {
            next[1] -= profile.gravity;
        }
        super::set_flight(state, slot, next);
        return false;
    }
    // Every write lands in the overlay: still empty, no write has replaced
    // the start cell since the sweep read it.
    let block = match start_block {
        Some(block) if state.world.overlay_is_empty() => Some(block),
        _ => state.world.block_at(
            pos[0].floor() as i32,
            pos[1].floor() as i32,
            pos[2].floor() as i32,
        ),
    };
    let water =
        block.is_some_and(|block| state.registry.materials.classify(block) == BlockMaterial::Water);
    let (drag, gravity) = match record.kind {
        ProjectileKind::Arrow | ProjectileKind::SpectralArrow => {
            (if water { 0.6 } else { 0.99 }, 0.05)
        }
        ProjectileKind::Trident => (0.99, 0.05),
        ProjectileKind::Potion | ProjectileKind::LingeringPotion => (0.99, 0.05),
        ProjectileKind::ExperienceBottle => (0.99, 0.07),
        ProjectileKind::Firework => (1.0, 0.0),
        ProjectileKind::SmallFireball
        | ProjectileKind::Fireball
        | ProjectileKind::DragonFireball
        | ProjectileKind::WitherSkull => (if water { 0.8 } else { 0.95 }, 0.0),
        ProjectileKind::WindCharge
        | ProjectileKind::BreezeWindCharge
        | ProjectileKind::ShulkerBullet => (1.0, 0.0),
        _ => (if water { 0.8 } else { 0.99 }, 0.03),
    };
    let speed = velocity
        .iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    let mut next = std::array::from_fn(|axis| {
        let acceleration = if speed > 1.0e-9 {
            velocity[axis] / speed * record.acceleration_power
        } else {
            0.0
        };
        (velocity[axis] + acceleration) * drag
    });
    if !state.entities.row(slot).controls.no_gravity {
        next[1] -= gravity;
    }
    if record.kind == ProjectileKind::Firework && !record.shot_at_angle {
        next[0] *= 1.15;
        next[2] *= 1.15;
        next[1] += 0.04;
    }
    super::set_flight(state, slot, next);
    false
}

fn pickup(
    state: &mut TickState,
    record: &ProjectileState,
    pos: [f64; 3],
    slots: &mut Vec<EntitySlot>,
) -> bool {
    if !record.pickup || record.stack.is_empty() {
        return false;
    }
    swept_slots(state, pos, pos, 1.5, slots);
    pickup_from_slots(state, record, pos, slots)
}

fn pickup_candidate(
    state: &TickState,
    record: &ProjectileState,
    pos: [f64; 3],
    slot: EntitySlot,
) -> bool {
    let body = state.entities.body(slot);
    body.alive
        && state
            .players
            .get(&body.id)
            .is_some_and(|player| !player.spectator())
        && state
            .entities
            .pos(slot)
            .iter()
            .zip(pos)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            < 2.25
        && (record.kind != ProjectileKind::Trident
            || super::owner_id(state, record) == Some(body.id))
}

fn pickup_from_slots(
    state: &mut TickState,
    record: &ProjectileState,
    pos: [f64; 3],
    slots: &[EntitySlot],
) -> bool {
    for &slot in slots {
        if pickup_candidate(state, record, pos, slot)
            && give(state, state.entities.body(slot).id, record.stack.clone())
        {
            return true;
        }
    }
    false
}

fn give(state: &mut TickState, player: EntityId, mut stack: mc_data::ItemStack) -> bool {
    let rules = state.registry.item_rules();
    let Some(player) = state.players.get_mut(&player) else {
        return false;
    };
    player.inventory.add(&rules, &mut stack);
    if !stack.is_empty() {
        return false;
    }
    let connection = player.connection;
    let change = player.inventory_contents();
    state.push_player(connection, change);
    true
}

fn loyalty(
    state: &mut TickState,
    id: EntityId,
    slot: EntitySlot,
    record: &mut ProjectileState,
    pos: [f64; 3],
) -> bool {
    if record.kind != ProjectileKind::Trident {
        return false;
    }
    let level = record
        .stack
        .enchantments
        .iter()
        .filter(|enchantment| enchantment.id.as_str() == "minecraft:loyalty")
        .map(|enchantment| enchantment.level.max(0) as u32)
        .max()
        .unwrap_or(0);
    if level == 0 || (!record.stuck && record.hit.is_empty() && !record.returning) {
        return false;
    }
    let Some(owner) = super::owner_id(state, record) else {
        return false;
    };
    let Some(owner_slot) = state.entities.slot_of(owner) else {
        return false;
    };
    if state
        .players
        .get(&owner)
        .is_some_and(|player| player.spectator())
    {
        return false;
    }
    let target = state.entities.pos(owner_slot);
    let delta = [
        target[0] - pos[0],
        target[1] + 1.0 - pos[1],
        target[2] - pos[2],
    ];
    let distance = delta.iter().map(|v| v * v).sum::<f64>().sqrt();
    record.returning = true;
    record.stuck = false;
    if distance < 1.5 {
        if !record.pickup || give(state, owner, record.stack.clone()) {
            super::remove(state, id);
        }
        return true;
    }
    let old = state.entities.vel(slot).map(|v| v / 20.0);
    let speed = 0.05 * f64::from(level);
    let velocity = std::array::from_fn(|axis| old[axis] * 0.95 + delta[axis] / distance * speed);
    state
        .entities
        .set_pos(slot, std::array::from_fn(|axis| pos[axis] + velocity[axis]));
    super::set_flight(state, slot, velocity);
    true
}
