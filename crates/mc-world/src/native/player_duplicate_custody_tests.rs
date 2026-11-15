use super::*;

#[test]
fn repeated_player_custody_names_refuse_raw_checkpoint_recovery() {
    for key in ["krasnokamen:Carried", "krasnokamen:CraftingGrid"] {
        let directory = tempfile::tempdir().unwrap();
        let store = NativeStore::create(&directory.path().join("state.redb"), [1; 32]).unwrap();
        let uuid = uuid::Uuid::from_u128(7);
        let empty = Tag::Compound(vec![]);
        let malformed = Tag::Compound(vec![
            ("count".into(), Tag::Int(7)),
            ("payload".into(), Tag::String("retained".into())),
        ]);
        let values = if key == "krasnokamen:Carried" {
            [empty, malformed]
        } else {
            [empty, malformed].map(|item| {
                Tag::List(mc_nbt::ListTag {
                    element_type: mc_nbt::tag_type::COMPOUND,
                    elements: vec![item],
                })
            })
        };
        let tag = Tag::Compound(
            values
                .into_iter()
                .map(|value| (key.into(), value))
                .collect(),
        );
        let mut payload = Vec::new();
        mc_nbt::write_named(&mut payload, "", &tag).unwrap();
        let ordinary = NativePlayerSnapshot {
            uuid: *uuid.as_bytes(),
            record: NativeDomainRecord {
                version: 1,
                captured_tick: 1,
                payload: codec::encode(&Tag::Compound(vec![])).unwrap(),
            },
        };
        commit(&store, 1, 0, &globals(1), &[], &[ordinary]).unwrap();
        // Model an old/corrupt stored record without weakening the native writer.
        let record = NativeDomainRecord {
            version: 1,
            captured_tick: 1,
            payload,
        };
        let mut encoded = Vec::new();
        encoded.extend_from_slice(&record.version.to_be_bytes());
        encoded.extend_from_slice(&record.captured_tick.to_be_bytes());
        encoded.extend_from_slice(&(record.payload.len() as u32).to_be_bytes());
        encoded.extend_from_slice(&record.payload);
        let tx = store.database.begin_write().unwrap();
        {
            let definition: TableDefinition<&[u8], &[u8]> = TableDefinition::new("players");
            tx.open_table(definition)
                .unwrap()
                .insert(uuid.as_bytes().as_slice(), encoded.as_slice())
                .unwrap();
        }
        tx.commit().unwrap();
        let error = store
            .checkpoint_reader()
            .unwrap()
            .read_checkpoint_metadata()
            .err()
            .expect("duplicate checkpoint must refuse")
            .to_string();
        assert!(error.contains("NBT/duplicate_field"), "{key}: {error}");
        drop(store);
        let path = directory.path().join("state.redb");
        let before = std::fs::read(&path).unwrap();
        let read_only = NativeReadOnlyStore::open_with_cache(&path, [1; 32], 1024 * 1024).unwrap();
        let error = read_only
            .checkpoint_reader()
            .unwrap()
            .read_checkpoint_metadata()
            .err()
            .expect("duplicate checkpoint must refuse")
            .to_string();
        assert!(error.contains("NBT/duplicate_field"), "{key}: {error}");
        drop(read_only);
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
}
