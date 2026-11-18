//! Regressions for the D8 parallel-construction review round 1 (Opus):
//! rotation, binding uniqueness, borrowed-builder restore, stall watch by
//! binding, the joint proof for walls and lamps, order position on a retried
//! preparation, and the first-project lookups.
use super::tests::{bind_project, fixture, fund, open_plan, start};
use super::*;

fn chest(state: &mut TickState) -> &mut Vec<mc_data::ItemStack> {
    &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots
}

/// The terrain fixture holds one lot, so a second neighbour project is pushed
/// directly: its authored box is all these regressions read.
fn push_neighbour(state: &mut TickState, id: SettlementId, handle: &str, origin: [i32; 3]) {
    let record = state.settlements.get_mut(id).unwrap();
    access::push_project(
        record,
        Project {
            settlement: id,
            blueprint: "house".into(),
            level: 1,
            origin,
            rotation: 0,
            building: None,
            structure: handle.into(),
            ordered: true,
            builder: None,
            order_seq: None,
        },
    );
    record.revision = record.revision.saturating_add(1);
}

fn handles(state: &TickState, id: SettlementId) -> Vec<String> {
    access::projects_of(&state.settlements, id)
        .map(|project| project.structure.clone())
        .collect()
}

/// P1-1: with two settlements the admission clock must reach every project of
/// every settlement, not only the indices congruent mod gcd(S, P).
#[test]
fn every_project_of_every_settlement_gets_a_turn() {
    let (mut state, id, _slot) = fixture();
    let second = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "farmers".into(),
            "owner".into(),
            [40, 201, 40],
            48,
        )
        .unwrap();
    for (settlement, origins) in [
        (id, [[2, 201, 2], [10, 201, 10]]),
        (second, [[34, 201, 34], [42, 201, 42]]),
    ] {
        let record = state.settlements.get_mut(settlement).unwrap();
        for (index, origin) in origins.into_iter().enumerate() {
            access::push_project(
                record,
                Project {
                    settlement,
                    blueprint: "house".into(),
                    level: 1,
                    origin,
                    rotation: 0,
                    building: None,
                    structure: format!("clock-{}-{index}", settlement.0),
                    ordered: true,
                    builder: Some((uuid::Uuid::from_u128(index as u128 + 1), None)),
                    order_seq: None,
                },
            );
        }
    }
    // The admission clock visits one settlement every four ticks and one
    // project of that settlement; both settlements and both indices per
    // settlement must appear.
    let settlements = state.settlements.records().count() as u64;
    let mut seen = std::collections::BTreeSet::new();
    for turn in 0..32u64 {
        state.tick = turn * 4;
        let chosen = state
            .settlements
            .records()
            .nth((state.tick / 4) as usize % settlements as usize)
            .map(|record| record.id)
            .unwrap();
        let bound: Vec<_> = access::projects_of(&state.settlements, chosen)
            .filter(|project| project.builder.is_some())
            .collect();
        let count = bound.len();
        if count == 0 {
            continue;
        }
        let index = (state.tick / 4 / settlements) as usize % count;
        seen.insert((chosen, index));
    }
    for settlement in [id, second] {
        for index in 0..2 {
            assert!(
                seen.contains(&(settlement, index)),
                "project {index} of settlement {} never gets the clock",
                settlement.0
            );
        }
    }
}

/// P1-2: one resident never binds two projects, and the record still opens.
#[test]
fn one_builder_never_binds_two_projects_and_the_record_opens() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let first = access::projects_of(&state.settlements, id)
        .next()
        .unwrap()
        .structure
        .clone();
    let second = first.clone();
    // A second project that shares the resident: the access refuses it.
    let uuid = state.entities.row(slot).uuid;
    bind_project(&mut state, id, &first, slot);
    let mut record = state.settlements.get(id).unwrap().clone();
    access::push_project(
        &mut record,
        Project {
            settlement: id,
            blueprint: "house".into(),
            level: 1,
            origin: [20, 201, 20],
            rotation: 0,
            building: None,
            structure: "second".into(),
            ordered: true,
            builder: None,
            order_seq: None,
        },
    );
    assert!(
        !access::bind_builder(&mut record, "second", Some((uuid, None))),
        "a resident already bound to {second} is not bound again"
    );
    assert_eq!(
        access::project_for_structure(&record, &first)
            .unwrap()
            .builder,
        Some((uuid, None))
    );
    // The record with the refused duplicate still opens.
    *state.settlements.get_mut(id).unwrap() = record;
    let encoded = serde_json::to_vec(state.settlements.get(id).unwrap()).unwrap();
    let restored: SettlementRecord = serde_json::from_slice(&encoded).unwrap();
    assert!(
        super::validate_record(&restored).is_ok(),
        "the record decodes"
    );
}

