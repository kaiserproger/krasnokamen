//! Air-breathing swimming companions, Grace and exact-stack fish feeding.
use crate::boundary::{EntityId, Hand};
use crate::effects::{ActiveEffect, EffectId};
use crate::entities::{EntityKind, EntitySlot};
use crate::intent::{Footprint, Intent, IntentKind, InvClaim};
use crate::tick::TickState;
use mc_nbt::Tag;
use mc_physics::mth::floor_i32;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DolphinState {
    pub got_fish: bool,
}

pub(crate) fn is_dolphin(type_id: u32) -> bool {
    mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(type_id)
        .is_some_and(|c| c.name == "minecraft:dolphin")
}
pub(crate) fn initialize(state: &mut TickState, slot: EntitySlot, type_id: u32) {
    if is_dolphin(type_id) {
        let (body, row) = state.entities.parts_mut(slot);
        body.air_supply = 4800;
        row.entity_life.dolphin = Some(DolphinState::default());
    }
}

pub(crate) fn breathe(air: i32, eye_in_water: bool) -> (i32, bool) {
    if !eye_in_water {
        return (4800, false);
    }
    let next = air.saturating_sub(1);
    if next <= -20 {
        (0, true)
    } else {
        (next, false)
    }
}

/// A low-air dolphin surfaces before following; otherwise follow a wet player
/// (the Grace pass separately requires swimming), or approach a floating toy.
pub(crate) fn drive(
    pos: [f64; 3],
    air: i32,
    speed: f32,
    players: impl Iterator<Item = [f64; 3]>,
    toys: impl Iterator<Item = [f64; 3]>,
    block_at: &dyn Fn(i32, i32, i32) -> Option<u32>,
    registry: &crate::registry::CoreRegistry,
) -> Option<[f64; 3]> {
    if air < 140 {
        return Some([0.0, f64::from(speed).max(1.0), 0.0]);
    }
    let water = |p: [f64; 3]| {
        block_at(floor_i32(p[0]), floor_i32(p[1]), floor_i32(p[2]))
            .is_some_and(|id| registry.materials.classify(id) == mc_physics::BlockMaterial::Water)
    };
    let distance = |other: [f64; 3]| {
        pos.into_iter()
            .zip(other)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
    };
    let target = players
        .filter(|p| water(*p) && distance(*p) < 100.0)
        .min_by(|a, b| distance(*a).total_cmp(&distance(*b)))
        .or_else(|| {
            toys.filter(|p| water(*p) && distance(*p) < 100.0)
                .min_by(|a, b| distance(*a).total_cmp(&distance(*b)))
        })?;
    if distance(target) < 6.25 {
        return Some([0.0; 3]);
    }
    Some(crate::entity_motion::steer(
        crate::blueprint::MovementClass::Swimmer,
        pos,
        target,
        f64::from(speed) * 2.0,
    ))
}

