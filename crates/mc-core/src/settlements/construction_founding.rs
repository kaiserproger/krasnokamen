//! Native outposts: ordinary paid construction first, atomic civil handover last.
//! Only the chosen site and household persist; stage is derived from the ledger.
use super::*;

static BREAD: std::sync::LazyLock<mc_data::Identifier> =
    std::sync::LazyLock::new(|| mc_data::Identifier::parse("minecraft:bread").unwrap());

const FOUNDING_POPULATION: usize = 24;
const CHILD_RADIUS: u32 = 32;
const SITES: [(&str, [i32; 3]); 4] = [
    ("spanish_house", [0, 0, 0]),
    ("spanish_house", [0, 0, 10]),
    ("spanish_house", [0, 0, 20]),
    ("spanish_plaza", [14, 0, 5]),
];

impl FoundingExpedition {
    pub(crate) fn bounds(&self) -> ScriptWorldBounds {
        ScriptWorldBounds::new(
            self.destination,
            [
                self.destination[0] + 24,
                self.destination[1] + 10,
                self.destination[2] + 30,
            ],
        )
        .expect("validated founding coordinates")
    }

    pub(crate) fn contains(&self, at: [i32; 3]) -> bool {
        let bounds = self.bounds();
        (0..3).all(|axis| (bounds.min[axis]..=bounds.max[axis]).contains(&at[axis]))
    }

    pub(crate) fn sites(&self) -> [(&'static str, [i32; 3]); 4] {
        SITES.map(|(name, offset)| {
            (
                name,
                std::array::from_fn(|axis| self.destination[axis] + offset[axis]),
            )
        })
    }

    fn anchor(&self) -> [i32; 3] {
        [
            self.destination[0] + 17,
            self.destination[1] + 3,
            self.destination[2] + 8,
        ]
    }
}

pub(in crate::settlements) fn validate_record(
    record: &SettlementRecord,
) -> Result<(), &'static str> {
    let Some(expedition) = &record.founding else {
        return Ok(());
    };
    if record.owner.is_none()
        || !record.households.contains_key(&expedition.household)
        || expedition.household.len() > 128
        || expedition
            .destination
            .iter()
            .any(|value| !(-29_999_900..=29_999_900).contains(value))
        || !(mc_world::MIN_Y..mc_world::MAX_Y - 10).contains(&expedition.destination[1])
    {
        return Err("invalid founding expedition household, owner or destination");
    }
    Ok(())
}

fn overlaps(a: ScriptWorldBounds, b: ScriptWorldBounds) -> bool {
    (0..3).all(|axis| a.min[axis] <= b.max[axis] && b.min[axis] <= a.max[axis])
}

pub(in crate::settlements) fn sphere_overlaps(
    anchor: [i32; 3],
    radius: u32,
    bounds: ScriptWorldBounds,
) -> bool {
    (0..3)
        .map(|axis| {
            let nearest = anchor[axis].clamp(bounds.min[axis], bounds.max[axis]);
            (i128::from(nearest) - i128::from(anchor[axis])).pow(2)
        })
        .sum::<i128>()
        <= i128::from(radius).pow(2)
}

fn expedition_overlaps(a: &FoundingExpedition, b: &FoundingExpedition) -> bool {
    // Reserve against the civil radius that handover will publish, not just
    // today's disjoint construction rectangles. Both directions must survive
    // either expedition completing first.
    sphere_overlaps(a.anchor(), CHILD_RADIUS, b.bounds())
        || sphere_overlaps(b.anchor(), CHILD_RADIUS, a.bounds())
}

