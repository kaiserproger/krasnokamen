//! Owner-order admission turns and the one site-search budget the whole server
//! shares (owner D42).
//!
//! Bottleneck: a site search runs on the tick thread. Every candidate lot it
//! examines (`site_refusals::advance`, a replanting proof's lot checks) runs
//! the full site check on the lot's terrain, and every slice of a sweep also
//! pays one street walk and one survey of the village walk. Before D42 an
//! owner command ran its own slice (or, for an upgrade, its whole admission)
//! inside its command tick, so N players ordering in one tick cost N slices
//! in that tick; a planner pass ran one slice for every funded order it held;
//! and each lumberjack's replanting proof scanned every frontage of the
//! village, up to twice per house blueprint.
//!
//! Boundary: one tick examines at most [`SITE_SLICES`] x `SITE_ATTEMPTS` lots
//! server-wide, every settlement, player and lumberjack together. The tick's
//! planner pass claims its slices first (its priority house and its funded
//! kinds, as before D42); the order turn takes a slice of what is left, and an
//! upgrade's admission, which searches no site, takes a slice too; the
//! villager brain's replanting proofs, later in the tick, take a slice each
//! of the rest. A proof checks the village's last free lot, then the lots its
//! crown would take; only a crown that takes a free lot scans its crowned
//! walk for another, and that scan is continued by its next proof. A proof
//! without a slice, or while that scan runs, does not plant yet and the
//! lumberjack asks again at its next brain run. A command examines no lot
//! and admits nothing: it checks the slot, the blueprint and the funding,
//! withdraws nothing, queues the order and answers.
//!
//! Fence: the order turn serves one settlement per tick, the next after the
//! one served last in id order; inside it one funded new building searches,
//! the next after the one searched last, and at most one order starts. So no
//! settlement starves another, and a sweep that never finds a site cannot
//! starve a later request. A settlement stays in the turn while a sweep or an
//! upgrade of its waits for the budget and leaves it when it has nothing to
//! admit (no funded order, no free slot, a refused order with no other to try,
//! its chunk not simulated); its planner pass enters it again. The turn is
//! runtime state: after a restart the queued orders wait for their planner
//! pass. Outside the budget: placements that check a fixed handful of
//! candidates (street work, fortification, meadow, a founding outpost).
//!
//! Effect: N simultaneous orders share one tick's slices instead of adding a
//! slice each to their command tick; the last of them starts after about
//! N x (slices its sweep needs) order turns. A single order on an idle server
//! gets one slice every tick and starts as soon as its sweep is complete.
use std::collections::BTreeMap;
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

/// Ticks a waiting crown's scan of its crowned walk outlives its last proof:
/// longer than the longest villager brain period (32 ticks,
/// `villager::brain::brain_period`), so a working lumberjack far from every
/// player, who asks again at its next brain run, keeps its scan; an older
/// scan belongs to a planting nobody waits for any more.
const CROWN_PATIENCE: u64 = 64;

/// The candidate lots the current tick may still examine: [`SITE_SLICES`]
/// slices of `SITE_ATTEMPTS` lots. Each tick's site work starts it afresh and
/// leaves the rest to the villager brain's replanting proofs later in the
/// same tick (`state.settlements.site_budget`). Runtime only.
#[derive(Debug)]
pub(crate) struct SiteBudget {
    lots: u32,
    /// Per settlement and house blueprint, the last free lot a replanting
    /// proof found; the next proof checks it first (`planting_verdict`).
    hints: BTreeMap<(SettlementId, &'static str), Option<super::Front>>,
    /// Per settlement, house blueprint and waiting crown (its minimum
    /// corner), that crown's scan of its crowned walk (`crowned_house_lot`).
    crowns: BTreeMap<(SettlementId, &'static str, [i32; 3]), super::CrownScan>,
}

impl Default for SiteBudget {
    fn default() -> Self {
        Self {
            lots: SITE_SLICES * super::SITE_ATTEMPTS,
            hints: BTreeMap::new(),
            crowns: BTreeMap::new(),
        }
    }
}

impl SiteBudget {
    /// Lots the next slice may examine: a whole slice of what is left, none
    /// once the tick's lots are spent.
    pub(crate) fn take(&mut self) -> u32 {
        let lots = self.lots.min(super::SITE_ATTEMPTS);
        self.lots -= lots;
        lots
    }

    /// A new tick's lots; hints and waiting crowns' scans carry over.
    pub(super) fn refill(&mut self) {
        self.lots = SITE_SLICES * super::SITE_ATTEMPTS;
    }

    /// Settlement `id`'s free-lot hint for house blueprint `blueprint`, and
    /// the scan of the crowned walk of the crown whose minimum corner is
    /// `crown`, used at `tick`.
    pub(crate) fn house(
        &mut self,
        id: SettlementId,
        blueprint: &'static str,
        crown: [i32; 3],
        tick: u64,
    ) -> (&mut Option<super::Front>, &mut super::CrownScan) {
        let progress = self.crowns.entry((id, blueprint, crown)).or_default();
        progress.touched = tick;
        (self.hints.entry((id, blueprint)).or_default(), progress)
    }
}

/// The tick's site work: the planner pass first, then one order turn with the
/// lots it left; the rest is the replanting proofs' (`replant_verdict`).
pub(crate) fn tick(state: &mut TickState) {
    let mut budget = std::mem::take(&mut state.settlements.site_budget);
    budget.refill();
    budget
        .hints
        .retain(|(id, _), _| state.settlements.get(*id).is_some());
    budget
        .crowns
        .retain(|_, progress| state.tick.saturating_sub(progress.touched) <= CROWN_PATIENCE);
    super::planner_tick(state, &mut budget);
    turn(state, &mut budget);
    state.settlements.site_budget = budget;
}

/// A lumberjack's verdict on planting a sapling at `root` now: each village
/// proof that the crown leaves building access and a house lot takes a slice
/// of the tick's remaining site budget (its walks and at most the slice's lot
/// checks, `layout::planting_verdict`). A proof without a slice, or while its
/// crown's scan for another free lot runs, is "not yet": it never plants onto
/// the last house lot unproven.
pub(crate) fn replant_verdict(state: &mut TickState, root: [i32; 3]) -> super::Replant {
    let mut budget = std::mem::take(&mut state.settlements.site_budget);
    let verdict = super::replant_check(state, root, &mut budget);
    state.settlements.site_budget = budget;
    verdict
}

/// Serve the next settlement whose funded new building is searching.
fn turn(state: &mut TickState, budget: &mut SiteBudget) {
    if budget.lots == 0 {
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
        let new_building = state
            .settlements
            .get(id)
            .and_then(|record| access::orders(record).nth(index))
            .is_some_and(|order| order.building.is_none());
        if new_building && searched.is_none() {
            continue;
        }
        // The searched building's site slice; an upgrade searches no site, but
        // its admission work takes a slice too.
        let attempts = budget.take();
        if attempts == 0 {
            // The tick's slices are spent: a later turn tries it.
            enter(state, id);
            continue;
        }
        let Some(order) = state
            .settlements
            .get_mut(id)
            .and_then(|record| access::take_order(record, index))
        else {
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
