use super::*;

/// A stalled journal receiver parks producers instead of growing the
/// queue: the bounded [`journal_work_channel`] refuses `try_send` while
/// full, a blocking `send` completes only after a drain frees a slot, and
/// every accepted batch arrives exactly once in FIFO order. No sleeps or
/// elapsed-time assertions: full/empty are channel state, and thread
/// `join` is the admission event.
#[test]
fn stalled_journal_receiver_applies_backpressure_and_drains_fifo() {
    bounded(
        "stalled_journal_receiver_applies_backpressure_and_drains_fifo",
        stalled_journal_receiver_applies_backpressure_and_drains_fifo_body,
    );
}

fn stalled_journal_receiver_applies_backpressure_and_drains_fifo_body() {
    let (work_tx, work_rx) = journal_work_channel();
    for tick in 0..JOURNAL_WORK_QUEUE as u64 {
        work_tx
            .send(JournalWork::Append {
                tick_lo: tick,
                tick_hi: tick,
                entries: Vec::new(),
                plugin_ack: None,
                reserved: 0,
            })
            .unwrap();
    }
    // Bounded: with the receiver stalled and the queue full, no further
    // batch is admitted and nothing accepted is dropped.
    assert!(matches!(
        work_tx.try_send(JournalWork::Shutdown),
        Err(std::sync::mpsc::TrySendError::Full(_))
    ));
    let blocked = std::thread::spawn({
        let work_tx = work_tx.clone();
        move || work_tx.send(JournalWork::Shutdown)
    });
    // Still no drain, so the producer cannot have been admitted yet: the
    // queue is still full and its thread still parked in `send`.
    assert!(matches!(
        work_tx.try_send(JournalWork::Append {
            tick_lo: u64::MAX,
            tick_hi: u64::MAX,
            entries: Vec::new(),
            plugin_ack: None,
            reserved: 0,
        }),
        Err(std::sync::mpsc::TrySendError::Full(_))
    ));
    assert!(!blocked.is_finished());
    // Drain one batch: the parked producer is admitted exactly once, and
    // everything arrives in FIFO order.
    let first = work_rx.recv().unwrap();
    blocked.join().expect("journal producer").unwrap();
    let drained: Vec<JournalWork> = std::iter::once(first)
        .chain((0..JOURNAL_WORK_QUEUE).map(|_| work_rx.recv().unwrap()))
        .collect();
    assert!(
        work_rx.try_recv().is_err(),
        "nothing beyond the admitted batches"
    );
    for (index, work) in drained.into_iter().enumerate() {
        if index < JOURNAL_WORK_QUEUE {
            match work {
                JournalWork::Append {
                    tick_lo, tick_hi, ..
                } => {
                    assert_eq!((tick_lo, tick_hi), (index as u64, index as u64));
                }
                JournalWork::Checkpoint(_)
                | JournalWork::RequireGenerated { .. }
                | JournalWork::BaselineDurable { .. }
                | JournalWork::Backup(_)
                | JournalWork::Shutdown => panic!("journal FIFO order broken"),
            }
        } else {
            assert!(matches!(work, JournalWork::Shutdown));
        }
    }
    // Disconnect still fails sends: the fail-closed journal-death signal
    // the tick loop relies on is preserved.
    drop(work_rx);
    assert!(work_tx.send(JournalWork::Shutdown).is_err());
}

fn block_batch(len: usize) -> Vec<JournalEntry> {
    (0..len)
        .map(|index| JournalEntry::Block {
            pos: [index as i32, 0, 0],
            state: 1,
        })
        .collect()
}

/// The count cap (128) alone would admit these batches; the byte budget parks
/// the producer once two batches are in flight and resumes it exactly when the
/// journal side releases a drained batch. Nothing is dropped, order is FIFO, a
/// batch larger than the whole budget is still admitted alone, and closing the
/// budget (journal thread death) fails a parked producer.
#[test]
fn oversized_batches_block_sender_until_bytes_are_released() {
    bounded(
        "oversized_batches_block_sender_until_bytes_are_released",
        oversized_batches_block_sender_until_bytes_are_released_body,
    );
}

fn oversized_batches_block_sender_until_bytes_are_released_body() {
    use crate::journal_budget::entries_bytes;
    let (work_tx, work_rx) = journal_work_channel();
    let bytes = entries_bytes(&block_batch(1000));
    let budget = Arc::new(JournalBudget::new(bytes * 2));
    for tick in 0..2 {
        send_append(&work_tx, &budget, tick, block_batch(1000), None).unwrap();
    }
    // Plenty of count slots remain, yet no third batch fits the bytes.
    assert_eq!(budget.try_reserve(bytes), Ok(false));
    let blocked = std::thread::spawn({
        let (work_tx, budget) = (work_tx.clone(), Arc::clone(&budget));
        move || send_append(&work_tx, &budget, 2, block_batch(1000), None)
    });
    assert!(!blocked.is_finished());
    // The journal thread drains and releases the first batch: the parked
    // producer is admitted, and all three arrive in FIFO order.
    let JournalWork::Append {
        tick_lo, reserved, ..
    } = work_rx.recv().unwrap()
    else {
        panic!("expected append");
    };
    assert_eq!(tick_lo, 0);
    budget.release(reserved);
    blocked.join().expect("journal producer").unwrap();
    for expected in 1..3 {
        let JournalWork::Append { tick_lo, .. } = work_rx.recv().unwrap() else {
            panic!("expected append");
        };
        assert_eq!(tick_lo, expected);
    }
    assert!(work_rx.try_recv().is_err(), "nothing beyond the admitted");
    // Batches 1 and 2 are still in flight (budget full); a dead journal
    // thread fails a parked producer instead of hanging it.
    let parked = std::thread::spawn({
        let (work_tx, budget) = (work_tx.clone(), Arc::clone(&budget));
        move || send_append(&work_tx, &budget, 3, block_batch(1000), None)
    });
    budget.close();
    assert!(parked.join().expect("journal producer").is_err());
    // A single batch above the whole budget is admitted when nothing is in flight.
    let tiny = JournalBudget::new(bytes / 2);
    assert_eq!(tiny.try_reserve(bytes), Ok(true));
    assert_eq!(tiny.try_reserve(1), Ok(false));
}

