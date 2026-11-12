//! A plugin whose callbacks stall must refuse its new commands, not halt the world.
//!
//! One host thread delivers every durable event, so a slow `on_events` callback
//! leaves the committed-event FIFO retained in the core. These tests drive the
//! same path the tick owner does (`run_tick`, then `seal_events`, then the WAL
//! `acknowledge_events`) with a host endpoint that is never read: the stalled
//! callback.

use std::task::{Context, Poll, Waker};

use crate::boundary::{ConnectionId, PlayerChange, PlayerInput};
use crate::plugin_tests::{fixture, join_native_player};
use crate::tick::TickState;

const COMMANDS_PER_TICK: usize = 64;
const TICKS: u64 = 64;

fn stalled_plugin_fixture() -> (TickState, mc_script::ScriptHostEndpoint) {
    let mut state = fixture();
    join_native_player(&mut state);
    // A small host mailbox that nobody reads fills at once: the retained FIFO
    // in the core then grows by every durable entry the tick seals.
    let capacity = std::num::NonZeroUsize::new(4).unwrap();
    let (boundary, endpoint) = mc_script::script_boundary_pair(capacity, capacity);
    let manifest = mc_script::ScriptPluginManifest::new(
        "owner",
        "Owner",
        "0.1.0",
        mc_script::COMPONENT_PLUGIN_API_VERSION,
    )
    .declare_player_command_root("owned")
    .validate()
    .unwrap();
    endpoint.register_plugin_routes(&manifest).unwrap();
    state.plugin_runtime = Some(crate::plugin::PluginRuntime::with_test_boundary(boundary));
    state.tick = 70;
    crate::plugin::start_events(&mut state).unwrap();
    (state, endpoint)
}

/// What players were sent in one tick.
type Deliveries = Vec<(ConnectionId, Vec<PlayerChange>)>;

/// One native tick with `inputs`, sealed and acknowledged the way the tick loop
/// does. Returns the seal verdict and what the players were sent.
pub(crate) fn tick_with_inputs(
    state: &mut TickState,
    inputs: &[PlayerInput],
) -> (Result<bool, &'static str>, Deliveries) {
    let (output, deliveries, _) = crate::tick::run_tick(state, inputs);
    let sealed = crate::plugin::seal_events(state, output.tick);
    if sealed.is_ok() {
        crate::plugin::acknowledge_events(state, output.tick);
    }
    (sealed, deliveries)
}

/// One tick with `commands` plugin commands from the player. Returns the seal
/// verdict and how many commands the player was told to retry.
fn tick_with_commands(
    state: &mut TickState,
    commands: usize,
) -> (Result<bool, &'static str>, usize) {
    let inputs: Vec<_> = (0..commands)
        .map(|n| PlayerInput::Command {
            connection: ConnectionId(1),
            command: format!("owned job {n}"),
            permission_level: 0,
        })
        .collect();
    let (sealed, deliveries) = tick_with_inputs(state, &inputs);
    let refused = deliveries
        .iter()
        .flat_map(|(_, changes)| changes)
        .filter(|change| {
            matches!(change, PlayerChange::Chat { message }
                if message == crate::plugin::COMMAND_BACKLOG_REFUSAL)
        })
        .count();
    (sealed, refused)
}

/// Read one event the stalled callback finally gets to, without blocking.
pub(crate) fn try_recv(
    endpoint: &mut mc_script::ScriptHostEndpoint,
) -> Option<mc_script::ScriptEvent> {
    let mut cx = Context::from_waker(Waker::noop());
    let mut next = std::pin::pin!(endpoint.recv_event());
    match next.as_mut().poll(&mut cx) {
        Poll::Ready(event) => event,
        Poll::Pending => None,
    }
}

#[test]
fn a_stalled_plugin_refuses_new_commands_instead_of_halting_the_world() {
    let (mut state, mut endpoint) = stalled_plugin_fixture();
    let mut refused_total = 0;
    for tick in 0..TICKS {
        let (sealed, refused) = tick_with_commands(&mut state, COMMANDS_PER_TICK);
        assert_eq!(
            sealed,
            Ok(true),
            "tick {tick}: the world must keep ticking while the plugin stalls"
        );
        refused_total += refused;
    }
    assert!(
        refused_total > 0,
        "commands past the backlog limit are refused to the player"
    );
    let backlog = state.plugin_runtime.as_ref().unwrap().backlog_len();
    assert!(
        backlog < mc_script::MAX_SCRIPT_EVENT_QUEUE_CAPACITY,
        "the retained backlog stays under the halt capacity: {backlog}"
    );

    // The callback finishes: the retained FIFO drains in order and the plugin
    // takes commands again.
    let written = state.tick;
    let mut delivered = 0;
    let mut delivered_commands = 0;
    loop {
        let before = delivered;
        crate::plugin::acknowledge_events(&mut state, written);
        while let Some(event) = try_recv(&mut endpoint) {
            delivered += 1;
            delivered_commands += usize::from(matches!(
                event.kind(),
                mc_script::ScriptEventKind::PlayerCommand { .. }
            ));
        }
        if delivered == before {
            break;
        }
    }
    assert_eq!(
        delivered_commands,
        COMMANDS_PER_TICK * TICKS as usize - refused_total,
        "every accepted command reaches the plugin; none is dropped"
    );
    assert_eq!(state.plugin_runtime.as_ref().unwrap().backlog_len(), 0);
    let (sealed, refused) = tick_with_commands(&mut state, 1);
    assert_eq!(sealed, Ok(true));
    assert_eq!(refused, 0, "a drained plugin accepts commands again");
    let mut reached_the_plugin = false;
    while let Some(event) = try_recv(&mut endpoint) {
        reached_the_plugin |= matches!(event.kind(),
            mc_script::ScriptEventKind::PlayerCommand { arguments, .. } if arguments == "job 0");
    }
    assert!(
        reached_the_plugin,
        "the accepted command is delivered to the plugin"
    );
}

#[test]
fn the_halt_log_counts_the_backlog_by_event_kind() {
    let (mut state, _endpoint) = stalled_plugin_fixture();
    let request = mc_script::ScriptPluginStorageGetRequest::try_new("read", "key").unwrap();
    let result =
        mc_script::ScriptEvent::plugin_storage_get_result("owner", &request, None, None).unwrap();
    for _ in 0..3 {
        crate::plugin::enqueue_result(&mut state, Some(result.clone()));
    }
    let summary = state.plugin_runtime.as_ref().unwrap().backlog_summary();
    assert!(
        summary.contains(&format!("{}=3", result.event_name())),
        "{summary}"
    );
}
