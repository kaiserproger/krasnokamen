//! Simplified village assault goals, sharing collision, spell and projectile owners.
use super::*;
use crate::entities::EntityKind;
use mc_physics::{Aabb, EntityBody, Vec3};
use std::sync::LazyLock;

/// Direct raider classification: dense protocol-ID index, no per-call strings.
///
/// The generated entity-type contract table is immutable, so the six raider
/// memberships resolve once below; every hot call is one indexed load.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Raider {
    Vindicator,
    Ravager,
    Evoker,
    Pillager,
    Witch,
    Illusioner,
}

impl Raider {
    fn name(self) -> &'static str {
        match self {
            Self::Vindicator => "minecraft:vindicator",
            Self::Ravager => "minecraft:ravager",
            Self::Evoker => "minecraft:evoker",
            Self::Pillager => "minecraft:pillager",
            Self::Witch => "minecraft:witch",
            Self::Illusioner => "minecraft:illusioner",
        }
    }

    fn ranged(self) -> bool {
        !matches!(self, Self::Vindicator | Self::Ravager)
    }
}

static RAIDER_BY_TYPE: LazyLock<[Option<Raider>; mc_data::entity_types::ENTITY_TYPE_COUNT]> =
    LazyLock::new(|| {
        let mut table = [None; mc_data::entity_types::ENTITY_TYPE_COUNT];
        for raider in [
            Raider::Vindicator,
            Raider::Ravager,
            Raider::Evoker,
            Raider::Pillager,
            Raider::Witch,
            Raider::Illusioner,
        ] {
            if let Some(contract) =
                mc_data::entity_types::entity_type_contract_26_1_2_by_name(raider.name())
                && let Some(slot) = table.get_mut(contract.protocol_id as usize)
            {
                *slot = Some(raider);
            }
        }
        table
    });

#[inline]
fn classify(type_id: u32) -> Option<Raider> {
    RAIDER_BY_TYPE.get(type_id as usize).copied().flatten()
}

#[inline]
pub(crate) fn owns(kind: EntityKind) -> bool {
    matches!(kind,EntityKind::Mob{type_id} if classify(type_id).is_some())
}

/// Summoned illusioners carry a bow; equipment uses the normal entity image.
pub(crate) fn initialize(state: &mut TickState, slot: EntitySlot, type_id: u32) {
    if classify(type_id) == Some(Raider::Illusioner)
        && let Some(bow) = item_id(state, "bow")
    {
        std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance).equipment[0] =
            Some(mc_data::ItemStack::new(bow, 1));
    }
}
/// Vanilla `NearestAttackableTargetGoal` default `randomInterval`
/// (`reducedTickDelay(10)`): an unengaged raider searches with probability
/// 1/10 per tick (javap `NearestAttackableTargetGoal.canUse`).
const SEARCH_INTERVAL: u64 = 10;
/// Vanilla `TargetGoal.unseenMemoryTicks`: a held target out of sight for more
/// than this many ticks is dropped (javap `TargetGoal.canContinueToUse`).
const UNSEEN_MEMORY_TICKS: u32 = 60;

/// The target a raider keeps between ticks (vanilla `Mob.getTarget`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct HeldTarget {
    pub(crate) target: EntityId,
    /// Consecutive ticks without line of sight (vanilla `unseenTicks`).
    unseen: u32,
}

/// Whether a slot is something raiders attack: a live survival/adventure
/// player, a villager that is not converting, or an iron golem.
fn candidate(state: &TickState, s: EntitySlot) -> bool {
    let row = state.entities.row(s);
    let body = state.entities.body(s);
    if !body.alive || row.health <= 0.0 {
        return false;
    }
    match body.kind {
        EntityKind::Player { .. } => state.players.get(&body.id).is_some_and(|p| {
            !p.creative() && !p.spectator() && !state.session_locked(p.connection)
        }),
        EntityKind::Mob { type_id } => {
            row.villager
                .as_ref()
                .is_some_and(|v| v.conversion_time.is_none())
                || mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(type_id)
                    .is_some_and(|c| c.name == "minecraft:iron_golem")
        }
        _ => false,
    }
}

