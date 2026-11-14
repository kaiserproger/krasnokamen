//! Reproducible item operations over real tick inputs, WAL and native saves.
//! Counts detect loss/duplication; frozen owner/slot maps detect a missing or
//! partially recovered operation even when its global item count is unchanged.
use super::*;
use crate::boundary::{BlockPos, ClickMode, ConnectionId, DigAction, Hand, PlayerInput};
use crate::native_checkpoint::tests::{fixture_store, read_manifest, read_player};
use std::collections::BTreeMap;
use std::sync::{Arc, mpsc};

const PLAYER: uuid::Uuid = uuid::Uuid::from_u128(0x6472696c6c);
const POS: BlockPos = BlockPos {
    x: 34,
    y: 200,
    z: 40,
};
const CHUNK: mc_world::ChunkPos = mc_world::ChunkPos { x: 2, z: 2 };
type Custody = BTreeMap<Box<str>, (u32, i32)>;
type Layout = BTreeMap<Box<str>, OwnedItem>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Owner {
    PlayerSlot(uuid::Uuid, usize),
    PlayerCursor(uuid::Uuid),
    PlayerGrid(uuid::Uuid, usize),
    Container((i32, i32, i32), usize),
    Dropped(uuid::Uuid),
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct OwnedItem {
    item_id: u32,
    count: i32,
    owner: Owner,
}
fn add_item(layout: &mut Layout, stack: &mc_data::ItemStack, owner: Owner) {
    let Some(name) = stack
        .custom_name
        .as_ref()
        .filter(|name| name.starts_with("durable-"))
    else {
        return;
    };
    assert_eq!(stack.count, 1, "individually named token {name}");
    assert!(
        layout
            .insert(
                name.clone(),
                OwnedItem {
                    item_id: stack.item_id,
                    count: i32::from(stack.count),
                    owner
                }
            )
            .is_none(),
        "duplicate token {name}"
    );
}
fn add_player(
    layout: &mut Layout,
    inventory: &[mc_data::ItemStack],
    carried: &mc_data::ItemStack,
    grid: &[mc_data::ItemStack],
) {
    for (slot, stack) in inventory.iter().enumerate() {
        add_item(layout, stack, Owner::PlayerSlot(PLAYER, slot));
    }
    add_item(layout, carried, Owner::PlayerCursor(PLAYER));
    for (slot, stack) in grid.iter().enumerate() {
        add_item(layout, stack, Owner::PlayerGrid(PLAYER, slot));
    }
}
fn counts(layout: &Layout) -> Custody {
    layout
        .iter()
        .map(|(name, item)| (name.clone(), (item.item_id, item.count)))
        .collect()
}
fn chest_layout(layout: &mut Layout, slots: &[mc_data::ItemStack]) {
    for (slot, stack) in slots.iter().enumerate() {
        add_item(layout, stack, Owner::Container((POS.x, POS.y, POS.z), slot));
    }
}

struct Batch {
    tick: u64,
    entries: Vec<JournalEntry>,
}
impl Batch {
    fn frame_len(&self) -> usize {
        32 + self
            .entries
            .iter()
            .map(|entry| serde_json::to_vec(entry).unwrap().len() + 1)
            .sum::<usize>()
    }
    fn work(self, ack: Option<mpsc::Sender<u64>>) -> JournalWork {
        JournalWork::Append {
            tick_lo: self.tick,
            tick_hi: self.tick,
            entries: self.entries,
            plugin_ack: ack,
            reserved: 0,
        }
    }
}
struct Outcome {
    before: Layout,
    after: Layout,
    attempted: bool,
    faulted: bool,
    durable_tick: u64,
}
impl Outcome {
    fn mutated(&self) -> bool {
        self.before != self.after
    }
    fn recovered(&self) -> &Layout {
        if self.faulted {
            &self.before
        } else {
            &self.after
        }
    }
}

struct Drill {
    config: CoreConfig,
    state: TickState,
    writer: Option<JournalWriter>,
    manifest: CheckpointManifest,
}
impl Drill {
    fn open(config: CoreConfig) -> Self {
        let state = recovered_world(&config);
        let manifest = read_manifest(&config).unwrap();
        let writer =
            JournalWriter::open_recovered(&config.world_dir, 1, manifest.tick, state.tick).unwrap();
        Self {
            config,
            state,
            writer: Some(writer),
            manifest,
        }
    }
    fn hydrate(&mut self) {
        self.state
            .world
            .ensure_blocking(&[(CHUNK, 0)].into_iter().collect());
        crate::tick::sync_chunk_block_entities(&mut self.state);
    }
    fn capture_tick(&mut self, inputs: &[PlayerInput]) -> Batch {
        let (output, _, _) = crate::tick::run_tick(&mut self.state, inputs);
        Batch {
            tick: output.tick,
            entries: journal_entries_for_tick(&mut self.state, &output),
        }
    }
    fn append(&mut self, batch: &Batch) -> Result<(), CoreError> {
        self.writer
            .as_mut()
            .expect("journal owner available")
            .append(batch.tick, batch.tick, &batch.entries)
    }
    fn run(&mut self, inputs: &[PlayerInput]) -> Result<(), CoreError> {
        let batch = self.capture_tick(inputs);
        self.append(&batch)
    }
    fn join(&mut self) {
        if self.state.connections.entity_of(ConnectionId(1)).is_none() {
            self.run(&[PlayerInput::Join {
                connection: ConnectionId(1),
                uuid: PLAYER,
                name: "durable".into(),
                permission_level: 0,
            }])
            .unwrap();
        }
        let id = self.state.connections.entity_of(ConnectionId(1)).unwrap();
        let slot = self.state.entities.slot_of(id).unwrap();
        self.state.entities.set_pos(slot, [34.5, 200.0, 42.5]);
        self.state.entities.set_ground(slot, true);
    }
    fn open_chest(&mut self) -> Option<(EntityId, u8, i32)> {
        self.join();
        self.hydrate();
        self.run(&[PlayerInput::UseItemOn {
            connection: ConnectionId(1),
            position: POS,
            hand: Hand::Main,
            face: 1,
            cursor: [0.5; 3],
            sequence: 1,
        }])
        .unwrap();
        let id = self.state.connections.entity_of(ConnectionId(1)).unwrap();
        let menu = self.state.players[&id].open.as_ref()?;
        Some((id, menu.window, menu.state_id))
    }
    fn capture_checkpoint(
        &mut self,
    ) -> (
        Box<CheckpointJob>,
        mpsc::Receiver<CheckpointAck>,
        mpsc::Receiver<bool>,
    ) {
        let (tx, rx) = journal_work_channel();
        let (done, ack) = mpsc::channel();
        let (reply, reply_rx) = mpsc::channel();
        let tick = self.state.tick;
        assert!(start_checkpoint(&mut self.state, &tx, &done, tick));
        let JournalWork::Checkpoint(mut job) = rx.recv().unwrap() else {
            panic!("capture")
        };
        job.reply = Some(reply);
        (job, ack, reply_rx)
    }
    /// The actual journal queue owns the WAL gate and dispatches to its native
    /// writer thread, which owns the commit, ACK and pruning. No hand-wired gate.
    fn dispatch_checkpoint(
        &mut self,
        job: Box<CheckpointJob>,
        append: Option<Batch>,
        tail: Option<(Batch, mpsc::Sender<u64>)>,
    ) {
        let (tx, rx) = journal_work_channel();
        if let Some(batch) = append {
            tx.send(batch.work(None)).unwrap();
        }
        tx.send(JournalWork::Checkpoint(job)).unwrap();
        if let Some((batch, ack)) = tail {
            tx.send(batch.work(Some(ack))).unwrap();
        }
        tx.send(JournalWork::Shutdown).unwrap();
        drop(tx);
        journal_loop(
            &self.config.world_dir,
            fixture_store(&self.config).unwrap(),
            rx,
            self.writer.take().unwrap(),
            &std::sync::atomic::AtomicBool::new(false),
            &crate::journal_budget::JournalBudget::new(crate::journal_budget::JOURNAL_QUEUE_BYTES),
            self.manifest.clone(),
            self.state.plugins.clone(),
        );
        self.manifest = read_manifest(&self.config).unwrap();
    }
    fn checkpoint(&mut self) {
        let (job, ack, reply) = self.capture_checkpoint();
        let tick = job.tick;
        self.dispatch_checkpoint(job, None, None);
        let ack = ack.recv().unwrap();
        assert_eq!(ack.as_ref().map(|ack| ack.0), Some(tick));
        assert!(reply.recv().unwrap());
        acknowledge_checkpoint(&mut self.state, ack);
        assert_eq!(self.manifest.tick, tick);
        self.writer = Some(
            JournalWriter::open_recovered(&self.config.world_dir, 1, tick, self.state.tick)
                .unwrap(),
        );
    }
    fn crash(self) -> Self {
        let config = self.config.clone();
        drop(self); // No shutdown or final checkpoint.
        let mut next = Self::open(config);
        next.hydrate();
        next
    }
    fn layout(&self) -> Layout {
        let mut layout = Layout::new();
        for player in self.state.players.values() {
            add_player(
                &mut layout,
                &player.inventory.slots,
                &player.inventory_menu.carried,
                &player.inventory_menu.grid,
            );
            if let Some(menu) = &player.open {
                add_item(&mut layout, &menu.carried, Owner::PlayerCursor(PLAYER));
                for (slot, stack) in menu.grid.iter().enumerate() {
                    add_item(&mut layout, stack, Owner::PlayerGrid(PLAYER, slot));
                }
            }
        }
        for player in self.state.offline_players.values() {
            add_player(
                &mut layout,
                &player.inventory,
                &player.carried,
                &player.grid,
            );
        }
        for (cell, container) in &self.state.block_entities {
            for (slot, stack) in container.slots.iter().enumerate() {
                add_item(&mut layout, stack, Owner::Container(*cell, slot));
            }
        }
        for slot in self.state.entities.item_slots() {
            let row = self.state.entities.row(slot);
            if let Some(item) = &row.item {
                add_item(&mut layout, &item.full_stack(), Owner::Dropped(row.uuid));
            }
        }
        layout
    }
    fn custody(&self) -> Custody {
        counts(&self.layout())
    }
    /// Read only checkpoint-owned records, without replaying their WAL.
    fn native_layout(&self) -> Layout {
        let chunk = self
            .state
            .world
            .chunk_source()
            .load_native(CHUNK)
            .unwrap()
            .unwrap();
        let slots: Vec<_> = chunk.chests[&POS]
            .slots
            .iter()
            .map(crate::tick::furnace_slot_to_stack)
            .collect();
        let player = read_player(
            &self.config,
            &PLAYER.to_string(),
            &self.state.registry.items,
        )
        .unwrap();
        let mut layout = Layout::new();
        chest_layout(&mut layout, &slots);
        add_player(
            &mut layout,
            &player.inventory,
            &player.carried,
            &player.grid,
        );
        layout
    }
    fn operation(&mut self, op: usize, loss: Option<usize>) -> Outcome {
        if op == 3 {
            self.join();
            self.hydrate();
            assert!(
                self.state.block_entities[&(POS.x, POS.y, POS.z)]
                    .slots
                    .iter()
                    .any(|stack| stack.custom_name.is_some()),
                "the model must prepare nonempty custody before a break"
            );
            crate::tick::mining_tests::ready_to_finish(&mut self.state, ConnectionId(1), POS);
            return self.apply_operation(
                op,
                None,
                &[PlayerInput::Dig {
                    connection: ConnectionId(1),
                    position: POS,
                    action: DigAction::Finish,
                    sequence: 2,
                }],
                loss,
            );
        }
        let Some((id, window, state_id)) = self.open_chest() else {
            let layout = self.layout();
            return Outcome {
                before: layout.clone(),
                after: layout,
                attempted: false,
                faulted: false,
                durable_tick: self.state.tick,
            };
        };
        let (slot, token) = if op == 1 {
            let inventory = &self.state.players[&id].inventory.slots;
            let index = inventory
                .iter()
                .position(|stack| stack.custom_name.is_some())
                .unwrap_or(0);
            (
                if index < 9 {
                    54 + index
                } else {
                    27 + index - 9
                },
                inventory[index].custom_name.clone(),
            )
        } else {
            let slots = &self.state.block_entities[&(POS.x, POS.y, POS.z)].slots;
            let index = slots
                .iter()
                .position(|stack| stack.custom_name.is_some())
                .unwrap_or(0);
            (index, slots[index].custom_name.clone())
        };
        self.apply_operation(
            op,
            token,
            &[PlayerInput::ContainerClick {
                connection: ConnectionId(1),
                window: if op == 2 {
                    window.wrapping_add(1)
                } else {
                    window
                },
                state_id,
                slot: slot as i16,
                button: 0,
                mode: ClickMode::QuickMove,
            }],
            loss,
        )
    }
    fn apply_operation(
        &mut self,
        op: usize,
        token: Option<Box<str>>,
        inputs: &[PlayerInput],
        loss: Option<usize>,
    ) -> Outcome {
        let before = self.layout();
        crate::journal::WAL_WRITE_FAILURE.set(loss);
        let batch = self.capture_tick(inputs);
        let result = self.append(&batch);
        let faulted = result.is_err();
        if faulted {
            assert!(loss.is_some());
            assert!(result.unwrap_err().to_string().contains("No space left"));
            assert!(
                crate::journal::WAL_WRITE_FAILURE.get().is_none(),
                "fault actually consumed"
            );
        } else {
            assert!(loss.is_none());
        }
        let after = self.layout();
        assert_operation(op, token.as_deref(), &before, &after);
        Outcome {
            before,
            after,
            attempted: true,
            faulted,
            durable_tick: batch.tick - u64::from(faulted),
        }
    }
}

fn assert_operation(op: usize, token: Option<&str>, before: &Layout, after: &Layout) {
    assert_eq!(counts(before), counts(after));
    if op == 2 || (token.is_none() && op != 3) {
        assert_eq!(
            before, after,
            "refused/empty click changes no owner or slot"
        );
        return;
    }
    let mut expected = before.clone();
    if op == 3 {
        let mut dropped = 0;
        for (name, item) in before {
            if matches!(&item.owner, Owner::Container(cell, _) if *cell == (POS.x, POS.y, POS.z)) {
                let new = &after[name];
                assert!(
                    matches!(&new.owner, Owner::Dropped(_)),
                    "break must spawn the container's token, not just conserve its count"
                );
                expected.get_mut(name).unwrap().owner = new.owner.clone();
                dropped += 1;
            }
        }
        assert!(dropped > 0, "break exercises nonempty custody");
    } else {
        let token = token.unwrap();
        let old = &before[token];
        let new = &after[token];
        match (&old.owner, &new.owner, op) {
            (Owner::Container(cell, _), Owner::PlayerSlot(uuid, slot), 0) => {
                assert_eq!(*cell, (POS.x, POS.y, POS.z));
                assert_eq!(*uuid, PLAYER);
                assert!(*slot < 36);
            }
            (Owner::PlayerSlot(uuid, _), Owner::Container(cell, slot), 1) => {
                assert_eq!(*uuid, PLAYER);
                assert_eq!(*cell, (POS.x, POS.y, POS.z));
                assert!(*slot < 27);
            }
            owners => {
                panic!("selected token {token} did not make the requested transfer: {owners:?}")
            }
        }
        expected.get_mut(token).unwrap().owner = new.owner.clone();
    }
    assert_eq!(
        &expected, after,
        "only the requested tokens may change owner/slot"
    );
}

fn fixture(directory: &std::path::Path) -> Drill {
    let mut config = CoreConfig::new(directory.to_path_buf(), 42);
    config.view_distance = 0;
    config.simulation_distance = 0;
    let mut drill = Drill::open(config);
    drill.join();
    drill.hydrate();
    let chest = crate::crafter::tests::block(&drill.state, "chest", &[]);
    drill.state.world.set_block(POS.x, POS.y, POS.z, chest);
    drill.state.world.fold_overlay();
    let item = drill
        .state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
        .unwrap();
    let mut slots = vec![mc_data::ItemStack::EMPTY; 27];
    for (index, stack) in slots.iter_mut().take(6).enumerate() {
        *stack = mc_data::ItemStack::new(item, 1);
        stack.custom_name = Some(format!("durable-{index}").into());
    }
    drill.state.block_entities.insert(
        (POS.x, POS.y, POS.z),
        crate::tick::BlockEntityState {
            slots,
            furnace: None,
        },
    );
    drill.run(&[]).unwrap();
    drill.checkpoint();
    drill
}

/// Checkpoint the preparation so the only tail frame is a genuine container
/// transfer, frozen independently before handing it to a WAL owner.
fn prepared_transfer(directory: &std::path::Path) -> (Drill, Batch, Layout, Layout) {
    let mut drill = fixture(directory);
    let (id, _, _) = drill.open_chest().expect("fixture chest opens");
    drill.checkpoint();
    let menu = drill.state.players[&id].open.as_ref().unwrap();
    let (window, state_id) = (menu.window, menu.state_id);
    let before = drill.layout();
    let token = drill.state.block_entities[&(POS.x, POS.y, POS.z)].slots[0]
        .custom_name
        .clone()
        .unwrap();
    let batch = drill.capture_tick(&[PlayerInput::ContainerClick {
        connection: ConnectionId(1),
        window,
        state_id,
        slot: 0,
        button: 0,
        mode: ClickMode::QuickMove,
    }]);
    let after = drill.layout();
    assert_operation(0, Some(&token), &before, &after);
    assert_ne!(before, after);
    (drill, batch, before, after)
}

#[derive(Default, Debug)]
struct Coverage {
    attempted: [u64; 4],
    mutated: [u64; 4],
    faulted_mutations: [u64; 4],
    append_faults: u64,
    checkpoints: u64,
    restarts: u64,
    durable_restarts: u64,
}
impl Coverage {
    fn record(&mut self, op: usize, outcome: &Outcome) {
        self.attempted[op] += u64::from(outcome.attempted);
        self.mutated[op] += u64::from(outcome.mutated());
        self.append_faults += u64::from(outcome.faulted);
        self.faulted_mutations[op] += u64::from(outcome.faulted && outcome.mutated());
    }
}
#[test]
fn randomized_item_conservation_across_wal_loss_and_checkpoints() {
    let mut coverage = Coverage::default();
    for seed in [1_u64, 7, 42, 0xdead_beef] {
        let directory = tempfile::tempdir().unwrap();
        let mut drill = fixture(directory.path());
        coverage.checkpoints += 1; // fixture() completed a real queued checkpoint.
        let expected = drill.custody();
        assert_eq!(expected.len(), 6);
        let mut random = seed;
        for step in 0..32 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let op = match step {
                0 | 3 | 4 => 0,
                1 | 5 => 1,
                2 => 2,
                24 | 29 => 3,
                _ => ((random >> 32) % 3) as usize,
            };
            let cut = [0, 35, usize::MAX, random as usize % 4096][step % 4];
            let loss = (step == 5 || (step > 2 && step != 4 && step % 3 != 2)).then_some(cut);
            if op == 3
                && !drill.state.block_entities[&(POS.x, POS.y, POS.z)]
                    .slots
                    .iter()
                    .any(|stack| stack.custom_name.is_some())
            {
                let preparation = drill.operation(1, None);
                assert!(preparation.mutated());
                coverage.record(1, &preparation);
            }
            let outcome = drill.operation(op, loss);
            coverage.record(op, &outcome);
            assert_eq!(drill.custody(), expected, "seed {seed} step {step}");
            if !outcome.faulted && random % 5 == 0 {
                drill.checkpoint();
                coverage.checkpoints += 1;
            }
            drill = drill.crash();
            coverage.restarts += 1;
            coverage.durable_restarts += u64::from(!outcome.faulted);
            assert_eq!(drill.state.tick, outcome.durable_tick);
            assert_eq!(
                &drill.layout(),
                outcome.recovered(),
                "complete old/new owners and slots, seed {seed} step {step}"
            );
            assert_eq!(drill.custody(), expected);
        }
        let final_layout = drill.layout();
        drill.checkpoint();
        coverage.checkpoints += 1;
        drill = drill.crash();
        coverage.restarts += 1;
        coverage.durable_restarts += 1;
        assert_eq!(
            drill.layout(),
            final_layout,
            "recovery after a NEW checkpoint and WAL pruning"
        );
    }
    assert!(
        coverage.attempted.iter().all(|count| *count > 0),
        "{coverage:?}"
    );
    assert!(coverage.mutated[0] > 0 && coverage.mutated[1] > 0 && coverage.mutated[3] > 0);
    assert_eq!(coverage.mutated[2], 0);
    assert!(
        coverage.faulted_mutations[0] > 0
            && coverage.faulted_mutations[1] > 0
            && coverage.faulted_mutations[3] > 0,
        "{coverage:?}"
    );
    assert!(
        coverage.append_faults > 0 && coverage.durable_restarts > 0 && coverage.checkpoints > 0
    );
    assert_eq!(
        coverage.restarts,
        coverage.append_faults + coverage.durable_restarts
    );
    eprintln!("actual operation/fault/restart coverage: {coverage:?}");
}

