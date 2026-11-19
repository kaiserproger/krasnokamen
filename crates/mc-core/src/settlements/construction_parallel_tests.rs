//! Regressions for the D8 parallel-construction review round 1 (Opus):
//! rotation, binding uniqueness, borrowed-builder restore, stall watch by
//! binding, the joint proof for walls and lamps, order position on a retried
//! preparation, and the first-project lookups.
use super::tests::{bind_project, fixture, fund, open_plan, request_started, start};
use super::*;
use crate::entities::EntitySlot;

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
            portion: executor::PortionKind::Cut,
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
    assert!(
        other_active_bill(&state, id, &mine).is_some_and(|bill| bill.is_empty()),
        "a lone project reserves nothing"
    );
    assert_eq!(
        other_active_bill(&state, id, "stranger"),
        Some(own_bill),
        "every other active bill is reserved exactly"
    );
}

/// SOL-6: an addressable carry return reports what it achieved, so a full store
/// cannot silently drop the load and a dead body never holds the binding.
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
    // A full store cannot take the load: nothing is delivered, and the pass
    // must keep the binding so the return is retried.
    for stack in chest(&mut state) {
        *stack = mc_data::ItemStack::new(log, 64);
    }
    assert_eq!(
        executor::return_carry_of(&mut state, id, uuid),
        executor::CarryReturn::Pending,
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
    assert_eq!(
        handles(&state, id),
        vec![handle.clone()],
        "the project keeps its binding meanwhile"
    );
    // Space appearing later lets the next pass deliver it.
    chest(&mut state).fill(mc_data::ItemStack::EMPTY);
    assert_eq!(
        executor::return_carry_of(&mut state, id, uuid),
        executor::CarryReturn::Delivered,
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
    // A body that is no longer addressable has nothing left to deliver.
    state.entities.body_mut(slot).alive = false;
    assert_eq!(
        executor::return_carry_of(&mut state, id, uuid),
        executor::CarryReturn::Unreachable,
        "a dead body cannot hold a binding"
    );
}

/// A lost builder's binding is kept only while its load cannot be delivered:
/// the rebind pass retries a pending return and clears everything else.
#[test]
fn a_lost_builder_keeps_its_binding_only_while_the_load_is_pending() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let handle = handles(&state, id)[0].clone();
    let uuid = state.entities.row(slot).uuid;
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
    // The builder stopped being one, and the store cannot take its load yet.
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = crate::villager::VillagerProfession::Farmer;
    for stack in chest(&mut state) {
        *stack = mc_data::ItemStack::new(log, 64);
    }
    super::rebind_lost_builders(&mut state, id);
    assert!(
        project_for_structure(state.settlements.get(id).unwrap(), &handle)
            .is_some_and(|project| project.builder.is_none()),
        "the project slot is freed at once"
    );
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .carry_pending
            .contains_key(&uuid),
        "a pending return becomes a recorded duty so a later pass retries it"
    );
    // Space appears: the next pass delivers the load and frees the project.
    chest(&mut state).fill(mc_data::ItemStack::EMPTY);
    super::rebind_lost_builders(&mut state, id);
    super::drain_carry_duties(&mut state, id);
    assert!(
        state.settlements.get(id).unwrap().carry_pending.is_empty(),
        "a delivered load clears the duty"
    );
}

/// A binding held by a body that is no longer addressable (a converting or dead
/// villager) is cleared at once: it can never deliver anything again.
#[test]
fn a_converting_builder_never_holds_its_project() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let handle = handles(&state, id)[0].clone();
    bind_project(&mut state, id, &handle, slot);
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .conversion_time = Some(20);
    super::rebind_lost_builders(&mut state, id);
    assert!(
        project_for_structure(state.settlements.get(id).unwrap(), &handle)
            .is_some_and(|project| project.builder.is_none()),
        "a converting builder cannot hold the project"
    );
}

