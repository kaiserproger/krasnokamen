//! Builder supply for construction (owner D8): which resident may take a
//! project, the binding pass, and the planner's check that one more
//! autonomous project would get a builder at all.
use super::*;
use crate::entities::EntitySlot;

/// Residents tried per pass; the rest wait for the next planning pass.
const BUILDER_CANDIDATES: usize = 8;
/// How long a failed route to storage stands for the planner's gate, which
/// searches no route itself: one minute of game time.
const STORAGE_ROUTE_MEMORY: u64 = 1_200;

/// `reaches_storage`, remembered per resident: a failure stands for
/// `STORAGE_ROUTE_MEMORY` ticks, a success clears it.
pub(super) fn probe_storage(
    state: &mut TickState,
    id: SettlementId,
    slot: EntitySlot,
    cache: &mut crate::settlements::allocator::PoiCache,
    path: &mut Vec<crate::boundary::BlockPos>,
) -> bool {
    let reached = reaches_storage(state, id, slot, cache, path);
    let (uuid, tick) = (state.entities.row(slot).uuid, state.tick);
    let memory = &mut state.settlements.storage_unreached;
    memory.retain(|_, until| *until > tick);
    if reached {
        memory.remove(&uuid);
    } else {
        memory.insert(uuid, tick.saturating_add(STORAGE_ROUTE_MEMORY));
    }
    reached
}

/// The resident's last route to storage failed and still stands.
fn storage_unreached(state: &TickState, slot: EntitySlot) -> bool {
    state
        .settlements
        .storage_unreached
        .get(&state.entities.row(slot).uuid)
        .is_some_and(|&until| state.tick < until)
}

/// An unbound native Builder of this settlement, before the storage probe.
fn free_native_builder(state: &TickState, id: SettlementId, slot: EntitySlot) -> bool {
    let row = state.entities.row(slot);
    state.entities.body(slot).alive
        && !access::bound_builder(&state.settlements, row.uuid)
        && !carry_debtor(state, id, row.uuid)
        && row.villager.as_ref().is_some_and(|v| {
            v.settlement == Some(id)
                && v.data.profession == Profession::Builder
                && v.conversion_time.is_none()
        })
}

/// The gatherer construction may not take a staffed worker from: the joint
/// bill's missing material, or, for autonomous work, the planner's own
/// gatherer demand while that gatherer is staffed.
fn protected_gatherer(state: &TickState, id: SettlementId, ordered: bool) -> Option<Profession> {
    let record = state.settlements.get(id)?;
    let images = executor::source_images(state, id);
    let bill = joint_active_bill(state, id)?;
    crate::settlements::construction_crafting::needed_gatherer(state, &bill, &images).or_else(
        || {
            if ordered {
                return None;
            }
            gatherer_demand(state, record, &images)
                .filter(|profession| has_staffed_gatherer(state, id, *profession))
        },
    )
}

/// The recruitment rule: the priority (lower first) of a resident that
/// construction may hire now, or `None`. Extraction workers are lent only to a
/// single project (`may_borrow`); a staffed `protected` gatherer keeps its job.
fn recruit_priority(
    state: &TickState,
    id: SettlementId,
    slot: EntitySlot,
    may_borrow: bool,
    protected: Option<Profession>,
) -> Option<u32> {
    let uuid = state.entities.row(slot).uuid;
    if !resident_simulated(state, slot)
        || crate::settlements::economy::worker_reserved(state, Some(id), uuid)
        || carry_debtor(state, id, uuid)
    {
        return None;
    }
    let body = state.entities.body(slot);
    let v = state.entities.row(slot).villager.as_ref()?;
    if protected.is_some_and(|needed| {
        v.job_site.is_some_and(|job| {
            buildings::job_profession(&state.settlements, Some(id), job) == Some(needed)
        })
    }) {
        return None;
    }
    if !body.alive
        || v.settlement != Some(id)
        || v.age < 0
        || v.conversion_time.is_some()
        || v.data.profession == Profession::Builder
        || access::bound_builder(&state.settlements, uuid)
        || (!may_borrow
            && matches!(
                v.data.profession,
                Profession::Miner | Profession::Lumberjack
            ))
    {
        return None;
    }
    let traded = crate::settlements::allocator::traded(v);
    match v.data.profession {
        Profession::None => Some(0),
        Profession::Mason if !traded => Some(1),
        profession
            if !traded
                && !matches!(
                    profession,
                    Profession::Farmer
                        | Profession::Miner
                        | Profession::Lumberjack
                        | Profession::Shepherd
                        | Profession::Soldier
                        | Profession::Nitwit
                ) =>
        {
            Some(2)
        }
        Profession::Miner | Profession::Lumberjack
            if !traded && Some(v.data.profession) != protected =>
        {
            Some(3)
        }
        _ => None,
    }
}

