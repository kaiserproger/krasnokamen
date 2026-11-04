//! Demand-bounded iron smelting through owned, physically tended furnaces.
//! The native furnace tick alone cooks; this worker only moves real stacks.
use super::{Activity, VillagerProfession, VillagerState, mining};
use crate::boundary::BlockPos;
use crate::entities::EntitySlot;
use crate::inventory::furnace::{FUEL, INPUT, OUTPUT};
use crate::settlements::{SettlementId, construction, inventory};
use crate::tick::TickState;
use mc_data::ItemStack;
use mc_data::recipes::CookingKind;
use serde::{Deserialize, Serialize};

#[path = "smelting_cache.rs"]
mod cache;
pub(crate) use cache::Cache;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Task {
    pub furnace: [i32; 3],
    pub phase: Phase,
    /// Quantities actually fetched for this task, tied to their carry slots.
    /// This is a receipt, not another physical inventory.
    #[serde(default)]
    pub carried: [Taken; 8],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Taken {
    item: u32,
    count: u32,
}

impl Task {
    fn new(furnace: [i32; 3], phase: Phase) -> Self {
        Self {
            furnace,
            phase,
            carried: [Taken { item: 0, count: 0 }; 8],
        }
    }

    fn record_taken(&mut self, before: [u32; 8], inventory: &[Option<ItemStack>; 8]) {
        for (index, stack) in inventory.iter().enumerate() {
            let Some(stack) = stack else { continue };
            let added = (stack.count.max(0) as u32).saturating_sub(before[index]);
            if added == 0 {
                continue;
            }
            let receipt = &mut self.carried[index];
            if receipt.item != stack.item_id {
                *receipt = Taken {
                    item: stack.item_id,
                    count: 0,
                };
            }
            receipt.count = receipt.count.saturating_add(added);
        }
    }
}

fn carry_counts(v: &VillagerState) -> [u32; 8] {
    std::array::from_fn(|index| {
        v.inventory[index]
            .as_ref()
            .map_or(0, |stack| stack.count.max(0) as u32)
    })
}

fn owned_amount(v: &VillagerState, index: usize, protected: u32) -> u32 {
    let Some(task) = v.smelting.as_ref() else {
        return 0;
    };
    let Some(stack) = v.inventory[index].as_ref() else {
        return 0;
    };
    let receipt = &task.carried[index];
    if receipt.item != stack.item_id {
        return 0;
    }
    receipt
        .count
        .min((stack.count.max(0) as u32).saturating_sub(protected))
}

fn owned_raw(v: &VillagerState, raw: u32) -> u32 {
    v.inventory
        .iter()
        .enumerate()
        .filter(|(_, stack)| stack.as_ref().is_some_and(|stack| stack.item_id == raw))
        .map(|(index, _)| owned_amount(v, index, 0))
        .sum()
}

fn debit_limit(state: &mut TickState, v: &VillagerState, raw: u32) -> Option<u32> {
    let (demand, committed) = cache::for_debit(state, v);
    (demand > 0).then(|| demand.saturating_sub(committed.saturating_sub(owned_raw(v, raw))))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Phase {
    Load,
    Deliver,
}

/// Immutable native recipe facts, resolved when the registry is admitted.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IronSmelting {
    pub raw: u32,
    pub iron: u32,
    pub cook_ticks: u32,
}

fn pos([x, y, z]: [i32; 3]) -> BlockPos {
    BlockPos { x, y, z }
}

fn furnace_point(point: &mc_script::ScriptWorksiteInteractionPoint) -> Option<[i32; 3]> {
    match point {
        mc_script::ScriptWorksiteInteractionPoint::Workstation { at, capability }
            if capability == "minecraft:furnace" =>
        {
            Some(*at)
        }
        _ => None,
    }
}

/// Loaded furnaces in certified buildings or town-registered workstations.
/// Natural mining/lumber zones never confer ownership of somebody's furnace.
pub(crate) fn owned_furnaces(
    state: &TickState,
    id: SettlementId,
) -> impl Iterator<Item = [i32; 3]> + '_ {
    let cached = cache::current(state, id);
    let uncached = cached.is_none().then(|| discover_owned_furnaces(state, id));
    cached
        .into_iter()
        .flat_map(|slice| slice.furnaces.iter().copied())
        .filter(move |&[x, y, z]| {
            state
                .block_entities
                .get(&(x, y, z))
                .is_some_and(|entity| entity.furnace.is_some() && entity.slots.len() == 3)
                && state
                    .world
                    .block_at(x, y, z)
                    .and_then(|block| state.registry.blocks.by_id(mc_world::BlockStateId(block)))
                    .is_some_and(|block| block.block.id.as_str() == "minecraft:furnace")
        })
        .chain(uncached.into_iter().flatten())
}

