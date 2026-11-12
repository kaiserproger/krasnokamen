//! Actual native store + published tick WAL + checkpoint recovery candidates.
//! No serde-only "restart", no synthetic delivery events, no GUI proof claim.
use super::*;
use crate::settlements::{economy, persistence};
use crate::{CoreConfig, dimensions::Dimension, journal::JournalWriter};

struct Disk {
    world: TickState,
    town: SettlementId,
    body: EntityId,
    worker: Uuid,
    chest: [i32; 3],
}

fn startup(world: &mut TickState) {
    crate::plugin_startup::apply(
        Arc::make_mut(&mut world.registry),
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )
    .unwrap();
}

fn fixture(config: &CoreConfig) -> Disk {
    let mut world = crate::core::recover_dimension_state(config, Dimension::Overworld).unwrap();
    startup(&mut world);
    let _ = crate::tick::run_tick(&mut world, &[]); // Creation commit is tick zero.
    let (mut world, town, body, worker, table, stand_at, worksite) = setup_in(world);
    let chest = supply_chest(&mut world, town, 0);
    let mut slots = vec![item(&world, "minecraft:cobblestone", 64); 27];
    slots[0] = item(&world, "minecraft:oak_planks", 63);
    world
        .block_entities
        .get_mut(&(chest[0], chest[1], chest[2]))
        .unwrap()
        .slots = slots;
    give(
        vec![
            item(&world, "minecraft:oak_log", 1),
            item(&world, "minecraft:wooden_axe", 1),
            item(&world, "minecraft:oak_planks", 2),
            item(&world, "minecraft:coal", 64),
        ],
        &mut world,
        body,
    );
    let slot = world.entities.slot_of(body).unwrap();
    // Deterministic DOMAIN durability fixture, not a real-client motion test.
    world.entities.row_mut(slot).controls.no_ai = true;
    economy::execute(
        &mut world,
        town,
        economy::Request::Production(Request::Start {
            job: "durable".into(),
            worker,
            recipe: "krasnokamen-settlements:oak_planks".into(),
            count: 1,
            station: table,
            stand_at,
            building: worksite_ref(&worksite),
        }),
    )
    .unwrap();
    Disk {
        world,
        town,
        body,
        worker,
        chest,
    }
}

fn checkpoint(f: &mut Disk, config: &CoreConfig) {
    use crate::native_checkpoint::{encode_entities, encode_players, record, refresh_metadata};
    use mc_world::native::NativeCheckpoint;
    let store = crate::native_checkpoint::tests::fixture_store(config).unwrap();
    let tick = f.world.tick;
    let (players, captured, block_entities) = crate::core::checkpoint_data_for(&f.world, tick);
    let entities = encode_entities(
        Dimension::Overworld,
        &captured,
        &f.world.registry.items,
        tick,
    )
    .unwrap();
    let players = encode_players(players.iter(), &f.world.registry.items, tick).unwrap();
    let capture = f.world.world.capture_dirty(&block_entities);
    let written = capture.written();
    let mut image = store.read_checkpoint().unwrap();
    let mut manifest: crate::journal::CheckpointManifest =
        serde_json::from_slice(&image.globals["checkpoint"].payload).unwrap();
    manifest.tick = tick;
    for child in manifest.dimensions.values_mut() {
        child.tick = tick;
    }
    image
        .globals
        .extend(persistence::checkpoint_rows(&f.world, tick).unwrap());
    image.globals.insert(
        "plugins".into(),
        record(tick, serde_json::to_vec(&f.world.plugins).unwrap()),
    );
    image.globals.insert(
        "checkpoint".into(),
        record(tick, serde_json::to_vec(&manifest).unwrap()),
    );
    refresh_metadata(&mut image.globals, &manifest).unwrap();
    for global in image.globals.values_mut() {
        global.captured_tick = tick;
    }
    store
        .commit_checkpoint_stream(
            NativeCheckpoint {
                tick,
                previous_tick: image.tick,
                chunks: &[],
                entities: &[],
                players: &players,
                globals: &image.globals,
            },
            |write| {
                capture
                    .visit_native(|snapshot| {
                        write(snapshot)
                            .map_err(|error| crate::config::CoreError::Storage(error.to_string()))
                    })
                    .map_err(|error| mc_world::native::NativeError::Source(error.to_string()))
            },
            entities.iter().map(Ok),
            &f.world.registry.blocks,
            &f.world.registry.items,
        )
        .unwrap();
    f.world.world.mark_flushed(&written);
}

