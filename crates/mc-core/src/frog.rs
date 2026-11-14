//! Frogspawn laying and the frog's tongue attack.
use crate::boundary::{BlockPos, EntityDataValue, EntityId};
use crate::entities::{ColdRow, EntityKind, EntitySlot};
use crate::intent::{DamageSource, Footprint, Intent, IntentKind};
use crate::tick::TickState;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Hunt {
    target: EntityId,
    target_pos: [f64; 3],
    started: u64,
    /// None while approaching, otherwise ticks since shooting the tongue.
    animation: Option<u8>,
}

const TONGUE_TARGET_INDEX: u8 = 19;
const USING_TONGUE_POSE: u8 = 9;

fn tongue_metadata(
    state: &mut TickState,
    slot: EntitySlot,
    target: Option<EntityId>,
    shooting: bool,
) {
    let row = state.entities.row_mut(slot);
    let appearance = Arc::make_mut(&mut row.appearance);
    appearance.pose = if shooting { USING_TONGUE_POSE } else { 0 };
    let value = EntityDataValue::OptionalUnsignedInt(target.map(|id| id.0 as u32));
    if let Some((_, old)) = appearance
        .extra
        .iter_mut()
        .find(|(index, _)| *index == TONGUE_TARGET_INDEX)
    {
        *old = value;
    } else {
        appearance.extra.push((TONGUE_TARGET_INDEX, value));
    }
    state.appearance_changed.push(state.entities.body(slot).id);
}

fn edible(state: &TickState, slot: EntitySlot) -> bool {
    let body = state.entities.body(slot);
    let EntityKind::Mob { type_id } = body.kind else {
        return false;
    };
    body.alive
        && state.registry.tags.contains_raw_id(
            "minecraft:entity_type",
            "minecraft:frog_food",
            type_id as i32,
        )
        && state.entities.row(slot).loot.slime_size.unwrap_or(1) == 1
}

fn prey(state: &TickState, frog: EntitySlot) -> Option<EntityId> {
    let at = state.entities.pos(frog);
    let chunk = |v: f64| mc_physics::mth::floor_i32(v).div_euclid(16);
    let mut best: Option<(EntityId, f64)> = None;
    for z in chunk(at[2] - 10.0)..=chunk(at[2] + 10.0) {
        for x in chunk(at[0] - 10.0)..=chunk(at[0] + 10.0) {
            state
                .entities
                .for_each_slot_in_chunk(crate::boundary::ChunkPos { x, z }, |slot| {
                    if !edible(state, slot) {
                        return;
                    }
                    let pos = state.entities.pos(slot);
                    let distance = (0..3).map(|i| (at[i] - pos[i]).powi(2)).sum::<f64>();
                    let id = state.entities.body(slot).id;
                    if distance <= 100.0
                        && best.is_none_or(|(old, d)| distance < d || (distance == d && id < old))
                        && crate::hostile::sight_clear(
                            state,
                            [at[0], at[1] + 0.5, at[2]],
                            [
                                pos[0],
                                pos[1] + state.entities.body(slot).height * 0.5,
                                pos[2],
                            ],
                        )
                    {
                        best = Some((id, distance));
                    }
                });
        }
    }
    best.map(|(id, _)| id)
}

fn sound(state: &mut TickState, slot: EntitySlot, name: &str) {
    let pos = state.entities.pos(slot);
    let sound = mc_data::Identifier::parse(name).expect("frog sound");
    let listeners: Vec<_> = state
        .players
        .iter()
        .filter_map(|(id, player)| {
            let listener = state.entities.slot_of(*id)?;
            let at = state.entities.pos(listener);
            ((0..3).map(|i| (at[i] - pos[i]).powi(2)).sum::<f64>() <= 1024.0)
                .then_some(player.connection)
        })
        .collect();
    for connection in listeners {
        state.push_player(
            connection,
            crate::boundary::PlayerChange::CommandSound {
                sound: sound.clone(),
                source: 5,
                position: pos.map(|v| (v * 8.0).floor() as i32),
                volume: 2.0,
                pitch: 1.0,
            },
        );
    }
}

static TYPE: std::sync::LazyLock<u32> =
    std::sync::LazyLock::new(|| crate::piglin::type_id_of("minecraft:frog").expect("frog type"));

pub(crate) fn is_frog(type_id: u32) -> bool {
    type_id == *TYPE
}

pub(crate) fn drive(row: &ColdRow) -> Option<([f64; 3], f64)> {
    if let Some(hunt) = &row.entity_life.frog_hunt {
        return Some((
            hunt.target_pos,
            if hunt.animation.is_none() {
                f64::from(row.speed) * 2.0
            } else {
                0.0
            },
        ));
    }
    if !row.entity_life.frog_pregnant {
        return None;
    }
    let target = row.entity_life.frog_spawn_target?;
    Some((
        [
            f64::from(target[0]) + 0.5,
            f64::from(target[1]),
            f64::from(target[2]) + 0.5,
        ],
        f64::from(row.speed),
    ))
}

fn spawn_at(state: &TickState, shore: [i32; 3]) -> Option<BlockPos> {
    let [x, y, z] = shore;
    let ground = state.world.block_at(x, y - 1, z)?;
    if state.registry.materials.classify(ground) != mc_physics::BlockMaterial::Solid
        || state.world.block_at(x, y, z) != Some(state.registry.states.air)
    {
        return None;
    }
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let water = state.world.block_at(x + dx, y - 1, z + dz);
        if water.is_some_and(|id| {
            state.registry.materials.classify(id) == mc_physics::BlockMaterial::Water
        }) && state.world.block_at(x + dx, y, z + dz) == Some(state.registry.states.air)
        {
            return Some(BlockPos {
                x: x + dx,
                y,
                z: z + dz,
            });
        }
    }
    None
}

