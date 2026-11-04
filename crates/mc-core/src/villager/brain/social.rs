//! Food sharing, gossip, courtship and golem summoning.
//! Moved verbatim from `brain.rs`.

use super::*;

pub(super) fn food_points(food: &FoodTable, item: u32) -> i32 {
    food.entries
        .get(item as usize)
        .map_or(0, |entry| i32::from(entry.points))
}
pub(super) fn total_food(food: &FoodTable, v: &VillagerState) -> i32 {
    v.food_level
        + v.inventory
            .iter()
            .flatten()
            .map(|s| s.count.max(0) * food_points(food, s.item_id))
            .sum::<i32>()
}
pub(super) fn willing(food: &FoodTable, v: &VillagerState) -> bool {
    v.age == 0 && !v.sleeping && v.activity != Activity::Panic && total_food(food, v) >= 12
}
fn willing_with_production(
    state: &TickState,
    slot: EntitySlot,
    food: &FoodTable,
    v: &VillagerState,
) -> bool {
    willing(food, v) && available_food(state, slot, food, v) >= 12
}

pub(super) fn available_food(
    state: &TickState,
    slot: EntitySlot,
    food: &FoodTable,
    v: &VillagerState,
) -> i32 {
    let protected = crate::settlements::economy::protected_carry(state, slot, v);
    let reserved: i32 = v
        .inventory
        .iter()
        .zip(protected)
        .filter_map(|(stack, count)| {
            stack
                .as_ref()
                .map(|stack| count as i32 * food_points(food, stack.item_id))
        })
        .sum();
    total_food(food, v) - reserved
}

/// Keep a personal breeding ration in addition to production's carry holds.
pub(in crate::villager) fn retain_food(
    state: &TickState,
    v: &VillagerState,
    protected: &mut [u32; 8],
) {
    let mut needed = (12 - v.food_level).max(0);
    for (stack, held) in v.inventory.iter().zip(protected.iter_mut()) {
        let Some(stack) = stack else { continue };
        let points = state
            .registry
            .items
            .name_of(stack.item_id)
            .map_or(0, |name| i32::from(food_points_for_name(name.path())));
        if points == 0 || needed == 0 {
            continue;
        }
        let free = (stack.count.max(0) as u32).saturating_sub(*held);
        let count = free.min(((needed + points - 1) / points) as u32);
        *held += count;
        needed = needed.saturating_sub(count as i32 * points);
    }
}