fn discover_owned_furnaces(
    state: &TickState,
    id: SettlementId,
) -> impl Iterator<Item = [i32; 3]> + '_ {
    let record = state.settlements.get(id);
    // Reuse the native certificate ledger, including its owner and committed-
    // structure checks; a standalone plugin registration grants no town access.
    let registered = record
        .and_then(|record| record.district.as_ref())
        .and_then(|district| {
            crate::plugin_structures::building_facts(
                state,
                &district.owner,
                district
                    .buildings
                    .iter()
                    .map(|building| (building.id.as_str(), building.kind == "worksite")),
            )
            .ok()
        })
        .unwrap_or_default();
    let workstation = record
        .and_then(|record| record.furnace_workstation.as_deref())
        .and_then(|handle| {
            crate::plugin_structures::settlement_builds::furnace_workstation(state, id, handle)
        });
    let (min_x, max_x) = record
        .into_iter()
        .flat_map(|record| record.buildings.values())
        .filter(|building| !building.natural_zone)
        .map(|building| (building.bounds.min[0], building.bounds.max[0]))
        .chain(
            registered
                .iter()
                .filter(|facts| facts.dimension == state.dimension.name())
                .flat_map(|facts| facts.points.iter().filter_map(furnace_point))
                .map(|at| (at[0], at[0])),
        )
        .chain(workstation.map(|at| (at[0], at[0])))
        .fold(None, |range, (left, right)| match range {
            None => Some((left, right)),
            Some((min, max)) => Some((min.min(left), max.max(right))),
        })
        .unwrap_or((0, 0));
    // Native custody is ordered by (x,y,z): range over this settlement, not
    // every loaded world's containers, and never allocate a discovery list.
    state
        .block_entities
        .range((min_x, i32::MIN, i32::MIN)..=(max_x, i32::MAX, i32::MAX))
        .filter_map(move |(&(x, y, z), entity)| {
            let record = record?;
            let at = [x, y, z];
            (entity.furnace.is_some()
                && entity.slots.len() == 3
                && (record.buildings.values().any(|building| {
                    !building.natural_zone
                        && at.into_iter().enumerate().all(|(axis, n)| {
                            n >= building.bounds.min[axis] && n <= building.bounds.max[axis]
                        })
                }) || workstation == Some(at)
                    || registered.iter().any(|facts| {
                        facts.dimension == state.dimension.name()
                            && facts
                                .points
                                .iter()
                                .filter_map(furnace_point)
                                .any(|point| point == at)
                    }))
                && state
                    .world
                    .block_at(x, y, z)
                    .and_then(|block| state.registry.blocks.by_id(mc_world::BlockStateId(block)))
                    .is_some_and(|block| block.block.id.as_str() == "minecraft:furnace"))
            .then_some(at)
        })
}

fn endpoint(state: &TickState, position: [i32; 3]) -> inventory::Endpoint {
    inventory::Endpoint::Container {
        dimension: state.dimension.name().into(),
        position,
    }
}

