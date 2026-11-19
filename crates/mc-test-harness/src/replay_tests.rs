use super::*;
use serde_json::{Value, json};

#[test]
fn concurrent_contract_rejects_group_shapes_the_executor_cannot_replay() {
    let checked = include_str!(
        "../../../tools/core-replay-scenarios/multiplayer-transactions-seed-8102.json"
    );

    let mut three_contenders: Value = serde_json::from_str(checked).expect("checked JSON");
    three_contenders["concurrent_groups"][0]["actions"]
        .as_array_mut()
        .expect("placement actions")
        .push(json!({
            "type": "place_block",
            "actor": "third-player",
            "item": "minecraft:cobblestone"
        }));
    assert!(
        ReplayScenarioManifest::from_json(&three_contenders.to_string()).is_err(),
        "same-target fixture currently supports exactly two contenders"
    );

    let mut repeated_chest: Value = serde_json::from_str(checked).expect("checked JSON");
    repeated_chest["concurrent_groups"][1]["repetitions"] = json!(2);
    assert!(
        ReplayScenarioManifest::from_json(&repeated_chest.to_string()).is_err(),
        "shared chest fixture currently represents one state-id race"
    );

    let mut split_slots: Value = serde_json::from_str(checked).expect("checked JSON");
    split_slots["concurrent_groups"][1]["actions"][1]["slot"] = json!(1);
    assert!(
        ReplayScenarioManifest::from_json(&split_slots.to_string()).is_err(),
        "shared chest contenders must target the same slot"
    );
}

#[test]
fn checked_core_action_fixture_is_strict_and_complete() {
    let fixture = include_str!("../../../tools/core-replay-scenarios/core-actions-seed-81.json");
    let scenario = ReplayScenarioManifest::from_json(fixture).expect("checked fixture parses");

    assert_eq!(scenario.id, "core-actions-seed-81");
    assert_eq!(scenario.seed, 81);
    assert_eq!(scenario.actions.len(), 4);
    assert_eq!(
        scenario
            .lanes
            .iter()
            .map(|lane| lane.driver)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            ReplayDriver::KrasnokamenProtocol,
            ReplayDriver::VanillaOracle,
            ReplayDriver::RealClient,
        ])
    );
    assert_eq!(scenario.expected_invariants.len(), 2);
    assert!(
        scenario
            .lanes
            .iter()
            .all(|lane| !lane.required_gates.is_empty())
    );
    let canonical = scenario
        .to_pretty_json()
        .expect("serialize checked fixture");
    assert_eq!(
        ReplayScenarioManifest::from_json(&canonical).expect("parse canonical fixture"),
        scenario
    );

    let mut wrong_schema: Value = serde_json::from_str(fixture).expect("fixture JSON");
    wrong_schema["schema"] = json!("krasnokamen.core_replay.scenario.v2");
    assert!(ReplayScenarioManifest::from_json(&wrong_schema.to_string()).is_err());

    let mut missing_seed: Value = serde_json::from_str(fixture).expect("fixture JSON");
    missing_seed
        .as_object_mut()
        .expect("fixture object")
        .remove("seed");
    assert!(ReplayScenarioManifest::from_json(&missing_seed.to_string()).is_err());
}

#[test]
fn checked_multiplayer_transaction_fixture_is_strict_and_complete() {
    let fixture = include_str!(
        "../../../tools/core-replay-scenarios/multiplayer-transactions-seed-8102.json"
    );
    let scenario = ReplayScenarioManifest::from_json(fixture)
        .expect("checked multiplayer transaction fixture parses");

    assert_eq!(scenario.id, "multiplayer-transactions-seed-8102");
    assert_eq!(scenario.seed, 8102);
    assert!(scenario.actions.is_empty());
    assert_eq!(scenario.concurrent_groups.len(), 2);
    assert_eq!(scenario.state_expectations.len(), 2);
    assert_eq!(scenario.lanes.len(), 1);
    assert_eq!(scenario.lanes[0].driver, ReplayDriver::KrasnokamenProtocol);
    assert_eq!(scenario.expected_invariants.len(), 3);
    let canonical = scenario
        .to_pretty_json()
        .expect("serialize checked multiplayer fixture");
    assert_eq!(
        ReplayScenarioManifest::from_json(&canonical).expect("parse canonical multiplayer fixture"),
        scenario
    );
}
