//! Horses refuse solid foot fronts; mounted pursuit is not an infantry damage
//! buff. Once their own foot is in contact, unengaged riders swing wide round
//! the nearest enemy foot block and turn into its rear.
use super::combat::{Action, CombatRuntime};
use super::exposure::{Plan, Sample};
use super::formation::Regiment;
use super::steering::NEIGHBOURS;
use super::{BattleState, Frame};
use crate::boundary::EntityId;
use crate::entities::EntityKind;
use crate::{CoreError, tick::TickState};
use mc_physics::mth;

pub const CHARGE_DECAY_TICKS: u64 = 260;
const REFORM_TICKS: u64 = 400;
/// Lateral clearance outside the enemy's end file while swinging round it.
const FLANK_CLEARANCE: f64 = 14.0;
/// Blocks per second of catch-up speed a rider gains for every block he stands from his place.
const SLOT_CATCH_UP: f64 = 3.0;
/// Horses closer than this push apart (a horse is 1.4 blocks wide), at most
/// `HORSE_PUSH` blocks per second.
const HORSE_SPACING: f64 = 1.5;
const HORSE_PUSH: f64 = 3.0;
/// How far a rider looks for horses: the spacing plus a gallop step
/// (6.4 blocks per second) of each of two horses closing head on, so none is
/// first seen already inside the spacing. The candidate window ends at 2.
const HORSE_SENSE: f64 = 2.0;
/// Golden angle: consecutive riders' attack places round one target never line up.
const ATTACK_ANGLE: f64 = 2.399_963_229_728_653;
/// Fugitives whose live centre is this close to the bounds have reached the
/// edge of the field: the chase ends and the riders rejoin the fight. Chasers
/// stood on the enemy's rear edge for 100 s in the G1 rehearsal.
const CHASE_EDGE_BLOCKS: f64 = 8.0;

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
            super::enemies(e.spec.faction, own.spec.faction)
                && e.spec.frontage <= 1.0
                && e.morale.broken == broken
                && e.live_count > 0
                && (broken || !battle.columns.ranged.get(e.start).copied().unwrap_or(true))
                && !(broken
                    && super::formation::near_bounds(
                        battle.formation.bounds,
                        live_centre(e),
                        CHASE_EDGE_BLOCKS,
                    ))
        })
        .min_by(|a, b| {
            let d = |e: &&Regiment| {
                let (p, q) = (live_centre(e), live_centre(own));
                mth::length2(p[0] - q[0], p[2] - q[2])
            };
            d(a).total_cmp(&d(b)).then(a.spec.id.cmp(&b.spec.id))
        })
}

/// Where a regiment's living soldiers stand. On relief the virtual leader
/// (`spec.origin`) marches on while files lag behind slopes and trees, so a
/// swing laid round the leader's line could end on empty ground: in the G1
/// rehearsals a red cavalry regiment waited there for 100 s with no foe within
/// reach.
fn live_centre(r: &Regiment) -> [f64; 3] {
    if r.live_count > 0 {
        r.live_sum.map(|x| x / r.live_count as f64)
    } else {
        r.spec.origin
    }
}

fn flank_round(own: &Regiment, e: &Regiment) -> Flank {
    let (from, origin) = (live_centre(own), live_centre(e));
    let right = [e.spec.facing[1], -e.spec.facing[0]];
    let lateral = (from[0] - origin[0]) * right[0] + (from[2] - origin[2]) * right[1];
    let sign = if lateral >= 0.0 { 1.0 } else { -1.0 };
    Flank {
        origin,
        facing: e.spec.facing,
        side: [right[0] * sign, right[1] * sign],
        half_width: f64::from(e.spec.files.saturating_sub(1)) * e.interval * 0.5,
        depth: 2.0 * f64::from(e.ranks.saturating_sub(1)),
    }
}

/// Per-tick plan, bounded by regiments squared (at most 128²), never per rider.
/// A broken regiment's origin is its live centroid, so a chase follows the rout
/// until the fugitives reach the edge of the field. A broken mounted regiment
/// keeps its plan: only its rallied riders, no longer routing, ride it.
fn plan_sweeps(battle: &BattleState, plans: &mut Vec<Option<Sweep>>) {
    let regiments = &battle.formation.regiments;
    plans.clear();
    for own in regiments {
        let plan = if own.spec.frontage <= 1.0 {
            None
        } else if let Some(fugitives) = nearest_enemy_foot(battle, own, true) {
            Some(Sweep::Chase(fugitives.spec.origin))
        } else if regiments.iter().any(|r| {
            super::relation(r.spec.faction, own.spec.faction) == super::Relation::Same
                && foot_closing(r)
        }) {
            nearest_enemy_foot(battle, own, false).map(|e| Sweep::Flank(flank_round(own, e)))
        } else {
            None
        };
        plans.push(plan);
    }
}