/// Take a breeding ration from actual town storage, at interaction range only.
/// The container authority owns fences, physical double chests and stock holds.
pub(super) fn provision_food(
    state: &mut TickState,
    cache: &mut PoiCache,
    slot: EntitySlot,
    food: &FoodTable,
    v: &mut VillagerState,
) -> Option<BlockPos> {
    use crate::settlements::inventory;

    let settlement = v.settlement?;
    let mut needed = 12 - available_food(state, slot, food, v);
    if needed <= 0 {
        return None;
    }
    let worker = state.entities.row(slot).uuid;
    let mut destination = None;
    for (position, before) in inventory::town_storage(state, settlement) {
        let Ok(mut free) = inventory::unreserved_stock(state, &before) else {
            continue;
        };
        let pos = BlockPos {
            x: position[0],
            y: position[1],
            z: position[2],
        };
        let near = super::super::mining::near(state, slot, pos, 2.5);
        let original = v.inventory.clone();
        let mut after = before.slots.clone();
        let mut remaining = needed;
        for stack in &mut after {
            let points = food_points(food, stack.item_id);
            if stack.is_empty()
                || points == 0
                || crate::settlements::economy::defer_output_pickup(
                    state,
                    v.settlement,
                    worker,
                    stack,
                )
            {
                continue;
            }
            let available = free
                .get(&stack.item_id)
                .copied()
                .unwrap_or(0)
                .min(stack.count as u64) as i32;
            let count = available.min((remaining + points - 1) / points);
            if count <= 0 {
                continue;
            }
            let mut moving = stack.clone();
            moving.count = count;
            let maximum = state.registry.item_rules().max_stack(&moving);
            let moved = add_inventory(v, &mut moving, maximum);
            stack.count -= moved;
            if stack.count == 0 {
                *stack = ItemStack::EMPTY;
            }
            *free.get_mut(&moving.item_id).expect("available stock") -= moved as u64;
            remaining -= moved * points;
            if remaining <= 0 {
                break;
            }
        }
        if remaining == needed {
            v.inventory = original;
            continue;
        }
        let distance = distance(state.entities.pos(slot), centre(pos));
        if !near && destination.is_some_and(|(_, best)| distance >= best) {
            v.inventory = original;
            continue;
        }
        if !cache.spend_path(state, Consumer::Brain) {
            v.inventory = original;
            break;
        }
        let from = at(state.entities.pos(slot));
        let horizontal = from
            .x
            .abs_diff(pos.x)
            .max(from.z.abs_diff(pos.z))
            .saturating_add(8)
            .clamp(48, 512);
        let vertical = from.y.abs_diff(pos.y).saturating_add(4).clamp(16, 512);
        let body = state.entities.body(slot);
        let (width, height) = (body.half_width, body.height);
        let mut path = std::mem::take(&mut cache.candidate_path);
        let mut ctx = allocator::RouteCtx {
            world: &state.world,
            registry: &state.registry,
            dimension: state.dimension,
            consumer: Consumer::Brain,
            budget: &mut state.tick_budget,
        };
        let reachable = allocator::find_path_ctx(
            &mut ctx,
            cache,
            state.entities.pos(slot),
            pos,
            width,
            height,
            &mut path,
            false,
            horizontal,
            vertical,
            true,
        );
        if reachable && !near {
            std::mem::swap(&mut v.navigation_path, &mut path);
            v.navigation_target = Some(pos);
            v.navigation_search_failed = false;
            allocator::capture_route(state, v, state.entities.pos(slot), true);
            v.navigation_repath_at = state.tick + 40;
        }
        path.clear();
        cache.candidate_path = path;
        if !reachable {
            v.inventory = original;
            if allocator::nodes_exhausted(state) {
                break;
            }
            continue;
        }
        if !near {
            v.inventory = original;
            destination = Some((pos, distance));
            continue;
        }
        if inventory::commit(state, vec![inventory::Change { before, after }]).is_err() {
            v.inventory = original;
            continue;
        }
        needed = remaining;
        if needed <= 0 {
            return None;
        }
    }
    destination.map(|(pos, _)| pos)
}
pub(super) fn eat_for_birth(food: &FoodTable, v: &mut VillagerState, protected: &[u32; 8]) {
    for (index, slot) in v.inventory.iter_mut().enumerate() {
        if v.food_level >= 12 {
            break;
        }
        if let Some(stack) = slot {
            let points = food_points(food, stack.item_id);
            while points > 0 && stack.count > protected[index] as i32 && v.food_level < 12 {
                stack.count -= 1;
                v.food_level += points;
            }
            if stack.count == 0 {
                *slot = None;
            }
        }
    }
    v.food_level -= 12;
}
pub(in crate::villager) fn add_inventory(
    v: &mut VillagerState,
    moving: &mut ItemStack,
    maximum: i32,
) -> i32 {
    let before = moving.count;
    if before <= 0 || maximum <= 0 {
        return 0;
    }
    for slot in &mut v.inventory {
        if let Some(existing) = slot
            && mc_data::inventory_semantics_26_1_2::can_stack(existing, moving)
        {
            let amount = (maximum - existing.count).max(0).min(moving.count);
            existing.count += amount;
            moving.count -= amount;
        }
        if moving.count == 0 {
            return before;
        }
    }
    for slot in &mut v.inventory {
        if slot.is_none() {
            let amount = moving.count.min(maximum);
            let mut stack = moving.clone();
            stack.count = amount;
            *slot = Some(stack);
            moving.count -= amount;
        }
        if moving.count == 0 {
            break;
        }
    }
    before - moving.count
}
pub(super) fn pickup(
    state: &mut TickState,
    food: &FoodTable,
    items: &[EntitySlot],
    slot: EntitySlot,
    v: &mut VillagerState,
    changes: &mut ChangeSet,
) {
    if !crate::command_rules::boolean(state, crate::command_rules::BoolRule::MOB_GRIEFING)
        || v.sleeping
    {
        return;
    }
    let pos = state.entities.pos(slot);
    let collector = state.entities.body(slot).id;
    for &item_slot in items {
        let body = state.entities.body(item_slot);
        let row = state.entities.row(item_slot);
        if !body.alive || !matches!(body.kind, EntityKind::Item { .. }) || body.pickup_delay > 0 {
            continue;
        }
        let at = state.entities.pos(item_slot);
        if (pos[0] - at[0]).abs() > PICKUP_REACH
            || (pos[2] - at[2]).abs() > PICKUP_REACH
            || at[1] + 0.25 < pos[1]
            || at[1] > pos[1] + state.entities.body(slot).height
        {
            continue;
        }
        let Some(item) = row.item.as_ref() else {
            continue;
        };
        let edible = food_points(food, item.item_id) > 0;
        let farmer = v.data.profession == Profession::Farmer
            && food
                .entries
                .get(item.item_id as usize)
                .is_some_and(|entry| entry.farmer_pickup);
        if !edible && !farmer {
            continue;
        }
        let mut moving = item.full_stack();
        if crate::settlements::economy::defer_output_pickup(
            state,
            v.settlement,
            state.entities.row(slot).uuid,
            &moving,
        ) {
            continue;
        }
        let id = body.id;
        let maximum = state.registry.item_rules().max_stack(&moving);
        let count = add_inventory(v, &mut moving, maximum);
        if count == 0 {
            continue;
        }
        // Both custodians mutate in this single tick-owner transaction. Dead
        // consumed rows remain slot-stable until all villagers have finished.
        let (body, row) = state.entities.parts_mut(item_slot);
        if moving.count == 0 {
            body.alive = false;
            row.item = None;
        } else {
            if let Some(item) = row.item.as_mut() {
                item.count = moving.count;
                item.stack = Some(moving.clone());
            }
            std::sync::Arc::make_mut(&mut row.appearance).item = Some(moving);
            state.appearance_changed.push(id);
        }
        changes.entity_events.push(EntityEvent::Pickup {
            item: id,
            collector,
            count: count.clamp(0, 255) as u8,
        });
    }
}
pub(super) fn share_food(
    state: &mut TickState,
    food_table: &FoodTable,
    slot: EntitySlot,
    v: &mut VillagerState,
    changes: &mut ChangeSet,
) {
    if v.sleeping || v.activity == Activity::Panic || !state.tick.is_multiple_of(20) {
        return;
    }
    let food = v
        .inventory
        .iter()
        .flatten()
        .map(|s| food_points(food_table, s.item_id) * s.count)
        .sum::<i32>();
    if food < 24 {
        return;
    }
    let pos = state.entities.pos(slot);
    let recipient = bodies_near(state, pos, 3.0, true).find(|&other| {
        other != slot
            && active(state, other)
            && distance(pos, state.entities.pos(other)) <= 5.0
            && state
                .entities
                .row(other)
                .villager
                .as_ref()
                .is_some_and(|v| total_food(food_table, v) - v.food_level < 12)
    });
    let Some(recipient) = recipient else {
        return;
    };
    let protected = crate::settlements::economy::protected_carry(state, slot, v);
    for (index, inventory) in v.inventory.iter_mut().enumerate() {
        let Some(stack) = inventory else {
            continue;
        };
        if food_points(food_table, stack.item_id) == 0 {
            continue;
        }
        let maximum = state.registry.item_rules().max_stack(stack);
        let count = if stack.count > maximum / 2 {
            stack.count / 2
        } else {
            (stack.count - 24).max(0)
        }
        .min(stack.count.saturating_sub(protected[index] as i32));
        if count <= 0 {
            continue;
        }
        let mut shared = stack.clone();
        shared.count = count;
        stack.count -= count;
        let target = state.entities.pos(recipient);
        let dx = target[0] - pos[0];
        let dz = target[2] - pos[2];
        let length = mth::length2(dx, dz).max(1e-6);
        let id = crate::tick::spawn_item_entity(state, [pos[0], pos[1] + 1.32, pos[2]], shared);
        if let Some(item) = state.entities.slot_of(id) {
            state
                .entities
                .set_vel(item, [dx / length * 6.0, 2.0, dz / length * 6.0]);
        }
        changes.entities_spawned.push(id);
        if stack.count == 0 {
            *inventory = None;
        }
        break;
    }
}