/// A few heap-heavy entries (NBT, named/componented stacks, an entity image)
/// reserve at least the heap they own, so they alone exhaust the budget and
/// block the sender until the journal side releases exactly what was
/// reserved, while a count of their slots (the old bound) would admit them.
#[test]
fn heap_heavy_entries_reserve_their_payload_and_block_the_sender() {
    bounded(
        "heap_heavy_entries_reserve_their_payload_and_block_the_sender",
        heap_heavy_entries_reserve_their_payload_and_block_the_sender_body,
    );
}

fn heap_heavy_entries_reserve_their_payload_and_block_the_sender_body() {
    use crate::journal_budget::entries_bytes;
    const NBT: usize = 1 << 20;
    let mut stack = mc_data::ItemStack::new(1, 1);
    stack.custom_name = Some("n".repeat(NBT).into_boxed_str());
    let components = mc_data::item_stack::ExtendedItemComponents {
        custom_data: Some(vec![0; NBT]),
        ..Default::default()
    };
    let mut component_stack = mc_data::ItemStack::new(2, 1);
    component_stack.components = Some(Box::new(components));
    let appearance = crate::journal::EntityAppearanceImage {
        equipment: vec![Some(stack.clone()); 2],
        ..Default::default()
    };
    let entity = JournalEntry::Entity {
        id: 1,
        uuid: String::new(),
        type_id: 0,
        health: 1.0,
        alive: true,
        age: 0,
        breed_cooldown: 0,
        pickup_delay: 0,
        item: None,
        appearance,
        active_effects: Default::default(),
        villager: None,
        state: Default::default(),
        projectile: None,
        tick: 0,
        pos: [0.0; 3],
        vel: [0.0; 3],
        yaw: 0.0,
        pitch: 0.0,
    };
    let batch = move || {
        vec![
            JournalEntry::OpaqueBlockEntity {
                pos: [0, 0, 0],
                nbt: Some(vec![0; NBT]),
            },
            JournalEntry::BlockEntity {
                pos: [1, 0, 0],
                slots: vec![component_stack.clone()],
                furnace: None,
                tick: 0,
            },
            entity.clone(),
        ]
    };
    // NBT + component custom_data + two 1 MiB names (entity equipment).
    let heap = 4 * NBT;
    let bytes = entries_bytes(&batch());
    assert!(bytes >= heap, "estimate {bytes} undercounts heap {heap}");
    let (work_tx, work_rx) = journal_work_channel();
    let budget = Arc::new(JournalBudget::new(bytes + bytes / 2));
    send_append(&work_tx, &budget, 0, batch(), None).unwrap();
    // Three entry slots only; the payload alone fills the budget.
    assert_eq!(budget.try_reserve(bytes), Ok(false));
    let blocked = std::thread::spawn({
        let (work_tx, budget, batch) = (work_tx.clone(), Arc::clone(&budget), batch.clone());
        move || send_append(&work_tx, &budget, 1, batch(), None)
    });
    assert!(!blocked.is_finished());
    let JournalWork::Append { reserved, .. } = work_rx.recv().unwrap() else {
        panic!("expected append");
    };
    assert_eq!(reserved, bytes);
    budget.release(reserved);
    blocked.join().expect("journal producer").unwrap();
}

/// One `Append` of `entries` reserves at least `heap` bytes, fills a budget
/// sized 1.5x its estimate, and blocks the next sender until the journal side
/// releases exactly the reserved number.
fn assert_reserves_heap_and_blocks(entries: Vec<JournalEntry>, heap: usize) {
    let bytes = crate::journal_budget::entries_bytes(&entries);
    assert!(bytes >= heap, "estimate {bytes} undercounts heap {heap}");
    let (work_tx, work_rx) = journal_work_channel();
    let budget = Arc::new(JournalBudget::new(bytes + bytes / 2));
    send_append(&work_tx, &budget, 0, entries.clone(), None).unwrap();
    assert_eq!(budget.try_reserve(bytes), Ok(false));
    let blocked = std::thread::spawn({
        let (work_tx, budget) = (work_tx.clone(), Arc::clone(&budget));
        move || send_append(&work_tx, &budget, 1, entries, None)
    });
    assert!(!blocked.is_finished());
    let JournalWork::Append { reserved, .. } = work_rx.recv().unwrap() else {
        panic!("expected append");
    };
    // The first batch is a clone (exact string capacities), so it reserves
    // what its own capacities say: at least the heap, at most the original's.
    assert!(reserved >= heap && reserved <= bytes);
    budget.release(reserved);
    blocked.join().expect("journal producer").unwrap();
}

