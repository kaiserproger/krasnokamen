//! Settlement planning and certification. Physical construction, material
//! debit, placement order and progress belong solely to the structure executor.
use serde::{Deserialize, Serialize};

use super::blueprints::{self, Rotation};
use super::buildings::{self, BuildingId, BuildingKind};
use super::decision_log;
use super::{SettlementId, SettlementRecord, layout};
use crate::plugin_structures::settlement_builds as executor;
use crate::tick::TickState;
use crate::villager::VillagerProfession as Profession;
use mc_script::{ScriptStructureState, ScriptWorldBounds};

#[path = "construction_availability.rs"]
mod availability;
pub(crate) use availability::SourceReason;
pub(crate) use availability::gatherer_for_item;

#[path = "construction_access.rs"]
mod access;
pub(crate) use access::projects_of;
pub(crate) use access::{
    bind_builder, bound_builder, head_order, idle, orders, pending_missing, project_for_structure,
    project_of_builder, projects, record_heap_bytes, unbound_project, upgrading,
};

#[path = "construction_furnace.rs"]
pub(crate) mod furnace;

#[path = "construction_founding.rs"]
pub(super) mod founding;

#[path = "construction_site_refusals.rs"]
pub(super) mod site_refusals;
// The funded site search and the off-cadence step live beside the sweeps they
// drive (code-health ratchet, 2026-11-26); the planner, `start_build` and the
// tick read them through these names.
use site_refusals::{FundedSite, SitePass, scan_sites, site_project, step_sites};

#[path = "construction_site_turn.rs"]
pub(super) mod site_turn;
use site_turn::plan_order;
pub(crate) use site_turn::{replant_verdict, tick};

#[path = "construction_replant.rs"]
mod replant;
use replant::replant_check;
pub(crate) use replant::{CrownScan, Replant};
pub(super) use replant::{
    Front, HouseLot, LotChecks, crowned_house_lot, keeps_free_lot, takes_free_lot,
};

#[path = "construction_upgrade.rs"]
mod upgrade;

#[path = "construction_builders.rs"]
mod builders;
use builders::{assign_builder, builder_free, probe_storage};

#[path = "construction_storage.rs"]
mod storage;
use storage::candidate_kinds;
pub(crate) use storage::order_storage_haul;

pub(crate) const PLAN_CADENCE: u64 = 20;
/// Active projects a settlement may hold at once: one per builder candidate the
/// planner scans, so the joint funding proof and the tick stay bounded.
pub(crate) const MAX_PROJECTS: usize = 8;
/// Owner requests kept in the queue; a request beyond it is refused by name.
pub(crate) const MAX_ORDERS: usize = 8;

/// A same-dimension native outpost. Construction stage and culture are derived
/// from committed structures; lineage belongs to the run's audit, not this row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundingExpedition {
    /// Ground-level min corner of the authored outpost plan.
    pub destination: [i32; 3],
    pub household: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub settlement: SettlementId,
    pub blueprint: String,
    pub level: u8,
    pub origin: [i32; 3],
    /// Clockwise quarter turns.
    pub rotation: u8,
    pub building: Option<BuildingId>,
    /// The canonical structure ledger owns all work/material progress.
    pub structure: String,
    /// Set by an owner/operator order. Active projects retain their FIFO slot,
    /// including autonomous work that has not placed a cell yet.
    #[serde(default)]
    pub ordered: bool,
    /// The resident bound to this project and the profession to restore on
    /// release (`None` for a native Builder). One binding per project, and a
    /// villager is bound to at most one project. A row written before this
    /// field refuses to decode instead of reading as unbound (DESIGN.md:25);
    /// an explicit `null` stays a valid "unbound".
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    pub builder: Option<(uuid::Uuid, Option<Profession>)>,
    /// Submission sequence of the owner request this project was lifted from,
    /// so a retried preparation restores its FIFO place even after other
    /// requests were cancelled (D8 review P2-3). A row without the key refuses
    /// rather than reading as "no sequence" (DESIGN.md:25); an explicit null
    /// stays valid for an autonomous project.
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    pub order_seq: Option<u64>,
}
/// One pending owner request, stored in the settlement post-image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerOrder {
    /// Submission order; the queue is sorted by it, so a restored request keeps
    /// its place relative to the requests submitted around it. No default: a
    /// row without the key refuses (DESIGN.md:25).
    pub seq: u64,
    pub blueprint: String,
    pub level: u8,
    pub building: Option<BuildingId>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum OrderOutcome {
    Started,
    Queued { reason: String },
}

impl OrderOutcome {
    pub(crate) fn message(&self, blueprint: &str) -> String {
        match self {
            Self::Started => format!("Started {blueprint} construction."),
            Self::Queued { reason } => format!("Queued {blueprint}: {reason}"),
        }
    }
}

pub(crate) fn request_build(
    state: &mut TickState,
    id: SettlementId,
    blueprint: &str,
) -> Result<OrderOutcome, String> {
    blueprints::get(&state.registry, blueprint).ok_or("Unknown blueprint.")?;
    submit_order(
        state,
        id,
        OwnerOrder {
            seq: 0,
            blueprint: blueprint.to_owned(),
            level: 1,
            building: None,
        },
    )
}

pub(crate) fn request_upgrade(
    state: &mut TickState,
    id: SettlementId,
    building: BuildingId,
) -> Result<OrderOutcome, String> {
    let record = state.settlements.get(id).ok_or("Settlement not found.")?;
    let target = record
        .buildings
        .get(&building)
        .ok_or("Building not found.")?;
    let (blueprint, ..) = upgrade_blueprint(state, id, target)
        .ok_or("No blueprint upgrade available for this building.")?;
    let level = owner_upgrade_level(state, id, building)
        .ok_or("No blueprint upgrade available for this building.")?;
    submit_order(
        state,
        id,
        OwnerOrder {
            seq: 0,
            blueprint: blueprint.id.clone(),
            level,
            building: Some(building),
        },
    )
}

/// Owner admission and its dialog preview share the same next-level choice.
pub(crate) fn owner_upgrade_level(
    state: &TickState,
    id: SettlementId,
    building: BuildingId,
) -> Option<u8> {
    let record = state.settlements.get(id)?;
    let target = record.buildings.get(&building)?;
    let (blueprint, ..) = upgrade_blueprint(state, id, target)?;
    // A confirmed level is immutable, including while its predecessor waits.
    let current = access::projects(record)
        .filter(|project| project.building == Some(building))
        .map(|project| project.level)
        .chain(
            access::orders(record)
                .filter(|order| order.building == Some(building))
                .map(|order| order.level),
        )
        .max()
        .unwrap_or(target.level)
        .max(target.level);
    blueprint.level(current + 1).map(|_| current + 1)
}

/// `attempts`: lots a new building's site slice may examine (an upgrade
/// searches no site).
fn start_order(
    state: &mut TickState,
    id: SettlementId,
    order: &OwnerOrder,
    attempts: u32,
) -> Result<String, String> {
    if upgrade::waits(state, id, order) {
        return Err("Busy.".into());
    }
    match order.building {
        Some(building) => start_upgrade(state, id, building),
        None => start_build(state, id, &order.blueprint, attempts),
    }
}

fn submit_order(
    state: &mut TickState,
    id: SettlementId,
    order: OwnerOrder,
) -> Result<OrderOutcome, String> {
    if state
        .settlements
        .get(id)
        .is_none_or(|record| access::orders(record).count() >= MAX_ORDERS)
    {
        return Err(format!(
            "Construction queue is full: MAX_ORDERS={MAX_ORDERS}."
        ));
    }
    // The request takes its submission sequence when it is queued: a project
    // started from it that later retries must still restore the place it was
    // submitted at (D8 review R2-4). The sequence is committed only when the
    // request is kept, so a refused submission leaves the record untouched.
    let seq = state
        .settlements
        .get(id)
        .ok_or("Settlement not found.")?
        .order_seq
        .saturating_add(1);
    let mut order = order;
    order.seq = seq;
    // A request never overtakes a waiting queue: the planner admits funded
    // requests in submission order, so a new one joins the queue and lifts on
    // the next pass (DESIGN 178-185, D8 review P2-3).
    if access::orders(state.settlements.get(id).expect("checked settlement"))
        .next()
        .is_some()
    {
        let record = state
            .settlements
            .get_mut(id)
            .ok_or("Settlement not found.")?;
        record.order_seq = seq;
        access::insert_order_by_seq(record, order);
        record.revision = record.revision.saturating_add(1);
        // The reason names this request's wait, not the older head's (D8
        // review P3/A2).
        let index = orders(state.settlements.get(id).expect("pushed settlement"))
            .position(|queued| queued.seq == seq)
            .unwrap_or(0);
        let reason = pending_reason_at(state, id, index).unwrap_or_else(|| "queued".into());
        if reason == site_refusals::WAITING_FOR_SITE || reason == READY {
            site_turn::enter(state, id);
        }
        return Ok(OrderOutcome::Queued { reason });
    }
    // Every order starts on a later order turn under the server's one site
    // budget (`site_turn`, owner D42), never in this command's tick: only its
    // admission checks run here, and nothing is withdrawn or reserved.
    let admitted = if upgrade::waits(state, id, &order) {
        Err("Busy.".to_owned())
    } else {
        match order.building {
            Some(building) => upgrade::upgradable(state, id, building).map(|()| READY),
            None => {
                buildable(state, id, &order.blueprint).map(|()| site_refusals::WAITING_FOR_SITE)
            }
        }
    };
    let (reason, turn) = match admitted {
        Ok(reason) => (reason.to_owned(), true),
        Err(reason) if reason == "Busy." => ("busy".to_owned(), false),
        Err(reason) if reason.starts_with("not enough:") => (reason, false),
        Err(error) => return Err(error),
    };
    let record = state
        .settlements
        .get_mut(id)
        .ok_or("Settlement not found.")?;
    record.order_seq = seq;
    access::insert_order_by_seq(record, order);
    record.revision = record.revision.saturating_add(1);
    if turn {
        site_turn::enter(state, id);
    }
    Ok(OrderOutcome::Queued { reason })
}

pub(crate) fn cancel_pending(state: &mut TickState, id: SettlementId) -> Result<(), String> {
    let record = state
        .settlements
        .get_mut(id)
        .ok_or("Settlement not found.")?;
    access::take_head_order(record).ok_or("No pending construction order.")?;
    record.revision = record.revision.saturating_add(1);
    state.settlements.site_watch.remove(&id);
    Ok(())
}

/// Cancel one queued request by zero-based position; the error names the
/// one-based position the player asked for. The queue is sequenced, so the
/// position is the one the owner sees (D8 review R4-5: the indexed cancel the
/// owner command and UI dispatch resolve).
pub(crate) fn cancel_pending_at(
    state: &mut TickState,
    id: SettlementId,
    index: usize,
) -> Result<(), String> {
    let record = state
        .settlements
        .get_mut(id)
        .ok_or("Settlement not found.")?;
    access::take_order(record, index)
        .ok_or_else(|| format!("No pending construction order #{}.", index + 1))?;
    record.revision = record.revision.saturating_add(1);
    state.settlements.site_watch.remove(&id);
    Ok(())
}

/// The waiting reason of one queued request (zero-based). Funding is checked
/// before the slot, so an unfunded request is never reported as merely busy.
pub(crate) fn pending_reason_at(
    state: &TickState,
    id: SettlementId,
    index: usize,
) -> Option<String> {
    let record = state.settlements.get(id)?;
    let order = orders(record).nth(index)?;
    let blueprint = blueprints::get(&state.registry, &order.blueprint)?;
    let level = blueprint.level(order.level)?;
    if let Some(reason) = upgrade::pending_reason(state, id, order) {
        return Some(reason);
    }
    if let Some(reason) = funding_error(state, id, &level.materials) {
        return Some(reason);
    }
    if claim_slot(state, id).is_err() {
        return Some("busy".into());
    }
    // A queued upgrade whose site diverged says so, with the cell: the owner must
    // know which block to clear, and the answer comes back to "ready" on its own
    // once it is cleared.
    if let Some(reason) = preparation::queued_refusal(state, id, order) {
        return Some(reason);
    }
    Some(
        if order.building.is_none() {
            site_refusals::WAITING_FOR_SITE
        } else {
            READY
        }
        .into(),
    )
}

/// The waiting reason of a funded upgrade with a free slot: its order turn
/// admits it (`site_turn`).
pub(crate) const READY: &str = "ready";

/// A confirmed level remains unchanged even while its predecessor is missing.
fn order_level(state: &TickState, id: SettlementId, order: &OwnerOrder) -> Option<u8> {
    let Some(building) = order.building else {
        return Some(1);
    };
    let record = state.settlements.get(id)?;
    let target = record.buildings.get(&building)?;
    let blueprint = blueprints::get(&state.registry, &order.blueprint)?;
    blueprint.level(target.level.saturating_add(1))?;
    blueprint.level(order.level).map(|_| order.level)
}

fn notify_owner(state: &mut TickState, id: SettlementId, message: String) {
    if let Some(connection) = super::owner_connection(state, id) {
        state.push_player(connection, crate::boundary::PlayerChange::Chat { message });
    }
}

/// An active project retains its FIFO slot, regardless of progress or origin.
fn claim_slot(state: &TickState, id: SettlementId) -> Result<(), String> {
    let record = state.settlements.get(id).ok_or("Settlement not found.")?;
    if access::projects(record).count() >= MAX_PROJECTS {
        Err("Busy.".into())
    } else {
        Ok(())
    }
}

/// Cancel without undoing placed cells or refunding stock. Callers own the
/// admission policy; stall recovery must never discard paid construction.
/// Addressed by structure handle so a parallel settlement cancels exactly its
/// own project (S1b); a handle that names no active project is a no-op.
fn cancel_project(state: &mut TickState, id: SettlementId, structure: &str) -> Result<(), String> {
    let Some((handle, borrowed)) = state
        .settlements
        .get(id)
        .and_then(|record| access::project_for_structure(record, structure))
        .map(|project| (project.structure.clone(), project.builder))
    else {
        return Ok(());
    };
    executor::cancel_structure(state, id, &handle)
        .map_err(|_| "Construction admission failed.".to_string())?;
    if borrowed.is_some() {
        release_builder(state, id, borrowed);
    }
    let record = state.settlements.get_mut(id).expect("preempted settlement");
    access::take_project(record, &handle);
    record.revision = record.revision.saturating_add(1);
    Ok(())
}

fn wood_part(path: &str) -> Option<(&str, i64)> {
    [
        ("_pressure_plate", 1),
        ("_fence_gate", 1),
        ("_trapdoor", 1),
        ("_planks", 1),
        ("_stairs", 1),
        ("_slab", 1),
        ("_fence", 1),
        ("_door", 1),
        ("_log", 4),
    ]
    .into_iter()
    .find_map(|(suffix, weight)| path.strip_suffix(suffix).map(|s| (s, weight)))
}

fn wood_species_name<'a>(
    state: &TickState,
    name: &'a mc_data::Identifier,
) -> Option<(&'a str, i64)> {
    if name.namespace() != "minecraft" {
        return None;
    }
    let (species, weight) = wood_part(name.path())?;
    if species.starts_with("stripped_") {
        return None;
    }
    let log = mc_data::Identifier::parse(format!("minecraft:{species}_log")).ok()?;
    state.registry.items.id_of(&log)?;
    Some((species, weight))
}

pub(super) fn wood_species(state: &TickState, item: u32) -> Option<&str> {
    wood_species_name(state, state.registry.items.name_of(item)?).map(|(species, _)| species)
}