/// Recovery uses exactly the same full site claims as admission. No chunk load.
pub(in crate::settlements) fn validate_claims(state: &TickState) -> Result<(), &'static str> {
    for record in state.settlements.records() {
        validate_record(record)?;
    }
    for parent in state.settlements.records() {
        let Some(expedition) = &parent.founding else {
            continue;
        };
        let bounds = expedition.bounds();
        if state.settlements.records().any(|other| {
            other.id != parent.id
                && (sphere_overlaps(other.anchor, other.radius, bounds)
                    || other
                        .buildings
                        .values()
                        .any(|building| overlaps(building.bounds, bounds))
                    || other
                        .founding
                        .as_ref()
                        .is_some_and(|other| expedition_overlaps(other, expedition)))
        }) || !executor::founding_claim_clear(state, parent.id, bounds, &expedition.sites())
        {
            return Err("founding site overlaps a settlement, expedition or structure claim");
        }
    }
    Ok(())
}

/// Shared founding/migration exclusion, independent of household size or food.
pub(crate) fn member_unreserved(
    state: &TickState,
    parent: SettlementId,
    entity: crate::boundary::EntityId,
) -> bool {
    let Some(slot) = state.entities.slot_of(entity) else {
        return false;
    };
    let row = state.entities.row(slot);
    row.villager.as_ref().is_some_and(|v| {
        v.soldier.is_none()
            && v.conversion_time.is_none()
            && !super::super::economy::worker_reserved(state, Some(parent), row.uuid)
    })
}

fn household_ready(state: &TickState, parent: SettlementId, household: &str, food: bool) -> bool {
    let members = super::super::households::members(state, parent, household);
    if !(2..=3).contains(&members.len())
        || members
            .iter()
            .filter(|entity| {
                let slot = state
                    .entities
                    .slot_of(**entity)
                    .expect("living household member");
                state.entities.row(slot).villager.as_ref().unwrap().age >= 0
            })
            .count()
            < 2
    {
        return false;
    }
    if state.other_dimensions.values().any(|world| {
        world.entities.slots().any(|slot| {
            world.entities.body(slot).alive
                && world
                    .entities
                    .row(slot)
                    .villager
                    .as_ref()
                    .is_some_and(|v| v.household.as_deref() == Some(household))
        })
    }) {
        return false;
    }
    let bread = if food {
        state.registry.items.id_of(&BREAD)
    } else {
        None
    };
    members.into_iter().all(|entity| {
        let slot = state
            .entities
            .slot_of(entity)
            .expect("living household member");
        let v = state.entities.row(slot).villager.as_ref().unwrap();
        member_unreserved(state, parent, entity)
            && (!food || v.data.profession != Profession::Builder)
            && (!food
                || v.age < 0
                || v.food_level >= 12
                || v.inventory
                    .iter()
                    .flatten()
                    .filter(|stack| Some(stack.item_id) == bread)
                    .map(|stack| stack.count.max(0))
                    .sum::<i32>()
                    >= 3)
    })
}

pub(super) fn site_reserved(
    state: &TickState,
    parent: SettlementId,
    origin: [i32; 3],
    size: [u8; 3],
) -> bool {
    let bounds = ScriptWorldBounds::new(
        origin,
        std::array::from_fn(|axis| origin[axis] + i32::from(size[axis]) - 1),
    )
    .expect("bounded authored footprint");
    state.settlements.records().any(|town| {
        town.founding.as_ref().is_some_and(|expedition| {
            overlaps(expedition.bounds(), bounds)
                && !(town.id == parent
                    && expedition.sites().into_iter().any(|(name, at)| {
                        at == origin
                            && blueprints::get(&state.registry, name)
                                .is_some_and(|blueprint| blueprint.footprint(Rotation(0)) == size)
                    }))
        })
    })
}

pub(super) fn next_blueprint(state: &TickState, record: &SettlementRecord) -> Option<&'static str> {
    record
        .founding
        .as_ref()?
        .sites()
        .into_iter()
        .find(|(name, origin)| !executor::built_at(state, record.id, name, *origin))
        .map(|(name, _)| name)
}

