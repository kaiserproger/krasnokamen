//! Adopted generated pieces certify by function and substance, not by the
//! builder envelope (`buildings::assess`). The two fixtures are real vanilla
//! jigsaw boxes lifted from the seed-24 generator that produced the gate world:
//! a plains house keeps its ridge, eaves, door opening and yard non-solid, so the
//! all-or-nothing envelope refuses it while the piece plainly holds beds and a
//! door.
use crate::settlements::SettlementId;
use crate::settlements::allocator::tests::{block, test_state};
use crate::settlements::buildings::{self, BuildingKind};
use crate::tick::TickState;
use mc_script::ScriptWorldBounds;

use crate::villager::VillagerProfession as Profession;

type Cell = (
    [i32; 3],
    &'static str,
    &'static [(&'static str, &'static str)],
);

struct LiftedBox {
    name: &'static str,
    dims: [i32; 3],
    cells: &'static [Cell],
}

impl LiftedBox {
    fn bounds(&self, min: [i32; 3]) -> ScriptWorldBounds {
        ScriptWorldBounds::new(
            min,
            [
                min[0] + self.dims[0] - 1,
                min[1] + self.dims[1] - 1,
                min[2] + self.dims[2] - 1,
            ],
        )
        .unwrap()
    }

    /// Writes the box's non-air cells at `min`; every other cell of the box keeps
    /// the fixture's air, exactly as the generator leaves it.
    fn place(&self, state: &mut TickState, min: [i32; 3]) {
        for (offset, name, props) in self.cells {
            let id = block(state, &format!("minecraft:{name}"), props);
            state.world.set_block(
                min[0] + offset[0],
                min[1] + offset[1],
                min[2] + offset[2],
                id,
            );
        }
        state.world.fold_overlay();
    }

    /// The first cell of the box the fixture leaves empty, floor level first: a
    /// place to put a workstation without destroying the house's own furniture.
    fn free_cell(&self, min: [i32; 3]) -> [i32; 3] {
        for y in (1..self.dims[1]).chain(0..1) {
            for z in 0..self.dims[2] {
                for x in 0..self.dims[0] {
                    if !self.cells.iter().any(|(offset, _, _)| *offset == [x, y, z]) {
                        return [min[0] + x, min[1] + y, min[2] + z];
                    }
                }
            }
        }
        panic!("{} has no empty cell", self.name);
    }

    /// Places only the cells whose block path matches, the furniture a ruin keeps.
    fn place_where(&self, state: &mut TickState, min: [i32; 3], keep: impl Fn(&str) -> bool) {
        for (offset, name, props) in self.cells.iter().filter(|(_, name, _)| keep(name)) {
            let id = block(state, &format!("minecraft:{name}"), props);
            state.world.set_block(
                min[0] + offset[0],
                min[1] + offset[1],
                min[2] + offset[2],
                id,
            );
        }
        state.world.fold_overlay();
    }
}

fn town(state: &mut TickState) -> SettlementId {
    state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "adoption".into(),
            "owner".into(),
            [8, 201, 8],
            48,
        )
        .unwrap()
}

fn air(state: &mut TickState, bounds: ScriptWorldBounds) {
    let air = state.registry.states.air;
    for x in bounds.min[0]..=bounds.max[0] {
        for z in bounds.min[2]..=bounds.max[2] {
            for y in bounds.min[1]..=bounds.max[1] {
                state.world.set_block(x, y, z, air);
            }
        }
    }
    state.world.fold_overlay();
}

/// Fills `cells` air cells of the box with stone: the substance a wall or floor
/// would contribute, never overwriting furniture that already counts.
fn stone_box(state: &mut TickState, bounds: ScriptWorldBounds, cells: u32) {
    let stone = block(state, "minecraft:stone", &[]);
    let air = state.registry.states.air;
    let mut left = cells;
    'fill: for x in bounds.min[0]..=bounds.max[0] {
        for z in bounds.min[2]..=bounds.max[2] {
            for y in bounds.min[1]..=bounds.max[1] {
                if left == 0 {
                    break 'fill;
                }
                if state.world.block_at(x, y, z).is_some_and(|id| id == air) {
                    state.world.set_block(x, y, z, stone);
                    left -= 1;
                }
            }
        }
    }
    assert_eq!(left, 0, "the box has room for the substance");
    state.world.fold_overlay();
}

fn place_bed_and_door(state: &mut TickState, at: [i32; 3]) {
    let foot = block(
        state,
        "minecraft:red_bed",
        &[("part", "foot"), ("facing", "east")],
    );
    let head = block(
        state,
        "minecraft:red_bed",
        &[("part", "head"), ("facing", "east")],
    );
    state.world.set_block(at[0], at[1], at[2], foot);
    state.world.set_block(at[0] + 1, at[1], at[2], head);
    let lower = block(
        state,
        "minecraft:oak_door",
        &[("half", "lower"), ("facing", "east")],
    );
    let upper = block(
        state,
        "minecraft:oak_door",
        &[("half", "upper"), ("facing", "east")],
    );
    state.world.set_block(at[0], at[1], at[2] + 2, lower);
    state.world.set_block(at[0], at[1] + 1, at[2] + 2, upper);
    state.world.fold_overlay();
}

fn missing(error: &buildings::CertError, description: &str) -> u32 {
    error
        .missing
        .iter()
        .find(|requirement| requirement.description == description)
        .unwrap_or_else(|| panic!("no {description} in {:?}", error.missing))
        .found
}

/// The builder envelope refuses a real house box: this is the measured reason
/// the row exists (`missing=[footprint(0/9), roof(0/1), enclosure(0/1)]`).
#[test]
fn the_builder_envelope_still_refuses_a_real_house_box() {
    let mut state = test_state();
    let min = [1, 201, 1];
    let bounds = PLAINS_SMALL_HOUSE_1.bounds(min);
    air(&mut state, bounds);
    PLAINS_SMALL_HOUSE_1.place(&mut state, min);
    let error = buildings::inspect(&state, BuildingKind::House, 1, bounds)
        .expect_err("the strict envelope is the bug, not the house");
    assert!(missing(&error, "footprint") == 0, "{error:?}");
}

#[test]
fn a_real_plains_small_house_box_certifies_as_a_house() {
    let mut state = test_state();
    let id = town(&mut state);
    let min = [1, 201, 1];
    let bounds = PLAINS_SMALL_HOUSE_1.bounds(min);
    air(&mut state, bounds);
    PLAINS_SMALL_HOUSE_1.place(&mut state, min);
    let building = buildings::certify_generated(&mut state, id, BuildingKind::House, bounds)
        .unwrap_or_else(|error| {
            panic!(
                "{} at {min:?} does not certify: {error:?}",
                PLAINS_SMALL_HOUSE_1.name
            )
        });
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.buildings[&building].kind, BuildingKind::House);
    assert_eq!(record.buildings[&building].bounds, bounds);
    assert_eq!(record.buildings[&building].level, 1);
}

