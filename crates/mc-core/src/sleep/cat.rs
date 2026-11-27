//! Tamed cat relaxation and morning gifts around a sleeping owner.
use crate::boundary::EntityId;
use crate::cat::{Relax, TYPE};
use crate::entities::EntitySlot;
use crate::tick::TickState;
use mc_data::entity_data_value::EntityDataValue;
use std::sync::Arc;

fn metadata(state: &mut TickState, slot: EntitySlot, lying: bool, relaxing: bool) {
    let extra = &state.entities.row(slot).appearance.extra;
    if [(21, lying), (22, relaxing)].iter().all(|(index, value)| {
        extra
            .iter()
            .find(|(i, _)| i == index)
            .map(|(_, v)| v)
            .unwrap_or(&EntityDataValue::Boolean(false))
            == &EntityDataValue::Boolean(*value)
    }) {
        return;
    }
    let row = state.entities.row_mut(slot);
    let appearance = Arc::make_mut(&mut row.appearance);
    let mut changed = false;
    for (index, value) in [(21, lying), (22, relaxing)] {
        let value = EntityDataValue::Boolean(value);
        if let Some((_, old)) = appearance.extra.iter_mut().find(|(i, _)| *i == index) {
            if *old != value {
                *old = value;
                changed = true;
            }
        } else if value == EntityDataValue::Boolean(true) {
            appearance.extra.push((index, value));
            changed = true;
        }
    }
    if changed {
        state.appearance_changed.push(state.entities.body(slot).id);
    }
}

pub(super) fn run(state: &mut TickState) {
    let cats: Vec<_> = state.entities.mob_slots(*TYPE).collect();
    for slot in cats {
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        let owner = row.entity_life.taming.owner;
        let eligible =
            body.alive && !row.controls.no_ai && row.entity_life.tamed && !row.entity_life.sitting;
        let Some(owner_slot) = owner.and_then(|owner| state.entities.slot_of_uuid(owner)) else {
            if row.entity_life.cat_relax.is_some() {
                state.entities.row_mut_unmarked(slot).entity_life.cat_relax = None;
                metadata(state, slot, false, false);
            }
            continue;
        };
        let owner_id = state.entities.body(owner_slot).id;
        let Some(sleeper) = state.sleepers.get(&owner_id).copied().filter(|_| eligible) else {
            if state.entities.row(slot).entity_life.cat_relax.is_some() {
                state.entities.row_mut_unmarked(slot).entity_life.cat_relax = None;
                metadata(state, slot, false, false);
            }
            continue;
        };
        let pos = state.entities.pos(slot);
        let owner_pos = state.entities.pos(owner_slot);
        let distance = (0..3).map(|i| (pos[i] - owner_pos[i]).powi(2)).sum::<f64>();
        let existing = state.entities.row(slot).entity_life.cat_relax.clone();
        if existing.is_none() && distance > 100.0 {
            continue;
        }
        let bed = sleeper.bed;
        let facing = state
            .world
            .block_at(bed.x, bed.y, bed.z)
            .and_then(|id| state.registry.blocks.by_id(mc_world::BlockStateId(id)))
            .and_then(|block| block.properties.iter().find(|(key, _)| key == "facing"))
            .map(|(_, value)| value.as_str())
            .unwrap_or("north");
        let (dx, dz) = match facing {
            "north" => (0, 1),
            "south" => (0, -1),
            "east" => (-1, 0),
            "west" => (1, 0),
            _ => (0, 0),
        };
        let target = [
            f64::from(bed.x + dx) + 0.5,
            f64::from(bed.y),
            f64::from(bed.z + dz) + 0.5,
        ];
        let occupied = state.entities.mob_slots(*TYPE).any(|other| {
            if other == slot {
                return false;
            }
            let row = state.entities.row(other);
            let Some(relax) = &row.entity_life.cat_relax else {
                return false;
            };
            relax.on_bed_ticks > 0
                && (0..3).all(|i| (state.entities.pos(other)[i] - target[i]).abs() <= 2.0)
        });
        if occupied {
            state.entities.row_mut_unmarked(slot).entity_life.cat_relax = None;
            metadata(state, slot, false, false);
            continue;
        }
        let mut relax = existing.unwrap_or(Relax {
            owner: owner.expect("owner"),
            target,
            on_bed_ticks: 0,
        });
        relax.target = target;
        if distance < 2.5 {
            relax.on_bed_ticks = relax.on_bed_ticks.saturating_add(1);
        } else {
            relax.on_bed_ticks = 0;
        }
        metadata(
            state,
            slot,
            relax.on_bed_ticks > 16,
            (1..=16).contains(&relax.on_bed_ticks),
        );
        state.entities.row_mut_unmarked(slot).entity_life.cat_relax = Some(relax);
    }
}

