//! Horses refuse solid foot fronts; mounted pursuit is not an infantry damage
//! buff. Once their own foot is in contact, unengaged riders swing wide round
//! the nearest enemy foot block and turn into its rear.
use super::combat::{Action, CombatRuntime};
use super::formation::Regiment;
use super::{BattleState, Frame};
use crate::boundary::EntityId;
use crate::entities::EntityKind;
use crate::{CoreError, tick::TickState};
use mc_physics::mth;

pub const CHARGE_DECAY_TICKS: u64 = 260;
const REFORM_TICKS: u64 = 400;
/// Lateral clearance outside the enemy's end file while swinging round it.
const FLANK_CLEARANCE: f64 = 14.0;

/// One mounted regiment's wide approach round one enemy foot regiment, in that
/// regiment's frame: `side` is the lateral unit vector of the chosen flank.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Flank {
    origin: [f64; 3],
    facing: [f64; 2],
    side: [f64; 2],
    half_width: f64,
    depth: f64,
}

impl Flank {
    /// Heading for a rider at `pos`: out past the flank, round behind the last
    /// rank, then into the rear half. Pure in the rider's own position.
    pub(super) fn heading(&self, pos: [f64; 3]) -> [f64; 2] {
        let delta = [pos[0] - self.origin[0], pos[2] - self.origin[2]];
        let lateral = delta[0] * self.side[0] + delta[1] * self.side[1];
        let ahead = delta[0] * self.facing[0] + delta[1] * self.facing[1];
        let (l, a) = if ahead <= -(self.depth + 4.0) {
            (self.half_width * 0.3, -self.depth * 0.5)
        } else if lateral < self.half_width + FLANK_CLEARANCE * 0.5 {
            (self.half_width + FLANK_CLEARANCE, ahead.max(6.0))
        } else {
            (self.half_width + FLANK_CLEARANCE * 0.5, -(self.depth + 8.0))
        };
        [
            self.origin[0] + self.side[0] * l + self.facing[0] * a - pos[0],
            self.origin[2] + self.side[1] * l + self.facing[1] * a - pos[2],
        ]
    }
}

/// What the unengaged riders of one mounted regiment ride for this tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Sweep {
    /// Ride down the fugitives of the nearest broken foot regiment.
    Chase([f64; 3]),
    /// Swing round an unbroken foot regiment into its rear.
    Flank(Flank),
}

impl Sweep {
    fn heading(&self, pos: [f64; 3]) -> [f64; 2] {
        match self {
            Self::Chase(at) => [at[0] - pos[0], at[2] - pos[2]],
            Self::Flank(flank) => flank.heading(pos),
        }
    }
}

/// The swing starts while the foot lines close, so riders reach the flank
/// about when the fronts meet rather than after the first regiment breaks.
fn foot_closing(regiment: &Regiment) -> bool {
    regiment.spec.frontage <= 1.0
        && !regiment.morale.broken
        && regiment.enemy.is_some()
        && regiment.gap <= 24.0
}

fn nearest_enemy_foot<'a>(
    battle: &'a BattleState,
    own: &Regiment,
    broken: bool,
) -> Option<&'a Regiment> {
    battle
        .formation
        .regiments
        .iter()
        .filter(|e| {
            e.spec.faction != own.spec.faction
                && e.spec.frontage <= 1.0
                && e.morale.broken == broken
                && e.live_count > 0
                && (broken || !battle.columns.ranged.get(e.start).copied().unwrap_or(true))
        })
        .min_by(|a, b| {
            let d = |e: &&Regiment| {
                mth::length2(
                    e.spec.origin[0] - own.spec.origin[0],
                    e.spec.origin[2] - own.spec.origin[2],
                )
            };
            d(a).total_cmp(&d(b)).then(a.spec.id.cmp(&b.spec.id))
        })
}

fn flank_round(own: &Regiment, e: &Regiment) -> Flank {
    let right = [e.spec.facing[1], -e.spec.facing[0]];
    let lateral = (own.spec.origin[0] - e.spec.origin[0]) * right[0]
        + (own.spec.origin[2] - e.spec.origin[2]) * right[1];
    let sign = if lateral >= 0.0 { 1.0 } else { -1.0 };
    Flank {
        origin: e.spec.origin,
        facing: e.spec.facing,
        side: [right[0] * sign, right[1] * sign],
        half_width: f64::from(e.spec.files.saturating_sub(1)) * e.interval * 0.5,
        depth: 2.0 * f64::from(e.ranks.saturating_sub(1)),
    }
}

