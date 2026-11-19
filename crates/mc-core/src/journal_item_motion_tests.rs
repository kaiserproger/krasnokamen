use super::*;
use mc_data::ItemStack;

fn stone(state: &TickState) -> u32 {
    state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap()
}

fn water_item(count: i32, x: f64) -> (TickState, EntityId) {
    let (mut state, _, centre) = crate::hostile_tests::setup(12.0);
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    for x in 14..=21 {
        for z in centre[2] as i32 - 1..=centre[2] as i32 + 1 {
            state
                .world
                .set_block(x, 200, z, state.registry.states.stone);
            for y in 201..=202 {
                state.world.set_block(x, y, z, state.registry.states.water);
            }
        }
    }
    state.world.fold_overlay();
    let stack = ItemStack::new(stone(&state), count);
    let item = crate::tick::spawn_item_entity(&mut state, [x, 201.0, centre[2]], stack);
    let slot = state.entities.slot_of(item).unwrap();
    state.entities.row_mut(slot).age = 123;
    state.entities.body_mut(slot).pickup_delay = 100;
    state.entities.set_vel(slot, [6.0, 0.0, 0.0]);
    (state, item)
}

fn assert_item(replay: &Replay, state: &TickState, item: EntityId, count: i32) {
    let slot = state.entities.slot_of(item).unwrap();
    let row = state.entities.row(slot);
    let saved = &replay.entities[&item.0];
    assert_eq!(saved.uuid, row.uuid.to_string());
    assert_eq!(
        saved.item.as_ref().unwrap(),
        &row.item.as_ref().unwrap().full_stack()
    );
    assert_eq!(saved.item.as_ref().unwrap().count, count);
    assert_eq!(saved.pos, state.entities.pos(slot));
    assert_eq!(saved.vel, state.entities.vel(slot));
    assert_eq!(saved.ground, state.entities.ground(slot));
    assert_eq!(saved.age, row.age);
    assert_eq!(saved.pickup_delay, state.entities.body(slot).pickup_delay);
    let stacks: Vec<_> = replay
        .entities
        .iter()
        .filter(|(_, row)| row.item.is_some())
        .collect();
    assert_eq!(stacks.len(), 1);
    assert_eq!(*stacks[0].0, item.0);
    assert_eq!(stacks[0].1.item.as_ref().unwrap().count, count);
}

