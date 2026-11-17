//! Bounded tick→edge outbox: heap accounting, the byte cap, the stall watchdog
//! and the fail-closed halt that keeps reliable batches from being dropped.

use super::*;
use crate::boundary::{ConnectionId, PlayerChange, PlayerInput};
use crate::config::CoreConfig;
use crate::core::Core;

fn chat(message: &str) -> PlayerChange {
    PlayerChange::Chat {
        message: message.into(),
    }
}

fn sender_pair(cap: usize) -> (OutboxSender, ReliableOutbox) {
    let (tx, rx) = std::sync::mpsc::channel();
    let budget = Arc::new(OutboxBudget::new(cap));
    (
        OutboxSender::new(tx, Arc::clone(&budget)),
        ReliableOutbox::new(rx, budget),
    )
}

fn join(connection: u32, name: &str) -> PlayerInput {
    PlayerInput::Join {
        connection: ConnectionId(u64::from(connection)),
        uuid: uuid::Uuid::from_u128(0xB2_0000 + u128::from(connection)),
        name: name.to_owned(),
        permission_level: 0,
    }
}

/// One connection's batch carrying `changes`.
fn batch(connection: u32, changes: Vec<PlayerChange>) -> OutboxBatch {
    vec![(ConnectionId(u64::from(connection)), changes)]
}

/// How a [`drive`] run ended.
#[derive(Debug, PartialEq, Eq)]
enum Stopped {
    /// The requested number of published ticks was observed.
    AfterTicks(u32),
    /// The tick owner stopped: the broadcast closed.
    Halted,
}

/// Drive the broadcast with a deadline, so a core that neither publishes nor
/// stops fails the gate instead of hanging it. Never polls: it awaits the
/// broadcast channel (push) and a single timeout.
fn drive(
    core: &Core,
    limit: std::time::Duration,
    ticks: Option<u32>,
    mut on_tick: impl FnMut(u32),
) -> Result<Stopped, String> {
    let mut output = core.subscribe();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("test runtime");
    runtime.block_on(async {
        tokio::time::timeout(limit, async {
            let mut seen = 0u32;
            loop {
                match output.recv().await {
                    Ok(_) => {
                        seen += 1;
                        on_tick(seen);
                        if ticks.is_some_and(|wanted| seen >= wanted) {
                            return Ok(Stopped::AfterTicks(seen));
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Ok(Stopped::Halted);
                    }
                }
            }
        })
        .await
        .map_err(|_| format!("the tick owner neither published nor stopped within {limit:?}"))?
    })
}

#[test]
fn the_hard_cap_refuses_a_batch_that_does_not_fit_and_admits_an_oversized_one_alone() {
    let one = batch_bytes(&batch(1, vec![chat("x")]));
    assert!(one > 0);
    let (sender, outbox) = sender_pair(one * 2 + one / 2);
    assert!(sender.push(1, batch(1, vec![chat("a")])).is_ok());
    assert!(sender.push(2, batch(1, vec![chat("b")])).is_ok());
    let refusal = sender.push(3, batch(1, vec![chat("c")])).unwrap_err();
    assert!(
        matches!(refusal, OutboxRefusal::Full { cap, .. } if cap == one * 2 + one / 2),
        "{refusal:?}"
    );
    assert!(refusal.reason().contains("reliable outbox full"));
    // Both batches coalesce into one entry per connection.
    let merged = outbox.drain(u64::MAX);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].1.len(), 2);
    assert_eq!(sender.queued_bytes(), 0);
    // A batch larger than the whole budget is admitted alone into an empty
    // queue, and nothing else fits until it is drained.
    let oversized = batch(
        1,
        vec![PlayerChange::PluginClientView {
            payload: vec![0u8; one * 4],
        }],
    );
    assert!(sender.push(4, oversized).is_ok());
    assert!(sender.push(5, batch(1, vec![chat("d")])).is_err());
}