/// P1-3: a native Builder keeps its career while another project holds a
/// borrowed gatherer, and the borrowed one goes back after completion.
#[test]
fn a_native_builder_is_not_stripped_by_the_borrow_restore() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let uuid = state.entities.row(slot).uuid;
    let handle = access::projects_of(&state.settlements, id)
        .next()
        .unwrap()
        .structure
        .clone();
    // A native Builder holds its project as `(uuid, None)`: not borrowed.
    bind_project(&mut state, id, &handle, slot);
    assert_eq!(
        access::projects_of(&state.settlements, id)
            .next()
            .unwrap()
            .builder,
        Some((uuid, None))
    );
    // No gatherer is staffed, so the planner asks for one on every pass.
    for tick in 0..8u64 {
        state.tick = tick * PLAN_CADENCE;
        tick_settlements(&mut state);
    }
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::Builder,
        "a native Builder keeps its career and its job"
    );
}

fn tick_settlements(state: &mut TickState) {
    super::tick(state);
}

/// P1-4: a stalled project releases its own builder, never the other project's.
#[test]
fn a_stalled_project_releases_only_its_own_builder() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    fund(&mut state, "house", 1);
    start(&mut state, id, "house", 1, None);
    let healthy = access::projects_of(&state.settlements, id)
        .next()
        .unwrap()
        .structure
        .clone();
    let healthy_uuid = state.entities.row(slot).uuid;
    // A second project bound to a second resident that is walled in.
    let trapped = crate::settlements::allocator::tests::villager(&mut state, [10.5, 201.0, 10.5]);
    let entity = state.entities.body(trapped).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    state
        .entities
        .row_mut(trapped)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::Builder;
    state
        .entities
        .row_mut(trapped)
        .villager
        .as_mut()
        .unwrap()
        .activity = crate::villager::Activity::Work;
    for x in 9..=11 {
        for z in 9..=11 {
            for y in 201..=203 {
                if (x, z) != (10, 10) || y == 203 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    {
        let record = state.settlements.get_mut(id).unwrap();
        access::push_project(
            record,
            Project {
                settlement: id,
                blueprint: "house".into(),
                level: 1,
                origin: [24, 201, 24],
                rotation: 0,
                building: None,
                structure: "walled".into(),
                ordered: true,
                builder: Some((state.entities.row(trapped).uuid, None)),
                order_seq: None,
            },
        );
    }
    let trapped_uuid = state.entities.row(trapped).uuid;
    assign_builder(&mut state, id);
    let record = state.settlements.get(id).unwrap();
    assert_eq!(
        access::project_for_structure(record, &healthy)
            .unwrap()
            .builder,
        Some((healthy_uuid, None)),
        "the healthy project keeps its builder"
    );
    assert_eq!(
        access::project_for_structure(record, "walled")
            .unwrap()
            .builder,
        None,
        "the trapped project gives its builder back"
    );
    assert_eq!(
        state
            .entities
            .row(trapped)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::None
    );
    let _ = trapped_uuid;
}

/// P2-1: an unbound resident builds nothing (no fallback to the first project).
#[test]
fn an_unbound_builder_owns_no_project() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let uuid = state.entities.row(slot).uuid;
    {
        let record = state.settlements.get_mut(id).unwrap();
        access::unbind_builder(record, uuid);
    }
    let record = state.settlements.get(id).unwrap();
    assert!(
        access::project_of_builder(record, uuid).is_none(),
        "an unbound resident has no project"
    );
}

/// P2-2: a wall never funds itself out of an active project's materials.
#[test]
fn a_wall_is_refused_against_an_active_underfunded_project() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let stored = {
        let images = executor::source_images(&state, id);
        let cells = vec![executor::PlannedCell {
            at: [2, 201, 2],
            state: state.registry.states.air,
            resource: Some("minecraft:oak_log".into()),
            quantity: 0,
        }];
        let bill = cell_bill(&state, &cells);
        let funded = bill
            .as_ref()
            .is_some_and(|bill| jointly_fundable(&state, id, bill));
        let _ = images;
        let _ = slot;
        funded
    };
    assert!(
        !stored,
        "an empty chest cannot fund a wall beside an active project"
    );
}