/// Prefer a fully funded wood family, then (for gathering/diagnostics only)
/// the largest existing partial stock. Admission never accepts a partial bill.
fn select_species<'a>(
    state: &'a TickState,
    materials: &[(u32, u32)],
    images: &[super::inventory::Image],
    allow_partial: bool,
) -> Option<&'a str> {
    species_candidates(state, images)
        .find_map(|species| {
            let bill = species_bill(state, materials, species)?;
            super::construction_crafting::can_fund(state, &bill, images).then_some(species)
        })
        .or_else(|| {
            allow_partial
                .then(|| species_candidates(state, images).next())
                .flatten()
        })
}

/// The wood species of this settlement's stock, richest first, always with oak
/// as the fallback: one order per candidate, so a reserved oak bill never hides
/// a fundable birch one (SOL-2b).
fn species_candidates<'a>(
    state: &'a TickState,
    images: &[super::inventory::Image],
) -> impl Iterator<Item = &'a str> {
    let mut candidates = vec![("oak", 0_i64)];
    for stack in images
        .iter()
        .flat_map(|image| &image.slots)
        .filter(|s| !s.is_empty())
    {
        let Some(name) = state.registry.items.name_of(stack.item_id) else {
            continue;
        };
        let Some((species, weight)) = wood_species_name(state, name) else {
            continue;
        };
        let amount = i64::from(stack.count) * weight;
        if let Some((_, total)) = candidates.iter_mut().find(|(known, _)| *known == species) {
            *total += amount;
        } else {
            candidates.push((species, amount));
        }
    }
    candidates.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    candidates.into_iter().map(|(species, _)| species)
}

fn species_item(state: &TickState, item: u32, species: &str) -> Option<u32> {
    let name = state.registry.items.name_of(item)?;
    if species == "oak" || !name.as_str().starts_with("minecraft:") {
        return Some(item);
    }
    let Some(suffix) = name.path().strip_prefix("oak_") else {
        return Some(item);
    };
    state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(format!("minecraft:{species}_{suffix}")).ok()?)
}

fn species_bill(
    state: &TickState,
    materials: &[(u32, u32)],
    species: &str,
) -> Option<Vec<(u32, u32)>> {
    materials
        .iter()
        .map(|&(item, count)| Some((species_item(state, item, species)?, count)))
        .collect()
}

/// Resolve the authored white bed to one concrete vanilla bed color. A
/// complete stock wins; otherwise prefer a live sheep and its partial stock.
fn bed_colors(
    state: &TickState,
    id: SettlementId,
    materials: &[(u32, u32)],
    images: &[super::inventory::Image],
) -> ([u8; 16], usize) {
    let mut result = [0; 16];
    let Some((_, needed)) = materials.iter().find(|(item, _)| {
        state
            .registry
            .items
            .name_of(*item)
            .is_some_and(|name| name.as_str() == "minecraft:white_bed")
    }) else {
        return (result, 1);
    };
    let sheep = state.settlements.get(id).map_or(0, |record| {
        availability::sheep_colors(state, record.anchor, record.radius.min(256))
    });
    let mut beds = [0_u64; 16];
    let mut wool = [0_u64; 16];
    for stack in images
        .iter()
        .flat_map(|image| &image.slots)
        .filter(|stack| stack.count > 0)
    {
        let Some(name) = state.registry.items.name_of(stack.item_id) else {
            continue;
        };
        if name.namespace() != "minecraft" {
            continue;
        }
        let (color, counts) = if let Some(color) = name.path().strip_suffix("_bed") {
            (color, &mut beds)
        } else if let Some(color) = name.path().strip_suffix("_wool") {
            (color, &mut wool)
        } else {
            continue;
        };
        if let Some(index) = crate::blueprint::DYE_COLORS
            .iter()
            .position(|candidate| *candidate == color)
        {
            counts[index] += stack.count as u64;
        }
    }
    let mut candidates = [(0_u8, false, false, 0_u64, 0_u64); 16];
    for (index, candidate) in candidates.iter_mut().enumerate() {
        let covered = beds[index] >= u64::from(*needed)
            || wool[index] >= u64::from(*needed).saturating_sub(beds[index]) * 3;
        *candidate = (
            index as u8,
            covered,
            sheep & (1 << index) != 0,
            beds[index],
            wool[index],
        );
    }
    let count = candidates.len();
    candidates[..count].sort_unstable_by(|a, b| {
        (b.1, b.2, b.3, b.4)
            .cmp(&(a.1, a.2, a.3, a.4))
            .then_with(|| a.0.cmp(&b.0))
    });
    for (slot, candidate) in result.iter_mut().zip(&candidates[..count]) {
        *slot = candidate.0;
    }
    (result, count)
}

fn bed_bill<'a>(
    state: &TickState,
    materials: &'a [(u32, u32)],
    color: u8,
) -> Option<std::borrow::Cow<'a, [(u32, u32)]>> {
    if color == 0 {
        return Some(std::borrow::Cow::Borrowed(materials));
    }
    let Some(white) = materials.iter().find_map(|(item, _)| {
        state
            .registry
            .items
            .name_of(*item)
            .filter(|name| name.as_str() == "minecraft:white_bed")
            .map(|_| *item)
    }) else {
        return Some(std::borrow::Cow::Borrowed(materials));
    };
    let colored = state.registry.items.id_of(
        &mc_data::Identifier::parse(format!(
            "minecraft:{}_bed",
            crate::blueprint::DYE_COLORS[usize::from(color)]
        ))
        .ok()?,
    )?;
    Some(std::borrow::Cow::Owned(
        materials
            .iter()
            .map(|&(item, count)| (if item == white { colored } else { item }, count))
            .collect(),
    ))
}

fn funded_variant<'a>(
    state: &'a TickState,
    id: SettlementId,
    materials: &[(u32, u32)],
    images: &[super::inventory::Image],
) -> Option<(&'a str, u8)> {
    let (colors, count) = bed_colors(state, id, materials, images);
    // An unreadable active bill proves nothing, so no candidate is funded.
    let joint = joint_requirement(state, id)?;
    colors[..count].iter().copied().find_map(|color| {
        let bill = bed_bill(state, materials, color)?;
        // Every species of the candidate is proved against the exact joint
        // bill: the species is chosen for the candidate alone (so another
        // project's oak obligation is never re-coloured, SOL-2), and a reserved
        // oak bill does not hide a fundable birch one (SOL-2b).
        species_candidates(state, images).find_map(|species| {
            let candidate = species_bill(state, &bill, species)?;
            let combined = with_joint(candidate, &joint);
            super::construction_crafting::can_fund(state, &combined, images)
                .then_some((species, color))
        })
    })
}

/// Materials already promised to the other active projects, per item, net of
/// each project's own bound builder carry. Admission reserves them so a later
/// project never funds itself out of an earlier one's stock (D8, joint proof).
fn joint_requirement(state: &TickState, id: SettlementId) -> Option<Vec<(u32, u32)>> {
    joint_active_bill(state, id)
}

/// The authored footprint box of one project. Only the neighbour tests read the
/// authored box now: production stands and placements use the body AABB
/// (`villager::building::in_other_body`), so this stays test-only (D8 merge).
#[cfg(test)]
pub(crate) fn footprint(
    state: &TickState,
    project: &Project,
) -> Option<mc_script::ScriptWorldBounds> {
    let blueprint = blueprints::get(&state.registry, &project.blueprint)?;
    let size = blueprint.footprint(blueprints::Rotation::from_quarter_turns(project.rotation));
    let min = project.origin;
    let max = [
        min[0] + i32::from(size[0]) - 1,
        min[1] + i32::from(size[1]) - 1,
        min[2] + i32::from(size[2]) - 1,
    ];
    mc_script::ScriptWorldBounds::new(min, max).ok()
}

/// Whether `at` falls inside another active project's authored footprint.
/// Test-only after the merge: the production rule is the body AABB.
#[cfg(test)]
pub(crate) fn in_other_footprint(
    state: &TickState,
    id: SettlementId,
    except: &str,
    at: [i32; 3],
) -> bool {
    access::projects_of(&state.settlements, id)
        .filter(|project| project.structure != except)
        .filter_map(|project| footprint(state, project))
        .any(|bounds| {
            (0..3).all(|axis| bounds.min[axis] <= at[axis] && at[axis] <= bounds.max[axis])
        })
}

/// Every other active project's remaining bill, net of that project's own bound
/// builder carry. A fetch or a generic recipe must not spend it (SOL-3).
pub(crate) fn other_active_bill(
    state: &TickState,
    id: SettlementId,
    except: &str,
) -> Option<Vec<(u32, u32)>> {
    let mut total: Vec<(u32, u32)> = Vec::new();
    for handle in access::projects_of(&state.settlements, id)
        .map(|project| project.structure.clone())
        .collect::<Vec<_>>()
    {
        if handle == except {
            continue;
        }
        // An unreadable active bill proves nothing: skipping it would let a
        // haul spend materials another project already needs (SOL-3b).
        let bill = project_bill(state, id, &handle)?;
        for (item, count) in bill {
            match total.iter_mut().find(|(known, _)| *known == item) {
                Some((_, sum)) => *sum = sum.saturating_add(count),
                None => total.push((item, count)),
            }
        }
    }
    Some(total)
}

/// The joint proof for a site that is not itself a project yet: the candidate
/// bill must be fundable on top of every active project's remaining bill
/// (D8 review P2-2). An unreadable active bill proves nothing.
fn jointly_fundable(state: &TickState, id: SettlementId, bill: &[(u32, u32)]) -> bool {
    let Some(joint) = joint_active_bill(state, id) else {
        return false;
    };
    // Nothing is promised to another project: the site's own proof decides.
    if joint.is_empty() {
        return true;
    }
    let images = executor::source_images(state, id);
    super::construction_crafting::can_fund(state, &with_joint(bill.to_vec(), &joint), &images)
}

/// Append the joint requirement to a candidate bill, keeping one entry per item.
fn with_joint(mut bill: Vec<(u32, u32)>, joint: &[(u32, u32)]) -> Vec<(u32, u32)> {
    for &(item, count) in joint {
        match bill.iter_mut().find(|(known, _)| *known == item) {
            Some((_, sum)) => *sum = sum.saturating_add(count),
            None => bill.push((item, count)),
        }
    }
    bill
}

fn selected_bill<'a>(
    state: &TickState,
    id: SettlementId,
    materials: &'a [(u32, u32)],
    images: &[super::inventory::Image],
) -> Option<std::borrow::Cow<'a, [(u32, u32)]>> {
    let bill = selected_bed_bill(state, id, materials, images)?;
    let species = select_species(state, &bill, images, true)?;
    if species == "oak" {
        Some(bill)
    } else {
        species_bill(state, &bill, species).map(std::borrow::Cow::Owned)
    }
}

/// Iron and wool demand do not depend on a wooden block variant. Keep those
/// per-brain queries on the bed proof instead of re-funding the entire house.
fn selected_bed_bill<'a>(
    state: &TickState,
    id: SettlementId,
    materials: &'a [(u32, u32)],
    images: &[super::inventory::Image],
) -> Option<std::borrow::Cow<'a, [(u32, u32)]>> {
    let (colors, count) = bed_colors(state, id, materials, images);
    bed_bill(state, materials, *colors.get(..count)?.first()?)
}

/// A real worker load can fund the next building or upgrade before a stack is full.
pub(crate) fn candidate_funded_by_delivery(
    state: &TickState,
    id: SettlementId,
    carry: &[Option<mc_data::ItemStack>],
) -> bool {
    let Some(record) = state.settlements.get(id) else {
        return false;
    };
    if carry.iter().all(Option::is_none) {
        return false;
    }
    let mut images = executor::source_images(state, id);
    let active_bill = joint_active_bill(state, id).filter(|bill| !bill.is_empty());
    let funded = |images: &[super::inventory::Image]| {
        if access::projects(record).next().is_some() {
            active_bill
                .as_ref()
                .is_some_and(|bill| super::construction_crafting::can_fund(state, bill, images))
        } else {
            funded_candidate(state, record, images)
        }
    };
    // The partial load must close a shortfall, not interrupt harvesting every
    // time an already-funded candidate is still waiting for a legal site.
    if funded(&images) {
        return false;
    }
    let Some(chest) = images.first_mut() else {
        return false;
    };
    // This is a read-only funding probe; only the regular inventory authority
    // can transfer the actual carry to the real container.
    chest.slots.extend(carry.iter().flatten().cloned());
    funded(&images)
}

fn funding_error(state: &TickState, id: SettlementId, materials: &[(u32, u32)]) -> Option<String> {
    let images = executor::source_images(state, id);
    if funded_variant(state, id, materials, &images).is_some() {
        return None;
    }
    let Some(bill) = selected_bill(state, id, materials, &images) else {
        return Some("Construction bed material is unavailable.".into());
    };
    for &(item, count) in bill.iter() {
        if !super::construction_crafting::can_fund(state, &[(item, count)], &images) {
            return Some(match state.registry.items.name_of(item) {
                Some(name) => format!("not enough: {name} x{count}"),
                None => format!("not enough: item #{item} x{count}"),
            });
        }
    }
    // Requirements can each fit separately while sharing the same finite logs,
    // their recipe surplus, or the materials already promised to another active
    // project. The whole-project proof authorizes admission.
    // A candidate that fails on its own is a shared-ingredient shortfall; only
    // a candidate that would be fundable alone but not on top of the other
    // projects' promises is a joint failure (D8 review P3).
    let own_fundable = super::construction_crafting::can_fund(state, &bill, &images);
    let Some(joint) = joint_active_bill(state, id) else {
        return Some("not enough: another project's bill is unreadable".into());
    };
    if own_fundable && !joint.is_empty() {
        let mut combined = bill.to_vec();
        for (item, count) in &joint {
            match combined.iter_mut().find(|(known, _)| known == item) {
                Some((_, sum)) => *sum = sum.saturating_add(*count),
                None => combined.push((*item, *count)),
            }
        }
        if !super::construction_crafting::can_fund(state, &combined, &images) {
            return Some("not enough: promised to another project".into());
        }
    }
    Some("not enough: shared recipe ingredients".into())
}

/// A new building's admission checks before any site search: a free slot, a
/// first level and its funding. Nothing is withdrawn or reserved: funding is
/// proved again when the project starts.
fn buildable(state: &TickState, id: SettlementId, blueprint: &str) -> Result<(), String> {
    claim_slot(state, id)?;
    let blueprint = blueprints::get(&state.registry, blueprint).ok_or("Unknown blueprint.")?;
    let level = blueprint.level(1).ok_or("Blueprint has no first level.")?;
    funding_error(state, id, &level.materials).map_or(Ok(()), Err)
}

