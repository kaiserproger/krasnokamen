//! Compact entity post-images: what the emitter writes for an appearance-only
//! mark and for a projectile flight tick, that both tails fold to the same
//! replayed state, and that a tail whose base image is missing fails closed.
use super::*;

/// One metadata byte, exactly what `battle::show_shield` writes.
fn shield(state: &mut TickState, id: EntityId, raised: bool) {
    let slot = state.entities.slot_of(id).unwrap();
    assert!(crate::hostile::set_extra(
        state,
        slot,
        crate::battle::LIVING_FLAGS,
        crate::boundary::EntityDataValue::Byte(0),
        crate::boundary::EntityDataValue::Byte(if raised { 3 } else { 0 }),
    ));
    state.appearance_changed.push(id);
}

/// Flip the shield byte from whatever the body shows now.
fn toggle_shield(state: &mut TickState, id: EntityId) {
    let slot = state.entities.slot_of(id).expect("live body");
    let raised = state
        .entities
        .row(slot)
        .appearance
        .extra
        .iter()
        .any(|(index, value)| {
            *index == crate::battle::LIVING_FLAGS
                && matches!(value, crate::boundary::EntityDataValue::Byte(flags) if *flags != 0)
        });
    shield(state, id, !raised);
}

fn entry_bytes(entry: &JournalEntry) -> usize {
    serde_json::to_vec(entry).unwrap().len() + 1
}

fn bytes(entries: &[JournalEntry]) -> usize {
    entries
        .iter()
        .map(|entry| serde_json::to_vec(entry).unwrap().len() + 1)
        .sum()
}

fn images(entries: &[JournalEntry], id: EntityId) -> usize {
    entries
        .iter()
        .filter(|entry| matches!(entry, JournalEntry::Entity { id: other, .. } if *other == id.0))
        .count()
}

fn compacts(entries: &[JournalEntry], id: EntityId) -> Vec<&JournalEntry> {
    entries
        .iter()
        .filter(|entry| {
            matches!(entry,
                JournalEntry::EntityAppearance { id: other, .. }
                | JournalEntry::ProjectileFlight { id: other, .. } if *other == id.0)
        })
        .collect()
}

#[test]
fn shield_toggle_journals_a_compact_record_not_the_whole_image() {
    let mut state = fixture();
    let cow = spawn_idle_cow(&mut state, 8.5);
    state.appearance_changed.push(cow);
    let spawn = step(&mut state);
    let image = spawn
        .iter()
        .find(|entry| matches!(entry, JournalEntry::Entity { id, .. } if *id == cow.0))
        .expect("a first appearance still needs the whole image");
    assert!(compacts(&spawn, cow).is_empty());

    shield(&mut state, cow, true);
    let raised = step(&mut state);
    assert_eq!(
        images(&raised, cow),
        0,
        "a shield byte must not rewrite the image"
    );
    let compact = compacts(&raised, cow);
    assert_eq!(compact.len(), 1, "one compact post-image per changed body");
    let JournalEntry::EntityAppearance { appearance, .. } = compact[0] else {
        panic!("a non-projectile body takes EntityAppearance");
    };
    let carries = |delta: &crate::journal::EntityAppearanceDelta| {
        let flag = crate::journal::EntityDataValueImage::Byte(3);
        delta.extra_changes.iter().any(|(index, value)| {
            *index == crate::battle::LIVING_FLAGS && value.as_ref() == Some(&flag)
        }) || delta.extra.as_ref().is_some_and(|extra| {
            extra
                .iter()
                .any(|(index, value)| *index == crate::battle::LIVING_FLAGS && *value == flag)
        })
    };
    assert!(
        carries(appearance),
        "the record carries the changed metadata entry"
    );
    assert!(appearance.equipment.is_none(), "untouched parts stay out");
    let compact_bytes = entry_bytes(compact[0]);
    let image_bytes = entry_bytes(image);
    assert!(
        compact_bytes < image_bytes,
        "compact {compact_bytes} B vs image {image_bytes} B"
    );

    // The entry exists now, so the next toggle is a per-index update.
    shield(&mut state, cow, false);
    let lowered = step(&mut state);
    assert_eq!(images(&lowered, cow), 0);
    assert_eq!(compacts(&lowered, cow).len(), 1);
    let JournalEntry::EntityAppearance { appearance, .. } = compacts(&lowered, cow)[0] else {
        panic!("appearance record");
    };
    assert!(
        appearance.extra.is_none(),
        "an existing entry updates in place: {appearance:?}"
    );
    let lowered_bytes = entry_bytes(compacts(&lowered, cow)[0]);
    assert!(
        lowered_bytes * 4 < image_bytes,
        "a per-index toggle is {lowered_bytes} B vs the image {image_bytes} B"
    );
}