#[test]
fn draining_releases_exactly_the_bytes_it_consumed() {
    let (sender, outbox) = sender_pair(1 << 20);
    for tick in 1..=3 {
        sender.push(tick, batch(1, vec![chat("a")])).unwrap();
    }
    let queued = sender.queued_bytes();
    assert!(queued > 0);
    assert_eq!(sender.budget().peak_bytes(), queued);
    let progress = sender.budget().drains();
    assert!(!outbox.drain(2).is_empty());
    // Two batches consumed: one progress epoch each (see
    // `every_consumed_batch_advances_the_progress_epoch`).
    assert_eq!(sender.budget().drains(), progress + 2);
    // Only the third batch is still queued, and it is still charged.
    assert_eq!(
        sender.queued_bytes(),
        batch_bytes(&batch(1, vec![chat("a")]))
    );
    assert!(sender.queued_bytes() < queued);
    assert!(!outbox.drain(u64::MAX).is_empty());
    assert_eq!(sender.queued_bytes(), 0);
}

#[test]
fn the_stall_watch_fires_only_after_the_limit_without_consuming_a_batch() {
    use std::time::{Duration, Instant};
    let (sender, outbox) = sender_pair(1 << 20);
    let limit = Duration::from_secs(10);
    let mut watch = StallWatch::new(limit);
    let t0 = Instant::now();
    // An empty outbox is never a stall.
    assert!(watch.stalled_at(sender.budget(), t0).is_none());
    sender.push(1, batch(1, vec![chat("q")])).unwrap();
    // The first observation only starts the clock. Steps stay under half the
    // limit: a longer gap would be a stopped owner, not a stalled pump.
    assert!(watch.stalled_at(sender.budget(), t0).is_none());
    assert!(
        watch
            .stalled_at(sender.budget(), t0 + Duration::from_secs(4))
            .is_none()
    );
    assert!(
        watch
            .stalled_at(sender.budget(), t0 + Duration::from_secs(8))
            .is_none()
    );
    // Drains that consume nothing — the queued batch is newer than `through` —
    // are not progress and cannot hold the watchdog off.
    assert!(outbox.drain(0).is_empty());
    assert!(outbox.drain(0).is_empty());
    assert!(sender.budget().queued_bytes() > 0);
    assert_eq!(sender.budget().drains(), 0);
    assert_eq!(
        watch.stalled_at(sender.budget(), t0 + Duration::from_secs(11)),
        Some(Duration::from_secs(11))
    );
    // Consuming a batch is progress: the clock restarts from the next look.
    sender.push(2, batch(1, vec![chat("r")])).unwrap();
    assert!(!outbox.drain(1).is_empty());
    assert_eq!(sender.budget().drains(), 1);
    let restart = t0 + Duration::from_secs(12);
    assert!(watch.stalled_at(sender.budget(), restart).is_none());
    for step in 1..=2 {
        assert!(
            watch
                .stalled_at(sender.budget(), restart + Duration::from_secs(4 * step))
                .is_none(),
            "step {step}"
        );
    }
    assert_eq!(
        watch.stalled_at(sender.budget(), restart + Duration::from_secs(10)),
        Some(Duration::from_secs(10))
    );
}

