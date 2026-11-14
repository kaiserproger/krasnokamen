//! Sparse per-team regiment distance samples plus bounded local steering.
//! The battlefield is natural relief: soldiers keep the formation's
//! horizontal drive, physics jumps one-block steps and gravity takes them down
//! slopes; a drive into a trunk or a wall of two or more blocks turns aside
//! ([`super::ground::deflect`]). There is no per-soldier path search.
//! Nearest front points are shared for ten ticks (AoE4 virtual-leader pattern).
//! Separation: PSCrowd's five nearest neighbours, capped cell inspection;
//! impulses use the existing mc-physics contact kernel, never mutate neighbours.

use super::formation::{FormationRuntime, Member, Regiment};
use super::{BattleState, Frame, Output};
use crate::entities::EntitySlot;
use mc_physics::entity_collision_26_1_2::{EntityPushInput, PushRecipient, vanilla_push_impulse};
use mc_physics::mth;
use rayon::prelude::*;

pub(crate) const NEIGHBOURS: usize = 5;
const CELL_CAP: usize = 8;
pub(crate) const MIN_SPACING: f64 = 0.65;
const SENSE: f64 = 1.5;
/// A body is crowded when another stands within this distance (blocks).
const CROWDED: f64 = 0.8;
/// A crowded body does not pull toward a slot this close (blocks); beyond it
/// the pull grows from zero.
const DEAD_ZONE: f64 = 0.15;
/// A crowded body gives up this much speed (blocks per tick) and so stands
/// rather than creep: its drive shrinks by it, to nothing below it.
const CROWD_BRAKE: f64 = 0.01;
/// Depth inside MIN_SPACING (blocks) over which the separation push grows from
/// nothing to its full vanilla strength. A body this deep in another is packed.
const SEPARATION_RAMP: f64 = 0.2;
const DRIVE_CHUNK: usize = 256;
/// Height apart, in blocks, within which bodies see each other: a body walks off
/// a drop of up to MAX_DROP blocks, from a one-block step down or from the top of
/// a step-up (its feet rest a little above the block: 64.02, 64.17), so one that
/// much lower stands where it may land. At exactly MAX_DROP, fugitives stepping
/// off a terrace ledge fell onto one point (111 in a terrace corner of
/// rehearsal-06198df4), and with it, from 2.02 above into a gully end (58-62 in
/// the rehearsals of 07222297).
pub(crate) const LEVELS: f64 = super::ground::MAX_DROP + 1.0;

/// CSR over occupied two-block cells. `member_cell[bi]` and the nine adjacent
/// cell ranges are prepared once; steering never hashes or searches cell keys.
/// Compact occupied cells avoid allocating a huge rectangle between distant
/// regiments. Sorting by (cell, bi) preserves roster order inside every cell.
#[derive(Debug, Default)]
pub(crate) struct SeparationGrid {
    keys: Vec<[i32; 2]>,
    offsets: Vec<usize>,
    items: Vec<usize>,
    member_cell: Vec<usize>,
    fine_cells: Vec<[i32; 2]>,
    adjacent: Vec<[usize; 9]>,
    entries: Vec<([i32; 2], usize)>,
    pub overflow: u64,
}

fn cell(p: [f64; 3]) -> [i32; 2] {
    [p[0].floor() as i32, p[2].floor() as i32]
}

