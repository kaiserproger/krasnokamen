//! Held-prefix accounting uses native boundaries, not membership or wall time.
use crate::boundary::ChunkPos;
use crate::journal::{JournalEntry, Replay, SimDebtImage};
use crate::weather::{ClimateImage, GameClockImage};

const CHUNK: ChunkPos = ChunkPos { x: 0, z: 0 };

fn world() -> crate::world::ChunkWorld {
    let mut world = crate::world::world_tests::test_world(None);
    world.ensure_blocking(&[(CHUNK, 0)].into_iter().collect());
    world
}

fn membership(
    world: &mut crate::world::ChunkWorld,
    tick: u64,
    held: u64,
    before: u64,
    active: bool,
) {
    world.set_simulation_clock(held, before);
    world.set_active_chunks(
        if active {
            [CHUNK].into_iter().collect()
        } else {
            Default::default()
        },
        tick,
    );
}

fn image(world: &crate::world::ChunkWorld, tick: u64) -> SimDebtImage {
    world.debt_checkpoint(tick)[0]
}

fn climate(game_clock: GameClockImage) -> ClimateImage {
    ClimateImage {
        day_time: 0,
        game_clock,
        sim_debt_anchored_since: Some(0),
        weather_kind: 0,
        remaining_ticks: 12000,
        thunder_remaining_ticks: 12000,
        clear_remaining_ticks: 0,
        thundering: false,
        rain_level_bits: 0,
        thunder_level_bits: 0,
        advance_time: true,
        advance_weather: true,
    }
}

#[test]
fn debt_game_clock_multiple_holds_accrue_only_dormant_running_ticks() {
    let mut world = world();
    membership(&mut world, 10, 0, 0, true);
    membership(&mut world, 11, 1, 0, false);
    membership(&mut world, 20, 10, 9, false);
    membership(&mut world, 21, 10, 10, true);
    assert_eq!(world.simulation_debt(CHUNK), 0);
    assert_eq!(
        (image(&world, 21).last_sim, image(&world, 21).held_anchor),
        (21, Some(10))
    );
    membership(&mut world, 22, 11, 10, false);
    membership(&mut world, 30, 19, 18, false);
    membership(&mut world, 31, 19, 19, false);
    membership(&mut world, 32, 19, 19, false);
    membership(&mut world, 37, 24, 23, false);
    membership(&mut world, 38, 24, 24, true);
    assert_eq!(
        world.simulation_debt(CHUNK),
        2,
        "only running ticks 31 and 32 were dormant"
    );
    membership(&mut world, 39, 24, 24, false);
    membership(&mut world, 41, 24, 24, true);
    assert_eq!(world.simulation_debt(CHUNK), 4);
    assert_eq!(image(&world, 41).held_anchor, Some(24));
}

#[test]
fn debt_game_clock_held_activation_excludes_its_own_held_tick() {
    let mut world = world();
    membership(&mut world, 10, 0, 0, true);
    membership(&mut world, 11, 0, 0, false);
    membership(&mut world, 20, 8, 7, true);
    assert_eq!(world.simulation_debt(CHUNK), 2);
    assert_eq!(
        (image(&world, 20).last_sim, image(&world, 20).held_anchor),
        (20, Some(8))
    );
    world.refresh_active_clock(20);
    assert_eq!(
        world.simulation_debt(CHUNK),
        2,
        "same-output refresh is idempotent"
    );
}

#[test]
fn debt_game_clock_mid_tick_rejection_keeps_both_possible_native_anchors() {
    for after_refresh in [false, true] {
        let mut world = world();
        membership(&mut world, 10, 0, 0, true);
        if after_refresh {
            membership(&mut world, 11, 1, 0, true);
        }
        world.reject_resident(CHUNK, "held-clock regression".into());
        world.set_simulation_clock(1, 0);
        world.refresh_active_clock(11);
        let mut entries = Vec::new();
        world.journal_debt(&mut entries);
        let debt = entries
            .iter()
            .find_map(|entry| match entry {
                JournalEntry::SimDebt { image } => Some(image),
                _ => None,
            })
            .unwrap();
        assert!(!debt.active);
        assert_eq!(
            (debt.last_sim, debt.held_anchor),
            if after_refresh {
                (11, Some(1))
            } else {
                (10, Some(0))
            }
        );
        assert_eq!(debt.tick, 11);
    }
}

