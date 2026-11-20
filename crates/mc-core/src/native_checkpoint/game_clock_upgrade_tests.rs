use super::*;

fn villager(value: u64) -> crate::villager::VillagerState {
    crate::villager::VillagerState {
        brain_clock: true,
        activity: crate::villager::Activity::Work,
        last_work: value as i64,
        sleeping: true,
        last_slept: Some(value),
        panic_until: 999,
        ..Default::default()
    }
}

#[test]
fn conversion_zero_game_one_tick_skips_all_memories() {
    let mut state = villager(0);
    state.last_slept = None;
    let mut report = Conversion {
        tick: 1,
        ..Default::default()
    };
    normalize_villager(&mut state, 0, &mut report).unwrap();
    assert_eq!(state.last_work, 0);
    assert_eq!(state.last_slept, None);
    assert_eq!(state.panic_until, 999);
    assert_eq!((report.work_normalized, report.sleep_normalized), (0, 0));
}

#[test]
fn conversion_memory_inclusive_boundaries_and_future_refusal() {
    for (input, expected, normalized) in [(8, 8, 0), (9, 18, 1), (18, 18, 1), (19, 18, 1)] {
        let mut state = villager(input);
        let mut report = Conversion {
            tick: 20,
            ..Default::default()
        };
        normalize_villager(&mut state, 10, &mut report).unwrap();
        assert_eq!(state.last_work, expected as i64);
        assert_eq!(state.last_slept, Some(expected));
        assert_eq!(state.panic_until, 999);
        assert_eq!(
            (report.work_normalized, report.sleep_normalized),
            (normalized, normalized)
        );
        // Exactly the live equality law on the first running tick after c'.
        crate::villager::persistence::advance_running_clocks(&mut state, 19, 21, 21);
        assert_eq!(state.last_work, if normalized == 1 { 20 } else { 8 });
        assert_eq!(state.last_slept, Some(if normalized == 1 { 20 } else { 8 }));
    }
    let mut report = Conversion {
        tick: 20,
        ..Default::default()
    };
    assert!(normalize_villager(&mut villager(20), 10, &mut report).is_err());
    let mut sleep_only = villager(20);
    sleep_only.activity = crate::villager::Activity::Idle;
    assert!(normalize_villager(&mut sleep_only, 10, &mut report).is_err());
}

#[test]
fn conversion_normalizes_only_the_live_law_gate() {
    for gate in 0..4 {
        let mut state = villager(19);
        match gate {
            0 => state.brain_clock = false,
            1 => state.wandering = true,
            2 => state.conversion_time = Some(73),
            _ => {
                state.activity = crate::villager::Activity::Idle;
                state.sleeping = false;
            }
        }
        let mut report = Conversion {
            tick: 20,
            ..Default::default()
        };
        normalize_villager(&mut state, 10, &mut report).unwrap();
        assert_eq!(state.last_work, 19);
        assert_eq!(state.last_slept, Some(19));
        assert_eq!(state.panic_until, 999);
        assert_eq!(
            state.conversion_time,
            if gate == 2 { Some(73) } else { None }
        );
        assert_eq!((report.work_normalized, report.sleep_normalized), (0, 0));
    }
}
