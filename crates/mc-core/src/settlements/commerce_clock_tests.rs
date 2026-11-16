//! Game-time payroll on actual terrain, native checkpoints and published WAL.
use super::*;

fn command(f: &mut Fixture, line: &str) {
    crate::command_console::execute(
        &mut f.state,
        crate::command_console::CONNECTION,
        line,
        4,
        &mut crate::boundary::ChangeSet::default(),
    );
}

fn contract(f: &Fixture) -> &Employment {
    &f.state
        .settlements
        .get(f.settlement)
        .unwrap()
        .economy
        .commerce
        .employments["1"]
}

#[test]
fn game_time_wages_share_watermark_across_freeze_activity_and_disk_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = disk_fixture(&config);
    for id in [f.worker, f.buyer] {
        let slot = f.state.entities.slot_of(id).unwrap();
        f.state.entities.row_mut(slot).controls.no_ai = true;
    }
    stock(&mut f.state, TREASURY_CHEST, f.emerald, 40);
    let request = employ(&f, "1", 2, 5, 40);
    native(&mut f, request);
    let first_due = contract(&f).next_due;
    checkpoint(&mut f, &config);
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    through(&mut f, &mut writer, first_due);
    assert!(f.state.world.is_active(0, 0));
    assert_eq!((contract(&f).earned, contract(&f).paid), (2, 2));

    f.state.tickets.clear();
    through(&mut f, &mut writer, first_due + 5);
    assert!(!f.state.world.is_active(0, 0));
    assert_eq!((contract(&f).earned, contract(&f).paid), (4, 4));
    command(&mut f, "tick freeze");
    assert!(f.state.tick_control.frozen());
    let game = f.state.tick_control.game_ticks(f.state.tick);
    for _ in 0..30 {
        step(&mut f, &mut writer);
    }
    assert_eq!(f.state.tick_control.game_ticks(f.state.tick), game);
    assert_eq!((contract(&f).earned, contract(&f).paid), (4, 4));
    command(&mut f, "tick unfreeze");
    step(&mut f, &mut writer);
    assert_eq!(
        (contract(&f).earned, contract(&f).paid),
        (4, 4),
        "frozen periods never accrue on resume"
    );
    checkpoint(&mut f, &config);
    let saved = contract(&f).clone();
    let game = f.state.tick_control.game_ticks(f.state.tick);
    let mut f = reopen(f, &config, writer);
    assert_eq!(
        f.state.tick_control.game_ticks(f.state.tick),
        game,
        "reopen invents no downtime"
    );
    assert_eq!(contract(&f), &saved);
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    step(&mut f, &mut writer);
    assert!(f.state.world.is_active(0, 0));
    assert_eq!(
        contract(&f).earned,
        4,
        "reconciliation cannot reconsume a wage period"
    );
    while f.state.tick_control.game_ticks(f.state.tick) <= first_due + 10 {
        step(&mut f, &mut writer);
    }
    assert_eq!((contract(&f).earned, contract(&f).paid), (6, 6));
    assert_eq!(contract(&f).next_due, first_due + 15);
    assert_eq!(carry_total(&f.state, f.worker), 6);
    assert_eq!(emerald_total(&f.state, TREASURY_CHEST), 34);
    writer.flush().unwrap();
}

#[test]
fn game_time_wage_old_store_converts_before_start_frozen_ticks_and_only_once() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = disk_fixture(&config);
    command(&mut f, "tick freeze");
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    for _ in 0..20 {
        step(&mut f, &mut writer);
    }
    let request = employ(&f, "1", 2, 5, 40);
    native(&mut f, request);
    let game_due = contract(&f).next_due;
    let mut legacy = serde_json::to_value(
        &f.state
            .settlements
            .get(f.settlement)
            .unwrap()
            .economy
            .commerce,
    )
    .unwrap();
    legacy.as_object_mut().unwrap().remove("clock");
    legacy["employments"]["1"]["next_due"] = (game_due + 20).into();
    f.state
        .settlements
        .get_mut(f.settlement)
        .unwrap()
        .economy
        .commerce = serde_json::from_value(legacy).unwrap();
    checkpoint(&mut f, &config);
    let mut f = reopen(f, &config, writer);
    assert_eq!(
        contract(&f).next_due,
        game_due,
        "recovery uses the restored held offset"
    );
    assert_eq!(
        f.state
            .settlements
            .get(f.settlement)
            .unwrap()
            .economy
            .commerce
            .clock,
        Clock::GameTicks
    );
    command(&mut f, "tick freeze");
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    for _ in 0..7 {
        step(&mut f, &mut writer);
    }
    assert_eq!(
        contract(&f).next_due,
        game_due,
        "start-frozen ticks cannot alter conversion"
    );
    assert_eq!(contract(&f).earned, 0);
    checkpoint(&mut f, &config);
    let f = reopen(f, &config, writer);
    assert_eq!(
        contract(&f).next_due,
        game_due,
        "the next recovery never converts again"
    );
    assert_eq!(contract(&f).earned, 0);
}

