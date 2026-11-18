use super::*;
use crate::settlements::allocator::tests::block;
use crate::settlements::construction::tests::{open_plan, start};

#[test]
fn replanting_respects_the_future_crown_around_an_active_house() {
    for (dx, dz, planted) in [
        (-1, 2, false),
        (-4, 2, false),
        (-5, 2, true),
        (-11, -11, true),
    ] {
        let (mut state, _, mut worker) =
            crate::villager::mining_tests::fixture(buildings::BuildingKind::LumberCamp);
        open_plan(&mut state);
        let id = worker.settlement.unwrap();
        start(&mut state, id, "house", 1, None);
        let origin = state.settlements.get(id).unwrap().projects[0].origin;
        // The retained Spanish house was blocked by a replanted root one cell
        // outside its west boundary: the protected crown grew into the site.
        let base = [origin[0] + dx, 201, origin[2] + dz];
        if matches!(dx, -4 | -5) {
            let crown = mc_script::ScriptWorldBounds::new(
                [base[0] - 4, base[1], base[2] - 4],
                [base[0] + 4, base[1] + 64, base[2] + 4],
            )
            .unwrap();
            assert_eq!(
                crown.max[0],
                origin[0] - i32::from(dx == -5),
                "radius-four crown touches the west face at -4, ends one column short at -5"
            );
            assert_eq!(
                crate::plugin_structures::settlement_builds::area_held(&state, crown),
                dx == -4,
                "touching the held site/corridor rejects; the next crown is disjoint"
            );
        }
        let dirt = block(&state, "minecraft:dirt", &[]);
        let sapling = block(&state, "minecraft:oak_sapling", &[]);
        state.world.set_block(base[0], base[1] - 1, base[2], dirt);
        state
            .world
            .set_block(base[0], base[1], base[2], state.registry.states.air);
        worker.lumber_progress.base = base;
        worker.lumber_progress.sapling = sapling;
        worker.lumber_progress.has_target = true;
        let mut changes = ChangeSet::default();
        replant(&mut state, &mut changes, &mut worker);
        assert_eq!(
            state.world.block_at(base[0], base[1], base[2]) == Some(sapling),
            planted,
            "root offset {dx} {dz}"
        );
        assert_eq!(
            state
                .settlements
                .get(id)
                .unwrap()
                .planted_trees
                .contains(&base),
            planted
        );
        assert!(
            !worker.lumber_progress.has_target,
            "skip releases the completed harvest target"
        );
    }
}

#[test]
fn replanting_respects_admitted_corridors_without_reserving_the_whole_world() {
    use crate::plugin_structures::settlement_builds::{self as executor, PlannedCell};
    use crate::settlements::construction::Project;
    use mc_script::ScriptWorldBounds;
    for blueprint in ["spanish_house", crate::settlements::layout::ROAD] {
        let (mut state, _, mut worker) =
            crate::villager::mining_tests::fixture(buildings::BuildingKind::LumberCamp);
        open_plan(&mut state);
        let id = worker.settlement.unwrap();
        let air = state.registry.states.air;
        let handle = executor::prepare(
            &mut state,
            id,
            blueprint.into(),
            ScriptWorldBounds::new([2, 200, 2], [26, 208, 23]).unwrap(),
            ScriptWorldBounds::new([16, 200, 16], [26, 208, 23]).unwrap(),
            vec![PlannedCell {
                portion: executor::PortionKind::Cut,
                at: [2, 201, 2],
                state: air,
                resource: None,
                quantity: 0,
            }],
        )
        .unwrap();
        state.settlements.get_mut(id).unwrap().projects = vec![Project {
            settlement: id,
            blueprint: blueprint.into(),
            level: 1,
            origin: [16, 200, 16],
            rotation: 0,
            building: None,
            structure: handle,
            ordered: false,
            builder: None,
            order_seq: None,
        }];
        for (base, planted) in [([2, 201, 2], false), ([-8, 201, -8], true)] {
            let dirt = block(&state, "minecraft:dirt", &[]);
            let sapling = block(&state, "minecraft:oak_sapling", &[]);
            state.world.set_block(base[0], base[1] - 1, base[2], dirt);
            state
                .world
                .set_block(base[0], base[1], base[2], state.registry.states.air);
            worker.lumber_progress.base = base;
            worker.lumber_progress.sapling = sapling;
            worker.lumber_progress.has_target = true;
            replant(&mut state, &mut ChangeSet::default(), &mut worker);
            assert_eq!(
                state.world.block_at(base[0], base[1], base[2]) == Some(sapling),
                planted,
                "{blueprint} root {base:?}"
            );
        }
    }
}