impl SeparationGrid {
    pub fn rebuild(&mut self, members: &[Member]) {
        self.keys.clear();
        self.offsets.clear();
        self.items.clear();
        self.entries.clear();
        // Any later spread/rout still has at most N occupied cells. Warm to
        // that bound once, not again when a tight formation disperses.
        self.keys.reserve(members.len());
        self.offsets.reserve(members.len() + 1);
        self.items.reserve(members.len());
        self.entries.reserve(members.len());
        self.adjacent
            .reserve(members.len().saturating_sub(self.adjacent.len()));
        self.member_cell.resize(members.len(), usize::MAX);
        self.member_cell.fill(usize::MAX);
        self.fine_cells.resize(members.len(), [0; 2]);
        self.overflow = 0;
        for (bi, member) in members.iter().enumerate().filter(|(_, m)| m.alive) {
            let fine = cell(member.pos);
            self.fine_cells[bi] = fine;
            // Keys sort by z then x, so translating a whole key stream keeps
            // it sorted and adjacent-cell ranges can be built by linear merge.
            self.entries
                .push(([fine[1].div_euclid(2), fine[0].div_euclid(2)], bi));
        }
        self.entries.sort_unstable();
        let mut previous = None;
        let mut counts = [0_u8; 4];
        for &(key, bi) in &self.entries {
            if previous != Some(key) {
                self.keys.push(key);
                self.offsets.push(self.items.len());
                counts.fill(0);
                previous = Some(key);
            }
            self.member_cell[bi] = self.keys.len() - 1;
            let fine = self.fine_cells[bi];
            let subcell = (fine[0].rem_euclid(2) + 2 * fine[1].rem_euclid(2)) as usize;
            // Keep the old first-eight-per-one-block-cell policy exactly:
            // CSR must change access cost, not dense-overlap behavior.
            if usize::from(counts[subcell]) < CELL_CAP {
                self.items.push(bi);
                counts[subcell] += 1;
            } else {
                self.overflow += 1;
            }
        }
        self.offsets.push(self.items.len());
        self.adjacent.resize(self.keys.len(), [usize::MAX; 9]);
        self.adjacent.fill([usize::MAX; 9]);
        for dz in -1..=1 {
            for dx in -1..=1 {
                let direction = ((dz + 1) * 3 + dx + 1) as usize;
                let mut next = 0;
                for own in 0..self.keys.len() {
                    let key = self.keys[own];
                    let wanted = [key[0] + dz, key[1] + dx];
                    while next < self.keys.len() && self.keys[next] < wanted {
                        next += 1;
                    }
                    if self.keys.get(next) == Some(&wanted) {
                        self.adjacent[own][direction] = next;
                    }
                }
            }
        }
    }

    pub(super) fn nearest(
        &self,
        index: usize,
        members: &[Member],
    ) -> ([(f64, usize); NEIGHBOURS], u64) {
        self.nearest_where(index, members, SENSE, |_| true)
    }

    /// [`Self::nearest`] within `radius` (at most 2 blocks, the candidate
    /// window) among the bodies `keep` accepts: the horses round a rider in a
    /// melee, where the five nearest bodies are often all foot.
    pub(super) fn nearest_where(
        &self,
        index: usize,
        members: &[Member],
        radius: f64,
        keep: impl Fn(&Member) -> bool,
    ) -> ([(f64, usize); NEIGHBOURS], u64) {
        let actor = &members[index];
        let mut best = [(f64::INFINITY, usize::MAX); NEIGHBOURS];
        let mut examined = 0;
        let Some(&own) = self.member_cell.get(index).filter(|&&c| c != usize::MAX) else {
            return (best, examined);
        };
        let fine = self.fine_cells[index];
        // Nine direct CSR ranges, at most 32 packed indices per macro-cell.
        for &adjacent in &self.adjacent[own] {
            if adjacent == usize::MAX {
                continue;
            }
            for &other in &self.items[self.offsets[adjacent]..self.offsets[adjacent + 1]] {
                if other == index {
                    continue;
                }
                let candidate_cell = self.fine_cells[other];
                // Preserve the legacy +/-2 one-block-cell candidate window,
                // including its exact examined counter and boundary behavior.
                if candidate_cell[0].abs_diff(fine[0]) > 2
                    || candidate_cell[1].abs_diff(fine[1]) > 2
                {
                    continue;
                }
                examined += 1;
                let m = &members[other];
                if (actor.pos[1] - m.pos[1]).abs() > LEVELS {
                    continue;
                }
                let d2 = (actor.pos[0] - m.pos[0]).powi(2) + (actor.pos[2] - m.pos[2]).powi(2);
                if d2 > radius * radius || !keep(m) {
                    continue;
                }
                let Some(at) = best.iter().position(|&(distance, old)| {
                    d2 < distance
                        || (d2 == distance && (old == usize::MAX || m.id < members[old].id))
                }) else {
                    continue;
                };
                for k in (at + 1..NEIGHBOURS).rev() {
                    best[k] = best[k - 1];
                }
                best[at] = (d2, other);
            }
        }
        (best, examined)
    }
}