/// Owner/operator entrypoints check `can_manage`; admission starts the sole
/// funded project, without withdrawing or placing any material. One bounded,
/// resumable sweep slice of at most `attempts` lots, granted by the tick's
/// site budget (`site_turn`): a sweep the slice does not finish waits for a
/// site, and later turns continue it.
fn start_build(
    state: &mut TickState,
    id: SettlementId,
    blueprint: &str,
    attempts: u32,
) -> Result<String, String> {
    buildable(state, id, blueprint)?;
    let registry = std::sync::Arc::clone(&state.registry);
    let blueprint = blueprints::get(&registry, blueprint).ok_or("Unknown blueprint.")?;
    let images = executor::source_images(state, id);
    let mut sweeps = state.settlements.site_sweep.remove(&id).unwrap_or_default();
    let pass = {
        let record = state.settlements.get(id).expect("claimed settlement");
        site_project(
            state,
            record,
            std::iter::once(blueprint),
            &mut sweeps,
            record.construction_cursor,
            attempts,
            &images,
        )
    };
    // An owner order's own scan never arms the stepping flag (review round 2
    // N2): `step_sites` repeats only the autonomous scan, and this very state
    // (a pending order, a storehouse wait) is what blocks that scan, so arming
    // here would step a walk the settlement cannot admit from. The settlement's
    // own planner pass owns the flag.
    state.settlements.site_sweep.insert(id, sweeps);
    let SitePass {
        site,
        cursor,
        report,
        running,
        ..
    } = pass;
    let Some(mut site) = site else {
        let record = state.settlements.get_mut(id).expect("planned settlement");
        record.construction_cursor = cursor;
        record.revision = record.revision.saturating_add(1);
        // Every examined slice belongs to the refusal watch's sweep total.
        site_refusals::observe(state, id, &blueprint.id, report);
        return Err(site_refusals::miss(running));
    };
    site.project.ordered = true;
    // The admitted project's own handle is the authority, not a search by
    // blueprint: two parallel houses share a blueprint (D8 review P2-3).
    let cells = site.cells;
    let bounds = site.bounds;
    if !begin_cells(state, site.project, bounds, cells) {
        return Err("Construction admission failed.".into());
    }
    let handle = crate::settlements::construction::access::projects_of(&state.settlements, id)
        .last()
        .map_or_else(String::new, |project| project.structure.clone());
    state
        .settlements
        .get_mut(id)
        .expect("admitted settlement")
        .construction_cursor = cursor;
    state.settlements.site_watch.remove(&id);
    Ok(handle)
}

/// Admit a funded upgrade through the canonical structure executor.
fn start_upgrade(
    state: &mut TickState,
    id: SettlementId,
    building: BuildingId,
) -> Result<String, String> {
    upgrade::upgradable(state, id, building)?;
    let record = state.settlements.get(id).expect("claimed settlement");
    let building = &record.buildings[&building];
    let (blueprint, origin, rotation) =
        upgrade_blueprint(state, id, building).expect("upgradable building");
    let level = building.level + 1;
    let images = executor::source_images(state, id);
    let (species, color) = funded_variant(
        state,
        id,
        &blueprint.level(level).expect("upgrade level").materials,
        &images,
    )
    .map(|(species, color)| (species.to_owned(), color))
    .ok_or("Construction materials changed during admission.")?;
    let project = Project {
        settlement: id,
        blueprint: blueprint.id.clone(),
        level,
        origin,
        rotation,
        building: Some(building.id),
        structure: String::new(),
        ordered: true,
        builder: None,
        order_seq: None,
    };
    let (bounds, mut cells) = planned_cells(state, &project, &species, &mut Default::default())
        .map_err(|reason| preparation::refusal_text(state, &project, &species, reason))?;
    apply_bed_color(state, &mut cells, color).ok_or("Construction bed color unavailable.")?;
    if !begin_cells(state, project, bounds, cells) {
        return Err("Construction admission failed.".into());
    }
    Ok(access::projects_of(&state.settlements, id)
        .last()
        .map_or_else(String::new, |project| project.structure.clone()))
}

/// No escrow is discarded: real carried items stay with the same native body.
pub(crate) fn detach(v: &mut crate::villager::VillagerState) -> bool {
    if v.data.profession != Profession::Builder {
        return false;
    }
    v.data.profession = if super::allocator::traded(v) {
        Profession::Mason
    } else {
        Profession::None
    };
    if v.data.profession == Profession::None {
        v.offers.clear();
    }
    v.job_site = None;
    v.navigation_target = None;
    v.navigation_path.clear();
    v.navigation_chunks.clear();
    v.navigation_repath_at = 0;
    true
}

/// Release canonical site reservations, without deleting placed blocks or stock.
pub(crate) fn remove(state: &mut TickState, id: SettlementId) -> bool {
    if state.settlements.get(id).is_none() {
        return true;
    }
    executor::cancel(state, id).is_ok()
}

// The priority table's array type keeps the stack funding cache in sync.
const AUTONOMOUS_KINDS: usize = 8;

/// Residents from which the settlement walls itself in and digs its well.
const FORTIFY_RESIDENTS: usize = 10;

/// Needed authored kinds in priority order. A natural work zone is not an
/// authored level-one building, and housing remains needed until beds suffice.
pub(super) fn needed_kinds(
    record: &SettlementRecord,
    residents: usize,
) -> impl Iterator<Item = BuildingKind> + '_ {
    let has = |kind| {
        record
            .buildings
            .values()
            .any(|building| building.kind == kind && !building.natural_zone)
    };
    let beds: usize = record
        .buildings
        .values()
        .filter(|building| building.kind == BuildingKind::House)
        .map(|building| usize::from(building.level))
        .sum();
    let kinds: [(BuildingKind, bool); AUTONOMOUS_KINDS] = [
        (BuildingKind::Storage, !has(BuildingKind::Storage)),
        // A newborn needs a free bed (the breeding path checks spare beds), so
        // housing is short while no bed is spare, not only when beds run out.
        (BuildingKind::House, beds <= residents),
        (
            BuildingKind::Workshop(Profession::Farmer),
            !has(BuildingKind::Workshop(Profession::Farmer)),
        ),
        (BuildingKind::LumberCamp, !has(BuildingKind::LumberCamp)),
        (BuildingKind::Mine, !has(BuildingKind::Mine)),
        (
            BuildingKind::Workshop(Profession::Builder),
            !has(BuildingKind::Workshop(Profession::Builder)),
        ),
        (
            BuildingKind::Barracks,
            residents >= 6 && !has(BuildingKind::Barracks),
        ),
        (
            BuildingKind::Tower,
            residents >= 8 && !has(BuildingKind::Tower),
        ),
    ];
    kinds
        .into_iter()
        .filter_map(|(kind, needed)| needed.then_some(kind))
}

/// Read the allocator's existing physical index, never certificate levels.
/// Wait for its bounded scans rather than treating an unknown bed as absent.
/// Roofed physical beds inside the settlement, counted up to one more than
/// the residents (only the comparison matters). `None` while the POI cache is
/// not ready: a pass plans nothing on bed data it does not have.
fn roofed_beds(state: &TickState, record: &SettlementRecord) -> Option<usize> {
    let cache = &state.villager_pois;
    if !cache.poi_initialized || !cache.pending_chunks.is_empty() {
        return None;
    }
    let residents = state.settlements.member_count(record.id);
    Some(
        cache
            .chunks
            .values()
            .flat_map(|chunk| &chunk.pois)
            .filter(|poi| {
                poi.kind == super::allocator::PoiKind::Home
                    && record.contains([poi.pos.x, poi.pos.y, poi.pos.z])
                    && super::allocator::bed_pair(&state.world, &state.registry, poi.pos).is_some()
                    && super::allocator::roofed(&state.world, &state.registry, poi.pos)
            })
            .take(residents.saturating_add(1))
            .count(),
    )
}

/// The bill the priority House will need while the village is short of roofed
/// beds. Stock promised here is never spent on an upgrade: runs 2-3 of the
/// final civ series upgraded two mason workshops to L3 and completed Storage
/// L1 while the House lane waited at `oak_planks 4/106` for a whole run, so
/// the House never became funded (receipts/D123/D123CivGates/FINAL-SERIES.md).
/// The House lane itself funds this bill (`priority_house`); an active House
/// project is the spender, not a reservation.
pub(super) fn house_reserve_bill(
    state: &TickState,
    record: &SettlementRecord,
    images: &[super::inventory::Image],
) -> Vec<(u32, u32)> {
    if state.settlements.member_count(record.id) == 0 {
        return Vec::new();
    }
    let Some(beds) = roofed_beds(state, record) else {
        return Vec::new();
    };
    if beds > state.settlements.member_count(record.id) {
        return Vec::new();
    }
    let Some(blueprint) = catalog(state, record).find(|bp| bp.kind == BuildingKind::House) else {
        return Vec::new();
    };
    if access::projects(record).any(|project| project.blueprint == blueprint.id) {
        return Vec::new();
    }
    let Some(level) = blueprint.level(1) else {
        return Vec::new();
    };
    selected_bill(state, record.id, &level.materials, images)
        .map(std::borrow::Cow::into_owned)
        .unwrap_or_default()
}

fn priority_house(
    state: &TickState,
    record: &SettlementRecord,
    images: &[super::inventory::Image],
) -> Option<&'static blueprints::Blueprint> {
    if state.settlements.member_count(record.id) == 0 {
        return None;
    }
    let Some(beds) = roofed_beds(state, record) else {
        let cache = &state.villager_pois;
        tracing::debug!(
            target: "settlements::construction",
            settlement = record.id.0,
            initialized = cache.poi_initialized,
            pending = cache.pending_chunks.len(),
            "priority house scan skipped: poi cache not ready"
        );
        return None;
    };
    let residents = state.settlements.member_count(record.id);
    if beds > residents {
        tracing::debug!(
            target: "settlements::construction",
            settlement = record.id.0,
            beds,
            residents,
            "priority house scan skipped: enough roofed beds"
        );
        return None;
    }
    let blueprint = catalog(state, record).find(|bp| bp.kind == BuildingKind::House)?;
    // Admission already proved this house's funding. Its untouched builder
    // may have hauled part of that stock, so do not re-prove the full bill.
    if access::projects(record)
        .next()
        .is_some_and(|project| project.blueprint == blueprint.id)
    {
        return Some(blueprint);
    }
    let level = blueprint.level(1)?;
    let funded = funded_variant(state, record.id, &level.materials, images).is_some();
    // The House lane is the only path that creates a house project, and it used to
    // fail silently: a settlement with 194 cobblestone and 186 legal House lots sat
    // for 708 planning passes without one (HANDOFF-49). Name the reason.
    tracing::debug!(
        target: "settlements::construction",
        settlement = record.id.0,
        beds,
        residents,
        funded,
        bill = ?level.materials,
        "priority house scan"
    );
    funded.then_some(blueprint)
}

pub(crate) fn sources(state: &TickState, id: SettlementId) -> Vec<[i32; 3]> {
    buildings::storage_points(state, id)
}

pub(crate) fn clearable(state: &TickState, block: u32) -> bool {
    // Clearing is a native-flora veneer only: air plus the vanilla placement-
    // replaceable surface plants below. Fluids, paths, buildings, torches,
    // flowers and paid farm crops stay blocking so a wider clear can never
    // wash or overwrite foreign/unsafe content, and an upgrade diff that
    // omits unchanged L1 fields never loses paid seeds.
    if block == state.registry.states.air {
        return true;
    }
    state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(block))
        .is_some_and(|definition| {
            !definition.has_fluid()
                && matches!(
                    definition.block.id.path(),
                    "short_grass"
                        | "tall_grass"
                        | "short_dry_grass"
                        | "tall_dry_grass"
                        | "fern"
                        | "large_fern"
                        | "dead_bush"
                        | "bush"
                        | "vine"
                        | "glow_lichen"
                        | "resin_clump"
                        | "leaf_litter"
                        | "hanging_roots"
                        | "warped_roots"
                        | "nether_sprouts"
                        | "crimson_roots"
                        | "fire"
                        | "soul_fire"
                        | "snow"
                        | "void_air"
                        | "cave_air"
                )
        })
}

/// Only natural terrain can be excavated; buildings, paths and fluids are not
/// site preparation materials.
pub(crate) fn excavatable(state: &TickState, block: u32) -> bool {
    state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(block))
        .is_some_and(|definition| {
            !definition.has_fluid()
                && matches!(
                    definition.block.id.path(),
                    "dirt"
                        | "grass_block"
                        | "coarse_dirt"
                        | "rooted_dirt"
                        | "podzol"
                        | "mycelium"
                        | "stone"
                        | "granite"
                        | "diorite"
                        | "andesite"
                        | "deepslate"
                        | "sand"
                        | "red_sand"
                        | "gravel"
                        | "clay"
                )
        })
}

/// Floor replacement permits solid ground or clearable fill space, but never
/// fluids, paths, paid crops or authored non-ground blocks under an overhang.
fn floor_replaceable(state: &TickState, block: u32) -> bool {
    clearable(state, block)
        || (state
            .registry
            .collision_boxes
            .get(block as usize)
            .is_some_and(|boxes| !boxes.is_empty())
            && state
                .registry
                .blocks
                .by_id(mc_world::BlockStateId(block))
                .is_some_and(|definition| {
                    !definition.has_fluid()
                        && !matches!(definition.block.id.path(), "dirt_path" | "farmland")
                }))
}

/// Check the whole future footprint, not merely this level's placed cells.
/// One candidate per planning admission; never load chunks or clear paths.
pub(super) fn site_free(
    state: &TickState,
    record: &SettlementRecord,
    origin: [i32; 3],
    size: [u8; 3],
) -> bool {
    site_free_claim(
        state,
        record,
        origin,
        size,
        record.founding.as_ref(),
        &mut Default::default(),
    )
}

fn site_free_claim(
    state: &TickState,
    record: &SettlementRecord,
    origin: [i32; 3],
    size: [u8; 3],
    expedition: Option<&FoundingExpedition>,
    survey: &mut site_refusals::Survey,
) -> bool {
    site_refusals::check(state, record, origin, size, expedition, survey, None).is_ok()
}

/// Rewrite the already-funded authored cells, not the stored project later.
fn apply_species(
    state: &TickState,
    cells: &mut [executor::PlannedCell],
    species: &str,
) -> Option<()> {
    if species == "oak" {
        return Some(());
    }
    for cell in cells {
        let original = state
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(cell.state))?;
        if original.block.id.as_str().starts_with("minecraft:")
            && let Some(suffix) = original.block.id.path().strip_prefix("oak_")
        {
            let name = mc_data::Identifier::parse(format!("minecraft:{species}_{suffix}")).ok()?;
            cell.state = state
                .registry
                .blocks
                .by_name_and_props(&name, &original.properties)?
                .0;
        }
        if let Some(resource) = &mut cell.resource {
            let item = state
                .registry
                .items
                .id_of(&mc_data::Identifier::parse(resource.as_str()).ok()?)?;
            let mapped = species_item(state, item, species)?;
            if mapped != item {
                *resource = state.registry.items.name_of(mapped)?.to_string();
            }
        }
    }
    Some(())
}

/// Both bed halves and the paid foot use the same selected color.
fn apply_bed_color(
    state: &TickState,
    cells: &mut [executor::PlannedCell],
    color: u8,
) -> Option<()> {
    if color == 0 {
        return Some(());
    }
    let color = crate::blueprint::DYE_COLORS.get(usize::from(color))?;
    let name = mc_data::Identifier::parse(format!("minecraft:{color}_bed")).ok()?;
    for cell in cells {
        let original = state
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(cell.state))?;
        if original.block.id.as_str() == "minecraft:white_bed" {
            cell.state = state
                .registry
                .blocks
                .by_name_and_props(&name, &original.properties)?
                .0;
        }
        if cell.resource.as_deref() == Some("minecraft:white_bed") {
            cell.resource = Some(name.to_string());
        }
    }
    Some(())
}

fn cell_bill(state: &TickState, cells: &[executor::PlannedCell]) -> Option<Vec<(u32, u32)>> {
    let mut bill = std::collections::BTreeMap::new();
    for cell in cells {
        if let Some(resource) = &cell.resource {
            let item = state
                .registry
                .items
                .id_of(&mc_data::Identifier::parse(resource).ok()?)?;
            *bill.entry(item).or_insert(0) += cell.quantity;
        }
    }
    Some(bill.into_iter().collect())
}

fn begin(state: &mut TickState, project: Project, species: &str, color: u8) -> bool {
    let Ok((bounds, mut cells)) = planned_cells(state, &project, species, &mut Default::default())
    else {
        return false;
    };
    let images = executor::source_images(state, project.settlement);
    filler::apply_filler(state, &project, &mut cells, &images);
    if apply_bed_color(state, &mut cells, color).is_none() {
        return false;
    }
    begin_cells(state, project, bounds, cells)
}