fn gift_teleport(state: &mut TickState, slot: EntitySlot) {
    let original = state.entities.pos(slot);
    let base = match state.entities.row(slot).entity_life.leash.as_ref() {
        Some(crate::leash::Holder::Entity(uuid)) => state
            .entities
            .slot_of_uuid(*uuid)
            .map(|holder| state.entities.pos(holder))
            .unwrap_or(original),
        Some(crate::leash::Holder::Fence(pos)) => pos.map(f64::from),
        None => original,
    };
    let row = state.entities.row_mut(slot);
    let x = base[0].floor() as i32 + crate::taming::roll(row, 11) as i32 - 5;
    let y = base[1].floor() as i32 + crate::taming::roll(row, 5) as i32 - 2;
    let z = base[2].floor() as i32 + crate::taming::roll(row, 11) as i32 - 5;
    let body = state.entities.body(slot);
    let (width, height, id) = (body.half_width, body.height, body.id);
    for floor in (state.world.geometry().min_y() + 1..=y).rev() {
        let Some(ground) = state.world.block_at(x, floor - 1, z) else {
            return;
        };
        if state.registry.materials.classify(ground) != mc_physics::BlockMaterial::Solid {
            continue;
        }
        let at = [f64::from(x) + 0.5, f64::from(floor), f64::from(z) + 0.5];
        let clear = (mc_physics::mth::floor_i32(at[0] - width)
            ..=mc_physics::mth::floor_i32(at[0] + width))
            .all(|bx| {
                (mc_physics::mth::floor_i32(at[2] - width)
                    ..=mc_physics::mth::floor_i32(at[2] + width))
                    .all(|bz| {
                        (floor..=mc_physics::mth::floor_i32(at[1] + height - 1.0e-6)).all(|by| {
                            state.world.block_at(bx, by, bz).is_some_and(|block| {
                                state.registry.collision_boxes[block as usize].is_empty()
                                    && state.registry.materials.classify(block)
                                        != mc_physics::BlockMaterial::Water
                            })
                        })
                    })
            });
        if clear {
            state.entities.set_pos(slot, at);
            state.entities.set_vel(slot, [0.0; 3]);
            state.entities.set_ground(slot, true);
            state.appearance_changed.push(id);
        }
        break;
    }
}

/// Resolve each active relaxation when its owner wakes.
pub(super) fn wake(state: &mut TickState, owner: EntityId, slept: u32) {
    let Some(owner_slot) = state.entities.slot_of_any(owner) else {
        return;
    };
    let uuid = state.entities.row(owner_slot).uuid;
    let cats: Vec<_> = state
        .entities
        .mob_slots(*TYPE)
        .filter(|slot| {
            state
                .entities
                .row(*slot)
                .entity_life
                .cat_relax
                .as_ref()
                .is_some_and(|relax| relax.owner == uuid)
        })
        .collect();
    for slot in cats {
        state.entities.row_mut_unmarked(slot).entity_life.cat_relax = None;
        metadata(state, slot, false, false);
        let time = state.day_time.rem_euclid(24_000);
        if slept < 100
            || (362..23_667).contains(&time)
            || crate::taming::roll(state.entities.row_mut(slot), 10) >= 7
        {
            continue;
        }
        let id =
            mc_data::Identifier::parse("minecraft:gameplay/cat_morning_gift").expect("gift id");
        gift_teleport(state, slot);
        let seed = state.tick ^ state.entities.row(slot).uuid.as_u128() as u64;
        let Some(table) = state.registry.chest_loot.get(&id) else {
            continue;
        };
        let Ok(stacks) = table.roll(
            &mut mc_data::loot::chest_26_1_2::ChestRng::new(seed),
            &state.registry.items,
            0.0,
        ) else {
            continue;
        };
        let pos = state.entities.pos(slot);
        state
            .merge_life_drops
            .extend(stacks.into_iter().map(|stack| (pos, stack)));
    }
}