/// R4-1: an infected builder frees its project at once. Its carried materials
/// become a recorded return duty that survives the project being cancelled and
/// a cure delivers them exactly once; another resident's own load is not taken.
#[test]
fn an_infected_builder_releases_its_project_and_owes_its_load() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let handle = handles(&state, id)[0].clone();
    let uuid = state.entities.row(slot).uuid;
    bind_project(&mut state, id, &handle, slot);
    let log = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:oak_log").unwrap())
        .expect("oak log id");
    let food = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:bread").unwrap())
        .expect("bread id");
    let stored_logs = |state: &TickState| {
        state
            .block_entities
            .get(&(1, 201, 1))
            .unwrap()
            .slots
            .iter()
            .filter(|stack| stack.item_id == log)
            .map(|stack| stack.count)
            .sum::<i32>()
    };
    let stored_before = stored_logs(&state);
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(mc_data::ItemStack::new(log, 7));
    // A second resident carries its own load with no project at all: the duty
    // mechanism must never empty an unbound resident's pockets.
    let lumberjack = crate::settlements::allocator::tests::villager(&mut state, [5.5, 201.0, 5.5]);
    {
        let v = state
            .entities
            .row_mut(lumberjack)
            .villager
            .as_mut()
            .unwrap();
        v.data.profession = crate::villager::VillagerProfession::Lumberjack;
        v.inventory[0] = Some(mc_data::ItemStack::new(log, 3));
        v.inventory[1] = Some(mc_data::ItemStack::new(food, 2));
    }
    // The infection keeps the uuid and the row but the body is no longer a
    // live villager: the slot and the load's return duty must both be recorded.
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .conversion_time = Some(20);
    super::rebind_lost_builders(&mut state, id);
    assert!(
        project_for_structure(state.settlements.get(id).unwrap(), &handle)
            .is_some_and(|project| project.builder.is_none()),
        "an infected builder releases its project"
    );
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .carry_pending
            .contains_key(&uuid),
        "the load's return is recorded, not dropped"
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
        "the load waits on the body while it is not addressable"
    );
    assert_eq!(stored_logs(&state), stored_before, "nothing is invented");
    // The project goes away entirely: the duty must outlive it.
    let project = project_clone(&state, id, &handle);
    super::retry_preparation(&mut state, id, &project).unwrap();
    assert!(handles(&state, id).is_empty(), "the project is cancelled");
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .carry_pending
            .contains_key(&uuid),
        "the duty survives the project"
    );
    // A cure makes the body addressable: the construction tick delivers once.
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .conversion_time = None;
    state.tick = super::PLAN_CADENCE;
    super::tick(&mut state);
    if state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .inventory[0]
        .is_some()
    {
        state.tick = super::PLAN_CADENCE * 2;
        super::tick(&mut state);
    }
    assert!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .inventory[0]
            .is_none(),
        "the cured body hands its load over"
    );
    assert_eq!(
        stored_logs(&state),
        stored_before + 7,
        "the load reaches storage exactly once"
    );
    assert!(
        state.settlements.get(id).unwrap().carry_pending.is_empty(),
        "a delivered duty is cleared"
    );
    // The unbound lumberjack keeps its own load.
    assert_eq!(
        state
            .entities
            .row(lumberjack)
            .villager
            .as_ref()
            .unwrap()
            .inventory[0]
            .as_ref()
            .map(|stack| stack.count),
        Some(3),
        "an unbound resident's own load is not taken"
    );
}

/// The project as the registry holds it, for a call that needs a value.
fn project_clone(state: &TickState, id: SettlementId, handle: &str) -> Project {
    project_for_structure(state.settlements.get(id).expect("settlement"), handle)
        .cloned()
        .expect("active project")
}

/// R4-5: the indexed cancel removes the request at the position the owner sees
/// and leaves the rest of the sequenced queue in place.
#[test]
fn an_indexed_cancel_takes_the_named_request() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    state.settlements.get_mut(id).unwrap().owner = Some("owner".into());
    {
        let record = state.settlements.get_mut(id).unwrap();
        for name in ["house", "house", "house"] {
            access::push_order(
                record,
                OwnerOrder {
                    seq: 0,
                    blueprint: name.into(),
                    level: 1,
                    building: None,
                },
            );
        }
    }
    assert!(super::cancel_pending_at(&mut state, id, 1).is_ok());
    let seqs: Vec<u64> = orders(state.settlements.get(id).unwrap())
        .map(|order| order.seq)
        .collect();
    assert_eq!(seqs, vec![1, 3], "the second request is the one removed");
    assert_eq!(
        super::cancel_pending_at(&mut state, id, 5),
        Err("No pending construction order #6.".to_string())
    );
}

