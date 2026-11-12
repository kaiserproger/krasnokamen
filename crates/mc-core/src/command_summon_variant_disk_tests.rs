use super::*;
use crate::journal::{JournalWriter, Replay, read_complete_batches_after, replay_batches};

#[test]
fn variant_disk_checkpoint_and_wal_round_trip() {
    for (kind, variant, age, index) in [
        ("cow", "cold", 0, 18),
        ("cow", "cold", -24000, 18),
        ("zombie_nautilus", "warm", 0, 21),
    ] {
        let (mut state, id) = summon(
            kind,
            &format!("{{variant:'minecraft:{variant}',Age:{age}}}"),
        );
        let registry = format!("{kind}_variant");
        let variant_id = mc_data::required_registry_entry_id(
            &registry,
            &mc_data::Identifier::parse(format!("minecraft:{variant}")).unwrap(),
        )
        .unwrap();
        let expected = if kind == "cow" {
            EntityDataValue::CowVariant(variant_id)
        } else {
            EntityDataValue::ZombieNautilusVariant(variant_id)
        };
        let root = std::env::temp_dir().join(format!("krasnokamen-variant-{}", uuid::Uuid::new_v4()));
        let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
        let native = crate::native_checkpoint::encode_entities(
            crate::dimensions::Dimension::Overworld,
            &chunks,
            &state.registry.items,
            state.tick,
        )
        .unwrap();
        let config = crate::CoreConfig::new(root.join("checkpoint"), 42);
        crate::native_checkpoint::tests::write_manifest(&config, &Default::default()).unwrap();
        let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
        let base = store.read_checkpoint().unwrap();
        store
            .commit_checkpoint(
                mc_world::native::NativeCheckpoint {
                    tick: state.tick,
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
        let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
        let replay = &recovered.dimensions[&crate::dimensions::Dimension::Overworld];
        let (mut restored, _, _) = crate::hostile_tests::setup(8.0);
        crate::core::restore_entity(&mut restored, id.0, &replay.entities[&id.0]).unwrap();
        let row = restored
            .entities
            .row(restored.entities.slot_of(id).unwrap());
        assert!(
            row.appearance.extra.contains(&(index, expected.clone())),
            "checkpoint {kind} {age}"
        );
        assert_eq!(row.entity_life.child_ticks, age.min(0_i32).unsigned_abs());
        assert_eq!(row.appearance.baby, age < 0);

        let (mut output, _, _) = crate::tick::run_tick(&mut state, &[]);
        std::sync::Arc::get_mut(&mut output.changes)
            .unwrap()
            .entities_spawned
            .push(id);
        let birth = crate::core::journal_entries_for_tick(&mut state, &output);
        let mut writer = JournalWriter::open(&root, 1).unwrap();
        writer.append(1, output.tick, &birth).unwrap();
        writer.flush().unwrap();
        drop(writer);
        let batches = read_complete_batches_after(&root.join("journal"), 0).unwrap();
        let replay = replay_batches(Replay::default(), &batches);
        let (mut restored, _, _) = crate::hostile_tests::setup(8.0);
        crate::core::restore_entity(&mut restored, id.0, &replay.entities[&id.0]).unwrap();
        let row = restored
            .entities
            .row(restored.entities.slot_of(id).unwrap());
        assert!(
            row.appearance.extra.contains(&(index, expected)),
            "wal {kind} {age}"
        );
        assert_eq!(row.appearance.baby, age < 0);
        if age < 0 {
            assert!(row.entity_life.child_ticks > 0);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
