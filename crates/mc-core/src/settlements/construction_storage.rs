//! D54: build receiving capacity before other autonomous construction needs it.
use super::*;
use crate::settlements::inventory::Image;

pub(super) fn candidate_kinds<'a>(
    state: &'a TickState,
    record: &'a SettlementRecord,
    images: &'a [Image],
) -> impl Iterator<Item = BuildingKind> + 'a {
    let cleric = BuildingKind::Workshop(Profession::Cleric);
    let farmer = BuildingKind::Workshop(Profession::Farmer);
    // Food before capacity: a town that grows nothing cannot feed anyone, so a
    // missing farm outranks further storehouse work (it never outranks the
    // first store, and once staffed the ordinary priority table decides).
    let farm_first = has_storage(record)
        && !record
            .buildings
            .values()
            .any(|building| building.kind == farmer && !building.natural_zone);
    std::iter::once(farmer)
        .filter(move |_| farm_first)
        .chain(
            std::iter::once(BuildingKind::Storage)
                .filter(move |_| needed(state, record, images)),
        )
        .chain(
            needed_kinds(record, state.settlements.member_count(record.id)).filter(move |kind| {
                *kind != BuildingKind::Storage && !(farm_first && *kind == farmer)
            }),
        )
        .chain(
            (spanish_faction(state, record)
                && !record
                    .buildings
                    .values()
                    .any(|building| building.kind == cleric))
            .then_some(cleric),
        )
}

fn has_storage(record: &SettlementRecord) -> bool {
    record
        .buildings
        .values()
        .any(|building| building.kind == BuildingKind::Storage && !building.natural_zone)
}

/// The first civic store follows culture. Further capacity uses the ordinary
/// wood/stone warehouse, not another non-renewable Spanish civic plaza.
pub(super) fn autonomous_catalog(
    state: &TickState,
    record: &SettlementRecord,
    spanish: bool,
) -> impl Iterator<Item = &'static blueprints::Blueprint> + Clone + use<> {
    let warehouse = has_storage(record)
        .then(|| blueprints::get(&state.registry, "storage").expect("shipped storage blueprint"));
    blueprints::catalog(&state.registry, spanish).map(move |blueprint| {
        if blueprint.kind == BuildingKind::Storage {
            warehouse.unwrap_or(blueprint)
        } else {
            blueprint
        }
    })
}

pub(super) fn funded_variant<'a>(
    state: &'a TickState,
    record: &SettlementRecord,
    blueprint: &blueprints::Blueprint,
    materials: &[(u32, u32)],
    images: &[Image],
) -> Option<(&'a str, u8)> {
    if blueprint.kind != BuildingKind::Storage || !has_storage(record) {
        return super::funded_variant(state, record.id, materials, images);
    }
    // Resource-name order, not report-assigned ids: consuming a small required
    // stack can make real room for a later recipe output in a full bank.
    // The canonical species/joint funding proof and preparation fence stay intact.
    let mut ordered = materials.to_vec();
    order_by_name(state, &mut ordered);
    super::funded_variant(state, record.id, &ordered, images)
}

fn order_by_name(state: &TickState, materials: &mut [(u32, u32)]) {
    materials.sort_unstable_by_key(|(item, _)| state.registry.items.name_of(*item));
}

pub(crate) fn order_storage_haul(
    state: &TickState,
    id: SettlementId,
    blueprint: &str,
    materials: &mut [(u32, u32)],
) {
    let additional =
        is_storehouse(state, blueprint) && state.settlements.get(id).is_some_and(has_storage);
    if additional {
        order_by_name(state, materials);
    }
}

fn is_storehouse(state: &TickState, blueprint: &str) -> bool {
    blueprints::get(&state.registry, blueprint)
        .is_some_and(|blueprint| blueprint.kind == BuildingKind::Storage)
}

pub(super) fn needed(state: &TickState, record: &SettlementRecord, images: &[Image]) -> bool {
    if access::projects(record).any(|project| is_storehouse(state, &project.blueprint))
        || access::orders(record).any(|order| is_storehouse(state, &order.blueprint))
    {
        return false;
    }
    let missing = !has_storage(record);
    let total: usize = images.iter().map(|image| image.slots.len()).sum();
    let free = images
        .iter()
        .flat_map(|image| &image.slots)
        .filter(|stack| stack.is_empty())
        .count();
    missing || (total != 0 && free * 4 < total)
}

/// Some means storage owns the next autonomous admission, including its wait.
/// Owner orders have already had their existing queue turn before this call.
pub(super) fn plan(
    state: &mut TickState,
    id: SettlementId,
    budget: &mut site_turn::SiteBudget,
    images: &[Image],
) -> Option<bool> {
    let record = state.settlements.get(id)?;
    if !has_storage(record) || !needed(state, record, images) {
        return None;
    }
    if state.settlements.member_count(id) == 0
        || claim_slot(state, id).is_err()
        || unbound_project(&state.settlements, id).is_some()
        || !builder_free(state, id)
    {
        return Some(false);
    }
    let blueprint =
        catalog(state, record).find(|blueprint| blueprint.kind == BuildingKind::Storage)?;
    let mut sweeps = state.settlements.site_sweep.remove(&id).unwrap_or_default();
    let record = state.settlements.get(id).expect("planning storehouse");
    let pass = site_project(
        state,
        record,
        std::iter::once(blueprint),
        &mut sweeps,
        record.construction_cursor,
        budget.take(),
        images,
    );
    state.settlements.site_sweep.insert(id, sweeps);
    site_refusals::observe(state, id, &blueprint.id, pass.report);
    let record = state.settlements.get_mut(id).expect("planning storehouse");
    if record.construction_cursor != pass.cursor {
        record.construction_cursor = pass.cursor;
        record.revision = record.revision.saturating_add(1);
    }
    Some(
        pass.site
            .is_some_and(|site| begin_cells(state, site.project, site.bounds, site.cells)),
    )
}

