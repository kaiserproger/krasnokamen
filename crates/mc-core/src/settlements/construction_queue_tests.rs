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
        blueprint: "spanish_plaza".into(),
        level: 1,
        building: None,
    };
    let lifted = OwnerOrder {
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
