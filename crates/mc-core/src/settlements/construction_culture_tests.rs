use super::tests::{complete, fixture, fund, start};
use super::*;

#[test]
fn spanish_blueprints_finish_through_paid_resident_construction() {
    for name in [
        "spanish_house",
        "spanish_church",
        "spanish_plaza",
        "spanish_fortress",
        "spanish_wall",
    ] {
        let (mut state, id, worker) = fixture();
        start(&mut state, id, name, 1, None);
        assert!(
            !spanish_faction(&state, state.settlements.get(id).unwrap()),
            "an order is not provenance"
        );
        fund(&mut state, name, 1);
        complete(&mut state, id, worker);
        let town = state.settlements.get(id).unwrap();
        assert!(town.projects.is_empty(), "{name}");
        let building = town.buildings.values().next().unwrap();
        assert_eq!(placement(&state, id, building).unwrap().0.id, name);
        assert!(spanish_faction(&state, town), "{name}");
        assert_eq!(
            catalog(&state, town)
                .find(|b| b.kind == BuildingKind::House)
                .unwrap()
                .id,
            "spanish_house"
        );
        assert_eq!(
            catalog(&state, town)
                .find(|b| b.kind == BuildingKind::Storage)
                .unwrap()
                .id,
            "spanish_plaza"
        );
    }
}

#[test]
fn style_follows_owner_cohort_but_not_names_pending_orders_or_unowned_peers() {
    let (mut state, parent, worker) = fixture();
    let peer = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "spanish_name_is_not_style".into(),
            "owner".into(),
            [80, 201, 8],
            48,
        )
        .unwrap();
    let foreign = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "foreign".into(),
            "other-owner".into(),
            [160, 201, 8],
            48,
        )
        .unwrap();
    assert!(!spanish_faction(
        &state,
        state.settlements.get(peer).unwrap()
    ));
    state.settlements.get_mut(parent).unwrap().orders = vec![OwnerOrder {
        blueprint: "spanish_house".into(),
        level: 1,
        building: None,
    }];
    assert!(!spanish_faction(
        &state,
        state.settlements.get(peer).unwrap()
    ));
    state.settlements.get_mut(parent).unwrap().orders.clear();
    start(&mut state, parent, "spanish_house", 1, None);
    fund(&mut state, "spanish_house", 1);
    complete(&mut state, parent, worker);
    assert!(spanish_faction(
        &state,
        state.settlements.get(peer).unwrap()
    ));
    assert!(!spanish_faction(
        &state,
        state.settlements.get(foreign).unwrap()
    ));
    let record = state.settlements.get_mut(parent).unwrap();
    record.name = "renamed".into();
    assert!(spanish_faction(
        &state,
        state.settlements.get(peer).unwrap()
    ));
    state.settlements.get_mut(parent).unwrap().owner = None;
    state.settlements.get_mut(peer).unwrap().owner = None;
    assert!(spanish_faction(
        &state,
        state.settlements.get(parent).unwrap()
    ));
    assert!(!spanish_faction(
        &state,
        state.settlements.get(peer).unwrap()
    ));
}