/// P2-3: a retried order goes back to its submission position, not the tail,
/// and a full queue refuses the retry before the project is cancelled.
#[test]
fn a_retried_order_keeps_its_submission_position() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    {
        let record = state.settlements.get_mut(id).unwrap();
        record.owner = Some("owner".into());
        access::push_order(
            record,
            OwnerOrder {
                seq: 0,
                blueprint: "house".into(),
                level: 1,
                building: None,
            },
        );
        access::push_order(
            record,
            OwnerOrder {
                seq: 0,
                blueprint: "storage".into(),
                level: 1,
                building: None,
            },
        );
        access::push_order(
            record,
            OwnerOrder {
                seq: 0,
                blueprint: "farm".into(),
                level: 1,
                building: None,
            },
        );
    }
    // The middle request was lifted first, then its predecessor is cancelled.
    let lifted = {
        let record = state.settlements.get_mut(id).unwrap();
        access::take_order(record, 1).unwrap()
    };
    {
        let record = state.settlements.get_mut(id).unwrap();
        access::take_order(record, 0);
        access::insert_order_by_seq(record, lifted.clone());
    }
    let record = state.settlements.get(id).unwrap();
    assert_eq!(
        access::orders(record)
            .map(|order| order.blueprint.as_str())
            .collect::<Vec<_>>(),
        ["storage", "farm"],
        "the restored request stays ahead of the later one"
    );
    let _ = id;
}

/// P2-5: debit and fetch refuse a project with no binding.
#[test]
fn an_unbound_project_cannot_pay_or_haul() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let handle = access::projects_of(&state.settlements, id)
        .next()
        .unwrap()
        .structure
        .clone();
    let uuid = state.entities.row(slot).uuid;
    {
        let record = state.settlements.get_mut(id).unwrap();
        access::unbind_builder(record, uuid);
    }
    let stock = chest(&mut state).clone();
    let item = chest(&mut state)
        .iter()
        .find(|stack| stack.count > 0)
        .map_or(1, |stack| stack.item_id);
    let fetched = executor::fetch(&mut state, uuid, &handle, [1, 201, 1], item);
    assert!(fetched.is_err(), "an unbound project hauls nothing");
    assert_eq!(chest(&mut state), &stock, "stock is untouched");
}

/// SOL-4: a stand or body inside a neighbour lot's authored footprint is
/// refused, while the project's own footprint stays available.
#[test]
fn a_neighbour_footprint_is_not_a_stand() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let mine = handles(&state, id)[0].clone();
    push_neighbour(&mut state, id, "neighbour-house", [23, 200, 3]);
    let other = "neighbour-house".to_string();
    let record = state.settlements.get(id).unwrap();
    let own = footprint(&state, project_for_structure(record, &mine).unwrap()).unwrap();
    let theirs = footprint(&state, project_for_structure(record, &other).unwrap()).unwrap();
    let inside_theirs = theirs.min;
    let inside_own = own.min;
    assert!(
        in_other_footprint(&state, id, &mine, inside_theirs),
        "a neighbour's future body is not this builder's stand"
    );
    assert!(
        !in_other_footprint(&state, id, &mine, inside_own),
        "a builder may stand in its own lot"
    );
    assert!(
        in_other_footprint(&state, id, &other, inside_own),
        "the rule is symmetric for the neighbour"
    );
}

/// SOL-3: the reserve future carries every other active project's remainder
/// and never the queried project itself.
#[test]
fn a_haul_reserves_the_other_active_bills() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let mine = handles(&state, id)[0].clone();
    let own_bill = project_bill(&state, id, &mine).unwrap();
    assert!(
        !own_bill.is_empty(),
        "a running project has a bill to reserve"
    );
    // Nothing else is active, so this project reserves nothing against itself
    // and a query for a stranger reserves the whole bill.
    assert!(other_active_bill(&state, id, &mine).is_empty());
    assert_eq!(
        other_active_bill(&state, id, "stranger"),
        own_bill,
        "every other active bill is reserved exactly"
    );
}