fn step(f: &mut Disk, writer: &mut JournalWriter) {
    let (output, _, _) = crate::tick::run_tick(&mut f.world, &[]);
    let entries = crate::core::journal_entries_for_tick(&mut f.world, &output);
    writer.append(output.tick, output.tick, &entries).unwrap();
}

fn until(f: &mut Disk, writer: &mut JournalWriter, condition: impl Fn(&Job) -> bool) {
    for _ in 0..2_000 {
        if condition(
            &f.world
                .settlements
                .get(f.town)
                .unwrap()
                .economy
                .production
                .jobs["durable"],
        ) {
            return;
        }
        step(f, writer);
    }
    panic!(
        "durable phase did not arrive: {:?}",
        f.world
            .settlements
            .get(f.town)
            .unwrap()
            .economy
            .production
            .jobs["durable"]
    );
}

fn command(f: &mut Disk, request: Request) {
    economy::execute(&mut f.world, f.town, economy::Request::Production(request)).unwrap();
}

fn set_deadline_through_owner(f: &mut Disk, name: &str, due: u64) {
    let revision = f
        .world
        .settlements
        .get(f.town)
        .unwrap()
        .revision
        .checked_add(1)
        .unwrap();
    f.world
        .settlements
        .economy_for_tick(f.town)
        .production
        .jobs
        .get_mut(name)
        .unwrap()
        .next_due = due;
    f.world.settlements.finish_economy(f.town, revision, true);
}

fn assert_policy_postimage(entries: &[crate::journal::JournalEntry], f: &Disk) {
    fn contains(
        entries: &[crate::journal::JournalEntry],
        town: SettlementId,
        expected: &serde_json::Value,
    ) -> bool {
        entries.iter().any(|entry| match entry {
            crate::journal::JournalEntry::Dimension { entries, .. } => {
                contains(entries, town, expected)
            }
            crate::journal::JournalEntry::Settlement { image, .. } => {
                image.id == town
                    && image.value.as_ref().is_some_and(|record| {
                        serde_json::to_value(&record.economy.production).unwrap() == *expected
                    })
            }
            _ => false,
        })
    }
    let expected =
        serde_json::to_value(&f.world.settlements.get(f.town).unwrap().economy.production).unwrap();
    assert!(
        contains(entries, f.town, &expected),
        "expected policy must actually be in the published WAL frame"
    );
}

#[derive(Debug, PartialEq)]
struct Image {
    policy: serde_json::Value,
    carry: Vec<ItemStack>,
    carry_fence: mc_script::ScriptInventoryFence,
    destination: Vec<ItemStack>,
    destination_fence: mc_script::ScriptInventoryFence,
}

fn image(f: &mut Disk) -> Image {
    let carry = inventory::load(
        &f.world,
        &inventory::Endpoint::VillagerCarry { uuid: f.worker },
    )
    .unwrap();
    let destination = inventory::load(
        &f.world,
        &inventory::Endpoint::Container {
            dimension: f.world.dimension.name().into(),
            position: f.chest,
        },
    )
    .unwrap();
    Image {
        policy: serde_json::to_value(&f.world.settlements.get(f.town).unwrap().economy.production)
            .unwrap(),
        carry: carry.slots,
        carry_fence: carry.fence,
        destination: destination.slots,
        destination_fence: destination.fence,
    }
}