fn entity_with(
    villager: Option<Box<crate::villager::VillagerState>>,
    projectile: Option<Box<crate::projectiles::ProjectileState>>,
) -> JournalEntry {
    JournalEntry::Entity {
        id: 1,
        uuid: String::new(),
        type_id: 0,
        health: 1.0,
        alive: true,
        age: 0,
        breed_cooldown: 0,
        pickup_delay: 0,
        item: None,
        appearance: Default::default(),
        active_effects: Default::default(),
        villager,
        state: Default::default(),
        projectile,
        tick: 0,
        pos: [0.0; 3],
        vel: [0.0; 3],
        yaw: 0.0,
        pitch: 0.0,
    }
}

/// Growing nested containers are walked, not flat-priced: a scoreboard team
/// with 100k members and villagers with large-custom-data offers reserve at
/// least their real heap and block the sender once the budget is full.
#[test]
fn growing_nested_containers_reserve_their_heap_and_block_the_sender() {
    bounded(
        "growing_nested_containers_reserve_their_heap_and_block_the_sender",
        growing_nested_containers_reserve_their_heap_and_block_the_sender_body,
    );
}

fn growing_nested_containers_reserve_their_heap_and_block_the_sender_body() {
    const MEMBERS: usize = 100_000;
    let mut board = crate::scoreboard::Scoreboard::default();
    let mut team = crate::scoreboard::Team {
        display_name: "t".into(),
        color: 21,
        friendly_fire: true,
        see_friendly_invisibles: true,
        nametag_visibility: 0,
        death_message_visibility: 0,
        collision_rule: 0,
        prefix: String::new(),
        suffix: String::new(),
        members: Default::default(),
    };
    team.members = (0..MEMBERS).map(|index| format!("member{index}")).collect();
    let member_heap: usize = team.members.iter().map(String::capacity).sum::<usize>()
        + MEMBERS * std::mem::size_of::<String>();
    board.teams.insert("t".into(), team);
    assert_reserves_heap_and_blocks(vec![JournalEntry::Scoreboard { value: board }], member_heap);

    const DATA: usize = 1 << 20;
    let mut result = mc_data::ItemStack::new(2, 1);
    result.components = Some(Box::new(mc_data::item_stack::ExtendedItemComponents {
        custom_data: Some(vec![0; DATA]),
        ..Default::default()
    }));
    let offer = crate::villager::VillagerTradeOffer {
        cost_a: mc_data::ItemStack::new(1, 1),
        cost_b: None,
        result,
        uses: 0,
        max_uses: 12,
        reward_exp: true,
        xp: 1,
        special_price: 0,
        demand: 0,
        price_multiplier: 0.05,
    };
    let villager = crate::villager::VillagerState {
        offers: vec![offer; 3],
        ..Default::default()
    };
    assert_reserves_heap_and_blocks(vec![entity_with(Some(Box::new(villager)), None)], 3 * DATA);
}

/// A lingering cloud's hit history is a growing map: it must move the estimate.
#[test]
fn cloud_hit_history_grows_the_estimate() {
    bounded(
        "cloud_hit_history_grows_the_estimate",
        cloud_hit_history_grows_the_estimate_body,
    );
}

fn cloud_hit_history_grows_the_estimate_body() {
    const HITS: usize = 50_000;
    let projectile = |hits: usize| {
        let mut state = crate::projectiles::ProjectileState::new(
            crate::projectiles::ProjectileKind::AreaEffectCloud,
            None,
            mc_data::ItemStack::EMPTY,
        );
        let mut cloud = crate::projectiles::payloads::CloudState::empty(0);
        cloud.affected_until = (0..hits as u128)
            .map(|index| (uuid::Uuid::from_u128(index), 0))
            .collect();
        state.data = crate::projectiles::ProjectileData::Cloud(cloud);
        vec![entity_with(None, Some(Box::new(state)))]
    };
    let empty = crate::journal_budget::entries_bytes(&projectile(0));
    let full = projectile(HITS);
    assert!(
        crate::journal_budget::entries_bytes(&full)
            >= empty + HITS * (std::mem::size_of::<uuid::Uuid>() + std::mem::size_of::<u64>())
    );
    assert_reserves_heap_and_blocks(full, HITS * 24);
}

#[test]
fn refused_required_generated_baseline_withholds_dependent_wal_and_closes_admission() {
    bounded(
        "refused_required_generated_baseline_withholds_dependent_wal_and_closes_admission",
        refused_required_generated_baseline_withholds_dependent_wal_and_closes_admission_body,
    );
}

fn refused_required_generated_baseline_withholds_dependent_wal_and_closes_admission_body() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_owned(), 42);
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let mut registry = CoreRegistry::load(None).unwrap();
    crate::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let registry = Arc::new(registry);
    let (mut state, manifest, _) =
        recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
    // A regular dirty capture cannot be accepted as an immutable generated
    // baseline. Exercise a real native-worker refusal, not a fake writer.
    let invalid = state.world.capture_dirty(&Default::default());
    let (work_tx, work_rx) = journal_work_channel();
    work_tx
        .send(JournalWork::RequireGenerated {
            capture: Box::new(invalid),
            wake: work_tx.clone(),
        })
        .unwrap();
    let budget = JournalBudget::new(1 << 20);
    send_append(
        &work_tx,
        &budget,
        manifest.tick + 1,
        vec![JournalEntry::Block {
            pos: [0, 200, 0],
            state: registry.states.dirt,
        }],
        None,
    )
    .unwrap();
    work_tx.send(JournalWork::Shutdown).unwrap();
    journal::journal_loop(
        directory.path(),
        Arc::clone(&store),
        work_rx,
        JournalWriter::open_test(directory.path(), 1).unwrap(),
        &budget,
        manifest,
        crate::plugin::PluginState::default(),
    );
    assert!(matches!(
        work_tx.try_send(JournalWork::Shutdown),
        Err(std::sync::mpsc::TrySendError::Disconnected(_))
    ));
    assert!(
        std::fs::read_dir(directory.path().join("journal"))
            .unwrap()
            .all(|entry| entry
                .unwrap()
                .path()
                .extension()
                .is_none_or(|ext| ext != "bin"))
    );
    assert_eq!(store.read_checkpoint().unwrap().tick, 0);
}