fn count(v: &VillagerState, item: u32) -> u32 {
    v.inventory
        .iter()
        .flatten()
        .filter(|s| s.item_id == item)
        .map(|s| s.count.max(0) as u32)
        .sum()
}

fn fuel_ticks(state: &TickState, stack: &ItemStack) -> u32 {
    if stack.is_empty() {
        return 0;
    }
    mc_data::recipes::furnace_fuel_ticks(&state.registry.tags, CookingKind::Furnace, stack.item_id)
        .map_or(0, |ticks| ticks.max(0) as u32)
}

/// All iron already promised to the bill, including this detached brain's carry.
fn committed(state: &TickState, v: &VillagerState, raw: u32, iron: u32) -> u32 {
    let id = v.settlement.expect("bound worker");
    let mut total = count(v, iron) + owned_raw(v, raw);
    for (_, image) in inventory::town_storage(state, id) {
        if let Ok(free) = inventory::unreserved_stock(state, &image) {
            total = total.saturating_add(
                free.get(&iron)
                    .copied()
                    .unwrap_or(0)
                    .min(u64::from(u32::MAX)) as u32,
            );
        }
    }
    for slot in construction::residents(state, id) {
        if state.entities.body(slot).alive
            && let Some(other) = &state.entities.row(slot).villager
            && other.settlement == Some(id)
            && !crate::settlements::economy::worker_reserved(
                state,
                Some(id),
                state.entities.row(slot).uuid,
            )
        {
            total += count(other, iron) + owned_raw(other, raw);
        }
    }
    for at in owned_furnaces(state, id) {
        total += state.block_entities[&(at[0], at[1], at[2])]
            .slots
            .iter()
            .filter(|s| s.item_id == raw || s.item_id == iron)
            .map(|s| s.count.max(0) as u32)
            .sum::<u32>();
    }
    total
}

/// Use the shared reservation-floor authority before selecting a source.
fn source(
    state: &TickState,
    image: &inventory::Image,
    accepts: impl Fn(usize, &ItemStack) -> bool,
) -> Option<(usize, u32)> {
    let free = inventory::unreserved_stock(state, image).ok()?;
    image.slots.iter().enumerate().find_map(|(index, stack)| {
        let amount = free
            .get(&stack.item_id)
            .copied()
            .unwrap_or(0)
            .min(u64::from(u32::MAX)) as u32;
        (stack.count > 0 && amount > 0 && accepts(index, stack)).then_some((index, amount))
    })
}

fn failed(v: &VillagerState, at: [i32; 3]) -> bool {
    v.navigation_search_failed && v.navigation_target == Some(pos(at))
}

/// Keep an approach stable, but do not monopolize a known failed source.
fn storage(
    state: &TickState,
    slot: EntitySlot,
    v: &VillagerState,
) -> Vec<([i32; 3], inventory::Image)> {
    let mut stock = inventory::town_storage(state, v.settlement.expect("bound worker"));
    stock.retain(|(at, _)| !failed(v, *at));
    stock.sort_by_key(|(at, _)| {
        (
            !mining::near(state, slot, pos(*at), 2.5),
            v.navigation_target != Some(pos(*at)),
        )
    });
    stock
}

/// A successful fenced source mutation and detached carry update share one tick.
fn take(
    state: &mut TickState,
    slot: EntitySlot,
    v: &mut VillagerState,
    before: inventory::Image,
    index: usize,
    maximum: u32,
) -> Option<BlockPos> {
    let inventory::Endpoint::Container { position, .. } = before.endpoint else {
        unreachable!("container source")
    };
    let source = &before.slots[index];
    let amount = (source.count.max(0) as u32)
        .min(maximum)
        .min(mining::room(state, v, source).max(0) as u32);
    if amount == 0 {
        return None;
    }
    if !mining::near(state, slot, pos(position), 2.5) {
        return Some(pos(position));
    }
    let mut moving = source.clone();
    moving.count = amount as i32;
    let mut after = before.slots.clone();
    after[index].count -= amount as i32;
    if after[index].count == 0 {
        after[index] = ItemStack::EMPTY;
    }
    let maximum = state.registry.item_rules().max_stack(&moving);
    if inventory::commit(state, vec![inventory::Change { before, after }]).is_ok() {
        super::brain::add_inventory(v, &mut moving, maximum);
    }
    None
}

