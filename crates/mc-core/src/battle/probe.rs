//! Facing counters of the phase log (`MC_CORE_PHASE_LOG`), for the D36 takes
//! (receipts/BattleTurnCapCause/D36-FACING-DESIGN.md section 7.3). Computed
//! only while the phase log is on and printed on its `battle detail us` line;
//! the battle never reads them. Unmounted rows only, as in the simulator.
//!
//! Each counter has one instant. A standing->driven transition is measured at
//! the battle step of the driven tick, from the yaw that tick's step starts
//! from and the stored head. A windup is measured at the end of the tick it
//! starts in, after the entity phase wrote that tick's rotations: head and
//! drawn body as the client receives them with the swing, against the bearing
//! to the windup's own target from the positions of that instant.

use super::BattleState;
use super::Frame;
use super::combat::Action;
use crate::entities::EntitySlot;
use crate::entity_motion::Facing;
use mc_domain::EntityId;
use mc_physics::mth;
use rustc_hash::FxHashMap;
use std::fmt;

#[derive(Debug, Default)]
pub(crate) struct Probe {
    /// Every unmounted row with a target at the last observed tick: that
    /// target and whether the row was driven.
    rows: FxHashMap<EntityId, (EntityId, bool)>,
    next: FxHashMap<EntityId, (EntityId, bool)>,
    /// The battle tick whose windups were counted last.
    windups_of: Option<u64>,
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
    /// them those whose starting yaw or head is off the bearing.
    transitions: u64,
    transitions_yaw: [u64; 2],
    transitions_head: u64,
    /// Windups started this tick (the `Swing` event), and of them those whose
    /// head or drawn body is off the bearing to the windup's own target.
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
    /// After the projection of a running battle tick: standing rows, target
    /// switches and transitions.
    pub(super) fn observe(&mut self, frame: &Frame<'_>, battle: &BattleState) {
        let columns = &battle.columns;
        let rotations = frame.entities.rotations();
        let mut counts = Counts::default();
        self.next.clear();
        for (bi, slot) in columns.slots.iter().enumerate() {
            let (Some(slot), Some(target)) = (*slot, columns.targets[bi]) else {
                continue;
            };
            if mounted(frame.entities, slot) {
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
                        let yaw = off(bearing, start_yaw(battle, slot, stored[0]));
                        counts.transitions_yaw[0] += u64::from(yaw >= 20.0);
                        counts.transitions_yaw[1] += u64::from(yaw >= 45.0);
                        counts.transitions_head += u64::from(off(bearing, stored[2]) >= 20.0);
                    }
                }
            }
            self.next.insert(id, (target.id, driven));
        }
        std::mem::swap(&mut self.rows, &mut self.next);
        self.counts = counts;
    }

    /// Battle paused or stopped: the first running tick after it compares
    /// with nothing (no switch or transition across the pause).
    pub(super) fn reset(&mut self) {
        self.rows.clear();
    }

    /// After the entity phase of a tick wrote its rotations: the windups the
    /// battle started this tick, once.
    pub(crate) fn windups(&mut self, state: &crate::tick::TickState) {
        let battle = &state.hostile.battle;
        let Some(tick) = battle.elapsed_ticks.checked_sub(1) else {
            return;
        };
        if !battle.running || self.windups_of == Some(tick) {
            return;
        }
        self.windups_of = Some(tick);
        let entities = &state.entities;
        let (rotations, positions) = (entities.rotations(), entities.positions());
        for (bi, fighter) in battle.combat.fighters.iter().enumerate() {
            let Action::Windup { target, start, .. } = fighter.action else {
                continue;
            };
            let (Some(slot), Some(victim)) = (
                entities.slot_of(battle.combat.ids[bi]),
                entities.slot_of(target),
            ) else {
                continue;
            };
            if start != tick || mounted(entities, slot) {
                continue;
            }
            let Some(bearing) = bearing(positions[slot.0 as usize], positions[victim.0 as usize])
            else {
                continue;
            };
            let after = rotations[slot.0 as usize];
            let (head, body) = (
                off(bearing, after[2]),
                off(bearing, drawn_body(battle, slot, after[0])),
            );
            let counts = &mut self.counts;
            counts.windups += 1;
            counts.windups_head[0] += u64::from(head >= 20.0);
            counts.windups_head[1] += u64::from(head >= 45.0);
            counts.windups_body[0] += u64::from(body >= 20.0);
            counts.windups_body[1] += u64::from(body >= 45.0);
        }
    }
}

fn mounted(entities: &crate::entities::EntityStore, slot: EntitySlot) -> bool {
    entities.row(slot).entity_life.mount.vehicle.is_some()
}

/// The yaw a driven step starts from: the drawn body the projection hands it
/// (`From`, under L the stored yaw lags it), else the stored yaw.
fn start_yaw(battle: &BattleState, slot: EntitySlot, stored_yaw: f32) -> f32 {
    match battle.facing.of(slot) {
        Some(Facing::From { yaw }) => yaw,
        _ => stored_yaw,
    }
}

/// The drawn body after a tick's step: the body the projection published for
/// a standing row (under L the stored yaw lags it), else the yaw written.
fn drawn_body(battle: &BattleState, slot: EntitySlot, written_yaw: f32) -> f32 {
    match battle.facing.of(slot) {
        Some(Facing::Set { body, .. }) => body,
        _ => written_yaw,
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