/// A pressured storehouse's pass-local proof. Receiving room is today's bank
/// room plus the minimum empty carry of its eligible builders. Excavated soil
/// remains physical custody until Fill consumes it or storage receives it.
pub(super) struct Capacity<'a> {
    receiving_slots: usize,
    blueprint: &'a blueprints::Blueprint,
    network: layout::Network<'a>,
    project: Project,
    trees: std::collections::HashMap<[i32; 3], bool>,
}

impl<'a> Capacity<'a> {
    pub(super) fn for_blueprint(
        state: &'a TickState,
        record: &'a SettlementRecord,
        blueprint: &'a blueprints::Blueprint,
        images: &[Image],
    ) -> Option<Self> {
        if blueprint.kind != BuildingKind::Storage || !has_storage(record) {
            return None;
        }
        let total: usize = images.iter().map(|image| image.slots.len()).sum();
        let free = images
            .iter()
            .flat_map(|image| &image.slots)
            .filter(|stack| stack.is_empty())
            .count();
        if total == 0 || free * 4 >= total {
            return None;
        }
        Some(Self {
            receiving_slots: free
                .saturating_add(builders::receiving_slots(state, record.id).unwrap_or(0)),
            blueprint,
            network: layout::Network::for_construction(state, record),
            project: Project {
                settlement: record.id,
                blueprint: blueprint.id.clone(),
                level: 1,
                origin: record.anchor,
                rotation: 0,
                building: None,
                structure: String::new(),
                ordered: false,
                builder: None,
                order_seq: None,
            },
            trees: Default::default(),
        })
    }

    fn has_tree(&mut self, state: &TickState, at: [i32; 3]) -> Result<bool, site_refusals::Reason> {
        if let Some(&tree) = self.trees.get(&at) {
            return Ok(tree);
        }
        // This is the same vertical reach as forestry::cells. Refusing even
        // an overhead tree is conservative: a touched registered crown can
        // schedule a whole tree outside the lot before any fill is consumed.
        let top = (at[1] + 64).min(state.world.geometry().max_y() - 1);
        for y in at[1] + 1..=top {
            let block = state
                .world
                .physics_block_at(at[0], y, at[2])
                .ok_or(site_refusals::Reason::Unloaded)?;
            if crate::settlements::forestry::tree_or_sapling(state, block) {
                self.trees.insert(at, true);
                return Ok(true);
            }
        }
        self.trees.insert(at, false);
        Ok(false)
    }
}

pub(super) fn candidate_fits(
    state: &TickState,
    origin: [i32; 3],
    size: [u8; 3],
    columns: &[([i32; 2], i32)],
    volume_cuts: u32,
    village: Option<&relayout::Relayout>,
    capacity: &mut Capacity<'_>,
) -> Result<(), site_refusals::Reason> {
    use site_refusals::Reason::StorageFull;
    // One existing empty bank/carry slot per possible cut conservatively covers
    // mixed terrain and plants. This never credits a future Fill or fetch:
    // the executor keeps unreceived soil on the real bound builder's body.
    let cuts = columns
        .iter()
        .map(|(_, height)| usize::try_from((height - origin[1]).max(0)).expect("cut height"))
        .sum::<usize>()
        .saturating_add(volume_cuts as usize);
    // A retired crop can credit both produce and seed before construction.
    let crops = village.map_or(0, |plan| plan.cleared.len().saturating_mul(2));
    if cuts.saturating_add(crops) > capacity.receiving_slots {
        tracing::debug!(
            settlement = capacity.project.settlement.0,
            ?origin,
            receiving_slots = capacity.receiving_slots,
            cuts,
            crops,
            "settlement_storehouse_capacity_refused"
        );
        return Err(StorageFull);
    }
    // The full manifest also clears trees, including an entire registered
    // component reached through the walking corridor. Its random leaf loot
    // is not a current-tick estimate: refuse unproved tree work altogether.
    for &([x, z], height) in columns {
        if capacity.has_tree(state, [x, height, z])? {
            tracing::debug!(settlement = capacity.project.settlement.0, ?origin,
                tree = ?[x, height, z], "settlement_storehouse_capacity_refused");
            return Err(StorageFull);
        }
    }
    capacity.project.origin = origin;
    // check receives the footprint, not the orientation. Prove every matching
    // orientation's possible corridor; an extra refusal is safe, unlike
    // accepting the other orientation's clear route beside a blocked one.
    for rotation in 0..4 {
        if capacity
            .blueprint
            .footprint(Rotation::from_quarter_turns(rotation))
            != size
        {
            continue;
        }
        capacity.project.rotation = rotation;
        if let Some(route) = capacity.network.construction_access(&capacity.project) {
            for at in route {
                if capacity.has_tree(state, at)? {
                    tracing::debug!(settlement = capacity.project.settlement.0, ?origin,
                        tree = ?at, corridor = true, "settlement_storehouse_capacity_refused");
                    return Err(StorageFull);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "construction_storage_tests.rs"]
mod tests;