pub(crate) fn begin_cells(
    state: &mut TickState,
    mut project: Project,
    bounds: ScriptWorldBounds,
    mut cells: Vec<executor::PlannedCell>,
) -> bool {
    if !cells.is_sorted_by_key(|cell| cell.portion) {
        cells.sort_by_key(|cell| cell.portion);
    }
    let site = blueprints::get(&state.registry, &project.blueprint)
        .map(|blueprint| {
            let size = blueprint.footprint(Rotation::from_quarter_turns(project.rotation));
            ScriptWorldBounds::new(
                [
                    project.origin[0],
                    bounds.min[1].min(project.origin[1]),
                    project.origin[2],
                ],
                std::array::from_fn(|axis| project.origin[axis] + i32::from(size[axis]) - 1),
            )
            .expect("authored lot bounds")
        })
        .unwrap_or(bounds);
    let Ok(handle) = executor::prepare(
        state,
        project.settlement,
        project.blueprint.clone(),
        bounds,
        site,
        cells,
    ) else {
        return false;
    };
    project.structure = handle;
    let record = state
        .settlements
        .get_mut(project.settlement)
        .expect("planned settlement");
    access::push_project(record, project);
    record.revision = record.revision.saturating_add(1);
    true
}

/// Blueprint, origin and quarter turns of a blueprint-built building's
/// current level. Natural zones and player-authored certificates are not
/// blueprint builds: the certificate must be exactly a rotated level box at a
/// committed site.
pub(super) fn placement<'a>(
    state: &'a TickState,
    id: SettlementId,
    building: &buildings::Building,
) -> Option<(&'a blueprints::Blueprint, [i32; 3], u8)> {
    if building.natural_zone {
        return None;
    }
    blueprints::all(&state.registry)
        .iter()
        .filter(|blueprint| blueprint.kind == building.kind)
        .find_map(|blueprint| {
            let current = blueprint.level(building.level)?;
            Rotation::ALL.into_iter().find_map(|rotation| {
                let rotated = current.rotated(rotation);
                let corner = rotated.bounds([0, 0, 0]).min;
                let origin: [i32; 3] =
                    std::array::from_fn(|axis| building.bounds.min[axis] - corner[axis]);
                (rotated.bounds(origin) == building.bounds
                    && executor::built_at(state, id, &blueprint.id, origin))
                .then_some((blueprint, origin, rotation.0))
            })
        })
}

/// The teaser's faction is the existing owner UUID. Unowned towns never share
/// a culture merely because both owners are absent. Only completed native
/// construction establishes style; pending orders and player-painted walls do not.
fn spanish_faction(state: &TickState, record: &SettlementRecord) -> bool {
    state.settlements.records().any(|town| {
        (town.id == record.id || (record.owner.is_some() && town.owner == record.owner))
            && town.buildings.values().any(|building| {
                placement(state, town.id, building)
                    .is_some_and(|(blueprint, ..)| blueprints::spanish(&blueprint.id))
            })
    })
}

fn catalog(
    state: &TickState,
    record: &SettlementRecord,
) -> impl Iterator<Item = &'static blueprints::Blueprint> + Clone + use<> {
    storage::autonomous_catalog(state, record, spanish_faction(state, record))
}

/// The next-level blueprint of a blueprint-built building, with the placement
/// origin and quarter turns its committed level-one structure used.
fn upgrade_blueprint<'a>(
    state: &'a TickState,
    id: SettlementId,
    building: &buildings::Building,
) -> Option<(&'a blueprints::Blueprint, [i32; 3], u8)> {
    let placed = placement(state, id, building)?;
    placed.0.level(building.level.checked_add(1)?)?;
    Some(placed)
}

#[cfg(test)]
pub(crate) fn upgrade_available(state: &TickState, id: SettlementId, building: BuildingId) -> bool {
    state
        .settlements
        .get(id)
        .and_then(|record| record.buildings.get(&building))
        .is_some_and(|building| upgrade_blueprint(state, id, building).is_some())
}

/// Lot candidates examined per planning pass, and by an owner order in the
/// tick that admits it.
const SITE_ATTEMPTS: u32 = 64;
/// Ground may differ this much from the bell's elevation (or from a lot's floor)
/// and still be found. A fill column below a floor, and a hillside cut above
/// it, stay within this band: it is also what a builder standing at the lot's
/// perimeter can reach (placement reach 12, margin 2, see `villager::building`).
pub(super) const SITE_BAND: i32 = 8;

/// Highest standing surface in the band around the bell for one column: a solid
/// block with clearable space directly above it. Hillside villages have no
/// flat ground at the bell's own elevation, so sites follow the terrain.
pub(super) fn surface_y(state: &TickState, anchor_y: i32, x: i32, z: i32) -> Option<i32> {
    surface_y_for_lot(
        state,
        anchor_y,
        x,
        z,
        None,
        &mut super::forestry::Clearance::prepared(),
    )
}

pub(super) fn surface_y_for_lot(
    state: &TickState,
    anchor_y: i32,
    x: i32,
    z: i32,
    lot: Option<ScriptWorldBounds>,
    clearance: &mut super::forestry::Clearance,
) -> Option<i32> {
    surface_y_between(
        state,
        anchor_y - SITE_BAND..=anchor_y + SITE_BAND,
        x,
        z,
        lot,
        clearance,
    )
}

pub(super) use preparation::liquid::surface_y_between;

/// Street work for the buildings already standing: the first unpaved road
/// stretch on a walk from the plaza to a door, then a funded lamp post along
/// those walks. Roads cost no materials; a lamp is two fences and a torch.
fn street_site(
    state: &TickState,
    record: &SettlementRecord,
    images: &[super::inventory::Image],
) -> Option<FundedSite> {
    let mut network = layout::Network::walk(state, record);
    let served = network.served();
    let project = |kind: &str, bounds: ScriptWorldBounds, axis: u8| Project {
        settlement: record.id,
        blueprint: kind.to_owned(),
        level: 1,
        origin: bounds.min,
        rotation: axis,
        building: None,
        structure: String::new(),
        ordered: false,
        builder: None,
        order_seq: None,
    };
    for tile in &served {
        if let Some(work) = network.road_work(tile)
            && !executor::area_held(state, work.bounds)
        {
            return Some(FundedSite {
                project: project(layout::ROAD, work.bounds, tile.axis),
                bounds: work.bounds,
                cells: work.cells,
            });
        }
    }
    for tile in &served {
        if let Some(mut work) = network.lamp_work(tile)
            && !executor::area_held(state, work.bounds)
            && let Some(bill) = cell_bill(state, &work.cells)
            // A lamp never funds itself out of an active project's materials.
            && jointly_fundable(state, record.id, &bill)
            && let Some(species) = select_species(state, &bill, images, false)
            && apply_species(state, &mut work.cells, species).is_some()
        {
            return Some(FundedSite {
                project: project(layout::LAMP, work.bounds, 0),
                bounds: work.bounds,
                cells: work.cells,
            });
        }
    }
    None
}

/// One planner pass with a fresh tick's whole site budget (test driver).
#[cfg(test)]
fn plan(state: &mut TickState, id: SettlementId) {
    plan_pass(state, id, &mut site_turn::SiteBudget::default());
}

/// The settlement's planner pass; its site slices come from `budget`. True
/// when it opened an autonomous project past the builder gate.
fn plan_pass(state: &mut TickState, id: SettlementId, budget: &mut site_turn::SiteBudget) -> bool {
    // Sampled under `MC_CORE_PHASE_LOG` only: one record per town per minute of
    // simulation, carrying the verdict of the pass that made it.
    if decision_log::enabled()
        && state.settlements.get(id).is_some()
        && state
            .settlements
            .plan_log
            .entry(id)
            .or_default()
            .claim(state.tick)
    {
        let mut diagnostic = decision_log::Plan::default();
        let started = plan_pass_observed(state, id, budget, Some(&mut diagnostic));
        diagnostic.emit(
            id.0,
            state.tick,
            state.tick_control.game_ticks(state.tick),
            started,
        );
        return started;
    }
    plan_pass_observed(state, id, budget, None)
}

/// The planner pass itself, with the branch that decided written to
/// `diagnostic` when one is sampled.
fn plan_pass_observed(
    state: &mut TickState,
    id: SettlementId,
    budget: &mut site_turn::SiteBudget,
    mut diagnostic: Option<&mut decision_log::Plan>,
) -> bool {
    macro_rules! finish {
        ($verdict:literal, $result:expr) => {{
            if let Some(log) = diagnostic {
                log.verdict = $verdict;
            }
            return $result;
        }};
    }
    // The settlement's own admission pass owns its stepping flag, and every
    // early return below (a head order, the storehouse, the founding handover,
    // the builder gate, no residents) must leave it cleared: a pass that cannot
    // scan must not leave a walk stepped off the clock (review round 2 N1). The
    // end of the scan sets it again when the pass leaves a holding walk.
    if let Some(sweeps) = state.settlements.site_sweep.get_mut(&id) {
        sweeps.stepping = false;
    }
    // Pending orders permit an existing expedition's handover or cancellation,
    // never admission or construction of another outpost.
    if state
        .settlements
        .get(id)
        .is_some_and(|record| access::head_order(record).is_some())
        && founding::plan(state, id)
    {
        finish!("ordered_founding", false);
    }
    // With a pending order, iron_demand considers only that order's bill.
    if furnace::plan(state, id) {
        finish!("furnace", false);
    }
    if plan_order(state, id, budget) {
        finish!("owner_order", false);
    }
    let images = executor::source_images(state, id);
    if let Some(started) = storage::plan(state, id, budget, &images, diagnostic.as_deref_mut()) {
        return started;
    }
    if founding::plan(state, id) {
        finish!("founding", false);
    }
    let Some(record) = state.settlements.get(id) else {
        finish!("missing_town", false);
    };
    // Autonomous work opens a site only when every active project has its
    // builder and a resident could take one more: a site nobody can build only
    // parks a slot and a lot and holds the planner (Spain seed 100: eight
    // projects for three builders; later a fourth house unbound for 30k ticks).
    if claim_slot(state, id).is_err() {
        finish!("no_slot", false);
    }
    if unbound_project(&state.settlements, id).is_some() {
        finish!("unbound", false);
    }
    if !builder_free(state, id) {
        finish!("no_builder", false);
    }
    let residents = state.settlements.member_count(id);
    if residents == 0 {
        finish!("no_residents", false);
    }
    // Keep the priority material worker out of optional street work, not out
    // of the entire planner: another resident can build a funded later kind.
    let gathering = gatherer_demand(state, record, &images)
        .is_some_and(|profession| has_staffed_gatherer(state, id, profession));
    // Housing gets its own bounded scan. If it has no site, ordinary funded
    // candidates still get their shared scan in this same planning pass. Both
    // read and extend the same runtime-only sweep scratch.
    let mut sweeps = state.settlements.site_sweep.remove(&id).unwrap_or_default();
    let scan = {
        let record = state.settlements.get(id).expect("planned settlement");
        scan_sites(
            state,
            id,
            record,
            &images,
            &mut sweeps,
            budget,
            diagnostic.as_deref_mut(),
            None,
        )
    };
    // Only a pass that ended with no candidate while a walk it advanced still
    // holds a legal lot leaves the settlement stepping (review round 3 C3).
    sweeps.stepping = scan.waiting
        && scan.house.is_none()
        && scan.funded.as_ref().is_none_or(|pass| pass.site.is_none());
    let (house, house_cursor, funded_site, search_running, search_waiting) = (
        scan.house,
        scan.cursor,
        scan.funded,
        scan.running,
        scan.waiting,
    );
    state.settlements.site_sweep.insert(id, sweeps);
    if let Some(site) = house {
        finish!(
            "priority_house_site",
            begin_cells(state, site.project, site.bounds, site.cells)
        );
    }
    let funded_kinds = funded_site.is_some();
    if let Some(pass) = funded_site {
        let record = state.settlements.get_mut(id).expect("planned settlement");
        record.construction_cursor = pass.cursor;
        record.revision = record.revision.saturating_add(1);
        if let Some(site) = pass.site {
            finish!(
                "ordinary_l1_site",
                begin_cells(state, site.project, site.bounds, site.cells)
            );
        }
    }
    // A kind's sweep is still running and already holds a legal candidate: its
    // cheapest lot is decided only after the last one, so the slot waits. An
    // unfinished sweep with nothing legal yet must not hold it.
    if search_running && search_waiting {
        if !funded_kinds {
            let record = state.settlements.get_mut(id).expect("planned settlement");
            if record.construction_cursor != house_cursor {
                record.construction_cursor = house_cursor;
                record.revision = record.revision.saturating_add(1);
            }
        }
        finish!("sweep_waiting", false);
    }
    let cursor = house_cursor;
    // A funded new building may have no legal lot. Existing houses must still
    // be able to gain beds through their reserved upgrade footprint.
    let record = state.settlements.get(id).expect("planned settlement");
    // Upgrade admission policy (docs settlements/construction.md): an upgrade
    // opens only from an idle planner (`projects_of(..).next().is_none()`), only
    // when its whole chosen bill is already in store - netted of what other
    // active projects are promised (`upgrade_project`, stock-only) - and only
    // when a builder could take it **without borrowing an extraction worker**
    // (`upgrade_staffable`, the same rule `assign_builder` applies to an
    // upgrade). Otherwise a one-worker village opens an upgrade that stays
    // unbound for good and the gate above refuses everything (review B1).
    if access::projects_of(&state.settlements, id).next().is_none()
        && builders::upgrade_staffable(state, id)
        && let Some((project, species, color)) = preparation::upgrade_project_observed(
            state,
            record,
            &images,
            diagnostic.as_deref_mut().map(|log| &mut log.upgrades),
        )
    {
        let started = begin(state, project, &species, color);
        if let Some(log) = diagnostic.as_deref_mut()
            && log.upgrades.count != 0
        {
            let last = log.upgrades.count - 1;
            log.upgrades.funded[last].1 = if started { "started" } else { "begin_refused" };
        }
        if started {
            finish!("upgrade", true);
        }
    }
    if !funded_kinds {
        let record = state.settlements.get_mut(id).expect("planned settlement");
        if record.construction_cursor != cursor {
            record.construction_cursor = cursor;
            record.revision = record.revision.saturating_add(1);
        }
    }
    // The first Farm's seeds are gathered even while a priority material
    // worker is out: `assign_builder` keeps that worker out of this project,
    // and another resident clears the meadow.
    // The F12 ordering below rests on `a_demanded_staffed_lumberjack_does_not_
    // hold_the_fortification` (red with an empty start list before the order
    // changed) and the gate fixture's miner-lane log — not on the 1580 s run of
    // 2026-11-24, whose wall scan read a 25-column ring, nor on its 768
    // cobblestone, which was the deposit. The comment inside that arm is kept
    // verbatim because it is the control patch's `+` side (review wave 2,
    // minor 4).
    let Some(record) = state.settlements.get(id) else {
        finish!("missing_town_after_upgrade", false);
    };
    if let Some(site) = meadow_site(state, record, residents, &images) {
        finish!(
            "meadow",
            begin_cells(state, site.project, site.bounds, site.cells)
        );
    }
    // The wall, its gates and the well come before street work: the streets
    // reach every standing building in the radius and owe work without end,
    // the fortification is a bounded list that the world itself shortens. A
    // staffed gatherer holds back the streets only, never the fortification:
    // that list is bounded and funded, and another resident can build it. The
    // 2026-11-24 perimeter gate stood still for 1580 s with a staffed Miner (a
    // certified natural Mine) while 768 cobblestone waited in storage.
    if let Some(site) = fortification_site(state, record, residents, &images) {
        finish!(
            "infrastructure",
            begin_cells(state, site.project, site.bounds, site.cells)
        );
    }
    if gathering {
        finish!("gathering", false);
    }
    let Some(site) = street_site(state, record, &images) else {
        finish!("no_admissible_project", false);
    };
    finish!(
        "infrastructure",
        begin_cells(state, site.project, site.bounds, site.cells)
    )
}
/// The next funded piece of the perimeter wall, its gates or the well, from
/// [`FORTIFY_RESIDENTS`] residents. Each wall stretch, gatehouse or well is a
/// project of its own, bought from real stock like a lamp post; the world, not
/// a saved plan, says what is already standing.
fn fortification_site(
    state: &TickState,
    record: &SettlementRecord,
    residents: usize,
    images: &[super::inventory::Image],
) -> Option<FundedSite> {
    if residents < FORTIFY_RESIDENTS {
        return None;
    }
    for (kind, mut work) in layout::fortification(state, record) {
        if executor::area_held(state, work.bounds) {
            continue;
        }
        let Some(bill) = cell_bill(state, &work.cells) else {
            continue;
        };
        // A wall never funds itself out of an active project's materials.
        if !jointly_fundable(state, record.id, &bill) {
            continue;
        }
        let Some(species) = select_species(state, &bill, images, false) else {
            continue;
        };
        if apply_species(state, &mut work.cells, species).is_none() {
            continue;
        }
        return Some(FundedSite {
            project: Project {
                settlement: record.id,
                blueprint: kind.to_owned(),
                level: 1,
                origin: work.bounds.min,
                rotation: 0,
                building: None,
                structure: String::new(),
                ordered: false,
                builder: None,
                order_seq: None,
            },
            bounds: work.bounds,
            cells: work.cells,
        });
    }
    None
}

