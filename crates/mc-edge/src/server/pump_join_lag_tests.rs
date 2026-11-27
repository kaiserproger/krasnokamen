//! A pump slower than the tick must not hold reliable changes behind stale
//! outputs. Candidate mechanism for Perf1M R3k-4 leg c (four joins timed out
//! while the tick kept running); that run's cause stays open, these are
//! synthetic repros with the real core and the real pump.
use super::*;

fn server(
    root: &std::path::Path,
    broadcast_cap: usize,
    outbox_stall_ms: Option<u64>,
) -> EdgeServer {
    let mut config = mc_core::CoreConfig::new(root.join("world"), 42);
    config.vanilla_data_dir =
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"));
    config.view_distance = 0;
    config.simulation_distance = 0;
    config.broadcast_cap = broadcast_cap;
    if let Some(stall) = outbox_stall_ms {
        config.outbox_stall_ms = stall;
    }
    let core = Arc::new(mc_core::Core::open(config).unwrap());
    let mut cfg = crate::config::EdgeConfig {
        server_dir: root.join("server"),
        ..Default::default()
    };
    std::fs::create_dir_all(&cfg.server_dir).unwrap();
    cfg.data.world_dir = root.join("world");
    cfg.data.vanilla_data_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    cfg.auth.online_mode = false;
    let data = crate::server::load_vanilla_data(&cfg).unwrap();
    let tags = crate::server::load_tags(&cfg, &data).unwrap();
    EdgeServer::new(core, data, tags, cfg).unwrap()
}

/// Bound on one whole scenario, from after the core opened: every wait in it
/// shares this deadline, so a stuck pump or a broken watchdog fails the test
/// instead of hanging it. None is a pass condition.
const SCENARIO: std::time::Duration = std::time::Duration::from_secs(240);

fn scenario_deadline() -> std::time::Instant {
    std::time::Instant::now() + SCENARIO
}

#[derive(Default)]
struct Gate {
    /// Outputs the pump may still process.
    permits: u64,
    /// The pump waits in the gate for a permit.
    parked: bool,
    /// Gate entries so far (one per processed output).
    entries: u64,
}

/// Pump permits: the armed gate lets the real pump process one received
/// output per permit and holds it (after it recorded the output's tick,
/// before it drains or publishes) while none is left. Dropping it frees the
/// pump for good, also when an assertion fails first.
struct Pacer {
    state: Arc<(std::sync::Mutex<Gate>, std::sync::Condvar)>,
    deadline: std::time::Instant,
}

impl Pacer {
    fn arm(server: &EdgeServer, permits: u64, deadline: std::time::Instant) -> Self {
        let state = Arc::new((
            std::sync::Mutex::new(Gate {
                permits,
                ..Gate::default()
            }),
            std::sync::Condvar::new(),
        ));
        let gate = Arc::clone(&state);
        crate::server::arm_pump_gate(
            server,
            Arc::new(move || {
                let (lock, wake) = &*gate;
                let mut gate = lock.lock().unwrap();
                gate.entries += 1;
                gate.parked = true;
                wake.notify_all();
                while gate.permits == 0 {
                    gate = wake.wait(gate).unwrap();
                }
                gate.permits -= 1;
                gate.parked = false;
            }),
        );
        Self { state, deadline }
    }

    fn set_permits(&self, permits: u64) -> u64 {
        let (lock, wake) = &*self.state;
        let mut gate = lock.lock().unwrap();
        gate.permits = permits;
        wake.notify_all();
        gate.entries
    }

    fn grant(&self) -> u64 {
        let (lock, wake) = &*self.state;
        let mut gate = lock.lock().unwrap();
        gate.permits += 1;
        wake.notify_all();
        gate.entries
    }

    /// Wait (within the scenario deadline) until the pump holds an output it
    /// entered the gate with after `entries` earlier entries; that output's
    /// tick.
    fn held_after(&self, server: &EdgeServer, entries: u64) -> u64 {
        let (lock, wake) = &*self.state;
        let gate = lock.lock().unwrap();
        let left = self
            .deadline
            .saturating_duration_since(std::time::Instant::now());
        let (gate, timeout) = wake
            .wait_timeout_while(gate, left, |gate| !(gate.parked && gate.entries > entries))
            .unwrap();
        assert!(!timeout.timed_out(), "the pump never reached its gate");
        drop(gate);
        server.tick_stats.tick()
    }