pub(super) fn gossip_limits(kind: &str) -> Option<(i32, i32, i32, i32)> {
    // maximum, daily decay, transfer decay, absolute reputation weight.
    Some(match kind {
        "trading" => (25, 2, 20, 1),
        "minor_negative" => (200, 20, 20, 1),
        "major_negative" => (100, 10, 10, 5),
        "minor_positive" => (25, 1, 5, 1),
        "major_positive" => (20, 0, 20, 5),
        _ => return None,
    })
}
pub(crate) fn record_gossip(v: &mut VillagerState, target: uuid::Uuid, kind: &str, amount: i32) {
    let Some((maximum, _, _, _)) = gossip_limits(kind) else {
        return;
    };
    if let Some(g) = v
        .gossips
        .iter_mut()
        .find(|g| g.target == target && g.kind == kind)
    {
        g.value = (g.value + amount).clamp(0, maximum);
    } else if amount >= 2
        && (v.gossips.iter().any(|g| g.target == target)
            || v.gossips
                .iter()
                .enumerate()
                .filter(|(i, g)| !v.gossips[..*i].iter().any(|other| other.target == g.target))
                .count()
                < 64)
    {
        v.gossips.push(Gossip {
            target,
            kind: kind.to_owned(),
            value: amount.min(maximum),
        });
    }
    v.gossips.retain(|g| g.value >= 2);
}
pub(super) fn decay_gossip(v: &mut VillagerState, tick: u64) {
    if v.last_gossip_decay == 0 {
        v.last_gossip_decay = tick;
        return;
    }
    if tick.saturating_sub(v.last_gossip_decay) < 24_000 {
        return;
    }
    for g in &mut v.gossips {
        if let Some((_, decay, _, _)) = gossip_limits(&g.kind) {
            g.value -= decay;
        }
    }
    v.gossips.retain(|g| g.value >= 2);
    v.last_gossip_decay = tick;
}
pub(super) fn transfer_gossip(receiver: &mut VillagerState, source: &VillagerState, seed: u64) {
    let total = source
        .gossips
        .iter()
        .filter_map(|g| gossip_limits(&g.kind).map(|(_, _, _, w)| g.value.max(0) as u32 * w as u32))
        .sum::<u32>();
    if total == 0 {
        return;
    }
    let mut random = (seed ^ 0x5DEECE66D) & ((1 << 48) - 1);
    let mut selected = [usize::MAX; 10];
    let mut n = 0;
    for _ in 0..10 {
        let choice = loop {
            random = random.wrapping_mul(0x5DEECE66D).wrapping_add(11) & ((1 << 48) - 1);
            let bits = (random >> 17) as u32;
            if total.is_power_of_two() {
                break ((u64::from(total) * u64::from(bits)) >> 31) as u32;
            }
            let value = bits % total;
            if bits.wrapping_sub(value).wrapping_add(total - 1) as i32 >= 0 {
                break value;
            }
        };
        let mut cumulative = 0;
        for (index, g) in source.gossips.iter().enumerate() {
            let Some((_, _, decay, weight)) = gossip_limits(&g.kind) else {
                continue;
            };
            cumulative += g.value.max(0) as u32 * weight as u32;
            if choice >= cumulative {
                continue;
            }
            if !selected[..n].contains(&index) {
                selected[n] = index;
                n += 1;
                let value = g.value - decay;
                if value >= 2 {
                    let existing = receiver
                        .gossips
                        .iter()
                        .find(|r| r.target == g.target && r.kind == g.kind)
                        .map_or(0, |r| r.value);
                    record_gossip(receiver, g.target, &g.kind, (value - existing).max(0));
                }
            }
            break;
        }
    }
}
/// Called only after an accepted player hit, before infection/removal.
pub(crate) fn harmed(
    state: &mut TickState,
    victim: EntityId,
    attacker: Option<EntityId>,
    killed: bool,
) {
    let Some(victim) = state.entities.slot_of_any(victim) else {
        return;
    };
    if state
        .entities
        .row(victim)
        .villager
        .as_ref()
        .is_none_or(|v| v.wandering || v.conversion_time.is_some())
    {
        return;
    }
    let Some(player) = attacker
        .and_then(|id| state.entities.slot_of_any(id))
        .filter(|&slot| matches!(state.entities.body(slot).kind, EntityKind::Player { .. }))
        .map(|slot| state.entities.row(slot).uuid)
    else {
        return;
    };
    let pos = state.entities.pos(victim);
    let mut witnesses = std::mem::take(&mut state.villager_pois.nearby_residents);
    witnesses.clear();
    if killed {
        witnesses.extend(bodies_near(state, pos, 16.0, true));
    } else {
        witnesses.push(victim);
    }
    for &witness in &witnesses {
        if (!killed && witness != victim)
            || (killed && !active(state, witness))
            || distance(pos, state.entities.pos(witness)) > 256.0
        {
            continue;
        }
        if let Some(v) = state.entities.row_mut(witness).villager.as_mut() {
            record_gossip(
                v,
                player,
                if killed {
                    "major_negative"
                } else {
                    "minor_negative"
                },
                25,
            );
        }
    }
    state.villager_pois.nearby_residents = witnesses;
}
pub(super) fn interact_gossip(state: &mut TickState, cache: &PoiCache) {
    let count = cache.actors().len();
    let start = (state.tick.wrapping_mul(64) % count.max(1) as u64) as usize;
    for offset in 0..count.min(64) {
        let first = cache.actors()[(start + offset) % count];
        let i = first.0 as usize;
        if !active(state, first) || !cache.admitted(first) {
            continue;
        }
        let a = state.entities.row(first).villager.as_ref().unwrap();
        if a.sleeping
            || a.activity == Activity::Panic
            || state.tick.saturating_sub(a.last_gossip) < 1_200
        {
            continue;
        }
        let pos = state.entities.pos(first);
        let second = bodies_near(state, pos, 3.0, true).find(|&s| {
            s.0 > first.0
                && active(state, s)
                && distance(pos, state.entities.pos(s)) <= 5.0
                && state.entities.row(s).villager.as_ref().is_some_and(|b| {
                    !b.sleeping
                        && b.activity != Activity::Panic
                        && state.tick.saturating_sub(b.last_gossip) >= 1_200
                })
        });
        let Some(second) = second else {
            continue;
        };
        let mut a = state.entities.row_mut(first).villager.take().unwrap();
        let b = state.entities.row_mut(second).villager.as_mut().unwrap();
        transfer_gossip(&mut a, b, state.tick ^ i as u64);
        a.last_gossip = state.tick;
        b.last_gossip = state.tick;
        state.entities.row_mut(first).villager = Some(a);
    }
}