/// The horses nearest a rider within [`HORSE_SENSE`]. In a melee the five
/// nearest bodies were often all foot, so a horse overlapping another one was
/// never pushed off it.
fn horses_near(
    formation: &super::formation::FormationRuntime,
    bi: usize,
) -> [(f64, usize); NEIGHBOURS] {
    if bi >= formation.members.len() {
        return [(f64::INFINITY, usize::MAX); NEIGHBOURS];
    }
    let regiments = &formation.regiments;
    formation
        .neighbours
        .nearest_where(bi, &formation.members, HORSE_SENSE, |m| {
            regiments[m.regiment].spec.frontage > 1.0
        })
        .0
}

/// Push of the nearest horses on a rider, blocks per second. Riders closing on a
/// foe or riding a plan steer without the foot separation, and rode onto one
/// block: six horses stood on one place at the foot line in the battle census.
fn horse_push(
    formation: &super::formation::FormationRuntime,
    bi: usize,
    near: &[(f64, usize); NEIGHBOURS],
) -> [f64; 2] {
    let Some(me) = formation.members.get(bi) else {
        return [0.0; 2];
    };
    let mut push = [0.0; 2];
    for &(distance, other) in near {
        if other == usize::MAX || distance >= HORSE_SPACING * HORSE_SPACING {
            continue;
        }
        let contact = &formation.members[other];
        let away = [me.pos[0] - contact.pos[0], me.pos[2] - contact.pos[2]];
        let length = distance.sqrt();
        let way = if length < 1e-8 {
            // Coincident horses part symmetrically by id.
            if me.id < contact.id {
                [-1.0, 0.0]
            } else {
                [1.0, 0.0]
            }
        } else {
            away.map(|v| v / length)
        };
        let strength = (1.0 - length / HORSE_SPACING) * HORSE_PUSH;
        push[0] += way[0] * strength;
        push[1] += way[1] * strength;
    }
    push
}

/// A horse's final velocity never closes on a horse within
/// [`HORSE_SPACING`], and on one farther off by at most half the free gap a
/// tick. The terrain turn replaces the drive and its push with the clear way
/// at the drive's speed, and the push never held off the gallop at a place
/// round the target: in the G2 take 7 horses overlapped at the foot of a
/// cliff for 35 s and a quarter of all live horses stood within half a block
/// of another. The common guard satisfies all neighbour limits together.
fn horse_gap(
    formation: &super::formation::FormationRuntime,
    bi: usize,
    near: &[(f64, usize); NEIGHBOURS],
    position: [f64; 3],
    velocity: [f64; 3],
) -> [f64; 3] {
    if bi >= formation.members.len() {
        return velocity;
    }
    super::steering::keep_gap(formation, bi, near, HORSE_SPACING, position, velocity)
}

/// Unengaged steady riders of a regiment with a plan take its heading as their
/// drive; the mounted pass below scales it to horse speed. The regiment rides
/// the plan as a block: each rider follows the plan from its own slot, i.e. as
/// if it were the leader shifted by its file/rank offset. One shared point for
/// every rider piled a whole regiment onto one block wherever it arrived with
/// no foe in reach, and the pile then rode as one body.
/// Only a seated rider takes it: the heading is a displacement in blocks, and
/// only the mounted pass turns it into a speed. A member whose horse is gone
/// keeps the foot drive steering gave him; with the raw heading as his drive
/// (blocks per second) he skated a twentieth of the way to the plan point
/// every tick, up to 107 blocks per second in the field-1k battle takes.
fn sweep_drives(frame: &Frame<'_>, battle: &mut BattleState, combat: &mut CombatRuntime) {
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
            let pos = battle.columns.positions[bi];
            let seated = m
                .slot
                .is_some_and(|slot| frame.entities.row(slot).entity_life.mount.vehicle.is_some());
            if !seated {
                let kind = match plan {
                    Sweep::Chase(_) => Plan::Chase,
                    Sweep::Flank(_) => Plan::Flank,
                };
                battle.exposure.exposed(m.id, bi, pos, kind);
                continue;
            }
            let slot = super::formation::slot_position(
                &r.spec,
                usize::from(m.rank) * usize::from(r.spec.files) + m.file,
            );
            let leader = [
                pos[0] - (slot[0] - r.spec.origin[0]),
                pos[1],
                pos[2] - (slot[2] - r.spec.origin[2]),
            ];
            let heading = plan.heading(leader);
            battle.columns.drive[bi] = [heading[0], 0.0, heading[1]];
        }
    }
    let sample = Sample {
        entities: frame.entities,
        formation: &battle.formation,
        columns: &battle.columns,
    };
    battle.exposure.close(&sample, &combat.fighters, &plans);
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

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
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
        || !super::battle_row(rider_body.faction)
        || !rider_body.alive
        || !horse_body.alive
        || !matches!(rider_body.kind, EntityKind::Mob { .. })
        || !equine
        || rider_row.health <= 0.0
        || horse_row.health <= 0.0
        || rider_row.entity_life.mount.vehicle.is_some()
        || crate::mounts::occupied(&state.entities, &rider_row.entity_life.mount)
        || horse_row.entity_life.mount.vehicle.is_some()
        || crate::mounts::occupied(&state.entities, &horse_row.entity_life.mount)
    {
        return Err(invalid());
    }
    let uuid = rider_row.uuid;
    let faction = rider_body.faction;
    crate::scoreboard::require_no_team(&state.scoreboard, &horse_row.uuid.to_string())
        .map_err(CoreError::Storage)?;
    state.entities.set_faction(horse_slot, faction);
    state.entities.set_riders(rider_slot, Vec::new());
    state.entities.set_riders(horse_slot, vec![uuid]);
    let horse_row = state.entities.row_mut(horse_slot);
    horse_row.persistent = true;
    horse_row.entity_life.tamed = true;
    state.entities.row_mut(rider_slot).persistent = true;
    crate::mounts::sync_rider(state, horse_slot);
    Ok(())
}