/// The first Farm's wheat cells cost seeds that only a farm produces. While
/// the stock is short of them, Builders clear seed-bearing grass near the bell
/// (vanilla: short grass drops a wheat seed one time in eight), the same way
/// they clear any plant: free work, loot credited to storage.
fn meadow_site(
    state: &TickState,
    record: &SettlementRecord,
    residents: usize,
    images: &[super::inventory::Image],
) -> Option<FundedSite> {
    let farmer = BuildingKind::Workshop(Profession::Farmer);
    if !needed_kinds(record, residents).any(|kind| kind == farmer) {
        return None;
    }
    let is_seed = |item: u32| {
        state
            .registry
            .items
            .name_of(item)
            .is_some_and(|name| matches!(name.path(), "wheat_seeds"))
    };
    let mut catalog = catalog(state, record);
    let level = catalog
        .find(|blueprint| blueprint.kind == farmer)?
        .level(1)?;
    let need: u64 = level
        .materials
        .iter()
        .filter(|(item, _)| is_seed(*item))
        .map(|(_, count)| u64::from(*count))
        .sum();
    let have: u64 = images
        .iter()
        .flat_map(|image| &image.slots)
        .filter(|stack| is_seed(stack.item_id))
        .map(|stack| stack.count.max(0) as u64)
        .sum();
    if have >= need {
        return None;
    }
    let work = layout::meadow_work(state, record)?;
    Some(FundedSite {
        project: Project {
            settlement: record.id,
            blueprint: layout::MEADOW.to_owned(),
            level: 1,
            origin: work.bounds.min,
            rotation: 0,
            building: None,
            structure: String::new(),
            ordered: false,
            builder: None,
            order_seq: None,
        },
        bounds: work.bounds,
        cells: work.cells,
    })
}

/// Living settlement members, including builders travelling to remote outposts.
/// The derived occupancy index keeps this bounded and independent of position.
pub(crate) fn residents(
    state: &TickState,
    id: SettlementId,
) -> impl Iterator<Item = crate::entities::EntitySlot> + '_ {
    state
        .settlements
        .member_ids(id)
        .take(1024)
        .filter_map(|entity| state.entities.slot_of(entity))
}

/// Builders fetch every block's material from storage. A resident that cannot
/// walk there (shut in by terrain, e.g. a house door blocked by a tree crown)
/// would hold the single builder slot without ever building.
fn reaches_storage(
    state: &TickState,
    id: SettlementId,
    slot: crate::entities::EntitySlot,
    cache: &mut super::allocator::PoiCache,
    path: &mut Vec<crate::boundary::BlockPos>,
) -> bool {
    let Some(chest) = buildings::storage_points(state, id).first().copied() else {
        return true;
    };
    let target = crate::boundary::BlockPos {
        x: chest[0],
        y: chest[1],
        z: chest[2],
    };
    let pos = state.entities.pos(slot);
    let body = state.entities.body(slot);
    let (width, height) = (body.half_width, body.height);
    let from = super::allocator::at(pos);
    let horizontal = from
        .x
        .abs_diff(target.x)
        .max(from.z.abs_diff(target.z))
        .saturating_add(8)
        .clamp(48, 512);
    let vertical = from.y.abs_diff(target.y).saturating_add(4).clamp(16, 512);
    // A one-shot admission probe, never a per-tick allocation: villagers and the
    // allocator routinely spend the whole shared route allowance before the
    // planning pass runs, which refused every candidate and left the project
    // without a builder. A fresh allowance bounds each probe at its own search.
    let mut budget = crate::tick_budget::Budget::default();
    let mut ctx = super::allocator::RouteCtx {
        world: &state.world,
        registry: &state.registry,
        dimension: state.dimension,
        consumer: crate::tick_budget::Consumer::Allocator,
        budget: &mut budget,
    };
    ctx.admit()
        && super::allocator::find_path_ctx(
            &mut ctx, cache, pos, target, width, height, path, false, horizontal, vertical, true,
        )
}

/// The slot of the resident bound to `project`, while it is still a living
/// Builder of this settlement. A project's own binding is the only incumbent
/// (D8): another project never stands in for it.
pub(crate) fn incumbent_slot(
    state: &TickState,
    id: SettlementId,
    project: &Project,
) -> Option<crate::entities::EntitySlot> {
    let (uuid, _) = project.builder?;
    // A dead, converting, foreign or re-assigned resident is not an incumbent:
    // the same predicate that clears the binding decides it (D8 review R2-1).
    incumbent_is_live(state, id, uuid)
        .then(|| state.entities.slot_of_uuid(uuid))
        .flatten()
}

/// Release a bound builder outside simulation, or an on-shift builder that
/// cannot walk to storage, so the planner can bind another resident to that
/// project. Returns true while every active project keeps an incumbent.
fn release_trapped_builder(state: &mut TickState, id: SettlementId) -> bool {
    let bound: Vec<(String, uuid::Uuid)> = access::projects_of(&state.settlements, id)
        .filter_map(|project| {
            project
                .builder
                .map(|(uuid, _)| (project.structure.clone(), uuid))
        })
        .collect();
    // An unbound project is work for assignment, never a held settlement.
    let unbound =
        access::projects_of(&state.settlements, id).any(|project| project.builder.is_none());
    let mut held = !unbound;
    for (handle, _uuid) in bound {
        let Some(project) = state
            .settlements
            .get(id)
            .and_then(|record| access::project_for_structure(record, &handle))
            .cloned()
        else {
            continue;
        };
        let Some(slot) = incumbent_slot(state, id, &project) else {
            // A dead, converting or re-assigned resident holds nothing. The
            // binding is cleared by rebind_lost_builders, which returns its
            // carry first (D8 review R2-1); clearing it here would drop the
            // load with the binding.
            held = false;
            continue;
        };
        let on_shift = state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .is_some_and(|v| crate::villager::building::working(state, v));
        let mut cache = super::allocator::PoiCache::default();
        let mut path = Vec::new();
        if resident_simulated(state, slot)
            && (!on_shift || probe_storage(state, id, slot, &mut cache, &mut path))
        {
            continue;
        }
        if release_incumbent(state, id, &handle, slot) {
            held = false;
        }
    }
    held
}

/// Detach the builder bound to `handle` from that project, returning a borrowed
/// gatherer to its profession. Its load of the project's materials becomes a
/// recorded return duty first, paid by the next pass (D8 review R4-1): another
/// builder may finish the project from storage meanwhile. Only that project's
/// own binding is released (D8).
fn release_incumbent(
    state: &mut TickState,
    id: SettlementId,
    handle: &str,
    slot: crate::entities::EntitySlot,
) -> bool {
    let borrowed = state
        .settlements
        .get(id)
        .and_then(|record| access::project_for_structure(record, handle))
        .and_then(|project| project.builder)
        .filter(|(uuid, _)| *uuid == state.entities.row(slot).uuid);
    let released = detach(
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .expect("builder incumbent"),
    );
    if released && let Some((uuid, profession)) = borrowed {
        owe_carry(state, id, uuid, handle);
        if let Some(profession) = profession {
            state
                .entities
                .row_mut(slot)
                .villager
                .as_mut()
                .unwrap()
                .data
                .profession = profession;
        }
        let record = state.settlements.get_mut(id).unwrap();
        access::unbind_builder(record, uuid);
        record.revision = record.revision.saturating_add(1);
    }
    crate::villager::sync_appearance(state, slot);
    released
}

/// Builders work only 2000..9000 of the day. A project whose work counter
/// stands still for this many simulated shift ticks (150 s of game time) has a
/// builder that cannot make progress (seed 24: a stationary zombie within eight
/// blocks of the only road to the lot kept the builder fleeing and returning
/// for 15,000 ticks, 35 of 239 cells, and held the single project slot).
/// Frozen ticks are not shift time: only simulated ticks move villagers
/// (tick.rs runs settlements and brains inside `if sim`), so a held world, and
/// the retention gate's between-sprint observation, add no window.
const BUILDER_STALL_SHIFT_TICKS: u64 = 3_000;
/// The replaced builder is chosen last for this many simulated ticks.
const BUILDER_SHUN_TICKS: u64 = 24_000;
/// Longest gap between two planning passes counted as shift time.
const BUILDER_WATCH_STEP: u64 = 100;

/// Runtime watch of the active project's work counter; derived, never persisted.
#[derive(Debug)]
pub(crate) struct BuilderWatch {
    progress: (String, u32, u64),
    /// Last simulated tick seen (`game_ticks`, not the global tick or the
    /// day clock).
    seen: u64,
    /// Simulated shift ticks without progress.
    idle: u64,
    reported: bool,
    refusal: Option<&'static str>,
    /// First recovery outcome, consumed by this episode's repeated-stall report.
    recovery: Option<&'static str>,
    /// Replaced builders and the simulated tick until which they are chosen
    /// last.
    shunned: Vec<(uuid::Uuid, u64)>,
    /// Some builder was bound to the project during this episode: only a site
    /// a builder failed is given back for another.
    bound: bool,
    /// The progress of the episode whose owner notice was settled (Sol S-2).
    told: Option<(String, u32, u64)>,
}

/// Drop a binding whose villager died, left, converted or stopped being a
/// Builder. The project keeps its slot and is rebound on a later pass, and the
/// loser's load goes back to storage before the binding is cleared.
fn rebind_lost_builders(state: &mut TickState, id: SettlementId) {
    let lost: Vec<(String, uuid::Uuid)> = access::projects_of(&state.settlements, id)
        .filter_map(|project| {
            project
                .builder
                .map(|(uuid, _)| (project.structure.clone(), uuid))
        })
        .filter(|(_, uuid)| !live_builder(state, id, *uuid))
        .collect();
    for (handle, uuid) in lost {
        // The slot is freed at once: the load's return is a recorded duty, so a
        // dead or converting body never holds the project and a delivery that
        // cannot happen now is retried by the construction pass after a cure
        // (D8 review SOL-6/R3-1, R4-1).
        if executor::return_carry_of(state, id, uuid) != executor::CarryReturn::Delivered {
            owe_carry(state, id, uuid, &handle);
        }
        let Some(record) = state.settlements.get_mut(id) else {
            return;
        };
        access::unbind_builder(record, uuid);
        record.revision = record.revision.saturating_add(1);
        let _ = handle;
    }
}

/// A bound villager is still a live Builder of this settlement, unassigned to
/// another job: the one predicate for both holding and clearing a binding.
fn incumbent_is_live(state: &TickState, id: SettlementId, uuid: uuid::Uuid) -> bool {
    live_builder(state, id, uuid)
        && state.entities.slot_of_uuid(uuid).is_some_and(|slot| {
            state
                .entities
                .row(slot)
                .villager
                .as_ref()
                .is_some_and(|v| v.age >= 0)
                && !crate::settlements::economy::worker_reserved(state, Some(id), uuid)
        })
}

/// A bound villager is still a living Builder of this settlement.
fn live_builder(state: &TickState, id: SettlementId, uuid: uuid::Uuid) -> bool {
    state.entities.slot_of_uuid(uuid).is_some_and(|slot| {
        let row = state.entities.row(slot);
        let body = state.entities.body(slot);
        body.alive
            && row.villager.as_ref().is_some_and(|v| {
                v.settlement == Some(id)
                    && v.data.profession == Profession::Builder
                    && v.conversion_time.is_none()
            })
    })
}

/// Record that `project` released `uuid`: it is chosen last for
/// [`BUILDER_SHUN_TICKS`] simulated ticks, so recovery rotates the builder
/// instead of rebinding the one that just failed. `game` is
/// `tick_control::game_ticks`, the same monotonic clock the window uses.
pub(crate) fn shun(
    state: &mut TickState,
    id: SettlementId,
    handle: &str,
    uuid: uuid::Uuid,
    game: u64,
) {
    if let Some(watch) = state
        .settlements
        .builder_watch
        .get_mut(&(id, handle.to_owned()))
    {
        watch.shunned.retain(|&(_, until)| game < until);
        watch
            .shunned
            .push((uuid, game.saturating_add(BUILDER_SHUN_TICKS)));
    }
}

/// Replace the builder of a running project that has not advanced a cell for
/// [`BUILDER_STALL_SHIFT_TICKS`] of shift: another resident takes it, whose
/// route may not cross whatever stopped the first.
fn release_stalled_builder(state: &mut TickState, id: SettlementId) {
    let handles: Vec<String> = access::projects_of(&state.settlements, id)
        .map(|project| project.structure.clone())
        .collect();
    if handles.is_empty() {
        state
            .settlements
            .builder_watch
            .retain(|(settlement, _), _| *settlement != id);
        return;
    }
    for handle in handles {
        let Some(project) = state
            .settlements
            .get(id)
            .and_then(|record| access::project_for_structure(record, &handle))
            .cloned()
        else {
            continue;
        };
        // A cancelled or finished project takes the rest of the pass with it.
        if watch_project(state, id, &project) {
            break;
        }
    }
}

