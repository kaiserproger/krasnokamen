//! ADR 0020 oracle: incremental entity checkpoints leave the native store equal
//! to a full capture at every durable fence, and a crash reopens at the WAL fence.
//! Entities sit in unticketed chunks (inactive), so only the test writes them.
//! Identity is checked independently of the capture code: stored and reopened
//! UUID sets must equal the live `EntityStore` identity column.
use super::*;
use crate::dimensions::Dimension;
use crate::entities::EntitySlot;
use crate::native_checkpoint::tests::{fixture_store, read_manifest};
use std::collections::{BTreeMap, BTreeSet};

/// Live entity rows by UUID: dimension id, chunk and encoded payload.
type Image = BTreeMap<uuid::Uuid, (u32, mc_world::ChunkPos, mc_nbt::Tag)>;
type Done = std::sync::mpsc::Receiver<CheckpointAck>;
/// One custody path's write between ticks.
type Mutation = Box<dyn Fn(&mut TickState)>;

fn worlds(state: &TickState) -> impl Iterator<Item = &TickState> {
    std::iter::once(state).chain(state.other_dimensions.values().map(|world| &**world))
}

/// The complete materialized capture of `state`, as the writer would store it.
fn full_image(state: &TickState) -> Image {
    let tick = state.tick;
    let mut image = Image::new();
    for world in worlds(state) {
        let (_, chunks, _) = checkpoint_data_for(world, tick);
        let encoded = crate::native_checkpoint::encode_entities(
            world.dimension,
            &chunks,
            &world.registry.items,
            tick,
        )
        .unwrap();
        for entity in encoded {
            let row = (entity.dimension, entity.position, entity.payload.unwrap());
            let uuid = uuid::Uuid::from_bytes(entity.identity);
            assert!(image.insert(uuid, row).is_none(), "duplicate {uuid}");
        }
    }
    image
}

/// Every live row of the committed native checkpoint.
fn stored_image(config: &CoreConfig) -> Image {
    let mut image = Image::new();
    fixture_store(config)
        .unwrap()
        .checkpoint_reader()
        .unwrap()
        .visit_entities(|entity| {
            if let Some(payload) = entity.payload {
                let row = (entity.dimension, entity.position, payload);
                image.insert(uuid::Uuid::from_bytes(entity.identity), row);
            }
            Ok(())
        })
        .unwrap();
    image
}

/// Non-player identities in every dimension's `EntityStore`, read from the
/// identity column rather than through checkpoint capture.
fn store_uuids(state: &TickState) -> BTreeSet<uuid::Uuid> {
    worlds(state)
        .flat_map(|world| {
            world.entities.slots().filter_map(move |slot| {
                let player = matches!(
                    world.entities.body(slot).kind,
                    crate::entities::EntityKind::Player { .. }
                );
                (!player).then(|| world.entities.row(slot).uuid)
            })
        })
        .collect()
}

fn type_id(state: &TickState, name: &str) -> u32 {
    state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse(name).unwrap())
        .unwrap()
}

fn world_mut(state: &mut TickState, id: EntityId) -> (&mut TickState, EntitySlot) {
    let (dimension, slot) = crate::dimensions::travel::locate(state, id).unwrap();
    if dimension == state.dimension {
        (state, slot)
    } else {
        (state.other_dimensions.get_mut(&dimension).unwrap(), slot)
    }
}

fn slot(state: &TickState, id: EntityId) -> EntitySlot {
    state.entities.slot_of_any(id).unwrap()
}

fn uuid_of(state: &mut TickState, id: EntityId) -> uuid::Uuid {
    let (world, slot) = world_mut(state, id);
    world.entities.row(slot).uuid
}

/// A removal the next tick publishes and journals, as item frames, paintings
/// and copper golems do through `hostile.removed`.
fn remove(state: &mut TickState, id: EntityId) {
    let (world, slot) = world_mut(state, id);
    world.remove_entity(slot);
    world.hostile.removed.push(id);
}

/// A hurt the next tick publishes, so its WAL batch carries the health.
fn hurt(state: &mut TickState, id: EntityId, health: f32) {
    let (world, slot) = world_mut(state, id);
    world.entities.row_mut(slot).health = health;
    world
        .hostile
        .events
        .push(crate::boundary::EntityEvent::Hurt { entity: id });
}

/// A rename the next tick journals as a full post-image under the new UUID.
fn rename(state: &mut TickState, id: EntityId, uuid: uuid::Uuid) {
    let (world, slot) = world_mut(state, id);
    world.entities.set_uuid(slot, uuid);
    world.appearance_changed.push(id);
}

struct World {
    config: CoreConfig,
    state: TickState,
    writer: JournalWriter,
    manifest: CheckpointManifest,
}

impl World {
    fn new(directory: &std::path::Path) -> Self {
        let mut config = CoreConfig::new(directory.to_path_buf(), 42);
        config.view_distance = 0;
        config.simulation_distance = 0;
        Self::open(config)
    }

    /// Recovery from the store and the WAL tail, as after a crash.
    fn open(config: CoreConfig) -> Self {
        let mut state = recover_dimension_state(&config, Dimension::Overworld).unwrap();
        for dimension in [Dimension::Nether, Dimension::End] {
            let world = recover_dimension_state(&config, dimension).unwrap();
            state.other_dimensions.insert(dimension, Box::new(world));
        }
        let manifest = read_manifest(&config).unwrap();
        let writer =
            JournalWriter::open_recovered(&config.world_dir, 1, manifest.tick, state.tick).unwrap();
        Self {
            manifest,
            writer,
            config,
            state,
        }
    }

    /// Kill without a final checkpoint and reopen; the state must equal the
    /// last WAL fence (only journaled changes since the last durable fence).
    fn crash(self) -> Self {
        let expected = full_image(&self.state);
        let identities = store_uuids(&self.state);
        let tick = self.state.tick;
        let config = self.config.clone();
        drop(self);
        let reopened = Self::open(config);
        assert_eq!(reopened.state.tick, tick);
        assert_eq!(
            store_uuids(&reopened.state),
            identities,
            "reopen at WAL fence {tick}: live identities"
        );
        assert_eq!(
            full_image(&reopened.state),
            expected,
            "reopen at WAL fence {tick}"
        );
        reopened
    }