fn reopen(f: Disk, config: &CoreConfig, mut writer: JournalWriter) -> (Disk, JournalWriter) {
    writer.flush().unwrap();
    drop(writer);
    let Disk {
        world,
        town,
        body,
        worker,
        chest,
    } = f;
    drop(world); // Close the old native-store Arc as well as the journal lease.
    let mut world = crate::core::recover_dimension_state(config, Dimension::Overworld).unwrap();
    startup(&mut world);
    ensure(&mut world);
    assert_eq!(
        world.entities.slot_of_uuid(worker),
        world.entities.slot_of(body)
    );
    let mut f = Disk {
        world,
        town,
        body,
        worker,
        chest,
    };
    let mut writer = JournalWriter::open(&config.world_dir, 1).unwrap();
    // Hydrate real block entities without advancing the scenario obligation.
    // Restore due values BEFORE collecting the absolute tick post-image. The
    // hydration frame is published, not silently skipped in the WAL sequence.
    let saved: Vec<_> = f
        .world
        .settlements
        .get(f.town)
        .unwrap()
        .economy
        .production
        .jobs
        .iter()
        .map(|(name, job)| (name.clone(), job.next_due))
        .collect();
    for (name, _) in &saved {
        set_deadline_through_owner(&mut f, name, u64::MAX);
    }
    let (output, _, _) = crate::tick::run_tick(&mut f.world, &[]);
    for (name, due) in saved {
        set_deadline_through_owner(&mut f, &name, due);
    }
    let entries = crate::core::journal_entries_for_tick(&mut f.world, &output);
    assert_policy_postimage(&entries, &f);
    writer.append(output.tick, output.tick, &entries).unwrap();
    (f, writer)
}

#[derive(Clone, Copy, Debug)]
enum Phase {
    BeforeProduction,
    Pending,
    Partial,
    Paused,
    Cancelled,
    Delivered,
    Invalidated,
}

fn reach_phase(f: &mut Disk, writer: &mut JournalWriter, phase: Phase) {
    if matches!(phase, Phase::BeforeProduction) {
        return;
    }
    until(f, writer, |job| job.done == 1);
    assert_eq!(
        f.world
            .settlements
            .get(f.town)
            .unwrap()
            .economy
            .production
            .jobs["durable"]
            .delivery
            .as_ref()
            .unwrap()
            .lots[0]
            .stack
            .count,
        4
    );
    if matches!(phase, Phase::Pending) {
        return;
    }
    beside(&mut f.world, f.body, f.chest);
    until(f, writer, |job| {
        job.delivery.as_ref().unwrap().sequence == 1
    });
    assert_eq!(
        f.world
            .settlements
            .get(f.town)
            .unwrap()
            .economy
            .production
            .jobs["durable"]
            .delivery
            .as_ref()
            .unwrap()
            .lots[0]
            .stack
            .count,
        3
    );
    match phase {
        Phase::Paused => {
            command(
                f,
                Request::Pause {
                    job: "durable".into(),
                },
            );
            step(f, writer);
        }
        Phase::Cancelled => {
            command(
                f,
                Request::Cancel {
                    job: "durable".into(),
                },
            );
            step(f, writer);
        }
        Phase::Delivered => {
            f.world
                .block_entities
                .get_mut(&(f.chest[0], f.chest[1], f.chest[2]))
                .unwrap()
                .slots[1] = ItemStack::EMPTY;
            f.world.world.mark_block_entity_dirty(BlockPos {
                x: f.chest[0],
                y: f.chest[1],
                z: f.chest[2],
            });
            until(f, writer, Job::delivery_complete);
            assert_eq!(stock_total(&f.world, f.body, "minecraft:oak_planks"), 2);
            assert_eq!(stock_total(&f.world, f.body, "minecraft:wooden_axe"), 1);
            assert_eq!(stock_total(&f.world, f.body, "minecraft:coal"), 64);
        }
        Phase::Invalidated => {
            let slot = f.world.entities.slot_of(f.body).unwrap();
            let planks = item(&f.world, "minecraft:oak_planks", 1).item_id;
            let v = f.world.entities.row_mut(slot).villager.as_mut().unwrap();
            v.inventory
                .iter_mut()
                .flatten()
                .find(|stack| stack.item_id == planks)
                .unwrap()
                .count -= 1;
            until(f, writer, |job| job.delivery.as_ref().unwrap().invalidated);
        }
        _ => {}
    }
}

