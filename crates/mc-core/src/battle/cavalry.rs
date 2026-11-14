//! Horses refuse solid fronts; mounted pursuit is not an infantry damage buff.
use super::combat::{Action, CombatRuntime};
use super::{BattleState, Frame};
use crate::boundary::EntityId;
use crate::entities::EntityKind;
use crate::{CoreError, tick::TickState};
use mc_physics::mth;

pub const CHARGE_DECAY_TICKS: u64 = 260;
const REFORM_TICKS: u64 = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Contact {
    SteadyFront,
    Wavering,
    Flank,
    Pursuit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    Refuse,
    Impact,
}

pub fn charge_decision(contact: Contact) -> Decision {
    if contact == Contact::SteadyFront {
        Decision::Refuse
    } else {
        Decision::Impact
    }
}

pub fn impact_bonus(elapsed: u64) -> f32 {
    2.0 * (CHARGE_DECAY_TICKS.saturating_sub(elapsed) as f32 / CHARGE_DECAY_TICKS as f32)
}

#[derive(Debug, Default)]
pub(super) struct Charge {
    impact: Option<u64>,
    refused: bool,
    reform_until: u64,
}

impl Charge {
    pub(super) fn bonus(&self, tick: u64) -> f32 {
        self.impact
            .map_or(0.0, |start| impact_bonus(tick.saturating_sub(start)))
    }
}

/// Staging uses the same UUID-owned mount relation as player riders. Only the
/// horse owns durable custody; reverse EntityIds and passengers rebuild at tick.
pub fn mount(state: &mut TickState, rider: EntityId, horse: EntityId) -> Result<(), CoreError> {
    let invalid = || {
        CoreError::Storage(format!(
            "battle mount {horse:?} rider {rider:?}: requires distinct live unmounted mobs, a nonzero rider faction and an equine vehicle"
        ))
    };
    let rider_slot = state.entities.slot_of(rider).ok_or_else(invalid)?;
    let horse_slot = state.entities.slot_of(horse).ok_or_else(invalid)?;
    let rider_body = state.entities.body(rider_slot);
    let horse_body = state.entities.body(horse_slot);
    let equine = crate::mounts::type_id(horse_body)
        .and_then(|id| state.registry.blueprint(id))
        .is_some_and(|bp| bp.capabilities.taming.equine());
    let rider_row = state.entities.row(rider_slot);
    let horse_row = state.entities.row(horse_slot);
    if rider == horse
        || rider_body.faction == 0
        || !rider_body.alive
        || !horse_body.alive
        || !matches!(rider_body.kind, EntityKind::Mob { .. })
        || !equine
        || rider_row.health <= 0.0
        || horse_row.health <= 0.0
        || rider_row.entity_life.mount.vehicle.is_some()
        || rider_row.entity_life.mount.rider.is_some()
        || horse_row.entity_life.mount.vehicle.is_some()
        || horse_row.entity_life.mount.rider.is_some()
    {
        return Err(invalid());
    }
    let uuid = rider_row.uuid;
    let faction = rider_body.faction;
    state.entities.set_faction(horse_slot, faction);
    state.entities.set_rider(horse_slot, Some(uuid));
    let horse_row = state.entities.row_mut(horse_slot);
    horse_row.persistent = true;
    horse_row.entity_life.mounted = true;
    horse_row.entity_life.tamed = true;
    state.entities.row_mut(rider_slot).persistent = true;
    crate::mounts::sync_rider(state, horse_slot);
    Ok(())
}