fn admit(state: &mut TickState, parent: SettlementId) {
    let record = state.settlements.get(parent).expect("planning settlement");
    if record.owner.is_none()
        || state.settlements.member_count(parent) < FOUNDING_POPULATION
        || !spanish_faction(state, record)
    {
        return;
    }
    if record.households.is_empty() {
        return;
    }
    let cursor = record.construction_cursor;
    let household = record
        .households
        .keys()
        .nth(cursor as usize % record.households.len())
        .unwrap();
    let ready = household_ready(state, parent, household, true);
    let distance = i32::try_from(record.radius).expect("bounded radius") + 40;
    let [dx, dz] =
        [[1, 0], [0, 1], [-1, 0], [0, -1]][cursor as usize / record.households.len() % 4];
    let x = record.anchor[0].checked_add(dx * distance);
    let z = record.anchor[2].checked_add(dz * distance);
    let candidate = ready
        .then(|| {
            x.zip(z).and_then(|(x, z)| {
                surface_y(state, record.anchor[1], x, z).map(|y| FoundingExpedition {
                    destination: [x, y, z],
                    household: household.clone(),
                })
            })
        })
        .flatten();
    let record = state.settlements.get_mut(parent).unwrap();
    record.construction_cursor = cursor.wrapping_add(1);
    record.revision = record.revision.saturating_add(1);
    let Some(expedition) = candidate else { return };
    if expedition
        .destination
        .iter()
        .any(|value| !(-29_999_900..=29_999_900).contains(value))
        || !(mc_world::MIN_Y..mc_world::MAX_Y - 10).contains(&expedition.destination[1])
    {
        return;
    }
    let bounds = expedition.bounds();
    if state.settlements.records().any(|town| {
        sphere_overlaps(town.anchor, town.radius, bounds)
            || town
                .founding
                .as_ref()
                .is_some_and(|other| expedition_overlaps(other, &expedition))
    }) || !executor::founding_claim_clear(state, parent, bounds, &expedition.sites())
    {
        return;
    }
    let record = state.settlements.get(parent).unwrap();
    let mut clearance = super::super::forestry::Clearance::default();
    if !expedition.sites().into_iter().all(|(name, origin)| {
        let blueprint = blueprints::get(&state.registry, name).expect("authored outpost");
        site_free_claim(
            state,
            record,
            origin,
            blueprint.footprint(Rotation(0)),
            Some(&expedition),
            &mut clearance,
        ) && planned_cells(
            state,
            &Project {
                settlement: parent,
                blueprint: name.into(),
                level: 1,
                origin,
                rotation: 0,
                building: None,
                structure: String::new(),
                ordered: false,
                borrowed_builder: None,
            },
            "oak",
            &mut clearance,
        )
        .is_ok()
    }) {
        return;
    }
    let record = state.settlements.get_mut(parent).unwrap();
    record.founding = Some(expedition);
    record.revision = record.revision.saturating_add(1);
}

pub(super) fn owns_project(record: &SettlementRecord, project: &Project) -> bool {
    record.founding.as_ref().is_some_and(|expedition| {
        expedition.sites().into_iter().any(|(name, origin)| {
            project.blueprint == name
                && project.origin == origin
                && project.rotation == 0
                && project.level == 1
        })
    })
}

pub(in crate::settlements) fn cancel(state: &mut TickState, parent: SettlementId) {
    let record = state.settlements.get(parent).expect("founding parent");
    if record
        .project
        .as_ref()
        .is_some_and(|project| owns_project(record, project))
    {
        cancel_project(state, parent).expect("cancel admitted founding structure");
    }
    let record = state.settlements.get_mut(parent).unwrap();
    record.founding = None;
    record.revision = record.revision.saturating_add(1);
}

