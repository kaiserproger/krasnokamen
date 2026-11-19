//! Real native transaction plus published WAL, without shutdown checkpointing the tail.
use super::*;

#[test]
fn native_frozen_item_clock_survives_checkpoint_tail_recovery_and_frozen_open() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = fixture(directory.path());
    drill.state.tick_control.set_frozen(true);
    let item = drill
        .state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap();
    let id = crate::tick::spawn_item_entity(
        &mut drill.state,
        [34.5, 200.0, 40.5],
        mc_data::ItemStack::new(item, 1),
    );
    let slot = drill.state.entities.slot_of(id).unwrap();
    drill.state.entities.row_mut(slot).age = 5999;
    let uuid = drill.state.entities.row(slot).uuid.to_string();
    drill.run(&[]).unwrap();
    drill.checkpoint();
    let checkpoint_tick = drill.manifest.tick;
    let game_time = drill.state.tick_control.game_ticks(drill.state.tick);
    for _ in 0..17 {
        drill.run(&[]).unwrap();
    }
    drill.writer.as_mut().unwrap().flush().unwrap();
    assert!(drill.state.tick > checkpoint_tick);
    assert_eq!(
        drill.state.tick_control.game_ticks(drill.state.tick),
        game_time
    );
    let config = drill.config.clone();
    let mut restored = drill.crash();
    let slot = restored.state.entities.slot_of(id).unwrap();
    assert_eq!(restored.state.entities.row(slot).age, 5999);
    assert_eq!(
        restored
            .state
            .entities
            .row(slot)
            .item
            .as_ref()
            .unwrap()
            .full_stack()
            .count,
        1
    );
    restored.state.tick_control.set_frozen(true);
    restored.run(&[]).unwrap();
    assert_eq!(restored.state.entities.row(slot).age, 5999);
    restored.checkpoint();
    drop(restored);

    let mut frozen_config = config.clone();
    frozen_config.start_frozen = true;
    let core = Core::open(frozen_config).unwrap();
    let mut ticks = core.subscribe();
    for _ in 0..3 {
        assert_eq!(ticks.blocking_recv().unwrap().snapshot.game_time, game_time);
    }
    core.shutdown();
    drop(core);
    let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
    let item = recovered.dimensions[&crate::dimensions::Dimension::Overworld]
        .entities
        .values()
        .find(|record| record.uuid == uuid)
        .unwrap();
    assert_eq!(item.age, 5999);
    assert_eq!(item.item.as_ref().unwrap().count, 1);
    drop(recovered);

    let mut resumed = recovered_world(&config);
    resumed
        .world
        .ensure_blocking(&[(CHUNK, 0)].into_iter().collect());
    resumed
        .tickets
        .insert("frozen-item-restart".into(), (CHUNK, 0));
    resumed.tick_control.set_frozen(false);
    let (output, _, _) = crate::tick::run_tick(&mut resumed, &[]);
    assert!(resumed.entities.slot_of(id).is_none());
    assert!(output.changes.entities_removed.contains(&id));
}

#[test]
fn native_game_clock_overworld_only_transition_controls_nether_and_end_items() {
    use crate::dimensions::Dimension;
    for dimension in [Dimension::Nether, Dimension::End] {
        let directory = tempfile::tempdir().unwrap();
        let mut drill = fixture(directory.path());
        let item = drill
            .state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
            .unwrap();
        let child = drill.state.other_dimensions.get_mut(&dimension).unwrap();
        child
            .world
            .ensure_blocking(&[(CHUNK, 0)].into_iter().collect());
        child.tickets.insert("native-game-clock".into(), (CHUNK, 0));
        child.entities.set_next_wire_id(1000);
        let id = crate::tick::spawn_item_entity(
            child,
            [34.5, 200.0, 40.5],
            mc_data::ItemStack::new(item, 1),
        );
        let slot = child.entities.slot_of(id).unwrap();
        child.entities.row_mut(slot).age = 5998;
        let uuid = child.entities.row(slot).uuid.to_string();
        drill.run(&[]).unwrap();
        let child = &drill.state.other_dimensions[&dimension];
        assert_eq!(
            child.entities.row(child.entities.slot_of(id).unwrap()).age,
            5999
        );
        drill.checkpoint();
        drill.state.tick_control.set_frozen(true);
        let mut transitions = 0;
        for _ in 0..17 {
            let mut batch = drill.capture_tick(&[]);
            for entry in &mut batch.entries {
                if let JournalEntry::Climate { value } = entry {
                    let value = *value;
                    *entry = JournalEntry::Dimension {
                        dimension: Dimension::Overworld,
                        entries: vec![JournalEntry::Climate { value }],
                    };
                    transitions += 1;
                }
            }
            drill.append(&batch).unwrap();
        }
        assert_eq!(transitions, 1);
        drill.writer.as_mut().unwrap().flush().unwrap();
        let config = drill.config.clone();
        drop(drill); // Published tail, no final checkpoint.
        let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
        let record = recovered.dimensions[&dimension]
            .entities
            .values()
            .find(|record| record.uuid == uuid)
            .unwrap();
        assert_eq!(record.age, 5999);
        assert_eq!(record.item.as_ref().unwrap().count, 1);
        drop(recovered);
        let mut resumed = recovered_world(&config);
        let child = resumed.other_dimensions.get_mut(&dimension).unwrap();
        child
            .world
            .ensure_blocking(&[(CHUNK, 0)].into_iter().collect());
        child.tickets.insert("native-game-clock".into(), (CHUNK, 0));
        resumed.tick_control.set_frozen(false);
        crate::tick::run_tick(&mut resumed, &[]);
        assert!(
            resumed.other_dimensions[&dimension]
                .entities
                .slot_of(id)
                .is_none()
        );
    }
}