#[test]
fn projectile_flight_journals_a_compact_record_per_tick() {
    let (mut state, _player, centre) = crate::hostile_tests::setup(12.0);
    let id = crate::projectiles::launch(
        &mut state,
        crate::projectiles::ProjectileState::new(
            crate::projectiles::ProjectileKind::Arrow,
            None,
            mc_data::ItemStack::EMPTY,
        ),
        [centre[0] + 2.5, centre[1] + 3.0, centre[2]],
        // A short hop inside one chunk: a crossing legitimately needs the image.
        [0.0, 0.0, 0.4],
    )
    .unwrap();
    let spawn = step(&mut state);
    assert_eq!(images(&spawn, id), 1, "the launch image seeds the base");
    let image_bytes = spawn
        .iter()
        .find(|entry| matches!(entry, JournalEntry::Entity { id: other, .. } if *other == id.0))
        .map(|entry| serde_json::to_vec(entry).unwrap().len() + 1)
        .expect("launch image");

    let mut records = 0;
    let mut record_bytes = 0;
    let mut landings = 0;
    for _ in 0..40 {
        let Some(slot) = state.entities.slot_of(id) else {
            break;
        };
        if state
            .entities
            .row(slot)
            .projectile
            .as_deref()
            .is_some_and(|record| record.stuck)
        {
            break;
        }
        let entries = step(&mut state);
        assert_eq!(
            images(&entries, id),
            0,
            "a flight tick must not write the image"
        );
        for entry in compacts(&entries, id) {
            let JournalEntry::ProjectileFlight {
                stuck, state, pos, ..
            } = entry
            else {
                panic!("a projectile body takes ProjectileFlight, got {entry:?}");
            };
            records += 1;
            record_bytes += serde_json::to_vec(entry).unwrap().len() + 1;
            assert!(pos.iter().all(|value| value.is_finite()));
            if stuck.is_some() || state.is_some() {
                landings += 1;
            }
        }
    }
    assert!(records >= 3, "the arrow must journal its flight: {records}");
    assert_eq!(
        landings, 1,
        "the landing transition is the one state-carrying tick"
    );
    assert!(
        record_bytes / records.max(1) * 2 < image_bytes,
        "flight {} B/record vs one image {image_bytes} B",
        record_bytes / records.max(1)
    );
}

#[test]
fn projectile_flight_keeps_the_whole_state_when_a_payload_field_changes() {
    let (mut state, _player, centre) = crate::hostile_tests::setup(12.0);
    let id = crate::projectiles::launch(
        &mut state,
        crate::projectiles::ProjectileState::new(
            crate::projectiles::ProjectileKind::Arrow,
            None,
            mc_data::ItemStack::EMPTY,
        ),
        [centre[0], centre[1] + 3.0, centre[2]],
        [0.0, 0.0, 1.2],
    )
    .unwrap();
    step(&mut state);
    let slot = state.entities.slot_of(id).unwrap();
    // A hit ledger entry is not the age or the landing transition: the record
    // must carry the whole absolute state, not a delta of it.
    state
        .entities
        .row_mut(slot)
        .projectile
        .as_mut()
        .unwrap()
        .hit
        .push(uuid::Uuid::from_u128(7));
    state.appearance_changed.push(id);
    let entries = step(&mut state);
    let compact = compacts(&entries, id);
    assert_eq!(compact.len(), 1);
    let JournalEntry::ProjectileFlight { state, .. } = compact[0] else {
        panic!("flight record");
    };
    let retained = state.as_deref().expect("a payload change keeps the state");
    assert_eq!(retained.hit, vec![uuid::Uuid::from_u128(7)]);
}