/// Per-tick plan, bounded by regiments squared (at most 128²), never per rider.
/// A broken regiment's origin is its live centroid, so a chase follows the rout.
fn plan_sweeps(battle: &BattleState, plans: &mut Vec<Option<Sweep>>) {
    let regiments = &battle.formation.regiments;
    plans.clear();
    for own in regiments {
        let plan = if own.spec.frontage <= 1.0 || own.morale.broken {
            None
        } else if let Some(fugitives) = nearest_enemy_foot(battle, own, true) {
            Some(Sweep::Chase(fugitives.spec.origin))
        } else if regiments
            .iter()
            .any(|r| r.spec.faction == own.spec.faction && foot_closing(r))
        {
            nearest_enemy_foot(battle, own, false).map(|e| Sweep::Flank(flank_round(own, e)))
        } else {
            None
        };
        plans.push(plan);
    }
}

/// Unengaged steady riders of a regiment with a plan take its heading as their
/// drive; the mounted pass below scales it to horse speed.
fn sweep_drives(battle: &mut BattleState, combat: &mut CombatRuntime) {
    let mut plans = std::mem::take(&mut combat.sweeps);
    plan_sweeps(battle, &mut plans);
    for (r, plan) in battle.formation.regiments.iter().zip(&plans) {
        let Some(plan) = plan else { continue };
        for bi in r.start..r.start + r.len {
            let m = &battle.formation.members[bi];
            if !m.alive
                || m.wounded
                || m.routing
                || !battle.columns.active[bi]
                || battle.columns.targets[bi].is_some()
                || matches!(combat.fighters[bi].action, Action::Wounded)
            {
                continue;
            }
            let heading = plan.heading(battle.columns.positions[bi]);
            battle.columns.drive[bi] = [heading[0], 0.0, heading[1]];
        }
    }
    combat.sweeps = plans;
}

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
    sweep_drives(battle, combat);
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
                let mounted_speed = if wounded {
                    0.8
                } else if routing {
                    5.4
                } else {
                    4.6
                };
                velocity[0] *= mounted_speed / speed;
                velocity[2] *= mounted_speed / speed;
            }
            let position = frame.entities.pos(horse);
            let velocity = battle.formation.constrain_velocity(position, velocity);
            // A rider in the formation heads for its slot; another keeps its drive.
            let formation = &battle.formation;
            let heading = if bi < formation.members.len() {
                let slot = formation.heading_target(bi);
                [slot[0] - position[0], slot[2] - position[2]]
            } else {
                [velocity[0], velocity[2]]
            };
            let velocity =
                super::steering::around_obstacles(frame, formation, horse, velocity, heading);
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
        let target_bi = frame
            .entities
            .slot_of(target.id)
            .and_then(|slot| battle.columns.index(slot));
        let context = target_bi.and_then(|bi| super::combat::context(battle, bi));
        // Horsemen close with other horsemen; only a steady FOOT front refuses.
        let mounted = target_bi.is_some_and(|bi| {
            battle
                .formation
                .members
                .get(bi)
                .is_some_and(|m| battle.formation.regiments[m.regiment].spec.frontage > 1.0)
        });
        let frontal =
            context.is_some_and(|c| -(toward[0] * c.facing[0] + toward[1] * c.facing[1]) > 0.5);
        let contact = if context.is_some_and(|c| c.routing) {
            Contact::Pursuit
        } else if !frontal || mounted {
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
        let velocity = battle.formation.constrain_velocity(position, velocity);
        let velocity = super::steering::around_obstacles(
            frame,
            &battle.formation,
            horse,
            velocity,
            [velocity[0], velocity[2]],
        );
        battle.columns.drive[horse_bi] = velocity;
        battle.columns.drive[bi] = [0.0; 3];
    }
}

#[cfg(test)]
#[path = "cavalry_tests.rs"]
mod tests;