/// A tick that generates a chunk and edits it in the same tick: the tick owner
/// queues the immutable baseline ahead of the dependent WAL, and the real
/// journal owner appends that WAL only once the baseline is durable in the
/// native store. A crash right after the append therefore never leaves a
/// published edit over a base that restart would have to regenerate.
#[test]
fn same_tick_generated_baseline_is_durable_before_its_dependent_wal_append() {
    bounded(
        "same_tick_generated_baseline_is_durable_before_its_dependent_wal_append",
        same_tick_generated_baseline_is_durable_before_its_dependent_wal_append_body,
    );
}

fn same_tick_generated_baseline_is_durable_before_its_dependent_wal_append_body() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_owned(), 42);
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let mut registry = CoreRegistry::load(None).unwrap();
    crate::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let registry = Arc::new(registry);
    let (mut state, manifest, _) =
        recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
    let pos = crate::boundary::ChunkPos { x: 3, z: -2 };
    let source = state.world.chunk_source();
    assert!(source.load_native(pos).unwrap().is_none());
    assert_eq!(
        state
            .world
            .ensure_blocking(&[(pos, 0)].into_iter().collect()),
        vec![pos]
    );
    let tick = manifest.tick + 1;
    let (work_tx, work_rx) = journal_work_channel();
    let budget = JournalBudget::new(1 << 20);
    send_tick_journal(
        &mut state,
        &work_tx,
        &budget,
        tick,
        vec![JournalEntry::Block {
            pos: [pos.x * 16, 200, pos.z * 16],
            state: registry.states.dirt,
        }],
        None,
    )
    .unwrap();
    work_tx.send(JournalWork::Shutdown).unwrap();
    let (appended, observed) = std::sync::mpsc::channel();
    *crate::core::JOURNAL_APPEND_OBSERVER.lock().unwrap() = Some((
        directory.path().to_owned(),
        Box::new(move |tick, _| {
            appended
                .send((tick, source.load_native(pos).unwrap().is_some()))
                .unwrap();
        }),
    ));
    journal::journal_loop(
        directory.path(),
        Arc::clone(&store),
        work_rx,
        JournalWriter::open_test(directory.path(), 1).unwrap(),
        &budget,
        manifest,
        crate::plugin::PluginState::default(),
    );
    *crate::core::JOURNAL_APPEND_OBSERVER.lock().unwrap() = None;
    assert_eq!(
        observed.try_iter().collect::<Vec<_>>(),
        vec![(tick, true)],
        "the dependent WAL batch was appended before its generated baseline was durable"
    );
}

/// Real symptom (checkpoint-join-stall M1): the native writer is held inside a
/// checkpoint (after its commit, before its ACK) and a tick that generated a
/// chunk queues its baseline behind it. The journal owner must keep draining
/// the tick's queue while that baseline waits: later batches are held in
/// arrival order and published only once the baseline is durable, so a tick
/// never parks its admission on a multi-second checkpoint.
#[test]
fn baseline_behind_held_checkpoint_keeps_tick_admission_draining() {
    bounded(
        "baseline_behind_held_checkpoint_keeps_tick_admission_draining",
        baseline_behind_held_checkpoint_keeps_tick_admission_draining_body,
    );
}

