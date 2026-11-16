use crate::boundary::PlayerInput;
use crate::{Core, CoreConfig};

#[test]
fn startup_freeze_holds_simulation_from_first_tick() {
    let root = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(root.path().join("world"), 42);
    let initial = Core::open(config.clone()).unwrap();
    let mut ticks = initial.subscribe();
    for _ in 0..3 {
        ticks.blocking_recv().unwrap();
    }
    initial.shutdown();
    drop(initial);
    let durable = crate::native_checkpoint::recover_offline(&config).unwrap();
    let durable_tick = durable.dimensions[&crate::dimensions::Dimension::Overworld].tick;
    drop(durable);

    config.start_frozen = true;
    let core = Core::open(config).unwrap();
    assert_eq!(core.inner.recovered_tick, durable_tick);
    let mut ticks = core.subscribe();
    let first = ticks.blocking_recv().unwrap();
    for _ in 0..3 {
        let next = ticks.blocking_recv().unwrap();
        assert!(next.tick > first.tick);
        assert_eq!(next.snapshot.game_time, first.snapshot.game_time);
    }

    let (command, completed) = crate::command_console::ConsoleCommand::new("tick unfreeze".into());
    assert!(core.submit(PlayerInput::ConsoleCommand(command)));
    completed.blocking_recv().unwrap();
    let mut resumed = false;
    for _ in 0..10 {
        if ticks.blocking_recv().unwrap().snapshot.game_time > first.snapshot.game_time {
            resumed = true;
            break;
        }
    }
    assert!(resumed, "game time did not resume after unfreeze");
    core.shutdown();
}