/// The same scripted tail journaled both ways: mob metadata (shield), an
/// equipment change, a hurt, a projectile moving each tick and a removal.
/// No AI runs, so both runs mutate identically.
fn scenario(force_full: bool) -> Vec<(u64, u64, Vec<JournalEntry>)> {
    let mut state = fixture();
    if force_full {
        crate::journal_touched::force_full_images(&mut state);
    }
    // An active chunk exercises the simulation-clock projection both paths share
    // (a residency interval, not wall time).
    let chunk = crate::boundary::ChunkPos { x: 0, z: 0 };
    state
        .world
        .ensure_blocking(&[(chunk, 0)].into_iter().collect());
    state.tickets.insert("kms:compact".into(), (chunk, 0));
    state.refresh_active_chunks(state.tick);
    let husk_type = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:husk").unwrap())
        .unwrap();
    let husk = state.spawn_mob(husk_type, [8.5, 65.0, 8.5], 20.0);
    let slot = state.entities.slot_of(husk).unwrap();
    state.entities.row_mut(slot).persistent = true;
    state.entities.row_mut(slot).controls.no_ai = true;
    // Bodies whose clocks are still running: a baby with child ticks, a breeding
    // cow with a breed cooldown, a chicken on its harvest clock and a wandering
    // villager whose age decays. Appearance changes land mid-countdown, so the
    // state/villager projection in both fold paths is exercised, not just held.
    let baby = state.spawn_mob(state.registry.cow_type_id, [10.5, 65.0, 8.5], 10.0);
    let slot = state.entities.slot_of(baby).unwrap();
    let row = state.entities.row_mut(slot);
    row.persistent = true;
    row.controls.no_ai = true;
    Arc::make_mut(&mut row.appearance).baby = true;
    row.entity_life.child_ticks = 400;
    let breeding = state.spawn_mob(state.registry.cow_type_id, [12.5, 65.0, 8.5], 10.0);
    let slot = state.entities.slot_of(breeding).unwrap();
    let row = state.entities.row_mut(slot);
    row.persistent = true;
    row.controls.no_ai = true;
    row.breed_cooldown = 200;
    row.love = 100;
    let chicken_type = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:chicken").unwrap())
        .unwrap();
    let chicken = state.spawn_mob(chicken_type, [14.5, 65.0, 8.5], 4.0);
    let slot = state.entities.slot_of(chicken).unwrap();
    let row = state.entities.row_mut(slot);
    row.persistent = true;
    row.controls.no_ai = true;
    row.entity_life.harvest_ticks = 200;
    let villager = state.spawn_mob(villager_type(&state), [6.5, 65.0, 10.5], 20.0);
    let slot = state.entities.slot_of(villager).unwrap();
    let row = state.entities.row_mut(slot);
    row.persistent = true;
    let state_villager = crate::villager::VillagerState {
        wandering: true,
        age: 50,
        ..Default::default()
    };
    row.villager = Some(Box::new(state_villager));
    state.appearance_changed.push(husk);
    state.appearance_changed.push(baby);
    state.appearance_changed.push(breeding);
    state.appearance_changed.push(chicken);
    state.appearance_changed.push(villager);
    let sword = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:iron_sword").unwrap())
        .unwrap();
    let mut batches = Vec::new();
    for index in 0..14u64 {
        if index == 3 || index == 9 || index == 12 {
            // Mid-countdown metadata changes on the running-clock bodies.
            for id in [baby, breeding, chicken, villager] {
                if state.entities.slot_of(id).is_some() {
                    toggle_shield(&mut state, id);
                }
            }
        }
        if index == 11 {
            // The chunk leaves residency; its clocks stop until reactivation.
            state.tickets.clear();
            state.refresh_active_chunks(state.tick);
        }
        if index == 13 {
            state.tickets.insert("kms:compact".into(), (chunk, 0));
            state.refresh_active_chunks(state.tick);
        }
        match index {
            2 | 6 | 10 => shield(&mut state, husk, index % 8 == 2),
            4 => {
                let slot = state.entities.slot_of(husk).unwrap();
                Arc::make_mut(&mut state.entities.row_mut(slot).appearance).equipment[0] =
                    Some(mc_data::ItemStack::new(sword, 1));
                state.appearance_changed.push(husk);
            }
            8 => {
                let slot = state.entities.slot_of(husk).unwrap();
                state.entities.row_mut(slot).health = 12.5;
                state.appearance_changed.push(husk);
            }
            _ => {}
        }
        let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
        if index == 7 || index == 8 {
            // A checkpoint capture between batches, and a refused capture that
            // keeps the bases cleared: the tail restarts with full images and the
            // byte accounting includes both reseeds.
            crate::journal_touched::clear_field_baselines(&mut state);
        }
        let entries = journal_entries_for_tick(&mut state, &output).unwrap();
        crate::journal_touched::assert_baselines_complete(&state, output.tick);
        batches.push((output.tick, output.tick, entries));
    }
    batches
}