#[test]
fn obligation_and_both_inventories_recover_through_actual_wal_and_checkpoint() {
    for phase in [
        Phase::BeforeProduction,
        Phase::Pending,
        Phase::Partial,
        Phase::Paused,
        Phase::Cancelled,
        Phase::Delivered,
        Phase::Invalidated,
    ] {
        for save_checkpoint in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let config = CoreConfig::new(directory.path().to_path_buf(), 42);
            let mut f = fixture(&config);
            checkpoint(&mut f, &config);
            let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
            reach_phase(&mut f, &mut writer, phase);
            let expected = image(&mut f);
            writer.flush().unwrap();
            if save_checkpoint {
                // Do not reuse creation tick as a second checkpoint.
                step(&mut f, &mut writer);
                checkpoint(&mut f, &config);
            }
            // Some waiting phases legitimately update next_due on step. Capture
            // the exact final durable policy, not a preferred earlier image.
            let expected = if save_checkpoint {
                image(&mut f)
            } else {
                expected
            };
            let (mut f, writer) = reopen(f, &config, writer);
            assert_eq!(
                image(&mut f),
                expected,
                "phase={phase:?} checkpoint={save_checkpoint}"
            );
            let (mut f, writer) = reopen(f, &config, writer);
            assert_eq!(image(&mut f), expected, "second reopen must be idempotent");
            if matches!(phase, Phase::Delivered) {
                let (frozen, _, _) = crate::tick::run_tick(&mut f.world, &[]);
                let _entries = crate::core::journal_entries_for_tick(&mut f.world, &frozen);
                assert_eq!(
                    image(&mut f),
                    expected,
                    "completed delivery does not replay"
                );
            }
            drop(writer);
        }
    }
}

#[test]
fn unpublished_store_rolls_back_but_published_store_replays_exactly_once() {
    for checkpoint_pending in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let config = CoreConfig::new(directory.path().to_path_buf(), 42);
        let mut f = fixture(&config);
        checkpoint(&mut f, &config);
        let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
        reach_phase(&mut f, &mut writer, Phase::Pending);
        if checkpoint_pending {
            checkpoint(&mut f, &config);
        }
        // Persist approach/deadline through the actual policy owner, not an
        // unjournaled mutable projection of economy state.
        beside(&mut f.world, f.body, f.chest);
        // Persist the approach but hold delivery until the following tick.
        let next_tick = f.world.tick.saturating_add(1);
        set_deadline_through_owner(&mut f, "durable", next_tick);
        let (output, _, _) = crate::tick::run_tick(&mut f.world, &[]);
        let entries = crate::core::journal_entries_for_tick(&mut f.world, &output);
        assert_policy_postimage(&entries, &f);
        writer.append(output.tick, output.tick, &entries).unwrap();
        let expected = image(&mut f);
        let (output, _, _) = crate::tick::run_tick(&mut f.world, &[]);
        let _unpublished = crate::core::journal_entries_for_tick(&mut f.world, &output);
        assert_eq!(
            f.world
                .settlements
                .get(f.town)
                .unwrap()
                .economy
                .production
                .jobs["durable"]
                .delivery
                .as_ref()
                .unwrap()
                .sequence,
            1
        );
        // No append: simulate process loss before the complete WAL frame was
        // published. This is a persistence cut, NOT a claimed SIGKILL run.
        let (mut f, mut writer) = reopen(f, &config, writer);
        assert_eq!(
            image(&mut f),
            expected,
            "unpublished store cannot survive only in the container"
        );
        until(&mut f, &mut writer, |job| {
            job.delivery.as_ref().unwrap().sequence == 1
        });
        let committed = image(&mut f);
        let (mut f, writer) = reopen(f, &config, writer);
        assert_eq!(
            image(&mut f),
            committed,
            "published store restores carry, destination, debt and fences together"
        );
        let (mut f, writer) = reopen(f, &config, writer);
        assert_eq!(
            image(&mut f),
            committed,
            "same frame cannot pay the obligation twice"
        );
        drop(writer);
    }
}

#[test]
fn unpublished_production_does_not_leave_an_obligation_without_its_items() {
    for checkpoint_before in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let config = CoreConfig::new(directory.path().to_path_buf(), 42);
        let mut f = fixture(&config);
        checkpoint(&mut f, &config);
        let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
        let due = f
            .world
            .settlements
            .get(f.town)
            .unwrap()
            .economy
            .production
            .jobs["durable"]
            .next_due;
        while f.world.tick < due {
            step(&mut f, &mut writer);
        }
        if checkpoint_before {
            checkpoint(&mut f, &config);
        }
        let expected = image(&mut f);
        let (output, _, _) = crate::tick::run_tick(&mut f.world, &[]);
        let _unpublished = crate::core::journal_entries_for_tick(&mut f.world, &output);
        assert_eq!(
            f.world
                .settlements
                .get(f.town)
                .unwrap()
                .economy
                .production
                .jobs["durable"]
                .done,
            1
        );
        let (mut f, mut writer) = reopen(f, &config, writer);
        assert_eq!(
            image(&mut f),
            expected,
            "unpublished production rolls back tool wear, inputs, output and debt together"
        );
        until(&mut f, &mut writer, |job| job.done == 1);
        let published = image(&mut f);
        let (mut f, writer) = reopen(f, &config, writer);
        assert_eq!(image(&mut f), published);
        let (mut f, writer) = reopen(f, &config, writer);
        assert_eq!(
            image(&mut f),
            published,
            "replay cannot manufacture another portion or obligation"
        );
        drop(writer);
    }
}

