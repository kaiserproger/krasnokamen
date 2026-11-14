//! Actual native autocraft admission/portion/storage paths in the domain fixture.
use super::*;

#[test]
fn multi_portion_final_pickaxes_deliver_without_distributing_chain_intermediates() {
    let mut town = village(&[("minecraft:oak_log", 2), ("minecraft:wooden_pickaxe", 1)]);
    town.equip("minecraft:wooden_axe");
    let steps = town
        .submit(
            "pick-two",
            "krasnokamen-settlements:wooden_pickaxe_ordinary",
            2,
        )
        .unwrap();
    assert_eq!(
        describe(&steps),
        "oak_planks x2 -> stick x1 -> wooden_pickaxe_ordinary x2"
    );
    town.run_until("pick-two-1");
    assert_eq!(town.carried("minecraft:oak_planks"), 8);
    assert_eq!(town.stored("minecraft:oak_planks"), 0);
    assert!(town.jobs()["pick-two-1"].delivery.is_none());
    town.run_until("pick-two-2");
    assert_eq!(town.carried("minecraft:oak_planks"), 6);
    assert_eq!(town.carried("minecraft:stick"), 4);
    assert!(town.jobs()["pick-two-2"].delivery.is_none());
    town.run_until("pick-two");
    assert_eq!(town.jobs()["pick-two"].done, 2);
    assert_eq!(town.jobs()["pick-two"].receipts.len(), 2);
    // The first final portion was delivered before making the second; a
    // preexisting identical tool in STORAGE is not a new production credit.
    assert_eq!(town.stored("minecraft:wooden_pickaxe"), 2);
    assert_eq!(town.carried("minecraft:wooden_pickaxe"), 1);
    assert!(!town.jobs()["pick-two"].delivery_complete());
    for _ in 0..2_000 {
        if town.jobs()["pick-two"].delivery_complete() {
            break;
        }
        town.step();
    }
    assert!(town.jobs()["pick-two"].delivery_complete());
    assert_eq!(town.stored("minecraft:wooden_pickaxe"), 3);
    assert_eq!(town.carried("minecraft:wooden_pickaxe"), 0);
    assert_eq!(town.carried("minecraft:oak_planks"), 0);
    assert_eq!(town.carried("minecraft:stick"), 0);
    let jobs = town.jobs();
    assert_eq!(
        (
            jobs["pick-two-1"].receipts.len(),
            jobs["pick-two-2"].receipts.len(),
            jobs["pick-two"].receipts.len()
        ),
        (2, 1, 2)
    );
    assert_eq!(jobs["pick-two"].delivery.as_ref().unwrap().sequence, 2);
    assert_eq!(
        town.state.entities.row(town.slot).appearance.equipment[0]
            .as_ref()
            .unwrap()
            .damage,
        Some(2)
    );
    let policy = &town
        .state
        .settlements
        .get(town.id)
        .unwrap()
        .economy
        .production;
    assert_eq!((policy.completed, policy.cancelled), (3, 0));
    assert!(!production::active(policy));
    validate_record(town.state.settlements.get(town.id).unwrap()).unwrap();
}