fn canonical(replay: &Replay) -> serde_json::Value {
    let mut value = serde_json::to_value(&replay.entities).unwrap();
    for record in value.as_object_mut().unwrap().values_mut() {
        // `EntityStateImage::no_action_time` is checkpoint-only by contract: the
        // touched-set comparison ignores it, so no WAL record refreshes it and a
        // full image only appears to because it carries the whole state image.
        // Every other field, including each record's clock anchor, must agree.
        if let Some(state) = record
            .get_mut("state")
            .and_then(|state| state.as_object_mut())
        {
            state.remove("no_action_time");
            // `Controls::journal_eq` deliberately ignores a non-zero mid-air
            // accumulator too: it rides the next image and checkpoints, never a
            // record of its own.
            if let Some(controls) = state
                .get_mut("controls")
                .and_then(|controls| controls.as_object_mut())
            {
                controls.remove("fall_distance");
            }
        }
    }
    value
}

/// Report the id and field that differ, instead of two truncated JSON blobs.
fn assert_same_state(compact: &serde_json::Value, full: &serde_json::Value) {
    let empty = serde_json::Map::new();
    let (compact, full) = (
        compact.as_object().unwrap_or(&empty),
        full.as_object().unwrap_or(&empty),
    );
    let mut ids: Vec<&String> = compact.keys().chain(full.keys()).collect();
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        let left = compact.get(id);
        let right = full.get(id);
        if left == right {
            continue;
        }
        let (Some(left), Some(right)) = (
            left.and_then(|v| v.as_object()),
            right.and_then(|v| v.as_object()),
        ) else {
            panic!("id {id}: compact {left:?} vs full {right:?}");
        };
        let mut keys: Vec<&String> = left.keys().chain(right.keys()).collect();
        keys.sort_unstable();
        keys.dedup();
        for key in keys {
            if left.get(key) != right.get(key) {
                panic!(
                    "id {id} differs in {key}:\n compact {:?}\n full    {:?}",
                    left.get(key),
                    right.get(key)
                );
            }
        }
    }
}

