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
    plan(&mut state, id);
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
