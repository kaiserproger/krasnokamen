use mc_data::Identifier;
use mc_data::builtin_sounds_26_3::{BIG_DRIPLEAF_TILT_DOWN, BIG_DRIPLEAF_TILT_UP};

use super::*;
use crate::bees::tests::{POS, joined, place, state};
use crate::boundary::{ConnectionId, PlayerChange};

fn tilt(state: &TickState) -> &str {
    crate::farming::prop(
        &state.registry.blocks,
        state.world.block_at(POS.x, POS.y, POS.z).unwrap(),
        "tilt",
    )
    .unwrap()
}

fn set(state: &mut TickState, value: &str) {
    let id = state.world.block_at(POS.x, POS.y, POS.z).unwrap();
    let tilted = crate::farming::with_prop(&state.registry.blocks, id, "tilt", value).unwrap();
    state.world.set_block(POS.x, POS.y, POS.z, tilted);
}

/// Tilt sound names heard by the joined player since the last call; each is a
/// BLOCKS sound at the leaf centre, volume 1, pitch in 0.8..1.2.
fn sounds(state: &mut TickState) -> Vec<String> {
    state
        .outbox
        .remove(&ConnectionId(1))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|change| match change {
            PlayerChange::CommandSound {
                sound,
                source,
                position,
                volume,
                pitch,
                ..
            } => {
                assert_eq!(source, crate::command_sound::BLOCKS);
                assert_eq!(
                    position,
                    [POS.x * 8 + 4, POS.y * 8 + 4, POS.z * 8 + 4],
                    "the block centre"
                );
                assert_eq!(volume, 1.0);
                assert!((0.8..1.2).contains(&pitch), "{pitch}");
                Some(sound.to_string())
            }
            _ => None,
        })
        .collect()
}

fn leaf() -> (TickState, Identifier) {
    let mut state = state(None);
    joined(&mut state);
    place(&mut state, POS, "minecraft:big_dripleaf");
    state.outbox.clear();
    (state, Identifier::parse("minecraft:big_dripleaf").unwrap())
}

/// `BigDripleafBlock.tick`: unstable -> partial -> full each with the tilt-down
/// sound and a rescheduled tick, then full -> none with the tilt-up sound and
/// no further tick.
#[test]
fn scheduled_ticks_walk_the_tilt_and_reset_a_full_leaf() {
    let (mut state, id) = leaf();
    set(&mut state, "unstable");
    let mut writes = Vec::new();
    for (next, sound) in [
        ("partial", BIG_DRIPLEAF_TILT_DOWN),
        ("full", BIG_DRIPLEAF_TILT_DOWN),
        ("none", BIG_DRIPLEAF_TILT_UP),
    ] {
        state.block_ticks = Default::default();
        scheduled(&mut state, POS, &mut writes);
        assert_eq!(tilt(&state), next);
        assert_eq!(sounds(&mut state), [sound]);
        assert_eq!(
            state.block_ticks.contains(POS, &id, 0),
            next != "none",
            "{next} schedules its successor"
        );
    }
    assert_eq!(writes.len(), 1, "one published cell, latest state");
    scheduled(&mut state, POS, &mut writes);
    assert_eq!(tilt(&state), "none");
    assert!(sounds(&mut state).is_empty(), "a level leaf stays silent");
}

/// `tick` and `neighborChanged` reset a tilted leaf under a neighbour signal.
#[test]
fn a_neighbour_signal_resets_a_tilted_leaf() {
    let (mut state, _) = leaf();
    set(&mut state, "partial");
    changed(&mut state, POS);
    assert_eq!(tilt(&state), "partial", "no signal: no reset");
    place(
        &mut state,
        BlockPos {
            x: POS.x + 1,
            ..POS
        },
        "minecraft:redstone_block",
    );
    changed(&mut state, POS);
    assert_eq!(tilt(&state), "none");
    assert_eq!(sounds(&mut state), [BIG_DRIPLEAF_TILT_UP]);
    set(&mut state, "full");
    scheduled(&mut state, POS, &mut Vec::new());
    assert_eq!(tilt(&state), "none");
}

/// A pending tick is not rescheduled: vanilla keeps one per cell and block.
#[test]
fn a_projectile_hit_keeps_the_pending_tick() {
    let (mut state, id) = leaf();
    projectile_hit(&mut state, POS);
    assert_eq!(tilt(&state), "full");
    assert_eq!(sounds(&mut state), [BIG_DRIPLEAF_TILT_DOWN]);
    assert!(state.block_ticks.contains(POS, &id, 0));
    let queued = state.block_ticks.image();
    state.tick += 50;
    projectile_hit(&mut state, POS);
    assert_eq!(state.block_ticks.image(), queued);
}