/// Deterministic stand-in for `mob.getRandom().nextInt(SEARCH_INTERVAL) == 0`.
fn search_due(state: &TickState, id: EntityId) -> bool {
    let mut x = (state.world.seed() as u64)
        ^ state.tick.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (id.0 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x.is_multiple_of(SEARCH_INTERVAL)
}

/// One tick of the raider target policy, vanilla-shaped: a held target stays
/// while it is a valid candidate within `range` (follow range) and not unseen
/// for more than [`UNSEEN_MEMORY_TICKS`]; losing it ends the goal and the next
/// search waits for the 1-in-[`SEARCH_INTERVAL`] roll. Only an unengaged raider
/// that wins the roll scans for the nearest visible candidate. Returns the
/// target with its current position and the memory to keep.
fn choose_target(
    state: &TickState,
    id: EntityId,
    slot: EntitySlot,
    held: Option<HeldTarget>,
    targets: &crate::target_grid::TargetGrid,
    range: f64,
) -> (Option<(EntityId, [f64; 3])>, Option<HeldTarget>) {
    if let Some(held) = held {
        let pos = state.entities.pos(slot);
        let kept = state
            .entities
            .slot_of(held.target)
            .filter(|&target| candidate(state, target))
            .map(|target| state.entities.pos(target))
            .filter(|at| distance_sq(pos, *at) <= range * range);
        let Some(at) = kept else {
            return (None, None);
        };
        let unseen = if visible(state, slot, at) {
            0
        } else {
            held.unseen + 1
        };
        if unseen > UNSEEN_MEMORY_TICKS {
            return (None, None);
        }
        let target = held.target;
        return (Some((target, at)), Some(HeldTarget { target, unseen }));
    }
    if !search_due(state, id) {
        return (None, None);
    }
    match target_in(state, slot, targets, range) {
        Some(found) => (
            Some(found),
            Some(HeldTarget {
                target: found.0,
                unseen: 0,
            }),
        ),
        None => (None, None),
    }
}

pub(crate) fn run(state: &mut TickState, intents: &mut Vec<Intent>) {
    // Admission is the exact raider type-membership list (`owns` classifier
    // over `EntityKind::Mob`); only the dynamic alive filter stays per slot.
    // Targets follow `choose_target`: held between ticks, searched on cadence.
    let ids: Vec<_> = state
        .entities
        .mob_slots_matching(|type_id| owns(EntityKind::Mob { type_id }))
        .into_iter()
        .filter_map(|s| {
            let row = state.entities.body(s);
            row.alive.then_some(row.id)
        })
        .collect();
    let entities = &state.entities;
    state
        .hostile
        .native
        .raider_targets
        .retain(|id, _| entities.slot_of(*id).is_some());
    if ids.is_empty() {
        return;
    }
    let targets: Vec<_> = state
        .entities
        .slots()
        .filter(|&s| candidate(state, s))
        .map(|s| (state.entities.body(s).id, state.entities.pos(s)))
        .collect();
    let grid = crate::target_grid::TargetGrid::for_consumers(&targets, ids.len());
    for id in ids {
        let Some(slot) = state.entities.slot_of(id) else {
            continue;
        };
        let pos0 = state.entities.pos(slot);
        if !state.world.is_active(
            (pos0[0].floor() as i32).div_euclid(16),
            (pos0[2].floor() as i32).div_euclid(16),
        ) {
            continue;
        }
        let row = state.entities.row(slot);
        if row.controls.no_ai {
            continue;
        }
        let EntityKind::Mob { type_id } = state.entities.body(slot).kind else {
            continue;
        };
        let raider = classify(type_id).unwrap();
        let pos = state.entities.pos(slot);
        let (target, held) = choose_target(
            state,
            id,
            slot,
            state.hostile.native.raider_targets.get(&id).copied(),
            &grid,
            f64::from(row.sight_range),
        );
        // A held target is chased from memory, but attacked only in sight
        // (vanilla `MeleeAttackGoal.canPerformAttack` -> `Sensing.hasLineOfSight`).
        let seen = held.is_some_and(|held| held.unseen == 0);
        match held {
            Some(held) => {
                state.hostile.native.raider_targets.insert(id, held);
            }
            None => {
                state.hostile.native.raider_targets.remove(&id);
            }
        }
        let ranged = raider.ranged();
        let destination = target.map(|(_, p)| p).or_else(|| {
            state
                .raids
                .raids
                .values()
                .find(|r| r.status == "ongoing" && r.members.contains(&row.uuid))
                .map(|r| r.center.map(f64::from))
        });
        let stop = ranged && target.is_some_and(|(_, at)| distance_sq(pos, at) < 36.0);
        let drive = if stop {
            [0.0; 3]
        } else if let Some(at) = destination {
            crate::entity_motion::steer(
                state.entities.body(slot).movement,
                pos,
                at,
                f64::from(row.speed),
            )
        } else {
            crate::entity_motion::wander(
                state.entities.body(slot).movement,
                id.0,
                state.tick,
                row.speed,
            )
        };
        let motion = crate::entity_motion::step(
            crate::entity_motion::MotionInput {
                body: EntityBody {
                    position: Vec3::new(pos[0], pos[1], pos[2]),
                    velocity: {
                        let v = state.entities.vel(slot);
                        Vec3::new(v[0], v[1], v[2])
                    },
                    aabb: Aabb {
                        half_width: state.entities.body(slot).half_width,
                        height: state.entities.body(slot).height,
                    },
                    on_ground: state.entities.ground(slot),
                },
                movement: state.entities.body(slot).movement,
                drive,
                impulse: state.entities.horizontal_impulse(slot),
                rotation: state.entities.rot(slot),
                look_target: destination,
                climbs_walls: false,
                aquatic_control: crate::species::AquaticControl::Other,
                air_supply: state.entities.body(slot).air_supply,
                levitation: 0.0,
                slow_falling: false,
                no_gravity: row.controls.no_gravity,
                id: id.0,
                tick: state.tick,
            },
            &crate::tick::TickSampler {
                world: &state.world,
                registry: &state.registry,
            },
            |x, z| state.world.has_chunk(x.div_euclid(16), z.div_euclid(16)),
        );
        state.entities.set_pos(
            slot,
            [
                motion.body.position.x,
                motion.body.position.y,
                motion.body.position.z,
            ],
        );
        state.entities.set_vel(
            slot,
            [
                motion.body.velocity.x,
                motion.body.velocity.y,
                motion.body.velocity.z,
            ],
        );
        state
            .entities
            .set_rot(slot, motion.rotation[0], motion.rotation[1]);
        state.entities.set_ground(slot, motion.body.on_ground);
        state.entities.set_horizontal_impulse(slot, motion.impulse);
        state.entities.body_mut(slot).air_supply = motion.air_supply;
        state.entities.body_mut(slot).attack_cooldown =
            state.entities.body(slot).attack_cooldown.saturating_sub(1);
        if let Some(kind) = state.registry.blueprint(type_id).and_then(goal) {
            super::run(state, intents, id, kind, &grid);
        } else if raider == Raider::Illusioner {
            illusioner(state, id, slot, target.filter(|_| seen));
        } else if let Some((victim, at)) = target.filter(|_| seen) {
            let reach = state.entities.body(slot).half_width + 1.5;
            if distance_sq(pos, at) < reach * reach
                && state.entities.body(slot).attack_cooldown == 0
            {
                let seq = state.next_seq(id);
                intents.push(Intent {
                    actor: id,
                    seq,
                    kind: IntentKind::DamageEntity {
                        target: victim,
                        amount: state
                            .difficulty
                            .scale_mob_damage(state.entities.row(slot).attack_damage),
                        kind: DamageKind::Melee,
                        source: Some(DamageSource::Melee {
                            attacker: id,
                            position: pos,
                            yaw: state.entities.rot(slot)[0],
                            bonus_knockback: 0.0,
                            sweep: false,
                            fire_ticks: 0,
                            bane_ticks: 0,
                        }),
                    },
                    footprint: Footprint {
                        entity_reads: vec![id, victim],
                        entity_writes: vec![victim],
                        ..Default::default()
                    },
                });
                state.entities.body_mut(slot).attack_cooldown = 20;
            }
        }
    }
}
fn illusioner(
    state: &mut TickState,
    id: EntityId,
    slot: EntitySlot,
    target: Option<(EntityId, [f64; 3])>,
) {
    let pos = state.entities.pos(slot);
    let Some((victim, at)) = target.filter(|(_, at)| distance_sq(pos, *at) <= 16.0 * 16.0) else {
        return;
    };
    let mut attack = state.hostile.native.attacks.remove(&id).unwrap_or_default();
    let has_bow = state.entities.row(slot).appearance.equipment[0]
        .as_ref()
        .is_some_and(|stack| Some(stack.item_id) == item_id(state, "bow") && !stack.is_empty());
    if has_bow && state.tick >= attack.deadline {
        let pos = state.entities.pos(slot);
        if crate::hostile::launch_arrow(
            state,
            id,
            [pos[0], pos[1] + 1.5, pos[2]],
            shooters::velocity(
                [pos[0], pos[1] + 1.5, pos[2]],
                [at[0], at[1] + 1.0, at[2]],
                1.6,
            ),
            2.0,
        )
        .is_some()
        {
            attack.deadline = state.tick + 40;
        }
    }
    if state.tick >= attack.summon_at {
        effect(
            state,
            id,
            ActiveEffect::new(EffectId::Invisibility, 1200, 0),
        );
        effect(
            state,
            victim,
            ActiveEffect::new(EffectId::Blindness, 400, 0),
        );
        attack.summon_at = state.tick + 1200;
    }
    state.hostile.native.attacks.insert(id, attack);
}