    /// Stop the pump at its next output; that output's tick.
    fn hold(&self, server: &EdgeServer) -> u64 {
        let entries = self.set_permits(0);
        self.held_after(server, entries)
    }
}

impl Drop for Pacer {
    fn drop(&mut self) {
        let (lock, wake) = &*self.state;
        lock.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .permits = u64::MAX;
        wake.notify_all();
    }
}

/// The next published output, or `None` once the core closed the broadcast;
/// an observer that fell behind skips ahead.
async fn next_output(
    observer: &mut broadcast::Receiver<TickOutput>,
    deadline: std::time::Instant,
) -> Option<TickOutput> {
    loop {
        match tokio::time::timeout_at(deadline.into(), observer.recv())
            .await
            .expect("scenario deadline")
        {
            Ok(out) => return Some(out),
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

/// Observe published outputs until `newest` reaches `tick`.
async fn until_tick(
    observer: &mut broadcast::Receiver<TickOutput>,
    tick: u64,
    deadline: std::time::Instant,
) {
    while next_output(observer, deadline)
        .await
        .expect("core stopped")
        .tick
        < tick
    {}
}

/// Run a console line through the tick and return its output tick.
async fn console(core: &mc_core::Core, line: &str, deadline: std::time::Instant) -> u64 {
    let (command, completed) = mc_core::command_console::ConsoleCommand::new(line.to_owned());
    assert!(core.submit(mc_core::boundary::PlayerInput::ConsoleCommand(command)));
    tokio::time::timeout_at(deadline.into(), completed)
        .await
        .expect("scenario deadline")
        .unwrap()
}

fn submit_join(core: &mc_core::Core, id: ConnectionId, name: &str) {
    assert!(core.submit(mc_core::boundary::PlayerInput::Join {
        connection: id,
        uuid: crate::crypto::offline_uuid(name),
        name: name.to_owned(),
        permission_level: 0,
    }));
}

/// When a joining connection's `Joined` reached the edge, measured from the
/// tick that joined the player.
#[derive(Debug)]
struct JoinLag {
    /// Ticks the core had published past the join tick.
    ticks: u64,
    join_tick: u64,
    /// The output the pump had published when the `Joined` was seen.
    pump_tick: u64,
}

/// The pump is held while the core publishes `hold_ticks` more outputs, a
/// connection then submits its Join, and the pump is released at half the
/// tick rate (one output per two published ticks). `None` when the `Joined`
/// did not reach the edge within `bound_ticks` of its join tick.
async fn join_after_held_publish(hold_ticks: u64, bound_ticks: u64) -> Option<JoinLag> {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(server(dir.path(), 256, None));
    let deadline = scenario_deadline();
    let core = Arc::clone(&server.hub);
    let mut observer = core.subscribe();
    let pacer = Pacer::arm(&server, 0, deadline);
    let pump = crate::server::spawn_pump(Arc::clone(&server)).unwrap();
    let held = pacer.held_after(&server, 0);
    until_tick(&mut observer, held + hold_ticks, deadline).await;
    let id = server.connections.joining_id();
    submit_join(&core, id, "late");
    let mut joined = None;
    let lag = loop {
        let out = next_output(&mut observer, deadline)
            .await
            .expect("core stopped");
        if joined.is_none()
            && out
                .snapshot
                .player_roster
                .iter()
                .any(|player| player.connection == id)
        {
            joined = Some(out.tick);
        }
        if server.connections.take_join(id).is_some() {
            let join_tick = joined.expect("Joined reached the edge before its tick");
            break Some(JoinLag {
                ticks: out.tick - join_tick,
                join_tick,
                pump_tick: server.tick_stats.tick(),
            });
        }
        if joined.is_some_and(|join_tick| out.tick > join_tick + bound_ticks) {
            break None;
        }
        if out.tick % 2 == 0 {
            pacer.grant();
        }
    };
    drop(pacer);
    pump.abort();
    core.shutdown();
    lag
}

/// The pump is held while the core publishes 64 outputs (3.2 s at 20 TPS)
/// in a 256-output window, then runs at half the tick rate. Draining only
/// through each published tick in order, it reaches the join tick after
/// about 128 more ticks; skipping outputs older than the queue budget, it
/// stays about 32 ticks (1.6 s) behind the newest output.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn a_slow_pump_does_not_hold_a_join_behind_stale_outputs() {
    let lag = join_after_held_publish(64, 200).await;
    eprintln!("20 TPS, window 256, held 64: {lag:?}");
    let lag = lag.expect("Joined never reached the edge");
    assert!(
        lag.ticks < 48,
        "Joined waited {} ticks behind stale outputs after join tick {} (pump at {})",
        lag.ticks,
        lag.join_tick,
        lag.pump_tick
    );
}

/// Default 32-output window, `/tick rate 3` and the default 10 s outbox
/// stall watch: the pump is held past the window, the connection joins
/// through the real `wait_for_join` (10 s deadline) and the pump runs at half
/// the tick rate. The join lands and the core never halts.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn a_join_lands_at_three_tps_behind_a_slow_pump_without_a_halt() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(server(dir.path(), 32, None));
    let deadline = scenario_deadline();
    let core = Arc::clone(&server.hub);
    let mut observer = core.subscribe();
    let pacer = Pacer::arm(&server, u64::MAX / 2, deadline);
    let pump = crate::server::spawn_pump(Arc::clone(&server)).unwrap();
    let rate_tick = console(&core, "tick rate 3", deadline).await;
    // The pump is past the command's tick before the hold: the held outbox
    // is empty.
    while server.tick_stats.tick() <= rate_tick {
        next_output(&mut observer, deadline)
            .await
            .expect("core stopped");
    }
    let held = pacer.hold(&server);
    until_tick(&mut observer, held + 34, deadline).await;
    let id = server.connections.joining_id();
    let session = core
        .join_session(
            id,
            crate::crypto::offline_uuid("late"),
            "late".to_owned(),
            0,
            mc_core::boundary::ClientSettings {
                view_distance: 2,
                locale: "en_us".to_owned(),
                chat_visibility: mc_domain::ChatVisibility::Full,
            },
        )
        .unwrap();
    let waiting = {
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            wait_for_join(
                &server,
                id,
                false,
                2,
                "en_us",
                mc_domain::ChatVisibility::Full,
            )
            .await
        })
    };
    while !waiting.is_finished() {
        let Some(out) = next_output(&mut observer, deadline).await else {
            break;
        };
        if out.tick % 2 == 0 {
            pacer.grant();
        }
    }
    let joined = waiting.await.unwrap();
    let halted = core.halt_reason();
    drop(session);
    drop(pacer);
    pump.abort();
    core.shutdown();
    assert_eq!(halted, None, "the outbox stall watch halted a live pump");
    joined.expect("join deadline");
}

