//! Owner-order admission turns and the one site-search budget the whole server
//! shares (owner D42).
//!
//! Bottleneck: a site search runs on the tick thread. Every candidate lot it
//! examines (`site_refusals::advance`) reads the lot's terrain column by
//! column, and every slice of a sweep also pays one street walk and one survey
//! of the village walk. Before D42 an owner command ran its own slice inside
//! its command tick, so N players ordering in one tick cost N slices in that
//! tick, and a planner pass ran one slice for every funded order it held.
//!
//! Boundary: one tick runs at most [`SITE_SLICES`] slices server-wide, every
//! settlement and player together, and a slice examines at most
//! `SITE_ATTEMPTS` lots. The tick's planner pass claims its slices first (its
//! priority house and its funded kinds, as before D42); the order turn takes
//! what is left. A command examines no lot: it checks the slot, the blueprint
//! and the funding, withdraws nothing, queues the order and answers.
//!
//! Fence: the order turn serves one settlement per tick, the next after the
//! one served last in id order; inside it one funded new building searches,
//! the next after the one searched last. So no settlement starves another, and
//! a sweep that never finds a site cannot starve a later request. A settlement
//! stays in the turn while a sweep of its is running and leaves it when it has
//! nothing to search (no funded new building, no free slot, a refused sweep
//! with no other building to search, its chunk not simulated); its planner
//! pass enters it again. The turn is runtime state: after a restart the queued
//! orders wait for their planner pass. Outside the budget: placements that
//! check a fixed handful of candidates (street work, fortification, meadow,
//! upgrades, a founding outpost) and the replanting house-lot proof.
//!
//! Effect: N simultaneous orders share one tick's slices instead of adding a
//! slice each to their command tick; the last of them starts after about
//! N x (slices its sweep needs) order turns. A single order on an idle server
//! gets one slice every tick and starts as soon as its sweep is complete.
use std::ops::Bound;

use super::{
    OrderOutcome, SettlementId, access, blueprints, claim_slot, executor, funded_variant,
    notify_owner, order_level, preparation, site_refusals, start_order,
};
use crate::tick::TickState;

/// Site slices one tick runs server-wide. Derived, not a setting: exactly the
/// two slices one planner pass already ran before D42 (a priority house and
/// the funded kinds), so the planner keeps its pass and no tick examines more
/// lots than that pass did, however many players order at once.
pub(super) const SITE_SLICES: u32 = 2;

/// The site slices the current tick has left, handed out whole.
#[derive(Debug)]
pub(crate) struct SiteBudget {
    slices: u32,
}

impl Default for SiteBudget {
    fn default() -> Self {
        Self {
            slices: SITE_SLICES,
        }
    }
}

impl SiteBudget {
    /// Lots the next slice may examine: a whole slice, or none once the tick's
    /// slices are spent.
    pub(super) fn take(&mut self) -> u32 {
        if self.slices == 0 {
            return 0;
        }
        self.slices -= 1;
        super::SITE_ATTEMPTS
    }
}

/// The tick's site work: the planner pass first, then one order turn with the
/// slices it left.
pub(crate) fn tick(state: &mut TickState) {
    let mut budget = SiteBudget::default();
    super::planner_tick(state, &mut budget);
    turn(state, &mut budget);
}

/// Serve the next settlement whose funded new building is searching.
fn turn(state: &mut TickState, budget: &mut SiteBudget) {
    if budget.slices == 0 {
        return;
    }
    let queue = &state.settlements.site_orders;
    let Some(id) = state
        .settlements
        .site_turn
        .and_then(|last| {
            queue
                .range((Bound::Excluded(last), Bound::Unbounded))
                .next()
        })
        .or_else(|| queue.iter().next())
        .map(|(id, _)| *id)
    else {
        return;
    };
    state.settlements.site_turn = Some(id);
    if !state.settlements.get(id).is_some_and(|record| {
        state.world.is_active(
            record.anchor[0].div_euclid(16),
            record.anchor[2].div_euclid(16),
        )
    }) {
        leave(state, id);
        return;
    }
    plan_order(state, id, budget);
}

