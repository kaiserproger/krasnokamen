//! The construction module's narrow surface (owner D9.3, D8).
//!
//! Nothing outside `settlements/construction*.rs` may name `record.project` or
//! `record.pending_order`: the code-health ratchet counts those accesses
//! (`crates/xtask/src/code_health/core_ast.rs`, counter
//! `ConstructionFieldAccess`) and its ceiling is zero. Callers use the functions
//! below instead.
//!
//! The shapes already describe the parallel model — `projects`/`orders` are
//! iterators and `head_order` is "the request the planner may start now" — so a
//! caller written against this surface keeps working when the single slot
//! becomes one project per builder and the single request becomes an ordered
//! list (D8). Today both iterators yield at most one item.
use super::{OwnerOrder, Project};
use crate::settlements::buildings::BuildingId;
use crate::settlements::{Registry, SettlementId, SettlementRecord};

/// Every active project, in admission order. Today the settlement holds at most
/// one.
pub(crate) fn projects(record: &SettlementRecord) -> impl Iterator<Item = &Project> {
    record.project.iter()
}

/// [`projects`] for a settlement id.
pub(crate) fn projects_of(
    settlements: &Registry,
    id: SettlementId,
) -> impl Iterator<Item = &Project> {
    settlements.get(id).into_iter().flat_map(projects)
}

/// The project whose canonical structure ledger entry is `structure`.
pub(crate) fn project_for_structure<'a>(
    record: &'a SettlementRecord,
    structure: &str,
) -> Option<&'a Project> {
    projects(record).find(|project| project.structure == structure)
}

/// Every pending owner request, in the order it was submitted.
pub(crate) fn orders(record: &SettlementRecord) -> impl Iterator<Item = &OwnerOrder> {
    record.pending_order.iter()
}

/// The request the planner may start on its next free slot: the queue head.
pub(crate) fn head_order(record: &SettlementRecord) -> Option<&OwnerOrder> {
    orders(record).next()
}

/// [`head_order`] for a settlement id.
pub(crate) fn head_order_of(settlements: &Registry, id: SettlementId) -> Option<&OwnerOrder> {
    settlements.get(id).and_then(head_order)
}

/// True when the settlement has neither active work nor a pending request.
pub(crate) fn idle(record: &SettlementRecord) -> bool {
    projects(record).next().is_none() && orders(record).next().is_none()
}

/// True while a project upgrades exactly this building.
pub(crate) fn upgrading(
    settlements: &Registry,
    settlement: SettlementId,
    building: BuildingId,
) -> bool {
    projects_of(settlements, settlement).any(|project| project.building == Some(building))
}

/// True when this villager is a borrowed builder of any settlement's project.
pub(crate) fn bound_builder(settlements: &Registry, builder: uuid::Uuid) -> bool {
    settlements.records().any(|record| {
        projects(record).any(|project| {
            project
                .borrowed_builder
                .is_some_and(|(borrowed, _)| borrowed == builder)
        })
    })
}

/// Heap held by the construction fields, for `SettlementRecord::heap_estimate`.
pub(crate) fn record_heap_bytes(record: &SettlementRecord) -> usize {
    projects(record)
        .map(|project| project.blueprint.capacity() + project.structure.capacity())
        .sum::<usize>()
        + orders(record)
            .map(|order| order.blueprint.capacity())
            .sum::<usize>()
}