pub(crate) fn unit(delta: [f64; 2], fallback: [f64; 2]) -> [f64; 2] {
    let length = mth::length2(delta[0], delta[1]);
    if length < 1e-9 {
        fallback
    } else {
        delta.map(|v| v / length)
    }
}

/// Distance field sampled only at regiment leaders, with one seed segment per
/// enemy front. O(R²/10), bounded R<=128; at 20x1000 men it is 20 samples/tick,
/// not 20,000 long-range target scans. Clamping to a front segment (not its
/// centroid) preserves parallel frontage instead of converging every file.
pub(crate) fn nearest_front(
    regiments: &[Regiment],
    actor: usize,
) -> (Option<usize>, Option<[f64; 3]>, bool, u64) {
    let own = &regiments[actor];
    let mut nearest = None;
    let mut selected = None;
    let mut routing = false;
    let mut best = f64::INFINITY;
    let mut best_id = u32::MAX;
    let mut examined = 0;
    for (index, enemy) in regiments
        .iter()
        .enumerate()
        .filter(|(_, r)| r.spec.faction != own.spec.faction && r.removed < r.len)
    {
        examined += 1;
        let right = [enemy.spec.facing[1], -enemy.spec.facing[0]];
        let delta = [
            own.spec.origin[0] - enemy.spec.origin[0],
            own.spec.origin[2] - enemy.spec.origin[2],
        ];
        let half_width = f64::from(enemy.spec.files - 1) * enemy.interval * 0.5;
        let lateral = (delta[0] * right[0] + delta[1] * right[1]).clamp(-half_width, half_width);
        let p = [
            enemy.spec.origin[0] + right[0] * lateral,
            enemy.spec.origin[1],
            enemy.spec.origin[2] + right[1] * lateral,
        ];
        let d2 = (own.spec.origin[0] - p[0]).powi(2) + (own.spec.origin[2] - p[2]).powi(2);
        if d2 < best || (d2 == best && enemy.spec.id < best_id) {
            best = d2;
            best_id = enemy.spec.id;
            nearest = Some(p);
            selected = Some(index);
            routing = enemy.morale.broken;
        }
    }
    (selected, nearest, routing, examined)
}

#[cfg(test)]
pub(crate) fn movement(runtime: &mut FormationRuntime, index: usize) -> [f64; 3] {
    if !runtime.members[index].alive {
        return [0.0; 3];
    }
    let (near, examined) = runtime.neighbours.nearest(index, &runtime.members);
    movement_with_neighbours(runtime, index, near, examined)
}

#[cfg(test)]
fn movement_with_neighbours(
    runtime: &mut FormationRuntime,
    index: usize,
    near: [(f64, usize); NEIGHBOURS],
    examined: u64,
) -> [f64; 3] {
    let (drive, pushes) = drive_with_neighbours(runtime, index, near);
    runtime.work.separation_candidates += examined;
    runtime.work.push_calls += pushes;
    drive
}

/// A rider is turned through its horse in cavalry (its own feet are a seat,
/// not the ground); every body on foot, a dismounted rider too, here.
fn sample_movement(
    frame: &Frame<'_>,
    runtime: &FormationRuntime,
    index: usize,
) -> ([f64; 3], u64, u64) {
    let m = &runtime.members[index];
    if !m.alive {
        return ([0.0; 3], 0, 0);
    }
    let (near, examined) = runtime.neighbours.nearest(index, &runtime.members);
    let (mut drive, pushes) = drive_with_neighbours(runtime, index, near);
    if let Some(slot) = m.slot
        && frame.entities.row(slot).entity_life.mount.vehicle.is_none()
    {
        let target = runtime.heading_target(index);
        let heading = [target[0] - m.pos[0], target[2] - m.pos[2]];
        let turned = around_obstacles(frame, runtime, slot, drive, heading, Mover::Foot);
        // A terrain turn (the clear way along a wall, at the drive's speed)
        // replaces the drive the gap was kept for. Keep it again, or every body
        // following one wall walks into the one ahead and the wall's end gathers
        // them on one point: 87 fugitives stood within 0.22 blocks of a cliff
        // notch in the G2 take. Under rock the walk out toward open sky comes
        // first: a pile packed into a tunnel would never leave it.
        drive = if turned == drive
            || super::ground::under_rock(frame.world, frame.registry, frame.entities.pos(slot))
        {
            turned
        } else {
            let tick = mc_physics::TICK_SECONDS;
            let step = keep_gap(
                &runtime.members,
                index,
                &near,
                MIN_SPACING,
                [turned[0] * tick, turned[2] * tick],
            );
            runtime.constrain_velocity(m.pos, [step[0] / tick, turned[1], step[1] / tick])
        };
    }
    (drive, examined, pushes)
}