    /// Mutate between ticks, then run one tick and append its WAL batch.
    fn tick(&mut self, mutate: impl FnOnce(&mut TickState)) {
        mutate(&mut self.state);
        self.run(&[]);
    }

    fn run(&mut self, inputs: &[crate::boundary::PlayerInput]) {
        let (output, _, _) = crate::tick::run_tick(&mut self.state, inputs);
        let entries = journal_entries_for_tick(&mut self.state, &output);
        self.writer
            .append(output.tick, output.tick, &entries)
            .unwrap();
    }

    /// A real cross-dimension move in one tick: `execute in … run tp`.
    fn transfer(&mut self, id: EntityId, dimension: Dimension) {
        let uuid = uuid_of(&mut self.state, id);
        let text = format!("execute in {} run tp {uuid} 8.5 70 8.5", dimension.name());
        let (command, mut done) = crate::command_console::ConsoleCommand::new(text);
        self.run(&[crate::boundary::PlayerInput::ConsoleCommand(command)]);
        assert!(done.try_recv().is_ok());
        let (located, _) = crate::dimensions::travel::locate(&self.state, id).unwrap();
        assert_eq!(located, dimension);
    }

    fn spawn(&mut self, name: &str, pos: [f64; 3]) -> EntityId {
        let kind = type_id(&self.state, name);
        self.state.spawn_mob(kind, pos, 20.0)
    }

    fn capture(&mut self) -> (Box<CheckpointJob>, Done) {
        let (work_tx, work_rx) = journal_work_channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let tick = self.state.tick;
        assert!(start_checkpoint(&mut self.state, &work_tx, &done_tx, tick));
        let JournalWork::Checkpoint(job) = work_rx.recv().unwrap() else {
            panic!("expected checkpoint capture");
        };
        (job, done_rx)
    }

    /// The native writer's job: WAL gate, one commit, prune; returns its ACK.
    fn commit(&mut self, job: Box<CheckpointJob>, done: &Done) -> CheckpointAck {
        self.writer.gate(self.manifest.tick, job.tick).unwrap();
        let outcome = run_native_work(
            NativeWork::Checkpoint {
                job,
                plugins: Ok(Box::new(self.state.plugins.clone())),
            },
            &self.config.world_dir,
            &std::sync::atomic::AtomicBool::new(false),
            &std::sync::atomic::AtomicU64::new(self.manifest.tick),
            &mut self.manifest,
            &mut Default::default(),
        );
        assert!(outcome.is_continue());
        done.recv().unwrap()
    }

    /// One durable checkpoint, checked against a full capture of the same state.
    fn checkpoint(&mut self) -> u64 {
        let expected = full_image(&self.state);
        let (job, done) = self.capture();
        let tick = job.tick;
        let ack = self.commit(job, &done);
        assert!(ack.is_some(), "checkpoint {tick} committed");
        acknowledge_checkpoint(&mut self.state, ack);
        self.assert_fence(&expected, tick);
        tick
    }

    fn assert_fence(&self, expected: &Image, tick: u64) {
        assert_eq!(self.manifest.tick, tick);
        let stored = stored_image(&self.config);
        assert_eq!(
            stored.keys().copied().collect::<BTreeSet<_>>(),
            store_uuids(&self.state),
            "durable fence {tick}: stored identities equal the live store"
        );
        assert_eq!(
            &stored, expected,
            "durable fence {tick}: every stored row equals a full capture"
        );
    }

    /// Rows whose stored record differs from a full capture now.
    fn stale(&self) -> BTreeSet<uuid::Uuid> {
        let stored = stored_image(&self.config);
        full_image(&self.state)
            .into_iter()
            .filter(|(uuid, row)| stored.get(uuid) != Some(row))
            .map(|(uuid, _)| uuid)
            .collect()
    }
}

/// UUIDs of every row a capture presents.
fn captured(job: &CheckpointJob) -> BTreeSet<uuid::Uuid> {
    std::iter::once(&job.entities)
        .chain(job.dimensions.values().map(|capture| &capture.entities))
        .flat_map(|chunks| chunks.values())
        .flat_map(|chunk| &chunk.entities)
        .map(|entity| entity.uuid)
        .collect()
}

/// Idle cows, chickens and zombies are never presented again; villagers whose
/// brain clock moved a persisted field (work/age clocks) are, and only they.
#[test]
fn checkpoint_presents_exactly_the_stale_rows() {
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    world.tick(|_| {});
    let names = [
        "minecraft:cow",
        "minecraft:villager",
        "minecraft:chicken",
        "minecraft:zombie",
    ];
    // Every spawned body's identity, recorded as it is created.
    let mut spawned = BTreeSet::new();
    let mut villagers = BTreeSet::new();
    for index in 0..8 {
        let name = names[index as usize % names.len()];
        let id = world.spawn(name, [f64::from(index) * 20.0 + 0.5, 80.0, 0.5]);
        let uuid = uuid_of(&mut world.state, id);
        spawned.insert(uuid);
        if name == "minecraft:villager" {
            villagers.insert(uuid);
        }
    }
    world.checkpoint();
    let stored: BTreeSet<_> = stored_image(&world.config).into_keys().collect();
    assert_eq!(
        stored, spawned,
        "the first stored checkpoint holds every body"
    );
    world.tick(|_| {});
    let (job, done) = world.capture();
    let stale = world.stale();
    assert!(
        stale.is_subset(&villagers),
        "only villager clocks move: {stale:?}"
    );
    assert_eq!(
        captured(&job),
        stale,
        "the capture presents exactly the stale rows"
    );
    assert!(job.tombstones.is_empty());
    let tick = job.tick;
    let expected = full_image(&world.state);
    assert!(world.commit(job, &done).is_some());
    world.assert_fence(&expected, tick);
    // A clean reopen inserts every stored row as unchanged.
    let mut world = world.crash();
    assert_eq!(
        store_uuids(&world.state),
        spawned,
        "reopen restores every body"
    );
    let (job, _) = world.capture();
    assert_eq!(
        captured(&job),
        BTreeSet::new(),
        "reopen alone dirties no row"
    );
    drop(job);
    world.tick(|_| {});
    let (job, _) = world.capture();
    let stale = world.stale();
    assert!(
        stale.is_subset(&villagers),
        "only villager clocks move: {stale:?}"
    );
    assert_eq!(
        captured(&job),
        stale,
        "the first reopened tick presents only stale rows"
    );
}

