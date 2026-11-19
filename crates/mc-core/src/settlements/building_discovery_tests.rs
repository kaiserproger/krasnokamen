use super::*;
use crate::settlements::allocator::tests::{block, test_state};
use crate::settlements::buildings::{self, BuildingKind};

pub(crate) fn room(state: &mut TickState, min: [i32; 2], max: [i32; 2], roof: bool) {
    let stone = state.registry.states.stone;
    let air = state.registry.states.air;
    for x in min[0]..=max[0] {
        for z in min[1]..=max[1] {
            state.world.set_block(x, 200, z, stone);
            for y in 201..=204 {
                let wall = (x == min[0] || x == max[0] || z == min[1] || z == max[1]) && y < 204;
                state.world.set_block(
                    x,
                    y,
                    z,
                    if wall || (roof && y == 204) {
                        stone
                    } else {
                        air
                    },
                );
            }
        }
    }
    let lower = block(state, "minecraft:oak_door", &[("half", "lower")]);
    let upper = block(state, "minecraft:oak_door", &[("half", "upper")]);
    state.world.set_block(min[0], 201, min[1] + 2, lower);
    state.world.set_block(min[0], 202, min[1] + 2, upper);
    state.world.fold_overlay();
}

#[test]
fn closed_house_finds_floor_walls_roof_and_all_requirement_counts() {
    let mut state = test_state();
    room(&mut state, [3, 3], [7, 7], true);
    crate::settlements::allocator::tests::bed(&mut state, 5, 5);
    state.world.fold_overlay();
    let bounds = find(&state, [4, 201, 4]).unwrap();
    assert_eq!(
        bounds,
        ScriptWorldBounds::new([3, 200, 3], [7, 204, 7]).unwrap()
    );
    let checks = buildings::assess(&state, BuildingKind::House, 1, bounds)
        .unwrap()
        .requirements;
    assert_eq!(
        checks.len(),
        buildings::requirements(BuildingKind::House, 1).len()
    );
    assert!(
        checks
            .iter()
            .all(|requirement| requirement.found >= requirement.required)
    );
    assert_eq!(
        checks
            .iter()
            .find(|requirement| requirement.description == "beds")
            .unwrap()
            .found,
        1
    );
}

#[test]
fn missing_roof_remains_a_checklist_failure_not_a_discovery_failure() {
    let mut state = test_state();
    room(&mut state, [3, 3], [7, 7], false);
    let bounds = find(&state, [4, 201, 4]).unwrap();
    assert_eq!(bounds.max[1], 204);
    let checks = buildings::assess(&state, BuildingKind::House, 1, bounds)
        .unwrap()
        .requirements;
    let roof = checks
        .iter()
        .find(|requirement| requirement.description == "roof")
        .unwrap();
    assert_eq!((roof.found, roof.required), (0, 1));
    assert_eq!(
        checks
            .iter()
            .find(|requirement| requirement.description == "enclosure")
            .unwrap()
            .found,
        1
    );
}

#[test]
fn open_wall_refuses_instead_of_certifying_an_outdoor_box() {
    let mut state = test_state();
    room(&mut state, [3, 3], [7, 7], true);
    state.world.set_block(7, 201, 4, state.registry.states.air);
    state.world.set_block(7, 202, 4, state.registry.states.air);
    state.world.fold_overlay();
    assert_eq!(find(&state, [4, 201, 4]), Err(Refusal::NotEnclosed));
}

#[test]
fn adjacent_rooms_do_not_borrow_each_others_beds_even_through_an_open_door() {
    let mut state = test_state();
    room(&mut state, [3, 3], [7, 7], true);
    room(&mut state, [7, 3], [11, 7], true);
    for (y, half) in [(201, "lower"), (202, "upper")] {
        let door = block(
            &state,
            "minecraft:oak_door",
            &[("half", half), ("open", "true")],
        );
        state.world.set_block(7, y, 5, door);
    }
    crate::settlements::allocator::tests::bed(&mut state, 9, 5);
    state.world.fold_overlay();
    let first = find(&state, [4, 201, 4]).unwrap();
    let second = find(&state, [8, 201, 4]).unwrap();
    assert_eq!((first.min[0], first.max[0]), (3, 7));
    assert_eq!((second.min[0], second.max[0]), (7, 11));
    let counts = |bounds| {
        buildings::assess(&state, BuildingKind::House, 1, bounds)
            .unwrap()
            .requirements
            .into_iter()
            .find(|requirement| requirement.description == "beds")
            .unwrap()
            .found
    };
    assert_eq!(counts(first), 0);
    assert_eq!(counts(second), 1);
}

#[test]
fn player_outside_has_no_discovered_building() {
    let mut state = test_state();
    room(&mut state, [3, 3], [7, 7], true);
    assert_eq!(find(&state, [9, 201, 9]), Err(Refusal::OpenAir));
}

#[test]
fn enclosure_over_sixty_four_blocks_refuses_with_a_size_limit() {
    let mut state = test_state();
    let chunks = (0..=4)
        .map(|x| (crate::boundary::ChunkPos { x, z: 0 }, 0))
        .collect();
    state.world.ensure_blocking(&chunks);
    room(&mut state, [3, 3], [67, 7], true);
    assert_eq!(find(&state, [35, 201, 4]), Err(Refusal::TooLarge));
}

#[test]
fn rechecking_an_existing_certified_house_preserves_its_identity() {
    let mut state = test_state();
    room(&mut state, [3, 3], [7, 7], true);
    crate::settlements::allocator::tests::bed(&mut state, 5, 5);
    state.world.fold_overlay();
    let settlement = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "oak".into(),
            "owner".into(),
            [8, 201, 8],
            48,
        )
        .unwrap();
    let bounds = find(&state, [4, 201, 4]).unwrap();
    let first = buildings::certify(&mut state, settlement, BuildingKind::House, bounds).unwrap();
    let again = find(&state, [6, 201, 4]).unwrap();
    assert_eq!(
        buildings::certify(&mut state, settlement, BuildingKind::House, again),
        Ok(first)
    );
    assert_eq!(
        state.settlements.get(settlement).unwrap().buildings.len(),
        1
    );
    assert_eq!(
        state.settlements.get(settlement).unwrap().buildings[&first].bounds,
        bounds
    );
}