fn baseline_behind_held_checkpoint_keeps_tick_admission_draining_body() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_owned(), 42);
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let mut registry = CoreRegistry::load(None).unwrap();
    crate::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let registry = Arc::new(registry);
    let (mut state, manifest, _) =
        recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
    let (entered_tx, entered) = std::sync::mpsc::channel::<()>();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let released = parking_lot::Mutex::new(released);
    journal::CHECKPOINT_STAGE_OBSERVERS.lock().push((
        directory.path().to_owned(),
        Box::new(move |stage| {
            if stage == "before_ack" {
                entered_tx.send(()).unwrap();
                released.lock().recv().unwrap();
            }
        }),
    ));
    let budget = Arc::new(JournalBudget::new(JOURNAL_QUEUE_BYTES));
    let (work_tx, work_rx) = journal_work_channel();
    let tick = manifest.tick + 1;
    let journal = {
        let root = directory.path().to_owned();
        let store = Arc::clone(&store);
        let budget = Arc::clone(&budget);
        std::thread::spawn(move || {
            journal::journal_loop(
                &root,
                store,
                work_rx,
                JournalWriter::open_test(&root, 1).unwrap(),
                &budget,
                manifest,
                crate::plugin::PluginState::default(),
            );
        })
    };
    state.tick = tick;
    send_append(&work_tx, &budget, tick, Vec::new(), None).unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    assert!(start_checkpoint(&mut state, &work_tx, &done_tx, tick));
    entered.recv().unwrap();
    // The native writer now holds checkpoint `tick`. The next tick generates a
    // chunk: its baseline goes ahead of its WAL batch.
    let pos = crate::boundary::ChunkPos { x: 3, z: -2 };
    let source = state.world.chunk_source();
    assert!(source.load_native(pos).unwrap().is_none());
    assert_eq!(
        state
            .world
            .ensure_blocking(&[(pos, 0)].into_iter().collect()),
        vec![pos]
    );
    state.tick = tick + 1;
    let (appended, observed) = std::sync::mpsc::channel();
    *crate::core::JOURNAL_APPEND_OBSERVER.lock().unwrap() = Some((
        directory.path().to_owned(),
        Box::new(move |tick, _| {
            appended
                .send((tick, source.load_native(pos).unwrap().is_some()))
                .unwrap();
        }),
    ));
    send_tick_journal(&mut state, &work_tx, &budget, tick + 1, Vec::new(), None).unwrap();
    // The real symptom: a tick's blocking sends keep completing while the
    // baseline waits behind the held checkpoint. A transient full queue is
    // expected (the producer outruns the owner); what must not happen is the
    // permanent stall of the old owner, which drained nothing until `release`.
    // The timeout below is a failure deadline for that stall, never a poll.
    let (sent_tx, sent) = std::sync::mpsc::channel();
    let parked = {
        let work_tx = work_tx.clone();
        let budget = Arc::clone(&budget);
        std::thread::spawn(move || {
            // One more than the bounded queue holds, so a queue-only drain
            // cannot pass this and the hold is what admits them.
            for admitted in 0..(JOURNAL_WORK_QUEUE as u64 + 2) {
                send_append(&work_tx, &budget, tick + 2 + admitted, Vec::new(), None).unwrap();
            }
            sent_tx.send(()).unwrap();
        })
    };
    sent.recv_timeout(std::time::Duration::from_secs(5))
        .expect("tick sends must not park behind the held checkpoint");
    parked.join().unwrap();
    release.send(()).unwrap();
    work_tx.send(JournalWork::Shutdown).unwrap();
    join_before_deadline(
        journal,
        "the journal owner never released a held batch after its baseline became durable",
    );
    journal::CHECKPOINT_STAGE_OBSERVERS
        .lock()
        .retain(|(path, _)| path != directory.path());
    *crate::core::JOURNAL_APPEND_OBSERVER.lock().unwrap() = None;
    assert!(done_rx.recv().unwrap().is_some());
    let seen: Vec<(u64, bool)> = observed.try_iter().collect();
    let count = JOURNAL_WORK_QUEUE as u64 + 2;
    assert_eq!(
        seen.iter().map(|seen| seen.0).collect::<Vec<_>>(),
        (tick + 1..=tick + 1 + count).collect::<Vec<_>>(),
        "the dependent and every held batch publish once, in arrival order, after the baseline"
    );
    assert!(
        seen.iter().all(|&(_, durable)| durable),
        "a later WAL batch published before its generated baseline was durable"
    );
}

/// Runs one test body and fails with a named assertion when it does not finish:
/// a deadlocked owner thread must fail the test instead of parking the whole
/// binary (libtest joins its test threads). The bound is a failure deadline for
/// the complete body, never a poll; the tighter per-step deadlines inside the
/// bodies stay the proving assertions.
fn bounded(name: &'static str, body: fn()) {
    const LIFECYCLE: std::time::Duration = std::time::Duration::from_secs(60);
    let (finished, done) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(body);
        let _ = finished.send(outcome);
    });
    match done.recv_timeout(LIFECYCLE) {
        Ok(Ok(())) => {}
        // Report the body's own assertion immediately instead of masking it
        // behind the lifecycle deadline.
        Ok(Err(payload)) => std::panic::resume_unwind(payload),
        Err(_) => panic!("{name}: test body did not finish within {LIFECYCLE:?}"),
    }
}

/// Joins `journal` and fails (rather than hanging the whole binary) when the
/// owner never returns: the deadline is a failure bound, never a poll.
fn join_before_deadline(journal: std::thread::JoinHandle<()>, what: &str) {
    join_before(journal, std::time::Duration::from_secs(10), what);
}

/// [`join_before_deadline`] with an explicit bound for bodies that publish many
/// batches; both are failure deadlines, never polls.
fn join_before(journal: std::thread::JoinHandle<()>, limit: std::time::Duration, what: &str) {
    let (finished, done) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        journal.join().unwrap();
        let _ = finished.send(());
    });
    done.recv_timeout(limit)
        .unwrap_or_else(|_| panic!("{what}"));
}

/// The journal owner must not keep a sender of its own work queue alive: when
/// the tick closes the queue while a baseline is still held behind a
/// checkpoint, the owner has to finish (closing admission) instead of waiting
/// for work that can no longer arrive.
#[test]
fn closed_tick_queue_releases_a_held_baseline_wait() {
    bounded(
        "closed_tick_queue_releases_a_held_baseline_wait",
        closed_tick_queue_releases_a_held_baseline_wait_body,
    );
}