/// Fail closed stays: a pump that stops for good (held in its gate) with a
/// reliable batch queued still halts the core once the stall watch expires.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn a_hung_pump_still_halts_the_core() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(server(dir.path(), 32, Some(1_000)));
    let deadline = scenario_deadline();
    let core = Arc::clone(&server.hub);
    let mut observer = core.subscribe();
    let pacer = Pacer::arm(&server, 0, deadline);
    let pump = crate::server::spawn_pump(Arc::clone(&server)).unwrap();
    pacer.held_after(&server, 0);
    submit_join(&core, server.connections.joining_id(), "stuck");
    // A broken watchdog fails at the scenario deadline instead of hanging.
    while next_output(&mut observer, deadline).await.is_some() {}
    let reason = core.halt_reason();
    drop(pacer);
    pump.abort();
    core.shutdown();
    let reason = reason.expect("a hung pump halts the core");
    assert!(reason.contains("edge pump not draining"), "{reason}");
}

/// Live lag is not a hang: at `/tick rate 3` with the default 32-output
/// window and the default 10 s outbox stall watch, a pump held past the
/// window and then running at half the tick rate drains a lone queued batch
/// (a `Joined`) long before the watch expires, and the core keeps ticking
/// for 11 s after it. Draining only through each published tick in order,
/// the batch waited 31 tick periods (10.3 s) and the watch halted the core.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn a_live_pump_behind_a_lone_batch_does_not_trip_the_stall_watch() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(server(dir.path(), 32, None));
    let deadline = scenario_deadline();
    let core = Arc::clone(&server.hub);
    let mut observer = core.subscribe();
    let pacer = Pacer::arm(&server, u64::MAX / 2, deadline);
    let pump = crate::server::spawn_pump(Arc::clone(&server)).unwrap();
    let rate_tick = console(&core, "tick rate 3", deadline).await;
    while server.tick_stats.tick() <= rate_tick {
        next_output(&mut observer, deadline)
            .await
            .expect("core stopped");
    }
    let held = pacer.hold(&server);
    assert_eq!(
        core.outbox_queued_bytes(),
        0,
        "the held outbox starts empty"
    );
    until_tick(&mut observer, held + 34, deadline).await;
    let id = server.connections.joining_id();
    submit_join(&core, id, "lone");
    let mut joined = None;
    let mut drained = None;
    loop {
        let Some(out) = next_output(&mut observer, deadline).await else {
            break;
        };
        if joined.is_none()
            && out
                .snapshot
                .player_roster
                .iter()
                .any(|player| player.connection == id)
        {
            assert!(core.outbox_queued_bytes() > 0, "the Joined batch is queued");
            joined = Some(out.tick);
        }
        if drained.is_none() && server.connections.take_join(id).is_some() {
            drained = Some(out.tick);
        }
        // 33 ticks are 11 s at 3 TPS: past the stall watch.
        if joined.is_some_and(|tick| out.tick > tick + 33) {
            break;
        }
        if out.tick % 2 == 0 {
            pacer.grant();
        }
    }
    let halted = core.halt_reason();
    drop(pacer);
    pump.abort();
    core.shutdown();
    assert_eq!(halted, None, "the outbox stall watch halted a live pump");
    let joined = joined.expect("the join never ran");
    let drained = drained.expect("the lone batch never drained");
    assert!(
        drained < joined + 30,
        "the lone batch waited {} ticks",
        drained - joined
    );
}