/// A tail post-image under a new UUID retires the stored identity: the crash
/// checkpoint tombstones it, so no two rows share a wire id at the next reopen.
#[test]
fn tail_rename_tombstones_the_stored_identity() {
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    world.tick(|_| {});
    let cow = world.spawn("minecraft:cow", [0.5, 80.0, 0.5]);
    world.checkpoint();
    let old = uuid_of(&mut world.state, cow);
    let new = uuid::Uuid::from_u128(0x0020_0030);
    world.tick(|state| rename(state, cow, new));
    let mut world = world.crash();
    world.checkpoint();
    let stored = stored_image(&world.config);
    assert!(!stored.contains_key(&old), "renamed identity tombstoned");
    assert!(stored.contains_key(&new));
    let world = world.crash();
    assert_eq!(store_uuids(&world.state), BTreeSet::from([new]));
}

/// Rename then removal in one WAL tail: both identities are logged, so the
/// stored original cannot resurrect after the crash checkpoint and reopen.
#[test]
fn tail_rename_then_removal_cannot_resurrect_the_original() {
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    world.tick(|_| {});
    let cow = world.spawn("minecraft:cow", [0.5, 80.0, 0.5]);
    let kept = world.spawn("minecraft:pig", [20.5, 80.0, 0.5]);
    world.checkpoint();
    let old = uuid_of(&mut world.state, cow);
    let new = uuid::Uuid::from_u128(0x0020_0031);
    world.tick(|state| rename(state, cow, new));
    world.tick(|state| remove(state, cow));
    let mut world = world.crash();
    world.checkpoint();
    let stored = stored_image(&world.config);
    assert!(!stored.contains_key(&old) && !stored.contains_key(&new));
    let mut world = world.crash();
    let kept = uuid_of(&mut world.state, kept);
    assert_eq!(store_uuids(&world.state), BTreeSet::from([kept]));
}

#[test]
fn custody_paths_reach_the_next_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    world.tick(|_| {});
    let cow = world.spawn("minecraft:cow", [0.5, 80.0, 0.5]);
    let chicken = world.spawn("minecraft:chicken", [20.5, 80.0, 0.5]);
    let villager = world.spawn("minecraft:villager", [40.5, 80.0, 0.5]);
    let pig = world.spawn("minecraft:pig", [60.5, 80.0, 0.5]);
    let sheep = world.spawn("minecraft:sheep", [80.5, 80.0, 0.5]);
    let goat = world.spawn("minecraft:goat", [100.5, 80.0, 0.5]);
    world.tick(|state| {
        let slot = slot(state, villager);
        state.entities.row_mut(slot).villager.as_mut().unwrap().age = -1000;
    });
    world.checkpoint();
    let custody: [(&str, Mutation); 12] = [
        (
            "advance_admitted_ages",
            Box::new(move |state| {
                let mut admitted = vec![0; state.entities.len()];
                admitted[slot(state, cow).0 as usize] = 1;
                state
                    .entities
                    .advance_admitted_ages(&admitted, u32::MAX, usize::MAX);
            }),
        ),
        (
            "advance_villager_clocks",
            Box::new(move |state| {
                let slot = slot(state, villager);
                let to = state.tick + 40;
                state.entities.advance_villager_clocks(slot, state.tick, to);
            }),
        ),
        (
            "set_harvest_ticks",
            Box::new(move |state| state.entities.set_harvest_ticks(slot(state, chicken), 77)),
        ),
        (
            "set_motion_state",
            Box::new(move |state| {
                state
                    .entities
                    .set_motion_state(slot(state, pig), [0.0; 2], 123, 0, 0);
            }),
        ),
        (
            "apply_motion_result",
            Box::new(move |state| {
                let result = crate::entities::MotionResult {
                    pos: [61.25, 80.0, 0.5],
                    vel: [0.5, 0.0, 0.0],
                    yaw: 45.0,
                    pitch: 5.0,
                    ground: true,
                    horizontal_impulse: [0.0; 2],
                    air_supply: 300,
                    attack_cooldown: None,
                };
                state
                    .entities
                    .apply_motion_result(slot(state, pig), &result);
            }),
        ),
        (
            "set_pos",
            Box::new(move |state| state.entities.set_pos(slot(state, cow), [1.5, 81.0, 0.5])),
        ),
        (
            "set_vel",
            Box::new(move |state| state.entities.set_vel(slot(state, cow), [0.0, -1.0, 0.0])),
        ),
        (
            "set_rot",
            Box::new(move |state| state.entities.set_rot(slot(state, cow), 90.0, 10.0)),
        ),
        (
            "set_ground",
            Box::new(move |state| {
                let slot = slot(state, sheep);
                let ground = state.entities.ground(slot);
                state.entities.set_ground(slot, !ground);
            }),
        ),
        (
            "row_mut_projected",
            Box::new(move |state| state.entities.row_mut_projected(slot(state, sheep)).age = 900),
        ),
        (
            "set_no_action_time",
            Box::new(move |state| state.entities.set_no_action_time(slot(state, goat), 600)),
        ),
        (
            "set_uuid",
            Box::new(move |state| {
                let slot = slot(state, goat);
                state
                    .entities
                    .set_uuid(slot, uuid::Uuid::from_u128(0x0020_0020));
            }),
        ),
    ];
    let goat_uuid = uuid_of(&mut world.state, goat);
    for (path, mutate) in custody {
        let before = stored_image(&world.config);
        world.tick(|state| mutate(state));
        world.checkpoint();
        assert_ne!(
            stored_image(&world.config),
            before,
            "{path} reached the store"
        );
    }
    let stored = stored_image(&world.config);
    assert!(
        !stored.contains_key(&goat_uuid),
        "set_uuid tombstones the old identity"
    );
    assert!(stored.contains_key(&uuid::Uuid::from_u128(0x0020_0020)));
    // Swap-removal moves the last slot's mark into the removed slot.
    world.tick(|state| {
        let last = state.entities.len() - 1;
        let moved = state.entities.body(EntitySlot(last as u32)).id;
        hurt(state, moved, 3.0);
        remove(state, cow);
        assert_eq!(slot(state, moved), EntitySlot(0));
    });
    world.checkpoint();
    // A marked row transferred out keeps custody in its new dimension.
    world.tick(|state| hurt(state, chicken, 4.0));
    world.transfer(chicken, Dimension::Nether);
    world.checkpoint();
    let stored = stored_image(&world.config);
    let chicken_uuid = uuid_of(&mut world.state, chicken);
    assert_eq!(stored[&chicken_uuid].0, 1, "transfer is an upsert");
}