fn closed_tick_queue_releases_a_held_baseline_wait_body() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_owned(), 42);
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let mut registry = CoreRegistry::load(None).unwrap();
    crate::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let registry = Arc::new(registry);
    let (mut state, manifest, _) =
        recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
    let (entered_tx, entered) = std::sync::mpsc::channel::<()>();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let released = parking_lot::Mutex::new(released);
    journal::CHECKPOINT_STAGE_OBSERVERS.lock().push((
        directory.path().to_owned(),
        Box::new(move |stage| {
            if stage == "before_ack" {
                entered_tx.send(()).unwrap();
                released.lock().recv().unwrap();
            }
        }),
    ));
    let budget = Arc::new(JournalBudget::new(JOURNAL_QUEUE_BYTES));
    let (work_tx, work_rx) = journal_work_channel();
    let tick = manifest.tick + 1;
    let journal = {
        let root = directory.path().to_owned();
        let store = Arc::clone(&store);
        let budget = Arc::clone(&budget);
        std::thread::spawn(move || {
            journal::journal_loop(
                &root,
                store,
                work_rx,
                JournalWriter::open_test(&root, 1).unwrap(),
                &budget,
                manifest,
                crate::plugin::PluginState::default(),
            );
        })
    };
    state.tick = tick;
    send_append(&work_tx, &budget, tick, Vec::new(), None).unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    assert!(start_checkpoint(&mut state, &work_tx, &done_tx, tick));
    entered.recv().unwrap();
    let pos = crate::boundary::ChunkPos { x: 3, z: -2 };
    assert_eq!(
        state
            .world
            .ensure_blocking(&[(pos, 0)].into_iter().collect()),
        vec![pos]
    );
    state.tick = tick + 1;
    send_tick_journal(&mut state, &work_tx, &budget, tick + 1, Vec::new(), None).unwrap();
    // Only the writer's answer and the close can arrive from here on: every
    // tick-side sender goes away while the hold is still waiting.
    drop(work_tx);
    release.send(()).unwrap();
    join_before_deadline(
        journal,
        "the journal owner kept waiting after the tick closed its queue",
    );
    journal::CHECKPOINT_STAGE_OBSERVERS
        .lock()
        .retain(|(path, _)| path != directory.path());
    assert!(done_rx.recv().unwrap().is_some());
}

/// A refused baseline must release the owner's hold from the writer's own
/// answer, with no further tick work and the queue still open.
#[test]
fn refused_baseline_releases_the_hold_without_further_tick_work() {
    bounded(
        "refused_baseline_releases_the_hold_without_further_tick_work",
        refused_baseline_releases_the_hold_without_further_tick_work_body,
    );
}

fn refused_baseline_releases_the_hold_without_further_tick_work_body() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_owned(), 42);
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let mut registry = CoreRegistry::load(None).unwrap();
    crate::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let registry = Arc::new(registry);
    let (mut state, manifest, _) =
        recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
    // A regular dirty capture cannot be accepted as an immutable baseline, so
    // the real native worker refuses it.
    let invalid = state.world.capture_dirty(&Default::default());
    let budget = Arc::new(JournalBudget::new(JOURNAL_QUEUE_BYTES));
    let (work_tx, work_rx) = journal_work_channel();
    let journal = {
        let root = directory.path().to_owned();
        let store = Arc::clone(&store);
        let budget = Arc::clone(&budget);
        std::thread::spawn(move || {
            journal::journal_loop(
                &root,
                store,
                work_rx,
                JournalWriter::open_test(&root, 1).unwrap(),
                &budget,
                manifest,
                crate::plugin::PluginState::default(),
            );
        })
    };
    work_tx
        .send(JournalWork::RequireGenerated {
            capture: Box::new(invalid),
            wake: work_tx.clone(),
        })
        .unwrap();
    join_before_deadline(
        journal,
        "the refused baseline never released the journal owner's hold",
    );
    assert!(matches!(
        work_tx.try_send(JournalWork::Shutdown),
        Err(std::sync::mpsc::TrySendError::Disconnected(_))
    ));
    assert!(
        std::fs::read_dir(directory.path().join("journal"))
            .unwrap()
            .all(|entry| entry
                .unwrap()
                .path()
                .extension()
                .is_none_or(|ext| ext != "bin")),
        "a refused baseline must not publish dependent WAL"
    );
}

/// Two baselines queued back-to-back behind one held checkpoint: the writer
/// answers the second batch while the first hold is still ending, so the owner
/// must keep that early wake for the next hold instead of dropping it.
#[test]
fn two_baselines_behind_one_held_checkpoint_keep_their_own_answers() {
    bounded(
        "two_baselines_behind_one_held_checkpoint_keep_their_own_answers",
        two_baselines_behind_one_held_checkpoint_keep_their_own_answers_body,
    );
}