pub(crate) fn run(state: &mut TickState) {
    if !state.tick.is_multiple_of(10) {
        return;
    }
    let dolphins: Vec<_> = state
        .entities
        .slots()
        .filter_map(|slot| {
            let body = state.entities.body(slot);
            let row = state.entities.row(slot);
            (body.alive
                && !row.controls.no_ai
                && is_dolphin(crate::mounts::type_id(body).unwrap_or(u32::MAX)))
            .then_some(state.entities.pos(slot))
        })
        .filter(|pos| crate::conduit::water(state, *pos))
        .collect();
    if dolphins.is_empty() {
        return;
    }
    for index in 0..state.entities.len() {
        let slot = EntitySlot(index as u32);
        let pos = state.entities.pos(slot);
        if !state.world.is_active(
            floor_i32(pos[0]).div_euclid(16),
            floor_i32(pos[2]).div_euclid(16),
        ) {
            continue;
        }
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        let entity = body.id;
        let Some(player) = state.players.get(&entity) else {
            continue;
        };
        if !body.alive
            || player.spectator()
            || player.vitals.is_dead()
            || !(player.sprinting || row.appearance.pose == 3)
            || !crate::conduit::water(state, pos)
            || !dolphins.iter().any(|d| {
                pos.into_iter()
                    .zip(*d)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f64>()
                    < 100.0
            })
        {
            continue;
        }
        let player = state.players.get_mut(&entity).expect("swimmer");
        let connection = player.connection;
        let changes = player.give_effect(ActiveEffect::new(EffectId::DolphinsGrace, 100, 0));
        for change in changes {
            state.push_player(connection, change);
        }
        crate::effects::sync_appearance(state, entity);
    }
    treasure_search(state);
    if state.tick.is_multiple_of(40) {
        // Play by tossing the existing floating item; no pickup/drop duplication.
        for index in 0..state.entities.len() {
            let slot = EntitySlot(index as u32);
            let body = state.entities.body(slot);
            let row = state.entities.row(slot);
            if !body.alive || row.item.is_none() || body.pickup_delay != 0 {
                continue;
            }
            let pos = state.entities.pos(slot);
            if !state.world.is_active(
                floor_i32(pos[0]).div_euclid(16),
                floor_i32(pos[2]).div_euclid(16),
            ) {
                continue;
            }
            if crate::conduit::water(state, pos)
                && dolphins.iter().any(|d| {
                    pos.into_iter()
                        .zip(*d)
                        .map(|(a, b)| (a - b).powi(2))
                        .sum::<f64>()
                        < 4.0
                })
            {
                let sampler = crate::tick::TickSampler {
                    world: &state.world,
                    registry: &state.registry,
                };
                if let Some(moved) = mc_physics::resolve_collision_displacement(
                    &sampler,
                    mc_physics::Vec3::new(pos[0], pos[1], pos[2]),
                    mc_physics::Aabb {
                        half_width: body.half_width,
                        height: body.height,
                    },
                    mc_physics::Vec3::new(0.15, 0.2, 0.15),
                    false,
                    0.0,
                ) {
                    state.entities.set_pos(
                        slot,
                        [
                            pos[0] + moved.delta.x,
                            pos[1] + moved.delta.y,
                            pos[2] + moved.delta.z,
                        ],
                    );
                }
            }
        }
    }
}

fn nearest_treasure(
    state: &TickState,
    at: [f64; 3],
    cache: &mut crate::world::SectionMatchCache,
) -> Option<([f64; 3], f64)> {
    let (cx, cy, cz) = (floor_i32(at[0]), floor_i32(at[1]), floor_i32(at[2]));
    let min = [cx - 48, cy - 24, cz - 48];
    let max = [cx + 48, cy + 24, cz + 48];
    let is_treasure = |id: mc_world::BlockStateId| {
        state
            .registry
            .blocks
            .by_id(id)
            .is_some_and(|b| matches!(b.block.id.path(), "chest" | "trapped_chest" | "barrel"))
    };
    let mut best: Option<([i32; 3], f64)> = None;
    for ((x, y, z), id) in state.world.overlay_entries() {
        let p = [x, y, z];
        if (0..3).all(|i| (min[i]..=max[i]).contains(&p[i]))
            && is_treasure(mc_world::BlockStateId(id))
        {
            let d2 = p
                .into_iter()
                .zip(at)
                .map(|(v, a)| (f64::from(v) + 0.5 - a).powi(2))
                .sum::<f64>();
            if best.is_none_or(|(old, distance)| d2 < distance || (d2 == distance && p < old)) {
                best = Some((p, d2));
            }
        }
    }
    for sx in min[0].div_euclid(16)..=max[0].div_euclid(16) {
        for sy in min[1].div_euclid(16)..=max[1].div_euclid(16) {
            for sz in min[2].div_euclid(16)..=max[2].div_euclid(16) {
                if !state.world.has_chunk(sx, sz) {
                    continue;
                }
                let lo = [
                    min[0].max(sx * 16),
                    min[1].max(sy * 16),
                    min[2].max(sz * 16),
                ];
                let hi = [
                    max[0].min(sx * 16 + 15),
                    max[1].min(sy * 16 + 15),
                    max[2].min(sz * 16 + 15),
                ];
                if !state.world.may_hold_matching_state_cached(
                    (lo[0], lo[1], lo[2]),
                    (hi[0], hi[1], hi[2]),
                    cache,
                    is_treasure,
                ) {
                    continue;
                }
                for x in lo[0]..=hi[0] {
                    for y in lo[1]..=hi[1] {
                        for z in lo[2]..=hi[2] {
                            if !state
                                .world
                                .block_at(x, y, z)
                                .is_some_and(|id| is_treasure(mc_world::BlockStateId(id)))
                            {
                                continue;
                            }
                            let p = [x, y, z];
                            let d2 = p
                                .into_iter()
                                .zip(at)
                                .map(|(v, a)| (f64::from(v) + 0.5 - a).powi(2))
                                .sum::<f64>();
                            if best.is_none_or(|(old, distance)| {
                                d2 < distance || (d2 == distance && p < old)
                            }) {
                                best = Some((p, d2));
                            }
                        }
                    }
                }
            }
        }
    }
    best.map(|(p, d2)| (p.map(|v| f64::from(v) + 0.5), d2))
}

