//! Settlement planning and certification. Physical construction, material
//! debit, placement order and progress belong solely to the structure executor.
use serde::{Deserialize, Serialize};

use super::blueprints::{self, Rotation};
use super::buildings::{self, BuildingId, BuildingKind};
use super::{SettlementId, SettlementRecord, layout};
use crate::plugin_structures::settlement_builds as executor;
use crate::tick::TickState;
use crate::villager::VillagerProfession as Profession;
use mc_script::{ScriptStructureState, ScriptWorldBounds};

#[path = "construction_availability.rs"]
mod availability;
pub(crate) use availability::SourceReason;
pub(crate) use availability::gatherer_for_item;

#[path = "construction_furnace.rs"]
pub(crate) mod furnace;

pub(crate) const PLAN_CADENCE: u64 = 20;

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
    /// Set by an owner/operator order. The planner never preempts it; an
    /// unordered (autonomous) project that placed nothing yields to an order.
    #[serde(default)]
    pub ordered: bool,
    /// Temporary builder identity and original profession, restored on release.
    pub borrowed_builder: Option<(uuid::Uuid, Profession)>,
}
/// The sole pending owner request, stored in the settlement post-image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerOrder {
    pub blueprint: String,
    pub level: u8,
    pub building: Option<BuildingId>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum OrderOutcome {
    Started,
    Queued { reason: String, replaced: bool },
}