/// The per-project stall episode: progress, refusal and recovery are watched
/// separately for every active project (D8).
fn watch_project(state: &mut TickState, id: SettlementId, project: &Project) -> bool {
    let Ok(snapshot) = executor::status(state, id, &project.structure) else {
        return false;
    };
    self::preparation::tell_provenance_wait(state, id, project, &snapshot);
    let progress = (
        project.structure.clone(),
        snapshot.current_stage_index,
        snapshot.completed_work_units,
    );
    // Shift time is simulated time: only simulated ticks move villagers
    // (tick.rs runs settlements and brains inside `if sim`). Counting
    // `state.tick` here expired the 3,000-tick window in ~190 shift ticks of
    // the R7 gate and released a builder before a 58-block walk could finish.
    // The monotonic simulation clock is `game_ticks`, not `day_time`: the
    // latter holds under a disabled daylight cycle and jumps with
    // `/time set|add`, so it decides only the shift phase.
    let game = state.tick_control.game_ticks(state.tick);
    let on_shift = (2_000..9_000).contains(&state.day_time.rem_euclid(24_000));
    let watch = state
        .settlements
        .builder_watch
        .entry((id, project.structure.clone()))
        .or_insert_with(|| BuilderWatch {
            progress: progress.clone(),
            seen: game,
            idle: 0,
            reported: false,
            refusal: None,
            recovery: None,
            shunned: Vec::new(),
            bound: false,
            told: None,
        });
    let elapsed = game.saturating_sub(watch.seen).min(BUILDER_WATCH_STEP);
    watch.seen = game;
    if watch.progress != progress {
        watch.progress = progress;
        watch.idle = 0;
        watch.reported = false;
        watch.refusal = None;
        watch.recovery = None;
        watch.bound = project.builder.is_some();
        return false;
    }
    watch.bound |= project.builder.is_some();
    if !on_shift {
        return false;
    }
    watch.idle += elapsed;
    if watch.idle < BUILDER_STALL_SHIFT_TICKS {
        return false;
    }
    watch.idle = 0;
    let report = !watch.reported;
    watch.reported = true;
    let (refusal, bound) = (watch.refusal, watch.bound);
    let first_recovery = if report { None } else { watch.recovery.take() };
    // The incumbent is the resident bound to this project, never another
    // project's builder standing first in the resident list (D8).
    let incumbent = incumbent_slot(state, id, project);
    let next = executor::next_cell(state, id, &project.structure)
        .ok()
        .flatten();
    if next.is_none()
        && executor::status(state, id, &project.structure).is_ok_and(|status| {
            status.state == ScriptStructureState::Committed
                && status.current_stage_index as usize >= status.stages.len()
        })
        && let Some(missing) = certificate_missing(state, project)
    {
        // Pure query only: naming the missing requirement must not disturb the settlement (calling
        // `finish_project` here certified the L1 house and broke the L2 admission, 20261125T080841).
        tracing::warn!(
            settlement = id.0,
            blueprint = %project.blueprint,
            level = project.level,
            cause = "certificate_incomplete",
            missing = %missing,
            "watch_project: committed structure cannot certify"
        );
    }
    let lot = super::forestry::lot_bounds(state, project);
    let protected = snapshot
        .stages
        .get(snapshot.current_stage_index as usize)
        .is_some_and(|stage| executor::PortionKind::is_preparation(&stage.stage))
        && next
            .zip(state.settlements.get(id))
            .is_some_and(|(at, record)| super::forestry::stalls_clearing(state, record, at, lot));
    let cause = if protected {
        "protected_tree_in_preparation"
    } else if let Some(reason) = snapshot.pause_reason.as_deref() {
        reason
    } else if let Some((slot, next)) = incumbent.zip(next) {
        let waiting = crate::villager::building::wait_reason(state, slot, project, next);
        if waiting == "placement_not_advancing" {
            refusal.unwrap_or(waiting)
        } else {
            waiting
        }
    } else if incumbent.is_some() {
        "pending_cell_unavailable"
    } else {
        "no_builder"
    };
    // Capture the stalled incumbent before recovery clears its navigation and
    // profession. Material diagnostics use the same source selection as work.
    let material = report
        .then(|| {
            executor::next_material(state, id, &project.structure)
                .ok()
                .flatten()
        })
        .flatten();
    let builder = incumbent.map(|slot| state.entities.row(slot).uuid);
    let position = incumbent.map(|slot| state.entities.pos(slot));
    let villager = incumbent.and_then(|slot| state.entities.row(slot).villager.as_deref());
    let carried = material.zip(villager).map(|((item, _), v)| {
        v.inventory
            .iter()
            .flatten()
            .filter(|stack| stack.item_id == item && stack.count > 0)
            .map(|stack| stack.count as u32)
            .sum::<u32>()
    });
    let pickup = material
        .zip(position)
        .and_then(|((item, _), pos)| executor::pickup_point(state, id, item, pos));
    let navigation = villager.and_then(|v| v.navigation_target);
    let navigation_failed = villager.map(|v| v.navigation_search_failed);
    // A failed first recovery (another window without a cell) gives back the
    // slot of unpaid work, as a protected tree does: rotating builders that
    // cannot reach the worksite would otherwise hold the only slot forever.
    // Only a site a builder failed is given back: a project nobody was bound
    // to in this episode keeps its site, because another lot has no builder
    // either and a re-sited order only lets autonomous work take its lot
    // (Spain seed 100: the church re-sited twice with no builder, then
    // site_refused 587/587 and the order was lost).
    let preparation = ((protected || (!report && bound)) && snapshot.consumed.is_empty())
        .then(|| retry_preparation(state, id, project));
    // A retry the queue refused is the owner's to act on: the paid request is
    // still in flight, so the reason reaches them instead of only the log
    // (D8 review R2-6).
    if let Some(Err(error)) = preparation.as_ref() {
        notify_owner(
            state,
            id,
            format!("{} could not be retried: {error}", project.blueprint),
        );
    }
    let recovery = if preparation.as_ref().is_some_and(|result| result.is_ok()) {
        "preparation_cancelled"
    } else if snapshot.pause_reason.is_some() {
        "project_paused"
    } else if let Some(slot) = incumbent {
        if release_incumbent(state, id, &project.structure, slot) {
            let released = state.entities.row(slot).uuid;
            shun(state, id, &project.structure, released, game);
            "builder_released"
        } else {
            "builder_retained"
        }
    } else {
        "no_builder"
    };
    if report {
        if let Some(watch) = state
            .settlements
            .builder_watch
            .get_mut(&(id, project.structure.clone()))
        {
            watch.recovery = Some(recovery);
        }
        tracing::warn!(settlement = id.0, blueprint = %project.blueprint,
            origin = ?project.origin, stage = snapshot.current_stage_index,
            work = snapshot.completed_work_units, ?next, cause, recovery, ?preparation,
            ?builder, ?position, ?navigation, ?navigation_failed,
            material = ?material.and_then(|(item, _)| state.registry.items.name_of(item)),
            required = ?material.map(|(_, count)| count), ?carried, ?pickup,
            last_refusal = ?refusal,
            shift_ticks = BUILDER_STALL_SHIFT_TICKS, "settlement_construction_stalled");
    }
    if let Some(first_recovery) = first_recovery {
        tracing::warn!(settlement = id.0, blueprint = %project.blueprint,
            origin = ?project.origin, stage = snapshot.current_stage_index,
            work = snapshot.completed_work_units, ?next, cause, first_recovery, recovery,
            shift_ticks = BUILDER_STALL_SHIFT_TICKS,
            "settlement_construction_recovery_failed");
    }
    preparation.is_some_and(|result| result.is_ok())
}

pub(crate) fn note_build_refusal(
    state: &mut TickState,
    id: SettlementId,
    handle: &str,
    reason: mc_script::ScriptOperationFailure,
) {
    if let Some(watch) = state
        .settlements
        .builder_watch
        .get_mut(&(id, handle.to_owned()))
    {
        watch.refusal = Some(match reason {
            mc_script::ScriptOperationFailure::Blocked => "executor_blocked",
            mc_script::ScriptOperationFailure::Busy => "reserved_worker",
            mc_script::ScriptOperationFailure::InsufficientItems => "insufficient_carry_or_storage",
            mc_script::ScriptOperationFailure::Capacity => "executor_capacity",
            mc_script::ScriptOperationFailure::Unloaded => "unloaded_worksite",
            _ => "executor_refused",
        });
    }
}

pub(crate) fn retry_preparation(
    state: &mut TickState,
    id: SettlementId,
    project: &Project,
) -> Result<(), String> {
    // A full queue refuses the retry before anything is cancelled: the paid
    // order must never be dropped because its place could not be restored.
    let requeue = project.ordered
        && state
            .settlements
            .get(id)
            .is_some_and(|record| record.owner.is_some());
    if requeue
        && state
            .settlements
            .get(id)
            .is_some_and(|record| access::orders(record).count() >= MAX_ORDERS)
    {
        return Err(format!(
            "Construction queue is full: MAX_ORDERS={MAX_ORDERS}."
        ));
    }
    if let Some((uuid, _)) = project.builder
        && executor::return_carry_of(state, id, uuid) != executor::CarryReturn::Delivered
    {
        // The cancelled request keeps its return duty even though the project
        // and its bill are gone (D8 review R4-1).
        owe_carry(state, id, uuid, &project.structure);
    }
    cancel_project(state, id, &project.structure)?;
    let record = state
        .settlements
        .get_mut(id)
        .expect("cancelled preparation");
    if requeue {
        // The submission sequence restores the request between the requests
        // submitted around it, even when some were cancelled meanwhile (D8
        // review P2-3).
        let seq = project.order_seq.unwrap_or(record.order_seq);
        access::insert_order_by_seq(
            record,
            OwnerOrder {
                seq,
                blueprint: project.blueprint.clone(),
                level: project.level,
                building: project.building,
            },
        );
        record.revision = record.revision.saturating_add(1);
    }
    if record.owner.is_some() {
        notify_owner(
            state,
            id,
            if requeue {
                format!(
                    "Cancelled unpaid {} work that made no progress; queued a new site.",
                    project.blueprint
                )
            } else {
                format!(
                    "Cancelled unpaid {} work that made no progress; any newer pending order is retained.",
                    project.blueprint
                )
            },
        );
    }
    Ok(())
}

fn resident_simulated(state: &TickState, slot: crate::entities::EntitySlot) -> bool {
    let at = super::allocator::at(state.entities.pos(slot));
    state
        .world
        .is_active(at.x.div_euclid(16), at.z.div_euclid(16))
}

fn has_staffed_gatherer(state: &TickState, id: SettlementId, profession: Profession) -> bool {
    residents(state, id).any(|slot| {
        resident_simulated(state, slot)
            && state.entities.body(slot).alive
            && state.entities.row(slot).villager.as_ref().is_some_and(|v| {
                v.age >= 0
                    && v.settlement == Some(id)
                    && v.conversion_time.is_none()
                    && v.data.profession == profession
                    && v.job_site.is_some_and(|job| {
                        buildings::job_profession(&state.settlements, Some(id), job)
                            == Some(profession)
                    })
            })
    })
}

fn restore_borrowed_gatherer(state: &mut TickState, id: SettlementId, profession: Profession) {
    if has_staffed_gatherer(state, id, profession) {
        return;
    }
    // Only a project that actually borrowed an extraction worker gives one back:
    // a native Builder bound as `(uuid, None)` keeps its career and its job.
    let binding = access::projects_of(&state.settlements, id)
        .filter_map(|project| project.builder)
        .find(|(_, borrowed)| borrowed.is_some());
    let Some((uuid, original)) = binding else {
        return;
    };
    release_builder(state, id, binding);
    if state.entities.slot_of_uuid(uuid).is_some_and(|slot| {
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .is_some_and(|v| Some(v.data.profession) == original)
    }) {
        let record = state.settlements.get_mut(id).expect("active settlement");
        access::unbind_builder(record, uuid);
        record.revision = record.revision.saturating_add(1);
    }
}

/// Record what a released builder still owes storage: the carried amount of
/// each item the project's plan uses. Only those items are owed, so a
/// resident's own food or a later working load is never taken (R4-1).
pub(crate) fn owe_carry(state: &mut TickState, id: SettlementId, uuid: uuid::Uuid, handle: &str) {
    // Placed items count too: a load the remaining bill no longer asks for
    // (a finished stage, a finished project) is still project stock.
    let items = executor::return_items(state, id, handle).ok();
    let Some(slot) = state.entities.slot_of_uuid(uuid) else {
        return;
    };
    let Some(v) = state.entities.row(slot).villager.as_ref() else {
        return;
    };
    let mut owed: Vec<(u32, u32)> = Vec::new();
    for stack in v.inventory.iter().flatten().filter(|stack| stack.count > 0) {
        let item = stack.item_id;
        // Unreadable plan: fail closed on the whole load this builder was
        // carrying, since it was bound to this project a moment ago.
        if let Some(items) = items.as_ref()
            && !items.contains(&item)
        {
            continue;
        }
        let count = stack.count as u32;
        match owed.iter_mut().find(|(known, _)| *known == item) {
            Some((_, sum)) => *sum = sum.saturating_add(count),
            None => owed.push((item, count)),
        }
    }
    if owed.is_empty() {
        return;
    }
    let Some(record) = state.settlements.get_mut(id) else {
        return;
    };
    record.carry_pending.insert(uuid, owed);
    record.revision = record.revision.saturating_add(1);
}

/// Drain the recorded material debts: what the store accepted leaves the debt,
/// what it could not take stays for a later pass, and a body that no longer
/// exists has nothing left to deliver.
fn drain_carry_duties(state: &mut TickState, id: SettlementId) {
    let Some(record) = state.settlements.get(id) else {
        return;
    };
    let debts: Vec<(uuid::Uuid, Vec<(u32, u32)>)> = record
        .carry_pending
        .iter()
        .map(|(uuid, owed)| (*uuid, owed.clone()))
        .collect();
    for (uuid, owed) in debts {
        let residual = executor::return_owed_carry(state, id, uuid, &owed);
        let Some(record) = state.settlements.get_mut(id) else {
            return;
        };
        match residual {
            // The body is gone: nothing is owed any more.
            None => {
                record.carry_pending.remove(&uuid);
                record.revision = record.revision.saturating_add(1);
            }
            Some(left) if left.is_empty() => {
                record.carry_pending.remove(&uuid);
                record.revision = record.revision.saturating_add(1);
            }
            Some(left) => {
                if record.carry_pending.get(&uuid) != Some(&left) {
                    record.carry_pending.insert(uuid, left);
                    record.revision = record.revision.saturating_add(1);
                }
            }
        }
    }
}

/// A resident that still owes storage is not hired for another project until
/// the debt is closed, so its new load is never mistaken for the old one.
fn carry_debtor(state: &TickState, id: SettlementId, uuid: uuid::Uuid) -> bool {
    state
        .settlements
        .get(id)
        .is_some_and(|record| record.carry_pending.contains_key(&uuid))
}

/// Planning keeps its one-settlement/second budget; its site slices come from
/// the tick's `budget` (`site_turn`). Construction keeps its own four-tick
/// admission clock, retaining the bounded round-robin settlement budget.
fn planner_tick(state: &mut TickState, budget: &mut site_turn::SiteBudget) {
    let count = state.settlements.records().count();
    if count == 0 {
        return;
    }
    if !state.tick.is_multiple_of(4) {
        // Off the admission clock only a settlement whose last pass left an
        // unfinished walk holding a legal lot is stepped, and only its site
        // scan: no duties, no gatherer staffing, no builder assignment, no
        // admission and no construction cell. A walk that finishes off the
        // clock parks its lot for the next admission pass.
        step_sites(state, budget);
        return;
    }
    let selected = |state: &TickState, turn: u64| {
        state
            .settlements
            .records()
            .nth(turn as usize % count)
            .filter(|record| {
                state.world.is_active(
                    record.anchor[0].div_euclid(16),
                    record.anchor[2].div_euclid(16),
                )
            })
            .map(|record| record.id)
    };
    // The planner's once-per-PLAN_CADENCE admission pass; the four-tick clock
    // below it only advances construction.
    if state.tick.is_multiple_of(PLAN_CADENCE)
        && let Some(id) = selected(state, state.tick / PLAN_CADENCE)
    {
        // Return duties are drained for every selected settlement, even one
        // whose projects are all cancelled or finished (D8 review R4-1).
        drain_carry_duties(state, id);
        if let Some(profession) = needed_gatherer(state, id) {
            super::allocator::staff_gatherer(state, id, profession);
            restore_borrowed_gatherer(state, id, profession);
            super::allocator::staff_gatherer(state, id, profession);
        }
        if std::env::var_os("MC_CORE_PHASE_LOG").is_some() {
            eprintln!(
                "settlement planner: tick={} id={} demand={:?} waiting={:?} project={:?}",
                state.tick,
                id.0,
                needed_gatherer(state, id),
                waiting(state, id),
                state
                    .settlements
                    .get(id)
                    .and_then(|record| access::projects(record).next()),
            );
            for slot in residents(state, id) {
                if let Some(v) = state.entities.row(slot).villager.as_ref() {
                    eprintln!(
                        "settlement resident: tick={} uuid={} pos={:?} profession={:?} activity={:?} sleeping={} target={:?} path={} failed={} repath={} ration={} lumber={:?}",
                        state.tick,
                        state.entities.row(slot).uuid,
                        state.entities.pos(slot),
                        v.data.profession,
                        v.activity,
                        v.sleeping,
                        v.navigation_target,
                        v.navigation_path.len(),
                        v.navigation_search_failed,
                        v.navigation_repath_at,
                        v.ration_at,
                        v.lumber_progress,
                    );
                }
            }
        }
        let opened = plan_pass(state, id, budget);
        if state
            .settlements
            .get(id)
            .is_some_and(|record| access::projects(record).next().is_some())
            && assign_builder(state, id)
            && opened
        {
            withdraw_opened_site(state, id);
        }
    }
    if let Some(id) = selected(state, state.tick / 4)
        && state
            .settlements
            .get(id)
            .is_some_and(|record| access::projects(record).next().is_some())
    {
        crate::villager::building::tick(state, id);
    }
}

