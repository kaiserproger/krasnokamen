//! The construction module's narrow surface (owner D9.3, D8).
//!
//! Nothing outside `settlements/construction*.rs` may name `projects` or
//! `orders` on the record: the code-health ratchet counts those accesses
//! (`crates/xtask/src/code_health/core_ast.rs`, counter
//! `ConstructionFieldAccess`) and its ceiling is zero. Callers use the
//! functions below instead, which is also how the store changed shape from one
//! project and one order to vectors without touching them.
use super::{OwnerOrder, Project};
use crate::settlements::buildings::BuildingId;
use crate::settlements::{Registry, SettlementId, SettlementRecord};
use crate::villager::VillagerProfession;

/// Every active project, in admission order.
pub(crate) fn projects(record: &SettlementRecord) -> impl Iterator<Item = &Project> {
    record.projects.iter()
}

/// Every active project, mutably.
pub(crate) fn projects_mut(record: &mut SettlementRecord) -> impl Iterator<Item = &mut Project> {
    record.projects.iter_mut()
}

/// [`projects`] for a settlement id.
pub(crate) fn projects_of(
    settlements: &Registry,
    id: SettlementId,
) -> impl Iterator<Item = &Project> {
    settlements.get(id).into_iter().flat_map(projects)
}

/// The project whose canonical structure ledger entry is `handle`.
pub(crate) fn project_for_structure<'a>(
    record: &'a SettlementRecord,
    handle: &str,
) -> Option<&'a Project> {
    projects(record).find(|project| project.structure == handle)
}

/// [`project_for_structure`], mutably.
pub(crate) fn project_for_structure_mut<'a>(
    record: &'a mut SettlementRecord,
    handle: &str,
) -> Option<&'a mut Project> {
    projects_mut(record).find(|project| project.structure == handle)
}

/// Admit `project` into the settlement's active set.
pub(crate) fn push_project(record: &mut SettlementRecord, project: Project) {
    record.projects.push(project);
}

/// Bind `builder` to the project that owns `handle`. Returns whether the
/// binding changed, so the caller can bump the revision without holding a
/// borrow across the mutation.
pub(crate) fn bind_builder(
    record: &mut SettlementRecord,
    handle: &str,
    builder: Option<(uuid::Uuid, Option<VillagerProfession>)>,
) -> bool {
    let Some(project) = record
        .projects
        .iter_mut()
        .find(|project| project.structure == handle)
    else {
        return false;
    };
    if project.builder == builder {
        return false;
    }
    project.builder = builder;
    true
}

/// Clear the binding of `builder` on whichever project holds it.
pub(crate) fn unbind_builder(record: &mut SettlementRecord, builder: uuid::Uuid) -> bool {
    let Some(project) = record
        .projects
        .iter_mut()
        .find(|project| project.builder.is_some_and(|(bound, _)| bound == builder))
    else {
        return false;
    };
    project.builder = None;
    true
}

/// Remove and return the project that owns `handle`.
pub(crate) fn take_project(record: &mut SettlementRecord, handle: &str) -> Option<Project> {
    let index = record
        .projects
        .iter()
        .position(|project| project.structure == handle)?;
    Some(record.projects.remove(index))
}

/// Drop every active project. Only a settlement teardown uses this.
pub(crate) fn clear_projects(record: &mut SettlementRecord) {
    record.projects.clear();
}

/// The project this villager builds.
///
/// The bound builder is the authority; a project with no binding answers only
/// while it is the settlement's first project, which is what the single-slot
/// store did before S1b. S2 raises the project cap once every caller resolves
/// by binding.
pub(crate) fn project_of_builder(
    record: &SettlementRecord,
    builder: uuid::Uuid,
) -> Option<&Project> {
    projects(record)
        .find(|project| project.builder.is_some_and(|(bound, _)| bound == builder))
        .or_else(|| projects(record).next())
}

/// Every pending owner request, in the order it was submitted.
pub(crate) fn orders(record: &SettlementRecord) -> impl Iterator<Item = &OwnerOrder> {
    record.orders.iter()
}

/// The request the planner may start on its next free slot: the queue head.
pub(crate) fn head_order(record: &SettlementRecord) -> Option<&OwnerOrder> {
    orders(record).next()
}

/// [`head_order`] for a settlement id.
pub(crate) fn head_order_of(settlements: &Registry, id: SettlementId) -> Option<&OwnerOrder> {
    settlements.get(id).and_then(head_order)
}

/// Append an owner request at the tail of the queue.
pub(crate) fn push_order(record: &mut SettlementRecord, order: OwnerOrder) {
    record.orders.push(order);
}

/// Remove and return the request at `index`.
pub(crate) fn take_order(record: &mut SettlementRecord, index: usize) -> Option<OwnerOrder> {
    if index < record.orders.len() {
        Some(record.orders.remove(index))
    } else {
        None
    }
}

/// Remove and return the queue head.
pub(crate) fn take_head_order(record: &mut SettlementRecord) -> Option<OwnerOrder> {
    if record.orders.is_empty() {
        None
    } else {
        Some(record.orders.remove(0))
    }
}

/// Insert an owner request at `index` (clamped to the queue length), used to
/// restore a request at the position it held before a failed start.
pub(crate) fn insert_order(record: &mut SettlementRecord, index: usize, order: OwnerOrder) {
    let index = index.min(record.orders.len());
    record.orders.insert(index, order);
}

/// Drop every pending request.
pub(crate) fn clear_orders(record: &mut SettlementRecord) {
    record.orders.clear();
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

/// True when this villager is the bound builder of any settlement's project.
pub(crate) fn bound_builder(settlements: &Registry, builder: uuid::Uuid) -> bool {
    settlements.records().any(|record| {
        projects(record).any(|project| project.builder.is_some_and(|(bound, _)| bound == builder))
    })
}

/// Heap held by the construction fields, for `SettlementRecord::heap_estimate`.
pub(crate) fn record_heap_bytes(record: &SettlementRecord) -> usize {
    record.projects.capacity() * std::mem::size_of::<Project>()
        + projects(record)
            .map(|project| project.blueprint.capacity() + project.structure.capacity())
            .sum::<usize>()
        + record.orders.capacity() * std::mem::size_of::<OwnerOrder>()
        + orders(record)
            .map(|order| order.blueprint.capacity())
            .sum::<usize>()
}