#[test]
fn debt_game_clock_native_projection_keeps_explicit_zero_and_one_key() {
    let mut world = world();
    membership(&mut world, 10, 0, 0, true);
    let mut chunk = world.chunk(CHUNK).unwrap().clone();
    for anchor in [0, 7, 9] {
        crate::debt::project(&mut chunk, 20, 3, Some(anchor)).unwrap();
        assert_eq!(
            chunk
                .extras
                .iter()
                .filter(|(key, _)| key == crate::debt::HELD_ANCHOR_KEY)
                .count(),
            1
        );
        assert!(
            chunk
                .extras
                .iter()
                .any(|(key, tag)| key == crate::debt::HELD_ANCHOR_KEY
                    && *tag == mc_nbt::Tag::Long(anchor as i64))
        );
    }
    let mut records = Default::default();
    crate::debt::adopt_native_extras(CHUNK, &std::sync::Arc::new(chunk), &mut records, 25, 10)
        .unwrap();
    assert_eq!(records[&CHUNK].held_anchor, Some(9));
    let zero = SimDebtImage {
        chunk: [0, 0],
        last_sim: 10,
        held_anchor: Some(0),
        remaining: 0,
        tick: 10,
        active: false,
    };
    assert_eq!(
        serde_json::from_slice::<SimDebtImage>(&serde_json::to_vec(&zero).unwrap()).unwrap(),
        zero
    );
}

#[test]
fn debt_game_clock_impossible_held_gap_is_a_storage_error() {
    for (last_sim, anchor, tick, held) in [
        (95, 0, 100, 10),
        (5, 6, 100, 10),
        (90, 11, 100, 10),
        (101, 10, 100, 10),
    ] {
        let error =
            crate::debt::validate_anchor(CHUNK, last_sim, Some(anchor), tick, held).unwrap_err();
        assert!(matches!(error, crate::CoreError::Storage(_)));
    }
}

#[test]
fn debt_game_clock_shared_projection_reanchors_all_dimensions_and_preserves_remaining() {
    use crate::dimensions::Dimension;
    let active = SimDebtImage {
        chunk: [0, 0],
        last_sim: 100,
        held_anchor: Some(20),
        remaining: 350,
        tick: 100,
        active: true,
    };
    let inactive = SimDebtImage {
        chunk: [1, 0],
        last_sim: 90,
        held_anchor: Some(20),
        remaining: 17,
        tick: 100,
        active: false,
    };
    let dimension = || Replay {
        tick: 100,
        sim_debt: [([0, 0], active), ([1, 0], inactive)].into(),
        ..Default::default()
    };
    let mut base = dimension();
    base.climate = Some(climate(GameClockImage::Running { held_ticks: 20 }));
    base.dimensions = [Dimension::Nether, Dimension::End]
        .into_iter()
        .map(|key| (key, Box::new(dimension())))
        .collect();
    let first = crate::journal::replay_batches(
        base,
        &[(
            101,
            120,
            vec![JournalEntry::Climate {
                value: climate(GameClockImage::Held {
                    game_ticks: 80,
                    held_since: Some(101),
                }),
            }],
        )],
    );
    let replay = crate::journal::replay_batches(
        first,
        &[
            (121, 125, vec![]),
            (
                126,
                126,
                vec![JournalEntry::Climate {
                    value: climate(GameClockImage::Running { held_ticks: 45 }),
                }],
            ),
        ],
    );
    crate::journal::validate_replay(&replay).unwrap();
    for replay in
        std::iter::once(&replay).chain(replay.dimensions.values().map(|value| value.as_ref()))
    {
        assert_eq!(
            (
                replay.sim_debt[&[0, 0]].last_sim,
                replay.sim_debt[&[0, 0]].held_anchor
            ),
            (126, Some(45))
        );
        assert_eq!(replay.sim_debt[&[0, 0]].remaining, 350);
        assert_eq!(
            (
                replay.sim_debt[&[1, 0]].last_sim,
                replay.sim_debt[&[1, 0]].held_anchor
            ),
            (90, Some(20))
        );
        assert_eq!(replay.sim_debt[&[1, 0]].remaining, 17);
    }
}

#[test]
fn debt_game_clock_anchorless_old_writer_is_refused_even_without_new_climate() {
    let base = Replay {
        tick: 100,
        climate: Some(climate(GameClockImage::Held {
            game_ticks: 80,
            held_since: Some(81),
        })),
        ..Default::default()
    };
    let old_pin_image = SimDebtImage {
        chunk: [0, 0],
        last_sim: 50,
        held_anchor: None,
        remaining: 350,
        tick: 101,
        active: false,
    };
    let replay = crate::journal::replay_batches(
        base,
        &[(
            101,
            101,
            vec![JournalEntry::SimDebt {
                image: old_pin_image,
            }],
        )],
    );
    let error = crate::journal::validate_replay(&replay).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("anchorless simulation-debt WAL image"),
        "{error}"
    );
}