/// Returns true whenever a pending order owns this planning turn.
///
/// Owner D8: a request whose materials are already in storage moves ahead of
/// unfunded ones; otherwise the entry order holds. The head's turn is never
/// given to autonomous planning while it waits for materials, so a later
/// autonomous house cannot overtake it. Runs in the settlement's planner pass
/// and in its order turns; `budget` is the tick's site budget.
pub(super) fn plan_order(state: &mut TickState, id: SettlementId, budget: &mut SiteBudget) -> bool {
    if state
        .settlements
        .get(id)
        .is_none_or(|record| access::orders(record).next().is_none())
    {
        leave(state, id);
        return false;
    }
    // An upgrade whose building is gone or has no next level is dropped from
    // wherever it sits in the queue.
    let mut dropped = Vec::new();
    for (index, order) in access::orders(state.settlements.get(id).expect("settlement")).enumerate()
    {
        if order.building.is_some() && order_level(state, id, order).is_none() {
            dropped.push((index, order.blueprint.clone()));
        }
    }
    for (index, blueprint) in dropped.into_iter().rev() {
        let record = state.settlements.get_mut(id).expect("pending settlement");
        access::take_order(record, index);
        record.revision = record.revision.saturating_add(1);
        notify_owner(
            state,
            id,
            format!("Dropped {blueprint} order: building gone or no upgrade available."),
        );
    }
    if access::orders(state.settlements.get(id).expect("settlement"))
        .next()
        .is_none()
    {
        leave(state, id);
        return false;
    }
    if claim_slot(state, id).is_err() {
        leave(state, id);
        return true;
    }
    // Funded candidates in queue order; the head is first when it is funded.
    let images = executor::source_images(state, id);
    let mut candidates = Vec::new();
    {
        let record = state.settlements.get(id).expect("settlement");
        for (index, order) in access::orders(record).enumerate() {
            let Some(level_number) = order_level(state, id, order) else {
                continue;
            };
            let Some(level) = blueprints::get(&state.registry, &order.blueprint)
                .and_then(|blueprint| blueprint.level(level_number))
            else {
                continue;
            };
            if funded_variant(state, id, &level.materials, &images).is_some() {
                candidates.push(index);
            }
        }
    }
    // Nothing funded: the head keeps the turn and the settlement waits.
    if candidates.is_empty() {
        leave(state, id);
        return true;
    }
    // One funded new building searches for its site per turn, the next after
    // the one searched last; the others keep their place. So a funded request
    // that cannot start for lack of a site does not block the next funded one,
    // and an upgrade, which searches no site, is tried in queue order as before.
    let search = next_build(state, id, &candidates);
    if search.is_none() {
        leave(state, id);
    }
    for index in candidates {
        let searched = search.filter(|(build, _)| *build == index);
        let Some(record) = state.settlements.get_mut(id) else {
            return true;
        };
        let new_building = access::orders(record)
            .nth(index)
            .is_some_and(|order| order.building.is_none());
        let attempts = if searched.is_some() { budget.take() } else { 0 };
        if new_building && attempts == 0 {
            continue;
        }
        let Some(order) = access::take_order(record, index) else {
            continue;
        };
        let seq = order.seq;
        match start_order(state, id, &order, attempts) {
            Ok(handle) => {
                // The admitted project's own handle decides where the sequence
                // is recorded; two parallel houses share a blueprint.
                if let Some(record) = state.settlements.get_mut(id) {
                    let mut recorded = false;
                    if let Some(project) =
                        access::projects_mut(record).find(|project| project.structure == handle)
                    {
                        project.order_seq = Some(seq);
                        recorded = true;
                    }
                    if recorded {
                        record.revision = record.revision.saturating_add(1);
                    }
                }
                // A start clears the refusal it was waiting on, so a later one
                // is news again.
                state.settlements.order_refusal.remove(&(id, seq));
                if searched.is_some() {
                    searched_order(state, id, seq, true);
                }
                notify_owner(state, id, OrderOutcome::Started.message(&order.blueprint));
                return true;
            }
            Err(reason) => {
                // The queue keeps its FIFO slot and this slot is retried, so the
                // owner hears a refusal once per distinct reason instead of every
                // planning pass. Runtime only: the reason disappears from the
                // status by itself once the world is fixed.
                let record = state.settlements.get_mut(id).expect("pending settlement");
                access::insert_order(record, index, order);
                if let Some((_, others)) = searched {
                    searched_order(
                        state,
                        id,
                        seq,
                        others || reason == site_refusals::WAITING_FOR_SITE,
                    );
                }
                preparation::note_order_refusal(state, id, seq, reason);
            }
        }
    }
    true
}

/// The funded new building this turn searches for, in queue order the first
/// after the one searched last (else the first), and whether another funded
/// new building waits beside it. `candidates` are queue indices.
fn next_build(state: &TickState, id: SettlementId, candidates: &[usize]) -> Option<(usize, bool)> {
    let record = state.settlements.get(id)?;
    let last = state.settlements.site_orders.get(&id).copied().unwrap_or(0);
    let builds: Vec<(usize, u64)> = candidates
        .iter()
        .filter_map(|&index| {
            let order = access::orders(record).nth(index)?;
            order.building.is_none().then_some((index, order.seq))
        })
        .collect();
    builds
        .iter()
        .find(|(_, seq)| *seq > last)
        .or(builds.first())
        .map(|(index, _)| (*index, builds.len() > 1))
}

/// A funded new building waits for a site: its settlement takes order turns.
pub(super) fn enter(state: &mut TickState, id: SettlementId) {
    state.settlements.site_orders.entry(id).or_insert(0);
}

/// The turn searched the order `seq`. A running sweep keeps the settlement in
/// the turn, and so does another funded new building; otherwise it leaves.
fn searched_order(state: &mut TickState, id: SettlementId, seq: u64, keep: bool) {
    if keep {
        state.settlements.site_orders.insert(id, seq);
    } else {
        leave(state, id);
    }
}

/// Nothing of the settlement searches until its planner pass enters it again.
fn leave(state: &mut TickState, id: SettlementId) {
    state.settlements.site_orders.remove(&id);
}
