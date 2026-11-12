//! Clearing stands must let the builder advance the cell it walked to.
use super::building::{PREPARATION_REACH, preparation_stand};
use super::mining::near;
use crate::tick::TickState;

fn block(state: &TickState, name: &str) -> u32 {
    state
        .registry
        .blocks
        .block(&mc_data::Identifier::parse(name).unwrap())
        .unwrap()
        .default
        .0
}

/// Seed-24 civilization run gh1mr776: Lumber camp L1 (origin 113,72,-226) stopped
/// at clear cell 47, short grass at [121,76,-225] above the already cleared pit
/// floor (footing [121,73,-225]). The builder stood on the hill at
/// (121.5,76,-227.5), three blocks above and three rows south of that footing;
/// the nearest stand to itself was where it stood, out of felling reach, so it
/// never moved and never advanced for 42,000 ticks while holding the slot.
#[test]
fn clearing_stand_on_a_hill_edge_is_within_felling_reach_of_the_pit_footing() {
    let (mut state, id, builder) = crate::settlements::construction::tests::fixture();
    let dirt = block(&state, "minecraft:dirt");
    // Pit floor stays at the fixture ground (footing y 201); rows z 4-6 are a
    // hill three blocks higher (footing y 204), like z -229..-227 at seed 24.
    for x in 0..16 {
        for z in 4..=6 {
            for y in 201..=203 {
                state.world.set_block(x, y, z, dirt);
            }
        }
    }
    let next = [6, 204, 8];
    let grass = block(&state, "minecraft:short_grass");
    state.world.set_block(next[0], next[1], next[2], grass);
    state.world.fold_overlay();
    state.entities.set_pos(builder, [6.5, 204.0, 5.5]);
    let base = crate::settlements::forestry::footing(&state, id, next).expect("pit footing");
    assert_eq!((base.x, base.y, base.z), (6, 201, 8));
    assert!(
        !near(&state, builder, base, PREPARATION_REACH),
        "the hill edge itself is out of felling reach, as at seed 24"
    );
    let stand = preparation_stand(&state, builder, id, next, base).expect("a stand exists");
    state.entities.set_pos(
        builder,
        [
            f64::from(stand.x) + 0.5,
            f64::from(stand.y),
            f64::from(stand.z) + 0.5,
        ],
    );
    assert!(
        near(&state, builder, base, PREPARATION_REACH),
        "after walking to its chosen stand {stand:?} the builder must reach footing {base:?}"
    );
}
