//! One immutable deadline across activity changes, freeze and native recovery.
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

fn shipment(f: &Fixture) -> &Shipment {
    &f.state
        .settlements
        .get(f.settlement)
        .unwrap()
        .economy
        .caravan
        .shipments["clock"]
}

fn admit(f: &mut Fixture) {
    stock(&mut f.state, SRC_CHEST, "minecraft:bread", 3);
    let mut request = open(f, "clock", 3, None);
    if let Request::Open(open) = &mut request {
        open.max_ticks = 10;
    }
    native(f, request);
}

#[test]
fn game_time_caravan_deadline_survives_freeze_activity_and_disk_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = disk_fixture(&config);
    admit(&mut f);
    let deadline = shipment(&f).game_deadline.unwrap();
    checkpoint(&mut f, &config);
    let mut writer =
        JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture()).unwrap();
    step(&mut f, &mut writer);
    assert!(f.state.world.is_active(0, 0));
    f.state.tickets.clear();
    step(&mut f, &mut writer);
    assert!(!f.state.world.is_active(0, 0));
    command(&mut f, "tick freeze");
    assert!(f.state.tick_control.frozen());
    let game = f.state.tick_control.game_ticks(f.state.tick);
    for _ in 0..30 {
        step(&mut f, &mut writer);
    }
    assert_eq!(f.state.tick_control.game_ticks(f.state.tick), game);
    assert_eq!(shipment(&f).status, Status::Loading);
    command(&mut f, "tick unfreeze");
    step(&mut f, &mut writer);
    assert_eq!(
        shipment(&f).status,
        Status::Loading,
        "freeze never spends the route budget"
    );
    checkpoint(&mut f, &config);
    let saved = shipment(&f).clone();
    let game = f.state.tick_control.game_ticks(f.state.tick);
    let (mut f, mut writer) = reopen(f, &config, writer);
    // The fixture executes exactly one real hydration tick after reopening.
    assert_eq!(f.state.tick_control.game_ticks(f.state.tick), game + 1);
    assert_eq!(
        shipment(&f),
        &saved,
        "reopen/activity cannot restart or spend a second deadline"
    );
    assert!(f.state.world.is_active(0, 0));
    while f.state.tick_control.game_ticks(f.state.tick) < deadline {
        step(&mut f, &mut writer);
        assert_eq!(shipment(&f).status, Status::Loading);
    }
    step(&mut f, &mut writer);
    assert_eq!(shipment(&f).status, Status::Blocked);
    assert_eq!(shipment(&f).game_deadline, Some(deadline));
    assert_eq!(chest_total(&f.state, SRC_CHEST, f.bread), 3);
    assert_eq!(carry_total(&f.state, f.carrier), 0);
    writer.flush().unwrap();
}

#[test]
fn game_time_caravan_old_store_converts_at_recovery_not_first_unfrozen_tick() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = disk_fixture(&config);
    command(&mut f, "tick freeze");
    let mut writer =
        JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture()).unwrap();
    for _ in 0..20 {
        step(&mut f, &mut writer);
    }
    admit(&mut f);
    let epoch = shipment(&f).epoch;
    let mut legacy = serde_json::to_value(
        &f.state
            .settlements
            .get(f.settlement)
            .unwrap()
            .economy
            .caravan,
    )
    .unwrap();
    legacy.as_object_mut().unwrap().remove("clock");
    legacy["shipments"]["clock"]
        .as_object_mut()
        .unwrap()
        .remove("game_deadline");
    f.state
        .settlements
        .get_mut(f.settlement)
        .unwrap()
        .economy
        .caravan = serde_json::from_value(legacy).unwrap();
    checkpoint(&mut f, &config);
    let (mut f, mut writer) = reopen(f, &config, writer);
    let converted = shipment(&f).game_deadline.unwrap();
    assert_eq!(converted, epoch + 11 - 20);
    assert_eq!(shipment(&f).epoch, epoch, "receipt identities never rebase");
    command(&mut f, "tick freeze");
    for _ in 0..4 {
        step(&mut f, &mut writer);
    }
    assert_eq!(shipment(&f).game_deadline, Some(converted));
    checkpoint(&mut f, &config);
    let (f, mut writer) = reopen(f, &config, writer);
    assert_eq!(
        shipment(&f).game_deadline,
        Some(converted),
        "conversion happens only once"
    );
    writer.flush().unwrap();
}

#[test]
fn game_time_caravan_conversion_refuses_overflow_and_mixed_units() {
    let mut f = fixture();
    admit(&mut f);
    let mut policy = f
        .state
        .settlements
        .get(f.settlement)
        .unwrap()
        .economy
        .caravan
        .clone();
    policy.clock = crate::settlements::commerce::Clock::WorldTicks;
    assert!(
        validate(&policy).is_err(),
        "legacy units cannot contain a game deadline"
    );
    policy.shipments.get_mut("clock").unwrap().game_deadline = None;
    policy.shipments.get_mut("clock").unwrap().epoch = u64::MAX;
    assert!(
        restore_clock(&mut policy, u64::MAX, 0)
            .unwrap_err()
            .contains("shipment clock")
    );
    policy.clock = crate::settlements::commerce::Clock::GameTicks;
    assert!(
        validate(&policy).is_err(),
        "game-time deadline is mandatory after migration"
    );
}