fn deliver(state: &mut TickState, slot: EntitySlot, v: &mut VillagerState) -> Option<BlockPos> {
    let shears = mining::item_id(state, "shears");
    let raw = state.registry.iron_smelting.map(|recipe| recipe.raw);
    let promised = raw.map_or(0, |raw| owned_raw(v, raw));
    let mut protected = crate::settlements::economy::protected_carry(state, slot, v);
    for (index, stack) in v.inventory.iter().enumerate() {
        if let Some(stack) = stack
            && Some(stack.item_id) == shears
        {
            protected[index] = stack.count.max(0) as u32;
        }
    }
    let target = mining::deposit_with_protected(state, slot, v, protected);
    if let (Some(id), Some(raw)) = (v.settlement, raw) {
        cache::returned_raw(state, id, promised.saturating_sub(owned_raw(v, raw)));
    }
    if !v
        .inventory
        .iter()
        .zip(protected)
        .any(|(stack, held)| stack.as_ref().is_some_and(|s| s.count > held as i32))
    {
        v.smelting = None;
    }
    target
}

fn insert(
    state: &mut TickState,
    slot: EntitySlot,
    v: &mut VillagerState,
    at: [i32; 3],
    raw: u32,
    mut remaining: u32,
) -> Option<BlockPos> {
    if !mining::near(state, slot, pos(at), 2.5) {
        return Some(pos(at));
    }
    let Ok(before) = inventory::load(state, &endpoint(state, at)) else {
        return None;
    };
    let mut after = before.slots.clone();
    let mut carry = v.inventory.clone();
    let protected = crate::settlements::economy::protected_carry(state, slot, v);
    let mut task = v.smelting.expect("active load");
    let rules = state.registry.item_rules();
    let mut moved = false;
    for (carry_index, stack) in carry.iter_mut().enumerate() {
        let Some(moving) = stack else {
            continue;
        };
        let available = owned_amount(v, carry_index, protected[carry_index]);
        if available == 0 {
            continue;
        }
        let index = if moving.item_id == raw {
            INPUT
        } else if fuel_ticks(state, moving) > 0 {
            FUEL
        } else {
            continue;
        };
        let stored = &mut after[index];
        if !stored.is_empty() && !mc_data::inventory_semantics_26_1_2::can_stack(stored, moving) {
            continue;
        }
        let amount = moving
            .count
            .min(available.min(i32::MAX as u32) as i32)
            .min(rules.max_stack(moving).min(99) - if stored.is_empty() { 0 } else { stored.count })
            .min(if index == INPUT {
                remaining.min(i32::MAX as u32) as i32
            } else {
                i32::MAX
            });
        if amount <= 0 {
            continue;
        }
        if stored.is_empty() {
            *stored = moving.clone();
            stored.count = 0;
        }
        stored.count += amount;
        moving.count -= amount;
        task.carried[carry_index].count -= amount as u32;
        moved = true;
        if index == INPUT {
            remaining -= amount as u32;
        }
        if moving.count == 0 {
            *stack = None;
        }
    }
    if moved && inventory::commit(state, vec![inventory::Change { before, after }]).is_ok() {
        v.inventory = carry;
        task.phase = Phase::Deliver;
        v.smelting = Some(task);
    }
    None
}