fn site(state: &TickState, pos: [f64; 3]) -> Option<[i32; 3]> {
    let origin = pos.map(|v| v.floor() as i32);
    let mut best = None;
    for x in origin[0] - 8..=origin[0] + 8 {
        for z in origin[2] - 8..=origin[2] + 8 {
            for y in origin[1] - 3..=origin[1] + 3 {
                let shore = [x, y, z];
                if spawn_at(state, shore).is_none() {
                    continue;
                }
                let distance = (f64::from(x) + 0.5 - pos[0]).powi(2)
                    + (f64::from(y) - pos[1]).powi(2)
                    + (f64::from(z) + 0.5 - pos[2]).powi(2);
                if best.is_none_or(|(_, old)| distance < old) {
                    best = Some((shore, distance));
                }
            }
        }
    }
    best.map(|(shore, _)| shore)
}
fn hunt(state: &mut TickState, slot: EntitySlot, intents: &mut Vec<Intent>) {
    let id = state.entities.body(slot).id;
    let pos = state.entities.pos(slot);
    let mut current = state.entities.row(slot).entity_life.frog_hunt.clone();
    if current.is_none() && state.tick.wrapping_add(id.0 as u64).is_multiple_of(20) {
        if let Some(target) = prey(state, slot)
            && let Some(target_slot) = state.entities.slot_of(target)
        {
            current = Some(Hunt {
                target,
                target_pos: state.entities.pos(target_slot),
                started: state.tick,
                animation: None,
            });
            tongue_metadata(state, slot, Some(target), false);
        }
    }
    let Some(mut hunt) = current else {
        return;
    };
    let target_slot = state.entities.slot_of(hunt.target);
    let valid = target_slot.is_some_and(|target| edible(state, target))
        && state.tick.saturating_sub(hunt.started) <= 100;
    if !valid {
        state.entities.row_mut(slot).entity_life.frog_hunt = None;
        tongue_metadata(state, slot, None, false);
        return;
    }
    let target_slot = target_slot.expect("valid target");
    hunt.target_pos = state.entities.pos(target_slot);
    let delta = [
        pos[0] - hunt.target_pos[0],
        pos[1] - hunt.target_pos[1],
        pos[2] - hunt.target_pos[2],
    ];
    let distance = delta.iter().map(|v| v * v).sum::<f64>().sqrt();
    match hunt.animation {
        None if distance < 1.75 => {
            let scale = if distance > 0.0 { 15.0 / distance } else { 0.0 };
            state
                .entities
                .set_vel(target_slot, delta.map(|v| v * scale));
            hunt.animation = Some(0);
            tongue_metadata(state, slot, Some(hunt.target), true);
            sound(state, slot, "minecraft:entity.frog.tongue");
        }
        Some(6) => {
            let seq = state.next_seq(id);
            intents.push(Intent {
                actor: id,
                seq,
                kind: IntentKind::DamageEntity {
                    target: hunt.target,
                    amount: state.entities.row(slot).attack_damage,
                    kind: crate::survival::DamageKind::Melee,
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
                    entity_reads: vec![id],
                    entity_writes: vec![hunt.target],
                    ..Default::default()
                },
            });
            hunt.animation = Some(7);
            sound(state, slot, "minecraft:entity.frog.eat");
        }
        Some(ticks) if ticks >= 10 => {
            state.entities.row_mut(slot).entity_life.frog_hunt = None;
            tongue_metadata(state, slot, None, false);
            return;
        }
        Some(ticks) => hunt.animation = Some(ticks + 1),
        None => {}
    }
    state.entities.row_mut(slot).entity_life.frog_hunt = Some(hunt);
}

pub(crate) fn run(state: &mut TickState, intents: &mut Vec<Intent>) {
    let slots: Vec<_> = state.entities.mob_slots(*TYPE).collect();
    for slot in slots {
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        let id = body.id;
        if !body.alive || row.controls.no_ai {
            if row.entity_life.frog_hunt.is_some() {
                state.entities.row_mut(slot).entity_life.frog_hunt = None;
                tongue_metadata(state, slot, None, false);
            }
            continue;
        }
        let pos = state.entities.pos(slot);
        if !state.world.is_active(
            (pos[0].floor() as i32).div_euclid(16),
            (pos[2].floor() as i32).div_euclid(16),
        ) {
            continue;
        }
        if !state.entities.row(slot).entity_life.frog_pregnant {
            hunt(state, slot, intents);
            continue;
        }
        if state.entities.row(slot).entity_life.frog_hunt.is_some() {
            state.entities.row_mut(slot).entity_life.frog_hunt = None;
            tongue_metadata(state, slot, None, false);
        }
        let here = pos.map(|v| v.floor() as i32);
        if state.entities.ground(slot)
            && let Some(at) = spawn_at(state, here)
        {
            let name = mc_data::Identifier::parse("minecraft:frogspawn").expect("frogspawn id");
            if let Some(block) = state.registry.blocks.block(&name) {
                let block = block.default.0;
                state.write_block(at, block);
                let life = &mut state.entities.row_mut(slot).entity_life;
                life.frog_pregnant = false;
                life.frog_spawn_target = None;
            }
        } else if state.tick.wrapping_add(id.0 as u64).is_multiple_of(20) {
            let target = site(state, pos);
            state.entities.row_mut(slot).entity_life.frog_spawn_target = target;
        }
    }
}

#[cfg(test)]
#[path = "frog_tests.rs"]
mod tests;
