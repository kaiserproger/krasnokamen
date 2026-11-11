//! Sparse per-team regiment distance samples plus bounded local steering.
//! The filmed battlefield is flat: no per-soldier path search is needed.
//! Nearest front points are shared for ten ticks (AoE4 virtual-leader pattern).
//! Separation: PSCrowd's five nearest neighbours, capped cell inspection;
//! impulses use the existing mc-physics contact kernel, never mutate neighbours.

use super::formation::{FormationRuntime, Member, Regiment};
use super::{BattleState, Frame, Output};
use crate::boundary::EntityId;
use mc_physics::entity_collision_26_1_2::{EntityPushInput, PushRecipient, vanilla_push_impulse};
use rustc_hash::FxHashMap;

pub(crate) const NEIGHBOURS: usize = 5;
const CELL_CAP: usize = 8;
pub(crate) const MIN_SPACING: f64 = 0.65;
const SENSE: f64 = 1.5;

#[derive(Debug, Clone, Copy)]
struct Cell {
    indices: [usize; CELL_CAP],
    len: usize,
}
impl Default for Cell {
    fn default() -> Self {
        Self {
            indices: [0; CELL_CAP],
            len: 0,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct SeparationGrid {
    cells: FxHashMap<(i32, i32), Cell>,
    pub overflow: u64,
}

fn cell(p: [f64; 3]) -> (i32, i32) {
    (p[0].floor() as i32, p[2].floor() as i32)
}

impl SeparationGrid {
    pub fn rebuild(&mut self, members: &[Member]) {
        // Fixed inline cells: clearing/rebuilding retains the hash allocation,
        // and no per-cell heap allocation occurs even at 20,000 bodies.
        self.cells.clear();
        self.overflow = 0;
        for (i, m) in members.iter().enumerate().filter(|(_, m)| m.alive) {
            let c = self.cells.entry(cell(m.pos)).or_default();
            if c.len < CELL_CAP {
                c.indices[c.len] = i;
                c.len += 1;
            } else {
                self.overflow += 1;
            }
        }
    }

    fn nearest(&self, index: usize, members: &[Member]) -> ([(f64, usize); NEIGHBOURS], u64) {
        let actor = &members[index];
        let (x, z) = cell(actor.pos);
        let mut best = [(f64::INFINITY, usize::MAX); NEIGHBOURS];
        let mut examined = 0;
        // At most 25 * 8 candidates even if someone deliberately stacks units.
        for dz in -2..=2 {
            for dx in -2..=2 {
                let Some(c) = self.cells.get(&(x + dx, z + dz)) else {
                    continue;
                };
                for &other in &c.indices[..c.len] {
                    if other == index {
                        continue;
                    }
                    examined += 1;
                    let m = &members[other];
                    if (actor.pos[1] - m.pos[1]).abs() >= 2.0 {
                        continue;
                    }
                    let d2 = (actor.pos[0] - m.pos[0]).powi(2) + (actor.pos[2] - m.pos[2]).powi(2);
                    if d2 > SENSE * SENSE {
                        continue;
                    }
                    let candidate = (d2, other);
                    let Some(at) = best.iter().position(|&(distance, old)| {
                        d2 < distance
                            || (d2 == distance && (old == usize::MAX || m.id < members[old].id))
                    }) else {
                        continue;
                    };
                    for k in (at + 1..NEIGHBOURS).rev() {
                        best[k] = best[k - 1];
                    }
                    best[at] = candidate;
                }
            }
        }
        (best, examined)
    }
}

pub(crate) fn unit(delta: [f64; 2], fallback: [f64; 2]) -> [f64; 2] {
    let length = delta[0].hypot(delta[1]);
    if length < 1e-9 {
        fallback
    } else {
        delta.map(|v| v / length)
    }
}

/// Distance field sampled only at regiment leaders, with one seed segment per
/// enemy front. O(R²/10), bounded R<=128; at 20x1000 men it is 40 samples/tick,
/// not 20,000 long-range target scans. Clamping to a front segment (not its
/// centroid) preserves parallel frontage instead of converging every file.
pub(crate) fn nearest_front(regiments: &[Regiment], actor: usize) -> (Option<[f64; 3]>, bool, u64) {
    let own = &regiments[actor];
    let mut nearest = None;
    let mut routing = false;
    let mut best = f64::INFINITY;
    let mut best_id = u32::MAX;
    let mut examined = 0;
    for enemy in regiments
        .iter()
        .filter(|r| r.spec.faction != own.spec.faction && r.removed < r.len)
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
            routing = enemy.morale.broken;
        }
    }
    (nearest, routing, examined)
}

pub(crate) fn movement(runtime: &mut FormationRuntime, index: usize) -> [f64; 3] {
    let m = &runtime.members[index];
    if !m.alive {
        return [0.0; 3];
    }
    let (mut target, max_step) = runtime.destination(index);
    if let Some((min, max)) = runtime.bounds {
        target[0] = target[0].clamp(min[0], max[0]);
        target[2] = target[2].clamp(min[1], max[1]);
    }
    let delta = [target[0] - m.pos[0], target[2] - m.pos[2]];
    let length = delta[0].hypot(delta[1]);
    let mut drive = delta.map(|v| v * (max_step / length.max(max_step)).min(1.0));
    let (near, examined) = runtime.neighbours.nearest(index, &runtime.members);
    runtime.work.separation_candidates += examined;
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
        runtime.work.push_calls += 1;
        drive[0] += impulse.x;
        drive[1] += impulse.z;
    }
    let max_step = max_step.max(0.05);
    let speed = drive[0].hypot(drive[1]);
    if speed > max_step {
        drive = drive.map(|v| v * max_step / speed);
    }
    // Reserve half the free gap for each participant's simultaneous movement.
    // This bounds closing even when the other row is steering towards us.
    for &(distance, other) in &near {
        if other == usize::MAX || distance < 1e-8 {
            continue;
        }
        let contact = &runtime.members[other];
        let dist = distance.sqrt();
        let towards = [
            (contact.pos[0] - m.pos[0]) / dist,
            (contact.pos[2] - m.pos[2]) / dist,
        ];
        let closing = drive[0] * towards[0] + drive[1] * towards[1];
        let allowed = ((dist - MIN_SPACING) * 0.5).max(0.0);
        if closing > allowed {
            drive[0] -= towards[0] * (closing - allowed);
            drive[1] -= towards[1] * (closing - allowed);
        }
    }
    // Physics drive is blocks/SECOND, whereas all battle tuning is blocks/tick.
    runtime.constrain_velocity(
        m.pos,
        [
            drive[0] / mc_physics::TICK_SECONDS,
            0.0,
            drive[1] / mc_physics::TICK_SECONDS,
        ],
    )
}

pub(crate) fn step(_frame: &Frame<'_>, battle: &mut BattleState, _out: &mut Output) {
    battle
        .formation
        .neighbours
        .rebuild(&battle.formation.members);
    for i in 0..battle.formation.members.len() {
        let id: EntityId = battle.formation.members[i].id;
        let drive = movement(&mut battle.formation, i);
        battle.steering.insert(id, drive);
    }
}

#[cfg(test)]
#[path = "steering_tests.rs"]
mod tests;
