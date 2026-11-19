//! Game-clock freeze is independent of a resident chunk's activity clock.
use super::*;

fn item_base(age: u32, tick: u64) -> (TickState, EntityId, Replay) {
    let mut state = fixture();
    state.tick = tick;
    let item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap();
    let id = crate::tick::spawn_item_entity(
        &mut state,
        [8.5, 65.0, 8.5],
        mc_data::ItemStack::new(item, 1),
    );
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.row_mut(slot).age = age;
    let (_, chunks, _) = checkpoint_data_for(&state, tick);
    let mut record = replay_record(&state, &chunks[&(0, 0)].entities[0]);
    record.tick = tick;
    let replay = Replay {
        tick,
        entities: [(id.0, record)].into(),
        sim_debt: active_clock(tick),
        climate: Some(crate::weather::ClimateImage::capture(&state)),
        ..Default::default()
    };
    (state, id, replay)
}

#[test]
fn frozen_game_clock_keeps_resident_item_age_equal_to_live_through_wal_replay() {
    let (mut state, id, base) = item_base(5999, 200);
    state.tick_control.set_frozen(true);
    let mut batches = Vec::new();
    for _ in 0..32 {
        let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
        let entries = journal_entries_for_tick(&mut state, &output).unwrap();
        batches.push((output.tick, output.tick, entries));
    }
    let slot = state.entities.slot_of(id).unwrap();
    assert_eq!(state.entities.row(slot).age, 5999);
    assert_eq!(state.tick_control.game_ticks(state.tick), 200);
    assert!(state.world.is_active(0, 0));
    let replay = replay_batches(base, &batches);
    assert_eq!(replay.entities[&id.0].age, 5999);
    assert_eq!(replay.entities[&id.0].state.age, 5999);
    assert_eq!(replay.entities[&id.0].item.as_ref().unwrap().count, 1);
}

#[test]
fn frozen_game_clock_gates_compact_vitals_and_final_projection() {
    let (_, id, mut base) = item_base(5999, 200);
    let climate = base.climate.as_mut().unwrap();
    climate.game_clock = crate::weather::GameClockImage::Held {
        game_ticks: 199,
        held_since: Some(200),
    };
    let replay = replay_batches(
        base,
        &[
            (
                201,
                230,
                vec![JournalEntry::EntityVitals {
                    id: id.0,
                    health: 1.0,
                    alive: true,
                    tick: 220,
                }],
            ),
            (231, 300, vec![]),
        ],
    );
    assert_eq!(replay.entities[&id.0].age, 5999);
    assert_eq!(replay.entities[&id.0].state.age, 5999);
    assert_eq!(replay.entities[&id.0].tick, 300);
}

#[test]
fn frozen_game_clock_preserves_steps_inactive_intervals_and_streamed_fold_boundaries() {
    use crate::weather::GameClockImage::{Held, Running};
    for inactive in [false, true] {
        let (_, id, base) = item_base(5900, 200);
        let mut climate = base.climate.unwrap();
        let mut clock = |value| {
            climate.game_clock = value;
            JournalEntry::Climate { value: climate }
        };
        let mut middle = vec![
            clock(Running { held_ticks: 10 }),
            JournalEntry::EntityVitals {
                id: id.0,
                health: 1.0,
                alive: true,
                tick: 213,
            },
            clock(Held {
                game_ticks: 205,
                held_since: Some(216),
            }),
        ];
        let mut last = vec![
            clock(Running { held_ticks: 24 }),
            clock(Held {
                game_ticks: 206,
                held_since: Some(231),
            }),
        ];
        if inactive {
            middle.push(JournalEntry::SimDebt {
                image: crate::journal::SimDebtImage {
                    chunk: [0, 0],
                    last_sim: 212,
                    remaining: 0,
                    active: false,
                    tick: 220,
                },
            });
            last.push(JournalEntry::SimDebt {
                image: crate::journal::SimDebtImage {
                    chunk: [0, 0],
                    last_sim: 230,
                    remaining: 0,
                    active: true,
                    tick: 230,
                },
            });
        }
        let batches = [
            (
                201,
                210,
                vec![clock(Held {
                    game_ticks: 200,
                    held_since: Some(201),
                })],
            ),
            (211, 220, middle),
            (221, 240, last),
        ];
        let expected = 5900 + if inactive { 3 } else { 6 };
        let replay = replay_batches(base, &batches);
        assert_eq!(replay.entities[&id.0].age, expected);
        let replay = replay_batches(replay, &batches);
        assert_eq!(replay.entities[&id.0].age, expected, "stale fold");
        for project_each_batch in [false, true] {
            let (_, _, mut streamed) = item_base(5900, 200);
            for batch in &batches {
                streamed =
                    crate::journal::apply_batch_images(streamed, std::slice::from_ref(batch));
                if project_each_batch {
                    crate::journal::advance_replay_clocks(&mut streamed);
                }
            }
            crate::journal::advance_replay_clocks(&mut streamed);
            assert_eq!(streamed.entities[&id.0].age, expected);
        }
    }
}