#[test]
fn capture_failures_and_stale_acks_keep_full_custody() {
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    world.tick(|_| {});
    let ids: Vec<EntityId> = (0..6)
        .map(|index| world.spawn("minecraft:cow", [f64::from(index) * 20.0 + 0.5, 80.0, 0.5]))
        .collect();
    world.checkpoint();

    // Mutation after capture, before its ACK.
    world.tick(|state| hurt(state, ids[0], 1.0));
    let expected = full_image(&world.state);
    let (job, done) = world.capture();
    let tick = job.tick;
    world.tick(|state| {
        state
            .entities
            .set_pos(slot(state, ids[1]), [21.5, 82.0, 0.5])
    });
    let ack = world.commit(job, &done);
    world.assert_fence(&expected, tick);
    acknowledge_checkpoint(&mut world.state, ack);
    world.checkpoint();

    // A capture dropped before handoff writes nothing and clears nothing.
    world.tick(|state| hurt(state, ids[2], 2.0));
    drop(world.capture());
    world.tick(|_| {});
    world.checkpoint();

    // A failed commit with the shutdown checkpoint already queued behind it.
    world.tick(|state| hurt(state, ids[3], 3.0));
    let (failing, failing_done) = world.capture();
    world.tick(|state| remove(state, ids[4]));
    let expected = full_image(&world.state);
    let (shutdown, shutdown_done) = world.capture();
    let tick = shutdown.tick;
    FAIL_CHECKPOINT_AFTER_REGIONS
        .lock()
        .unwrap()
        .push(world.config.world_dir.clone());
    let failed = world.commit(failing, &failing_done);
    assert!(failed.is_none(), "injected commit failure");
    acknowledge_checkpoint(&mut world.state, failed);
    let ack = world.commit(shutdown, &shutdown_done);
    acknowledge_checkpoint(&mut world.state, ack);
    world.assert_fence(&expected, tick);

    // A stale ACK arriving after a newer one clears nothing newer.
    world.tick(|state| hurt(state, ids[5], 5.0));
    let (older, older_done) = world.capture();
    let older_ack = world.commit(older, &older_done);
    world.tick(|state| hurt(state, ids[0], 6.0));
    world.checkpoint();
    world.tick(|state| hurt(state, ids[1], 7.0));
    acknowledge_checkpoint(&mut world.state, older_ack);
    world.tick(|_| {});
    world.checkpoint();
}

#[test]
fn crash_after_commit_before_ack_reopens_at_the_fence() {
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    world.tick(|_| {});
    let cow = world.spawn("minecraft:cow", [0.5, 80.0, 0.5]);
    let pig = world.spawn("minecraft:pig", [20.5, 80.0, 0.5]);
    world.checkpoint();
    world.tick(|state| {
        hurt(state, cow, 2.0);
        state.entities.set_pos(slot(state, pig), [21.5, 81.0, 0.5]);
    });
    let expected = full_image(&world.state);
    let (job, done) = world.capture();
    let tick = job.tick;
    *FAIL_CHECKPOINT_AFTER_NATIVE_COMMIT.lock().unwrap() = Some(world.config.world_dir.clone());
    assert!(world.commit(job, &done).is_none(), "crash before ACK");
    assert_eq!(stored_image(&world.config), expected);
    let mut world = world.crash();
    assert_eq!(world.manifest.tick, tick);
    world.tick(|state| hurt(state, pig, 3.0));
    world.checkpoint();
}

#[test]
fn tail_removal_and_transfer_survive_reopen_checkpoint_and_prune() {
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    world.tick(|_| {});
    let removed = world.spawn("minecraft:cow", [0.5, 80.0, 0.5]);
    let travelled = world.spawn("minecraft:pig", [20.5, 80.0, 0.5]);
    let kept = world.spawn("minecraft:sheep", [40.5, 80.0, 0.5]);
    world.checkpoint();
    let removed_uuid = uuid_of(&mut world.state, removed);
    let travelled_uuid = uuid_of(&mut world.state, travelled);
    world.tick(|state| {
        remove(state, removed);
        hurt(state, kept, 9.0);
    });
    world.transfer(travelled, Dimension::End);
    // NPC custody: a spawn renamed in its own tick never reaches the store.
    world.tick(|state| {
        let kind = type_id(state, "minecraft:villager");
        let npc = state.spawn_mob(kind, [60.5, 80.0, 0.5], 20.0);
        let slot = slot(state, npc);
        state
            .entities
            .set_uuid(slot, uuid::Uuid::from_u128(0x0020_0021));
    });
    let mut world = world.crash();
    world.checkpoint();
    let stored = stored_image(&world.config);
    assert!(
        !stored.contains_key(&removed_uuid),
        "tail removal tombstoned"
    );
    assert_eq!(stored[&travelled_uuid].0, 2, "tail transfer upserted");
    assert!(stored.contains_key(&uuid::Uuid::from_u128(0x0020_0021)));
    let mut world = world.crash();
    world.tick(|_| {});
    let (job, _) = world.capture();
    assert_eq!(
        captured(&job),
        world.stale(),
        "the reopened fence presents only stale rows"
    );
    assert!(job.tombstones.is_empty());
}

#[test]
fn pending_golem_deadline_is_rewritten_until_stored_expired() {
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    world.tick(|_| {});
    let villager = world.spawn("minecraft:villager", [0.5, 80.0, 0.5]);
    let until = world.state.tick + 6;
    world.tick(|state| {
        let slot = slot(state, villager);
        state
            .entities
            .row_mut(slot)
            .villager
            .as_mut()
            .unwrap()
            .golem_detected_until = Some(until);
    });
    world.checkpoint();
    let deadline = |world: &World| {
        let slot = slot(&world.state, villager);
        let row = world.state.entities.row(slot);
        row.villager.as_ref().unwrap().golem_detected_until
    };
    // Pending: rewritten with no store write, so the ttl follows the fence.
    for _ in 0..2 {
        world.tick(|_| {});
        world.tick(|_| {});
        let (job, _) = world.capture();
        assert_eq!(
            captured(&job).len(),
            1,
            "pending ttl is relative to the capture"
        );
        drop(job);
        world.checkpoint();
        world = world.crash();
        assert_eq!(deadline(&world), Some(until));
    }
    // Expired at the previous fence's tick or later: written once as expired.
    while world.state.tick <= until {
        world.tick(|_| {});
    }
    world.checkpoint();
    world.tick(|_| {});
    let (job, _) = world.capture();
    let rows = captured(&job);
    assert!(
        rows.is_empty(),
        "an expired deadline is tick-independent: {rows:?}"
    );
    drop(job);
    let world = world.crash();
    assert_eq!(deadline(&world), Some(until));
}