/// The heartbeat of every join: advancement progress JSON, the recipe book, a
/// full map patch and a 46-slot menu. `heap_bytes` must not under-count any of
/// them, because a mass reconnect is exactly the traffic the bound must survive.
#[test]
fn join_sized_payloads_are_accounted_at_least_their_heap() {
    // Advancement progress: nested JSON with long keys and strings.
    let mut progress = serde_json::Map::new();
    for index in 0..64 {
        progress.insert(
            format!("minecraft:story/advancement_{index}_with_a_long_name"),
            serde_json::json!({
                "criteria": {
                    "requirement_with_a_long_key": "2026-11-17 10:00:00 +0000",
                },
                "done": true,
            }),
        );
    }
    let progress = serde_json::Value::Object(progress);
    let mut actual = 0;
    fn walk_json(value: &serde_json::Value, actual: &mut usize) {
        match value {
            serde_json::Value::String(text) => *actual += text.capacity(),
            serde_json::Value::Array(items) => {
                *actual += items.capacity() * std::mem::size_of::<serde_json::Value>();
                for item in items {
                    walk_json(item, actual);
                }
            }
            serde_json::Value::Object(fields) => {
                *actual += fields.len() * std::mem::size_of::<(String, serde_json::Value)>();
                for (key, value) in fields {
                    *actual += key.capacity();
                    walk_json(value, actual);
                }
            }
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            }
        }
    }
    walk_json(&progress, &mut actual);
    let change = PlayerChange::Advancements {
        reset: false,
        definitions: std::sync::Arc::new(mc_data::advancements::AdvancementTree {
            definitions: Default::default(),
            trigger_index: Default::default(),
            candidate_index: Default::default(),
        }),
        progress,
    };
    assert!(
        change.heap_bytes() >= actual,
        "advancement progress under-counted: {} < {actual}",
        change.heap_bytes()
    );

    // A full map patch: 128x128 colours plus named markers.
    let colors = vec![7u8; 128 * 128];
    let markers: Vec<crate::maps::Marker> = (0..16)
        .map(|index| crate::maps::Marker {
            kind: 0,
            x: index as i8,
            z: index as i8,
            rotation: 0,
            name: Some(format!("waypoint number {index} with a long label")),
        })
        .collect();
    let update = crate::maps::MapUpdate {
        id: 1,
        scale: 0,
        locked: false,
        markers,
        rectangle: [0, 0, 128, 128],
        colors,
    };
    let colors_bytes = update.colors.capacity();
    let marker_names: usize = update
        .markers
        .iter()
        .map(|marker| marker.name.as_ref().map_or(0, |name| name.len()))
        .sum();
    let change = PlayerChange::Map {
        update: update.clone(),
    };
    let map_heap = change.heap_bytes();
    // The 128x128 colour buffer and every marker label are the payload; the
    // accounting counts capacities, so name lengths are the sound lower bound.
    assert!(
        map_heap >= colors_bytes + marker_names,
        "map update under-counted: {map_heap} < {} (colors {colors_bytes}, marker names {marker_names})",
        colors_bytes + marker_names
    );

    // A 46-slot menu of named stacks plus the cursor stack.
    let stack = |name: &str| mc_data::ItemStack {
        count: 64,
        item_id: 1,
        damage: None,
        enchantments: vec![mc_data::ItemEnchantment {
            id: mc_data::Identifier::parse("minecraft:sharpness").unwrap(),
            level: 5,
        }],
        custom_name: Some(name.into()),
        item_model: None,
        stew_effects: Vec::new(),
        components: None,
    };
    let slots: Vec<Option<mc_data::ItemStack>> = (0..46)
        .map(|index| Some(stack(&format!("Named sword {index}"))))
        .collect();
    let mut menu_actual = slots.capacity() * std::mem::size_of::<Option<mc_data::ItemStack>>();
    for slot in slots.iter().flatten() {
        menu_actual +=
            slot.enchantments.capacity() * std::mem::size_of::<mc_data::ItemEnchantment>();
        menu_actual += slot
            .enchantments
            .iter()
            .map(|enchantment| enchantment.id.heap_bytes())
            .sum::<usize>();
        menu_actual += slot.custom_name.as_ref().map_or(0, |name| name.len());
    }
    let change = PlayerChange::MenuContents {
        window: 1,
        state_id: 1,
        slots,
        cursor: Some(stack("Cursor sword")),
    };
    assert!(
        change.heap_bytes() >= menu_actual,
        "menu contents under-counted: {} < {menu_actual}",
        change.heap_bytes()
    );

    // A burst of chat lines compacted into one entry.
    let lines: Vec<PlayerChange> = (0..32)
        .map(|index| chat(&format!("system line {index} with enough text to matter")))
        .collect();
    let lines_actual = lines.capacity() * std::mem::size_of::<PlayerChange>()
        + lines
            .iter()
            .map(|change| match change {
                PlayerChange::Chat { message } => message.capacity(),
                other => panic!("unexpected line {other:?}"),
            })
            .sum::<usize>();
    let change = PlayerChange::TextBurst { lines };
    assert!(
        change.heap_bytes() >= lines_actual,
        "text burst under-counted"
    );
}