fn two_baselines_behind_one_held_checkpoint_keep_their_own_answers_body() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_owned(), 42);
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let mut registry = CoreRegistry::load(None).unwrap();
    crate::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let registry = Arc::new(registry);
    let (mut state, manifest, _) =
        recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
    let (entered_tx, entered) = std::sync::mpsc::channel::<()>();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let released = parking_lot::Mutex::new(released);
    journal::CHECKPOINT_STAGE_OBSERVERS.lock().push((
        directory.path().to_owned(),
        Box::new(move |stage| {
            if stage == "before_ack" {
                entered_tx.send(()).unwrap();
                released.lock().recv().unwrap();
            }
        }),
    ));
    let budget = Arc::new(JournalBudget::new(JOURNAL_QUEUE_BYTES));
    let (work_tx, work_rx) = journal_work_channel();
    let tick = manifest.tick + 1;
    let journal = {
        let root = directory.path().to_owned();
        let store = Arc::clone(&store);
        let budget = Arc::clone(&budget);
        std::thread::spawn(move || {
            journal::journal_loop(
                &root,
                store,
                work_rx,
                JournalWriter::open_test(&root, 1).unwrap(),
                &budget,
                manifest,
                crate::plugin::PluginState::default(),
            );
        })
    };
    state.tick = tick;
    send_append(&work_tx, &budget, tick, Vec::new(), None).unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    assert!(start_checkpoint(&mut state, &work_tx, &done_tx, tick));
    entered.recv().unwrap();
    let first = crate::boundary::ChunkPos { x: 3, z: -2 };
    let second = crate::boundary::ChunkPos { x: 4, z: -2 };
    let source = state.world.chunk_source();
    let (observed_tx, observed) = std::sync::mpsc::channel::<(u64, bool, bool)>();
    *crate::core::JOURNAL_APPEND_OBSERVER.lock().unwrap() = Some((
        directory.path().to_owned(),
        Box::new(move |tick, _| {
            let _ = observed_tx.send((
                tick,
                source.load_native(first).unwrap().is_some(),
                source.load_native(second).unwrap().is_some(),
            ));
        }),
    ));
    // One handoff per tick: the second baseline is only queued after the first
    // tick's append, while the same checkpoint still holds the writer.
    assert_eq!(
        state
            .world
            .ensure_blocking(&[(first, 0)].into_iter().collect()),
        vec![first]
    );
    state.tick = tick + 1;
    send_tick_journal(&mut state, &work_tx, &budget, tick + 1, Vec::new(), None).unwrap();
    assert_eq!(
        state
            .world
            .ensure_blocking(&[(second, 0)].into_iter().collect()),
        vec![second]
    );
    state.tick = tick + 2;
    send_tick_journal(&mut state, &work_tx, &budget, tick + 2, Vec::new(), None).unwrap();
    release.send(()).unwrap();
    work_tx.send(JournalWork::Shutdown).unwrap();
    join_before_deadline(
        journal,
        "two baselines behind one held checkpoint never released their holds",
    );
    journal::CHECKPOINT_STAGE_OBSERVERS
        .lock()
        .retain(|(path, _)| path != directory.path());
    assert!(done_rx.recv().unwrap().is_some());
    *crate::core::JOURNAL_APPEND_OBSERVER.lock().unwrap() = None;
    // Each tick's append publishes only after its own baseline is durable.
    assert_eq!(
        observed.try_iter().collect::<Vec<_>>(),
        vec![(tick + 1, true, false), (tick + 2, true, true)],
        "each tick's append must publish after its own generated baseline"
    );
}

/// The writer dies on the checkpoint the baseline waits behind: the queued
/// baseline job must be dropped with its answer closed and the owner must halt,
/// with no tick work needed and nothing published.
#[test]
fn writer_death_before_a_queued_baseline_halts_without_tick_work() {
    bounded(
        "writer_death_before_a_queued_baseline_halts_without_tick_work",
        writer_death_before_a_queued_baseline_halts_without_tick_work_body,
    );
}

fn writer_death_before_a_queued_baseline_halts_without_tick_work_body() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_owned(), 42);
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let mut registry = CoreRegistry::load(None).unwrap();
    crate::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let registry = Arc::new(registry);
    let (mut state, manifest, _) =
        recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
    let (entered_tx, entered) = std::sync::mpsc::channel::<()>();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let released = parking_lot::Mutex::new(released);
    journal::CHECKPOINT_STAGE_OBSERVERS.lock().push((
        directory.path().to_owned(),
        Box::new(move |stage| {
            if stage == "before_ack" {
                entered_tx.send(()).unwrap();
                released.lock().recv().unwrap();
                panic!("injected checkpoint writer failure");
            }
        }),
    ));
    let budget = Arc::new(JournalBudget::new(JOURNAL_QUEUE_BYTES));
    let (work_tx, work_rx) = journal_work_channel();
    let tick = manifest.tick + 1;
    let journal = {
        let root = directory.path().to_owned();
        let store = Arc::clone(&store);
        let budget = Arc::clone(&budget);
        std::thread::spawn(move || {
            journal::journal_loop(
                &root,
                store,
                work_rx,
                JournalWriter::open_test(&root, 1).unwrap(),
                &budget,
                manifest,
                crate::plugin::PluginState::default(),
            );
        })
    };
    state.tick = tick;
    send_append(&work_tx, &budget, tick, Vec::new(), None).unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    assert!(start_checkpoint(&mut state, &work_tx, &done_tx, tick));
    entered.recv().unwrap();
    let pos = crate::boundary::ChunkPos { x: 3, z: -2 };
    assert_eq!(
        state
            .world
            .ensure_blocking(&[(pos, 0)].into_iter().collect()),
        vec![pos]
    );
    state.tick = tick + 1;
    let (observed_tx, observed) = std::sync::mpsc::channel::<u64>();
    *crate::core::JOURNAL_APPEND_OBSERVER.lock().unwrap() = Some((
        directory.path().to_owned(),
        Box::new(move |tick, _| {
            let _ = observed_tx.send(tick);
        }),
    ));
    send_tick_journal(&mut state, &work_tx, &budget, tick + 1, Vec::new(), None).unwrap();
    // No further tick work: only the writer's own failure can end the hold.
    release.send(()).unwrap();
    join_before_deadline(
        journal,
        "a queued baseline kept the owner waiting after its writer died",
    );
    journal::CHECKPOINT_STAGE_OBSERVERS
        .lock()
        .retain(|(path, _)| path != directory.path());
    assert!(done_rx.recv().unwrap().is_none());
    assert!(matches!(
        work_tx.try_send(JournalWork::Shutdown),
        Err(std::sync::mpsc::TrySendError::Disconnected(_))
    ));
    // The pre-checkpoint append is published by design; the dependent batch
    // that waited for the dead baseline must never be.
    let published: Vec<u64> = observed.try_iter().collect();
    *crate::core::JOURNAL_APPEND_OBSERVER.lock().unwrap() = None;
    assert!(
        !published.contains(&(tick + 1)),
        "the dependent WAL must be withheld after its baseline writer died"
    );
}