pub(super) fn abort_courtship(v: &mut VillagerState) {
    v.partner = None;
    v.courtship_until = None;
    v.courtship_started_tick = None;
}
pub(super) fn courtship(state: &mut TickState, cache: &mut PoiCache, changes: &mut ChangeSet) {
    // Birth searches charge the same shared pool as movement.
    let count = cache.actors().len();
    for offset in 0..count {
        let first = cache.actors()[offset];
        if !active(state, first) || !cache.admitted(first) {
            continue;
        }
        let a = state.entities.row(first).villager.as_ref().unwrap();
        let first_id = state.entities.body(first).id;
        if let Some(partner) = a.partner {
            let Some(second) = state.entities.slot_of(partner) else {
                abort_courtship(state.entities.row_mut(first).villager.as_mut().unwrap());
                continue;
            };
            if first_id.0 > partner.0 {
                continue;
            }
            let valid = active(state, second)
                && state
                    .entities
                    .row(second)
                    .villager
                    .as_ref()
                    .is_some_and(|b| {
                        b.partner == Some(first_id)
                            && b.courtship_until == a.courtship_until
                            && b.courtship_started_tick == a.courtship_started_tick
                            && willing_with_production(state, second, &cache.food, b)
                    })
                && willing_with_production(state, first, &cache.food, a)
                && a.courtship_started_tick.is_some()
                && crate::settlements::households::can_breed(state, [first_id, partner])
                && distance(state.entities.pos(first), state.entities.pos(second)) <= 8.0 * 8.0;
            let due = a.courtship_until.is_some_and(|until| state.tick >= until);
            if !valid {
                abort_courtship(state.entities.row_mut(first).villager.as_mut().unwrap());
                if let Some(b) = state.entities.row_mut(second).villager.as_mut()
                    && b.partner == Some(first_id)
                {
                    abort_courtship(b);
                }
                continue;
            }
            if !due {
                continue;
            }
            if distance(state.entities.pos(first), state.entities.pos(second)) > 5.0 {
                abort_courtship(state.entities.row_mut(first).villager.as_mut().unwrap());
                abort_courtship(state.entities.row_mut(second).villager.as_mut().unwrap());
                continue;
            }
            let mut path = std::mem::take(
                &mut state
                    .entities
                    .row_mut(first)
                    .villager
                    .as_mut()
                    .unwrap()
                    .navigation_path,
            );
            let bed = free_bed(state, cache, first, &mut path);
            if bed.is_none() && allocator::nodes_exhausted(state) {
                state
                    .entities
                    .row_mut(first)
                    .villager
                    .as_mut()
                    .unwrap()
                    .navigation_path = path;
                continue;
            }
            let birth_sequence = if bed.is_some() {
                let ordinal = state
                    .entities
                    .slots()
                    .filter(|&slot| {
                        state
                            .entities
                            .row(slot)
                            .villager
                            .as_ref()
                            .and_then(|v| v.birth_origin.as_ref())
                            .is_some_and(|origin| origin.birth_tick == state.tick)
                    })
                    .count() as u64
                    + 1;
                let Some(sequence) = state
                    .tick
                    .checked_mul(65_536)
                    .and_then(|start| start.checked_add(ordinal))
                    .filter(|_| ordinal <= 65_535)
                else {
                    let v = state.entities.row_mut(first).villager.as_mut().unwrap();
                    v.navigation_path = path;
                    abort_courtship(v);
                    abort_courtship(state.entities.row_mut(second).villager.as_mut().unwrap());
                    continue;
                };
                Some(sequence)
            } else {
                None
            };
            let protected_a = crate::settlements::economy::protected_carry(
                state,
                first,
                state.entities.row(first).villager.as_ref().unwrap(),
            );
            let protected_b = crate::settlements::economy::protected_carry(
                state,
                second,
                state.entities.row(second).villager.as_ref().unwrap(),
            );
            let mut a = state.entities.row_mut(first).villager.take().unwrap();
            a.navigation_path = path;
            let kind = a.data.kind;
            let pos = state.entities.pos(first);
            let started_tick = a.courtship_started_tick.expect("validated courtship clock");
            eat_for_birth(&cache.food, &mut a, &protected_a);
            abort_courtship(&mut a);
            let b = state.entities.row_mut(second).villager.as_mut().unwrap();
            eat_for_birth(&cache.food, b, &protected_b);
            abort_courtship(b);
            if bed.is_some() {
                a.age = 6_000;
                b.age = 6_000;
            }
            state.entities.row_mut(first).villager = Some(a);
            if let Some(home) = bed {
                let EntityKind::Mob { type_id } = state.entities.body(first).kind else {
                    continue;
                };
                let id = state.spawn_mob(type_id, pos, 20.0);
                if let Some(child) = state.entities.slot_of(id) {
                    let dimension = state.dimension.name().to_owned();
                    let mut parents = [
                        state.entities.row(first).uuid.to_string(),
                        state.entities.row(second).uuid.to_string(),
                    ];
                    parents.sort_unstable();
                    let baby = VillagerState {
                        age: -24_000,
                        data: crate::villager::VillagerData {
                            kind,
                            ..Default::default()
                        },
                        home: Some(home),
                        home_dimension: dimension,
                        birth_origin: Some(mc_script::ScriptNativeBirthOrigin::new(
                            parents,
                            started_tick,
                            state.tick,
                        )),
                        birth_sequence,
                        ..Default::default()
                    };
                    state.entities.row_mut(child).villager = Some(Box::new(baby));
                    cache.update_claims(id, [None; 3], [Some(home), None, None]);
                    cache.newborn_homes.push((id, home));
                    crate::villager::sync_appearance(state, child);
                    crate::settlements::households::inherit_birth(state, id, [first_id, partner]);
                }
                changes.entities_spawned.push(id);
            }
        } else if state.tick.is_multiple_of(20)
            && willing_with_production(state, first, &cache.food, a)
        {
            let pos = state.entities.pos(first);
            let second = bodies_near(state, pos, 8.0, true).find(|&s| {
                s.0 > first.0
                    && active(state, s)
                    && distance(pos, state.entities.pos(s)) <= 8.0 * 8.0
                    && state.entities.row(s).villager.as_ref().is_some_and(|b| {
                        b.partner.is_none() && willing_with_production(state, s, &cache.food, b)
                    })
                    && crate::settlements::households::can_breed(
                        state,
                        [first_id, state.entities.body(s).id],
                    )
            });
            let Some(second) = second else {
                continue;
            };
            let second_id = state.entities.body(second).id;
            let until =
                state.tick + 275 + ((first_id.0 as u64 ^ second_id.0 as u64 ^ state.tick) % 50);
            let a = state.entities.row_mut(first).villager.as_mut().unwrap();
            a.partner = Some(second_id);
            a.courtship_until = Some(until);
            a.courtship_started_tick = Some(state.tick);
            let b = state.entities.row_mut(second).villager.as_mut().unwrap();
            b.partner = Some(first_id);
            b.courtship_until = Some(until);
            b.courtship_started_tick = Some(state.tick);
        }
    }
}
/// Vanilla `wantsToSpawnGolem`: slept within a day and saw no golem recently.
pub(super) fn wants_golem(v: &VillagerState, tick: u64) -> bool {
    v.age >= 0
        && v.last_slept
            .is_some_and(|last| tick.saturating_sub(last) < 24_000)
        && v.golem_detected_until.is_none_or(|until| tick > until)
}
/// Willing villagers (summoner included) a summon needs: three around a
/// panicking villager, five around one that gossiped at a meeting within the
/// gossip interval.
pub(super) fn golem_quorum(v: &VillagerState, tick: u64) -> Option<usize> {
    if !wants_golem(v, tick) {
        return None;
    }
    match v.activity {
        Activity::Panic => Some(3),
        Activity::Meet if tick.saturating_sub(v.last_gossip) < 1_200 => Some(5),
        _ => None,
    }
}
/// Vanilla's golem sensor box: 16 blocks along each axis.
pub(super) fn golem_near(state: &TickState, cache: &PoiCache, pos: [f64; 3]) -> bool {
    cache.golems.iter().any(|&s| {
        state
            .entities
            .pos(s)
            .iter()
            .zip(pos)
            .all(|(a, b)| (a - b).abs() <= 16.0)
    })
}
pub(super) fn golems(state: &mut TickState, cache: &mut PoiCache, changes: &mut ChangeSet) {
    if !state.tick.is_multiple_of(100) || cache.actors().is_empty() {
        return;
    }
    static GOLEM: std::sync::LazyLock<mc_data::Identifier> = std::sync::LazyLock::new(|| {
        mc_data::Identifier::parse("minecraft:iron_golem").expect("static identifier")
    });
    let Some(type_id) = state.registry.entity_types.id_of(&GOLEM) else {
        return;
    };
    // Brains may move before village defense. Gather from their current
    // chunks now, not from the pre-motion threat/pickup projection.
    cache.refresh_sensor_chunks(state);
    cache.golems.clear();
    for pos in &cache.near_sensor_chunks {
        if let Some(ids) = state.entities.brain_sensor_chunks().get(pos) {
            for &id in ids {
                if let Some(slot) = state.entities.slot_of(id)
                    && state.entities.body(slot).alive
                    && state.entities.body(slot).kind == (EntityKind::Mob { type_id })
                {
                    cache.golems.push(slot);
                }
            }
        }
    }
    if state.tick.is_multiple_of(200) {
        for index in 0..cache.actors().len() {
            let slot = cache.actors()[index];
            if !active(state, slot) || !cache.admitted(slot) {
                continue;
            }
            if golem_near(state, cache, state.entities.pos(slot)) {
                state
                    .entities
                    .row_mut(slot)
                    .villager
                    .as_mut()
                    .unwrap()
                    .golem_detected_until = Some(state.tick + 599);
            }
        }
    }
    let mut spawned = 0;
    let count = cache.actors().len();
    let start = ((state.tick / 100).wrapping_mul(64) % count.max(1) as u64) as usize;
    for offset in 0..count.min(64) {
        if spawned >= 4 {
            break;
        }
        let slot = cache.actors()[(start + offset) % count];
        if !active(state, slot) || !cache.admitted(slot) {
            continue;
        }
        let v = state.entities.row(slot).villager.as_ref().unwrap();
        let Some(quorum) = golem_quorum(v, state.tick) else {
            continue;
        };
        let origin = state.entities.pos(slot);
        let agrees = bodies_near(state, origin, 10.0, true)
            .filter(|&s| {
                active(state, s)
                    && wants_golem(state.entities.row(s).villager.as_ref().unwrap(), state.tick)
                    && state
                        .entities
                        .pos(s)
                        .iter()
                        .zip(origin)
                        .all(|(a, b)| (a - b).abs() <= 10.0)
            })
            .take(quorum)
            .count();
        if agrees < quorum || golem_near(state, cache, origin) {
            continue;
        }
        let mut spawn = None;
        for attempt in 0..10u64 {
            let mut seed = state.tick.rotate_left(17)
                ^ (state.entities.body(slot).id.0 as u64).wrapping_mul(0x9E3779B97F4A7C15)
                ^ attempt.wrapping_mul(0xD1B54A32D192ED03);
            let mut draw = |range: i32| {
                seed ^= seed >> 30;
                seed = seed.wrapping_mul(0xBF58476D1CE4E5B9);
                seed ^= seed >> 27;
                (seed % (range * 2 + 1) as u64) as i32 - range
            };
            let offset = if attempt == 0 {
                [0, 0, 0]
            } else {
                [draw(8), draw(6), draw(8)]
            };
            let p = BlockPos {
                x: origin[0].floor() as i32 + offset[0],
                y: origin[1].floor() as i32 + offset[1],
                z: origin[2].floor() as i32 + offset[2],
            };
            let Some(pos) = standing(&state.world, &state.registry, p, 0.7, 2.7) else {
                continue;
            };
            if bodies_near(state, pos, 3.0, false).any(|s| {
                let body = state.entities.body(s);
                let at = state.entities.pos(s);
                body.alive
                    && (pos[0] - at[0]).abs() < 0.7 + body.half_width
                    && (pos[2] - at[2]).abs() < 0.7 + body.half_width
                    && pos[1] < at[1] + body.height
                    && pos[1] + 2.7 > at[1]
            }) {
                continue;
            }
            spawn = Some(pos);
            break;
        }
        if let Some(pos) = spawn {
            let id = state.spawn_mob(type_id, pos, 100.0);
            if let Some(slot) = state.entities.slot_of(id) {
                cache.golems.push(slot);
            }
            changes.entities_spawned.push(id);
            spawned += 1;
            cache.nearby_residents.clear();
            cache
                .nearby_residents
                .extend(bodies_near(state, origin, 10.0, true));
            for &s in &cache.nearby_residents {
                if active(state, s)
                    && state
                        .entities
                        .pos(s)
                        .iter()
                        .zip(origin)
                        .all(|(a, b)| (a - b).abs() <= 10.0)
                {
                    state
                        .entities
                        .row_mut(s)
                        .villager
                        .as_mut()
                        .unwrap()
                        .golem_detected_until = Some(state.tick + 599);
                }
            }
        }
    }
}