#[test]
fn the_cap_and_the_watchdog_fire_on_a_large_payload_stream_with_a_stalled_consumer() {
    use std::time::{Duration, Instant};
    // A stalled consumer with megabyte payloads: the byte cap must refuse the
    // batch that would exceed it, and the watchdog must still fire afterwards.
    let payload = vec![0u8; 3 << 20];
    let (sender, outbox) = sender_pair(8 << 20);
    let large = || {
        batch(
            1,
            vec![PlayerChange::PluginClientView {
                payload: payload.clone(),
            }],
        )
    };
    assert!(sender.push(1, large()).is_ok());
    assert!(sender.push(2, large()).is_ok());
    let refusal = sender.push(3, large()).unwrap_err();
    assert!(matches!(refusal, OutboxRefusal::Full { .. }), "{refusal:?}");
    // Empty drains do not hide the stall.
    assert!(outbox.drain(0).is_empty());
    let mut watch = StallWatch::new(Duration::from_secs(10));
    let t0 = Instant::now();
    assert!(watch.stalled_at(sender.budget(), t0).is_none());
    assert!(
        watch
            .stalled_at(sender.budget(), t0 + Duration::from_secs(4))
            .is_none()
    );
    assert!(
        watch
            .stalled_at(sender.budget(), t0 + Duration::from_secs(8))
            .is_none()
    );
    assert!(outbox.drain(0).is_empty());
    assert_eq!(
        watch.stalled_at(sender.budget(), t0 + Duration::from_secs(10)),
        Some(Duration::from_secs(10))
    );
    // A real drain releases the whole charge and restarts the clock.
    let drained = outbox.drain(u64::MAX);
    assert_eq!(drained.len(), 1);
    assert_eq!(sender.queued_bytes(), 0);
    assert!(
        watch
            .stalled_at(sender.budget(), t0 + Duration::from_secs(40))
            .is_none()
    );
}

#[test]
fn a_halted_tick_still_queues_its_terminal_disconnect_past_the_cap() {
    // Room for exactly one batch: the next push is refused, and the halt's
    // terminal batch is still admitted past the cap.
    let one = batch_bytes(&batch(1, vec![chat("reliable")]));
    let (sender, outbox) = sender_pair(one);
    assert!(sender.push(1, batch(1, vec![chat("reliable")])).is_ok());
    let refusal = sender
        .push(2, batch(1, vec![chat("reliable")]))
        .unwrap_err();
    assert!(matches!(refusal, OutboxRefusal::Full { .. }), "{refusal:?}");
    sender
        .push_terminal(
            1,
            batch(
                1,
                vec![PlayerChange::Disconnect {
                    reason: "outbox full".into(),
                }],
            ),
        )
        .expect("the terminal batch is admitted past the cap");
    assert!(
        sender.queued_bytes() > sender.budget().cap(),
        "terminal batch not admitted past the cap: {} <= {}",
        sender.queued_bytes(),
        sender.budget().cap()
    );
    let drained = outbox.drain(u64::MAX);
    assert_eq!(drained.len(), 1, "{drained:?}");
    // The queued chat batch is delivered first; the terminal reason last.
    assert_eq!(drained[0].1.len(), 2, "{drained:?}");
    assert!(
        matches!(drained[0].1.last(), Some(PlayerChange::Disconnect { .. })),
        "{drained:?}"
    );
    assert_eq!(sender.queued_bytes(), 0);
}

/// A join burst with realistic payload sizes and a consumer one tick behind:
/// a healthy pump must never look like a stall.
#[test]
fn a_burst_of_join_sized_batches_with_a_draining_consumer_does_not_halt() {
    let (sender, outbox) = sender_pair(crate::config::OUTBOX_QUEUE_BYTES);
    let registry = std::sync::Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let mut watch = StallWatch::new(OUTBOX_STALL_LIMIT);
    let now = std::time::Instant::now();
    let mut peak = 0;
    for index in 0..200u32 {
        let recipes: Vec<String> = (0..16)
            .map(|recipe| format!("minecraft:recipe_{index}_{recipe}_crafting"))
            .collect();
        sender
            .push(
                u64::from(index),
                batch(
                    index + 1,
                    vec![PlayerChange::RecipesUnlocked {
                        registry: crate::boundary::RecipeSource(std::sync::Arc::clone(&registry)),
                        recipes,
                        replace: true,
                    }],
                ),
            )
            .expect("a join-sized burst stays below the hard cap");
        peak = peak.max(sender.queued_bytes());
        // One tick behind: drain every other push.
        if index % 2 == 1 {
            let _ = outbox.drain(u64::from(index));
        }
        assert!(
            watch.stalled_at(sender.budget(), now).is_none(),
            "a draining consumer was treated as a stall at push {index}"
        );
    }
    assert!(peak > 0);
    assert!(sender.budget().peak_bytes() >= peak);
    let _ = outbox.drain(u64::MAX);
    assert_eq!(sender.queued_bytes(), 0);
}

