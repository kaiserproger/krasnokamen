//! D8 queue rules: several owner requests, funded ones lift, the entry order
//! otherwise holds, and the queue is bounded.
use super::tests::{fixture, fund, open_plan};
use super::*;

/// Owner D8: a request whose materials are in storage moves ahead of unfunded
/// ones, so a funded request queued behind an unfunded head starts first while
/// the head keeps its position.
#[test]
fn a_funded_queued_order_lifts_over_an_unfunded_head() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    // The head cannot be funded from this fixture's stock; the house can.
    let head = OwnerOrder {
        seq: 0,
        blueprint: "spanish_plaza".into(),
        level: 1,
        building: None,
    };
    let lifted = OwnerOrder {
        seq: 0,
        blueprint: "house".into(),
        level: 1,
        building: None,
    };
    {
        let record = state.settlements.get_mut(id).unwrap();
        access::push_order(record, head);
        access::push_order(record, lifted);
    }
    fund(&mut state, "house", 1);
    // The site sweep may need several passes before the funded lot is found.
    for _ in 0..64 {
        plan(&mut state, id);
        if !state.settlements.get(id).unwrap().projects.is_empty() {
            break;
        }
    }
    let record = state.settlements.get(id).unwrap();
    assert_eq!(
        access::projects(record)
            .map(|project| project.blueprint.as_str())
            .collect::<Vec<_>>(),
        ["house"],
        "the funded request started while the unfunded head waited"
    );
    assert_eq!(
        access::orders(record)
            .map(|order| order.blueprint.as_str())
            .collect::<Vec<_>>(),
        ["spanish_plaza"],
        "the unfunded head keeps its place"
    );
}

/// The queue is bounded and the refusal names the limit.
#[test]
fn the_queue_refuses_an_order_beyond_its_limit() {
    let (mut state, id, _) = fixture();
    let record = state.settlements.get_mut(id).unwrap();
    for _ in 0..MAX_ORDERS {
        access::push_order(
            record,
            OwnerOrder {
                seq: 0,
                blueprint: "house".into(),
                level: 1,
                building: None,
            },
        );
    }
    let refused = request_build(&mut state, id, "house").unwrap_err();
    assert!(
        refused.contains(&MAX_ORDERS.to_string()),
        "the refusal names the queue limit: {refused}"
    );
}

/// One admission per planning pass, and a settlement may hold several projects
/// once each is funded on top of the others (D8).
#[test]
fn a_pass_admits_one_project_and_a_second_pass_admits_the_next() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    for _ in 0..2 {
        access::push_order(
            state.settlements.get_mut(id).unwrap(),
            OwnerOrder {
                seq: 0,
                blueprint: "house".into(),
                level: 1,
                building: None,
            },
        );
    }
    // Two houses' worth: the fixture writes one bill, so double every stack.
    fund(&mut state, "house", 1);
    for stack in state
        .block_entities
        .get_mut(&(1, 201, 1))
        .unwrap()
        .slots
        .iter_mut()
        .filter(|stack| stack.count > 0)
    {
        stack.count *= 2;
    }
    let mut passes = 0;
    while access::projects(state.settlements.get(id).unwrap()).count() == 0 && passes < 64 {
        plan(&mut state, id);
        passes += 1;
    }
    assert_eq!(
        access::projects(state.settlements.get(id).unwrap()).count(),
        1,
        "the first admission"
    );
    let mut passes = 0;
    while access::projects(state.settlements.get(id).unwrap()).count() == 1 && passes < 64 {
        plan(&mut state, id);
        passes += 1;
    }
    let record = state.settlements.get(id).unwrap();
    assert_eq!(
        access::projects(record).count(),
        2,
        "the next pass admits the second funded project"
    );
    let mut handles: Vec<&str> = access::projects(record)
        .map(|project| project.structure.as_str())
        .collect();
    handles.sort_unstable();
    handles.dedup();
    assert_eq!(handles.len(), 2, "each project owns its own structure");
}

/// The joint proof: a second project is not admitted out of the first one's
/// materials, even though it would be fundable on its own.
#[test]
fn the_joint_proof_keeps_a_second_project_out_of_the_first_ones_stock() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    for _ in 0..2 {
        access::push_order(
            state.settlements.get_mut(id).unwrap(),
            OwnerOrder {
                seq: 0,
                blueprint: "house".into(),
                level: 1,
                building: None,
            },
        );
    }
    fund(&mut state, "house", 1);
    for _ in 0..64 {
        plan(&mut state, id);
    }
    let record = state.settlements.get(id).unwrap();
    assert_eq!(
        access::projects(record).count(),
        1,
        "the second house is not funded on top of the first"
    );
    assert_eq!(
        access::orders(record)
            .map(|order| order.blueprint.as_str())
            .collect::<Vec<_>>(),
        ["house"],
        "the refused request keeps its place in the queue"
    );
}