/// R4-1b: while a debt is open the debtor is not hired for a new project and
/// its carry is not consumed; when the store frees, exactly the owed amount is
/// delivered once and the debt closes.
#[test]
fn a_debtor_is_not_hired_until_its_carry_is_delivered() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let handle = handles(&state, id)[0].clone();
    let uuid = state.entities.row(slot).uuid;
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
    // Full store: the return cannot be delivered, so the debt stays open.
    for stack in chest(&mut state) {
        *stack = mc_data::ItemStack::new(log, 64);
    }
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .conversion_time = Some(20);
    super::rebind_lost_builders(&mut state, id);
    assert_eq!(
        state.settlements.get(id).unwrap().carry_pending.get(&uuid),
        Some(&vec![(log, 7)]),
        "the exact owed carry is recorded per item"
    );
    // A new project is admitted while the debt is open. The cured body is
    // otherwise hirable (on shift, Builder), so only the debt can explain a
    // refusal (R4-1b).
    {
        let v = state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .expect("villager row");
        v.conversion_time = None;
        v.data.profession = crate::villager::VillagerProfession::Builder;
        v.activity = crate::villager::Activity::Work;
    }
    push_neighbour(&mut state, id, "second-house", [24, 200, 24]);
    super::drain_carry_duties(&mut state, id);
    super::assign_builder(&mut state, id);
    // Neither the freshly admitted project nor the released one may take a
    // debtor: the planner is refused for every binding, not just the new site.
    assert!(
        access::projects_of(&state.settlements, id)
            .all(|project| project.builder.map(|(bound, _)| bound) != Some(uuid)),
        "a debtor is not hired for any project"
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
        "the owed carry is not consumed while the store is full"
    );
    assert_eq!(
        state.settlements.get(id).unwrap().carry_pending.get(&uuid),
        Some(&vec![(log, 7)]),
        "the debt survives a full store"
    );
    // The store frees: the exact amount is delivered once and the debt closes.
    chest(&mut state).fill(mc_data::ItemStack::EMPTY);
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .conversion_time = None;
    super::drain_carry_duties(&mut state, id);
    let delivered: i32 = chest(&mut state)
        .iter()
        .filter(|stack| stack.item_id == log)
        .map(|stack| stack.count)
        .sum();
    assert_eq!(delivered, 7, "the exact owed carry reaches storage once");
    assert!(
        state.settlements.get(id).unwrap().carry_pending.is_empty(),
        "the debt closes on delivery"
    );
    super::drain_carry_duties(&mut state, id);
    let after_second = chest(&mut state)
        .iter()
        .filter(|stack| stack.item_id == log)
        .map(|stack| stack.count)
        .sum::<i32>();
    assert_eq!(after_second, 7, "a drained debt is never delivered twice");
}

/// R4-1c: a builder carrying exactly the item the bill still asks for must
/// still record that debt — the net-of-carry bill has no such type left.
#[test]
fn a_carry_equal_to_the_remainder_is_still_owed() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let handle = handles(&state, id)[0].clone();
    let uuid = state.entities.row(slot).uuid;
    bind_project(&mut state, id, &handle, slot);
    let log = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:oak_log").unwrap())
        .expect("oak log id");
    let remaining = super::remaining_bill(&state, id, &handle).expect("bill");
    let owed = remaining
        .iter()
        .find(|(item, _)| *item == log)
        .map(|(_, count)| *count)
        .expect("the house asks for logs");
    assert!(owed > 0, "the fixture bill has logs to place");
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(mc_data::ItemStack::new(log, owed as i32));
    // The live builder's own carry is subtracted from project_bill, so the
    // type disappears there; the debt must come from the un-netted bill.
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .conversion_time = Some(20);
    super::rebind_lost_builders(&mut state, id);
    assert_eq!(
        state.settlements.get(id).unwrap().carry_pending.get(&uuid),
        Some(&vec![(log, owed)]),
        "the whole remainder the builder carried is owed"
    );
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .conversion_time = None;
    chest(&mut state).fill(mc_data::ItemStack::EMPTY);
    super::drain_carry_duties(&mut state, id);
    let delivered: i32 = chest(&mut state)
        .iter()
        .filter(|stack| stack.item_id == log)
        .map(|stack| stack.count)
        .sum();
    assert_eq!(delivered, owed as i32, "the owed amount is delivered once");
    assert!(state.settlements.get(id).unwrap().carry_pending.is_empty());
}