#[test]
fn game_time_wage_conversion_refuses_inconsistent_future_clock() {
    let mut f = fixture();
    let mut policy = State::default();
    hire(&mut f, &mut policy, "1", 2, 40);
    policy.clock = Clock::WorldTicks;
    policy.employments.get_mut("1").unwrap().next_due = 100;
    let before = policy.clone();
    assert!(
        restore_clock(&mut policy, 0, 0)
            .unwrap_err()
            .contains("employment 1")
    );
    assert_eq!(policy, before);
    let mut invalid = serde_json::to_value(policy).unwrap();
    invalid["clock"] = "wall_time".into();
    assert!(serde_json::from_value::<State>(invalid).is_err());
}

#[test]
fn game_time_explicit_pay_stop_and_hire_use_the_same_clock() {
    let mut f = fixture();
    stock(&mut f.state, TREASURY_CHEST, f.emerald, 20);
    let mut policy = State::default();
    hire(&mut f, &mut policy, "1", 2, 20);
    f.state.tick_control.set_frozen(true);
    for _ in 0..20 {
        f.state.tick += 1;
        f.state.tick_control.begin_tick();
    }
    let request = Request::Pay {
        contract: "1".into(),
        treasury_fence: fences(&f, &f.treasury),
        worker_fence: fences(&f, &carry(f.worker_uuid)),
        op: pay_op("1", 1),
    };
    assert_eq!(
        fail(&mut f.state, f.settlement, &mut policy, request),
        Failure::InvalidRequest
    );
    assert_eq!(policy.employments["1"].earned, 0);
    run(
        &mut f.state,
        f.settlement,
        &mut policy,
        Request::StopEmployment {
            contract: "1".into(),
        },
    );
    assert_eq!(
        policy.employments["1"].earned, 0,
        "stop cannot accrue frozen periods"
    );
    hire(&mut f, &mut policy, "2", 2, 20);
    assert_eq!(
        policy.employments["2"].next_due, 5,
        "admission uses game time too"
    );
    assert_eq!(emerald_total(&f.state, TREASURY_CHEST), 20);
}

#[test]
fn game_time_wage_pre_freeze_legacy_debt_is_not_forgiven_or_counted_twice() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = disk_fixture(&config);
    stock(&mut f.state, TREASURY_CHEST, f.emerald, 40);
    let request = employ(&f, "1", 2, 100, 40);
    native(&mut f, request);
    let first_due = contract(&f).next_due;
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    through(&mut f, &mut writer, first_due - 51);
    command(&mut f, "tick freeze");
    for _ in 0..1000 {
        step(&mut f, &mut writer);
    }
    let game = f.state.tick_control.game_ticks(f.state.tick);
    let now = f.state.tick;
    let periods = (now - first_due) / 100 + 1;
    let next_due = game + 100 - (now - first_due) % 100;
    f.state
        .settlements
        .get_mut(f.settlement)
        .unwrap()
        .economy
        .commerce
        .clock = Clock::WorldTicks;
    checkpoint(&mut f, &config);
    let mut f = reopen(f, &config, writer);
    assert_eq!(contract(&f).earned, 0, "recovery does not accrue or pay");
    assert_eq!(contract(&f).legacy_owed, periods * 2);
    assert_eq!(contract(&f).next_due, next_due);
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    command(&mut f, "tick freeze");
    step(&mut f, &mut writer);
    assert_eq!(contract(&f).legacy_owed, periods * 2);
    checkpoint(&mut f, &config);
    let mut f = reopen(f, &config, writer);
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    step(&mut f, &mut writer);
    assert_eq!(contract(&f).legacy_owed, 0);
    assert_eq!(contract(&f).earned, periods * 2);
    assert_eq!(contract(&f).paid, periods * 2);
    checkpoint(&mut f, &config);
    let mut f = reopen(f, &config, writer);
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    step(&mut f, &mut writer);
    assert_eq!(contract(&f).earned, periods * 2);
    assert_eq!(contract(&f).paid, periods * 2);
    writer.flush().unwrap();
}