/// Reliable frames of one connection, decoded.
fn frames(rx: &mut mpsc::Receiver<bytes::Bytes>) -> Vec<mc_protocol::frame::RawFrame> {
    let mut frames = Vec::new();
    while let Ok(data) = rx.try_recv() {
        let mut bytes = bytes::BytesMut::from(data.as_ref());
        while let Some(frame) = mc_protocol::frame::try_decode_frame(
            &mut bytes,
            mc_protocol::frame::Compression::Threshold(256),
        )
        .unwrap()
        {
            frames.push(frame);
        }
    }
    frames
}

/// What one reliable frame of the FIFO test carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Chat(u64),
    Teleport,
}

fn mark(frame: &mc_protocol::frame::RawFrame) -> Option<Mark> {
    use mc_protocol::packets::Packet as _;
    use mc_protocol::packets::play::{ClientboundSystemChat, SynchronizePlayerPosition};
    if frame.id == ClientboundSystemChat::ID {
        let body = &frame.body;
        let at = body.windows(5).position(|window| window == b"fifo-")?;
        let digits: String = body[at + 5..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .map(|&byte| char::from(byte))
            .collect();
        return digits.parse().ok().map(Mark::Chat);
    }
    if frame.id == SynchronizePlayerPosition::ID
        && SynchronizePlayerPosition::decode(&mut &frame.body[..])
            .ok()?
            .y
            == 120.0
    {
        return Some(Mark::Teleport);
    }
    None
}

/// Reliable changes queued while the pump is held arrive across a stale skip
/// in tick order, once each, and never ahead of the output the pump drains
/// through: at every hold, exactly the changes of ticks the pump already
/// published have arrived. A teleport rides the same order.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn a_stale_skip_keeps_reliable_changes_in_order_and_never_drains_ahead() {
    let dir = tempfile::tempdir().unwrap();
    let server = Arc::new(server(dir.path(), 256, None));
    let deadline = scenario_deadline();
    let core = Arc::clone(&server.hub);
    let mut observer = core.subscribe();
    let id = server.connections.joining_id();
    let (reliable_tx, mut reliable_rx) = mpsc::channel(4096);
    let (world_tx, _world_rx) = world_queue(4096, WORLD_QUEUE_BYTES);
    server
        .connections
        .register(
            id,
            ConnectionHandles::new(reliable_tx, world_tx, crate::plan::CHUNKS_PER_TICK),
            || server.hub.published_knockback_count(id),
        )
        .unwrap();
    let pacer = Pacer::arm(&server, u64::MAX / 2, deadline);
    let pump = crate::server::spawn_pump(Arc::clone(&server)).unwrap();
    submit_join(&core, id, "fifo");
    let joined = loop {
        let out = next_output(&mut observer, deadline)
            .await
            .expect("core stopped");
        if out
            .snapshot
            .player_roster
            .iter()
            .any(|player| player.connection == id)
        {
            break out.tick;
        }
    };
    while server.tick_stats.tick() <= joined + 2 {
        next_output(&mut observer, deadline)
            .await
            .expect("core stopped");
    }
    // The pump holds output `holding`: it drained and published everything
    // before it, and drains through `holding` only once released.
    let mut holding = pacer.hold(&server);
    frames(&mut reliable_rx);
    let mut sent = Vec::new();
    for n in 0..10u64 {
        if n == 5 {
            let tick = console(&core, "tp fifo 0.5 120 0.5", deadline).await;
            sent.push((tick, Mark::Teleport));
        }
        let line = format!("tellraw fifo \"fifo-{n}\"");
        sent.push((console(&core, &line, deadline).await, Mark::Chat(n)));
    }
    let last = sent.last().unwrap().0;
    until_tick(&mut observer, holding + 48, deadline).await;
    let skipped_before = server.dashboard.snapshot().pump_skipped_ticks;
    let mut arrived = Vec::new();
    loop {
        let entries = pacer.grant();
        let held = pacer.held_after(&server, entries);
        arrived.extend(frames(&mut reliable_rx).iter().filter_map(mark));
        let expected: Vec<Mark> = sent
            .iter()
            .filter(|(tick, _)| *tick <= holding)
            .map(|(_, mark)| *mark)
            .collect();
        assert_eq!(
            arrived, expected,
            "drained through {holding}, now holds {held}: {sent:?}"
        );
        if holding >= last {
            break;
        }
        holding = held;
    }
    let skipped = server.dashboard.snapshot().pump_skipped_ticks - skipped_before;
    drop(pacer);
    pump.abort();
    core.shutdown();
    assert!(skipped > 0, "the release never skipped a stale output");
}