/// Returns true only while this expedition owns the planning turn. Material
/// shortages leave ordinary native extraction and crafting active.
pub(super) fn plan(state: &mut TickState, parent: SettlementId) -> bool {
    let record = state.settlements.get(parent).unwrap();
    let pending = record.pending_order.is_some();
    if record.founding.is_none() {
        if pending {
            return false;
        }
        admit(state, parent);
    }
    let Some(expedition) = state.settlements.get(parent).unwrap().founding.clone() else {
        return false;
    };
    if !household_ready(state, parent, &expedition.household, false) {
        cancel(state, parent);
        return false;
    }
    if state.settlements.get(parent).unwrap().project.is_some() {
        return true;
    }
    for (name, origin) in expedition.sites() {
        if executor::built_at(state, parent, name, origin) {
            continue;
        }
        if pending {
            return false;
        }
        let blueprint = blueprints::get(&state.registry, name).expect("authored outpost");
        let images = executor::source_images(state, parent);
        let Some((species, color)) = funded_variant(
            state,
            parent,
            &blueprint.level(1).unwrap().materials,
            &images,
        ) else {
            return false;
        };
        let species = species.to_owned();
        let record = state.settlements.get(parent).unwrap();
        if !site_free(state, record, origin, blueprint.footprint(Rotation(0))) {
            return true;
        }
        let project = Project {
            settlement: parent,
            blueprint: name.into(),
            level: 1,
            origin,
            rotation: 0,
            building: None,
            structure: String::new(),
            ordered: false,
            borrowed_builder: None,
        };
        return begin(state, project, &species, color);
    }
    finish(state, parent, &expedition);
    true
}

/// Every fallible operation (including ledger ownership transfer) is prepared
/// before mutation. The native tick's one WAL batch publishes the whole result.
fn finish(state: &mut TickState, parent: SettlementId, expedition: &FoundingExpedition) -> bool {
    if validate_claims(state).is_err()
        || !household_ready(state, parent, &expedition.household, false)
    {
        return false;
    }
    let source = state.settlements.get(parent).unwrap();
    if source.project.is_some()
        || source.revision > u64::MAX - 2
        || source.households[&expedition.household].revision == u64::MAX
    {
        return false;
    }
    let Some(next) = state
        .settlement_sequence
        .max(state.settlements.high_water())
        .checked_add(1)
        .filter(|next| *next <= u64::from(u32::MAX))
    else {
        return false;
    };
    let child = SettlementId(next);
    let name = format!("colony_{next}");
    if state.settlements.find(&name).is_some() {
        return false;
    }
    let mut certificates = Vec::with_capacity(4);
    for (name, origin) in expedition.sites() {
        let blueprint = blueprints::get(&state.registry, name).unwrap();
        let bounds = blueprint
            .level(1)
            .unwrap()
            .rotated(Rotation(0))
            .bounds(origin);
        if source
            .buildings
            .values()
            .any(|building| overlaps(building.bounds, bounds))
        {
            return false;
        }
        let Ok(points) = buildings::inspect(state, blueprint.kind, 1, bounds) else {
            return false;
        };
        certificates.push((blueprint.kind, bounds, points));
    }
    let Ok(transfer) =
        executor::prepare_founding_transfer(state, parent, child, &expedition.sites())
    else {
        return false;
    };
    let owner = source.owner.clone().expect("validated founding owner");
    let owner_name = source.owner_name.clone();
    // The source reservation is consumed by this same synchronous tick. Every
    // fallible check above has completed; creation now admits against all
    // remaining claims, and certificates belong directly to the daughter.
    state.settlements.get_mut(parent).unwrap().founding = None;
    let created = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            name,
            owner,
            expedition.anchor(),
            CHILD_RADIUS,
        )
        .expect("prevalidated daughter identity");
    assert_eq!(created, child);
    for (kind, bounds, _) in certificates {
        buildings::certify(state, child, kind, bounds).expect("prevalidated daughter certificate");
    }
    state.settlements.get_mut(child).unwrap().owner_name = owner_name;
    executor::commit_founding_transfer(state, transfer);
    // The complete small household remains alive and unreserved; destination
    // is new and cannot contain its identity. No body or inventory is copied.
    super::super::households::relocate(state, parent, &expedition.household, child)
        .expect("prevalidated founding household relocation");
    let record = state.settlements.get_mut(parent).unwrap();
    record.founding = None;
    record.revision += 1;
    tracing::info!(target: "settlement_audit", parent = parent.0, settlement = child.0,
        tick = state.tick, game_time = state.tick_control.game_ticks(state.tick), "settlement_daughter_founded");
    true
}

#[cfg(test)]
#[path = "construction_founding_tests.rs"]
mod tests;