#[test]
fn compact_and_full_tails_replay_to_the_same_state() {
    let full = scenario(true);
    let compact = scenario(false);
    let entries = |batches: &[(u64, u64, Vec<JournalEntry>)]| -> usize {
        batches.iter().map(|(_, _, entries)| bytes(entries)).sum()
    };
    assert!(
        !full
            .iter()
            .flat_map(|(_, _, entries)| entries)
            .any(|entry| matches!(
                entry,
                JournalEntry::EntityAppearance { .. } | JournalEntry::ProjectileFlight { .. }
            ))
    );
    let count = |variant: fn(&JournalEntry) -> bool| {
        compact
            .iter()
            .flat_map(|(_, _, entries)| entries)
            .filter(|entry| variant(entry))
            .count()
    };
    // The focused tests own the exact counts; here every record class must appear
    // so the equality below is not vacuous.
    let appearances = count(|entry| matches!(entry, JournalEntry::EntityAppearance { .. }));
    let images = count(|entry| matches!(entry, JournalEntry::Entity { .. }));
    assert!(
        appearances > 0,
        "shield/equipment changes take the appearance record"
    );
    assert!(
        images > 0,
        "spawns and a post-capture change keep the whole image"
    );
    println!("compact tail: {appearances} appearance, {images} image records");

    let full_replay = replay_batches(Replay::default(), &full);
    let compact_replay = replay_batches(Replay::default(), &compact);
    assert!(crate::journal::validate_replay(&full_replay).is_ok());
    assert!(crate::journal::validate_replay(&compact_replay).is_ok());
    assert_same_state(&canonical(&compact_replay), &canonical(&full_replay));
    assert_eq!(compact_replay.entity_removals, full_replay.entity_removals);
    assert_eq!(compact_replay.entity_changes, full_replay.entity_changes);
    // The scenario is small (spawns and two reseeds are whole images); the real
    // ratio on a battle tail belongs to the CoreWalBloat run, not this test.
    assert!(
        entries(&compact) < entries(&full),
        "compact {} B vs full {} B",
        entries(&compact),
        entries(&full)
    );
}

#[test]
fn compact_record_without_its_base_refuses_recovery() {
    let tail = vec![(
        9,
        9,
        vec![JournalEntry::EntityAppearance {
            id: 7,
            tick: 9,
            age: 0,
            health: 20.0,
            alive: true,
            active_effects: Default::default(),
            appearance: Default::default(),
        }],
    )];
    let replay = replay_batches(Replay::default(), &tail);
    let error = crate::journal::validate_replay(&replay)
        .expect_err("a delta without its image must not be served");
    assert!(
        format!("{error}").contains("without an earlier full image"),
        "{error}"
    );
    let tail = vec![(
        9,
        9,
        vec![JournalEntry::ProjectileFlight {
            id: 7,
            tick: 9,
            age: 0,
            health: 1.0,
            alive: true,

            active_effects: Default::default(),
            appearance: Default::default(),
            pos: [0.0; 3],
            vel: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            projectile_age: 3,
            stuck: None,
            grounded_at_age: crate::journal::DeltaValue::Unchanged,
            state: None,
        }],
    )];
    let replay = replay_batches(Replay::default(), &tail);
    assert!(crate::journal::validate_replay(&replay).is_err());
}

#[test]
fn checkpoint_baseline_reset_forces_the_next_full_image() {
    let mut state = fixture();
    let cow = spawn_idle_cow(&mut state, 8.5);
    state.appearance_changed.push(cow);
    step(&mut state);
    shield(&mut state, cow, true);
    assert_eq!(images(&step(&mut state), cow), 0);
    // A checkpoint stores no living flags, so the tail after a capture must
    // start from the whole image again.
    crate::journal_touched::clear_field_baselines(&mut state);
    shield(&mut state, cow, false);
    let entries = step(&mut state);
    assert_eq!(images(&entries, cow), 1, "the capture reseeds the base");
    assert!(compacts(&entries, cow).is_empty());
}