/// Seed-100 trial 2 planted over its only usable street access while no
/// construction envelope existed yet. Growing timber must leave that access.
#[test]
fn repeated_lumber_replants_leave_a_reachable_funded_house_lot() {
    use crate::settlements::construction;
    let (mut state, slot, mut worker) =
        crate::villager::mining_tests::fixture(buildings::BuildingKind::LumberCamp);
    open_plan(&mut state);
    let id = worker.settlement.unwrap();
    state.settlements.get_mut(id).unwrap().radius = 48;
    crate::villager::mining_tests::storage(&mut state, &worker, [1, 201, 1]);
    construction::tests::fund(&mut state, "spanish_house", 1);
    let dirt = block(&state, "minecraft:dirt", &[]);
    let sapling = block(&state, "minecraft:oak_sapling", &[]);
    let mut attempts = Vec::new();
    for coordinate in (-14..=30).step_by(2) {
        attempts.push([8, 201, coordinate]);
        attempts.push([coordinate, 201, 8]);
    }
    // Many real plantings remain possible away from the access strips.
    for x in (-12..=0).step_by(2) {
        for z in (-12..=0).step_by(2) {
            attempts.push([x, 201, z]);
        }
    }
    for base in &attempts {
        state.world.set_block(base[0], base[1] - 1, base[2], dirt);
        worker.lumber_progress.base = *base;
        worker.lumber_progress.sapling = sapling;
        worker.lumber_progress.has_target = true;
        replant(&mut state, &mut ChangeSet::default(), &mut worker);
        assert!(!worker.lumber_progress.has_target);
    }
    state.world.fold_overlay();
    state.entities.row_mut(slot).villager = Some(worker);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    assert_eq!(
        construction::request_build(&mut state, id, "spanish_house")
            .expect("many lumber plantings must leave a reachable fully funded house lot"),
        construction::OrderOutcome::Started,
    );
    let town = state.settlements.get(id).unwrap();
    assert_eq!(town.projects[0].blueprint, "spanish_house");
    assert!(town.planted_trees.len() >= 10, "not a blanket planting ban");
    assert!(
        town.planted_trees.len() < attempts.len(),
        "access plantings were refused"
    );
    for root in &town.planted_trees {
        assert_eq!(
            state.world.block_at(root[0], root[1], root[2]),
            Some(sapling)
        );
    }
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
}

#[test]
fn unused_street_does_not_discard_renewable_timber_or_the_last_house_lot() {
    use crate::settlements::construction;
    let (mut state, slot, mut worker) =
        crate::villager::mining_tests::fixture(buildings::BuildingKind::LumberCamp);
    open_plan(&mut state);
    let id = worker.settlement.unwrap();
    state.settlements.get_mut(id).unwrap().radius = 48;
    crate::villager::mining_tests::storage(&mut state, &worker, [1, 201, 1]);
    construction::tests::fund(&mut state, "spanish_house", 1);
    // This loaded avenue is on the grid, but other streets still serve the
    // standing buildings and a funded Spanish home remains buildable.
    let root = [-8, 201, 8];
    let dirt = block(&state, "minecraft:dirt", &[]);
    let sapling = block(&state, "minecraft:oak_sapling", &[]);
    state.world.set_block(root[0], root[1] - 1, root[2], dirt);
    worker.lumber_progress.base = root;
    worker.lumber_progress.sapling = sapling;
    worker.lumber_progress.has_target = true;
    let mut changes = ChangeSet::default();
    replant(&mut state, &mut changes, &mut worker);
    assert_eq!(
        state.world.block_at(root[0], root[1], root[2]),
        Some(sapling)
    );
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .planted_trees
            .contains(&root)
    );
    state.entities.row_mut(slot).villager = Some(worker);
    assert_eq!(
        construction::request_build(&mut state, id, "spanish_house").unwrap(),
        construction::OrderOutcome::Started,
    );
}
