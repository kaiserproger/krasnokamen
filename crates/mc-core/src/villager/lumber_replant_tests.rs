use super::*;
use crate::settlements::allocator::tests::block;
use crate::settlements::construction::tests::{open_plan, start};

#[test]
fn replanting_respects_the_future_crown_around_an_active_house() {
    for (offset, planted) in [(-1, false), (-4, false), (-5, true)] {
        let (mut state, _, mut worker) =
            crate::villager::mining_tests::fixture(buildings::BuildingKind::LumberCamp);
        open_plan(&mut state);
        let id = worker.settlement.unwrap();
        start(&mut state, id, "house", 1, None);
        let origin = state
            .settlements
            .get(id)
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .origin;
        // The retained Spanish house was blocked by a replanted root one cell
        // outside its west boundary: the protected crown grew into the site.
        let base = [origin[0] + offset, 201, origin[2] + 2];
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
            "root offset {offset}"
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
                at: [2, 201, 2],
                state: air,
                resource: None,
                quantity: 0,
            }],
        )
        .unwrap();
        state.settlements.get_mut(id).unwrap().project = Some(Project {
            settlement: id,
            blueprint: blueprint.into(),
            level: 1,
            origin: [16, 200, 16],
            rotation: 0,
            building: None,
            structure: handle,
            ordered: false,
            borrowed_builder: None,
        });
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