/// DOLPHIN-TREASURE (DolphinSwimToTreasureGoal, 26.1.2): a fed dolphin
/// with air >= 100 locates the nearest DOLPHIN_LOCATED structure
/// (ocean_ruin/shipwreck via findNearestMapStructure radius 50) and swims
/// there; arriving within 4.0 (closerToCenterThan) clears GotFish.
/// Engine approximation: the async structure lookup is requester-bound, so
/// the nearest treasure container within 48 blocks stands in for the
/// structure position. Gate (air >= 100) and arrival radius (4.0) match
/// bytecode.
pub(crate) fn treasure_search(state: &mut TickState) {
    // Collect seekers first (immutable borrow), then steer (mutable).
    let mut seekers: Vec<EntitySlot> = Vec::new();
    for index in 0..state.entities.len() as u32 {
        let slot = EntitySlot(index);
        let body = state.entities.body(slot);
        let EntityKind::Mob { type_id } = body.kind else {
            continue;
        };
        if !body.alive
            || body.air_supply < 100
            || !is_dolphin(type_id)
            || !state
                .entities
                .row(slot)
                .entity_life
                .dolphin
                .as_ref()
                .is_some_and(|d| d.got_fish)
        {
            continue;
        }
        seekers.push(slot);
    }
    if seekers.is_empty() {
        return;
    }
    let mut cache = crate::world::SectionMatchCache::default();
    cache.reset(&state.world);
    for slot in seekers {
        let at = state.entities.pos(slot);
        if !crate::conduit::water(state, at) {
            continue;
        }
        let Some((target, d2)) = nearest_treasure(state, at, &mut cache) else {
            continue;
        };
        if d2 < 16.0 {
            // Treasure found: clear GotFish (vanilla stops leading here).
            if let Some(d) = state.entities.row_mut(slot).entity_life.dolphin.as_mut() {
                d.got_fish = false;
            }
            continue;
        }
        // Swim toward the treasure (vanilla speed ~0.5 blocks/tick pace,
        // applied per 10-tick run: 5-block steps).
        let dist = d2.sqrt();
        let step = (5.0 / dist).min(1.0);
        let desired = mc_physics::Vec3::new(
            (target[0] - at[0]) * step,
            (target[1] - at[1]) * step,
            (target[2] - at[2]) * step,
        );
        let body = state.entities.body(slot);
        let sampler = crate::tick::TickSampler {
            world: &state.world,
            registry: &state.registry,
        };
        if let Some(moved) = mc_physics::resolve_collision_displacement(
            &sampler,
            mc_physics::Vec3::new(at[0], at[1], at[2]),
            mc_physics::Aabb {
                half_width: body.half_width,
                height: body.height,
            },
            desired,
            false,
            0.0,
        ) {
            state.entities.set_pos(
                slot,
                [
                    at[0] + moved.delta.x,
                    at[1] + moved.delta.y,
                    at[2] + moved.delta.z,
                ],
            );
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeedPlan {
    pub target: EntityId,
    pub slot: usize,
    pub held: mc_data::ItemStack,
}
fn fish(state: &TickState, held: &mc_data::ItemStack) -> bool {
    !held.is_empty()
        && state
            .registry
            .items
            .name_of(held.item_id)
            .is_some_and(|name| {
                matches!(
                    name.path(),
                    "cod"
                        | "salmon"
                        | "tropical_fish"
                        | "pufferfish"
                        | "cooked_cod"
                        | "cooked_salmon"
                )
            })
}
pub(crate) fn valid(state: &TickState, actor: EntityId, plan: &FeedPlan) -> bool {
    state.players.get(&actor).is_some_and(|p| {
        !p.spectator()
            && !p.vitals.is_dead()
            && p.inventory.slots.get(plan.slot) == Some(&plan.held)
    }) && fish(state, &plan.held)
        && state.entities.slot_of(plan.target).is_some_and(|slot| {
            let body = state.entities.body(slot);
            body.alive && is_dolphin(crate::mounts::type_id(body).unwrap_or(u32::MAX))
        })
}
pub(crate) fn interact(
    state: &mut TickState,
    actor: EntityId,
    target: EntityId,
    hand: Hand,
    intents: &mut Vec<Intent>,
) -> bool {
    let Some(player) = state.players.get(&actor) else {
        return false;
    };
    let slot = match hand {
        Hand::Main => usize::from(player.inventory.selected.min(8)),
        Hand::Off => crate::inventory::OFF_HAND,
    };
    let plan = FeedPlan {
        target,
        slot,
        held: player.inventory.slots[slot].clone(),
    };
    if !valid(state, actor, &plan) {
        return false;
    }
    let take = i32::from(!player.creative());
    let footprint = Footprint {
        entity_reads: vec![actor],
        entity_writes: vec![target],
        inv: vec![InvClaim {
            owner: actor,
            slot: slot as u16,
            expect_item: plan.held.item_id,
            expect_count: plan.held.count,
            take,
            put_item: None,
            put_count: 0,
        }],
        ..Default::default()
    };
    let seq = state.next_seq(actor);
    intents.push(Intent {
        actor,
        seq,
        kind: IntentKind::DolphinFeed { plan },
        footprint,
    });
    true
}
pub(crate) fn feed(state: &mut TickState, plan: &FeedPlan) {
    let slot = state
        .entities
        .slot_of(plan.target)
        .expect("validated dolphin");
    if state.entities.row(slot).entity_life.child_ticks > 0 {
        crate::entity_life::feed(state, plan.target);
    } else {
        state
            .entities
            .row_mut(slot)
            .entity_life
            .dolphin
            .get_or_insert_with(DolphinState::default)
            .got_fish = true;
    }
    crate::mounts::status(state, plan.target, 38);
}

pub(crate) fn from_record(
    record: &mc_data::entity_nbt::VanillaEntity,
) -> Result<Option<DolphinState>, crate::CoreError> {
    if record.id != "minecraft:dolphin" {
        return Ok(None);
    }
    let mut dolphin = DolphinState::default();
    let mut found = false;
    for (key, tag) in &record.extra_fields {
        if key == "GotFish" {
            if found {
                return Err(crate::CoreError::Storage(
                    "dolphin GotFish: duplicate field".into(),
                ));
            }
            found = true;
            match tag {
                Tag::Byte(value) if matches!(*value, 0 | 1) => dolphin.got_fish = *value != 0,
                _ => {
                    return Err(crate::CoreError::Storage(
                        "dolphin GotFish: expected TAG_Byte boolean".into(),
                    ));
                }
            }
        }
    }
    Ok(Some(dolphin))
}
pub(crate) fn to_fields(dolphin: &DolphinState) -> Vec<(String, Tag)> {
    vec![("GotFish".into(), Tag::Byte(i8::from(dolphin.got_fish)))]
}

#[cfg(test)]
#[path = "dolphin_tests.rs"]
mod tests;