/// One roster snapshot goes to every recipient; five roster changes at 1000
/// clients must charge five snapshots, not five thousand.
#[test]
fn a_roster_snapshot_shared_by_every_recipient_is_charged_once_per_batch() {
    let (sender, outbox) = sender_pair(crate::config::OUTBOX_QUEUE_BYTES);
    let players: Arc<[crate::dimensions::RosterPlayer]> = (0..64usize)
        .map(|index| crate::dimensions::RosterPlayer {
            connection: ConnectionId(index as u64 + 1),
            entity: crate::boundary::EntityId(index as i32 + 1),
            uuid: uuid::Uuid::from_u128(index as u128),
            name: format!("roster player {index} with a longer name"),
            game_mode: 0,
        })
        .collect::<Vec<_>>()
        .into();
    let removed: Arc<[uuid::Uuid]> = Arc::from(Vec::new());
    let snapshot = players.len() * std::mem::size_of::<crate::dimensions::RosterPlayer>()
        + players
            .iter()
            .map(|player| player.name.capacity())
            .sum::<usize>();
    let recipients = 1000u32;
    let per_recipient = std::mem::size_of::<PlayerChange>() + CONNECTION_ALLOWANCE;
    let changes = 5u32;
    for change in 1..=changes {
        let mut batch: OutboxBatch = Vec::with_capacity(recipients as usize);
        for connection in 1..=recipients {
            batch.push((
                ConnectionId(u64::from(connection)),
                vec![PlayerChange::Roster {
                    players: Arc::clone(&players),
                    removed: Arc::clone(&removed),
                }],
            ));
        }
        sender
            .push(u64::from(change), batch)
            .expect("a shared snapshot stays far below the cap");
    }
    let queued = sender.queued_bytes();
    let expected = changes as usize * (snapshot + recipients as usize * per_recipient);
    assert!(
        queued >= changes as usize * snapshot,
        "the snapshot was not charged"
    );
    assert!(
        queued <= expected + expected / 4,
        "a shared snapshot was charged per recipient: {queued} > {expected} (snapshot {snapshot}, recipients {recipients})"
    );
    let _ = outbox.drain(u64::MAX);
    assert_eq!(sender.queued_bytes(), 0);
}

/// A new roster snapshot is allocated on every roster change (joins/leaves), and
/// a queued batch keeps its generation alive: the accounting must grow with
/// every retained snapshot, not count them as shared.
#[test]
fn retained_roster_snapshots_are_accounted_by_the_queued_bytes() {
    let (sender, outbox) = sender_pair(1 << 20);
    let mut retained = 0usize;
    for players in 1..=8usize {
        let roster: Vec<crate::dimensions::RosterPlayer> = (0..players)
            .map(|index| crate::dimensions::RosterPlayer {
                connection: ConnectionId(index as u64 + 1),
                entity: crate::boundary::EntityId(index as i32 + 1),
                uuid: uuid::Uuid::from_u128(index as u128),
                name: format!("roster player {index} with a longer name"),
                game_mode: 0,
            })
            .collect();
        retained += roster.len() * std::mem::size_of::<crate::dimensions::RosterPlayer>()
            + roster
                .iter()
                .map(|player| player.name.capacity())
                .sum::<usize>();
        sender
            .push(
                players as u64,
                batch(
                    1,
                    vec![PlayerChange::Roster {
                        players: roster.into(),
                        removed: std::sync::Arc::from(Vec::<uuid::Uuid>::new()),
                    }],
                ),
            )
            .expect("roster batches stay below the cap");
    }
    assert!(
        sender.queued_bytes() >= retained,
        "retained roster snapshots were not charged: {} < {retained}",
        sender.queued_bytes()
    );
    let _ = outbox.drain(u64::MAX);
    assert_eq!(sender.queued_bytes(), 0);
}