#[test]
fn cancelled_partial_obligation_stays_unpaid_when_same_uuid_delivers_a_new_job_after_reopen() {
    for save_checkpoint in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let config = CoreConfig::new(directory.path().to_path_buf(), 42);
        let mut f = fixture(&config);
        checkpoint(&mut f, &config);
        let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
        reach_phase(&mut f, &mut writer, Phase::Cancelled);
        let old = f
            .world
            .settlements
            .get(f.town)
            .unwrap()
            .economy
            .production
            .jobs["durable"]
            .clone();
        assert_eq!(old.delivery.as_ref().unwrap().lots[0].stack.count, 3);
        if save_checkpoint {
            checkpoint(&mut f, &config);
        }
        let (mut f, mut writer) = reopen(f, &config, writer);
        let mut inputs = carry(&f.world, f.body);
        let empty = inputs.iter().position(ItemStack::is_empty).unwrap();
        inputs[empty] = item(&f.world, "minecraft:oak_log", 1);
        give(inputs, &mut f.world, f.body);
        at(&mut f.world, f.body, [7.5, 201.0, 8.5]);
        let start = Request::Start {
            job: "new-after-cancel".into(),
            worker: f.worker,
            recipe: old.recipe.clone(),
            count: 1,
            station: old.station,
            stand_at: old.stand_at,
            building: old.building.clone(),
        };
        command(&mut f, start);
        for _ in 0..1_000 {
            if f.world
                .settlements
                .get(f.town)
                .unwrap()
                .economy
                .production
                .jobs["new-after-cancel"]
                .done
                == 1
            {
                break;
            }
            step(&mut f, &mut writer);
        }
        assert_eq!(
            f.world
                .settlements
                .get(f.town)
                .unwrap()
                .economy
                .production
                .jobs["new-after-cancel"]
                .done,
            1
        );
        // Make real fixture capacity without changing the produced carry.
        f.world
            .block_entities
            .get_mut(&(f.chest[0], f.chest[1], f.chest[2]))
            .unwrap()
            .slots[1] = ItemStack::EMPTY;
        f.world.world.mark_block_entity_dirty(BlockPos {
            x: f.chest[0],
            y: f.chest[1],
            z: f.chest[2],
        });
        beside(&mut f.world, f.body, f.chest);
        for _ in 0..1_000 {
            if f.world
                .settlements
                .get(f.town)
                .unwrap()
                .economy
                .production
                .jobs["new-after-cancel"]
                .delivery_complete()
            {
                break;
            }
            step(&mut f, &mut writer);
        }
        assert!(
            f.world
                .settlements
                .get(f.town)
                .unwrap()
                .economy
                .production
                .jobs["new-after-cancel"]
                .delivery_complete()
        );
        assert_eq!(
            f.world
                .settlements
                .get(f.town)
                .unwrap()
                .economy
                .production
                .jobs["durable"],
            old
        );
        assert_eq!(
            stock_total(&f.world, f.body, "minecraft:oak_planks"),
            5,
            "two retained plus three cancelled unpaid planks remain; only new four ship"
        );
        let expected = image(&mut f);
        let (mut f, writer) = reopen(f, &config, writer);
        assert_eq!(image(&mut f), expected);
        assert_eq!(
            f.world
                .settlements
                .get(f.town)
                .unwrap()
                .economy
                .production
                .jobs["durable"],
            old
        );
        drop(writer);
    }
}

// A native checkpoint image and published WAL postimage are inspected before
// reopening, so a red/green result cannot be attributed to a missing fixture.
fn checkpoint_delivery(f: &Disk, config: &CoreConfig) -> serde_json::Value {
    let store = crate::native_checkpoint::tests::fixture_store(config).unwrap();
    let image = store.read_checkpoint().unwrap();
    let row = &image.globals[&persistence::row_key(f.town)];
    let value: serde_json::Value = serde_json::from_slice(&row.payload).unwrap();
    value["value"]["economy"]["production"]["jobs"]["durable"]["delivery"].clone()
}

