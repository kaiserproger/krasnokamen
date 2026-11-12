//! Flowing water never washes kelp, seagrass or sugar cane (native-100 kelp wash-out).
//!
//! Vanilla 26.3 `FlowingFluid.canHoldFluid`: only `washed_away_by_fluids` blocks give
//! way, and `LiquidBlockContainer`s (kelp, kelp_plant, seagrass, tall_seagrass) refuse
//! a flow; sugar cane is not in the tag. A refused block stays and drops nothing.

use super::*;

/// The plant column (lowest block first) placed on the stone platform.
fn column(state: &TickState, name: &str) -> Vec<u32> {
    match name {
        "kelp" | "sugar_cane" => vec![block(state, name, &[("age", "0")])],
        "tall_seagrass" => vec![
            block(state, name, &[("half", "lower")]),
            block(state, name, &[("half", "upper")]),
        ],
        _ => vec![block(state, name, &[])],
    }
}

/// Run fluid ticks until the water has had time to spread everywhere it can.
fn flow(state: &mut TickState) -> Vec<EntityId> {
    let mut spawned = Vec::new();
    for _ in 0..40 {
        let mut writes = Vec::new();
        state.tick += 5;
        crate::fluids::run(state, &mut writes, &mut spawned);
    }
    spawned
}

#[test]
fn flowing_water_leaves_kelp_seagrass_and_sugar_cane_standing() {
    for name in [
        "kelp",
        "kelp_plant",
        "seagrass",
        "tall_seagrass",
        "sugar_cane",
    ] {
        for from_above in [true, false] {
            let (mut state, _, centre) = setup();
            let plants = column(&state, name);
            let source = state.registry.states.water_levels[0];
            for (dy, plant) in plants.iter().enumerate() {
                place(
                    &mut state,
                    BlockPos {
                        y: centre.y + dy as i32,
                        ..centre
                    },
                    *plant,
                );
            }
            // The kelp tip's water source above (the growth wake-up), or one beside
            // every block of the column.
            if from_above {
                let top = BlockPos {
                    y: centre.y + plants.len() as i32,
                    ..centre
                };
                place(&mut state, top, source);
            } else {
                for dy in 0..plants.len() as i32 {
                    let side = BlockPos {
                        x: centre.x + 1,
                        y: centre.y + dy,
                        ..centre
                    };
                    place(&mut state, side, source);
                }
            }
            let spawned = flow(&mut state);
            for (dy, plant) in plants.iter().enumerate() {
                let cell = BlockPos {
                    y: centre.y + dy as i32,
                    ..centre
                };
                assert_eq!(
                    at(&state, cell, 0),
                    *plant,
                    "{name} (water {}) stays",
                    if from_above { "above" } else { "beside" }
                );
            }
            assert!(spawned.is_empty(), "{name} drops no loot");
        }
    }
}

#[test]
fn flow_into_refuses_plants_and_still_washes_tagged_decoration() {
    let (mut state, _, centre) = setup();
    type WashCase = (&'static str, &'static [(&'static str, &'static str)], bool);
    let cases: [WashCase; 8] = [
        ("torch", &[], true),
        ("short_grass", &[], true),
        ("kelp", &[("age", "0")], false),
        ("kelp_plant", &[], false),
        ("seagrass", &[], false),
        ("sugar_cane", &[("age", "0")], false),
        ("bubble_column", &[("drag", "false")], false),
        (
            "oak_sign",
            &[("rotation", "0"), ("waterlogged", "false")],
            false,
        ),
    ];
    for (name, props, washed) in cases {
        let id = block(&state, name, props);
        place(&mut state, centre, id);
        let flow = crate::fluids::flow_into(&state, centre, crate::fluids::Fluid::Water);
        assert_eq!(
            matches!(flow, Some(crate::fluids::Flow::Wash(_))),
            washed,
            "{name}"
        );
        assert!(washed || flow.is_none(), "{name} blocks the flow");
    }
}

/// One kelp-growth write (the writes of `plants_growth::extend`) in a still pool wakes the
/// water around it; the wake-up must neither wash the grown column nor walk on to a neighbour.
#[test]
fn kelp_growth_write_in_a_pool_washes_no_column() {
    let (mut state, _, centre) = setup();
    let stone = state.registry.states.stone;
    let source = state.registry.states.water_levels[0];
    for dx in -3i32..=3 {
        for dz in -3i32..=3 {
            for dy in 1..=5 {
                let wall = dx.abs() == 3 || dz.abs() == 3;
                let cell = if wall { stone } else { source };
                state
                    .world
                    .set_block(centre.x + dx, Y + dy, centre.z + dz, cell);
            }
        }
    }
    let body = block(&state, "kelp_plant", &[]);
    let tip0 = block(&state, "kelp", &[("age", "0")]);
    let tip1 = block(&state, "kelp", &[("age", "1")]);
    let grown = BlockPos {
        x: centre.x,
        y: Y + 1,
        z: centre.z,
    };
    let other = BlockPos {
        x: centre.x + 2,
        ..grown
    };
    for col in [grown, other] {
        state.world.set_block(col.x, col.y, col.z, body);
        state.world.set_block(col.x, col.y + 1, col.z, tip0);
    }
    state.world.fold_overlay();
    // The growth: the old tip becomes a body block and a new tip is written above it.
    let up = BlockPos {
        y: grown.y + 1,
        ..grown
    };
    let tip = BlockPos {
        y: grown.y + 2,
        ..grown
    };
    state.world.set_block(up.x, up.y, up.z, body);
    state.world.set_block(tip.x, tip.y, tip.z, tip1);
    state.world.fold_overlay();
    crate::fluids::schedule(&mut state, &[(up, body), (tip, tip1)]);
    let mut spawned = Vec::new();
    for _ in 0..80 {
        let mut writes = Vec::new();
        state.tick += 1;
        crate::fluids::run(&mut state, &mut writes, &mut spawned);
    }
    for (cell, want) in [
        (grown, body),
        (up, body),
        (tip, tip1),
        (other, body),
        (
            BlockPos {
                y: other.y + 1,
                ..other
            },
            tip0,
        ),
    ] {
        assert_eq!(at(&state, cell, 0), want, "kelp column cell {cell:?} stays");
    }
    assert!(spawned.is_empty(), "no kelp is washed into an item");
}