/// The fixture town with its owner joined inside it, a house funded after the
/// join tick (whose planning pass then has no sweep to start), and the number
/// of street frontages its site sweep walks.
fn owner_in_open_plan() -> (TickState, SettlementId, usize) {
    const CONN: crate::boundary::ConnectionId = crate::boundary::ConnectionId(7);
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    crate::tick::run_tick(
        &mut state,
        &[crate::boundary::PlayerInput::Join {
            connection: CONN,
            uuid: uuid::Uuid::nil(),
            name: "Mayor".into(),
            permission_level: 0,
        }],
    );
    state.settlements.get_mut(id).unwrap().owner = Some(uuid::Uuid::nil().to_string());
    let entity = state.connections.entity_of(CONN).unwrap();
    let slot = state.entities.slot_of(entity).unwrap();
    state.entities.set_pos(slot, [8.5, 201.0, 8.5]);
    fund(&mut state, "house", 1);
    assert_eq!(examined(&state, id), None);
    let frontages = layout::Network::for_construction(&state, state.settlements.get(id).unwrap())
        .frontage()
        .len();
    assert!(
        frontages > 2 * SITE_ATTEMPTS as usize,
        "the sweep must outlast two slices: {frontages} frontages"
    );
    (state, id, frontages)
}

fn examined(state: &TickState, id: SettlementId) -> Option<u32> {
    state
        .settlements
        .site_sweep
        .get(&id)
        .and_then(|sweeps| sweeps.examined("house"))
}

/// Pin 7e421d34c060: `/settlement build spanish_plaza` on the seed-100 village
/// swept every street frontage inside its command tick (4.3 s). The command
/// tick examines one planner slice and answers at once; planner passes finish
/// the same resumable sweep, one slice each, and start the order.
#[test]
fn an_owner_order_examines_one_sweep_slice_in_its_command_tick() {
    let (mut state, id, frontages) = owner_in_open_plan();
    // No planning pass shares the command tick.
    assert!(
        !state.tick.is_multiple_of(PLAN_CADENCE) && !(state.tick + 1).is_multiple_of(PLAN_CADENCE)
    );
    let (_, outbox, _) = crate::tick::run_tick(
        &mut state,
        &[crate::boundary::PlayerInput::Command {
            connection: crate::boundary::ConnectionId(7),
            command: "settlement build house".into(),
            permission_level: 0,
        }],
    );
    let chat: Vec<String> = outbox
        .into_iter()
        .flat_map(|(_, changes)| changes)
        .filter_map(|change| match change {
            crate::boundary::PlayerChange::Chat { message } => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(chat, ["Queued house: waiting for a site"]);
    assert_eq!(examined(&state, id), Some(SITE_ATTEMPTS));
    let record = state.settlements.get(id).unwrap();
    assert!(record.projects.is_empty());
    assert_eq!(
        access::orders(record)
            .map(|order| order.blueprint.as_str())
            .collect::<Vec<_>>(),
        ["house"]
    );
    let mut passes = 0;
    while state.settlements.get(id).unwrap().projects.is_empty() {
        let before = examined(&state, id).unwrap_or(0);
        plan(&mut state, id);
        passes += 1;
        if let Some(after) = examined(&state, id) {
            assert!(after - before <= SITE_ATTEMPTS, "one slice per pass");
        }
        assert!(passes <= frontages, "the planner must finish the sweep");
    }
    assert!(passes >= frontages / SITE_ATTEMPTS as usize - 1);
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.projects[0].blueprint, "house");
    assert!(record.projects[0].ordered);
    assert_eq!(access::orders(record).count(), 0);
}

/// The sweep is runtime scratch: a restart mid-sweep keeps only the saved
/// order, whose next planning pass re-derives the sweep from its first slice
/// and still starts it.
#[test]
fn a_restart_mid_sweep_rederives_the_search_and_starts_the_order() {
    let (mut state, id, frontages) = owner_in_open_plan();
    assert_eq!(
        request_build(&mut state, id, "house").unwrap(),
        OrderOutcome::Queued {
            reason: site_refusals::WAITING_FOR_SITE.into()
        }
    );
    plan(&mut state, id);
    assert_eq!(examined(&state, id), Some(2 * SITE_ATTEMPTS));
    let restored: crate::settlements::SettlementRecord =
        serde_json::from_slice(&serde_json::to_vec(state.settlements.get(id).unwrap()).unwrap())
            .unwrap();
    crate::settlements::validate_record(&restored).unwrap();
    state.settlements.site_sweep.clear();
    state.settlements.site_watch.clear();
    state.settlements.order_refusal.clear();
    state
        .settlements
        .apply_image(crate::settlements::RecordImage {
            id,
            value: Some(restored),
        });
    plan(&mut state, id);
    assert_eq!(examined(&state, id), Some(SITE_ATTEMPTS));
    for _ in 0..frontages {
        if !state.settlements.get(id).unwrap().projects.is_empty() {
            break;
        }
        plan(&mut state, id);
    }
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.projects[0].blueprint, "house");
    assert_eq!(access::orders(record).count(), 0);
}