#[test]
fn finite_water_stack_crosses_chunks_and_late_motion_cannot_resurrect_a_removed_stack() {
    let root = tempfile::tempdir().unwrap();
    let (mut state, item) = water_item(17, 15.4);
    let slot = state.entities.slot_of(item).unwrap();
    let mut writer = JournalWriter::open(root.path(), 1).unwrap();
    let birth = step(&mut state);
    writer.append(1, state.tick, &birth).unwrap();
    let initial = state.entities.pos(slot);
    assert_eq!((initial[0].floor() as i32).div_euclid(16), 0);
    let mut crossed = false;
    let mut old_motion = None;
    for _ in 0..8 {
        let previous_chunk = (state.entities.pos(slot)[0].floor() as i32).div_euclid(16);
        let entries = step(&mut state);
        let motion = entries
            .iter()
            .find(|entry| {
                matches!(entry,
                    JournalEntry::ItemMotion { id, .. } if *id == item.0
                )
            })
            .expect("native water motion must produce a compact pose");
        if old_motion.is_none() {
            old_motion = Some(motion.clone());
        }
        if (state.entities.pos(slot)[0].floor() as i32).div_euclid(16) != previous_chunk {
            crossed = true;
            assert!(
                entries.iter().any(|entry| matches!(entry,
                    JournalEntry::Entity { id, item: Some(stack), .. }
                        if *id == item.0 && stack.count == 17
                )),
                "the chunk crossing also carries the full stack image"
            );
        }
        writer.append(state.tick, state.tick, &entries).unwrap();
    }
    assert!(crossed);
    assert!(state.entities.pos(slot)[0] > initial[0]);
    assert!(
        state.entities.vel(slot)[1] > 0.0,
        "water buoyancy, not a manually assigned pose"
    );
    let [x, y, z] = state.entities.pos(slot).map(|value| value.floor() as i32);
    assert_eq!(
        state.world.block_at(x, y, z),
        Some(state.registry.states.water)
    );
    writer.flush().unwrap();
    let batches = read_complete_batches_after(&root.path().join("journal"), 0).unwrap();
    let replay = replay_batches(Replay::default(), &batches);
    assert_item(&replay, &state, item, 17);
    let replay = replay_batches(replay, &batches);
    assert_item(&replay, &state, item, 17);

    state.entities.row_mut(slot).age = 5_999;
    let removal = step(&mut state);
    assert!(
        state.entities.slot_of_any(item).is_none(),
        "native item expiry removes the row"
    );
    assert!(removal.iter().any(|entry| matches!(entry,
        JournalEntry::RemoveEntity { id, .. } if *id == item.0
    )));
    writer.append(state.tick, state.tick, &removal).unwrap();
    let late_tick = state.tick + 1;
    let mut late = old_motion.unwrap();
    if let JournalEntry::ItemMotion { tick, .. } = &mut late {
        *tick = late_tick;
    }
    writer.append(late_tick, late_tick, &[late]).unwrap();
    writer.flush().unwrap();
    drop(writer);
    let batches = read_complete_batches_after(&root.path().join("journal"), 0).unwrap();
    let replay = replay_batches(Replay::default(), &batches);
    assert!(!replay.entities.contains_key(&item.0));
    let replay = replay_batches(replay, &batches);
    assert!(!replay.entities.contains_key(&item.0));
    assert_eq!(
        replay
            .entities
            .values()
            .filter_map(|row| row.item.as_ref())
            .map(|stack| stack.count)
            .sum::<i32>(),
        0
    );
}

#[test]
fn partial_pickup_and_motion_replay_preserve_inventory_plus_exact_ground_remainder() {
    let root = tempfile::tempdir().unwrap();
    let (mut state, player, centre) = crate::hostile_tests::setup(0.0);
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    let stone = stone(&state);
    let inventory = &mut state.players.get_mut(&player).unwrap().inventory;
    for slot in &mut inventory.slots {
        *slot = ItemStack::new(stone, 64);
    }
    inventory.slots[0].count = 60;
    let before = inventory.slots.iter().map(|stack| stack.count).sum::<i32>();
    let item = crate::tick::spawn_item_entity(&mut state, centre, ItemStack::new(stone, 10));
    let slot = state.entities.slot_of(item).unwrap();
    state.entities.body_mut(slot).pickup_delay = 100;
    state.entities.set_vel(slot, [2.0, 0.0, 0.0]);
    let mut writer = JournalWriter::open(root.path(), 1).unwrap();
    let birth = step(&mut state);
    writer.append(1, state.tick, &birth).unwrap();
    let before_position = state.entities.pos(slot);
    state.entities.body_mut(slot).pickup_delay = 0;
    let entries = step(&mut state);
    assert!(entries.iter().any(|entry| matches!(entry,
        JournalEntry::ItemMotion { id, .. } if *id == item.0
    )));
    assert!(
        entries.iter().any(|entry| matches!(entry,
            JournalEntry::Entity { id, item: Some(stack), .. } if *id == item.0 && stack.count == 6
        )),
        "the same tick must carry the changed stack as well as its motion"
    );
    assert_ne!(state.entities.pos(slot), before_position);
    assert_eq!(state.entities.row(slot).item.as_ref().unwrap().count, 6);
    let inventory_count = state.players[&player]
        .inventory
        .slots
        .iter()
        .map(|stack| stack.count)
        .sum::<i32>();
    assert_eq!(inventory_count, before + 4);
    assert_eq!(inventory_count + 6, before + 10);
    writer.append(state.tick, state.tick, &entries).unwrap();
    let motion = step(&mut state);
    assert!(motion.iter().any(|entry| matches!(entry,
        JournalEntry::ItemMotion { id, .. } if *id == item.0
    )));
    writer.append(state.tick, state.tick, &motion).unwrap();
    writer.flush().unwrap();
    drop(writer);
    let batches = read_complete_batches_after(&root.path().join("journal"), 0).unwrap();
    let uuid = state
        .entities
        .row(state.entities.slot_of(player).unwrap())
        .uuid
        .to_string();
    let mut replay = replay_batches(Replay::default(), &batches);
    for _ in 0..2 {
        assert_item(&replay, &state, item, 6);
        let inventory_count = replay.players[&uuid]
            .inventory
            .iter()
            .map(|stack| stack.count)
            .sum::<i32>();
        assert_eq!(inventory_count, before + 4);
        assert_eq!(
            inventory_count + replay.entities[&item.0].item.as_ref().unwrap().count,
            before + 10
        );
        replay = replay_batches(replay, &batches);
    }
}