fn load(
    state: &mut TickState,
    slot: EntitySlot,
    v: &mut VillagerState,
    at: [i32; 3],
    raw: u32,
    cook: u32,
    mut maximum: u32,
) -> Option<BlockPos> {
    let task = v.smelting.expect("active load");
    // A pre-receipt persisted task may own no carried material. Return legacy
    // raw/fuel carry instead of adopting unrelated stacks as furnace supplies.
    if task.carried.iter().all(|receipt| receipt.count == 0)
        && v.inventory
            .iter()
            .flatten()
            .any(|stack| stack.item_id == raw || fuel_ticks(state, stack) > 0)
    {
        v.smelting.as_mut().expect("active load").phase = Phase::Deliver;
        return deliver(state, slot, v);
    }
    if mining::near(state, slot, pos(at), 2.5) {
        let Some(limit) = debit_limit(state, v, raw) else {
            v.smelting.as_mut().expect("active load").phase = Phase::Deliver;
            return deliver(state, slot, v);
        };
        if limit == 0 && owned_raw(v, raw) > 0 {
            v.smelting.as_mut().expect("active load").phase = Phase::Deliver;
            return deliver(state, slot, v);
        }
        maximum = limit;
    }
    let furnace = &state.block_entities[&(at[0], at[1], at[2])];
    let input = &furnace.slots[INPUT];
    if !input.is_empty() && input.item_id != raw {
        v.smelting.as_mut().expect("active load").phase = Phase::Deliver;
        return deliver(state, slot, v);
    }
    let protected = crate::settlements::economy::protected_carry(state, slot, v);
    let carried_raw: u32 = v
        .inventory
        .iter()
        .enumerate()
        .filter(|(_, stack)| stack.as_ref().is_some_and(|stack| stack.item_id == raw))
        .map(|(index, _)| owned_amount(v, index, protected[index]))
        .sum();
    let progress = furnace.furnace.as_ref().expect("owned furnace");
    let ore = carried_raw.min(maximum)
        + if input.item_id == raw {
            input.count.max(0) as u32
        } else {
            0
        };
    let needed = ore
        .saturating_mul(cook)
        .saturating_sub(progress.cook_progress.max(0) as u32);
    let supplied = progress.lit_remaining.max(0) as u32
        + fuel_ticks(state, &furnace.slots[FUEL])
            .saturating_mul(furnace.slots[FUEL].count.max(0) as u32)
        + v.inventory
            .iter()
            .enumerate()
            .filter_map(|(index, stack)| {
                let stack = stack.as_ref()?;
                Some(fuel_ticks(state, stack).saturating_mul(owned_amount(
                    v,
                    index,
                    protected[index],
                )))
            })
            .sum::<u32>();
    if supplied < needed {
        for (source_at, before) in storage(state, slot, v) {
            if let Some((index, free)) = source(state, &before, |_, s| fuel_ticks(state, s) > 0) {
                let stack = &before.slots[index];
                if mining::room(state, v, stack) == 0 {
                    v.smelting.as_mut().expect("active load").phase = Phase::Deliver;
                    return deliver(state, slot, v);
                }
                if mining::near(state, slot, pos(source_at), 2.5) {
                    let Some(limit) = debit_limit(state, v, raw) else {
                        v.smelting.as_mut().expect("active load").phase = Phase::Deliver;
                        return deliver(state, slot, v);
                    };
                    if limit == 0 && owned_raw(v, raw) > 0 {
                        v.smelting.as_mut().expect("active load").phase = Phase::Deliver;
                        return deliver(state, slot, v);
                    }
                    if limit != maximum {
                        // Demand is refreshed at most once this tick: recomputing
                        // the fuel budget cannot recurse a second time.
                        return load(state, slot, v, at, raw, cook, limit);
                    }
                }
                let amount = (needed - supplied)
                    .div_ceil(fuel_ticks(state, stack))
                    .min(free);
                let prior = carry_counts(v);
                let target = take(state, slot, v, before, index, amount);
                if prior != carry_counts(v) {
                    v.smelting
                        .as_mut()
                        .expect("active load")
                        .record_taken(prior, &v.inventory);
                    return target;
                }
                if target.is_some() {
                    return target;
                }
                v.smelting.as_mut().expect("active load").phase = Phase::Deliver;
                return deliver(state, slot, v);
            }
        }
    }
    let target = insert(state, slot, v, at, raw, maximum);
    if target.is_none() && v.smelting.is_some_and(|task| task.phase == Phase::Load) {
        // No insertion and no route means this load cannot make progress.
        // Release the durable task rather than monopolizing every work turn.
        v.smelting.as_mut().expect("active load").phase = Phase::Deliver;
        return deliver(state, slot, v);
    }
    target
}