#[test]
fn a_real_plains_medium_house_box_certifies_as_a_house() {
    let mut state = test_state();
    let id = town(&mut state);
    let min = [1, 201, 1];
    let bounds = PLAINS_MEDIUM_HOUSE_1.bounds(min);
    air(&mut state, bounds);
    PLAINS_MEDIUM_HOUSE_1.place(&mut state, min);
    buildings::certify_generated(&mut state, id, BuildingKind::House, bounds).unwrap_or_else(
        |error| {
            panic!(
                "{} at {min:?} does not certify: {error:?}",
                PLAINS_MEDIUM_HOUSE_1.name
            )
        },
    );
}

/// A box of air is not a building, whatever a plan claims.
#[test]
fn an_air_box_certifies_nothing() {
    let mut state = test_state();
    let id = town(&mut state);
    let bounds = ScriptWorldBounds::new([1, 201, 1], [7, 207, 7]).unwrap();
    air(&mut state, bounds);
    let error = buildings::certify_generated(&mut state, id, BuildingKind::House, bounds)
        .expect_err("air is not a house");
    assert_eq!(missing(&error, "beds"), 0, "{error:?}");
    assert_eq!(missing(&error, "entrance"), 0, "{error:?}");
    assert_eq!(missing(&error, "structure"), 0, "{error:?}");
}

/// The furniture a ruin keeps is not the structure: four solid cells cannot
/// stand in for a house.
#[test]
fn a_lone_bed_and_door_do_not_make_a_house() {
    let mut state = test_state();
    let id = town(&mut state);
    let bounds = ScriptWorldBounds::new([1, 201, 1], [7, 207, 7]).unwrap();
    air(&mut state, bounds);
    place_bed_and_door(&mut state, [2, 202, 2]);
    // The function is there (one paired bed, one door), so neither is reported
    // missing; only the substance is, which is the point of the structure floor.
    // The test names no new API, so the row's reversal control still compiles.
    let error = buildings::certify_generated(&mut state, id, BuildingKind::House, bounds)
        .expect_err("a floating bed is not a house");
    assert_eq!(
        error.missing.len(),
        1,
        "only the substance is missing: {error:?}"
    );
    assert_eq!(missing(&error, "structure"), 4);
}

/// The real small house stripped to its furniture refuses on substance alone: the
/// rule is presence, so this is exactly where a ruin stops counting.
#[test]
fn a_real_house_reduced_to_its_furniture_refuses() {
    let mut state = test_state();
    let id = town(&mut state);
    let min = [1, 201, 1];
    let bounds = PLAINS_SMALL_HOUSE_1.bounds(min);
    air(&mut state, bounds);
    PLAINS_SMALL_HOUSE_1.place_where(&mut state, min, |name| {
        name.ends_with("_bed") || name.ends_with("_door")
    });
    let error = buildings::certify_generated(&mut state, id, BuildingKind::House, bounds)
        .expect_err("beds and a door alone are not a house");
    assert!(missing(&error, "structure") < 32, "{error:?}");
}

/// The substance floor is the boundary: 31 solid cells refuse, 32 certify, with
/// the same functional content in both boxes.
#[test]
fn the_structure_floor_is_the_boundary() {
    let bounds = ScriptWorldBounds::new([1, 201, 1], [7, 207, 7]).unwrap();
    for (stones, certifies) in [(27_u32, false), (28, true)] {
        let mut state = test_state();
        let id = town(&mut state);
        air(&mut state, bounds);
        place_bed_and_door(&mut state, [2, 202, 2]);
        // The bed pair and the door are 4 solid cells of their own.
        stone_box(&mut state, bounds, stones);
        let outcome = buildings::certify_generated(&mut state, id, BuildingKind::House, bounds);
        assert_eq!(
            outcome.is_ok(),
            certifies,
            "{stones} extra solid cells ({} in total): {outcome:?}",
            stones + 4
        );
        if let Err(error) = outcome {
            assert_eq!(missing(&error, "structure"), stones + 4, "{error:?}");
        }
    }
}

/// A library-shaped piece (a house box that also holds a lectern) certifies as
/// the House+Workshop pair on the piece's own bounds.
#[test]
fn a_piece_that_holds_its_own_workstation_certifies_as_the_pair() {
    let mut state = test_state();
    let id = town(&mut state);
    let min = [1, 201, 1];
    let bounds = PLAINS_SMALL_HOUSE_1.bounds(min);
    air(&mut state, bounds);
    PLAINS_SMALL_HOUSE_1.place(&mut state, min);
    let lectern_at = PLAINS_SMALL_HOUSE_1.free_cell(min);
    let lectern = block(&state, "minecraft:lectern", &[]);
    state
        .world
        .set_block(lectern_at[0], lectern_at[1], lectern_at[2], lectern);
    state.world.fold_overlay();
    let house = buildings::certify_generated(&mut state, id, BuildingKind::House, bounds)
        .expect("the house half");
    let workshop = buildings::certify_generated(
        &mut state,
        id,
        BuildingKind::Workshop(Profession::Librarian),
        bounds,
    )
    .expect("the library's lectern is the workshop half");
    assert_ne!(house, workshop);
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.buildings[&house].kind, BuildingKind::House);
    assert_eq!(
        record.buildings[&workshop].kind,
        BuildingKind::Workshop(Profession::Librarian)
    );
    assert_eq!(record.buildings[&workshop].work_points, vec![lectern_at]);
}

/// The strict callers are unchanged: the same real box is still refused when an
/// owner or construction certifies it, and the pair rule is still generated-only.
#[test]
fn the_strict_callers_still_refuse_the_real_house_box() {
    let mut state = test_state();
    let id = town(&mut state);
    let min = [1, 201, 1];
    let bounds = PLAINS_SMALL_HOUSE_1.bounds(min);
    air(&mut state, bounds);
    PLAINS_SMALL_HOUSE_1.place(&mut state, min);
    assert!(
        buildings::certify(&mut state, id, BuildingKind::House, bounds).is_err(),
        "a builder-built house keeps the envelope rule"
    );
}

// ---- lifted fixtures (generated by /tmp/lift_fixture.py from the probe log) ----