#[test]
fn post_checkpoint_compact_water_motion_reuses_the_native_stack_baseline_exactly() {
    let root = tempfile::tempdir().unwrap();
    let (mut state, item) = water_item(23, 18.0);
    let mut writer = JournalWriter::open(root.path(), 1).unwrap();
    let birth = step(&mut state);
    writer.append(1, state.tick, &birth).unwrap();
    let checkpoint_tick = state.tick;
    let (_, chunks, _) = crate::core::checkpoint_data_for(&state, checkpoint_tick);
    let native = crate::native_checkpoint::encode_entities(
        crate::dimensions::Dimension::Overworld,
        &chunks,
        &state.registry.items,
        checkpoint_tick,
    )
    .unwrap();
    let config = crate::CoreConfig::new(root.path().join("checkpoint"), 42);
    crate::native_checkpoint::tests::write_manifest(
        &config,
        &crate::journal::CheckpointManifest {
            tick: checkpoint_tick,
            climate: Some(crate::weather::ClimateImage::capture(&state)),
            ..Default::default()
        },
    )
    .unwrap();
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let base = store.read_checkpoint().unwrap();
    store
        .commit_checkpoint(
            mc_world::native::NativeCheckpoint {
                tick: checkpoint_tick,
                previous_tick: base.tick,
                chunks: &[],
                entities: &native,
                players: &[],
                globals: &base.globals,
            },
            &state.registry.blocks,
            &state.registry.items,
        )
        .unwrap();
    drop(store);
    let mut checkpoint = crate::native_checkpoint::recover_offline(&config).unwrap();
    let baseline = checkpoint
        .dimensions
        .remove(&crate::dimensions::Dimension::Overworld)
        .unwrap();
    assert_item(&baseline, &state, item, 23);
    let initial = baseline.entities[&item.0].pos;
    for _ in 0..4 {
        let entries = step(&mut state);
        assert!(entries.iter().any(|entry| matches!(entry,
            JournalEntry::ItemMotion { id, .. } if *id == item.0
        )));
        assert!(
            !entries.iter().any(|entry| matches!(entry,
                JournalEntry::Entity { id, .. } if *id == item.0
            )),
            "the WAL tail must depend on the checkpoint's stack, not rewrite it"
        );
        writer.append(state.tick, state.tick, &entries).unwrap();
    }
    writer.flush().unwrap();
    drop(writer);
    let batches =
        read_complete_batches_after(&root.path().join("journal"), checkpoint_tick).unwrap();
    assert_eq!(batches.len(), 4);
    let replay = replay_batches(baseline, &batches);
    crate::journal::validate_replay(&replay).unwrap();
    assert_ne!(replay.entities[&item.0].pos, initial);
    assert_item(&replay, &state, item, 23);
    let replay = replay_batches(replay, &batches);
    assert_item(&replay, &state, item, 23);
}