fn replay_transfer_layout(
    config: &CoreConfig,
    registry: &Arc<crate::registry::CoreRegistry>,
    store: &Arc<mc_world::native::NativeStore>,
    base_slots: &[mc_data::ItemStack],
) -> (u64, Layout) {
    let recovered = crate::native_checkpoint::recover_with_reader(
        config,
        Arc::clone(registry),
        store.checkpoint_reader().unwrap(),
    )
    .unwrap();
    let replay = &recovered.dimensions[&Dimension::Overworld];
    let player = &replay.players[&PLAYER.to_string()];
    let slots = replay
        .block_entities
        .get(&(POS.x, POS.y, POS.z))
        .map_or(base_slots, |container| &container.slots);
    let mut layout = Layout::new();
    add_player(
        &mut layout,
        &player.inventory,
        &player.carried,
        &player.grid,
    );
    chest_layout(&mut layout, slots);
    (replay.tick, layout)
}

#[test]
fn enospc_at_every_transfer_wal_byte_refuses_partial_custody() {
    let directory = tempfile::tempdir().unwrap();
    let (drill, batch, before, after) = prepared_transfer(directory.path());
    let config = drill.config.clone();
    let registry = Arc::clone(&drill.state.registry);
    let store = fixture_store(&config).unwrap();
    let chunk = drill
        .state
        .world
        .chunk_source()
        .load_native(CHUNK)
        .unwrap()
        .unwrap();
    let base_slots: Vec<_> = chunk.chests[&POS]
        .slots
        .iter()
        .map(crate::tick::furnace_slot_to_stack)
        .collect();
    let previous_tick = drill.manifest.tick;
    assert_eq!(batch.tick, previous_tick + 1);
    assert_eq!(drill.native_layout(), before);
    let bytes = batch.frame_len();
    drop(drill);
    let mut actual_faults = 0;
    for cut in 0..=bytes {
        let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
        crate::journal::WAL_WRITE_FAILURE.set(Some(cut));
        let error = writer
            .append(batch.tick, batch.tick, &batch.entries)
            .unwrap_err();
        assert!(crate::journal::WAL_WRITE_FAILURE.get().is_none());
        assert!(
            error.to_string().contains("No space left"),
            "byte {cut}: {error}"
        );
        actual_faults += 1;
        assert!(writer.gate(previous_tick, batch.tick).is_err());
        drop(writer);
        // Production WAL fold against an unchanged pinned native checkpoint.
        // This is not represented as a fresh Core process at every byte.
        let (tick, layout) = replay_transfer_layout(&config, &registry, &store, &base_slots);
        assert_eq!(tick, previous_tick);
        assert_eq!(
            layout, before,
            "transfer frame byte {cut}: exact old owners/slots"
        );
    }
    assert_eq!(actual_faults, bytes + 1);
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    writer
        .append(batch.tick, batch.tick, &batch.entries)
        .unwrap();
    drop(writer);
    let (tick, layout) = replay_transfer_layout(&config, &registry, &store, &base_slots);
    assert_eq!(tick, batch.tick);
    assert_eq!(layout, after);
    drop(store);
    let mut recovered = Drill::open(config);
    recovered.hydrate();
    assert_eq!(recovered.layout(), after);
    recovered.checkpoint();
    recovered = recovered.crash();
    assert_eq!(
        recovered.layout(),
        after,
        "new checkpoint/prune, then a new recovery"
    );
    eprintln!("actual ENOSPC transfer-frame faults: {actual_faults}");
}