#[test]
fn raider_leaving_its_raid_is_rewritten() {
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    world.tick(|_| {});
    let pillager = world.spawn("minecraft:pillager", [0.5, 80.0, 0.5]);
    let uuid = uuid_of(&mut world.state, pillager);
    world.state.raids.raids.insert(
        1,
        crate::raids::Raid {
            center: [0, 64, 0],
            started: true,
            active: false,
            ticks_active: 0,
            raid_omen_level: 1,
            groups_spawned: 1,
            cooldown_ticks: 0,
            post_raid_ticks: 0,
            total_health: 0.0,
            group_count: 3,
            status: "stopped".into(),
            heroes: Default::default(),
            members: [uuid].into_iter().collect(),
        },
    );
    world.checkpoint();
    let raid_id = |world: &World| match &stored_image(&world.config)[&uuid].2 {
        mc_nbt::Tag::Compound(fields) => fields
            .iter()
            .find(|(key, _)| key == "RaidId")
            .map(|(_, value)| value.clone()),
        _ => None,
    };
    assert_eq!(raid_id(&world), Some(mc_nbt::Tag::Int(1)));
    world.state.raids.raids.get_mut(&1).unwrap().members.clear();
    world.checkpoint();
    assert_eq!(raid_id(&world), None, "membership owned outside the store");
}

#[test]
fn randomized_custody_matches_full_capture_at_every_fence() {
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    let mut seed = 0x2020_0020_d1ce_u64;
    let mut next = move |bound: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % bound as u64) as usize
    };
    let names = [
        "minecraft:cow",
        "minecraft:pig",
        "minecraft:chicken",
        "minecraft:villager",
        "minecraft:sheep",
    ];
    let dimensions = [Dimension::Overworld, Dimension::Nether, Dimension::End];
    world.tick(|_| {});
    let mut live: Vec<EntityId> = (0..10)
        .map(|index| world.spawn(names[index % names.len()], [index as f64 * 20.0, 80.0, 0.5]))
        .collect();
    world.checkpoint();
    let mut renamed = 0x0020_1000_u128;
    for step in 0..48 {
        let pick = live[next(live.len())];
        let op = next(8);
        let x = next(400) as f64 - 200.0;
        let dimension = dimensions[next(3)];
        let name = names[next(names.len())];
        if op == 6 {
            if crate::dimensions::travel::locate(&world.state, pick)
                .unwrap()
                .0
                != dimension
            {
                world.transfer(pick, dimension);
            } else {
                world.tick(|_| {});
            }
        } else {
            world.tick(|state| match op {
                0 => hurt(state, pick, 1.0 + (step % 19) as f32),
                1 => {
                    let (world, slot) = world_mut(state, pick);
                    world.entities.set_pos(slot, [x, 80.0, 0.5]);
                }
                2 => {
                    let (world, slot) = world_mut(state, pick);
                    world.entities.set_no_action_time(slot, step);
                }
                3 => {
                    let (world, slot) = world_mut(state, pick);
                    let mut admitted = vec![0; world.entities.len()];
                    admitted[slot.0 as usize] = 1;
                    world
                        .entities
                        .advance_admitted_ages(&admitted, u32::MAX, usize::MAX);
                }
                4 => {
                    let kind = type_id(state, name);
                    live.push(state.spawn_mob(kind, [x, 80.0, 0.5], 20.0));
                }
                5 if live.len() > 3 => {
                    remove(state, pick);
                    live.retain(|id| *id != pick);
                }
                _ => {
                    renamed += 1;
                    let (world, slot) = world_mut(state, pick);
                    world
                        .entities
                        .set_uuid(slot, uuid::Uuid::from_u128(renamed));
                }
            });
        }
        if step % 4 != 3 {
            continue;
        }
        match next(5) {
            // Lost capture: nothing written, nothing cleared.
            0 => drop(world.capture()),
            // Failed commit: nothing written, nothing cleared.
            1 => {
                let (job, done) = world.capture();
                FAIL_CHECKPOINT_AFTER_REGIONS
                    .lock()
                    .unwrap()
                    .push(world.config.world_dir.clone());
                assert!(world.commit(job, &done).is_none());
            }
            // Crash after a durable commit, before its ACK.
            2 => {
                let (job, done) = world.capture();
                *FAIL_CHECKPOINT_AFTER_NATIVE_COMMIT.lock().unwrap() =
                    Some(world.config.world_dir.clone());
                assert!(world.commit(job, &done).is_none());
                world = world.crash();
            }
            _ => {
                world.checkpoint();
            }
        }
    }
    world.tick(|_| {});
    world.checkpoint();
}