/// Tri-state fields must survive a real batch encode/decode: `null` decodes to
/// the outer `None` with plain serde, which would silently keep a cleared value.
#[test]
fn delta_values_round_trip_through_a_batch() {
    fn flight(
        tick: u64,
        stuck: Option<bool>,
        grounded_at_age: crate::journal::DeltaValue<u32>,
    ) -> JournalEntry {
        JournalEntry::ProjectileFlight {
            id: 3,
            tick,
            age: 7,
            health: 1.0,
            alive: true,

            active_effects: Default::default(),
            appearance: Default::default(),
            pos: [1.0, 2.0, 3.0],
            vel: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            projectile_age: 9,
            stuck,
            grounded_at_age,
            state: None,
        }
    }
    let directory =
        std::env::temp_dir().join(format!("mc-core-compact-codec-{}", uuid::Uuid::new_v4()));
    let landed = flight(5, Some(true), crate::journal::DeltaValue::Set(9));
    let ungrounded = flight(6, Some(false), crate::journal::DeltaValue::Clear);
    let renamed = JournalEntry::EntityAppearance {
        id: 4,
        tick: 6,
        age: 0,
        health: 20.0,
        alive: true,
        active_effects: Default::default(),
        appearance: crate::journal::EntityAppearanceDelta {
            custom_name: crate::journal::DeltaValue::Clear,
            item: crate::journal::DeltaValue::Set(mc_data::ItemStack::EMPTY),
            ..Default::default()
        },
    };
    let entries = vec![landed.clone(), ungrounded.clone(), renamed.clone()];
    let mut writer =
        JournalWriter::open(&directory, 1, crate::journal::WalIdentity::fixture()).unwrap();
    writer.append(5, 6, &entries).unwrap();
    writer.flush().unwrap();
    drop(writer);
    // The tail starts at the checkpoint tick: 5..6 follows 4.
    let batches = read_complete_batches_after(
        &directory.join("journal"),
        4,
        crate::journal::WalIdentity::fixture(),
    )
    .unwrap();
    let decoded = &batches[0].2;
    assert_eq!(decoded.len(), 3);
    assert_eq!(decoded[0], landed, "landing keeps an absolute ground age");
    assert_eq!(
        decoded[1], ungrounded,
        "leaving the ground is a clear, not an absent field"
    );
    assert_eq!(
        decoded[2], renamed,
        "a cleared name and a set item round-trip"
    );

    let mut projectile = crate::projectiles::ProjectileState::new(
        crate::projectiles::ProjectileKind::Arrow,
        None,
        mc_data::ItemStack::EMPTY,
    );
    projectile.stuck = true;
    projectile.grounded_at_age = Some(9);
    let JournalEntry::ProjectileFlight {
        grounded_at_age, ..
    } = &decoded[1]
    else {
        panic!("flight record");
    };
    crate::journal::apply_delta(grounded_at_age, &mut projectile.grounded_at_age);
    assert_eq!(
        projectile.grounded_at_age, None,
        "the clear reaches the state"
    );

    let mut appearance = crate::journal::EntityAppearanceImage {
        custom_name: Some("named".into()),
        ..Default::default()
    };
    let JournalEntry::EntityAppearance {
        appearance: delta, ..
    } = &decoded[2]
    else {
        panic!("appearance record");
    };
    delta.apply(&mut appearance);
    assert_eq!(
        appearance.custom_name, None,
        "the cleared name reaches the state"
    );
    let _ = std::fs::remove_dir_all(directory);
}

/// A checkpoint populates `Replay.entities` without a WAL image; a compact record
/// must not fold onto it (committee F1).
#[test]
fn compact_record_on_a_checkpoint_image_refuses_recovery() {
    let mut state = fixture();
    let cow = spawn_idle_cow(&mut state, 8.5);
    state.appearance_changed.push(cow);
    let spawn = step(&mut state);
    assert_eq!(images(&spawn, cow), 1, "the spawn image seeds the tail");
    let mut replay = replay_batches(Replay::default(), &[(1, 1, spawn)]);
    assert!(replay.tail_images.contains(&cow.0));
    // A checkpoint boundary drops the tail: the record survives, the proof does not.
    replay.tail_images.clear();
    shield(&mut state, cow, true);
    let entries = step(&mut state);
    assert_eq!(compacts(&entries, cow).len(), 1);
    let replay = replay_batches(replay, &[(2, 2, entries)]);
    let error = crate::journal::validate_replay(&replay)
        .expect_err("a delta must not fold onto a checkpoint image");
    assert!(
        format!("{error}").contains("without an earlier full image"),
        "{error}"
    );
}