#[test]
fn enospc_transfer_wal_halts_admission_without_ack_or_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let (mut drill, batch, before, _) = prepared_transfer(directory.path());
    let config = drill.config.clone();
    let previous_tick = drill.manifest.tick;
    let (job, checkpoint_ack, checkpoint_reply) = drill.capture_checkpoint();
    let (tx, rx) = journal_work_channel();
    let (append_ack, ack_rx) = mpsc::channel();
    tx.send(batch.work(Some(append_ack))).unwrap();
    tx.send(JournalWork::Checkpoint(job)).unwrap();
    crate::journal::WAL_WRITE_FAILURE.set(Some(35));
    journal_loop(
        &config.world_dir,
        fixture_store(&config).unwrap(),
        rx,
        drill.writer.take().unwrap(),
        &std::sync::atomic::AtomicBool::new(false),
        &crate::journal_budget::JournalBudget::new(crate::journal_budget::JOURNAL_QUEUE_BYTES),
        drill.manifest.clone(),
        drill.state.plugins.clone(),
    );
    assert!(
        crate::journal::WAL_WRITE_FAILURE.get().is_none(),
        "actual append fault"
    );
    assert!(matches!(
        ack_rx.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
    assert!(matches!(
        checkpoint_ack.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
    assert!(matches!(
        checkpoint_reply.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
    assert!(tx.send(JournalWork::Shutdown).is_err());
    assert_eq!(read_manifest(&config).unwrap().tick, previous_tick);
    assert_eq!(drill.native_layout(), before);
    let recovered = drill.crash();
    assert_eq!(recovered.state.tick, previous_tick);
    assert_eq!(recovered.layout(), before);
}

#[path = "native_checkpoint_pipeline_drill_tests.rs"]
mod checkpoint_pipeline;

#[test]
fn abrupt_stop_at_each_wal_publication_stage_conserves_transferred_items() {
    const CHILD: &str = "KRASNOKAMEN_ITEM_WAL_DRILL_PATH";
    const STAGES: [&str; 5] = ["created", "written", "synced", "published", "durable"];
    if let Some(path) = std::env::var_os(CHILD) {
        let mut config = CoreConfig::new(path.into(), 42);
        config.view_distance = 0;
        config.simulation_distance = 0;
        let mut drill = Drill::open(config);
        let stage = std::env::var("KRASNOKAMEN_ITEM_WAL_DRILL_STAGE").unwrap();
        let stage = STAGES.into_iter().find(|value| *value == stage).unwrap();
        // The parent checkpointed an open chest, but runtime menus do not survive
        // recovery. Reopen first, then checkpoint preparation before the transfer.
        let (id, _, _) = drill.open_chest().unwrap();
        drill.checkpoint();
        let menu = drill.state.players[&id].open.as_ref().unwrap();
        let (window, state_id) = (menu.window, menu.state_id);
        let batch = drill.capture_tick(&[PlayerInput::ContainerClick {
            connection: ConnectionId(1),
            window,
            state_id,
            slot: 0,
            button: 0,
            mode: ClickMode::QuickMove,
        }]);
        crate::journal::WAL_CRASH_STAGE.set(Some(stage));
        drill.append(&batch).unwrap();
        panic!("WAL crash hook not reached");
    }
    let mut actual_stops = 0;
    for stage in STAGES {
        let directory = tempfile::tempdir().unwrap();
        let (drill, _, before, after) = prepared_transfer(directory.path());
        let config = drill.config.clone();
        drop(drill);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "core::native_checkpoint_crash_tests::persistence_drills::abrupt_stop_at_each_wal_publication_stage_conserves_transferred_items", "--nocapture"])
            .env(CHILD, directory.path()).env("KRASNOKAMEN_ITEM_WAL_DRILL_STAGE", stage).status().unwrap();
        assert_eq!(status.code(), Some(39), "{stage}");
        actual_stops += 1;
        let published = matches!(stage, "published" | "durable");
        let mut recovered = Drill::open(config.clone());
        recovered.hydrate();
        let checkpoint_tick = recovered.manifest.tick;
        assert_eq!(recovered.state.tick, checkpoint_tick + u64::from(published));
        assert_eq!(
            &recovered.layout(),
            if published { &after } else { &before },
            "{stage}: exact whole operation"
        );
        let final_layout = if stage == "published" {
            drop(recovered);
            let tick = checkpoint_tick + 1;
            std::fs::remove_file(
                directory
                    .path()
                    .join("journal")
                    .join(format!("batch-{tick:020}-{tick:020}.bin")),
            )
            .unwrap();
            recovered = Drill::open(config);
            recovered.hydrate();
            assert_eq!(recovered.state.tick, checkpoint_tick);
            assert_eq!(recovered.layout(), before);
            before
        } else if published {
            after
        } else {
            before
        };
        recovered.checkpoint();
        recovered = recovered.crash();
        assert_eq!(
            recovered.layout(),
            final_layout,
            "{stage}: NEW checkpoint/prune followed by recovery"
        );
    }
    assert_eq!(actual_stops, STAGES.len());
}