/// More items than the owner can hold: past `JOURNAL_HOLD_LIMIT` the original
/// blocking wait applies, and once the baseline is durable every admitted batch
/// publishes exactly once in arrival order.
#[test]
fn more_than_the_hold_limit_publishes_every_batch_in_order() {
    bounded(
        "more_than_the_hold_limit_publishes_every_batch_in_order",
        more_than_the_hold_limit_publishes_every_batch_in_order_body,
    );
}

fn more_than_the_hold_limit_publishes_every_batch_in_order_body() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_owned(), 42);
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let mut registry = CoreRegistry::load(None).unwrap();
    crate::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
    let registry = Arc::new(registry);
    let (mut state, manifest, _) =
        recover_tick_state(&config, &registry, &store, false, None, None).unwrap();
    let (entered_tx, entered) = std::sync::mpsc::channel::<()>();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let released = parking_lot::Mutex::new(released);
    journal::CHECKPOINT_STAGE_OBSERVERS.lock().push((
        directory.path().to_owned(),
        Box::new(move |stage| {
            if stage == "before_ack" {
                entered_tx.send(()).unwrap();
                released.lock().recv().unwrap();
            }
        }),
    ));
    let budget = Arc::new(JournalBudget::new(JOURNAL_QUEUE_BYTES));
    let (work_tx, work_rx) = journal_work_channel();
    let tick = manifest.tick + 1;
    let journal = {
        let root = directory.path().to_owned();
        let store = Arc::clone(&store);
        let budget = Arc::clone(&budget);
        std::thread::spawn(move || {
            journal::journal_loop(
                &root,
                store,
                work_rx,
                JournalWriter::open_test(&root, 1).unwrap(),
                &budget,
                manifest,
                crate::plugin::PluginState::default(),
            );
        })
    };
    state.tick = tick;
    send_append(&work_tx, &budget, tick, Vec::new(), None).unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    assert!(start_checkpoint(&mut state, &work_tx, &done_tx, tick));
    entered.recv().unwrap();
    let pos = crate::boundary::ChunkPos { x: 3, z: -2 };
    assert_eq!(
        state
            .world
            .ensure_blocking(&[(pos, 0)].into_iter().collect()),
        vec![pos]
    );
    state.tick = tick + 1;
    let (observed_tx, observed) = std::sync::mpsc::channel::<u64>();
    *crate::core::JOURNAL_APPEND_OBSERVER.lock().unwrap() = Some((
        directory.path().to_owned(),
        Box::new(move |tick, _| {
            let _ = observed_tx.send(tick);
        }),
    ));
    send_tick_journal(&mut state, &work_tx, &budget, tick + 1, Vec::new(), None).unwrap();
    // Exactly queue plus hold minus one, so the dependent append fills the last
    // slot: the owner is then parked on the answer at the item limit, and one
    // more send must report the bounded queue rather than an unbounded hold.
    let extra = (journal::JOURNAL_HOLD_LIMIT + JOURNAL_WORK_QUEUE - 1) as u64;
    let (sent_tx, sent) = std::sync::mpsc::channel();
    let sender = {
        let work_tx = work_tx.clone();
        let budget = Arc::clone(&budget);
        std::thread::spawn(move || {
            for offset in 0..extra {
                send_append(&work_tx, &budget, tick + 2 + offset, Vec::new(), None).unwrap();
            }
            let _ = sent_tx.send(());
        })
    };
    sent.recv_timeout(std::time::Duration::from_secs(20))
        .expect("admission must not park below queue plus hold");
    sender.join().unwrap();
    assert!(
        matches!(
            work_tx.try_send(JournalWork::Append {
                tick_lo: tick + 2 + extra,
                tick_hi: tick + 2 + extra,
                entries: Vec::new(),
                plugin_ack: None,
                reserved: 0,
            }),
            Err(std::sync::mpsc::TrySendError::Full(_))
        ),
        "past queue plus hold the owner must fall back to the bounded queue"
    );
    release.send(()).unwrap();
    work_tx.send(JournalWork::Shutdown).unwrap();
    join_before(
        journal,
        std::time::Duration::from_secs(30),
        "the journal owner never published the over-limit batches",
    );
    journal::CHECKPOINT_STAGE_OBSERVERS
        .lock()
        .retain(|(path, _)| path != directory.path());
    *crate::core::JOURNAL_APPEND_OBSERVER.lock().unwrap() = None;
    assert!(done_rx.recv().unwrap().is_some());
    let published: Vec<u64> = observed.try_iter().collect();
    assert_eq!(
        published,
        (tick + 1..=tick + 1 + extra).collect::<Vec<_>>(),
        "every batch above the hold limit publishes once, in arrival order"
    );
}