impl OrderOutcome {
    pub(crate) fn message(&self, blueprint: &str) -> String {
        match self {
            Self::Started => format!("Started {blueprint} construction."),
            Self::Queued { reason, replaced } => format!(
                "Queued {blueprint}: {reason}{}",
                if *replaced {
                    " (replaced previous order)"
                } else {
                    ""
                }
            ),
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
    let current = if claim_slot(state, id).is_err() {
        record
            .project
            .as_ref()
            .filter(|project| project.building == Some(building))
            .map_or(target.level, |project| project.level)
    } else {
        target.level
    };
    blueprint.level(current + 1).map(|_| current + 1)
}

fn start_order(
    state: &mut TickState,
    id: SettlementId,
    order: &OwnerOrder,
    planning_cursor: Option<u32>,
) -> Result<(), String> {
    match order.building {
        Some(building) => start_upgrade(state, id, building),
        None => start_build(state, id, &order.blueprint, planning_cursor),
    }
}

fn submit_order(
    state: &mut TickState,
    id: SettlementId,
    order: OwnerOrder,
) -> Result<OrderOutcome, String> {
    match start_order(state, id, &order, None) {
        Ok(()) => {
            let record = state.settlements.get_mut(id).expect("admitted settlement");
            record.pending_order = None;
            Ok(OrderOutcome::Started)
        }
        Err(reason)
            if reason == "Busy."
                || reason.starts_with("not enough:")
                || reason == "waiting for forest evidence" =>
        {
            let record = state
                .settlements
                .get_mut(id)
                .ok_or("Settlement not found.")?;
            let replaced = record.pending_order.replace(order).is_some();
            record.revision = record.revision.saturating_add(1);
            Ok(OrderOutcome::Queued {
                reason: if reason == "Busy." {
                    "busy".into()
                } else {
                    reason
                },
                replaced,
            })
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn cancel_pending(state: &mut TickState, id: SettlementId) -> Result<(), String> {
    let record = state
        .settlements
        .get_mut(id)
        .ok_or("Settlement not found.")?;
    record
        .pending_order
        .take()
        .ok_or("No pending construction order.")?;
    record.revision = record.revision.saturating_add(1);
    Ok(())
}

pub(crate) fn pending_reason(state: &TickState, id: SettlementId) -> Option<String> {
    let record = state.settlements.get(id)?;
    let order = record.pending_order.as_ref()?;
    if claim_slot(state, id).is_err() {
        return Some("busy".into());
    }
    let blueprint = blueprints::get(&state.registry, &order.blueprint)?;
    let level = blueprint.level(order.level)?;
    if let Some(reason) = funding_error(state, id, &level.materials) {
        return Some(reason);
    }
    Some(
        if order.building.is_none() {
            "waiting for a site"
        } else {
            "ready"
        }
        .into(),
    )
}

/// Returns true whenever the pending order owns this planning turn.
fn plan_order(state: &mut TickState, id: SettlementId) -> bool {
    let Some(record) = state.settlements.get(id) else {
        return false;
    };
    let Some(order) = &record.pending_order else {
        return false;
    };
    let level_number = if let Some(building) = order.building {
        let target = record.buildings.get(&building);
        let next = target.and_then(|target| {
            blueprints::get(&state.registry, &order.blueprint)
                .and_then(|blueprint| blueprint.level(target.level + 1))
                .map(|_| target.level + 1)
        });
        let Some(next) = next else {
            let blueprint = order.blueprint.clone();
            cancel_pending(state, id).expect("pending order");
            notify_owner(
                state,
                id,
                format!("Dropped {blueprint} order: building gone or no upgrade available."),
            );
            return false;
        };
        next
    } else {
        1
    };
    if claim_slot(state, id).is_err() {
        return true;
    }
    let Some(level) =
        blueprints::get(&state.registry, &order.blueprint).and_then(|bp| bp.level(level_number))
    else {
        return true;
    };
    if funding_error(state, id, &level.materials).is_some() {
        return true;
    }
    let mut order = state
        .settlements
        .get_mut(id)
        .expect("pending settlement")
        .pending_order
        .take()
        .expect("pending order");
    order.level = level_number;
    let cursor = state
        .settlements
        .get(id)
        .expect("pending settlement")
        .construction_cursor;
    if start_order(state, id, &order, Some(cursor)).is_err() {
        state
            .settlements
            .get_mut(id)
            .expect("pending settlement")
            .pending_order = Some(order);
        return true;
    }
    notify_owner(state, id, OrderOutcome::Started.message(&order.blueprint));
    true
}

fn notify_owner(state: &mut TickState, id: SettlementId, message: String) {
    let record = state.settlements.get(id).expect("settlement");
    let owner = record
        .owner
        .as_deref()
        .and_then(|owner| owner.parse::<uuid::Uuid>().ok());
    let connection = owner.and_then(|uuid| {
        state
            .connections
            .all_connections()
            .find(|connection| state.connections.uuid_of(*connection) == Some(uuid))
    });
    if let Some(connection) = connection {
        state.push_player(connection, crate::boundary::PlayerChange::Chat { message });
    }
}

/// `Busy.` unless the active project is autonomous and has placed nothing.
/// Returns whether such a project must be cancelled to admit the order.
fn claim_slot(state: &TickState, id: SettlementId) -> Result<bool, String> {
    let record = state.settlements.get(id).ok_or("Settlement not found.")?;
    let Some(project) = &record.project else {
        return Ok(false);
    };
    let untouched = !project.ordered
        && executor::status(state, id, &project.structure).is_ok_and(|snapshot| {
            snapshot.current_stage_index == 0 && snapshot.completed_work_units == 0
        });
    if untouched {
        Ok(true)
    } else {
        Err("Busy.".into())
    }
}

/// Cancel the untouched autonomous project: block stock and builder carry
/// stay where they are, nothing is debited or placed.
fn preempt(state: &mut TickState, id: SettlementId) -> Result<(), String> {
    let Some((handle, borrowed)) = state
        .settlements
        .get(id)
        .and_then(|record| record.project.as_ref())
        .map(|project| (project.structure.clone(), project.borrowed_builder))
    else {
        return Ok(());
    };
    executor::cancel_structure(state, id, &handle)
        .map_err(|_| "Construction admission failed.".to_string())?;
    if borrowed.is_some() {
        release_builder(state, id, borrowed);
    }
    let record = state.settlements.get_mut(id).expect("preempted settlement");
    record.project = None;
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
    let partial = candidates
        .first()
        .map(|(species, _)| *species)
        .filter(|_| allow_partial);
    candidates
        .into_iter()
        .find_map(|(species, _)| {
            let bill = species_bill(state, materials, species)?;
            super::construction_crafting::can_fund(state, &bill, images).then_some(species)
        })
        .or(partial)
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
    colors[..count].iter().copied().find_map(|color| {
        let bill = bed_bill(state, materials, color)?;
        select_species(state, &bill, images, false).map(|species| (species, color))
    })
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
    let active_bill = record
        .project
        .as_ref()
        .and_then(|_| active_project_bill(state, id));
    let funded = |images: &[super::inventory::Image]| {
        if record.project.is_some() {
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
    // Requirements can each fit separately while sharing the same finite logs
    // or recipe surplus. Only the whole-project proof authorizes admission.
    Some("not enough: shared recipe ingredients".into())
}

/// Owner/operator entrypoints check `can_manage`; admission starts the sole
/// funded project immediately, without withdrawing or placing any material.
fn start_build(
    state: &mut TickState,
    id: SettlementId,
    blueprint: &str,
    planning_cursor: Option<u32>,
) -> Result<(), String> {
    let preempting = claim_slot(state, id)?;
    if preempting {
        // Materials the autonomous project's builder already hauled count as stock.
        executor::return_carry(state, id);
    }
    let registry = std::sync::Arc::clone(&state.registry);
    let blueprint = blueprints::get(&registry, blueprint).ok_or("Unknown blueprint.")?;
    let level = blueprint.level(1).ok_or("Blueprint has no first level.")?;
    if let Some(error) = funding_error(state, id, &level.materials) {
        return Err(error);
    }
    // The untouched autonomous site is released first so the order may take
    // its footprint; an unplaceable order leaves the planner to replan.
    if preempting {
        preempt(state, id)?;
    }
    let record = state.settlements.get(id).expect("claimed settlement");
    let images = executor::source_images(state, id);
    // A direct owner request searches once for immediate admission. Retried
    // pending work shares the autonomous planner's bounded, resumable scan.
    let (start, attempts) = planning_cursor.map_or((0, u32::MAX), |cursor| (cursor, SITE_ATTEMPTS));
    let forest_misses = state
        .settlements
        .forest
        .lock()
        .expect("forest cache poisoned")
        .misses();
    let (site, cursor) = site_project(
        state,
        record,
        std::iter::once(blueprint),
        start,
        attempts,
        &images,
    );
    let Some(mut site) = site else {
        if planning_cursor.is_none()
            && state
                .settlements
                .forest
                .lock()
                .expect("forest cache poisoned")
                .misses()
                != forest_misses
        {
            return Err("waiting for forest evidence".into());
        }
        if planning_cursor.is_some() {
            let record = state.settlements.get_mut(id).expect("planned settlement");
            record.construction_cursor = cursor;
            record.revision = record.revision.saturating_add(1);
        }
        return Err(
            "No free loaded construction site is available within the material budget.".into(),
        );
    };
    site.project.ordered = true;
    if !begin_cells(state, site.project, site.bounds, site.cells) {
        return Err("Construction admission failed.".into());
    }
    state
        .settlements
        .get_mut(id)
        .expect("admitted settlement")
        .construction_cursor = cursor;
    Ok(())
}

/// Admit a funded upgrade through the canonical structure executor.
fn start_upgrade(
    state: &mut TickState,
    id: SettlementId,
    building: BuildingId,
) -> Result<(), String> {
    let preempting = claim_slot(state, id)?;
    if preempting {
        executor::return_carry(state, id);
    }
    let record = state.settlements.get(id).expect("claimed settlement");
    let building = record
        .buildings
        .get(&building)
        .ok_or("Building not found.")?;
    let (blueprint, origin, rotation) = upgrade_blueprint(state, id, building)
        .ok_or("No blueprint upgrade available for this building.")?;
    let level = building.level + 1;
    if let Some(error) = funding_error(
        state,
        id,
        &blueprint.level(level).expect("upgrade level").materials,
    ) {
        return Err(error);
    }
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
        borrowed_builder: None,
    };
    if preempting {
        preempt(state, id)?;
    }
    if !begin(state, project, &species, color) {
        return Err("Construction admission failed.".into());
    }
    Ok(())
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
const AUTONOMOUS_KINDS: usize = 9;

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
        (BuildingKind::House, beds < residents),
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
        (
            BuildingKind::Wall,
            residents >= 10 && !has(BuildingKind::Wall),
        ),
    ];
    kinds
        .into_iter()
        .filter_map(|(kind, needed)| needed.then_some(kind))
}

fn candidate_kinds<'a>(
    state: &'a TickState,
    record: &'a SettlementRecord,
    images: &'a [super::inventory::Image],
) -> impl Iterator<Item = BuildingKind> + 'a {
    let _ = images;
    needed_kinds(record, state.settlements.member_count(record.id))
}

pub(crate) fn sources(state: &TickState, id: SettlementId) -> Vec<[i32; 3]> {
    buildings::storage_points(state, id)
}

pub(super) fn clearable(state: &TickState, block: u32) -> bool {
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
fn excavatable(state: &TickState, block: u32) -> bool {
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

const SITE_VARIANCE: i32 = 3;

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
    let max: [i32; 3] = std::array::from_fn(|axis| origin[axis] + i32::from(size[axis]) - 1);
    let mut minimum = origin[1];
    let mut maximum = origin[1];
    if record.buildings.values().any(|building| {
        [0, 2].into_iter().all(|axis| {
            origin[axis] <= building.bounds.max[axis] + 1
                && building.bounds.min[axis] <= max[axis] + 1
        })
    }) {
        return false;
    }
    for x in origin[0]..=max[0] {
        for z in origin[2]..=max[2] {
            if [x, max[1], z]
                .into_iter()
                .zip(record.anchor)
                .map(|(value, anchor)| (i64::from(value) - i64::from(anchor)).pow(2))
                .sum::<i64>()
                > i64::from(record.radius).pow(2)
            {
                return false;
            }
            let Some(height) = surface_y(state, record.anchor[1], x, z) else {
                return false;
            };
            minimum = minimum.min(height);
            maximum = maximum.max(height);
            if maximum - minimum > SITE_VARIANCE {
                return false;
            }
            let ground = state
                .world
                .physics_block_at(x, height, z)
                .expect("loaded surface");
            if !floor_replaceable(state, ground) {
                return false;
            }
            for y in height.min(origin[1])..=max[1].max(height) {
                let Some(block) = state.world.physics_block_at(x, y, z) else {
                    return false;
                };
                if state.block_entities.contains_key(&(x, y, z))
                    || (super::forestry::tree(state, block)
                        && !super::forestry::clearable(state, [x, y, z]))
                    || (y == origin[1] && !floor_replaceable(state, block))
                    || (y > origin[1]
                        && !clearable(state, block)
                        && !excavatable(state, block)
                        && !super::forestry::clearable(state, [x, y, z]))
                    || (y > height
                        && y <= origin[1]
                        && !clearable(state, block)
                        && !super::forestry::clearable(state, [x, y, z]))
                {
                    return false;
                }
            }
        }
    }
    !executor::area_reserved(
        state,
        ScriptWorldBounds::new([origin[0], minimum, origin[2]], max).expect("blueprint bounds"),
    )
}

fn planned_cells(
    state: &TickState,
    project: &Project,
    species: &str,
) -> Option<(ScriptWorldBounds, Vec<executor::PlannedCell>)> {
    let blueprint = blueprints::get(&state.registry, &project.blueprint)?;
    let level = blueprint.level(project.level)?;
    let rotated = level.rotated(Rotation::from_quarter_turns(project.rotation));
    let mut quota = level.materials.clone();
    let mut cells: Vec<_> = rotated
        .removals
        .iter()
        .map(|offset| executor::PlannedCell {
            at: rotated.world(project.origin, *offset),
            state: state.registry.states.air,
            resource: None,
            quantity: 0,
        })
        .collect();
    let size = blueprint.footprint(Rotation::from_quarter_turns(project.rotation));
    let mut bottom = project.origin[1];
    if project.building.is_none() {
        let dirt = state
            .registry
            .blocks
            .block(&mc_data::Identifier::parse("minecraft:dirt").ok()?)?
            .default
            .0;
        for x in project.origin[0]..project.origin[0] + i32::from(size[0]) {
            for z in project.origin[2]..project.origin[2] + i32::from(size[2]) {
                let ground = surface_y(state, project.origin[1], x, z)?;
                for y in ground + 1..=project.origin[1] {
                    // The authored floor already pays for its own cells.
                    if y == project.origin[1]
                        && rotated
                            .blocks
                            .iter()
                            .any(|(offset, _)| rotated.world(project.origin, *offset) == [x, y, z])
                    {
                        continue;
                    }
                    bottom = bottom.min(y);
                    cells.push(executor::PlannedCell {
                        at: [x, y, z],
                        state: dirt,
                        resource: Some("minecraft:dirt".into()),
                        quantity: 1,
                    });
                }
            }
        }
    }
    for x in project.origin[0]..project.origin[0] + i32::from(size[0]) {
        for z in project.origin[2]..project.origin[2] + i32::from(size[2]) {
            for y in project.origin[1] + 1..project.origin[1] + i32::from(size[1]) {
                let block = state.world.physics_block_at(x, y, z)?;
                if block != state.registry.states.air
                    && (clearable(state, block)
                        || (project.building.is_none() && excavatable(state, block)))
                    && !cells.iter().any(|cell| cell.at == [x, y, z])
                {
                    cells.push(executor::PlannedCell {
                        at: [x, y, z],
                        state: state.registry.states.air,
                        resource: None,
                        quantity: 0,
                    });
                }
            }
        }
    }
    for (offset, block) in &rotated.blocks {
        let placed = *block;
        let (resource, quantity) =
            if let Some((item, quantity)) = blueprints::material_for(&state.registry, *block) {
                let (_, remaining) = quota.iter_mut().find(|(id, _)| *id == item)?;
                *remaining = remaining.checked_sub(quantity)?;
                (
                    Some(state.registry.items.name_of(item)?.to_string()),
                    quantity,
                )
            } else {
                (None, 0)
            };
        cells.push(executor::PlannedCell {
            at: rotated.world(project.origin, *offset),
            state: placed.0,
            resource,
            quantity,
        });
    }
    // Catalog materials must map exactly to physical cells; never build free
    // blocks because a blueprint cannot be funded by its declared stock.
    if quota.iter().any(|(_, count)| *count != 0) {
        return None;
    }
    apply_species(state, &mut cells, species)?;
    if project.building.is_none() {
        let mut preparation = super::forestry::cells(state, project, size)?;
        if !preparation.is_empty() {
            let removed: std::collections::BTreeSet<_> =
                preparation.iter().map(|cell| cell.at).collect();
            cells.retain(|cell| {
                cell.state != state.registry.states.air || !removed.contains(&cell.at)
            });
            preparation.append(&mut cells);
            cells = preparation;
        }
    }
    // Include the whole placement extent (roof crowns/yards) in executor
    // interference fencing; certification uses the narrower authored bounds.
    let mut min = [project.origin[0], bottom, project.origin[2]];
    let mut max = std::array::from_fn(|axis| project.origin[axis] + i32::from(size[axis]) - 1);
    for cell in &cells {
        for axis in 0..3 {
            min[axis] = min[axis].min(cell.at[axis]);
            max[axis] = max[axis].max(cell.at[axis]);
        }
    }
    let bounds = ScriptWorldBounds::new(min, max).ok()?;
    Some((bounds, cells))
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
    let Some((bounds, mut cells)) = planned_cells(state, &project, species) else {
        return false;
    };
    if apply_bed_color(state, &mut cells, color).is_none() {
        return false;
    }
    begin_cells(state, project, bounds, cells)
}

fn begin_cells(
    state: &mut TickState,
    mut project: Project,
    bounds: ScriptWorldBounds,
    cells: Vec<executor::PlannedCell>,
) -> bool {
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
    record.project = Some(project);
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
    let blueprint = blueprints::all(&state.registry)
        .iter()
        .find(|blueprint| blueprint.kind == building.kind)?;
    let current = blueprint.level(building.level)?;
    Rotation::ALL.into_iter().find_map(|rotation| {
        let rotated = current.rotated(rotation);
        let corner = rotated.bounds([0, 0, 0]).min;
        let origin: [i32; 3] = std::array::from_fn(|axis| building.bounds.min[axis] - corner[axis]);
        (rotated.bounds(origin) == building.bounds
            && executor::built_at(state, id, &blueprint.id, origin))
        .then_some((blueprint, origin, rotation.0))
    })
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

fn upgrade_project(
    state: &TickState,
    record: &SettlementRecord,
    images: &[super::inventory::Image],
) -> Option<(Project, String, u8)> {
    record.buildings.values().find_map(|building| {
        let (blueprint, origin, rotation) = upgrade_blueprint(state, record.id, building)?;
        let level = building.level + 1;
        let (species, color) =
            funded_variant(state, record.id, &blueprint.level(level)?.materials, images)?;
        let species = species.to_owned();
        Some((
            Project {
                settlement: record.id,
                blueprint: blueprint.id.clone(),
                level,
                origin,
                rotation,
                building: Some(building.id),
                structure: String::new(),
                ordered: false,
                borrowed_builder: None,
            },
            species,
            color,
        ))
    })
}

/// Lot candidates examined per autonomous planning pass; an owner order
/// examines every candidate of the street plan once.
const SITE_ATTEMPTS: u32 = 64;
/// Ground may differ this much from the bell's elevation and still be a site.
pub(super) const SITE_BAND: i32 = 8;

/// Highest standing surface in the band around the bell for one column: a solid
/// block with clearable space directly above it. Hillside villages have no
/// flat ground at the bell's own elevation, so sites follow the terrain.
pub(super) fn surface_y(state: &TickState, anchor_y: i32, x: i32, z: i32) -> Option<i32> {
    (anchor_y - SITE_BAND..=anchor_y + SITE_BAND)
        .rev()
        .find(|&y| {
            state.world.physics_block_at(x, y, z).is_some_and(|ground| {
                !super::forestry::tree(state, ground)
                    && state
                        .registry
                        .collision_boxes
                        .get(ground as usize)
                        .is_some_and(|boxes| !boxes.is_empty())
            }) && state
                .world
                .physics_block_at(x, y + 1, z)
                .is_some_and(|above| {
                    clearable(state, above) || super::forestry::clearable(state, [x, y + 1, z])
                })
        })
}

struct FundedSite {
    project: Project,
    bounds: ScriptWorldBounds,
    cells: Vec<executor::PlannedCell>,
}

/// Search the funded kinds in priority order over one shared street walk.
/// Divide the admission's total attempt budget among them: an unplaceable
/// early kind must not hide a later legal lot. Each kind resumes at the same
/// frontage cursor. Owner requests pass one kind and retain their full scan.
fn site_project<'a>(
    state: &TickState,
    record: &SettlementRecord,
    blueprints: impl ExactSizeIterator<Item = &'a blueprints::Blueprint>,
    start: u32,
    attempts: u32,
    images: &[super::inventory::Image],
) -> (Option<FundedSite>, u32) {
    let share = attempts / blueprints.len().max(1) as u32;
    let mut network = layout::Network::for_construction(state, record);
    let fronts = network.frontage();
    let count = u32::try_from(fronts.len()).unwrap_or(u32::MAX);
    if count == 0 {
        return (None, 0);
    }
    let mut index = start % count;
    for blueprint in blueprints {
        index = start % count;
        for _ in 0..share.min(count) {
            let (front, normal) = fronts[index as usize];
            index = (index + 1) % count;
            let Some(lot) = network.lot(blueprint, front, normal) else {
                continue;
            };
            let size = blueprint.footprint(Rotation::from_quarter_turns(lot.rotation));
            if !site_free(state, record, lot.origin, size) {
                continue;
            }
            let project = Project {
                settlement: record.id,
                blueprint: blueprint.id.clone(),
                level: 1,
                origin: lot.origin,
                rotation: lot.rotation,
                building: None,
                structure: String::new(),
                ordered: false,
                borrowed_builder: None,
            };
            if let Some((bounds, mut cells)) = planned_cells(state, &project, "oak")
                && let Some(bill) = cell_bill(state, &cells)
                && let Some((species, color)) = funded_variant(state, record.id, &bill, images)
                && apply_species(state, &mut cells, species).is_some()
                && apply_bed_color(state, &mut cells, color).is_some()
            {
                return (
                    Some(FundedSite {
                        project,
                        bounds,
                        cells,
                    }),
                    0,
                );
            }
        }
    }
    (None, index)
}

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
        borrowed_builder: None,
    };
    for tile in &served {
        if let Some(work) = network.road_work(tile)
            && !executor::area_reserved(state, work.bounds)
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
            && !executor::area_reserved(state, work.bounds)
            && let Some(bill) = cell_bill(state, &work.cells)
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

fn plan(state: &mut TickState, id: SettlementId) {
    if furnace::plan(state, id) {
        return;
    }
    if plan_order(state, id) {
        return;
    }
    let Some(record) = state.settlements.get(id) else {
        return;
    };
    if record.project.is_some() {
        return;
    }
    let residents = state.settlements.member_count(id);
    if residents == 0 {
        return;
    }
    // Probe one canonical storage image set; an unfundable early autonomous
    // need cannot monopolize the single project slot.
    let images = executor::source_images(state, id);
    // Keep the priority material worker out of optional street work, not out
    // of the entire planner: another resident can build a funded later kind.
    let gathering = gatherer_demand(state, record, &images)
        .is_some_and(|profession| has_staffed_gatherer(state, id, profession));
    let catalog = blueprints::all(&state.registry);
    let mut funded = [None; AUTONOMOUS_KINDS];
    let mut funded_count = 0;
    for kind in candidate_kinds(state, record, &images) {
        let Some(blueprint) = catalog.iter().find(|blueprint| blueprint.kind == kind) else {
            continue;
        };
        if blueprint
            .level(1)
            .is_some_and(|level| funded_variant(state, id, &level.materials, &images).is_some())
        {
            funded[funded_count] = Some(blueprint);
            funded_count += 1;
        }
    }
    if funded_count != 0 {
        let (site, cursor) = site_project(
            state,
            record,
            funded
                .into_iter()
                .take(funded_count)
                .map(|blueprint| blueprint.expect("funded prefix")),
            record.construction_cursor,
            SITE_ATTEMPTS,
            &images,
        );
        let record = state.settlements.get_mut(id).expect("planned settlement");
        record.construction_cursor = cursor;
        record.revision = record.revision.saturating_add(1);
        if let Some(site) = site {
            begin_cells(state, site.project, site.bounds, site.cells);
            return;
        }
    } else if let Some((project, species, color)) = upgrade_project(state, record, &images)
        && begin(state, project, &species, color)
    {
        return;
    }
    if gathering {
        return;
    }
    // Free work fills every pass that starts no building or upgrade: seed
    // gathering while the first Farm waits on wheat seeds, then street work.
    let Some(record) = state.settlements.get(id) else {
        return;
    };
    if let Some(site) = meadow_site(state, record, residents, &images)
        .or_else(|| street_site(state, record, &images))
    {
        begin_cells(state, site.project, site.bounds, site.cells);
    }
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
    let catalog = blueprints::all(&state.registry);
    let level = catalog
        .iter()
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
            borrowed_builder: None,
        },
        bounds: work.bounds,
        cells: work.cells,
    })
}

/// Chunk-local resident index, with a hard admission cap. Other mob populations
/// never contribute to planner or builder search cost.
pub(crate) fn residents(
    state: &TickState,
    id: SettlementId,
) -> impl Iterator<Item = crate::entities::EntitySlot> + '_ {
    let (anchor, radius) = state.settlements.get(id).map_or(([0; 3], 0), |record| {
        (record.anchor, record.radius.min(256) as i32)
    });
    let min_x = (anchor[0] - radius).div_euclid(16);
    let max_x = (anchor[0] + radius).div_euclid(16);
    let min_z = (anchor[2] - radius).div_euclid(16);
    let max_z = (anchor[2] + radius).div_euclid(16);
    (min_x..=max_x)
        .flat_map(move |x| {
            (min_z..=max_z).flat_map(move |z| {
                state
                    .entities
                    .villager_chunks()
                    .get(&crate::boundary::ChunkPos { x, z })
                    .into_iter()
                    .flatten()
                    .filter_map(|id| state.entities.slot_of(*id))
            })
        })
        .take(1024)
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

/// Residents tried per pass; the rest wait for the next planning pass.
const BUILDER_CANDIDATES: usize = 8;

/// Release a builder outside simulation, or an on-shift builder that cannot
/// walk to storage, so an active reachable resident can take the project.
fn release_trapped_builder(state: &mut TickState, id: SettlementId) -> bool {
    let incumbent = residents(state, id).find(|&slot| {
        state.entities.body(slot).alive
            && !super::economy::worker_reserved(state, Some(id), state.entities.row(slot).uuid)
            && state.entities.row(slot).villager.as_ref().is_some_and(|v| {
                v.settlement == Some(id)
                    && v.age >= 0
                    && v.conversion_time.is_none()
                    && v.data.profession == Profession::Builder
            })
    });
    let Some(slot) = incumbent else {
        return false;
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
        && (!on_shift || reaches_storage(state, id, slot, &mut cache, &mut path))
    {
        return true;
    }
    !release_incumbent(state, id, slot)
}

/// Detach the builder in `slot` from the project, returning a borrowed gatherer
/// to its profession. False when the builder carries project stock it must
/// first deliver.
fn release_incumbent(
    state: &mut TickState,
    id: SettlementId,
    slot: crate::entities::EntitySlot,
) -> bool {
    let borrowed = state
        .settlements
        .get(id)
        .and_then(|record| record.project.as_ref())
        .and_then(|project| project.borrowed_builder)
        .filter(|(uuid, _)| *uuid == state.entities.row(slot).uuid);
    let released = detach(
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .expect("builder incumbent"),
    );
    if released && let Some((_, profession)) = borrowed {
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .data
            .profession = profession;
        let record = state.settlements.get_mut(id).unwrap();
        record.project.as_mut().unwrap().borrowed_builder = None;
        record.revision = record.revision.saturating_add(1);
    }
    crate::villager::sync_appearance(state, slot);
    released
}

/// Builders work only 2000..9000 of the day. A project whose work counter
/// stands still for this many shift ticks (150 s of game time) has a builder
/// that cannot make progress (seed 24: a stationary zombie within eight blocks
/// of the only road to the lot kept the builder fleeing and returning for
/// 15,000 ticks, 35 of 239 cells, and held the single project slot).
const BUILDER_STALL_SHIFT_TICKS: u64 = 3_000;
/// The replaced builder is chosen last for this long.
const BUILDER_SHUN_TICKS: u64 = 24_000;
/// Longest gap between two planning passes counted as shift time.
const BUILDER_WATCH_STEP: u64 = 100;

/// Runtime watch of the active project's work counter; derived, never persisted.
#[derive(Debug)]
pub(crate) struct BuilderWatch {
    progress: (String, u32, u64),
    seen: u64,
    /// Shift ticks without progress.
    idle: u64,
    /// Replaced builders and the tick until which they are chosen last.
    shunned: Vec<(uuid::Uuid, u64)>,
}

/// Replace the builder of a running project that has not advanced a cell for
/// [`BUILDER_STALL_SHIFT_TICKS`] of shift: another resident takes it, whose
/// route may not cross whatever stopped the first.
fn release_stalled_builder(state: &mut TickState, id: SettlementId) {
    let Some(project) = state
        .settlements
        .get(id)
        .and_then(|record| record.project.clone())
    else {
        state.settlements.builder_watch.remove(&id);
        return;
    };
    let Ok(snapshot) = executor::status(state, id, &project.structure) else {
        return;
    };
    let progress = (
        project.structure.clone(),
        snapshot.current_stage_index,
        snapshot.completed_work_units,
    );
    let tick = state.tick;
    let on_shift = (2_000..9_000).contains(&state.day_time.rem_euclid(24_000));
    let watch = state
        .settlements
        .builder_watch
        .entry(id)
        .or_insert_with(|| BuilderWatch {
            progress: progress.clone(),
            seen: tick,
            idle: 0,
            shunned: Vec::new(),
        });
    let elapsed = tick.saturating_sub(watch.seen).min(BUILDER_WATCH_STEP);
    watch.seen = tick;
    if watch.progress != progress {
        watch.progress = progress;
        watch.idle = 0;
        return;
    }
    if snapshot.pause_reason.is_some() || !on_shift {
        return;
    }
    watch.idle += elapsed;
    if watch.idle < BUILDER_STALL_SHIFT_TICKS {
        return;
    }
    watch.idle = 0;
    let incumbent = residents(state, id).find(|&slot| {
        state.entities.body(slot).alive
            && state.entities.row(slot).villager.as_ref().is_some_and(|v| {
                v.settlement == Some(id)
                    && v.age >= 0
                    && v.conversion_time.is_none()
                    && v.data.profession == Profession::Builder
            })
    });
    let Some(slot) = incumbent else {
        return;
    };
    let uuid = state.entities.row(slot).uuid;
    if release_incumbent(state, id, slot)
        && let Some(watch) = state.settlements.builder_watch.get_mut(&id)
    {
        watch.shunned.retain(|&(_, until)| tick < until);
        watch
            .shunned
            .push((uuid, tick.saturating_add(BUILDER_SHUN_TICKS)));
    }
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
    let borrowed = state
        .settlements
        .get(id)
        .and_then(|record| record.project.as_ref())
        .and_then(|project| project.borrowed_builder);
    let Some((uuid, original)) = borrowed else {
        return;
    };
    release_builder(state, id, borrowed);
    if state.entities.slot_of_uuid(uuid).is_some_and(|slot| {
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .is_some_and(|v| v.data.profession == original)
    }) {
        let record = state.settlements.get_mut(id).expect("active settlement");
        record
            .project
            .as_mut()
            .expect("active project")
            .borrowed_builder = None;
        record.revision = record.revision.saturating_add(1);
    }
}

fn assign_builder(state: &mut TickState, id: SettlementId) {
    release_stalled_builder(state, id);
    if release_trapped_builder(state, id) {
        return;
    }
    let protected = state.settlements.get(id).and_then(|record| {
        record.project.as_ref().and_then(|project| {
            let images = executor::source_images(state, id);
            active_project_bill(state, id)
                .and_then(|bill| {
                    super::construction_crafting::needed_gatherer(state, &bill, &images)
                })
                .or_else(|| {
                    if project.ordered {
                        return None;
                    }
                    gatherer_demand(state, record, &images)
                        .filter(|profession| has_staffed_gatherer(state, id, *profession))
                })
        })
    });
    let mut candidates = Vec::new();
    for slot in residents(state, id) {
        if !resident_simulated(state, slot)
            || super::economy::worker_reserved(state, Some(id), state.entities.row(slot).uuid)
        {
            continue;
        }
        let body = state.entities.body(slot);
        let Some(v) = state.entities.row(slot).villager.as_ref() else {
            continue;
        };
        if protected.is_some_and(|needed| {
            v.job_site.is_some_and(|job| {
                buildings::job_profession(&state.settlements, Some(id), job) == Some(needed)
            })
        }) {
            continue;
        }
        if body.alive && v.settlement == Some(id) && v.age >= 0 && v.conversion_time.is_none() {
            if v.data.profession == Profession::Builder {
                return;
            }
            let priority = match v.data.profession {
                Profession::None => Some(0),
                Profession::Mason if !super::allocator::traded(v) => Some(1),
                profession
                    if !super::allocator::traded(v)
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
                    if !super::allocator::traded(v) && Some(v.data.profession) != protected =>
                {
                    Some(3)
                }
                _ => None,
            };
            if let Some(priority) = priority {
                let shunned = state
                    .settlements
                    .builder_watch
                    .get(&id)
                    .is_some_and(|watch| {
                        watch.shunned.iter().any(|&(uuid, until)| {
                            uuid == state.entities.row(slot).uuid && state.tick < until
                        })
                    });
                candidates.push((priority + if shunned { 10 } else { 0 }, slot));
            }
        }
    }
    // Stable: residents of equal priority keep their index order.
    candidates.sort_by_key(|&(priority, _)| priority);
    let mut cache = super::allocator::PoiCache::default();
    let mut path = Vec::new();
    let Some(slot) = candidates
        .into_iter()
        .take(BUILDER_CANDIDATES)
        .map(|(_, slot)| slot)
        .find(|&slot| reaches_storage(state, id, slot, &mut cache, &mut path))
    else {
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
        Profession::Miner | Profession::Lumberjack => {
            Some((state.entities.row(slot).uuid, profession))
        }
        _ => None,
    };
    if let Some(record) = state.settlements.get_mut(id)
        && let Some(project) = record.project.as_mut()
        && project.borrowed_builder != borrowed
    {
        project.borrowed_builder = borrowed;
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

/// Planning keeps its one-settlement/second budget. Construction has a separate
/// four-tick admission clock, retaining the bounded round-robin settlement budget.
pub(crate) fn tick(state: &mut TickState) {
    if !state.tick.is_multiple_of(4) {
        return;
    }
    let count = state.settlements.records().count();
    if count == 0 {
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
    if state.tick.is_multiple_of(PLAN_CADENCE)
        && let Some(id) = selected(state, state.tick / PLAN_CADENCE)
    {
        if let Some(profession) = needed_gatherer(state, id) {
            super::allocator::staff_gatherer(state, id, profession);
            restore_borrowed_gatherer(state, id, profession);
            super::allocator::staff_gatherer(state, id, profession);
        }
        plan(state, id);
        if state
            .settlements
            .get(id)
            .is_some_and(|record| record.project.is_some())
        {
            assign_builder(state, id);
        }
    }
    if let Some(id) = selected(state, state.tick / 4)
        && state
            .settlements
            .get(id)
            .is_some_and(|record| record.project.is_some())
    {
        crate::villager::building::tick(state, id);
    }
}

fn release_builder(
    state: &mut TickState,
    id: SettlementId,
    borrowed: Option<(uuid::Uuid, Profession)>,
) {
    let builder = residents(state, id).find(|slot| {
        let row = state.entities.row(*slot);
        borrowed.is_none_or(|(uuid, _)| row.uuid == uuid)
            && row.villager.as_ref().is_some_and(|v| {
                v.settlement == Some(id) && v.data.profession == Profession::Builder
            })
    });
    if let Some(slot) = builder
        && let Some(v) = state.entities.row_mut(slot).villager.as_mut()
        && detach(v)
    {
        if let Some((_, profession)) = borrowed {
            v.data.profession = profession;
        }
        crate::villager::sync_appearance(state, slot);
    }
}

pub(crate) fn finish_project(state: &mut TickState, id: SettlementId) -> bool {
    let Some(project) = state
        .settlements
        .get(id)
        .and_then(|record| record.project.clone())
    else {
        return false;
    };
    if !executor::status(state, id, &project.structure)
        .is_ok_and(|snapshot| snapshot.state == ScriptStructureState::Committed)
    {
        return false;
    }
    if project.blueprint == furnace::NAME || layout::infra(&project.blueprint) {
        // Infrastructure work is temporary employment.
        release_builder(state, id, project.borrowed_builder);
        let record = state.settlements.get_mut(id).expect("finished settlement");
        if project.blueprint == furnace::NAME {
            record.furnace_workstation = Some(project.structure);
        }
        record.project = None;
        record.revision = record.revision.saturating_add(1);
        return true;
    }
    let Some(blueprint) = blueprints::get(&state.registry, &project.blueprint) else {
        return false;
    };
    let Some(level) = blueprint.level(project.level) else {
        return false;
    };
    let bounds = level
        .rotated(Rotation::from_quarter_turns(project.rotation))
        .bounds(project.origin);
    let Ok(building) = buildings::certify(state, id, blueprint.kind, bounds) else {
        return false;
    };
    if project
        .building
        .is_some_and(|expected| expected != building)
    {
        return false;
    }
    if project.level > 1 && buildings::upgrade(state, building).is_err() {
        return false;
    }
    let record = state.settlements.get_mut(id).expect("certified settlement");
    record.project = None;
    record.revision = record.revision.saturating_add(1);
    if project.borrowed_builder.is_some() {
        release_builder(state, id, project.borrowed_builder);
    }
    true
}

/// One needed blueprint kind whose level-one bill the settlement cannot fund.
#[derive(Debug)]
pub(crate) struct WaitingKind {
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

/// Remaining physical project requirements after subtracting exact items in
/// builder carry. Ingredients in carry are not craftable until deposited.
fn active_project_bill(state: &TickState, id: SettlementId) -> Option<Vec<(u32, u32)>> {
    let project = state.settlements.get(id)?.project.as_ref()?;
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
    for slot in residents(state, id) {
        let Some(v) = state.entities.row(slot).villager.as_ref() else {
            continue;
        };
        if v.data.profession != Profession::Builder {
            continue;
        }
        for stack in v.inventory.iter().flatten().filter(|stack| stack.count > 0) {
            if let Some((_, count)) = bill.iter_mut().find(|(item, _)| *item == stack.item_id) {
                *count = count.saturating_sub(stack.count as u32);
            }
        }
    }
    bill.retain(|(_, count)| *count != 0);
    Some(bill)
}

/// One real bill drives gathering, including a previously funded project
/// whose stock was spent or withdrawn after admission.
pub(crate) fn needed_gatherer(state: &TickState, id: SettlementId) -> Option<Profession> {
    let record = state.settlements.get(id)?;
    let images = executor::source_images(state, id);
    if record.project.is_some() {
        let bill = active_project_bill(state, id)?;
        super::construction_crafting::needed_gatherer(state, &bill, &images)
    } else {
        gatherer_demand(state, record, &images)
    }
}

/// The active blueprint's fixed wood family while it needs more logs.
pub(crate) fn active_project_wood(state: &TickState, id: SettlementId) -> Option<&str> {
    let bill = active_project_bill(state, id)?;
    let images = executor::source_images(state, id);
    if super::construction_crafting::needed_gatherer(state, &bill, &images)
        != Some(Profession::Lumberjack)
    {
        return None;
    }
    bill.into_iter()
        .find_map(|(item, _)| wood_species(state, item))
}

fn gatherer_demand(
    state: &TickState,
    record: &SettlementRecord,
    images: &[super::inventory::Image],
) -> Option<Profession> {
    let (name, level) = if let Some(order) = &record.pending_order {
        (order.blueprint.as_str(), order.level)
    } else {
        let kind = candidate_kinds(state, record, images).next()?;
        (
            blueprints::all(&state.registry)
                .iter()
                .find(|blueprint| blueprint.kind == kind)?
                .id
                .as_str(),
            1,
        )
    };
    let level = blueprints::get(&state.registry, name)?.level(level)?;
    let bill = selected_bill(state, record.id, &level.materials, images)?;
    super::construction_crafting::needed_gatherer(state, &bill, images)
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
    if let Some(order) = &record.pending_order {
        return blueprints::get(&state.registry, &order.blueprint)
            .and_then(|blueprint| blueprint.level(order.level))
            .and_then(|level| selected_bed_bill(state, id, &level.materials, &images))
            .map_or(0, |bill| demand(&bill));
    }
    let active = record.project.as_ref().map_or(0, |project| {
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
            })
    });
    let catalog = blueprints::all(&state.registry);
    candidate_kinds(state, record, &images)
        .filter_map(|kind| catalog.iter().find(|blueprint| blueprint.kind == kind))
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
    if let Some(project) = &record.project {
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
    if let Some(order) = &record.pending_order {
        return blueprints::get(&state.registry, &order.blueprint)
            .and_then(|blueprint| blueprint.level(order.level))
            .and_then(|level| selected_bed_bill(state, id, &level.materials, &images))
            .is_some_and(|bill| availability::needs_wool(state, &bill, &images));
    }
    let catalog = blueprints::all(&state.registry);
    candidate_kinds(state, record, &images)
        .filter_map(|kind| catalog.iter().find(|blueprint| blueprint.kind == kind))
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
    if let Some(project) = &record.project
        && let Ok(snapshot) = executor::status(state, id, &project.structure)
    {
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
    if let Some(order) = &record.pending_order {
        let level = blueprints::get(&state.registry, &order.blueprint)?.level(order.level)?;
        return color_for(&selected_bed_bill(state, id, &level.materials, &images)?);
    }
    let catalog = blueprints::all(&state.registry);
    candidate_kinds(state, record, &images)
        .filter_map(|kind| catalog.iter().find(|blueprint| blueprint.kind == kind))
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
    if let Some(order) = &record.pending_order {
        return blueprints::get(&state.registry, &order.blueprint)
            .and_then(|blueprint| blueprint.level(order.level))
            .is_some_and(|level| {
                funded_variant(state, record.id, &level.materials, images).is_some()
            });
    }
    let catalog = blueprints::all(&state.registry);
    candidate_kinds(state, record, images).any(|kind| {
        catalog
            .iter()
            .find(|blueprint| blueprint.kind == kind)
            .and_then(|blueprint| blueprint.level(1))
            .is_some_and(|level| {
                funded_variant(state, record.id, &level.materials, images).is_some()
            })
    }) || upgrade_project(state, record, images).is_some()
}

/// Needed authored kinds the settlement cannot fund yet, one entry per kind.
/// Formatting (translation, per-line cap) belongs to the dialog.
pub(crate) fn waiting(state: &TickState, id: SettlementId) -> Vec<WaitingKind> {
    let Some(record) = state.settlements.get(id) else {
        return Vec::new();
    };
    let images = executor::source_images(state, id);
    let catalog = blueprints::all(&state.registry);
    let active = record
        .project
        .as_ref()
        .map(|project| project.blueprint.as_str());
    let mut kinds = Vec::new();
    let pending = record.pending_order.as_ref();
    let queued = pending.and_then(|order| {
        blueprints::get(&state.registry, &order.blueprint).map(|blueprint| (blueprint, order.level))
    });
    let autonomous = candidate_kinds(state, record, &images)
        .filter(|_| pending.is_none())
        .filter_map(|kind| catalog.iter().find(|blueprint| blueprint.kind == kind))
        .filter(|blueprint| active != Some(blueprint.id.as_str()))
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
    let Some(project) = record.project.as_ref() else {
        return vec!["No active project. Waiting for residents, materials or a free site.".into()];
    };
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
    if let Some(project) = &record.project
        && (project.settlement != record.id
            || project.blueprint.is_empty()
            || project.blueprint.len() > 128
            || project.structure.is_empty()
            || project.structure.len() > 128
            || !(1..=3).contains(&project.level)
            || project.rotation > 3
            || project
                .building
                .is_some_and(|id| !record.buildings.contains_key(&id))
            || (project.level > 1) != project.building.is_some())
    {
        return Err("invalid settlement construction project");
    }
    if let Some(order) = &record.pending_order
        && (record.owner.is_none()
            || order.blueprint.is_empty()
            || order.blueprint.len() > 128
            || !(1..=3).contains(&order.level)
            || (order.level > 1) != order.building.is_some()
            || order
                .building
                .is_some_and(|id| id.0 >> 32 != record.id.0 || id.0 as u32 == 0))
    {
        return Err("invalid pending owner construction order");
    }
    Ok(())
}

pub(super) fn validate_pending(
    registry: &crate::registry::CoreRegistry,
    record: &SettlementRecord,
) -> Result<(), &'static str> {
    let Some(order) = &record.pending_order else {
        return Ok(());
    };
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
    Ok(())
}

#[cfg(test)]
#[path = "construction_tests.rs"]
pub(crate) mod tests;

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
#[path = "construction_site_tests.rs"]
mod site_tests;