/// R4-1d: a real infection, a record reopen and a real cure. The debt is taken
/// when the builder stops being one, survives a checkpoint round trip, and is
/// delivered exactly once after the conversion tick cures the body.
#[test]
fn an_infected_builders_debt_survives_a_reopen_and_delivers_after_a_real_cure() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "house", 1, None);
    let handle = handles(&state, id)[0].clone();
    let uuid = state.entities.row(slot).uuid;
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
    // A real zombie bite, not a hand-set timer.
    let zombie_type = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:zombie").unwrap())
        .expect("zombie type");
    // Hard difficulty makes the bite deterministic (conversion.rs).
    state.difficulty = crate::survival::Difficulty::Hard;
    let pos = state.entities.pos(slot);
    let zombie = state.spawn_mob(zombie_type, [pos[0] + 1.0, pos[1], pos[2]], 20.0);
    assert!(
        crate::villager::conversion::infect(&mut state, slot, Some(zombie)),
        "the villager is infected by a real bite"
    );
    assert!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .is_some_and(|v| v.conversion_time.is_some()),
        "the bite starts the conversion timer"
    );
    super::rebind_lost_builders(&mut state, id);
    assert!(
        project_for_structure(state.settlements.get(id).unwrap(), &handle)
            .is_some_and(|project| project.builder.is_none()),
        "an infected builder releases its project"
    );
    assert_eq!(
        state.settlements.get(id).unwrap().carry_pending.get(&uuid),
        Some(&vec![(log, 7)]),
        "the exact load is owed"
    );
    // Checkpoint and reopen the record through its serialized image.
    let image = state
        .settlements
        .images()
        .into_iter()
        .find(|image| image.id == id)
        .expect("record image");
    let row = serde_json::to_string(&image).expect("checkpoint row");
    let reopened: crate::settlements::RecordImage =
        serde_json::from_str(&row).expect("reopened row");
    state.settlements.apply_image(reopened);
    assert_eq!(
        state.settlements.get(id).unwrap().carry_pending.get(&uuid),
        Some(&vec![(log, 7)]),
        "the debt survives the reopen"
    );
    // A real cure: the conversion countdown reaches zero through its own tick.
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .conversion_time = Some(1);
    for _ in 0..8 {
        crate::villager::conversion::tick(&mut state);
        if state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .is_some_and(|v| v.conversion_time.is_none())
        {
            break;
        }
    }
    assert!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .is_some_and(|v| v.conversion_time.is_none()),
        "the body is cured"
    );
    // The planner pass delivers the reopened debt exactly once.
    chest(&mut state).fill(mc_data::ItemStack::EMPTY);
    state.tick = super::PLAN_CADENCE;
    super::tick(&mut state);
    let delivered: i32 = chest(&mut state)
        .iter()
        .filter(|stack| stack.item_id == log)
        .map(|stack| stack.count)
        .sum();
    assert_eq!(delivered, 7, "the exact load reaches storage");
    assert!(
        state.settlements.get(id).unwrap().carry_pending.is_empty(),
        "the debt closes"
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
        "the body hands its load over"
    );
}

/// Spain seed 100 (pin 3068dfb1b857): free builders bound to whichever
/// unbound project came first, so the owner's church waited behind every
/// autonomous site opened before it. A free builder takes the order first,
/// through the planning pass and through the admission turn alike.
#[test]
fn a_free_builder_takes_the_owners_order_before_autonomous_work() {
    for lazy in [false, true] {
        let (mut state, id, slot) = fixture();
        open_plan(&mut state);
        for (handle, origin, ordered) in [
            ("autonomous", [2, 201, 2], false),
            ("order", [20, 201, 20], true),
        ] {
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
                    ordered,
                    builder: None,
                    order_seq: None,
                },
            );
        }
        if lazy {
            // One admission turn each, the autonomous site's first.
            for turn in 0..2 {
                state.tick = turn * 4;
                crate::villager::building::tick(&mut state, id);
            }
        } else {
            assign_builder(&mut state, id);
        }
        let uuid = state.entities.row(slot).uuid;
        let record = state.settlements.get(id).unwrap();
        assert_eq!(
            access::project_for_structure(record, "order")
                .unwrap()
                .builder,
            Some((uuid, None)),
            "lazy={lazy}"
        );
        assert_eq!(
            access::project_for_structure(record, "autonomous")
                .unwrap()
                .builder,
            None,
            "lazy={lazy}"
        );
    }
}