pub(super) fn step(frame: &Frame<'_>, battle: &mut BattleState, combat: &mut CombatRuntime) {
    for horse in frame.entities.rider_slots() {
        let row = frame.entities.row(horse);
        let Some(rider) = row
            .entity_life
            .mount
            .rider
            .and_then(|uuid| frame.entities.slot_of_uuid(uuid))
        else {
            continue;
        };
        let Some(bi) = battle.columns.index(rider) else {
            continue;
        };
        let Some(horse_bi) = battle.columns.index(horse) else {
            continue;
        };
        if !battle.columns.active[bi] || battle.columns.carried[bi] {
            continue;
        }
        let wounded = matches!(combat.fighters[bi].action, Action::Wounded);
        let routing = super::combat::context(battle, bi).is_some_and(|c| c.routing);
        if wounded || routing || battle.columns.targets[bi].is_none() {
            // Carry the formation's approach/withdrawal/rout direction on the
            // horse, never override a routing rider with an enemy pursuit.
            let mut velocity = battle.columns.drive[bi];
            let speed = mth::length2(velocity[0], velocity[2]);
            if speed > 0.001 {
                let mut mounted_speed: f64 = if wounded {
                    0.8
                } else if routing {
                    5.4
                } else {
                    4.6
                };
                let formation = &battle.formation;
                // A rider dressing his place keeps its pace: the leader's speed plus a
                // catch-up that grows with the distance, never the full trot for a
                // hand's breadth (which overshot the place every tick and left the
                // block ragged). A sweeping rider heads for a plan point instead.
                if let Some(member) = formation.members.get(bi)
                    && !wounded
                    && !routing
                    && !combat
                        .sweeps
                        .get(member.regiment)
                        .is_some_and(Option::is_some)
                {
                    let slot = formation.heading_target(bi);
                    let away = mth::length2(slot[0] - position[0], slot[2] - position[2]);
                    let leader =
                        formation.regiments[member.regiment].speed.abs() / mc_physics::TICK_SECONDS;
                    mounted_speed = mounted_speed.min((leader + away * SLOT_CATCH_UP).max(speed));
                }
                velocity[0] *= mounted_speed / speed;
                velocity[2] *= mounted_speed / speed;
            }
            let velocity = battle
                .formation
                .constrain_velocity(frame.entities.pos(horse), velocity);
            battle.columns.drive[horse_bi] = velocity;
            battle.columns.drive[bi] = [0.0; 3];
            continue;
        }
        let target = battle.columns.targets[bi].as_ref().expect("targeted rider");
        let position = frame.entities.pos(horse);
        let delta = [target.pos[0] - position[0], target.pos[2] - position[2]];
        let distance = mth::length2(delta[0], delta[1]);
        if distance < 0.001 {
            continue;
        }
        let toward = [delta[0] / distance, delta[1] / distance];
        let context = frame
            .entities
            .slot_of(target.id)
            .and_then(|slot| battle.columns.index(slot))
            .and_then(|bi| super::combat::context(battle, bi));
        let frontal =
            context.is_some_and(|c| -(toward[0] * c.facing[0] + toward[1] * c.facing[1]) > 0.5);
        let contact = if context.is_some_and(|c| c.routing) {
            Contact::Pursuit
        } else if !frontal {
            Contact::Flank
        } else if context.is_some_and(|c| c.wavering || c.morale < 50.0) {
            Contact::Wavering
        } else {
            Contact::SteadyFront
        };
        let charge = &mut combat.fighters[bi].charge;
        if distance > 46.0 && frame.tick >= charge.reform_until {
            charge.impact = None;
            charge.refused = false;
        }
        if distance <= 6.0 && charge.impact.is_none() && !charge.refused {
            combat.work.charge_checks += 1;
            if charge_decision(contact) == Decision::Refuse {
                charge.refused = true;
                charge.reform_until = frame.tick + REFORM_TICKS;
            } else {
                charge.impact = Some(frame.tick);
                charge.reform_until = frame.tick + REFORM_TICKS;
            }
        }
        let velocity = if charge.refused {
            // Run out toward the nearer flank; never steer through the steady line.
            let facing = context.map_or([-toward[0], -toward[1]], |c| c.facing);
            let right = [-facing[1], facing[0]];
            let side = if -delta[0] * right[0] - delta[1] * right[1] >= 0.0 {
                1.0
            } else {
                -1.0
            };
            [
                (right[0] * side - toward[0]) * 3.8,
                0.0,
                (right[1] * side - toward[1]) * 3.8,
            ]
        } else {
            let speed = if distance <= 46.0 { 6.4 } else { 4.6 };
            [toward[0] * speed, 0.0, toward[1] * speed]
        };
        let push = horse_push(&battle.formation, bi);
        let velocity = [velocity[0] + push[0], velocity[1], velocity[2] + push[1]];
        let velocity = battle.formation.constrain_velocity(position, velocity);
        battle.columns.drive[horse_bi] = velocity;
        battle.columns.drive[bi] = [0.0; 3];
    }
}

#[cfg(test)]
#[path = "cavalry_tests.rs"]
mod tests;
