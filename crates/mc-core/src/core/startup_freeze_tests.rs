use crate::{Core, CoreConfig};

#[test]
fn startup_freeze_holds_simulation_from_first_tick() {
    let root = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(root.path().join("world"), 42);
    config.start_frozen = true;
    let core = Core::open(config).unwrap();
    let mut ticks = core.subscribe();
    let first = ticks.blocking_recv().unwrap();
    for _ in 0..3 {
        let next = ticks.blocking_recv().unwrap();
        assert!(next.tick > first.tick);
        assert_eq!(next.snapshot.game_time, first.snapshot.game_time);
    }
    core.shutdown();
}