/// The price must follow the owned payload, not the variant: the same change
/// with a small and a large payload differs by at least the payload's size. A
/// flat per-change allowance would hide a 64 KiB chat line or a 512-entry recipe
/// book behind the same few bytes.
#[test]
fn accounting_grows_with_the_owned_payload() {
    let empty = batch_bytes(&batch(1, vec![chat("")]));
    let large_text = "x".repeat(1 << 16);
    let large = batch_bytes(&batch(1, vec![chat(&large_text)]));
    assert!(
        large >= empty + large_text.capacity(),
        "a 64 KiB chat line was not priced: {large} vs {empty}"
    );

    let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let recipes = |count: usize| PlayerChange::RecipesUnlocked {
        registry: crate::boundary::RecipeSource(Arc::clone(&registry)),
        recipes: (0..count)
            .map(|index| format!("minecraft:recipe_with_a_name_{index}"))
            .collect(),
        replace: true,
    };
    let none = batch_bytes(&batch(1, vec![recipes(0)]));
    let many = batch_bytes(&batch(1, vec![recipes(512)]));
    let recipe_bytes: usize = (0..512usize)
        .map(|index| format!("minecraft:recipe_with_a_name_{index}").len())
        .sum();
    assert!(
        many >= none + recipe_bytes,
        "a 512-entry recipe book was not priced: {many} vs {none} (+{recipe_bytes})"
    );
}

#[test]
fn a_full_outbox_halts_the_tick_and_keeps_the_terminal_disconnect() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.checkpoint_every_ticks = u64::MAX;
    // The very first batch cannot fit: the push is refused, never dropped
    // silently and never a blocking send.
    config.outbox_queue_bytes = 1;
    let core = Core::open(config.clone()).expect("open");
    core.submit(join(1, "capped"));
    assert_eq!(
        drive(&core, std::time::Duration::from_secs(60), None, |_| {})
            .expect("the tick owner stops after the refused push"),
        Stopped::Halted
    );
    let drained = core.try_drain_outbox();
    assert!(
        drained
            .iter()
            .any(|(connection, changes)| *connection == ConnectionId(1)
                && changes
                    .iter()
                    .any(|change| matches!(change, PlayerChange::Disconnect { .. }))),
        "the halted tick must still deliver its Disconnect: {drained:?}"
    );
    assert_eq!(core.outbox_queued_bytes(), 0);
    core.shutdown();
    // Fail-closed is the designed recovery path: the WAL replays and the world
    // opens again, with the lease and store released.
    Core::open(config)
        .expect("reopen after the outbox halt")
        .shutdown();
}

#[test]
fn every_consumed_batch_advances_the_progress_epoch() {
    let (sender, outbox) = sender_pair(1 << 20);
    for connection in 1..=3u32 {
        sender
            .push(u64::from(connection), batch(connection, vec![chat("q")]))
            .unwrap();
    }
    assert_eq!(sender.budget().drains(), 0);
    let drained = outbox.drain(u64::MAX);
    assert_eq!(drained.len(), 3);
    // One epoch step per consumed batch. A pump that stops after batch 1 of 3
    // has already recorded progress: its epoch moved at that batch, so the tick
    // cannot charge it with a stale epoch, and the next observation restarts
    // the stall clock instead of halting a pump that is merely late.
    assert_eq!(sender.budget().drains(), 3);
}

/// A paused world has no other stall check: without the check inside the paused
/// branch, a wedged pump would hold the queue until an unrelated join woke the
/// world.
#[test]
fn a_paused_world_halts_when_the_pump_stops_consuming() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.checkpoint_every_ticks = u64::MAX;
    config.pause_when_empty_seconds = 1;
    config.outbox_stall_ms = 2_000;
    let core = Core::open(config).expect("open");
    core.submit(join(1, "leaver"));
    assert_eq!(
        drive(&core, std::time::Duration::from_secs(60), Some(1), |_| {})
            .expect("the join tick publishes"),
        Stopped::AfterTicks(1)
    );
    core.submit(PlayerInput::Leave {
        connection: ConnectionId(1),
    });
    // Never drain: the world empties and pauses after one second, and the
    // queued batch then outlives the 2 s stall limit with the world paused.
    assert_eq!(
        drive(&core, std::time::Duration::from_secs(20), None, |_| {})
            .expect("a paused world halts when the pump stops consuming"),
        Stopped::Halted
    );
    core.shutdown();
}