#[test]
fn game_time_caravan_pre_freeze_legacy_deadline_expires_on_first_tick() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = disk_fixture(&config);
    admit(&mut f);
    let mut writer =
        JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture()).unwrap();
    command(&mut f, "tick freeze");
    for _ in 0..20 {
        step(&mut f, &mut writer);
    }
    let game = f.state.tick_control.game_ticks(f.state.tick);
    let policy = &mut f
        .state
        .settlements
        .get_mut(f.settlement)
        .unwrap()
        .economy
        .caravan;
    policy.clock = crate::settlements::commerce::Clock::WorldTicks;
    policy.shipments.get_mut("clock").unwrap().game_deadline = None;
    checkpoint(&mut f, &config);
    let (f, mut writer) = reopen(f, &config, writer);
    assert_eq!(shipment(&f).game_deadline, Some(game));
    assert_eq!(shipment(&f).status, Status::Blocked);
    assert_eq!(chest_total(&f.state, SRC_CHEST, f.bread), 3);
    writer.flush().unwrap();
}

#[test]
fn game_time_caravan_expired_at_zero_blocks_before_pickup() {
    let mut f = fixture();
    admit(&mut f);
    teleport(&mut f.state, f.carrier, SRC_DOOR);
    f.state.tick = 20;
    f.state
        .tick_control
        .restore_clock(crate::weather::GameClockImage::Held { game_ticks: 0 }, 20)
        .unwrap();
    let mut policy = f
        .state
        .settlements
        .get(f.settlement)
        .unwrap()
        .economy
        .caravan
        .clone();
    policy.clock = crate::settlements::commerce::Clock::WorldTicks;
    policy.shipments.get_mut("clock").unwrap().game_deadline = None;
    restore_clock(&mut policy, 20, 0).unwrap();
    assert_eq!(policy.shipments["clock"].game_deadline, Some(0));
    assert!(tick(&mut f.state, f.settlement, &mut policy));
    assert_eq!(policy.shipments["clock"].status, Status::Blocked);
    assert!(!policy.shipments["clock"].loaded);
    assert_eq!(chest_total(&f.state, SRC_CHEST, f.bread), 3);
    assert_eq!(carry_total(&f.state, f.carrier), 0);
}

#[test]
fn game_time_legacy_caravan_wal_cuts_preserve_converted_deadline() {
    for save_checkpoint in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let config = CoreConfig::new(directory.path().to_path_buf(), 42);
        let mut f = disk_fixture(&config);
        admit(&mut f);
        command(&mut f, "tick freeze");
        let mut writer =
            JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture())
                .unwrap();
        for _ in 0..20 {
            step(&mut f, &mut writer);
        }
        let game = f.state.tick_control.game_ticks(f.state.tick);
        let policy = &mut f
            .state
            .settlements
            .get_mut(f.settlement)
            .unwrap()
            .economy
            .caravan;
        policy.clock = crate::settlements::commerce::Clock::WorldTicks;
        policy.shipments.get_mut("clock").unwrap().game_deadline = None;
        checkpoint(&mut f, &config);
        let mut f = recover(f, &config, writer);
        let converted = shipment(&f).clone();
        assert_eq!(converted.game_deadline, Some(game));
        // Lose the first converted batch before publication.
        command(&mut f, "tick freeze");
        let (output, _, _) = crate::tick::run_tick(&mut f.state, &[]);
        let unpublished = crate::core::journal_entries_for_tick(&mut f.state, &output).unwrap();
        assert!(!unpublished.is_empty());
        drop(unpublished);
        let writer =
            JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture())
                .unwrap();
        let mut f = recover(f, &config, writer);
        assert_eq!(shipment(&f), &converted);
        command(&mut f, "tick freeze");
        let mut writer =
            JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture())
                .unwrap();
        step(&mut f, &mut writer);
        if save_checkpoint {
            checkpoint(&mut f, &config);
        }
        let mut f = recover(f, &config, writer);
        assert_eq!(
            shipment(&f),
            &converted,
            "WAL conversion must not run again"
        );
        let mut writer =
            JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture())
                .unwrap();
        step(&mut f, &mut writer);
        assert_eq!(shipment(&f).status, Status::Blocked);
        assert_eq!(shipment(&f).game_deadline, Some(game));
        assert_eq!(chest_total(&f.state, SRC_CHEST, f.bread), 3);
        assert_eq!(carry_total(&f.state, f.carrier), 0);
        writer.flush().unwrap();
    }
}