/// The gate saw no failed route to storage, but every resident the binding
/// pass then tried failed it: the autonomous site this pass opened is
/// withdrawn unbuilt, so it never holds the planner, and the recorded failures
/// keep the gate closed until they lapse. Owner orders are never withdrawn.
fn withdraw_opened_site(state: &mut TickState, id: SettlementId) {
    // `push_project` appends: the last project is the one this pass opened.
    let Some(handle) = state
        .settlements
        .get(id)
        .and_then(|record| access::projects(record).last())
        .filter(|project| !project.ordered && project.builder.is_none())
        .map(|project| project.structure.clone())
    else {
        return;
    };
    match cancel_project(state, id, &handle) {
        Ok(()) => tracing::info!(target: "settlement_audit", settlement = id.0,
            structure = %handle, tick = state.tick, "settlement_site_withdrawn"),
        Err(error) => tracing::warn!(settlement = id.0, structure = %handle, %error,
            "settlement_site_withdrawal_failed"),
    }
}

fn release_builder(
    state: &mut TickState,
    id: SettlementId,
    binding: Option<(uuid::Uuid, Option<Profession>)>,
) {
    let builder = residents(state, id).find(|slot| {
        let row = state.entities.row(*slot);
        binding.is_none_or(|(uuid, _)| row.uuid == uuid)
            && row.villager.as_ref().is_some_and(|v| {
                v.settlement == Some(id) && v.data.profession == Profession::Builder
            })
    });
    if let Some(slot) = builder
        && let Some(v) = state.entities.row_mut(slot).villager.as_mut()
        && detach(v)
    {
        if let Some((_, Some(profession))) = binding {
            v.data.profession = profession;
        }
        crate::villager::sync_appearance(state, slot);
    }
}

/// Finishes the project that owns `structure`. A handle that names no active
/// Why the certificate would refuse this project, without touching any state.
///
/// `finish_project` mutates before it refuses (`owe_carry`, `buildings::certify` certifies the
/// building), so calling it speculatively from a watcher disturbs the settlement. This mirrors the
/// certificate side only: bounds, blueprint level and the missing building requirements, rendered as
/// `description: found/required`.
pub(crate) fn certificate_missing(state: &TickState, project: &Project) -> Option<String> {
    let (blueprint, bounds) = match certificate_bounds(state, project) {
        Ok(found) => found,
        Err(reason) => return Some(reason),
    };
    buildings::inspect(state, blueprint.kind, 1, bounds)
        .err()
        .map(|error| error.to_string())
}

/// The certificate's blueprint and box for `project`, or why there is none. Shared by the refusal
/// path (`finish_project`) and the pure watcher query (`certificate_missing`) so one geometry is
/// computed in one place.
fn certificate_bounds(
    state: &TickState,
    project: &Project,
) -> Result<(&'static blueprints::Blueprint, ScriptWorldBounds), String> {
    let Some(blueprint) = blueprints::get(&state.registry, &project.blueprint) else {
        return Err(format!("unknown blueprint {}", project.blueprint));
    };
    let Some(level) = blueprint.level(project.level) else {
        return Err(format!(
            "no level {} for {} ({} level(s))",
            project.level,
            project.blueprint,
            blueprint.levels.len()
        ));
    };
    Ok((
        blueprint,
        level
            .rotated(Rotation::from_quarter_turns(project.rotation))
            .bounds(project.origin),
    ))
}

/// The completion pass of one project: a committed structure whose cell a foreign edit emptied
/// gets the repair stage, a whole certificate certifies the building, and a hole refuses with its
/// named cause and the missing requirement (once per pass; the watcher's own query is pure).
pub(crate) fn finish_project(state: &mut TickState, id: SettlementId, structure: &str) -> bool {
    let Some(project) = state
        .settlements
        .get(id)
        .and_then(|record| access::project_for_structure(record, structure))
        .cloned()
    else {
        tracing::warn!(
            settlement = id.0,
            structure,
            cause = "no_project",
            "finish_project: no project for the structure"
        );
        return false;
    };
    // Certification is not a substitute for matching the placed blueprint:
    // the executor may have reached its last cell after a player broke one.
    // `recover` returns `Ok(false)` only when it did nothing; a repair it opens
    // (or a terminal-index structure it finishes in place) keeps the project
    // open for this pass instead of certifying it.
    if executor::recover(state, id, &project.structure) != Ok(false) {
        return false;
    }
    if !executor::status(state, id, &project.structure)
        .is_ok_and(|snapshot| snapshot.state == ScriptStructureState::Committed)
    {
        tracing::warn!(
            settlement = id.0,
            structure = %project.structure,
            cause = "not_committed",
            "finish_project: the structure is not committed"
        );
        return false;
    }
    // Every cell is placed: project stock the bound builder still carries is a
    // leftover it owes storage, whichever way the project closes (R4-1).
    if let Some((uuid, _)) = project.builder {
        owe_carry(state, id, uuid, &project.structure);
    }
    if project.blueprint == furnace::NAME || layout::infra(&project.blueprint) {
        // Infrastructure work is temporary employment.
        release_builder(state, id, project.builder);
        let record = state.settlements.get_mut(id).expect("finished settlement");
        if project.blueprint == furnace::NAME {
            record.furnace_workstation = Some(project.structure.clone());
        }
        access::take_project(record, &project.structure);
        record.revision = record.revision.saturating_add(1);
        return true;
    }
    // A placed cell that a foreign edit emptied is queued for re-placement and
    // the project stays open; the hole would otherwise block the certificate.
    if executor::reopen_emptied(state, id, &project.structure) != Ok(false) {
        tracing::warn!(
            settlement = id.0,
            structure = %project.structure,
            cause = "reopen_emptied",
            "finish_project: a placed cell was reopened"
        );
        return false;
    }
    let Ok((blueprint, bounds)) = certificate_bounds(state, &project) else {
        return false;
    };
    if state
        .settlements
        .get(id)
        .is_some_and(|record| founding::owns_project(record, &project))
    {
        if buildings::inspect(state, blueprint.kind, 1, bounds).is_err() {
            return false;
        }
        // Outpost POIs stay outside the parent's civil allocator until the
        // whole household can receive all certificates in the daughter.
        release_builder(state, id, project.builder);
        let record = state.settlements.get_mut(id).expect("finished outpost");
        access::take_project(record, &project.structure);
        record.revision = record.revision.saturating_add(1);
        tracing::info!(target: "settlement_audit", settlement = id.0,
            blueprint = %project.blueprint, origin = ?project.origin,
            tick = state.tick, game_time = state.tick_control.game_ticks(state.tick),
            "settlement_outpost_completed");
        return true;
    }
    // A refusal is not logged here: `watch_project` names the same requirement once per shift
    // (`certificate_incomplete`), and this path runs on every builder tick that has no cell.
    let Ok(building) = buildings::certify(state, id, blueprint.kind, bounds) else {
        return false;
    };
    if project
        .building
        .is_some_and(|expected| expected != building)
    {
        tracing::warn!(
            settlement = id.0,
            structure = %project.structure,
            cause = "building_mismatch",
            "finish_project: the certificate is another building"
        );
        return false;
    }
    if project.level > 1
        && let Err(error) = buildings::upgrade(state, building)
    {
        tracing::warn!(
            settlement = id.0,
            blueprint = %project.blueprint,
            level = project.level,
            building = building.0,
            cause = "upgrade_failed",
            missing = %error,
            "finish_project: the level upgrade did not certify"
        );
        return false;
    }
    let record = state.settlements.get_mut(id).expect("certified settlement");
    access::take_project(record, &project.structure);
    record.revision = record.revision.saturating_add(1);
    tracing::info!(
        target: "settlement_audit",
        settlement = id.0,
        building = building.0,
        blueprint = %project.blueprint,
        level = project.level,
        tick = state.tick,
        game_time = state.tick_control.game_ticks(state.tick),
        "settlement_building_completed"
    );
    // Only a borrowed gatherer is released at certification; an ordinary
    // recruit keeps the builder career it was recruited into.
    if project
        .builder
        .is_some_and(|(_, profession)| profession.is_some())
    {
        release_builder(state, id, project.builder);
    }
    true
}

/// One needed blueprint kind whose level-one bill the settlement cannot fund.
#[derive(Debug)]
pub(crate) struct WaitingKind {
    #[cfg(test)]
    pub(crate) blueprint: String,
    /// Unfunded bill entries, in bill order.
    pub(crate) needs: Vec<WaitingNeed>,
}

/// One unfunded bill entry of [`WaitingKind`].
#[derive(Debug)]
pub(crate) struct WaitingNeed {
    pub(crate) item: String,
    pub(crate) have: u64,
    pub(crate) total: u64,
    pub(crate) source_item: Option<String>,
    pub(crate) no_source: Option<SourceReason>,
}

/// One project's remaining bill, net of the carry of the builder bound to it.
/// Another project's load never pays for these cells (D8).
pub(crate) fn project_bill(
    state: &TickState,
    id: SettlementId,
    handle: &str,
) -> Option<Vec<(u32, u32)>> {
    let project =
        access::projects_of(&state.settlements, id).find(|project| project.structure == handle)?;
    let snapshot = executor::status(state, id, &project.structure).ok()?;
    let mut bill: Vec<_> = snapshot
        .remaining
        .into_iter()
        .map(|material| {
            let name = mc_data::Identifier::parse(&material.resource).ok()?;
            Some((
                state.registry.items.id_of(&name)?,
                u32::try_from(material.quantity).ok()?,
            ))
        })
        .collect::<Option<_>>()?;
    if let Some((bound, _)) = project.builder
        && let Some(slot) =
            residents(state, id).find(|slot| state.entities.row(*slot).uuid == bound)
        && let Some(v) = state.entities.row(slot).villager.as_ref()
    {
        for stack in v.inventory.iter().flatten().filter(|stack| stack.count > 0) {
            if let Some((_, count)) = bill.iter_mut().find(|(item, _)| *item == stack.item_id) {
                *count = count.saturating_sub(stack.count as u32);
            }
        }
    }
    bill.retain(|(_, count)| *count != 0);
    Some(bill)
}

/// Every active project's bill, summed per item: the demand every worker query
/// uses once a settlement builds more than one project at a time.
pub(crate) fn joint_active_bill(state: &TickState, id: SettlementId) -> Option<Vec<(u32, u32)>> {
    let mut total: Vec<(u32, u32)> = Vec::new();
    for handle in access::projects_of(&state.settlements, id)
        .map(|project| project.structure.clone())
        .collect::<Vec<_>>()
    {
        // A project whose ledger cannot be read might still owe materials:
        // fail closed rather than under-count the joint bill.
        let bill = project_bill(state, id, &handle)?;
        for (item, count) in bill {
            match total.iter_mut().find(|(known, _)| *known == item) {
                Some((_, sum)) => *sum = sum.saturating_add(count),
                None => total.push((item, count)),
            }
        }
    }
    Some(total)
}

/// One real bill drives gathering, including a previously funded project
/// whose stock was spent or withdrawn after admission.
pub(crate) fn needed_gatherer(state: &TickState, id: SettlementId) -> Option<Profession> {
    let record = state.settlements.get(id)?;
    let images = executor::source_images(state, id);
    if access::projects(record).next().is_some() {
        let bill = joint_active_bill(state, id)?;
        if bill.is_empty() {
            return None;
        }
        super::construction_crafting::needed_gatherer(state, &bill, &images)
    } else {
        gatherer_demand(state, record, &images)
    }
}

/// The active blueprint's fixed wood family while it needs more logs.
pub(crate) fn active_project_wood(state: &TickState, id: SettlementId) -> Option<&str> {
    let bill = joint_active_bill(state, id)?;
    if bill.is_empty() {
        return None;
    }
    let images = executor::source_images(state, id);
    if super::construction_crafting::needed_gatherer(state, &bill, &images)
        != Some(Profession::Lumberjack)
    {
        return None;
    }
    bill.into_iter()
        .find_map(|(item, _)| wood_species(state, item))
}

/// The extraction worker the settlement's next bill needs: a pending owner
/// order's, else the first autonomous candidate whose unfunded bill needs one.
/// A priority bill short only of what no worker produces (a bed's wool with
/// no sheep) must not leave the later Farm without its dirt and stone.
fn gatherer_demand(
    state: &TickState,
    record: &SettlementRecord,
    images: &[super::inventory::Image],
) -> Option<Profession> {
    let demand = |name: &str, level: u8| {
        let level = blueprints::get(&state.registry, name)?.level(level)?;
        let bill = selected_bill(state, record.id, &level.materials, images)?;
        super::construction_crafting::needed_gatherer(state, &bill, images)
    };
    if let Some(order) = access::head_order(record) {
        return demand(order.blueprint.as_str(), order.level);
    }
    if let Some(name) = founding::next_blueprint(state, record)
        && let Some(profession) = demand(name, 1)
    {
        return Some(profession);
    }
    let catalog = catalog(state, record);
    candidate_kinds(state, record, images).find_map(|kind| {
        demand(
            catalog
                .clone()
                .find(|blueprint| blueprint.kind == kind)?
                .id
                .as_str(),
            1,
        )
    })
}

/// Current bounded construction target, before subtracting the iron pipeline.
/// A pending owner order takes priority. Otherwise the larger of the active
/// project and the largest single autonomous candidate (not the sum of
/// speculative builds): a funded project must not starve the iron for an
/// unfunded priority house it was admitted ahead of, as `wool_demand` agrees.
pub(crate) fn iron_demand(
    state: &TickState,
    id: SettlementId,
    detached: Option<&crate::villager::VillagerState>,
) -> u32 {
    let Some(record) = state.settlements.get(id) else {
        return 0;
    };
    let Some(iron) = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:iron_ingot").expect("identifier"))
    else {
        return 0;
    };
    let shears_id = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:shears").expect("identifier"));
    let detached_shears = detached.is_some_and(|villager| {
        villager
            .inventory
            .iter()
            .flatten()
            .any(|stack| Some(stack.item_id) == shears_id && stack.count > 0)
    });
    let images = executor::source_images(state, id);
    let demand = |bill: &[(u32, u32)]| {
        let shears = !detached_shears && availability::needs_shears(state, id, bill, &images);
        let direct = bill
            .iter()
            .filter(|(item, _)| *item == iron)
            .fold(0u32, |sum, (_, count)| sum.saturating_add(*count));
        direct.saturating_add(if shears { 2 } else { 0 })
    };
    if let Some(order) = access::head_order(record) {
        return blueprints::get(&state.registry, &order.blueprint)
            .and_then(|blueprint| blueprint.level(order.level))
            .and_then(|level| selected_bed_bill(state, id, &level.materials, &images))
            .map_or(0, |bill| demand(&bill));
    }
    // Every active project's bill asks for its own iron (D8).
    let mut active: u32 = 0;
    for project in access::projects(record) {
        active = active.saturating_add(
            executor::status(state, id, &project.structure)
                .ok()
                .map_or(0, |snapshot| {
                    let bill: Vec<_> = snapshot
                        .remaining
                        .iter()
                        .filter_map(|material| {
                            let name = mc_data::Identifier::parse(&material.resource).ok()?;
                            let item = state.registry.items.id_of(&name)?;
                            Some((item, u32::try_from(material.quantity).unwrap_or(u32::MAX)))
                        })
                        .collect();
                    demand(&bill)
                }),
        );
    }
    let catalog = catalog(state, record);
    candidate_kinds(state, record, &images)
        .filter_map(|kind| catalog.clone().find(|blueprint| blueprint.kind == kind))
        .filter_map(|blueprint| blueprint.level(1))
        .filter_map(|level| selected_bed_bill(state, id, &level.materials, &images))
        .map(|bill| demand(&bill))
        .fold(active, u32::max)
}