/// The pause refusal is a reliable message to its sender: with a full queue the
/// tick must halt (fail closed) rather than drop the refusal. The stall limit is
/// far out, so only the refusal path can stop this tick owner.
#[test]
fn a_refused_pause_answer_halts_instead_of_dropping_it() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.checkpoint_every_ticks = u64::MAX;
    // Any push after the single admitted batch is refused.
    config.outbox_queue_bytes = 1;
    config.outbox_stall_ms = 60_000;
    let core = Core::open(config).expect("open");
    core.submit(join(1, "operator"));
    assert_eq!(
        drive(&core, std::time::Duration::from_secs(60), Some(1), |_| {})
            .expect("the join tick publishes"),
        Stopped::AfterTicks(1)
    );
    // One level-0 player: the pause is refused, and the refusal cannot be
    // queued, so the tick stops instead of losing the message.
    core.submit(PlayerInput::Command {
        connection: ConnectionId(1),
        command: "krasnokamen_pause on".into(),
        permission_level: 0,
    });
    assert_eq!(
        drive(&core, std::time::Duration::from_secs(20), None, |_| {})
            .expect("a refused reliable pause answer stops the tick owner"),
        Stopped::Halted
    );
    // The admitted join batch is still drainable; nothing was lost by the halt.
    assert!(!core.try_drain_outbox().is_empty());
    core.shutdown();
}

#[test]
fn a_pump_that_stops_draining_halts_the_tick_after_the_stall_limit() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.checkpoint_every_ticks = u64::MAX;
    // Production is 10 s; the gate proves the same path without the wait.
    config.outbox_stall_ms = 50;
    let core = Core::open(config.clone()).expect("open");
    core.submit(join(1, "stalled"));
    assert_eq!(
        drive(&core, std::time::Duration::from_secs(60), None, |_| {})
            .expect("the tick owner stops after the stall limit"),
        Stopped::Halted
    );
    // The batches queued before the halt were never dropped: draining still
    // delivers them, ordered, with the terminal Disconnect last.
    let drained = core.try_drain_outbox();
    let changes: Vec<&PlayerChange> = drained
        .iter()
        .filter(|(connection, _)| *connection == ConnectionId(1))
        .flat_map(|(_, changes)| changes.iter())
        .collect();
    assert!(!changes.is_empty(), "queued batches were dropped");
    assert!(matches!(
        changes.last(),
        Some(PlayerChange::Disconnect { .. })
    ));
    assert!(core.outbox_peak_bytes() > 0);
    core.shutdown();
}

#[test]
fn a_mass_join_burst_does_not_halt_a_consumer_that_drains_every_other_tick() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.checkpoint_every_ticks = u64::MAX;
    // Production cap and stall limit: a restart burst of joins must not look
    // like a stalled pump.
    config.outbox_queue_bytes = crate::config::OUTBOX_QUEUE_BYTES;
    config.outbox_stall_ms = OUTBOX_STALL_LIMIT.as_millis() as u64;
    let core = Core::open(config).expect("open");
    for index in 0..200u32 {
        assert!(
            core.submit(join(index + 1, &format!("burst{index}"))),
            "join {index} was dropped by the input queue"
        );
    }
    let mut peak = 0usize;
    let stopped = drive(
        &core,
        std::time::Duration::from_secs(120),
        Some(20),
        |ticks| {
            peak = peak.max(core.outbox_peak_bytes());
            // A consumer one tick behind: drains, but not on every tick.
            if ticks % 2 == 0 {
                let _ = core.try_drain_outbox();
            }
        },
    )
    .expect("a join burst halted a consumer that drains every other tick");
    assert_eq!(stopped, Stopped::AfterTicks(20));
    assert!(peak > 0, "the burst never queued a reliable batch");
    core.shutdown();
}