/// Custody across a runtime chunk refusal: an item leaves a chest of chunk X for a
/// chest of chunk Y, then X is refused by another record. Committing the rest of
/// the world while skipping X would store Y's diamond next to X's stale record, so a
/// restart would hold it twice. The checkpoint must decline (WAL kept) and the
/// restart replays the move: exactly one diamond.
#[test]
fn refused_chunk_with_a_moved_item_declines_the_checkpoint_and_the_item_exists_once() {
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    world.tick(|_| {});
    let from = crate::boundary::BlockPos {
        x: 40,
        y: 100,
        z: 40,
    };
    let to = crate::boundary::BlockPos {
        x: 72,
        y: 100,
        z: 40,
    };
    let chunk_of = |pos: crate::boundary::BlockPos| mc_world::ChunkPos {
        x: pos.x.div_euclid(16),
        z: pos.z.div_euclid(16),
    };
    world.tick(|state| {
        let chest = state
            .registry
            .blocks
            .block(&mc_data::Identifier::parse("minecraft:chest").unwrap())
            .unwrap()
            .default
            .0;
        // Tickets keep both chunks resident when the source is refused.
        state
            .tickets
            .insert("custody-from".into(), (chunk_of(from), 0));
        state.tickets.insert("custody-to".into(), (chunk_of(to), 0));
        state.refresh_active_chunks(state.tick);
        let want = [(chunk_of(from), 0), (chunk_of(to), 0)]
            .into_iter()
            .collect();
        state.world.ensure_blocking(&want);
        for pos in [from, to] {
            state.world.set_block(pos.x, pos.y, pos.z, chest);
            state
                .block_entities
                .hydrate((pos.x, pos.y, pos.z), || crate::tick::BlockEntityState {
                    slots: vec![mc_data::ItemStack::EMPTY; 27],
                    furnace: None,
                });
        }
        state.world.fold_overlay();
    });
    let diamond = |state: &TickState| {
        mc_data::ItemStack::new(
            state
                .registry
                .items
                .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
                .unwrap(),
            1,
        )
    };
    world.tick(|state| {
        let stack = diamond(state);
        state
            .block_entities
            .get_mut(&(from.x, from.y, from.z))
            .expect("chest record")
            .slots[0] = stack;
        state.world.mark_block_entity_dirty(from);
    });
    world.checkpoint();

    // The move, then the refusal of the source chunk by an unrelated record.
    world.tick(|state| {
        let stack = diamond(state);
        state
            .block_entities
            .get_mut(&(from.x, from.y, from.z))
            .unwrap()
            .slots[0] = mc_data::ItemStack::EMPTY;
        state
            .block_entities
            .get_mut(&(to.x, to.y, to.z))
            .unwrap()
            .slots[0] = stack;
        state.world.mark_block_entity_dirty(from);
        state.world.mark_block_entity_dirty(to);
        state
            .world
            .reject_resident(chunk_of(from), "undecodable record".to_string());
    });
    let committed = world.manifest.tick;
    for _ in 0..2 {
        let (work_tx, _work_rx) = journal_work_channel();
        let (done_tx, _done_rx) = std::sync::mpsc::channel();
        let tick = world.state.tick;
        assert!(
            !start_checkpoint(&mut world.state, &work_tx, &done_tx, tick),
            "a refused chunk with unsaved state declines the checkpoint"
        );
        assert_eq!(world.manifest.tick, committed);
    }

    // Restart: the WAL replays the move; the diamond exists exactly once.
    let config = world.config.clone();
    drop(world);
    let reopened = World::open(config);
    let count: i32 = [from, to]
        .iter()
        .filter_map(|pos| reopened.state.block_entities.get(&(pos.x, pos.y, pos.z)))
        .flat_map(|record| record.slots.iter())
        .map(|stack| stack.count)
        .sum();
    assert_eq!(count, 1, "exactly one diamond after restart");
}

/// A container broken in chunk X (record removed, contents dropped as an item
/// entity), then X refused at runtime. Only the dirty mark remembers the removal,
/// so the checkpoint must decline: committing the rest would store the dropped
/// item while X's stale record still holds it. After a restart the diamond exists
/// exactly once, as the container's content or as the dropped item, never both.
#[test]
fn refused_chunk_with_a_broken_container_declines_the_checkpoint_and_the_item_exists_once() {
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    world.tick(|_| {});
    let at = crate::boundary::BlockPos {
        x: 40,
        y: 100,
        z: 40,
    };
    let chunk = mc_world::ChunkPos {
        x: at.x.div_euclid(16),
        z: at.z.div_euclid(16),
    };
    world.tick(|state| {
        let chest = state
            .registry
            .blocks
            .block(&mc_data::Identifier::parse("minecraft:chest").unwrap())
            .unwrap()
            .default
            .0;
        // A ticket keeps the chunk resident, so the break and the later refusal
        // act on a loaded chunk.
        state.tickets.insert("custody".into(), (chunk, 0));
        state.refresh_active_chunks(state.tick);
        state
            .world
            .ensure_blocking(&[(chunk, 0)].into_iter().collect());
        state.world.set_block(at.x, at.y, at.z, chest);
        state.world.fold_overlay();
        let diamond = mc_data::ItemStack::new(
            state
                .registry
                .items
                .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
                .unwrap(),
            1,
        );
        let mut slots = vec![mc_data::ItemStack::EMPTY; 27];
        slots[0] = diamond;
        state
            .block_entities
            .hydrate((at.x, at.y, at.z), || crate::tick::BlockEntityState {
                slots,
                furnace: None,
            });
        state.world.mark_block_entity_dirty(at);
    });
    world.checkpoint();

    let (command, mut done) = crate::command_console::ConsoleCommand::new(format!(
        "setblock {} {} {} air destroy",
        at.x, at.y, at.z
    ));
    world.run(&[crate::boundary::PlayerInput::ConsoleCommand(command)]);
    assert!(done.try_recv().is_ok());
    assert!(
        !world.state.block_entities.contains_key(&(at.x, at.y, at.z)),
        "the break removed the container record"
    );
    let dropped = |state: &TickState| -> i32 {
        let diamond = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
            .unwrap();
        state
            .entities
            .slots()
            .filter(|slot| state.entities.body(*slot).alive)
            .filter_map(|slot| state.entities.row(slot).item.as_ref())
            .filter(|item| item.item_id == diamond)
            .map(|item| item.count)
            .sum()
    };
    assert_eq!(dropped(&world.state), 1, "the contents dropped");
    world
        .state
        .world
        .reject_resident(chunk, "undecodable record".to_string());

    let committed = world.manifest.tick;
    for _ in 0..2 {
        let (work_tx, _work_rx) = journal_work_channel();
        let (done_tx, _done_rx) = std::sync::mpsc::channel();
        let tick = world.state.tick;
        assert!(
            !start_checkpoint(&mut world.state, &work_tx, &done_tx, tick),
            "the removal tombstone of a refused chunk declines the checkpoint"
        );
        assert_eq!(world.manifest.tick, committed);
    }

    let config = world.config.clone();
    drop(world);
    let reopened = World::open(config);
    let held: i32 = reopened
        .state
        .block_entities
        .get(&(at.x, at.y, at.z))
        .map_or(0, |record| {
            record.slots.iter().map(|stack| stack.count).sum()
        });
    assert_eq!(
        held + dropped(&reopened.state),
        1,
        "exactly one diamond after restart"
    );
}