/// `plains_small_house_1` box [125, 77, -234]..[131, 83, -228], lifted from the seed-24 generator (content cache
/// 26.3): 343 cells, 172 non-air, 169 solid, 1 paired beds, 1 doors.
static PLAINS_SMALL_HOUSE_1: LiftedBox = LiftedBox {
    name: "minecraft:village/plains/houses/plains_small_house_1",
    dims: [7, 7, 7],
    cells: &[
        ([2, 0, 0], "grass_block", &[("snowy", "false")]),
        ([3, 0, 0], "grass_block", &[("snowy", "false")]),
        ([4, 0, 0], "grass_block", &[("snowy", "false")]),
        ([0, 0, 1], "grass_block", &[("snowy", "false")]),
        ([1, 0, 1], "stripped_oak_log", &[("axis", "y")]),
        ([2, 0, 1], "cobblestone", &[]),
        ([3, 0, 1], "cobblestone", &[]),
        ([4, 0, 1], "cobblestone", &[]),
        ([5, 0, 1], "stripped_oak_log", &[("axis", "y")]),
        ([6, 0, 1], "grass_block", &[("snowy", "false")]),
        ([0, 0, 2], "grass_block", &[("snowy", "false")]),
        ([1, 0, 2], "cobblestone", &[]),
        ([2, 0, 2], "oak_planks", &[]),
        ([3, 0, 2], "oak_planks", &[]),
        ([4, 0, 2], "oak_planks", &[]),
        ([5, 0, 2], "cobblestone", &[]),
        ([6, 0, 2], "grass_block", &[("snowy", "false")]),
        ([0, 0, 3], "grass_block", &[("snowy", "false")]),
        ([1, 0, 3], "cobblestone", &[]),
        ([2, 0, 3], "oak_planks", &[]),
        ([3, 0, 3], "oak_planks", &[]),
        ([4, 0, 3], "oak_planks", &[]),
        ([5, 0, 3], "cobblestone", &[]),
        (
            [6, 0, 3],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([0, 0, 4], "grass_block", &[("snowy", "false")]),
        ([1, 0, 4], "cobblestone", &[]),
        ([2, 0, 4], "oak_planks", &[]),
        ([3, 0, 4], "oak_planks", &[]),
        ([4, 0, 4], "oak_planks", &[]),
        ([5, 0, 4], "cobblestone", &[]),
        ([6, 0, 4], "grass_block", &[("snowy", "false")]),
        ([0, 0, 5], "grass_block", &[("snowy", "false")]),
        ([1, 0, 5], "stripped_oak_log", &[("axis", "y")]),
        ([2, 0, 5], "cobblestone", &[]),
        ([3, 0, 5], "cobblestone", &[]),
        ([4, 0, 5], "cobblestone", &[]),
        ([5, 0, 5], "stripped_oak_log", &[("axis", "y")]),
        ([6, 0, 5], "grass_block", &[("snowy", "false")]),
        ([0, 0, 6], "grass_block", &[("snowy", "false")]),
        ([1, 0, 6], "grass_block", &[("snowy", "false")]),
        ([2, 0, 6], "grass_block", &[("snowy", "false")]),
        ([3, 0, 6], "grass_block", &[("snowy", "false")]),
        ([4, 0, 6], "grass_block", &[("snowy", "false")]),
        ([5, 0, 6], "grass_block", &[("snowy", "false")]),
        ([6, 0, 6], "grass_block", &[("snowy", "false")]),
        ([1, 1, 1], "stripped_oak_log", &[("axis", "y")]),
        ([2, 1, 1], "cobblestone", &[]),
        ([3, 1, 1], "cobblestone", &[]),
        ([4, 1, 1], "cobblestone", &[]),
        ([5, 1, 1], "stripped_oak_log", &[("axis", "y")]),
        ([1, 1, 2], "cobblestone", &[]),
        (
            [2, 1, 2],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([5, 1, 2], "cobblestone", &[]),
        ([1, 1, 3], "cobblestone", &[]),
        (
            [5, 1, 3],
            "oak_door",
            &[
                ("facing", "west"),
                ("half", "lower"),
                ("hinge", "right"),
                ("open", "false"),
                ("powered", "false"),
            ],
        ),
        ([1, 1, 4], "cobblestone", &[]),
        (
            [2, 1, 4],
            "white_bed",
            &[("facing", "west"), ("occupied", "false"), ("part", "head")],
        ),
        (
            [3, 1, 4],
            "white_bed",
            &[("facing", "west"), ("occupied", "false"), ("part", "foot")],
        ),
        ([5, 1, 4], "cobblestone", &[]),
        ([1, 1, 5], "stripped_oak_log", &[("axis", "y")]),
        ([2, 1, 5], "cobblestone", &[]),
        ([3, 1, 5], "cobblestone", &[]),
        ([4, 1, 5], "cobblestone", &[]),
        ([5, 1, 5], "stripped_oak_log", &[("axis", "y")]),
        ([1, 2, 1], "stripped_oak_log", &[("axis", "y")]),
        ([2, 2, 1], "mossy_cobblestone", &[]),
        (
            [3, 2, 1],
            "glass_pane",
            &[
                ("east", "true"),
                ("north", "false"),
                ("south", "false"),
                ("waterlogged", "false"),
                ("west", "true"),
            ],
        ),
        ([4, 2, 1], "cobblestone", &[]),
        ([5, 2, 1], "stripped_oak_log", &[("axis", "y")]),
        ([1, 2, 2], "cobblestone", &[]),
        ([5, 2, 2], "cobblestone", &[]),
        ([6, 2, 2], "wall_torch", &[("facing", "east")]),
        (
            [1, 2, 3],
            "glass_pane",
            &[
                ("east", "false"),
                ("north", "true"),
                ("south", "true"),
                ("waterlogged", "false"),
                ("west", "false"),
            ],
        ),
        (
            [5, 2, 3],
            "oak_door",
            &[
                ("facing", "west"),
                ("half", "upper"),
                ("hinge", "right"),
                ("open", "false"),
                ("powered", "false"),
            ],
        ),
        ([1, 2, 4], "cobblestone", &[]),
        ([5, 2, 4], "cobblestone", &[]),
        ([6, 2, 4], "wall_torch", &[("facing", "east")]),
        ([1, 2, 5], "stripped_oak_log", &[("axis", "y")]),
        ([2, 2, 5], "cobblestone", &[]),
        (
            [3, 2, 5],
            "glass_pane",
            &[
                ("east", "true"),
                ("north", "false"),
                ("south", "false"),
                ("waterlogged", "false"),
                ("west", "true"),
            ],
        ),
        ([4, 2, 5], "cobblestone", &[]),
        ([5, 2, 5], "stripped_oak_log", &[("axis", "y")]),
        ([1, 3, 1], "stripped_oak_log", &[("axis", "y")]),
        ([2, 3, 1], "cobblestone", &[]),
        ([3, 3, 1], "cobblestone", &[]),
        ([4, 3, 1], "cobblestone", &[]),
        ([5, 3, 1], "stripped_oak_log", &[("axis", "y")]),
        ([1, 3, 2], "cobblestone", &[]),
        ([5, 3, 2], "cobblestone", &[]),
        ([1, 3, 3], "cobblestone", &[]),
        ([2, 3, 3], "wall_torch", &[("facing", "east")]),
        ([5, 3, 3], "cobblestone", &[]),
        ([1, 3, 4], "cobblestone", &[]),
        ([5, 3, 4], "cobblestone", &[]),
        ([1, 3, 5], "stripped_oak_log", &[("axis", "y")]),
        ([2, 3, 5], "cobblestone", &[]),
        ([3, 3, 5], "cobblestone", &[]),
        ([4, 3, 5], "cobblestone", &[]),
        ([5, 3, 5], "stripped_oak_log", &[("axis", "y")]),
        (
            [0, 4, 0],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "outer_left"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 4, 0],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 4, 0],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [3, 4, 0],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [4, 4, 0],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [5, 4, 0],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [6, 4, 0],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "outer_left"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [0, 4, 1],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([1, 4, 1], "oak_planks", &[]),
        ([2, 4, 1], "oak_planks", &[]),
        ([3, 4, 1], "oak_planks", &[]),
        ([4, 4, 1], "oak_planks", &[]),
        ([5, 4, 1], "oak_planks", &[]),
        (
            [6, 4, 1],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [0, 4, 2],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([1, 4, 2], "oak_planks", &[]),
        ([5, 4, 2], "oak_planks", &[]),
        (
            [6, 4, 2],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [0, 4, 3],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([1, 4, 3], "oak_planks", &[]),
        ([5, 4, 3], "oak_planks", &[]),
        (
            [6, 4, 3],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [0, 4, 4],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([1, 4, 4], "oak_planks", &[]),
        ([5, 4, 4], "oak_planks", &[]),
        (
            [6, 4, 4],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [0, 4, 5],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([1, 4, 5], "oak_planks", &[]),
        ([2, 4, 5], "oak_planks", &[]),
        ([3, 4, 5], "oak_planks", &[]),
        ([4, 4, 5], "oak_planks", &[]),
        ([5, 4, 5], "oak_planks", &[]),
        (
            [6, 4, 5],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [0, 4, 6],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "outer_left"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 4, 6],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 4, 6],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [3, 4, 6],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [4, 4, 6],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [5, 4, 6],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [6, 4, 6],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "outer_left"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 5, 1],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "outer_left"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 5, 1],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [3, 5, 1],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [4, 5, 1],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [5, 5, 1],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "outer_left"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 5, 2],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([2, 5, 2], "oak_planks", &[]),
        ([3, 5, 2], "oak_planks", &[]),
        ([4, 5, 2], "oak_planks", &[]),
        (
            [5, 5, 2],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 5, 3],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([2, 5, 3], "oak_planks", &[]),
        ([4, 5, 3], "oak_planks", &[]),
        (
            [5, 5, 3],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 5, 4],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([2, 5, 4], "oak_planks", &[]),
        ([3, 5, 4], "oak_planks", &[]),
        ([4, 5, 4], "oak_planks", &[]),
        (
            [5, 5, 4],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 5, 5],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "outer_right"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 5, 5],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [3, 5, 5],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [4, 5, 5],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [5, 5, 5],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "outer_right"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 6, 2],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "outer_right"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [3, 6, 2],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [4, 6, 2],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "outer_left"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 6, 3],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([3, 6, 3], "oak_planks", &[]),
        (
            [4, 6, 3],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 6, 4],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "outer_right"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [3, 6, 4],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [4, 6, 4],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "outer_right"),
                ("waterlogged", "false"),
            ],
        ),
    ],
};