/// Which terrain rules turn a body ([`around_obstacles`]).
pub(crate) enum Mover<'a> {
    /// On foot: boxed in, it stands ([`super::ground::deflect_foot`]).
    Foot,
    /// A horse with a target or a slot ([`super::ground::deflect`]).
    Rider,
    /// A fleeing horse and its way out of a pocket
    /// ([`super::ground::deflect_mounted`]).
    Fugitive(&'a mut Option<super::ground::Escape>),
}

/// Terrain turn for a final physics velocity; `heading` is where the body is
/// going without crowd pushes or lateral dressing. Bounds are re-applied only
/// when the drive turned, so an unobstructed drive stays bit-identical.
pub(crate) fn around_obstacles(
    frame: &Frame<'_>,
    runtime: &FormationRuntime,
    slot: EntitySlot,
    velocity: [f64; 3],
    heading: [f64; 2],
    mover: Mover<'_>,
) -> [f64; 3] {
    let pos = frame.entities.pos(slot);
    let drive = [velocity[0], velocity[2]];
    let half_width = frame.entities.body(slot).half_width;
    let (world, registry) = (frame.world, frame.registry);
    let turned = match mover {
        Mover::Foot => super::ground::deflect_foot(world, registry, pos, half_width, drive, heading, 1.0),
        Mover::Rider => super::ground::deflect(world, registry, pos, half_width, drive, heading, 1.0),
        Mover::Fugitive(escape) => super::ground::deflect_mounted(
            world,
            registry,
            pos,
            half_width,
            drive,
            heading,
            1.0,
            escape,
            frame.tick,
        ),
    };
    if turned == drive {
        return velocity;
    }
    runtime.constrain_velocity(pos, [turned[0], velocity[1], turned[1]])
}

fn drive_with_neighbours(
    runtime: &FormationRuntime,
    index: usize,
    near: [(f64, usize); NEIGHBOURS],
) -> ([f64; 3], u64) {
    let m = &runtime.members[index];
    let (mut target, max_step) = runtime.destination(index);
    if let Some((min, max)) = runtime.bounds {
        target[0] = target[0].clamp(min[0], max[0]);
        target[2] = target[2].clamp(min[1], max[1]);
    }
    let delta = [target[0] - m.pos[0], target[2] - m.pos[2]];
    let length = mth::length2(delta[0], delta[1]);
    // A soldier with a neighbour inside CROWDED does not chase a slot closer
    // than DEAD_ZONE (no pull inside it, a ramp outside) and stands rather
    // than creep under CROWD_BRAKE. Routers and the wounded dress to exact spots.
    let settling = !m.routing
        && !m.wounded
        && near
            .iter()
            .any(|&(distance, other)| other != usize::MAX && distance < CROWDED * CROWDED);
    let mut drive = if settling {
        let pull = (length - DEAD_ZONE).clamp(0.0, max_step);
        delta.map(|v| v * pull / length.max(f64::MIN_POSITIVE))
    } else {
        delta.map(|v| v * (max_step / length.max(max_step)).min(1.0))
    };
    let mut pushes = 0;
    // Inside the whole ramp: a heap or an overlap, which must keep unstacking
    // however slowly.
    let mut packed = false;
    for &(distance, other) in &near {
        if other == usize::MAX || distance >= MIN_SPACING * MIN_SPACING {
            continue;
        }
        let contact = &runtime.members[other];
        let mut separation = [contact.pos[0] - m.pos[0], contact.pos[2] - m.pos[2]];
        if distance < 1e-8 {
            // Vanilla intentionally returns zero for exact overlap. This named
            // battle-only tie-break unsticks coincident bodies symmetrically.
            separation = if m.id < contact.id {
                [0.01, 0.0]
            } else {
                [-0.01, 0.0]
            };
        }
        let impulse = vanilla_push_impulse(EntityPushInput {
            caller_to_other_x: separation[0],
            caller_to_other_z: separation[1],
            recipient: PushRecipient::Caller,
            caller_physics_enabled: true,
            other_physics_enabled: true,
            passenger_of_same_vehicle: false,
            recipient_pushable: true,
            recipient_is_vehicle: false,
        })
        .expect("finite battle snapshot contact");
        pushes += 1;
        // Nothing at the body spacing, all of it SEPARATION_RAMP deeper: a step
        // here made a body pressed against its neighbours trade a push and a
        // pull every tick.
        let ease = ((MIN_SPACING - distance.sqrt()) / SEPARATION_RAMP).min(1.0);
        packed |= ease >= 1.0;
        drive[0] += impulse.x * ease;
        drive[1] += impulse.z * ease;
    }
    let max_step = max_step.max(0.05);
    let speed = mth::length2(drive[0], drive[1]);
    if speed > max_step {
        drive = drive.map(|v| v * max_step / speed);
    }
    let mut drive = keep_gap(&runtime.members, index, &near, MIN_SPACING, drive);
    let moving = mth::length2(drive[0], drive[1]);
    if settling && !packed && moving > 0.0 {
        let kept = (moving - CROWD_BRAKE).max(0.0) / moving;
        drive = drive.map(|v| v * kept);
    }
    // Physics drive is blocks/SECOND, whereas all battle tuning is blocks/tick.
    (
        runtime.constrain_velocity(
            m.pos,
            [
                drive[0] / mc_physics::TICK_SECONDS,
                0.0,
                drive[1] / mc_physics::TICK_SECONDS,
            ],
        ),
        pushes,
    )
}