fn persist_legacy(f: &mut Disk) {
    let revision = f
        .world
        .settlements
        .get(f.town)
        .unwrap()
        .revision
        .checked_add(1)
        .unwrap();
    let job = f
        .world
        .settlements
        .economy_for_tick(f.town)
        .production
        .jobs
        .get_mut("durable")
        .unwrap();
    job.delivery = None;
    job.paused = Some(MANUAL.into());
    f.world.settlements.finish_economy(f.town, revision, true);
}

fn publish_exact(f: &mut Disk, writer: &mut JournalWriter) {
    let (output, _, _) = crate::tick::run_tick(&mut f.world, &[]);
    let entries = crate::core::journal_entries_for_tick(&mut f.world, &output);
    assert_policy_postimage(&entries, f);
    writer.append(output.tick, output.tick, &entries).unwrap();
    writer.flush().unwrap();
}

// This reads the actual complete journal tail above the native checkpoint,
// independently of the guard and of the test's pre-append image assertion.
fn replayed_delivery(
    config: &CoreConfig,
    registry: Arc<crate::registry::CoreRegistry>,
    town: SettlementId,
) -> Option<bool> {
    let store = crate::native_checkpoint::tests::fixture_store(config).unwrap();
    let reader = store.checkpoint_reader().unwrap();
    let recovered =
        crate::native_checkpoint::recover_with_reader(config, registry, reader).unwrap();
    recovered.dimensions[&Dimension::Overworld]
        .settlements
        .get(&town.0)
        .map(|record| record.economy.production.jobs["durable"].delivery.is_some())
}

fn refuses_legacy_recovery(save_checkpoint: bool) {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = fixture(&config);
    checkpoint(&mut f, &config);
    assert!(
        checkpoint_delivery(&f, &config).is_object(),
        "baseline has tracked delivery"
    );
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    persist_legacy(&mut f);
    publish_exact(&mut f, &mut writer);
    if save_checkpoint {
        checkpoint(&mut f, &config);
        assert!(
            checkpoint_delivery(&f, &config).is_null(),
            "checkpoint carries the legacy image"
        );
    } else {
        assert!(
            checkpoint_delivery(&f, &config).is_object(),
            "only WAL carries the legacy image"
        );
    }
    let town = f.town;
    let registry = Arc::clone(&f.world.registry);
    drop(writer);
    drop(f);
    let replayed = replayed_delivery(&config, registry, town);
    if save_checkpoint {
        assert_ne!(
            replayed,
            Some(true),
            "WAL must not replace the checkpoint with a tracked image"
        );
    } else {
        assert_eq!(
            replayed,
            Some(false),
            "the actual WAL tail must contain the legacy policy"
        );
    }
    let error = crate::core::recover_dimension_state(&config, Dimension::Overworld)
        .err()
        .expect("untracked final work must refuse recovery");
    assert!(
        error
            .to_string()
            .contains("legacy unfinished production job durable"),
        "{error}"
    );
}

#[test]
fn legacy_wal_refuses_native_recovery() {
    refuses_legacy_recovery(false);
}

#[test]
fn legacy_checkpoint_refuses_native_recovery() {
    refuses_legacy_recovery(true);
}

#[test]
fn wal_new_policy_supersedes_legacy_checkpoint_before_guard() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = fixture(&config);
    let delivery = f
        .world
        .settlements
        .get(f.town)
        .unwrap()
        .economy
        .production
        .jobs["durable"]
        .delivery
        .clone();
    assert!(delivery.is_some());
    persist_legacy(&mut f);
    checkpoint(&mut f, &config);
    assert!(checkpoint_delivery(&f, &config).is_null());
    let revision = f
        .world
        .settlements
        .get(f.town)
        .unwrap()
        .revision
        .checked_add(1)
        .unwrap();
    f.world
        .settlements
        .economy_for_tick(f.town)
        .production
        .jobs
        .get_mut("durable")
        .unwrap()
        .delivery = delivery;
    f.world.settlements.finish_economy(f.town, revision, true);
    let mut writer = JournalWriter::open(directory.path(), 1).unwrap();
    publish_exact(&mut f, &mut writer);
    assert!(
        checkpoint_delivery(&f, &config).is_null(),
        "checkpoint still has legacy data"
    );
    let town = f.town;
    let registry = Arc::clone(&f.world.registry);
    drop(writer);
    drop(f);
    assert_eq!(
        replayed_delivery(&config, registry, town),
        Some(true),
        "the actual WAL tail must replace the legacy image"
    );
    let reopened = crate::core::recover_dimension_state(&config, Dimension::Overworld).unwrap();
    assert!(
        reopened
            .settlements
            .get(town)
            .unwrap()
            .economy
            .production
            .jobs["durable"]
            .delivery
            .is_some()
    );
}

