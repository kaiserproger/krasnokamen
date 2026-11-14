use super::*;
use crate::mounts::tests::{fixture, slot, spawn};
use crate::tick::run_tick;

#[test]
fn adult_sniffer_digs_seed_and_enters_cooldown() {
    let (mut state, actor, pos) = fixture();
    let actor_slot = slot(&state, actor);
    state.entities.set_pos(actor_slot, [pos[0] + 7.0, pos[1], pos[2] + 7.0]);
    let dirt = state.registry.blocks.block(&mc_data::Identifier::parse("minecraft:dirt").unwrap()).unwrap().default.0;
    state.world.set_block(pos[0] as i32, pos[1] as i32 - 1, pos[2] as i32, dirt);
    state.world.fold_overlay();
    let sniffer = spawn(&mut state, "sniffer", pos);
    let sniffer_slot = slot(&state, sniffer);
    let block = state.registry.blocks.by_id(mc_world::BlockStateId(dirt)).unwrap();
    assert!(state.registry.tags.contains_raw_id("minecraft:block", "minecraft:sniffer_diggable_block", block.block.raw_id),
        "embedded sniffer tag must include dirt");
    assert_eq!(state.world.block_at(pos[0] as i32, pos[1] as i32, pos[2] as i32), Some(state.registry.states.air));
    assert!(diggable(&state, [pos[0] as i32, pos[1] as i32 - 1, pos[2] as i32], &[]),
        "the fixture's dirt must belong to the sniffer diggable tag");
    state.tick = (200 - (sniffer.0 as u64 % 200)) % 200;
    let mut saw_digging = false;
    for _ in 0..300 {
        run_tick(&mut state, &[]);
        let row = state.entities.row(sniffer_slot);
        saw_digging |= row.appearance.extra.contains(&(18, EntityDataValue::SnifferState(5)));
        if row.entity_life.sniffer.as_ref().is_some_and(|s| s.cooldown > 0) { break; }
    }
    let sniffer_state = state.entities.row(sniffer_slot).entity_life.sniffer.as_ref().unwrap();
    assert!(saw_digging);
    assert!(sniffer_state.cooldown > 9000);
    assert_eq!(sniffer_state.explored.len(), 1);
    assert!(state.entities.slots().any(|slot| state.entities.row(slot).item.as_ref().is_some_and(|item| {
        state.registry.items.name_of(item.item_id).is_some_and(|name| matches!(name.as_str(), "minecraft:torchflower_seeds" | "minecraft:pitcher_pod"))
    })));
}

#[test]
fn sniffer_memory_round_trips_through_animal_fields() {
    let before = Sniffer {
        cooldown: 9270,
        explored: vec![[2, 63, -4], [19, 64, 7]],
        phase: 5,
        elapsed: 73,
        target: Some([20, 63, 7]),
    };
    assert_eq!(*from_fields(&fields(&before)).unwrap(), before);
}

#[test]
fn malformed_sniffer_memory_is_rejected() {
    use mc_nbt::Tag;
    assert!(validate_fields(&[("solaris:SnifferPhase".into(), Tag::Byte(5))]).is_err());
    assert!(validate_fields(&[("solaris:SnifferExplored".into(), Tag::IntArray(vec![1, 2]))]).is_err());
    assert!(validate_fields(&[
        ("solaris:SnifferCooldown".into(), Tag::Int(2)),
        ("solaris:SnifferCooldown".into(), Tag::Int(3)),
    ]).is_err());
    let complete = fields(&Sniffer::default());
    for index in 0..complete.len() {
        let mut partial = complete.clone();
        partial.remove(index);
        assert!(validate_fields(&partial).is_err(), "missing required field {index}");
    }
    assert!(validate_fields(&[("solaris:SnifferTarget".into(), Tag::IntArray(vec![0, 64, 0]))]).is_err());
}

#[test]
fn sniffer_active_dig_survives_real_wal_and_region_checkpoint() {
    use crate::journal::{JournalWriter, Replay, read_complete_batches_after, replay_batches};
    let (mut state, actor, pos) = fixture();
    let actor_slot = slot(&state, actor);
    state.entities.set_pos(actor_slot, [pos[0] + 7.0, pos[1], pos[2] + 7.0]);
    let dirt = state.registry.blocks.block(&mc_data::Identifier::parse("minecraft:dirt").unwrap()).unwrap().default.0;
    state.world.set_block(pos[0] as i32, pos[1] as i32 - 1, pos[2] as i32, dirt);
    state.world.fold_overlay();
    let sniffer = spawn(&mut state, "sniffer", pos);
    assert!(diggable(&state, [pos[0] as i32, pos[1] as i32 - 1, pos[2] as i32], &[]),
        "the fixture's dirt must belong to the sniffer diggable tag");
    let sniffer_slot = slot(&state, sniffer);
    state.tick = (200 - (sniffer.0 as u64 % 200)) % 200;
    let mut output = None;
    for _ in 0..300 {
        let (tick, _, _) = run_tick(&mut state, &[]);
        if state.entities.row(sniffer_slot).entity_life.sniffer.as_ref().is_some_and(|s| s.phase == 5) {
            output = Some(tick);
            break;
        }
    }
    let output = output.expect("active dig");
    let expected = state.entities.row(sniffer_slot).entity_life.sniffer.clone();
    let entries = crate::core::journal_entries_for_tick(&mut state, &output);
    let dir = tempfile::tempdir().unwrap();
    let mut writer = JournalWriter::open(dir.path(), 1).unwrap();
    writer.append(1, output.tick, &entries).unwrap();
    writer.flush().unwrap();
    drop(writer);
    let replay = replay_batches(Replay::default(), &read_complete_batches_after(&dir.path().join("journal"), 0).unwrap());
    let mut restored = fixture().0;
    crate::core::restore_entity(&mut restored, sniffer.0, &replay.entities[&sniffer.0]).unwrap();
    assert_eq!(restored.entities.row(slot(&restored, sniffer)).entity_life.sniffer, expected);
    let (_, regions, _) = crate::core::checkpoint_data_for(&state, state.tick);
    mc_world::anvil::entity_regions::write_entity_regions(dir.path(), &state.registry.items, &regions, state.tick as u32).unwrap();
    let regions = mc_world::anvil::entity_regions::read_entity_regions(dir.path(), &state.registry.items).unwrap();
    let uuid = state.entities.row(sniffer_slot).uuid;
    let record = regions.values().flat_map(|chunk| &chunk.entities).find(|entity| entity.uuid == uuid).unwrap();
    let image = crate::journal::EntityStateImage::from_record(&state.registry, record).unwrap();
    assert_eq!(image.entity_life.sniffer, expected);
}
