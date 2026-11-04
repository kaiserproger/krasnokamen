//! Planning follows construction cadence; physical promises refresh each tick.
use super::{IronSmelting, VillagerState, committed, discover_owned_furnaces};
use crate::settlements::{SettlementId, construction};
use crate::tick::TickState;
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct Cache {
    tick: Option<u64>,
    towns: BTreeMap<SettlementId, Slice>,
    #[cfg(test)]
    pub(crate) snapshots: usize,
    #[cfg(test)]
    pub(crate) demand_scans: usize,
}

#[derive(Default)]
pub(super) struct Slice {
    tick: Option<u64>,
    planned_at: Option<u64>,
    demand_tick: Option<u64>,
    pub(super) furnaces: Vec<[i32; 3]>,
    revision: u64,
    building_count: usize,
    container_count: usize,
    pub(super) demand: u32,
    pub(super) committed: u32,
}

pub(super) fn current(state: &TickState, id: SettlementId) -> Option<&Slice> {
    state.smelting_cache.towns.get(&id).filter(|slice| {
        slice.planned_at.is_some_and(|at| {
            state
                .tick
                .checked_sub(at)
                .is_some_and(|age| age < construction::PLAN_CADENCE)
        }) && state.settlements.get(id).is_some_and(|record| {
            record.revision == slice.revision && record.buildings.len() == slice.building_count
        }) && state.block_entities.len() == slice.container_count
    })
}

pub(super) fn prepare(state: &mut TickState, v: &VillagerState, recipe: IronSmelting) {
    let id = v.settlement.expect("bound worker");
    if current(state, id).is_some_and(|slice| slice.tick == Some(state.tick)) {
        return;
    }
    if state.smelting_cache.tick != Some(state.tick) {
        state.smelting_cache.tick = Some(state.tick);
        // Keep reusable vectors for live towns, not an ever-growing history of
        // deleted settlements. Each dimension owns its own TickState/cache.
        state
            .smelting_cache
            .towns
            .retain(|id, _| state.settlements.get(*id).is_some());
    }
    let refresh_plan = current(state, id).is_none();
    if refresh_plan {
        let mut furnaces =
            std::mem::take(&mut state.smelting_cache.towns.entry(id).or_default().furnaces);
        furnaces.clear();
        furnaces.extend(discover_owned_furnaces(state, id));
        let slice = state.smelting_cache.towns.get_mut(&id).expect("town slice");
        slice.furnaces = furnaces;
        slice.planned_at = Some(state.tick);
        slice.demand_tick = None;
        slice.demand = 0;
        slice.revision = state.settlements.get(id).expect("bound town").revision;
        // Structural changes invalidate discovery; stack transfers do not.
        slice.building_count = state
            .settlements
            .get(id)
            .expect("bound town")
            .buildings
            .len();
        slice.container_count = state.block_entities.len();
    }
    let slice = state.smelting_cache.towns.get_mut(&id).expect("town slice");
    slice.tick = Some(state.tick);
    slice.committed = 0;
    #[cfg(test)]
    {
        state.smelting_cache.snapshots += 1;
    }
    if current(state, id).expect("town slice").furnaces.is_empty() {
        return;
    }
    // Publish ownership before planning so availability reuses the same list.
    if refresh_plan {
        refresh_demand(state, v);
    }
    let demand = current(state, id).expect("town slice").demand;
    let committed = if demand == 0 {
        0
    } else {
        committed(state, v, recipe.raw, recipe.iron)
    };
    let slice = state.smelting_cache.towns.get_mut(&id).expect("town slice");
    slice.committed = committed;
}

fn refresh_demand(state: &mut TickState, v: &VillagerState) {
    let id = v.settlement.expect("bound worker");
    if current(state, id).is_some_and(|slice| slice.demand_tick == Some(state.tick)) {
        return;
    }
    let demand = construction::iron_demand(state, id, Some(v));
    let slice = state.smelting_cache.towns.get_mut(&id).expect("town slice");
    slice.demand = demand;
    slice.demand_tick = Some(state.tick);
    #[cfg(test)]
    {
        state.smelting_cache.demand_scans += 1;
    }
}

/// Revalidate the target before a real debit, not during every idle/navigation
/// decision. A new wool/shears supply or consumed construction bill must not
/// cause ore/fuel to be committed against a stale planning-cadence target.
pub(super) fn for_debit(state: &mut TickState, v: &VillagerState) -> (u32, u32) {
    refresh_demand(state, v);
    let slice = current(state, v.settlement.expect("bound worker")).expect("town slice");
    (slice.demand, slice.committed)
}

/// Raw iron in storage is not yet promised to a bill. A successful withdrawal
/// commits it immediately, so a later worker in this same tick cannot overbook.
pub(super) fn taken_raw(state: &mut TickState, id: SettlementId, amount: u32) {
    if let Some(slice) = state.smelting_cache.towns.get_mut(&id)
        && slice.tick == Some(state.tick)
    {
        slice.committed = slice.committed.saturating_add(amount);
    }
}

/// Returning task ore to storage releases its promise, including after a failed
/// route or a missing furnace. Ingot transfers do not change the committed sum.
pub(super) fn returned_raw(state: &mut TickState, id: SettlementId, amount: u32) {
    if let Some(slice) = state.smelting_cache.towns.get_mut(&id)
        && slice.tick == Some(state.tick)
    {
        slice.committed = slice.committed.saturating_sub(amount);
    }
}