/// Reserve half the free gap to each of `near` for its simultaneous
/// movement: `drive` (blocks per tick) never closes on a neighbour faster
/// than that, and not at all inside `spacing`. This bounds closing even when
/// the other row is steering towards us.
pub(super) fn keep_gap(
    members: &[Member],
    index: usize,
    near: &[(f64, usize); NEIGHBOURS],
    spacing: f64,
    mut drive: [f64; 2],
) -> [f64; 2] {
    let m = &members[index];
    for &(distance, other) in near {
        if other == usize::MAX || distance < 1e-8 {
            continue;
        }
        let contact = &members[other];
        let dist = distance.sqrt();
        let towards = [
            (contact.pos[0] - m.pos[0]) / dist,
            (contact.pos[2] - m.pos[2]) / dist,
        ];
        let closing = drive[0] * towards[0] + drive[1] * towards[1];
        let allowed = ((dist - spacing) * 0.5).max(0.0);
        if closing > allowed {
            drive[0] -= towards[0] * (closing - allowed);
            drive[1] -= towards[1] * (closing - allowed);
        }
    }
    drive
}

pub(crate) fn step(frame: &Frame<'_>, battle: &mut BattleState, _out: &mut Output) {
    battle
        .formation
        .neighbours
        .rebuild(&battle.formation.members);
    let formation = &battle.formation;
    let drives = &mut battle.columns.drive[..formation.members.len()];
    let sample = |(chunk_index, chunk): (usize, &mut [[f64; 3]])| {
        let mut work = (0_u64, 0_u64);
        for (offset, drive) in chunk.iter_mut().enumerate() {
            let (value, examined, pushes) =
                sample_movement(frame, formation, chunk_index * DRIVE_CHUNK + offset);
            *drive = value;
            work.0 += examined;
            work.1 += pushes;
        }
        work
    };
    let merge = |a: (u64, u64), b: (u64, u64)| (a.0 + b.0, a.1 + b.1);
    let work = if drives.len() >= DRIVE_CHUNK * 2 && rayon::current_num_threads() > 1 {
        drives
            .par_chunks_mut(DRIVE_CHUNK)
            .enumerate()
            .map(sample)
            .reduce(|| (0, 0), merge)
    } else {
        drives
            .chunks_mut(DRIVE_CHUNK)
            .enumerate()
            .map(sample)
            .fold((0, 0), merge)
    };
    battle.formation.work.separation_candidates += work.0;
    battle.formation.work.push_calls += work.1;
}

#[cfg(test)]
#[path = "steering_tests.rs"]
mod tests;