#[test]
fn game_clock_rejects_ambiguous_counters_and_conflicting_held_boundaries() {
    use crate::weather::GameClockImage::{Held, Running};
    let state = fixture();
    let mut climate = crate::weather::ClimateImage::capture(&state);
    for image in [
        Running { held_ticks: 1 },
        Held {
            game_ticks: 200,
            held_since: Some(203),
        },
    ] {
        let mut game =
            crate::journal::ReplayGameClock::new(200, Some(Running { held_ticks: 0 })).unwrap();
        climate.game_clock = image;
        assert!(
            game.observe_batches(&[(201, 210, vec![JournalEntry::Climate { value: climate },])])
                .is_err()
        );
    }
    let mut game = crate::journal::ReplayGameClock::new(
        200,
        Some(Held {
            game_ticks: 100,
            held_since: Some(150),
        }),
    )
    .unwrap();
    climate.game_clock = Running { held_ticks: 110 };
    game.observe_batches(&[(201, 211, vec![JournalEntry::Climate { value: climate }])])
        .unwrap();
    climate.game_clock = Held {
        game_ticks: 101,
        held_since: Some(213),
    };
    assert!(
        game.observe_batches(&[(212, 220, vec![JournalEntry::Climate { value: climate },])])
            .is_err()
    );
}

#[test]
fn game_clock_shared_overworld_envelope_gates_untouched_dimension_children() {
    use crate::dimensions::Dimension;
    for dimension in [Dimension::Nether, Dimension::End] {
        let (_, id, mut child) = item_base(5999, 200);
        let mut climate = child.climate.take().unwrap();
        climate.game_clock = crate::weather::GameClockImage::Held {
            game_ticks: 200,
            held_since: Some(201),
        };
        let base = Replay {
            tick: 200,
            dimensions: [(dimension, Box::new(child))].into(),
            ..Default::default()
        };
        let mut replay = replay_batches(
            base,
            &[(
                201,
                232,
                vec![
                    JournalEntry::Dimension {
                        dimension,
                        entries: vec![JournalEntry::EntityVitals {
                            id: id.0,
                            health: 1.0,
                            alive: true,
                            tick: 220,
                        }],
                    },
                    JournalEntry::Dimension {
                        dimension: Dimension::Overworld,
                        entries: vec![JournalEntry::Climate { value: climate }],
                    },
                ],
            )],
        );
        assert_eq!(replay.dimensions[&dimension].entities[&id.0].age, 5999);
        assert_eq!(replay.dimensions[&dimension].entities[&id.0].tick, 232);
        for _ in 0..2 {
            crate::journal::advance_replay_clocks(&mut replay);
            assert_eq!(replay.dimensions[&dimension].entities[&id.0].age, 5999);
        }
        let replay = replay_batches(replay, &[(233, 250, vec![])]);
        assert_eq!(replay.dimensions[&dimension].entities[&id.0].age, 5999);
        assert_eq!(replay.dimensions[&dimension].entities[&id.0].tick, 250);
    }
}