fn output_published(at: Option<std::time::Instant>) -> TickOutput {
    let mut out = empty_output();
    out.times.published_at = at;
    out
}

#[test]
fn a_fresh_output_after_a_long_tick_is_not_stale() {
    let now = std::time::Instant::now();
    let mut out = output_published(Some(now));
    // The tick itself ran for a minute: only the publish instant counts.
    out.times.wall_start_unix_ns = 1;
    out.times.wall_us = 60_000_000;
    // A timely output never asks for a newer one (the broadcast tail lock).
    assert!(!stale(&out, now, || unreachable!(
        "tail lock on a fresh output"
    )));
}

#[test]
fn the_queue_budget_is_exclusive() {
    let now = std::time::Instant::now();
    let at_budget = now.checked_sub(PUBLISH_QUEUE_BUDGET).unwrap();
    assert!(!stale(&output_published(Some(at_budget)), now, || true));
    let past = at_budget
        .checked_sub(std::time::Duration::from_micros(1))
        .unwrap();
    assert!(stale(&output_published(Some(past)), now, || true));
}

#[test]
fn the_newest_output_is_never_stale() {
    let now = std::time::Instant::now();
    let old = now.checked_sub(std::time::Duration::from_secs(60)).unwrap();
    assert!(!stale(&output_published(Some(old)), now, || false));
    assert!(!stale(&output_published(None), now, || true));
}