/// `plains_medium_house_1` box [112, 70, -283]..[124, 77, -273], lifted from the seed-24 generator (content cache
/// 26.3): 1144 cells, 554 non-air, 543 solid, 2 paired beds, 1 doors.
static PLAINS_MEDIUM_HOUSE_1: LiftedBox = LiftedBox {
    name: "minecraft:village/plains/houses/plains_medium_house_1",
    dims: [13, 8, 11],
    cells: &[
        ([7, 0, 0], "grass_block", &[("snowy", "false")]),
        ([8, 0, 0], "grass_block", &[("snowy", "false")]),
        ([9, 0, 0], "grass_block", &[("snowy", "false")]),
        ([10, 0, 0], "grass_block", &[("snowy", "false")]),
        ([11, 0, 0], "dirt", &[]),
        ([12, 0, 0], "dirt", &[]),
        ([1, 0, 1], "dirt", &[]),
        ([2, 0, 1], "dirt", &[]),
        ([3, 0, 1], "dirt", &[]),
        ([4, 0, 1], "dirt", &[]),
        ([5, 0, 1], "dirt", &[]),
        ([6, 0, 1], "dirt", &[]),
        ([7, 0, 1], "dirt", &[]),
        ([8, 0, 1], "dirt", &[]),
        ([9, 0, 1], "dirt", &[]),
        ([10, 0, 1], "dirt", &[]),
        ([11, 0, 1], "dirt", &[]),
        ([12, 0, 1], "dirt", &[]),
        ([1, 0, 2], "dirt", &[]),
        ([2, 0, 2], "dirt", &[]),
        ([3, 0, 2], "dirt", &[]),
        ([4, 0, 2], "dirt", &[]),
        ([5, 0, 2], "dirt", &[]),
        ([6, 0, 2], "dirt", &[]),
        ([7, 0, 2], "dirt", &[]),
        ([8, 0, 2], "dirt", &[]),
        ([9, 0, 2], "dirt", &[]),
        ([10, 0, 2], "dirt", &[]),
        ([11, 0, 2], "dirt", &[]),
        ([12, 0, 2], "dirt", &[]),
        ([1, 0, 3], "dirt", &[]),
        ([2, 0, 3], "dirt", &[]),
        ([3, 0, 3], "dirt", &[]),
        ([4, 0, 3], "dirt", &[]),
        ([5, 0, 3], "dirt", &[]),
        ([6, 0, 3], "dirt", &[]),
        ([7, 0, 3], "dirt", &[]),
        ([8, 0, 3], "dirt", &[]),
        ([9, 0, 3], "dirt", &[]),
        ([10, 0, 3], "dirt", &[]),
        ([11, 0, 3], "dirt", &[]),
        ([12, 0, 3], "dirt", &[]),
        ([1, 0, 4], "dirt", &[]),
        ([2, 0, 4], "dirt", &[]),
        ([3, 0, 4], "dirt", &[]),
        ([4, 0, 4], "dirt", &[]),
        ([5, 0, 4], "dirt", &[]),
        ([6, 0, 4], "dirt", &[]),
        ([7, 0, 4], "dirt", &[]),
        ([8, 0, 4], "dirt", &[]),
        ([9, 0, 4], "dirt", &[]),
        ([10, 0, 4], "dirt", &[]),
        ([11, 0, 4], "dirt", &[]),
        ([12, 0, 4], "dirt", &[]),
        ([1, 0, 5], "dirt", &[]),
        ([2, 0, 5], "dirt", &[]),
        ([3, 0, 5], "dirt", &[]),
        ([4, 0, 5], "dirt", &[]),
        ([5, 0, 5], "dirt", &[]),
        ([6, 0, 5], "dirt", &[]),
        ([7, 0, 5], "dirt", &[]),
        ([8, 0, 5], "dirt", &[]),
        ([9, 0, 5], "dirt", &[]),
        ([10, 0, 5], "dirt", &[]),
        ([11, 0, 5], "dirt", &[]),
        ([12, 0, 5], "dirt", &[]),
        ([1, 0, 6], "dirt", &[]),
        ([2, 0, 6], "dirt", &[]),
        ([3, 0, 6], "dirt", &[]),
        ([4, 0, 6], "dirt", &[]),
        ([5, 0, 6], "dirt", &[]),
        ([6, 0, 6], "dirt", &[]),
        ([7, 0, 6], "dirt", &[]),
        ([8, 0, 6], "dirt", &[]),
        ([9, 0, 6], "dirt", &[]),
        ([10, 0, 6], "dirt", &[]),
        ([11, 0, 6], "dirt", &[]),
        ([12, 0, 6], "dirt", &[]),
        ([1, 0, 7], "dirt", &[]),
        ([2, 0, 7], "dirt", &[]),
        ([3, 0, 7], "dirt", &[]),
        ([4, 0, 7], "dirt", &[]),
        ([5, 0, 7], "dirt", &[]),
        ([6, 0, 7], "dirt", &[]),
        ([7, 0, 7], "dirt", &[]),
        ([8, 0, 7], "dirt", &[]),
        ([9, 0, 7], "dirt", &[]),
        ([10, 0, 7], "dirt", &[]),
        ([11, 0, 7], "dirt", &[]),
        ([12, 0, 7], "grass_block", &[("snowy", "false")]),
        ([6, 0, 8], "dirt", &[]),
        ([7, 0, 8], "dirt", &[]),
        ([8, 0, 8], "dirt", &[]),
        ([9, 0, 8], "dirt", &[]),
        ([10, 0, 8], "dirt", &[]),
        ([11, 0, 8], "dirt", &[]),
        ([12, 0, 8], "dirt", &[]),
        ([6, 0, 9], "dirt", &[]),
        ([7, 0, 9], "dirt", &[]),
        ([8, 0, 9], "dirt", &[]),
        ([9, 0, 9], "dirt", &[]),
        ([10, 0, 9], "dirt", &[]),
        ([11, 0, 9], "dirt", &[]),
        ([12, 0, 9], "dirt", &[]),
        ([2, 0, 10], "dirt", &[]),
        ([9, 0, 10], "grass_block", &[("snowy", "false")]),
        ([10, 0, 10], "grass_block", &[("snowy", "false")]),
        ([11, 0, 10], "grass_block", &[("snowy", "false")]),
        ([12, 0, 10], "grass_block", &[("snowy", "false")]),
        ([7, 1, 0], "dandelion", &[]),
        ([1, 1, 1], "stripped_oak_log", &[("axis", "y")]),
        ([2, 1, 1], "cobblestone", &[]),
        ([3, 1, 1], "cobblestone", &[]),
        ([4, 1, 1], "cobblestone", &[]),
        ([5, 1, 1], "mossy_cobblestone", &[]),
        ([6, 1, 1], "cobblestone", &[]),
        ([7, 1, 1], "cobblestone", &[]),
        ([8, 1, 1], "cobblestone", &[]),
        ([9, 1, 1], "cobblestone", &[]),
        ([10, 1, 1], "cobblestone", &[]),
        ([11, 1, 1], "stripped_oak_log", &[("axis", "y")]),
        ([1, 1, 2], "cobblestone", &[]),
        ([2, 1, 2], "oak_planks", &[]),
        ([3, 1, 2], "oak_planks", &[]),
        ([4, 1, 2], "oak_planks", &[]),
        ([5, 1, 2], "oak_planks", &[]),
        ([6, 1, 2], "oak_planks", &[]),
        ([7, 1, 2], "oak_planks", &[]),
        ([8, 1, 2], "oak_planks", &[]),
        ([9, 1, 2], "oak_planks", &[]),
        ([10, 1, 2], "oak_planks", &[]),
        ([11, 1, 2], "cobblestone", &[]),
        ([1, 1, 3], "cobblestone", &[]),
        ([2, 1, 3], "oak_planks", &[]),
        ([3, 1, 3], "oak_planks", &[]),
        ([4, 1, 3], "oak_planks", &[]),
        ([5, 1, 3], "oak_planks", &[]),
        ([6, 1, 3], "oak_planks", &[]),
        ([7, 1, 3], "oak_planks", &[]),
        ([8, 1, 3], "oak_planks", &[]),
        ([9, 1, 3], "oak_planks", &[]),
        ([10, 1, 3], "oak_planks", &[]),
        ([11, 1, 3], "cobblestone", &[]),
        ([1, 1, 4], "mossy_cobblestone", &[]),
        ([2, 1, 4], "oak_planks", &[]),
        ([3, 1, 4], "oak_planks", &[]),
        ([4, 1, 4], "oak_planks", &[]),
        ([5, 1, 4], "oak_planks", &[]),
        ([6, 1, 4], "oak_planks", &[]),
        ([7, 1, 4], "oak_planks", &[]),
        ([8, 1, 4], "oak_planks", &[]),
        ([9, 1, 4], "oak_planks", &[]),
        ([10, 1, 4], "oak_planks", &[]),
        ([11, 1, 4], "cobblestone", &[]),
        ([1, 1, 5], "cobblestone", &[]),
        ([2, 1, 5], "oak_planks", &[]),
        ([3, 1, 5], "oak_planks", &[]),
        ([4, 1, 5], "oak_planks", &[]),
        ([5, 1, 5], "oak_planks", &[]),
        ([6, 1, 5], "oak_planks", &[]),
        ([7, 1, 5], "oak_planks", &[]),
        ([8, 1, 5], "oak_planks", &[]),
        ([9, 1, 5], "oak_planks", &[]),
        ([10, 1, 5], "oak_planks", &[]),
        ([11, 1, 5], "cobblestone", &[]),
        ([1, 1, 6], "cobblestone", &[]),
        ([2, 1, 6], "oak_planks", &[]),
        ([3, 1, 6], "oak_planks", &[]),
        ([4, 1, 6], "oak_planks", &[]),
        ([5, 1, 6], "oak_planks", &[]),
        ([6, 1, 6], "oak_planks", &[]),
        ([7, 1, 6], "oak_planks", &[]),
        ([8, 1, 6], "oak_planks", &[]),
        ([9, 1, 6], "oak_planks", &[]),
        ([10, 1, 6], "oak_planks", &[]),
        ([11, 1, 6], "cobblestone", &[]),
        ([1, 1, 7], "stripped_oak_log", &[("axis", "y")]),
        ([2, 1, 7], "cobblestone", &[]),
        ([3, 1, 7], "cobblestone", &[]),
        ([4, 1, 7], "cobblestone", &[]),
        ([5, 1, 7], "cobblestone", &[]),
        ([6, 1, 7], "cobblestone", &[]),
        ([7, 1, 7], "oak_planks", &[]),
        ([8, 1, 7], "oak_planks", &[]),
        ([9, 1, 7], "oak_planks", &[]),
        ([10, 1, 7], "oak_planks", &[]),
        ([11, 1, 7], "cobblestone", &[]),
        (
            [12, 1, 7],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([6, 1, 8], "cobblestone", &[]),
        ([7, 1, 8], "oak_planks", &[]),
        ([8, 1, 8], "oak_planks", &[]),
        ([9, 1, 8], "oak_planks", &[]),
        ([10, 1, 8], "oak_planks", &[]),
        ([11, 1, 8], "cobblestone", &[]),
        ([6, 1, 9], "stripped_oak_log", &[("axis", "y")]),
        ([7, 1, 9], "cobblestone", &[]),
        ([8, 1, 9], "cobblestone", &[]),
        ([9, 1, 9], "cobblestone", &[]),
        ([10, 1, 9], "cobblestone", &[]),
        ([11, 1, 9], "stripped_oak_log", &[("axis", "y")]),
        ([1, 2, 1], "stripped_oak_log", &[("axis", "y")]),
        ([2, 2, 1], "cobblestone", &[]),
        ([3, 2, 1], "cobblestone", &[]),
        ([4, 2, 1], "cobblestone", &[]),
        ([5, 2, 1], "cobblestone", &[]),
        ([6, 2, 1], "mossy_cobblestone", &[]),
        ([7, 2, 1], "cobblestone", &[]),
        ([8, 2, 1], "cobblestone", &[]),
        ([9, 2, 1], "cobblestone", &[]),
        ([10, 2, 1], "cobblestone", &[]),
        ([11, 2, 1], "stripped_oak_log", &[("axis", "y")]),
        ([1, 2, 2], "cobblestone", &[]),
        (
            [2, 2, 2],
            "oak_fence",
            &[
                ("east", "false"),
                ("north", "true"),
                ("south", "false"),
                ("waterlogged", "false"),
                ("west", "true"),
            ],
        ),
        (
            [6, 2, 2],
            "oak_fence",
            &[
                ("east", "false"),
                ("north", "true"),
                ("south", "false"),
                ("waterlogged", "false"),
                ("west", "false"),
            ],
        ),
        ([11, 2, 2], "cobblestone", &[]),
        ([1, 2, 3], "cobblestone", &[]),
        (
            [2, 2, 3],
            "white_bed",
            &[("facing", "west"), ("occupied", "false"), ("part", "head")],
        ),
        (
            [3, 2, 3],
            "white_bed",
            &[("facing", "west"), ("occupied", "false"), ("part", "foot")],
        ),
        (
            [10, 2, 3],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([11, 2, 3], "cobblestone", &[]),
        ([1, 2, 4], "cobblestone", &[]),
        (
            [10, 2, 4],
            "oak_fence",
            &[
                ("east", "true"),
                ("north", "false"),
                ("south", "false"),
                ("waterlogged", "false"),
                ("west", "false"),
            ],
        ),
        ([11, 2, 4], "cobblestone", &[]),
        ([1, 2, 5], "cobblestone", &[]),
        (
            [2, 2, 5],
            "white_bed",
            &[("facing", "west"), ("occupied", "false"), ("part", "head")],
        ),
        (
            [3, 2, 5],
            "white_bed",
            &[("facing", "west"), ("occupied", "false"), ("part", "foot")],
        ),
        (
            [10, 2, 5],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([11, 2, 5], "cobblestone", &[]),
        ([1, 2, 6], "cobblestone", &[]),
        (
            [2, 2, 6],
            "oak_fence",
            &[
                ("east", "false"),
                ("north", "false"),
                ("south", "true"),
                ("waterlogged", "false"),
                ("west", "true"),
            ],
        ),
        ([11, 2, 6], "cobblestone", &[]),
        ([1, 2, 7], "stripped_oak_log", &[("axis", "y")]),
        ([2, 2, 7], "cobblestone", &[]),
        ([3, 2, 7], "cobblestone", &[]),
        ([4, 2, 7], "cobblestone", &[]),
        ([5, 2, 7], "cobblestone", &[]),
        ([6, 2, 7], "cobblestone", &[]),
        (
            [11, 2, 7],
            "oak_door",
            &[
                ("facing", "west"),
                ("half", "lower"),
                ("hinge", "left"),
                ("open", "false"),
                ("powered", "false"),
            ],
        ),
        ([6, 2, 8], "cobblestone", &[]),
        ([11, 2, 8], "cobblestone", &[]),
        ([6, 2, 9], "stripped_oak_log", &[("axis", "y")]),
        ([7, 2, 9], "mossy_cobblestone", &[]),
        ([8, 2, 9], "cobblestone", &[]),
        ([9, 2, 9], "cobblestone", &[]),
        ([10, 2, 9], "cobblestone", &[]),
        ([11, 2, 9], "stripped_oak_log", &[("axis", "y")]),
        ([1, 3, 1], "stripped_oak_log", &[("axis", "y")]),
        ([2, 3, 1], "oak_log", &[("axis", "y")]),
        (
            [3, 3, 1],
            "glass_pane",
            &[
                ("east", "true"),
                ("north", "false"),
                ("south", "false"),
                ("waterlogged", "false"),
                ("west", "true"),
            ],
        ),
        (
            [4, 3, 1],
            "glass_pane",
            &[
                ("east", "true"),
                ("north", "false"),
                ("south", "false"),
                ("waterlogged", "false"),
                ("west", "true"),
            ],
        ),
        ([5, 3, 1], "oak_log", &[("axis", "y")]),
        ([6, 3, 1], "stripped_oak_log", &[("axis", "x")]),
        ([7, 3, 1], "oak_log", &[("axis", "y")]),
        (
            [8, 3, 1],
            "glass_pane",
            &[
                ("east", "true"),
                ("north", "false"),
                ("south", "false"),
                ("waterlogged", "false"),
                ("west", "true"),
            ],
        ),
        (
            [9, 3, 1],
            "glass_pane",
            &[
                ("east", "true"),
                ("north", "false"),
                ("south", "false"),
                ("waterlogged", "false"),
                ("west", "true"),
            ],
        ),
        ([10, 3, 1], "oak_log", &[("axis", "y")]),
        ([11, 3, 1], "stripped_oak_log", &[("axis", "y")]),
        ([1, 3, 2], "cobblestone", &[]),
        ([2, 3, 2], "oak_pressure_plate", &[("powered", "false")]),
        ([6, 3, 2], "oak_pressure_plate", &[("powered", "false")]),
        ([11, 3, 2], "stripped_oak_log", &[("axis", "z")]),
        ([1, 3, 3], "cobblestone", &[]),
        ([11, 3, 3], "oak_log", &[("axis", "y")]),
        ([0, 3, 4], "wall_torch", &[("facing", "west")]),
        ([1, 3, 4], "cobblestone", &[]),
        ([2, 3, 4], "wall_torch", &[("facing", "east")]),
        ([10, 3, 4], "oak_pressure_plate", &[("powered", "false")]),
        (
            [11, 3, 4],
            "glass_pane",
            &[
                ("east", "false"),
                ("north", "true"),
                ("south", "true"),
                ("waterlogged", "false"),
                ("west", "false"),
            ],
        ),
        ([1, 3, 5], "mossy_cobblestone", &[]),
        ([11, 3, 5], "oak_log", &[("axis", "y")]),
        ([1, 3, 6], "cobblestone", &[]),
        ([2, 3, 6], "oak_pressure_plate", &[("powered", "false")]),
        ([11, 3, 6], "stripped_oak_log", &[("axis", "z")]),
        ([12, 3, 6], "wall_torch", &[("facing", "east")]),
        ([1, 3, 7], "stripped_oak_log", &[("axis", "y")]),
        ([2, 3, 7], "oak_log", &[("axis", "y")]),
        (
            [3, 3, 7],
            "glass_pane",
            &[
                ("east", "true"),
                ("north", "false"),
                ("south", "false"),
                ("waterlogged", "false"),
                ("west", "true"),
            ],
        ),
        (
            [4, 3, 7],
            "glass_pane",
            &[
                ("east", "true"),
                ("north", "false"),
                ("south", "false"),
                ("waterlogged", "false"),
                ("west", "true"),
            ],
        ),
        ([5, 3, 7], "oak_log", &[("axis", "y")]),
        ([6, 3, 7], "oak_planks", &[]),
        (
            [11, 3, 7],
            "oak_door",
            &[
                ("facing", "west"),
                ("half", "upper"),
                ("hinge", "left"),
                ("open", "false"),
                ("powered", "false"),
            ],
        ),
        ([6, 3, 8], "oak_planks", &[]),
        ([11, 3, 8], "stripped_oak_log", &[("axis", "z")]),
        ([12, 3, 8], "wall_torch", &[("facing", "east")]),
        ([6, 3, 9], "stripped_oak_log", &[("axis", "y")]),
        ([7, 3, 9], "oak_log", &[("axis", "y")]),
        (
            [8, 3, 9],
            "glass_pane",
            &[
                ("east", "true"),
                ("north", "false"),
                ("south", "false"),
                ("waterlogged", "false"),
                ("west", "true"),
            ],
        ),
        (
            [9, 3, 9],
            "glass_pane",
            &[
                ("east", "true"),
                ("north", "false"),
                ("south", "false"),
                ("waterlogged", "false"),
                ("west", "true"),
            ],
        ),
        ([10, 3, 9], "oak_log", &[("axis", "y")]),
        ([11, 3, 9], "stripped_oak_log", &[("axis", "y")]),
        (
            [12, 3, 9],
            "oak_leaves",
            &[
                ("distance", "1"),
                ("persistent", "false"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [12, 3, 10],
            "oak_leaves",
            &[
                ("distance", "2"),
                ("persistent", "false"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [0, 4, 0],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 4, 0],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 4, 0],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [3, 4, 0],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [4, 4, 0],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [5, 4, 0],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "inner_right"),
                ("waterlogged", "false"),
            ],
        ),
        ([6, 4, 0], "oak_planks", &[]),
        ([11, 4, 0], "oak_planks", &[]),
        (
            [12, 4, 0],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([0, 4, 1], "oak_planks", &[]),
        ([1, 4, 1], "stripped_oak_log", &[("axis", "y")]),
        ([2, 4, 1], "cobblestone", &[]),
        ([3, 4, 1], "cobblestone", &[]),
        ([4, 4, 1], "cobblestone", &[]),
        ([5, 4, 1], "cobblestone", &[]),
        ([6, 4, 1], "cobblestone", &[]),
        ([7, 4, 1], "mossy_cobblestone", &[]),
        ([8, 4, 1], "cobblestone", &[]),
        ([9, 4, 1], "cobblestone", &[]),
        ([10, 4, 1], "cobblestone", &[]),
        ([11, 4, 1], "stripped_oak_log", &[("axis", "y")]),
        (
            [12, 4, 1],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([1, 4, 2], "cobblestone", &[]),
        ([6, 4, 2], "wall_torch", &[("facing", "south")]),
        ([11, 4, 2], "oak_planks", &[]),
        (
            [12, 4, 2],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([1, 4, 3], "cobblestone", &[]),
        ([11, 4, 3], "oak_planks", &[]),
        (
            [12, 4, 3],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([1, 4, 4], "cobblestone", &[]),
        ([11, 4, 4], "oak_planks", &[]),
        (
            [12, 4, 4],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([1, 4, 5], "cobblestone", &[]),
        ([11, 4, 5], "oak_planks", &[]),
        (
            [12, 4, 5],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([1, 4, 6], "cobblestone", &[]),
        ([11, 4, 6], "oak_planks", &[]),
        (
            [12, 4, 6],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([0, 4, 7], "oak_planks", &[]),
        ([1, 4, 7], "stripped_oak_log", &[("axis", "y")]),
        ([2, 4, 7], "cobblestone", &[]),
        ([3, 4, 7], "cobblestone", &[]),
        ([4, 4, 7], "cobblestone", &[]),
        ([5, 4, 7], "cobblestone", &[]),
        ([6, 4, 7], "oak_planks", &[]),
        ([10, 4, 7], "wall_torch", &[("facing", "west")]),
        ([11, 4, 7], "oak_planks", &[]),
        (
            [12, 4, 7],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [0, 4, 8],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 4, 8],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 4, 8],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [3, 4, 8],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [4, 4, 8],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([5, 4, 8], "oak_planks", &[]),
        ([6, 4, 8], "oak_planks", &[]),
        ([11, 4, 8], "oak_planks", &[]),
        (
            [12, 4, 8],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [5, 4, 9],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([6, 4, 9], "stripped_oak_log", &[("axis", "y")]),
        ([7, 4, 9], "cobblestone", &[]),
        ([8, 4, 9], "cobblestone", &[]),
        ([9, 4, 9], "cobblestone", &[]),
        ([10, 4, 9], "cobblestone", &[]),
        ([11, 4, 9], "stripped_oak_log", &[("axis", "y")]),
        (
            [12, 4, 9],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [5, 4, 10],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([6, 4, 10], "oak_planks", &[]),
        ([11, 4, 10], "oak_planks", &[]),
        (
            [12, 4, 10],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [6, 5, 0],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([7, 5, 0], "oak_planks", &[]),
        ([10, 5, 0], "oak_planks", &[]),
        (
            [11, 5, 0],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [0, 5, 1],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 5, 1],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 5, 1],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [3, 5, 1],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [4, 5, 1],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [5, 5, 1],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [6, 5, 1],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "inner_left"),
                ("waterlogged", "false"),
            ],
        ),
        ([7, 5, 1], "oak_planks", &[]),
        ([8, 5, 1], "cobblestone", &[]),
        ([9, 5, 1], "cobblestone", &[]),
        ([10, 5, 1], "oak_planks", &[]),
        (
            [11, 5, 1],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([0, 5, 2], "oak_planks", &[]),
        ([1, 5, 2], "oak_planks", &[]),
        ([2, 5, 2], "oak_planks", &[]),
        ([3, 5, 2], "oak_planks", &[]),
        ([4, 5, 2], "oak_planks", &[]),
        ([5, 5, 2], "oak_planks", &[]),
        ([6, 5, 2], "oak_planks", &[]),
        ([7, 5, 2], "oak_planks", &[]),
        ([8, 5, 2], "oak_planks", &[]),
        ([9, 5, 2], "oak_planks", &[]),
        ([10, 5, 2], "oak_planks", &[]),
        (
            [11, 5, 2],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([1, 5, 3], "oak_log", &[("axis", "y")]),
        ([10, 5, 3], "oak_planks", &[]),
        (
            [11, 5, 3],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 5, 4],
            "glass_pane",
            &[
                ("east", "false"),
                ("north", "true"),
                ("south", "true"),
                ("waterlogged", "false"),
                ("west", "false"),
            ],
        ),
        ([10, 5, 4], "oak_planks", &[]),
        (
            [11, 5, 4],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([1, 5, 5], "oak_log", &[("axis", "y")]),
        ([10, 5, 5], "oak_planks", &[]),
        (
            [11, 5, 5],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([0, 5, 6], "oak_planks", &[]),
        ([1, 5, 6], "oak_planks", &[]),
        ([2, 5, 6], "oak_planks", &[]),
        ([3, 5, 6], "oak_planks", &[]),
        ([4, 5, 6], "oak_planks", &[]),
        ([5, 5, 6], "oak_planks", &[]),
        ([6, 5, 6], "oak_planks", &[]),
        ([7, 5, 6], "oak_planks", &[]),
        ([10, 5, 6], "oak_planks", &[]),
        (
            [11, 5, 6],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [0, 5, 7],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 5, 7],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 5, 7],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [3, 5, 7],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [4, 5, 7],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [5, 5, 7],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([6, 5, 7], "oak_planks", &[]),
        ([7, 5, 7], "oak_planks", &[]),
        ([10, 5, 7], "oak_planks", &[]),
        (
            [11, 5, 7],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [12, 5, 7],
            "oak_leaves",
            &[
                ("distance", "2"),
                ("persistent", "false"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [6, 5, 8],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([7, 5, 8], "oak_planks", &[]),
        ([10, 5, 8], "oak_planks", &[]),
        (
            [11, 5, 8],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [12, 5, 8],
            "oak_leaves",
            &[
                ("distance", "1"),
                ("persistent", "false"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [6, 5, 9],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([7, 5, 9], "oak_planks", &[]),
        ([8, 5, 9], "cobblestone", &[]),
        ([9, 5, 9], "mossy_cobblestone", &[]),
        ([10, 5, 9], "oak_planks", &[]),
        (
            [11, 5, 9],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [12, 5, 9],
            "oak_leaves",
            &[
                ("distance", "2"),
                ("persistent", "false"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [6, 5, 10],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([7, 5, 10], "oak_planks", &[]),
        ([10, 5, 10], "oak_planks", &[]),
        (
            [11, 5, 10],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [7, 6, 0],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([8, 6, 0], "oak_planks", &[]),
        ([9, 6, 0], "oak_planks", &[]),
        (
            [10, 6, 0],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [7, 6, 1],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([8, 6, 1], "oak_planks", &[]),
        ([9, 6, 1], "oak_planks", &[]),
        (
            [10, 6, 1],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [0, 6, 2],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 6, 2],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 6, 2],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [3, 6, 2],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [4, 6, 2],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [5, 6, 2],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [6, 6, 2],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([7, 6, 2], "oak_planks", &[]),
        ([8, 6, 2], "oak_planks", &[]),
        ([9, 6, 2], "oak_planks", &[]),
        (
            [10, 6, 2],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([0, 6, 3], "oak_planks", &[]),
        ([1, 6, 3], "oak_planks", &[]),
        ([2, 6, 3], "oak_planks", &[]),
        ([3, 6, 3], "oak_planks", &[]),
        ([4, 6, 3], "oak_planks", &[]),
        ([5, 6, 3], "oak_planks", &[]),
        ([6, 6, 3], "oak_planks", &[]),
        ([7, 6, 3], "oak_planks", &[]),
        ([8, 6, 3], "oak_planks", &[]),
        ([9, 6, 3], "oak_planks", &[]),
        (
            [10, 6, 3],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([1, 6, 4], "cobblestone", &[]),
        ([8, 6, 4], "oak_planks", &[]),
        ([9, 6, 4], "oak_planks", &[]),
        (
            [10, 6, 4],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([0, 6, 5], "oak_planks", &[]),
        ([1, 6, 5], "oak_planks", &[]),
        ([2, 6, 5], "oak_planks", &[]),
        ([3, 6, 5], "oak_planks", &[]),
        ([4, 6, 5], "oak_planks", &[]),
        ([5, 6, 5], "oak_planks", &[]),
        ([6, 6, 5], "oak_planks", &[]),
        ([7, 6, 5], "oak_planks", &[]),
        ([8, 6, 5], "oak_planks", &[]),
        ([9, 6, 5], "oak_planks", &[]),
        (
            [10, 6, 5],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [0, 6, 6],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 6, 6],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 6, 6],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [3, 6, 6],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [4, 6, 6],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [5, 6, 6],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [6, 6, 6],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([7, 6, 6], "oak_planks", &[]),
        ([8, 6, 6], "oak_planks", &[]),
        ([9, 6, 6], "oak_planks", &[]),
        (
            [10, 6, 6],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [7, 6, 7],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([8, 6, 7], "oak_planks", &[]),
        ([9, 6, 7], "oak_planks", &[]),
        (
            [10, 6, 7],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [7, 6, 8],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([8, 6, 8], "oak_planks", &[]),
        ([9, 6, 8], "oak_planks", &[]),
        (
            [10, 6, 8],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [12, 6, 8],
            "oak_leaves",
            &[
                ("distance", "2"),
                ("persistent", "false"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [7, 6, 9],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([8, 6, 9], "oak_planks", &[]),
        ([9, 6, 9], "oak_planks", &[]),
        (
            [10, 6, 9],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [7, 6, 10],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([8, 6, 10], "oak_planks", &[]),
        ([9, 6, 10], "oak_planks", &[]),
        (
            [10, 6, 10],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [8, 7, 0],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [9, 7, 0],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [8, 7, 1],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [9, 7, 1],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [8, 7, 2],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [9, 7, 2],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [0, 7, 3],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 7, 3],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 7, 3],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [3, 7, 3],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [4, 7, 3],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [5, 7, 3],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [6, 7, 3],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [7, 7, 3],
            "oak_stairs",
            &[
                ("facing", "south"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([8, 7, 3], "oak_planks", &[]),
        (
            [9, 7, 3],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([0, 7, 4], "oak_planks", &[]),
        ([1, 7, 4], "oak_planks", &[]),
        ([2, 7, 4], "oak_planks", &[]),
        ([3, 7, 4], "oak_planks", &[]),
        ([4, 7, 4], "oak_planks", &[]),
        ([5, 7, 4], "oak_planks", &[]),
        ([6, 7, 4], "oak_planks", &[]),
        ([7, 7, 4], "oak_planks", &[]),
        ([8, 7, 4], "oak_planks", &[]),
        (
            [9, 7, 4],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [0, 7, 5],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [1, 7, 5],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [2, 7, 5],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [3, 7, 5],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [4, 7, 5],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [5, 7, 5],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [6, 7, 5],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [7, 7, 5],
            "oak_stairs",
            &[
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        ([8, 7, 5], "oak_planks", &[]),
        (
            [9, 7, 5],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [8, 7, 6],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [9, 7, 6],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [8, 7, 7],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [9, 7, 7],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [8, 7, 8],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [9, 7, 8],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [8, 7, 9],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [9, 7, 9],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [8, 7, 10],
            "oak_stairs",
            &[
                ("facing", "east"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
        (
            [9, 7, 10],
            "oak_stairs",
            &[
                ("facing", "west"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false"),
            ],
        ),
    ],
};
