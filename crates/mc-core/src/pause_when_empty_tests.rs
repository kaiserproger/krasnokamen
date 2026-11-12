//! Empty-server pause (vanilla `pause-when-empty-seconds`): the world stops
//! advancing after the idle grace period, saves still work, and a join
//! resumes ticking.

use std::time::Duration;

use crate::boundary::{ConnectionId, PlayerInput, TickOutput};
use crate::config::CoreConfig;
use crate::core::Core;

pub(crate) fn config(name: &str, pause_seconds: u32) -> (CoreConfig, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("mc-core-pause-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut config = CoreConfig::new(dir.clone(), 42);
    config.view_distance = 2;
    config.simulation_distance = 0;
    config.pause_when_empty_seconds = pause_seconds;
    // Keep the test off the checkpoint cadence (explicit saves are tested).
    config.checkpoint_every_ticks = u64::MAX;
    (config, dir)
}

pub(crate) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("test runtime")
}

/// Wait for the tick owner's acknowledgement, then drain only pre-pause broadcasts.
pub(crate) fn wait_for_pause(
    core: &Core,
    rt: &tokio::runtime::Runtime,
    rx: &mut tokio::sync::broadcast::Receiver<TickOutput>,
) -> (u64, Option<TickOutput>) {
    let mut pause = core.subscribe_pause();
    let paused_at = rt.block_on(async {
        let observed =
            tokio::time::timeout(Duration::from_secs(2), pause.wait_for(Option::is_some))
                .await
                .expect("tick owner acknowledges pause")
                .expect("tick owner remains alive");
        observed.expect("acknowledged pause tick")
    });
    let mut last = None;
    loop {
        match rx.try_recv() {
            Ok(output) => {
                assert!(
                    output.tick <= paused_at,
                    "tick published after pause acknowledgement"
                );
                last = Some(output);
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                panic!("tick owner exited while paused")
            }
        }
    }
    (paused_at, last)
}

pub(crate) fn next_tick(
    rt: &tokio::runtime::Runtime,
    rx: &mut tokio::sync::broadcast::Receiver<TickOutput>,
    wait: Duration,
) -> Option<TickOutput> {
    rt.block_on(async { tokio::time::timeout(wait, rx.recv()).await.ok()?.ok() })
}

#[test]
fn empty_server_stops_ticking_and_a_join_resumes() {
    let (config, dir) = config("resume", 1);
    let core = Core::open(config).expect("open");
    let rt = runtime();
    let mut rx = core.subscribe();

    // One second of grace = 20 ticks, then the world pauses.
    let (paused_at, _) = wait_for_pause(&core, &rt, &mut rx);
    assert!(
        paused_at >= 19,
        "the world ticked through its grace period before pausing (tick {paused_at})"
    );
    // No further world ticks arrive while empty: the channel stays quiet.
    assert!(
        next_tick(&rt, &mut rx, Duration::from_millis(400)).is_none(),
        "a paused world must not publish ticks"
    );

    // Saves keep being serviced while paused (vanilla tickConnection path).
    assert!(core.checkpoint(), "checkpoint while paused");

    // A join wakes the world and continuous ticking resumes.
    core.submit(PlayerInput::Join {
        connection: ConnectionId(1),
        uuid: uuid::Uuid::new_v4(),
        name: "resumer".to_owned(),
        permission_level: 0,
    });
    let resumed =
        next_tick(&rt, &mut rx, Duration::from_secs(2)).expect("the join resumes ticking");
    assert!(resumed.tick > paused_at, "tick advances after the join");
    let mut after_join = resumed.tick;
    for _ in 0..5 {
        after_join = next_tick(&rt, &mut rx, Duration::from_secs(2))
            .expect("online player keeps world running")
            .tick;
    }
    assert!(
        after_join >= resumed.tick + 5,
        "a player online keeps the world ticking (tick {after_join})"
    );

    core.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn zero_seconds_never_pauses() {
    let (config, dir) = config("disabled", 0);
    let core = Core::open(config).expect("open");
    let rt = runtime();
    let mut rx = core.subscribe();
    let mut last = 0;
    for _ in 0..40 {
        last = next_tick(&rt, &mut rx, Duration::from_secs(2))
            .expect("empty world keeps running when pause is disabled")
            .tick;
    }
    assert!(
        last >= 25,
        "0 disables the pause: the world keeps ticking while empty (tick {last})"
    );
    core.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}