/// `Some` owns this work turn, even when the transfer completes without walking.
/// This prevents the shepherd consuming freshly extracted ingots before deposit.
pub(super) fn work(
    state: &mut TickState,
    slot: EntitySlot,
    v: &mut VillagerState,
) -> Option<Option<BlockPos>> {
    if v.age < 0
        || v.sleeping
        || v.conversion_time.is_some()
        || v.activity == Activity::Panic
        || crate::settlements::economy::worker_reserved(
            state,
            v.settlement,
            state.entities.row(slot).uuid,
        )
    {
        return None;
    }
    if v.smelting.is_none()
        && !matches!(
            v.data.profession,
            VillagerProfession::None | VillagerProfession::Shepherd
        )
    {
        return None;
    }
    let id = v.settlement?;
    if v.smelting.is_some_and(|task| task.phase == Phase::Deliver) {
        return Some(deliver(state, slot, v));
    }
    if v.activity != Activity::Work
        || !(2_000..9_000).contains(&state.day_time.rem_euclid(24_000))
        || !crate::command_rules::boolean(state, crate::command_rules::BoolRule::MOB_GRIEFING)
    {
        return None;
    }
    let recipe = state.registry.iron_smelting?;
    state.settlements.get(id)?;
    cache::prepare(state, v, recipe);
    if owned_furnaces(state, id).next().is_none() {
        if let Some(task) = v.smelting {
            v.smelting = Some(Task {
                phase: Phase::Deliver,
                ..task
            });
            return Some(deliver(state, slot, v));
        }
        return None;
    }
    let raw = recipe.raw;
    let cook = recipe.cook_ticks;
    let slice = cache::current(state, id).expect("prepared town slice");
    let (demand, committed) = (slice.demand, slice.committed);
    if let Some(mut task) = v.smelting {
        if demand == 0 || !owned_furnaces(state, id).any(|at| at == task.furnace) {
            v.smelting = Some(Task {
                phase: Phase::Deliver,
                ..task
            });
            return Some(deliver(state, slot, v));
        }
        let maximum = demand.saturating_sub(committed.saturating_sub(owned_raw(v, raw)));
        if maximum == 0 && owned_raw(v, raw) > 0 {
            v.smelting = Some(Task {
                phase: Phase::Deliver,
                ..task
            });
            return Some(deliver(state, slot, v));
        }
        if failed(v, task.furnace) {
            let alternate = owned_furnaces(state, id).find(|at| {
                *at != task.furnace
                    && state.block_entities[&(at[0], at[1], at[2])].slots[INPUT].is_empty()
            });
            if let Some(at) = alternate {
                task.furnace = at;
                v.smelting = Some(task);
            } else {
                v.smelting = Some(Task {
                    phase: Phase::Deliver,
                    ..task
                });
                return Some(deliver(state, slot, v));
            }
        }
        return Some(load(state, slot, v, task.furnace, raw, cook, maximum));
    }
    let output = owned_furnaces(state, id)
        .filter(|at| !failed(v, *at))
        .filter_map(|at| {
            let entity = &state.block_entities[&(at[0], at[1], at[2])];
            if entity.slots[OUTPUT].is_empty()
                && (entity.slots[FUEL].is_empty() || fuel_ticks(state, &entity.slots[FUEL]) > 0)
            {
                return None;
            }
            let before = inventory::load(state, &endpoint(state, at)).ok()?;
            let (index, free) = source(state, &before, |index, s| {
                index == OUTPUT || (index == FUEL && fuel_ticks(state, s) == 0)
            })?;
            (mining::room(state, v, &before.slots[index]) > 0).then_some((at, before, index, free))
        })
        .min_by_key(|(at, _, _, _)| {
            (
                !mining::near(state, slot, pos(*at), 2.5),
                v.navigation_target != Some(pos(*at)),
            )
        });
    if let Some((at, before, index, free)) = output {
        let prior: i32 = v.inventory.iter().flatten().map(|s| s.count).sum();
        let target = take(state, slot, v, before, index, free);
        if v.inventory.iter().flatten().map(|s| s.count).sum::<i32>() > prior {
            v.smelting = Some(Task::new(at, Phase::Deliver));
        }
        return Some(target);
    }
    if demand == 0 {
        return None;
    }
    // Finish fuel for an already committed batch before loading any new ore.
    let unfueled = owned_furnaces(state, id)
        .filter(|at| !failed(v, *at))
        .find(|at| {
            let entity = &state.block_entities[&(at[0], at[1], at[2])];
            entity.slots[INPUT].item_id == raw
                && entity.slots[INPUT].count > 0
                && entity.slots[FUEL].is_empty()
                && !entity.furnace.as_ref().expect("furnace").is_lit()
        });
    if let Some(at) = unfueled {
        if v.inventory
            .iter()
            .flatten()
            .any(|stack| stack.item_id == raw || fuel_ticks(state, stack) > 0)
        {
            v.smelting = Some(Task::new(at, Phase::Deliver));
            return Some(deliver(state, slot, v));
        }
        if !storage(state, slot, v).iter().any(|(_, image)| {
            source(state, image, |_, stack| fuel_ticks(state, stack) > 0).is_some()
        }) {
            return None;
        }
        v.smelting = Some(Task::new(at, Phase::Load));
        return Some(load(state, slot, v, at, raw, cook, 0));
    }
    let missing = demand.saturating_sub(committed);
    if missing == 0 {
        return None;
    }
    if v.inventory
        .iter()
        .flatten()
        .any(|s| s.item_id == raw || fuel_ticks(state, s) > 0)
    {
        v.smelting = Some(Task::new(owned_furnaces(state, id).next()?, Phase::Deliver));
        return Some(deliver(state, slot, v));
    }
    let at = owned_furnaces(state, id)
        .filter(|at| !failed(v, *at))
        .find(|at| {
            let entity = &state.block_entities[&(at[0], at[1], at[2])];
            entity.slots[INPUT].is_empty()
                && (entity.slots[FUEL].is_empty() || fuel_ticks(state, &entity.slots[FUEL]) > 0)
        })?;
    let stock = storage(state, slot, v);
    if !stock
        .iter()
        .any(|(_, image)| source(state, image, |_, s| fuel_ticks(state, s) > 0).is_some())
        && !state.block_entities[&(at[0], at[1], at[2])]
            .furnace
            .as_ref()
            .expect("furnace")
            .is_lit()
        && state.block_entities[&(at[0], at[1], at[2])].slots[FUEL].is_empty()
    {
        return None;
    }
    for (source_at, before) in stock {
        if let Some((index, free)) = source(state, &before, |_, s| s.item_id == raw) {
            let prior = count(v, raw);
            let prior_slots = carry_counts(v);
            let mut amount = missing.min(free);
            if mining::near(state, slot, pos(source_at), 2.5) {
                let (demand, committed) = cache::for_debit(state, v);
                amount = amount.min(demand.saturating_sub(committed));
                if amount == 0 {
                    return None;
                }
            }
            let target = take(state, slot, v, before, index, amount);
            if count(v, raw) > prior {
                let mut task = Task::new(at, Phase::Load);
                task.record_taken(prior_slots, &v.inventory);
                v.smelting = Some(task);
                cache::taken_raw(state, id, count(v, raw) - prior);
            }
            return Some(target);
        }
    }
    None
}