// Admit the chain through the real economy owner; only removal of delivery
// emulates the old serialized schema. Native order cancellation is also real.
fn admitted_order(f: &mut Disk) {
    let old = f
        .world
        .settlements
        .get(f.town)
        .unwrap()
        .economy
        .production
        .jobs["durable"]
        .clone();
    command(
        f,
        Request::Cancel {
            job: "durable".into(),
        },
    );
    let step = Step {
        recipe: old.recipe,
        count: 1,
        station: old.station,
        stand_at: old.stand_at,
        building: old.building,
    };
    command(
        f,
        Request::Order {
            order: "legacy-order".into(),
            worker: f.worker,
            steps: vec![step.clone(), step],
        },
    );
}

#[test]
fn legacy_order_final_refuses_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = fixture(&config);
    admitted_order(&mut f);
    let revision = f
        .world
        .settlements
        .get(f.town)
        .unwrap()
        .revision
        .checked_add(1)
        .unwrap();
    let jobs = &mut f.world.settlements.economy_for_tick(f.town).production.jobs;
    assert!(jobs["legacy-order-1"].delivery.is_none());
    jobs.get_mut("legacy-order").unwrap().delivery = None;
    f.world.settlements.finish_economy(f.town, revision, true);
    checkpoint(&mut f, &config);
    drop(f);
    let error = crate::core::recover_dimension_state(&config, Dimension::Overworld)
        .err()
        .expect("legacy order final must refuse recovery");
    assert!(
        error
            .to_string()
            .contains("legacy unfinished production job legacy-order in settlement"),
        "{error}"
    );
}

#[test]
fn cancelled_legacy_order_and_intermediate_recover() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = fixture(&config);
    admitted_order(&mut f);
    let revision = f
        .world
        .settlements
        .get(f.town)
        .unwrap()
        .revision
        .checked_add(1)
        .unwrap();
    f.world
        .settlements
        .economy_for_tick(f.town)
        .production
        .jobs
        .get_mut("legacy-order")
        .unwrap()
        .delivery = None;
    f.world.settlements.finish_economy(f.town, revision, true);
    command(
        &mut f,
        Request::Cancel {
            job: "legacy-order-1".into(),
        },
    );
    let town = f.town;
    for name in ["legacy-order-1", "legacy-order"] {
        let job = &f
            .world
            .settlements
            .get(town)
            .unwrap()
            .economy
            .production
            .jobs[name];
        assert!(job.cancelled && job.delivery.is_none());
    }
    checkpoint(&mut f, &config);
    drop(f);
    let reopened = crate::core::recover_dimension_state(&config, Dimension::Overworld).unwrap();
    for name in ["legacy-order-1", "legacy-order"] {
        let job = &reopened
            .settlements
            .get(town)
            .unwrap()
            .economy
            .production
            .jobs[name];
        assert!(job.cancelled && job.delivery.is_none());
    }
}

#[test]
fn current_order_intermediate_none_remains_valid() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let mut f = fixture(&config);
    admitted_order(&mut f);
    let town = f.town;
    let jobs = &f
        .world
        .settlements
        .get(town)
        .unwrap()
        .economy
        .production
        .jobs;
    assert!(jobs["legacy-order-1"].delivery.is_none());
    assert!(jobs["legacy-order"].delivery.is_some());
    checkpoint(&mut f, &config);
    drop(f);
    let reopened = crate::core::recover_dimension_state(&config, Dimension::Overworld).unwrap();
    let jobs = &reopened
        .settlements
        .get(town)
        .unwrap()
        .economy
        .production
        .jobs;
    assert!(jobs["legacy-order-1"].delivery.is_none());
    assert!(jobs["legacy-order"].delivery.is_some());
}
