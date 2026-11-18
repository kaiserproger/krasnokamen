//! Facing counters of the phase log (`MC_CORE_PHASE_LOG`), for the D36 takes
//! (receipts/BattleTurnCapCause/D36-FACING-DESIGN.md section 7.3). Computed
//! only while the phase log is on and printed on its `battle detail us` line;
//! the battle never reads them. Unmounted rows only, as in the simulator.

use super::BattleState;
use super::Frame;
use super::combat::Action;
use crate::boundary::EntityId;
use crate::entities::EntitySlot;
use crate::entity_motion::Facing;
use mc_physics::mth;
use rustc_hash::FxHashMap;
use std::fmt;

#[derive(Debug, Default)]
pub(crate) struct Probe {
    /// Every unmounted row with a target at the last observed tick: that
    /// target and whether the row was driven.
    rows: FxHashMap<EntityId, (EntityId, bool)>,
    next: FxHashMap<EntityId, (EntityId, bool)>,
    pub(crate) counts: Counts,
}

/// One battle tick. Error thresholds are 20 and 45 degrees.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Counts {
    /// Rows with a target and zero drive.
    standing: u64,
    /// Rows whose target id differs from the one they had the tick before.
    switches: u64,
    /// Rows standing with a target the tick before and driven now, and of
    /// them those whose starting body yaw or head is off the bearing.
    transitions: u64,
    transitions_yaw: [u64; 2],
    transitions_head: u64,
    /// Windups started this tick (the `Swing` event), and of them those whose
    /// head or body is off the bearing to the windup's own target.
    windups: u64,
    windups_head: [u64; 2],
    windups_body: [u64; 2],
}

impl fmt::Display for Counts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "standing={} switches={} transitions={} transitions_yaw20={} transitions_yaw45={} transitions_head20={} windups={} windups_head20={} windups_head45={} windups_body20={} windups_body45={}",
            self.standing,
            self.switches,
            self.transitions,
            self.transitions_yaw[0],
            self.transitions_yaw[1],
            self.transitions_head,
            self.windups,
            self.windups_head[0],
            self.windups_head[1],
            self.windups_body[0],
            self.windups_body[1],
        )
    }
}

impl Probe {
    /// After the projection of a running battle tick.
    pub(super) fn observe(&mut self, frame: &Frame<'_>, battle: &BattleState) {
        let columns = &battle.columns;
        let rotations = frame.entities.rotations();
        let positions = frame.entities.positions();
        let mounted =
            |slot: EntitySlot| frame.entities.row(slot).entity_life.mount.vehicle.is_some();
        let mut counts = Counts::default();
        self.next.clear();
        for (bi, slot) in columns.slots.iter().enumerate() {
            let (Some(slot), Some(target)) = (*slot, columns.targets[bi]) else {
                continue;
            };
            if mounted(slot) {
                continue;
            }
            let id = columns.ids[bi];
            let driven = columns.drive[bi] != [0.0; 3];
            counts.standing += u64::from(!driven);
            if let Some(&(was, was_driven)) = self.rows.get(&id) {
                counts.switches += u64::from(was != target.id);
                if driven && !was_driven {
                    counts.transitions += 1;
                    let stored = rotations[slot.0 as usize];
                    if let Some(bearing) = bearing(columns.positions[bi], target.pos) {
                        let yaw = off(bearing, body_yaw(battle, slot, stored));
                        counts.transitions_yaw[0] += u64::from(yaw >= 20.0);
                        counts.transitions_yaw[1] += u64::from(yaw >= 45.0);
                        counts.transitions_head += u64::from(off(bearing, stored[2]) >= 20.0);
                    }
                }
            }
            self.next.insert(id, (target.id, driven));
        }
        std::mem::swap(&mut self.rows, &mut self.next);
        for (bi, fighter) in battle.combat.fighters.iter().enumerate() {
            let Action::Windup { target, start, .. } = fighter.action else {
                continue;
            };
            let Some(slot) = columns.slots.get(bi).copied().flatten() else {
                continue;
            };
            if start != frame.tick
                || battle.combat.ids.get(bi) != columns.ids.get(bi)
                || mounted(slot)
            {
                continue;
            }
            let Some(victim) = frame.entities.slot_of(target) else {
                continue;
            };
            let Some(bearing) = bearing(positions[slot.0 as usize], positions[victim.0 as usize])
            else {
                continue;
            };
            let stored = rotations[slot.0 as usize];
            let (head, body) = (
                off(bearing, stored[2]),
                off(bearing, body_yaw(battle, slot, stored)),
            );
            counts.windups += 1;
            counts.windups_head[0] += u64::from(head >= 20.0);
            counts.windups_head[1] += u64::from(head >= 45.0);
            counts.windups_body[0] += u64::from(body >= 20.0);
            counts.windups_body[1] += u64::from(body >= 45.0);
        }
        self.counts = counts;
    }
}

/// The yaw a row's body starts from: the body published this tick (under L
/// the stored yaw lags it; a row leaving the projection starts from it), else
/// the stored yaw.
fn body_yaw(battle: &BattleState, slot: EntitySlot, stored: [f32; 3]) -> f32 {
    match battle.facing.of(slot) {
        Some(Facing::Set { body, .. }) => body,
        Some(Facing::From { yaw }) => yaw,
        None => stored[0],
    }
}

fn bearing(from: [f64; 3], to: [f64; 3]) -> Option<f32> {
    let (dx, dz) = (to[0] - from[0], to[2] - from[2]);
    (mth::length2(dx, dz) >= 1.0e-3).then(|| -(mth::atan2(dx, dz) as f32) * mth::RAD_TO_DEG)
}

fn off(bearing: f32, yaw: f32) -> f32 {
    ((bearing - yaw + 180.0).rem_euclid(360.0) - 180.0).abs()
}

#[cfg(test)]
#[path = "probe_tests.rs"]
mod tests;