/// Whether one more autonomous project would get a builder now: a free native
/// Builder, or a resident the recruitment rule hires at the project count the
/// new project makes, unless its last route to storage failed (the probe
/// itself stays with `assign_builder`). Spain seed 100: a fourth house opened
/// for three builders, none of the other residents (farmers, a nitwit, kept
/// extraction workers) could take it, and it stood unbound for 30k ticks
/// while it held the planner.
pub(super) fn builder_free(state: &TickState, id: SettlementId) -> bool {
    let may_borrow = access::projects_of(&state.settlements, id).next().is_none();
    let protected = protected_gatherer(state, id, false);
    residents(state, id).any(|slot| {
        !storage_unreached(state, slot)
            && (free_native_builder(state, id, slot)
                || recruit_priority(state, id, slot, may_borrow, protected).is_some())
    })
}

/// Bind an already-working native Builder to the first unbound project.
fn bind_first_unbound(state: &mut TickState, id: SettlementId, slot: EntitySlot) {
    let Some(target) = unbound_project(&state.settlements, id) else {
        return;
    };
    let binding = Some((state.entities.row(slot).uuid, None));
    if let Some(record) = state.settlements.get_mut(id)
        && access::bind_builder(record, &target, binding)
    {
        record.revision = record.revision.saturating_add(1);
    }
}

pub(super) fn assign_builder(state: &mut TickState, id: SettlementId) {
    drain_carry_duties(state, id);
    release_stalled_builder(state, id);
    if release_trapped_builder(state, id) {
        return;
    }
    rebind_lost_builders(state, id);
    let Some(target) = unbound_project(&state.settlements, id) else {
        return;
    };
    // Extraction workers are lent to construction one at a time and only while
    // the settlement holds a single project; from two projects on, gathering
    // keeps its workers and parallel sites recruit ordinary residents (D8).
    let may_borrow = access::projects_of(&state.settlements, id).count() == 1;
    // The wake reason belongs to the project being bound, not to whichever
    // project happens to be first (D8 review R3-8).
    let ordered = state
        .settlements
        .get(id)
        .and_then(|record| access::project_for_structure(record, &target))
        .is_some_and(|project| project.ordered);
    let protected = protected_gatherer(state, id, ordered);
    // A native Builder already on the settlement takes the first unbound
    // project directly; no recruitment and no profession change. It must still
    // be able to reach storage, or a walled builder would hold the project.
    let mut ready_cache = crate::settlements::allocator::PoiCache::default();
    let mut ready_path = Vec::new();
    let mut candidates: Vec<(u32, EntitySlot)> = residents(state, id)
        .filter(|&slot| free_native_builder(state, id, slot))
        .map(|slot| (0, slot))
        .collect();
    for (_, slot) in candidates.drain(..) {
        if probe_storage(state, id, slot, &mut ready_cache, &mut ready_path) {
            bind_first_unbound(state, id, slot);
            return;
        }
    }
    for slot in residents(state, id) {
        let Some(priority) = recruit_priority(state, id, slot, may_borrow, protected) else {
            continue;
        };
        let shunned = state
            .settlements
            .builder_watch
            .get(&(id, target.clone()))
            .is_some_and(|watch| {
                watch.shunned.iter().any(|&(uuid, until)| {
                    uuid == state.entities.row(slot).uuid && state.tick < until
                })
            });
        candidates.push((priority + if shunned { 10 } else { 0 }, slot));
    }
    // Stable: residents of equal priority keep their index order.
    candidates.sort_by_key(|&(priority, _)| priority);
    let mut cache = crate::settlements::allocator::PoiCache::default();
    let mut path = Vec::new();
    let mut chosen = None;
    for (_, slot) in candidates.into_iter().take(BUILDER_CANDIDATES) {
        if probe_storage(state, id, slot, &mut cache, &mut path) {
            chosen = Some(slot);
            break;
        }
    }
    let Some(slot) = chosen else {
        return;
    };
    // Ordinary recruits keep the authored builder career. Only extraction
    // workers are loaned to construction and restored on completion.
    let profession = state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .data
        .profession;
    let borrowed = match profession {
        Profession::Miner | Profession::Lumberjack => Some(profession),
        _ => None,
    };
    let binding = Some((state.entities.row(slot).uuid, borrowed));
    if let Some(record) = state.settlements.get_mut(id)
        && access::bind_builder(record, &target, binding)
    {
        record.revision = record.revision.saturating_add(1);
    }
    let v = state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .expect("builder candidate");
    v.data.profession = Profession::Builder;
    v.offers.clear();
    v.job_site = None;
    v.navigation_target = None;
    v.navigation_path.clear();
    v.navigation_chunks.clear();
    v.navigation_repath_at = 0;
    crate::villager::sync_appearance(state, slot);
}