/// The removal of a container record leaves nothing but the dirty mark behind
/// (no stale record, no block edit). The mark must survive a refusal of its chunk
/// (made before it) and must also count when it is made after the refusal.
#[test]
fn container_removal_mark_of_a_refused_chunk_counts_before_and_after_the_refusal() {
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    world.tick(|_| {});
    let chunk_of = |x: i32| mc_world::ChunkPos {
        x: x.div_euclid(16),
        z: 2,
    };
    let before = crate::boundary::BlockPos {
        x: 40,
        y: 100,
        z: 40,
    };
    let after = crate::boundary::BlockPos {
        x: 41,
        y: 100,
        z: 41,
    };
    world.tick(|state| {
        state
            .tickets
            .insert("custody".into(), (chunk_of(before.x), 0));
        state.refresh_active_chunks(state.tick);
        state
            .world
            .ensure_blocking(&[(chunk_of(before.x), 0)].into_iter().collect());
        for pos in [before, after] {
            state
                .block_entities
                .hydrate((pos.x, pos.y, pos.z), || crate::tick::BlockEntityState {
                    slots: vec![mc_data::ItemStack::EMPTY; 27],
                    furnace: None,
                });
            state.world.mark_block_entity_dirty(pos);
        }
    });
    world.checkpoint();
    let chunk = chunk_of(before.x);

    // Removal and mark first, then the refusal: only the mark remembers it.
    world.tick(|state| {
        state.block_entities.remove(&(before.x, before.y, before.z));
        state.world.mark_block_entity_dirty(before);
        state
            .world
            .reject_resident(chunk, "undecodable record".to_string());
    });
    let capture = world.state.world.capture_dirty(&Default::default());
    assert!(
        capture.load_error().is_some(),
        "mark made before the refusal"
    );

    // A clean refusal accepts nothing until a removal mark arrives after it.
    let mut fresh = World::new(directory.path());
    fresh.tick(|state| {
        state.tickets.insert("custody".into(), (chunk, 0));
        state.refresh_active_chunks(state.tick);
        state
            .world
            .ensure_blocking(&[(chunk, 0)].into_iter().collect());
        state
            .world
            .reject_resident(chunk, "undecodable record".to_string());
    });
    let clean = fresh.state.world.capture_dirty(&Default::default());
    assert!(clean.load_error().is_none());
    assert_eq!(clean.refused()[0].unsaved, 0);
    fresh.state.world.mark_block_entity_dirty(after);
    let marked = fresh.state.world.capture_dirty(&Default::default());
    assert!(marked.load_error().is_some(), "mark made after the refusal");
    assert_eq!(marked.refused()[0].unsaved, 1);
}

/// Pending scheduled work of a refused chunk: the checkpoint skips the chunk-local
/// projection (counted), but the global scheduler images in the manifest keep the
/// queues, so a restart restores the same block and fluid ticks.
#[test]
fn scheduled_ticks_of_a_refused_chunk_survive_checkpoint_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    world.tick(|_| {});
    let at = crate::boundary::BlockPos {
        x: 40,
        y: 100,
        z: 40,
    };
    let chunk = mc_world::ChunkPos {
        x: at.x.div_euclid(16),
        z: at.z.div_euclid(16),
    };
    world.tick(|state| {
        state.tickets.insert("custody".into(), (chunk, 0));
        state.refresh_active_chunks(state.tick);
        state
            .world
            .ensure_blocking(&[(chunk, 0)].into_iter().collect());
        let due = state.tick + 600;
        assert!(
            state
                .block_ticks
                .schedule(mc_world::ScheduledBlockTick::new(
                    at,
                    mc_data::Identifier::parse("minecraft:stone").unwrap(),
                    due,
                    0,
                ))
        );
        assert!(state.fluid_ticks.schedule(
            at,
            &mc_data::Identifier::parse("minecraft:water").unwrap(),
            due,
            0,
        ));
        state
            .world
            .reject_resident(chunk, "undecodable record".to_string());
    });
    let queued = |state: &TickState| {
        (
            state
                .block_ticks
                .image()
                .chunks
                .iter()
                .map(|image| image.ticks.len())
                .sum::<usize>(),
            state.fluid_ticks.image().due.len(),
        )
    };
    assert_eq!(queued(&world.state), (1, 1));
    // The refused chunk holds nothing unsaved but scheduled work: it commits.
    let capture = world.state.world.capture_dirty(&Default::default());
    assert!(capture.load_error().is_none());
    world.checkpoint();
    let config = world.config.clone();
    drop(world);
    let reopened = World::open(config);
    assert_eq!(
        queued(&reopened.state),
        (1, 1),
        "the manifest scheduler images kept both ticks"
    );
}