/// A current construction bill still needs wool, directly or through a bed.
/// Uses the same stock and recipe proof as shears/iron provisioning.
pub(crate) fn wool_demand(state: &TickState, id: SettlementId) -> bool {
    let Some(record) = state.settlements.get(id) else {
        return false;
    };
    let images = executor::source_images(state, id);
    for project in access::projects(record) {
        let active_wool = executor::status(state, id, &project.structure)
            .ok()
            .is_some_and(|snapshot| {
                let bill: Vec<_> = snapshot
                    .remaining
                    .iter()
                    .filter_map(|material| {
                        let item = state
                            .registry
                            .items
                            .id_of(&mc_data::Identifier::parse(&material.resource).ok()?)?;
                        Some((item, u32::try_from(material.quantity).unwrap_or(u32::MAX)))
                    })
                    .collect();
                availability::needs_wool(state, &bill, &images)
            });
        if active_wool {
            return true;
        }
    }
    if let Some(order) = access::head_order(record) {
        return blueprints::get(&state.registry, &order.blueprint)
            .and_then(|blueprint| blueprint.level(order.level))
            .and_then(|level| selected_bed_bill(state, id, &level.materials, &images))
            .is_some_and(|bill| availability::needs_wool(state, &bill, &images));
    }
    let catalog = catalog(state, record);
    candidate_kinds(state, record, &images)
        .filter_map(|kind| catalog.clone().find(|blueprint| blueprint.kind == kind))
        .filter_map(|blueprint| blueprint.level(1))
        .filter_map(|level| selected_bed_bill(state, id, &level.materials, &images))
        .any(|bill| availability::needs_wool(state, &bill, &images))
}

/// Color of the concrete unmet bed bill, for the worker that supplies it.
pub(crate) fn wool_color(state: &TickState, id: SettlementId) -> Option<u8> {
    let record = state.settlements.get(id)?;
    let images = executor::source_images(state, id);
    let color_for = |bill: &[(u32, u32)]| {
        if !availability::needs_wool(state, bill, &images) {
            return None;
        }
        bill.iter().find_map(|(item, _)| {
            let name = state.registry.items.name_of(*item)?;
            let color = name
                .path()
                .strip_suffix("_bed")
                .or_else(|| name.path().strip_suffix("_wool"))?;
            crate::blueprint::DYE_COLORS
                .iter()
                .position(|candidate| *candidate == color)
                .map(|index| index as u8)
        })
    };
    // Every active project's bed decides its own colour (D8).
    for project in access::projects(record) {
        let Ok(snapshot) = executor::status(state, id, &project.structure) else {
            continue;
        };
        let bill: Vec<_> = snapshot
            .remaining
            .iter()
            .filter_map(|material| {
                let item = state
                    .registry
                    .items
                    .id_of(&mc_data::Identifier::parse(&material.resource).ok()?)?;
                Some((item, u32::try_from(material.quantity).unwrap_or(u32::MAX)))
            })
            .collect();
        if let Some(color) = color_for(&bill) {
            return Some(color);
        }
    }
    if let Some(order) = access::head_order(record) {
        let level = blueprints::get(&state.registry, &order.blueprint)?.level(order.level)?;
        return color_for(&selected_bed_bill(state, id, &level.materials, &images)?);
    }
    let catalog = catalog(state, record);
    candidate_kinds(state, record, &images)
        .filter_map(|kind| catalog.clone().find(|blueprint| blueprint.kind == kind))
        .filter_map(|blueprint| blueprint.level(1))
        .filter_map(|level| selected_bed_bill(state, id, &level.materials, &images))
        .find_map(|bill| color_for(&bill))
}

/// Whether the planner can fund at least one of its current candidates (test oracle: the
/// project card no longer derives an idle reason from it).
#[cfg(test)]
pub(crate) fn has_funded_candidate(state: &TickState, id: SettlementId) -> bool {
    let Some(record) = state.settlements.get(id) else {
        return false;
    };
    let images = executor::source_images(state, id);
    funded_candidate(state, record, &images)
}

fn funded_candidate(
    state: &TickState,
    record: &SettlementRecord,
    images: &[super::inventory::Image],
) -> bool {
    if let Some(order) = access::head_order(record) {
        return blueprints::get(&state.registry, &order.blueprint)
            .and_then(|blueprint| blueprint.level(order.level))
            .is_some_and(|level| {
                funded_variant(state, record.id, &level.materials, images).is_some()
            });
    }
    let catalog = catalog(state, record);
    candidate_kinds(state, record, images).any(|kind| {
        catalog
            .clone()
            .find(|blueprint| blueprint.kind == kind)
            .and_then(|blueprint| blueprint.level(1))
            .is_some_and(|level| {
                funded_variant(state, record.id, &level.materials, images).is_some()
            })
    }) || preparation::upgrade_project(state, record, images).is_some()
}

/// Needed authored kinds the settlement cannot fund yet, one entry per kind.
/// Formatting (translation, per-line cap) belongs to the dialog.
pub(crate) fn waiting(state: &TickState, id: SettlementId) -> Vec<WaitingKind> {
    let Some(record) = state.settlements.get(id) else {
        return Vec::new();
    };
    let images = executor::source_images(state, id);
    let catalog = catalog(state, record);
    let active: Vec<&str> = access::projects(record)
        .map(|project| project.blueprint.as_str())
        .collect();
    let mut kinds = Vec::new();
    let pending = access::head_order(record);
    let queued = pending.and_then(|order| {
        blueprints::get(&state.registry, &order.blueprint).map(|blueprint| (blueprint, order.level))
    });
    let autonomous = candidate_kinds(state, record, &images)
        .filter(|_| pending.is_none())
        .filter_map(|kind| catalog.clone().find(|blueprint| blueprint.kind == kind))
        .filter(|blueprint| !active.contains(&blueprint.id.as_str()))
        .map(|blueprint| (blueprint, 1));
    for (blueprint, level) in queued.into_iter().chain(autonomous) {
        let Some(level) = blueprint.level(level) else {
            continue;
        };
        if funded_variant(state, id, &level.materials, &images).is_some() {
            continue;
        }
        let Some(bill) = selected_bill(state, id, &level.materials, &images) else {
            continue;
        };
        let mut needs = Vec::new();
        for (item, count) in super::construction_crafting::missing_base(state, &bill, &images) {
            if let Some(name) = state.registry.items.name_of(item) {
                let total = bill
                    .iter()
                    .find(|(id, _)| *id == item)
                    .map_or(u64::from(count), |(_, total)| u64::from(*total));
                let missing = availability::missing_explanation(state, id, item, &images);
                needs.push(WaitingNeed {
                    item: name.to_string(),
                    have: total.saturating_sub(u64::from(count)),
                    total,
                    source_item: missing.and_then(|(leaf, _)| {
                        (leaf != item)
                            .then(|| state.registry.items.name_of(leaf).map(ToString::to_string))
                            .flatten()
                    }),
                    no_source: missing.map(|(_, reason)| reason),
                });
            }
        }
        if !needs.is_empty() {
            kinds.push(WaitingKind {
                #[cfg(test)]
                blueprint: blueprint.id.clone(),
                needs,
            });
        }
    }
    kinds
}

/// Plain-text waiting lines for [`project_status`] diagnostics only.
#[cfg(test)]
fn waiting_materials(state: &TickState, id: SettlementId, lines: &mut Vec<String>) {
    for kind in waiting(state, id) {
        for need in kind.needs {
            lines.push(format!(
                "Waiting {}: needs {} x{} — {} — {}",
                kind.blueprint,
                need.item,
                need.total - need.have,
                need.source_item.as_deref().unwrap_or(&need.item),
                if need.no_source.is_some() {
                    "no source"
                } else {
                    "awaiting production"
                },
            ));
        }
    }
}

/// Plain-text project status (summary plus one English waiting line per unmet
/// item) for failure diagnostics; the dialog formats [`waiting`] itself.
#[cfg(test)]
pub(crate) fn project_status(state: &TickState, id: SettlementId) -> Vec<String> {
    let mut lines = project_summary(state, id);
    waiting_materials(state, id, &mut lines);
    lines
}

/// Plain-text progress diagnostics for construction tests.
#[cfg(test)]
pub(crate) fn project_summary(state: &TickState, id: SettlementId) -> Vec<String> {
    let Some(record) = state.settlements.get(id) else {
        return vec!["Settlement not found.".into()];
    };
    let mut lines = Vec::new();
    // Every active project is summarised: parallel sites all report their own
    // ledger and materials (D8 review R2-5).
    for project in access::projects(record) {
        if !lines.is_empty() {
            lines.push(format!("--- {}", project.structure));
        }
        lines.extend(project_lines(state, id, project));
    }
    if lines.is_empty() {
        return vec!["No active project. Waiting for residents, materials or a free site.".into()];
    }
    lines
}

/// One project's status lines, from its own ledger and material proof.
#[cfg(test)]
fn project_lines(state: &TickState, id: SettlementId, project: &Project) -> Vec<String> {
    let Ok(snapshot) = executor::status(state, id, &project.structure) else {
        return vec!["Project ledger unavailable; paused.".into()];
    };
    let total: u64 = snapshot.stages.iter().map(|stage| stage.work_units).sum();
    let done: u64 = snapshot
        .stages
        .iter()
        .take(snapshot.current_stage_index as usize)
        .map(|stage| stage.work_units)
        .sum::<u64>()
        + snapshot.completed_work_units;
    let mut lines = vec![
        format!(
            "{} · level {} · {}%",
            project.blueprint,
            project.level,
            done * 100 / total.max(1)
        ),
        format!("Blocks: {done}/{total} · {:?}", snapshot.state),
        format!(
            "Site: {}, {}, {}",
            project.origin[0], project.origin[1], project.origin[2]
        ),
    ];
    lines.push("Remaining materials (including site preparation):".into());
    lines.extend(
        snapshot
            .remaining
            .iter()
            .map(|material| format!("{}: {}", material.resource, material.quantity)),
    );
    match executor::missing(state, id, &project.structure) {
        Ok(missing) if missing.is_empty() => {
            lines.push("Materials ready; builder collecting and placing.".into())
        }
        Ok(missing) => {
            lines.push("Missing materials:".into());
            lines.extend(
                missing
                    .into_iter()
                    .map(|(item, count)| format!("{item}: {count}")),
            );
        }
        Err(_) => lines.push("Storage unavailable.".into()),
    }
    lines
}

pub(super) fn validate_record(record: &SettlementRecord) -> Result<(), &'static str> {
    if record.projects.len() > MAX_PROJECTS || record.orders.len() > MAX_ORDERS {
        return Err("too many construction projects or orders");
    }
    for (index, project) in access::projects(record).enumerate() {
        if project.settlement != record.id
            || project.blueprint.is_empty()
            || project.blueprint.len() > 128
            || project.structure.is_empty()
            || project.structure.len() > 128
            || !(1..=3).contains(&project.level)
            || project.rotation > 3
            || project
                .building
                .is_some_and(|id| !record.buildings.contains_key(&id))
            || (project.level > 1) != project.building.is_some()
        {
            return Err("invalid settlement construction project");
        }
        // One structure handle and one builder belong to one project.
        if access::projects(record)
            .skip(index + 1)
            .any(|other| other.structure == project.structure)
        {
            return Err("duplicate settlement construction structure");
        }
        if let Some((builder, _)) = project.builder
            && access::projects(record)
                .skip(index + 1)
                .any(|other| other.builder.is_some_and(|(bound, _)| bound == builder))
        {
            return Err("a villager is bound to two construction projects");
        }
    }
    let mut previous = 0;
    for order in access::orders(record) {
        if order.seq < previous || order.seq > record.order_seq {
            return Err("construction orders are not in submission order");
        }
        previous = order.seq;
        if record.owner.is_none()
            || order.blueprint.is_empty()
            || order.blueprint.len() > 128
            || !(1..=3).contains(&order.level)
            || (order.level > 1) != order.building.is_some()
            || order
                .building
                .is_some_and(|id| id.0 >> 32 != record.id.0 || id.0 as u32 == 0)
        {
            return Err("invalid pending owner construction order");
        }
    }
    Ok(())
}

pub(super) fn validate_pending(
    registry: &crate::registry::CoreRegistry,
    record: &SettlementRecord,
) -> Result<(), &'static str> {
    // Every queued request is checked: a restored queue can lift any of them
    // (D8 review P3).
    for order in access::orders(record) {
        let blueprint = blueprints::get(registry, &order.blueprint)
            .filter(|blueprint| blueprint.level(order.level).is_some())
            .ok_or("unknown pending owner blueprint or level")?;
        if let Some(id) = order.building
            && record
                .buildings
                .get(&id)
                .is_some_and(|building| building.kind != blueprint.kind)
        {
            return Err("invalid pending owner upgrade target");
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "construction_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "construction_culture_tests.rs"]
mod culture_tests;

#[cfg(test)]
#[path = "construction_fencing_tests.rs"]
mod fencing_tests;

#[cfg(test)]
#[path = "construction_durability_tests.rs"]
mod durability_tests;

#[cfg(test)]
#[path = "construction_planner_tests.rs"]
mod planner_tests;

#[cfg(test)]
#[path = "construction_diagnostics_tests.rs"]
mod diagnostics_tests;
#[cfg(test)]
#[path = "construction_queue_tests.rs"]
mod queue_tests;

#[cfg(test)]
#[path = "construction_parallel_tests.rs"]
mod parallel_tests;

#[cfg(test)]
#[path = "construction_parallel_tick_tests.rs"]
mod parallel_tick_tests;

#[cfg(test)]
#[path = "construction_site_tests.rs"]
mod site_tests;

#[cfg(test)]
#[path = "construction_terrain_tests.rs"]
mod terrain_tests;

#[cfg(test)]
#[path = "construction_stall_tests.rs"]
mod stall_tests;

#[cfg(test)]
#[path = "construction_fortification_tests.rs"]
mod fortification_tests;

#[cfg(test)]
#[path = "construction_growth_tests.rs"]
mod growth_tests;

#[cfg(test)]
#[path = "construction_provenance_tests.rs"]
mod provenance_tests;

#[path = "construction_filler.rs"]
pub(super) mod filler;

#[cfg(test)]
#[path = "construction_liquid_tests.rs"]
mod liquid_tests;

#[path = "construction_preparation.rs"]
pub(crate) mod preparation;
#[path = "relayout.rs"]
pub(crate) mod relayout;
use preparation::planned_cells;
