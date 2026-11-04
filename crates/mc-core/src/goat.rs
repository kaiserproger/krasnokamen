//! Goat ram preparation, charge and horn loss.
use crate::boundary::{ChangeSet, EntityDataValue, EntityId};
use crate::entities::{ColdRow, EntityKind};
use crate::intent::{DamageSource, Footprint, Intent, IntentKind};
use crate::natural_spawn::Rng;
use crate::special_mobs::Retained;
use crate::tick::TickState;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

static GOAT_TYPE: std::sync::LazyLock<u32> = std::sync::LazyLock::new(|| {
    mc_data::entity_types::entity_type_contract_26_1_2_by_name("minecraft:goat")
        .expect("goat type")
        .protocol_id
});

pub(crate) fn is_goat(type_id: u32) -> bool {
    type_id == *GOAT_TYPE
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Idle(u64),
    Align {
        target: Uuid,
        start: [i32; 3],
        destination: [i32; 3],
        deadline: u64,
    },
    Prepare {
        target: Uuid,
        destination: [i32; 3],
        until: u64,
    },
    Charge {
        target: Uuid,
        destination: [i32; 3],
        deadline: u64,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goat {
    pub phase: Phase,
    pub screaming: bool,
    pub left: bool,
    pub right: bool,
}

pub(crate) fn to_fields(goat: &Goat) -> Vec<(String, mc_nbt::Tag)> {
    vec![
        (
            "IsScreamingGoat".into(),
            mc_nbt::Tag::Byte(i8::from(goat.screaming)),
        ),
        ("HasLeftHorn".into(), mc_nbt::Tag::Byte(i8::from(goat.left))),
        (
            "HasRightHorn".into(),
            mc_nbt::Tag::Byte(i8::from(goat.right)),
        ),
    ]
}

pub(crate) fn drive(row: &ColdRow) -> Option<([f64; 3], f64)> {
    let Some(Retained::Goat(goat)) = row.special_mob.as_deref() else {
        return None;
    };
    match goat.phase {
        Phase::Idle(_) => None,
        Phase::Align { start, .. } => Some((
            start.map(|v| f64::from(v) + 0.5),
            f64::from(row.speed) * 1.25,
        )),
        Phase::Prepare { destination, .. } => Some((destination.map(|v| f64::from(v) + 0.5), 0.0)),
        Phase::Charge { destination, .. } => Some((
            destination.map(|v| f64::from(v) + 0.5),
            f64::from(row.speed) * 3.0,
        )),
    }
}
fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum()
}
fn cooldown(goat: &Goat, tick: u64, rng: &mut Rng) -> Phase {
    let (base, range) = if goat.screaming {
        (100, 201)
    } else {
        (600, 5401)
    };
    Phase::Idle(tick + base + rng.below(range))
}
fn valid_target(state: &TickState, id: EntityId, goat: EntityId) -> bool {
    let Some(slot) = state.entities.slot_of(id) else {
        return false;
    };
    let body = state.entities.body(slot);
    if id == goat || !body.alive || state.entities.row(slot).health <= 0.0 {
        return false;
    }
    match body.kind {
        EntityKind::Player { .. } => state
            .players
            .get(&id)
            .is_some_and(|p| !p.creative() && !p.spectator()),
        EntityKind::Mob { type_id } => {
            mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(type_id).is_some_and(
                |c| {
                    c.name != "minecraft:goat"
                        && c.behavior.archetype == mc_data::entity_types::EntityArchetype::Living
                },
            )
        }
        _ => false,
    }
}
fn start_pos(state: &TickState, goat: [f64; 3], target: [i32; 3]) -> Option<[i32; 3]> {
    let mut best = None;
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        for distance in 4..=7 {
            let candidate = [
                target[0] + dx * distance,
                target[1],
                target[2] + dz * distance,
            ];
            let clear = (1..=distance).all(|n| {
                let x = target[0] + dx * n;
                let z = target[2] + dz * n;
                (target[1]..=target[1] + 1).all(|y| {
                    state.world.block_at(x, y, z).is_some_and(|block| {
                        state.registry.materials.classify(block) == mc_physics::BlockMaterial::Air
                    })
                }) && state
                    .world
                    .block_at(x, target[1] - 1, z)
                    .is_some_and(|block| {
                        state.registry.materials.classify(block) == mc_physics::BlockMaterial::Solid
                    })
            });
            if clear {
                let d = dist(goat, candidate.map(|v| f64::from(v) + 0.5));
                if best.is_none_or(|(_, old)| d < old) {
                    best = Some((candidate, d));
                }
            }
        }
    }
    best.map(|(pos, _)| pos)
}
fn drop_horn(
    state: &mut TickState,
    slot: crate::entities::EntitySlot,
    goat: &mut Goat,
    changes: &mut ChangeSet,
    rng: &mut Rng,
) {
    if !crate::entity_life::adult(state.entities.row(slot)) || (!goat.left && !goat.right) {
        return;
    }
    let left = goat.left && (!goat.right || rng.below(2) == 0);
    if left {
        goat.left = false;
    } else {
        goat.right = false;
    }
    let id = state.entities.body(slot).id;
    crate::special_mobs::metadata(
        state,
        slot,
        if left { 19 } else { 20 },
        EntityDataValue::Boolean(false),
    );
    state.appearance_changed.push(id);
    let Some(item) = crate::special_mobs::item_id(state, "goat_horn") else {
        return;
    };
    let mut stack = mc_data::ItemStack::new(item, 1);
    let names = if goat.screaming {
        ["admire", "call", "yearn", "dream"]
    } else {
        ["ponder", "sing", "seek", "feel"]
    };
    let instrument = format!(
        "minecraft:{}_goat_horn",
        names[(state.entities.row(slot).uuid.as_u128() as usize) % 4]
    );
    let mut value = Vec::new();
    mc_nbt::write_network_value(&mut value, &mc_nbt::Tag::String(instrument))
        .expect("instrument NBT");
    stack
        .extended_mut()
        .preserved_components
        .push(mc_data::item_stack::PreservedItemComponent {
            name: "minecraft:instrument".into(),
            value,
            baseline: None,
        });
    changes
        .entities_spawned
        .push(crate::tick::spawn_item_entity(
            state,
            state.entities.pos(slot),
            stack,
        ));
}