/// The same flight journaled both ways must fold to the same projectile state:
/// the dedicated record class of the compact tail, on a real flight path.
#[test]
fn projectile_flight_compact_and_full_tails_replay_to_the_same_state() {
    fn flight_tail(force_full: bool) -> Vec<(u64, u64, Vec<JournalEntry>)> {
        let (mut state, _player, centre) = crate::hostile_tests::setup(12.0);
        if force_full {
            crate::journal_touched::force_full_images(&mut state);
        }
        crate::projectiles::launch(
            &mut state,
            crate::projectiles::ProjectileState::new(
                crate::projectiles::ProjectileKind::Arrow,
                None,
                mc_data::ItemStack::EMPTY,
            ),
            [centre[0] + 2.5, centre[1] + 3.0, centre[2]],
            [0.0, 0.0, 0.4],
        )
        .unwrap();
        let mut batches = Vec::new();
        for _ in 0..10 {
            let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
            let entries = journal_entries_for_tick(&mut state, &output).unwrap();
            crate::journal_touched::assert_baselines_complete(&state, output.tick);
            batches.push((output.tick, output.tick, entries));
        }
        batches
    }
    let full = flight_tail(true);
    let compact = flight_tail(false);
    let flights = compact
        .iter()
        .flat_map(|(_, _, entries)| entries)
        .filter(|entry| matches!(entry, JournalEntry::ProjectileFlight { .. }))
        .count();
    assert!(flights > 0, "the arrow journals its flight compactly");
    assert!(
        !full
            .iter()
            .flat_map(|(_, _, entries)| entries)
            .any(|entry| matches!(entry, JournalEntry::ProjectileFlight { .. }))
    );
    let full_replay = replay_batches(Replay::default(), &full);
    let compact_replay = replay_batches(Replay::default(), &compact);
    assert!(crate::journal::validate_replay(&full_replay).is_ok());
    assert!(crate::journal::validate_replay(&compact_replay).is_ok());
    assert_same_state(&canonical(&compact_replay), &canonical(&full_replay));
}

/// The test-only fence must see an appearance write that no mark carries.
#[test]
fn unmarked_appearance_change_is_caught_by_the_fence() {
    let mut state = fixture();
    let cow = spawn_idle_cow(&mut state, 8.5);
    state.appearance_changed.push(cow);
    let _ = step(&mut state);
    // The tick drains the marks; this write leaves no mark behind.
    let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
    let slot = state.entities.slot_of(cow).unwrap();
    assert!(crate::hostile::set_extra(
        &mut state,
        slot,
        crate::battle::LIVING_FLAGS,
        crate::boundary::EntityDataValue::Byte(0),
        crate::boundary::EntityDataValue::Byte(3),
    ));
    let _ = journal_entries_for_tick(&mut state, &output).unwrap();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::journal_touched::assert_baselines_complete(&state, output.tick)
    }));
    assert!(
        caught.is_err(),
        "an appearance change with no record must fail the fence"
    );
}

/// The pickup-delay guard: the compact record cannot carry it, so a change is a
/// whole image.
#[test]
fn pickup_delay_change_forces_a_full_image() {
    let mut state = fixture();
    let cow = spawn_idle_cow(&mut state, 8.5);
    state.appearance_changed.push(cow);
    step(&mut state);
    let slot = state.entities.slot_of(cow).unwrap();
    state.entities.body_mut(slot).pickup_delay = 40;
    state.entities.row_mut(slot).health = 9.0;
    state.appearance_changed.push(cow);
    let entries = step(&mut state);
    assert_eq!(
        images(&entries, cow),
        1,
        "a pickup-delay change keeps the image"
    );
    assert!(compacts(&entries, cow).is_empty());
}