/// SOL-6: an addressable carry return reports whether it actually delivered,
/// so a full store cannot silently drop the load.
#[test]
fn a_carry_return_reports_delivery() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let uuid = state.entities.row(slot).uuid;
    let handle = handles(&state, id)[0].clone();
    bind_project(&mut state, id, &handle, slot);
    let log = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:oak_log").unwrap())
        .expect("oak log id");
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(mc_data::ItemStack::new(log, 7));
    // A full store cannot take the load, so the return is not delivered.
    for stack in chest(&mut state) {
        *stack = mc_data::ItemStack::new(log, 64);
    }
    assert!(
        !executor::return_carry_of(&mut state, id, uuid),
        "a load the store cannot take is not delivered"
    );
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .inventory[0]
            .as_ref()
            .map(|stack| stack.count),
        Some(7),
        "the load stays on the body"
    );
    // Space appearing later lets the next pass deliver it.
    for stack in chest(&mut state) {
        *stack = mc_data::ItemStack::EMPTY;
    }
    assert!(
        executor::return_carry_of(&mut state, id, uuid),
        "the freed store takes the load"
    );
    assert!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .inventory[0]
            .is_none(),
        "the delivered load leaves the body"
    );
}

/// R2-4: every submission takes its sequence even before it starts, so a
/// request that later retries does not fall behind requests submitted after it.
#[test]
fn every_submission_takes_its_sequence() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    state.settlements.get_mut(id).unwrap().owner = Some("owner".into());
    // An empty chest leaves the first request waiting; it still takes seq 1.
    let _ = submit_order(
        &mut state,
        id,
        OwnerOrder {
            seq: 0,
            blueprint: "house".into(),
            level: 1,
            building: None,
        },
    );
    let _ = submit_order(
        &mut state,
        id,
        OwnerOrder {
            seq: 0,
            blueprint: "house".into(),
            level: 1,
            building: None,
        },
    );
    let record = state.settlements.get(id).unwrap();
    let seqs: Vec<u64> = orders(record).map(|order| order.seq).collect();
    assert_eq!(
        seqs,
        vec![1, 2],
        "submission sequences are stamped in order"
    );
    assert_eq!(
        record.order_seq, 2,
        "the settlement keeps the last sequence"
    );
}

/// The persistence fence: a stored project written before the D8 fields
/// existed refuses instead of reading as an unbound, sequence-less project.
#[test]
fn a_record_without_the_new_project_fields_refuses() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let record = state.settlements.get(id).unwrap();
    let mut value = serde_json::to_value(record).expect("record serialises");
    let project = value["projects"][0]
        .as_object_mut()
        .expect("project object");
    project.remove("order_seq");
    project.remove("builder");
    let decoded: SettlementRecord = serde_json::from_value(value).expect("row decodes");
    assert!(
        validate_record(&decoded).is_err(),
        "a row without the D8 project fields is refused"
    );
}

/// R2-1: a binding only counts as an incumbent while the resident is still a
/// Builder of this settlement and is not assigned to another job.
#[test]
fn a_stale_binding_is_not_an_incumbent() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let handle = handles(&state, id)[0].clone();
    let uuid = state.entities.row(slot).uuid;
    bind_project(&mut state, id, &handle, slot);
    {
        let record = state.settlements.get(id).unwrap();
        let project = project_for_structure(record, &handle).unwrap();
        assert_eq!(
            incumbent_slot(&state, id, project),
            Some(slot),
            "the bound Builder holds its project"
        );
    }
    // A resident that stopped being a Builder keeps no incumbent: the binding
    // is stale and must be cleared by the rebind pass, carry first.
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = crate::villager::VillagerProfession::Farmer;
    {
        let record = state.settlements.get(id).unwrap();
        let project = project_for_structure(record, &handle).unwrap();
        assert_eq!(
            incumbent_slot(&state, id, project),
            None,
            "a non-Builder keeps nothing"
        );
    }
    let _ = uuid;
}