/// A WAL write into a chunk refused at load. The chunk checkpointed valid; then an item
/// put on its campfire (an opaque block-entity record) and a scheduled tick of the same
/// chunk reach the WAL with no checkpoint after them, and the stored chunk turns
/// unloadable. Recovery cannot replay the record into the chunk, so it refuses to open
/// the world rather than drop it: nothing is checkpointed past the record and the WAL
/// keeps it until the stored chunk is valid again, when the replay installs it and a
/// checkpoint stores the item.
#[test]
fn opaque_replay_into_a_chunk_refused_at_load_fails_recovery_and_keeps_the_wal() {
    use mc_nbt::Tag;
    let directory = tempfile::tempdir().unwrap();
    let mut world = World::new(directory.path());
    world.tick(|_| {});
    let at = crate::boundary::BlockPos {
        x: 40,
        y: 100,
        z: 40,
    };
    let chunk = mc_world::ChunkPos {
        x: at.x.div_euclid(16),
        z: at.z.div_euclid(16),
    };
    world.tick(|state| {
        state.tickets.insert("custody".into(), (chunk, 0));
        state.refresh_active_chunks(state.tick);
        state
            .world
            .ensure_blocking(&[(chunk, 0)].into_iter().collect());
        // Unlit, so the campfire never cooks or rewrites its record by itself.
        let id = mc_data::Identifier::parse("minecraft:campfire").unwrap();
        let block = state.registry.blocks.block(&id).unwrap();
        let mut properties = state
            .registry
            .blocks
            .by_id(block.default)
            .unwrap()
            .properties
            .clone();
        properties
            .iter_mut()
            .find(|(name, _)| name == "lit")
            .unwrap()
            .1 = "false".into();
        let unlit = state
            .registry
            .blocks
            .by_name_and_props(&id, &properties)
            .unwrap()
            .0;
        state.world.set_block(at.x, at.y, at.z, unlit);
        state.world.fold_overlay();
    });
    world.checkpoint();
    let committed = world.manifest.tick;
    let source = world.state.world.chunk_source();
    let store = Arc::clone(source.native.as_ref().unwrap());
    let valid = source.load_native(chunk).unwrap().expect("checkpointed chunk");
    let base = source.resolve_base(chunk).unwrap();
    let registry = Arc::clone(&world.state.registry);

    let beef = mc_data::ItemStack::new(
        registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:beef").unwrap())
            .unwrap(),
        1,
    );
    let mut item = mc_data::item_nbt::stack_to_fields(&registry.items, &beef).unwrap();
    item.push(("Slot".into(), Tag::Byte(0)));
    let mut bytes = Vec::new();
    mc_nbt::write_network(
        &mut bytes,
        &Tag::Compound(vec![
            (
                "Items".into(),
                Tag::List(mc_nbt::ListTag {
                    element_type: mc_nbt::tag_type::COMPOUND,
                    elements: vec![Tag::Compound(item)],
                }),
            ),
            ("id".into(), Tag::String("minecraft:campfire".into())),
            ("x".into(), Tag::Int(at.x)),
            ("y".into(), Tag::Int(at.y)),
            ("z".into(), Tag::Int(at.z)),
            ("CookingTimes".into(), Tag::IntArray(vec![0; 4])),
            ("CookingTotalTimes".into(), Tag::IntArray(vec![600, 0, 0, 0])),
        ]),
    )
    .unwrap();
    // The campfire owner's write path: the chunk record plus its WAL image.
    world.tick(|state| {
        state.world.set_opaque_block_entity(at, Some(bytes.clone()));
        state.opaque_changes.push((at, Some(bytes.clone())));
        let due = state.tick + 600;
        assert!(
            state
                .block_ticks
                .schedule(mc_world::ScheduledBlockTick::new(
                    at,
                    mc_data::Identifier::parse("minecraft:stone").unwrap(),
                    due,
                    0,
                ))
        );
    });
    let config = world.config.clone();
    drop(world);
    // What a restart replays past the committed checkpoint: the campfire record and a
    // pending scheduled tick of the same chunk.
    let wal = || {
        let recovered = crate::native_checkpoint::recover_with_reader(
            &config,
            Arc::clone(&registry),
            store.checkpoint_reader().unwrap(),
        )
        .unwrap();
        let replay = &recovered.dimensions[&Dimension::Overworld];
        let scheduled = replay
            .block_ticks
            .chunks
            .iter()
            .any(|image| image.chunk == [chunk.x, chunk.z] && !image.ticks.is_empty());
        (
            replay.opaque_block_entities.get(&(at.x, at.y, at.z)).cloned(),
            scheduled,
        )
    };
    assert_eq!(wal(), (Some(Some(bytes.clone())), true));

    // The stored chunk turns unloadable: a conduit record the native load refuses.
    let mut corrupt = valid.clone();
    let conduit = mc_world::BlockPos { x: at.x + 1, ..at };
    let mut record = Vec::new();
    mc_nbt::write_network(
        &mut record,
        &Tag::Compound(vec![
            ("id".into(), Tag::String("minecraft:conduit".into())),
            ("x".into(), Tag::Int(conduit.x)),
            ("y".into(), Tag::Int(conduit.y)),
            ("z".into(), Tag::Int(conduit.z)),
            ("Target".into(), Tag::IntArray(vec![1, 2, 3])),
        ]),
    )
    .unwrap();
    corrupt.block_entities.insert(conduit, record);
    store
        .apply_offline_snapshot(
            source.dimension,
            &base,
            &mut corrupt,
            &registry.blocks,
            &registry.items,
        )
        .unwrap();
    let refusal = source.load_native(chunk).unwrap_err().to_string();
    let counts = store.record_counts().unwrap();

    // Recovery must make the chunk resident to replay the record and cannot, so it
    // stops with the chunk's load error instead of dropping the record: the world never
    // opens, no checkpoint runs, and the store and the WAL stay as they were.
    for _ in 0..2 {
        let Err(error) = recover_dimension_state(&config, Dimension::Overworld) else {
            panic!("recovery opened a world whose WAL writes into an unloadable chunk");
        };
        assert!(error.to_string().contains(&refusal), "{error}");
        assert_eq!(read_manifest(&config).unwrap().tick, committed);
        assert_eq!(store.record_counts().unwrap(), counts);
    }
    assert_eq!(
        wal(),
        (Some(Some(bytes.clone())), true),
        "the WAL still carries the campfire record and the scheduled tick"
    );

    // The stored chunk is valid again: the replay installs the record and a
    // checkpoint stores the campfire's item.
    let mut repaired = valid;
    store
        .apply_offline_snapshot(
            source.dimension,
            &base,
            &mut repaired,
            &registry.blocks,
            &registry.items,
        )
        .unwrap();
    let mut world = World::open(config.clone());
    assert!(world.state.world.load_error(chunk).is_none());
    assert_eq!(
        world
            .state
            .world
            .chunk(chunk)
            .expect("repaired chunk resident")
            .block_entities
            .get(&at),
        Some(&bytes),
        "the replay installed the campfire record"
    );
    world.tick(|_| {});
    world.tick(|_| {});
    world.checkpoint();
    let stored = source.load_native(chunk).unwrap().unwrap();
    let Tag::Compound(fields) =
        mc_nbt::read_network(&mut stored.block_entities[&at].as_slice()).unwrap()
    else {
        panic!("campfire record is a compound");
    };
    let Some(Tag::List(items)) = mc_data::item_nbt::field(&fields, "Items") else {
        panic!("campfire Items");
    };
    let [Tag::Compound(item)] = &items.elements[..] else {
        panic!("one campfire item");
    };
    assert_eq!(
        mc_data::item_nbt::stack_from_fields(&registry.items, item)
            .unwrap()
            .unwrap(),
        beef,
        "the campfire's beef reached the store"
    );
}