#[test]
fn game_clock_mid_hold_checkpoint_bridges_fresh_memories_not_stale_or_expired_ones() {
    use crate::villager::{Activity, VillagerState};
    use crate::weather::GameClockImage::{Held, Running};
    let state = fixture();
    for (activity, stale) in [
        (Activity::Work, false),
        (Activity::Work, true),
        (Activity::Rest, false),
        (Activity::Rest, true),
        (Activity::Panic, false),
    ] {
        let native = mc_data::entity_nbt::VanillaEntity {
            id: "minecraft:villager".into(),
            ..Default::default()
        };
        let mut record = replay_record(&state, &native);
        record.tick = 200;
        record.villager = Some(Box::new(VillagerState {
            brain_clock: true,
            age: -200,
            activity,
            sleeping: activity == Activity::Rest,
            last_work: if stale { 148 } else { 149 },
            last_slept: Some(if stale { 148 } else { 149 }),
            panic_until: 249,
            ..Default::default()
        }));
        let mut climate = crate::weather::ClimateImage::capture(&state);
        climate.game_clock = Held {
            game_ticks: 100,
            held_since: Some(151),
        };
        let base = Replay {
            tick: 200,
            climate: Some(climate),
            sim_debt: active_clock(200),
            entities: [(1, record)].into(),
            ..Default::default()
        };
        let held = replay_batches(base, &[(201, 300, vec![])]);
        let memory = held.entities[&1].villager.as_ref().unwrap();
        assert_eq!(memory.age, -200);
        assert_eq!(memory.panic_until, 249, "deadline does not pause");
        climate.game_clock = Running { held_ticks: 200 };
        let mut resumed = replay_batches(
            held,
            &[(301, 302, vec![JournalEntry::Climate { value: climate }])],
        );
        let memory = resumed.entities[&1].villager.as_ref().unwrap();
        assert_eq!(memory.age, -198);
        match activity {
            Activity::Work => assert_eq!(memory.last_work, if stale { 148 } else { 301 }),
            Activity::Rest => assert_eq!(memory.last_slept, Some(if stale { 148 } else { 301 })),
            Activity::Panic => assert_eq!(memory.panic_until, 249, "expired across the hold"),
            _ => unreachable!(),
        }
        let mut step_batches = Vec::new();
        for (lo, hi, clock) in [
            (
                303,
                312,
                Held {
                    game_ticks: 102,
                    held_since: Some(303),
                },
            ),
            (313, 313, Running { held_ticks: 210 }),
            (
                314,
                323,
                Held {
                    game_ticks: 103,
                    held_since: Some(314),
                },
            ),
            (324, 324, Running { held_ticks: 220 }),
            (
                325,
                326,
                Held {
                    game_ticks: 104,
                    held_since: Some(325),
                },
            ),
        ] {
            climate.game_clock = clock;
            step_batches.push((lo, hi, vec![JournalEntry::Climate { value: climate }]));
        }
        resumed = replay_batches(resumed, &step_batches);
        let memory = resumed.entities[&1].villager.as_ref().unwrap();
        assert_eq!(memory.age, -196);
        match activity {
            Activity::Work => assert_eq!(memory.last_work, if stale { 148 } else { 323 }),
            Activity::Rest => assert_eq!(memory.last_slept, Some(if stale { 148 } else { 323 })),
            Activity::Panic => assert_eq!(memory.panic_until, 249),
            _ => unreachable!(),
        }
    }
}

#[test]
fn game_clock_curing_countdown_advances_only_on_two_running_steps() {
    use crate::weather::GameClockImage::{Held, Running};
    let state = fixture();
    let native = mc_data::entity_nbt::VanillaEntity {
        id: "minecraft:zombie_villager".into(),
        ..Default::default()
    };
    let mut record = replay_record(&state, &native);
    record.tick = 100;
    record.villager = Some(Box::new(crate::villager::VillagerState {
        brain_clock: true,
        age: -200,
        conversion_time: Some(200),
        ..Default::default()
    }));
    let mut climate = crate::weather::ClimateImage::capture(&state);
    let base = Replay {
        tick: 100,
        entities: [(1, record)].into(),
        sim_debt: active_clock(100),
        climate: Some(climate),
        ..Default::default()
    };
    let mut batches = Vec::new();
    for (lo, hi, clock) in [
        (
            101,
            300,
            Held {
                game_ticks: 100,
                held_since: Some(101),
            },
        ),
        (301, 301, Running { held_ticks: 200 }),
        (
            302,
            400,
            Held {
                game_ticks: 101,
                held_since: Some(302),
            },
        ),
        (401, 401, Running { held_ticks: 299 }),
        (
            402,
            450,
            Held {
                game_ticks: 102,
                held_since: Some(402),
            },
        ),
    ] {
        climate.game_clock = clock;
        batches.push((lo, hi, vec![JournalEntry::Climate { value: climate }]));
    }
    let replay = replay_batches(base, &batches);
    let memory = replay.entities[&1].villager.as_ref().unwrap();
    assert_eq!(memory.conversion_time, Some(198));
    assert_eq!(memory.age, -200);
}