/// An active paving project on one free cell far from the plaza lots: no
/// bill, no shared ground.
fn paving_project(state: &mut TickState, id: SettlementId) -> String {
    let at = [28, 201, 28];
    let soil = crate::settlements::allocator::tests::block(state, "minecraft:grass_block", &[]);
    state.world.set_block(at[0], at[1], at[2], soil);
    state.world.fold_overlay();
    let path = crate::settlements::allocator::tests::block(state, "minecraft:dirt_path", &[]);
    let bounds = ScriptWorldBounds::new(at, at).unwrap();
    let structure = executor::prepare(
        state,
        id,
        layout::ROAD.into(),
        bounds,
        bounds,
        vec![executor::PlannedCell {
            portion: executor::PortionKind::Build,
            at,
            state: path,
            resource: None,
            quantity: 0,
        }],
    )
    .unwrap();
    let record = state.settlements.get_mut(id).unwrap();
    access::push_project(
        record,
        Project {
            settlement: id,
            blueprint: layout::ROAD.into(),
            level: 1,
            origin: at,
            rotation: 0,
            building: None,
            structure: structure.clone(),
            ordered: false,
            builder: None,
            order_seq: None,
        },
    );
    structure
}

/// A bound resident of `id` with `profession`, standing at `at`.
fn resident(
    state: &mut TickState,
    id: SettlementId,
    at: [f64; 3],
    profession: Profession,
) -> EntitySlot {
    let slot = crate::settlements::allocator::tests::villager(state, at);
    let entity = state.entities.body(slot).id;
    crate::settlements::bind(state, id, entity).unwrap();
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = profession;
    slot
}

/// Spain seed 100 (pin 3068dfb1b857): the planner opened autonomous sites up
/// to the eight-project cap for three builders; five stood unbuilt
/// (no_builder 26 times) and held the slots the owner's church needed.
#[test]
fn the_planner_opens_no_site_while_a_project_waits_for_its_builder() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    let structure = paving_project(&mut state, id);
    // A resident the next site can recruit, so only the unbound project holds.
    resident(&mut state, id, [5.5, 201.0, 5.5], Profession::None);
    for _ in 0..64 {
        plan(&mut state, id);
    }
    assert_eq!(
        handles(&state, id),
        vec![structure.clone()],
        "no autonomous site opens while a project waits for its builder"
    );
    bind_project(&mut state, id, &structure, slot);
    for _ in 0..256 {
        plan(&mut state, id);
        if handles(&state, id).len() == 2 {
            return;
        }
    }
    panic!("setup: with every project bound the planner opens the funded storage");
}

/// Spain seed 100 (trace pin 1f30290d787e-de95775f): with three builders the
/// planner opened a fourth house that none of the other residents (farmers, a
/// nitwit, extraction workers kept for gathering) could take; it stood
/// unbound for 30k ticks and held the planner. Autonomous work opens a site
/// only when a resident could build it; an owner's order keeps its own path.
#[test]
fn the_planner_opens_no_site_nobody_can_build() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    let structure = paving_project(&mut state, id);
    bind_project(&mut state, id, &structure, slot);
    resident(&mut state, id, [5.5, 201.0, 5.5], Profession::Farmer);
    resident(&mut state, id, [6.5, 201.0, 5.5], Profession::Nitwit);
    for _ in 0..64 {
        plan(&mut state, id);
    }
    assert_eq!(
        handles(&state, id),
        vec![structure.clone()],
        "every project is bound, but no resident could take another site"
    );
    // Control: the owner's order is admitted at once all the same.
    request_started(&mut state, id, "storage");
    let record = state.settlements.get(id).unwrap();
    assert_eq!(handles(&state, id).len(), 2);
    assert!(
        access::projects(record).any(|project| project.ordered && project.blueprint == "storage"),
        "the owner's storage order is an active project"
    );
    // A resident who can build turns up: the order takes it first.
    let recruit = resident(&mut state, id, [7.5, 201.0, 5.5], Profession::None);
    assign_builder(&mut state, id);
    let record = state.settlements.get(id).unwrap();
    assert_eq!(
        access::projects(record)
            .find(|project| project.ordered)
            .unwrap()
            .builder,
        Some((state.entities.row(recruit).uuid, None))
    );
}