pub(super) fn step(frame: &Frame<'_>, battle: &mut BattleState, combat: &mut CombatRuntime) {
    sweep_drives(frame, battle, combat);
    for horse in frame.entities.rider_slots() {
        let row = frame.entities.row(horse);
        let Some(rider) = crate::mounts::live_riders(frame.entities, &row.entity_life.mount).next()
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
        let near = horses_near(&battle.formation, bi);
        let routing = super::combat::context(battle, bi).is_some_and(|c| c.routing);
        if wounded || routing || battle.columns.targets[bi].is_none() {
            // Carry the formation's approach/withdrawal/rout direction on the
            // horse, never override a routing rider with an enemy pursuit.
            let mut velocity = battle.columns.drive[bi];
            let speed = mth::length2(velocity[0], velocity[2]);
            let position = frame.entities.pos(horse);
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
            if !wounded && !routing {
                let push = horse_push(&battle.formation, bi, &near);
                velocity[0] += push[0];
                velocity[2] += push[1];
            }
            let velocity = battle.formation.constrain_velocity(position, velocity);
            // A rider in the formation heads for its slot; another keeps its drive.
            let formation = &battle.formation;
            let heading = if bi < formation.members.len() {
                let slot = formation.heading_target(bi);
                [slot[0] - position[0], slot[2] - position[2]]
            } else {
                [velocity[0], velocity[2]]
            };
            // A target or slot cannot make a canopy-blocked step jumpable:
            // every mounted body takes the bounded way out of a pocket.
            let mover = super::steering::Mover::Mounted(&mut battle.columns.escape[horse_bi]);
            let velocity = super::steering::around_obstacles(
                frame, formation, horse, velocity, heading, mover,
            );
            let velocity = horse_gap(&battle.formation, bi, &near, position, velocity);
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
            // Each rider closes on its own place round its target: angle from
            // its column index, four rings 1.2..3.6 blocks out. Riding at the
            // target itself piled every rider that chose the same foe onto
            // one block (76 riders on two foot in the G1 rehearsal).
            let angle = bi as f64 * ATTACK_ANGLE;
            let (sin, cos) = (f64::from(mth::sin(angle)), f64::from(mth::cos(angle)));
            let ring = 1.2 + 0.8 * (bi % 4) as f64;
            let place = [
                target.pos[0] + cos * ring - position[0],
                target.pos[2] + sin * ring - position[2],
            ];
            let gap = mth::length2(place[0], place[1]);
            let way = super::steering::unit(place, toward);
            let speed = (if distance <= 46.0 { 6.4 } else { 4.6 }) * (gap / 0.5).min(1.0);
            [way[0] * speed, 0.0, way[1] * speed]
        };
        let push = horse_push(&battle.formation, bi, &near);
        let velocity = [velocity[0] + push[0], velocity[1], velocity[2] + push[1]];
        let velocity = battle.formation.constrain_velocity(position, velocity);
        let velocity = super::steering::around_obstacles(
            frame,
            &battle.formation,
            horse,
            velocity,
            [velocity[0], velocity[2]],
            super::steering::Mover::Mounted(&mut battle.columns.escape[horse_bi]),
        );
        let velocity = horse_gap(&battle.formation, bi, &near, position, velocity);
        battle.columns.drive[horse_bi] = velocity;
        battle.columns.drive[bi] = [0.0; 3];
    }
}

#[cfg(test)]
#[path = "cavalry_tests.rs"]
mod tests;