pub(crate) fn run(state: &mut TickState, intents: &mut Vec<Intent>, changes: &mut ChangeSet) {
    let goats: Vec<_> = state.entities.mob_slots(*GOAT_TYPE).collect();
    for slot in goats {
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        let id = body.id;
        let pos = state.entities.pos(slot);
        if !body.alive
            || row.controls.no_ai
            || !state.world.is_active(
                (pos[0].floor() as i32).div_euclid(16),
                (pos[2].floor() as i32).div_euclid(16),
            )
        {
            continue;
        }
        let mut rng = Rng(state.world.seed() as u64 ^ state.tick.rotate_left(17) ^ id.0 as u64);
        let mut goat = match row.special_mob.as_deref() {
            Some(Retained::Goat(value)) => value.clone(),
            Some(_) => continue,
            None => {
                let screaming = rng.below(50) == 0;
                let mut value = Goat {
                    phase: Phase::Idle(0),
                    screaming,
                    left: true,
                    right: true,
                };
                if rng.below(10) == 0 {
                    if rng.below(2) == 0 {
                        value.left = false;
                    } else {
                        value.right = false;
                    }
                }
                value.phase = cooldown(&value, state.tick, &mut rng);
                crate::special_mobs::metadata(
                    state,
                    slot,
                    18,
                    EntityDataValue::Boolean(value.screaming),
                );
                crate::special_mobs::metadata(
                    state,
                    slot,
                    19,
                    EntityDataValue::Boolean(value.left),
                );
                crate::special_mobs::metadata(
                    state,
                    slot,
                    20,
                    EntityDataValue::Boolean(value.right),
                );
                state.appearance_changed.push(id);
                value
            }
        };
        let old = goat.clone();
        match goat.phase.clone() {
            Phase::Idle(until) if state.tick >= until => {
                let target = state
                    .entities
                    .slots()
                    .filter_map(|s| {
                        let other = state.entities.body(s).id;
                        let at = state.entities.pos(s);
                        let d = dist(pos, at);
                        (d <= 256.0 && valid_target(state, other, id)).then_some((
                            state.entities.row(s).uuid,
                            at,
                            d,
                        ))
                    })
                    .min_by(|a, b| a.2.total_cmp(&b.2));
                if let Some((target, at, _)) = target {
                    let destination = at.map(|v| v.floor() as i32);
                    if let Some(start) = start_pos(state, pos, destination) {
                        goat.phase = Phase::Align {
                            target,
                            start,
                            destination,
                            deadline: state.tick + 200,
                        };
                    } else {
                        goat.phase = cooldown(&goat, state.tick, &mut rng);
                    }
                } else {
                    goat.phase = cooldown(&goat, state.tick, &mut rng);
                }
            }
            Phase::Align {
                target,
                start,
                destination,
                deadline,
            } => {
                let valid = state.entities.slot_of_uuid(target).is_some_and(|s| {
                    valid_target(state, state.entities.body(s).id, id)
                        && dist(
                            state.entities.pos(s),
                            destination.map(|v| f64::from(v) + 0.5),
                        ) < 2.0
                });
                if !valid || state.tick >= deadline {
                    goat.phase = cooldown(&goat, state.tick, &mut rng);
                } else if dist(pos, start.map(|v| f64::from(v) + 0.5)) < 1.0 {
                    goat.phase = Phase::Prepare {
                        target,
                        destination,
                        until: state.tick + 20,
                    };
                    crate::special_mobs::event(state, id, 58);
                }
            }
            Phase::Prepare {
                target,
                destination,
                until,
            } => {
                let valid = state.entities.slot_of_uuid(target).is_some_and(|s| {
                    valid_target(state, state.entities.body(s).id, id)
                        && dist(
                            state.entities.pos(s),
                            destination.map(|v| f64::from(v) + 0.5),
                        ) < 2.0
                });
                if !valid {
                    crate::special_mobs::event(state, id, 59);
                    goat.phase = cooldown(&goat, state.tick, &mut rng);
                } else if state.tick >= until {
                    goat.phase = Phase::Charge {
                        target,
                        destination,
                        deadline: state.tick + 100,
                    };
                }
            }
            Phase::Charge {
                target,
                destination,
                deadline,
            } => {
                let target_slot = state.entities.slot_of_uuid(target);
                let hit = target_slot.filter(|&s| {
                    valid_target(state, state.entities.body(s).id, id)
                        && dist(pos, state.entities.pos(s))
                            <= (state.entities.body(slot).half_width
                                + state.entities.body(s).half_width
                                + 0.5)
                                .powi(2)
                });
                if let Some(target_slot) = hit {
                    let victim = state.entities.body(target_slot).id;
                    let seq = state.next_seq(id);
                    let damage = if crate::entity_life::adult(state.entities.row(slot)) {
                        2.0
                    } else {
                        1.0
                    };
                    intents.push(Intent {
                        actor: id,
                        seq,
                        kind: IntentKind::DamageEntity {
                            target: victim,
                            amount: damage,
                            kind: crate::survival::DamageKind::Melee,
                            source: Some(DamageSource::Melee {
                                attacker: id,
                                position: pos,
                                yaw: state.entities.rot(slot)[0],
                                bonus_knockback: 2.5,
                                sweep: false,
                                fire_ticks: 0,
                                bane_ticks: 0,
                            }),
                        },
                        footprint: Footprint {
                            entity_reads: vec![id],
                            entity_writes: vec![victim],
                            ..Default::default()
                        },
                    });
                    crate::special_mobs::event(state, id, 59);
                    goat.phase = cooldown(&goat, state.tick, &mut rng);
                } else {
                    let dx = f64::from(destination[0]) + 0.5 - pos[0];
                    let dz = f64::from(destination[2]) + 0.5 - pos[2];
                    let length = (dx * dx + dz * dz).sqrt();
                    let front = if length > 0.001 {
                        [
                            pos[0] + dx / length * 0.8,
                            pos[1],
                            pos[2] + dz / length * 0.8,
                        ]
                    } else {
                        pos
                    };
                    let [x, y, z] = front.map(|v| v.floor() as i32);
                    let obstacle = state
                        .world
                        .block_at(x, y, z)
                        .and_then(|b| state.registry.blocks.by_id(mc_world::BlockStateId(b)));
                    let blocked = obstacle.is_some_and(|b| {
                        state.registry.materials.classify(b.id.0) != mc_physics::BlockMaterial::Air
                    });
                    if blocked
                        && obstacle.is_some_and(|b| {
                            state.registry.tags.contains_raw_id(
                                "minecraft:block",
                                "minecraft:snaps_goat_horn",
                                b.block.raw_id,
                            )
                        })
                    {
                        drop_horn(state, slot, &mut goat, changes, &mut rng);
                    }
                    if blocked || state.tick >= deadline || length < 0.4 {
                        crate::special_mobs::event(state, id, 59);
                        goat.phase = cooldown(&goat, state.tick, &mut rng);
                    }
                }
            }
            _ => {}
        }
        if old != goat
            || !matches!(
                state.entities.row(slot).special_mob.as_deref(),
                Some(Retained::Goat(_))
            )
        {
            state.entities.row_mut(slot).special_mob = Some(Box::new(Retained::Goat(goat)));
        }
    }
}
