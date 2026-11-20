use super::*;

fn checkpoint(tags: Tag) -> (tempfile::TempDir, crate::CoreConfig, TickState, EntityId) {
    let (mut state, id) = summon("cow", "{NoAI:1b,PersistenceRequired:1b}");
    state.tick = 1;
    let (_, mut chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
    let native = chunks
        .values_mut()
        .flat_map(|chunk| &mut chunk.entities)
        .find(|entity| entity.wire_id == id.0)
        .unwrap();
    let uuid = native.uuid;
    let Tag::Compound(fields) = tags else {
        unreachable!()
    };
    let mut entities = crate::native_checkpoint::encode_entities(
        state.dimension,
        &chunks,
        &state.registry.items,
        state.tick,
    )
    .unwrap();
    let Some(Tag::Compound(payload)) = entities
        .iter_mut()
        .find(|entity| entity.identity == *uuid.as_bytes())
        .unwrap()
        .payload
        .as_mut()
    else {
        unreachable!()
    };
    payload.extend(fields);
    let directory = tempfile::tempdir().unwrap();
    let config = crate::CoreConfig::new(directory.path().to_owned(), 42);
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let mut base = store.read_checkpoint().unwrap();
    let mut manifest: crate::journal::CheckpointManifest =
        serde_json::from_slice(&base.globals["checkpoint"].payload).unwrap();
    manifest.tick = 1;
    for child in manifest.dimensions.values_mut() {
        child.tick = 1;
    }
    base.globals.insert(
        "checkpoint".into(),
        crate::native_checkpoint::record(1, serde_json::to_vec(&manifest).unwrap()),
    );
    crate::native_checkpoint::refresh_metadata(&mut base.globals, &manifest).unwrap();
    for global in base.globals.values_mut() {
        global.captured_tick = 1;
    }
    store
        .commit_checkpoint(
            mc_world::native::NativeCheckpoint {
                tick: 1,
                previous_tick: base.tick,
                chunks: &[],
                entities: &entities,
                players: &[],
                globals: &base.globals,
            },
            &state.registry.blocks,
            &state.registry.items,
        )
        .unwrap();
    drop(store);
    (directory, config, state, id)
}

fn tail(config: &crate::CoreConfig, state: &TickState, id: EntityId, tags: &Tag) {
    let slot = state.entities.slot_of(id).unwrap();
    let mut image = EntityStateImage::from_row(state.entities.body(slot), state.entities.row(slot));
    image.opaque_entity_fields.clear();
    mc_nbt::write_named(&mut image.opaque_entity_fields, "", tags).unwrap();
    tail_image(config, id, image);
}

fn tail_image(config: &crate::CoreConfig, id: EntityId, image: EntityStateImage) {
    let mut writer = crate::journal::JournalWriter::open_test(&config.world_dir, 1).unwrap();
    writer
        .append(
            2,
            2,
            &[crate::journal::JournalEntry::EntityState {
                id: id.0,
                state: Arc::new(image),
                tick: 2,
            }],
        )
        .unwrap();
    writer.flush().unwrap();
}

fn recovered_fields(config: &crate::CoreConfig, id: EntityId) -> Result<Vec<u8>, crate::CoreError> {
    let recovered = crate::native_checkpoint::recover_offline(config)?;
    Ok(
        recovered.dimensions[&crate::dimensions::Dimension::Overworld].entities[&id.0]
            .state
            .opaque_entity_fields
            .clone(),
    )
}

#[test]
fn duplicate_tags_checkpoint_and_wal_recovery_is_idempotent() {
    let (_dir, config, state, id) = checkpoint(
        crate::command_snbt::parse("{Tags:['base','base'],Other:'checkpoint'}").unwrap(),
    );
    let mut expected = Vec::new();
    mc_nbt::write_named(
        &mut expected,
        "",
        &crate::command_snbt::parse("{Tags:['base'],Other:'checkpoint'}").unwrap(),
    )
    .unwrap();
    for _ in 0..2 {
        assert_eq!(recovered_fields(&config, id).unwrap(), expected);
    }
    tail(
        &config,
        &state,
        id,
        &crate::command_snbt::parse("{Tags:['tail','tail'],Other:'wal'}").unwrap(),
    );
    expected.clear();
    mc_nbt::write_named(
        &mut expected,
        "",
        &crate::command_snbt::parse("{Tags:['tail'],Other:'wal'}").unwrap(),
    )
    .unwrap();
    for _ in 0..2 {
        assert_eq!(recovered_fields(&config, id).unwrap(), expected);
    }
}

fn repeated_keys() -> Tag {
    // Binary NBT admits repeated names; the SNBT command parser already
    // refuses them, so build the corrupt persisted image directly.
    Tag::Compound(vec![
        ("Tags".into(), crate::command_snbt::parse("['a']").unwrap()),
        ("Tags".into(), crate::command_snbt::parse("['b']").unwrap()),
    ])
}

#[test]
fn duplicate_tags_keys_in_checkpoint_refuse_recovery() {
    use redb::ReadableTable;
    let (_dir, config, state, id) = checkpoint(crate::command_snbt::parse("{Tags:['a']}").unwrap());
    let uuid = state.entities.row(state.entities.slot_of(id).unwrap()).uuid;
    // The normal checkpoint writer already rejects duplicate compound names.
    // Inject corrupt bytes below that fence to exercise actual recovery.
    let database = redb::Database::open(config.world_dir.join("native/state.redb")).unwrap();
    let transaction = database.begin_write().unwrap();
    {
        let mut table = transaction
            .open_table(redb::TableDefinition::<&[u8], &[u8]>::new(
                "persistent_entities",
            ))
            .unwrap();
        let mut bytes = table
            .get(uuid.as_bytes().as_slice())
            .unwrap()
            .unwrap()
            .value()
            .to_vec();
        // Native state.rs: dimension/x/z/lsn + absent generated-slot + present payload.
        const HEADER: usize = 4 + 4 + 4 + 8 + 1 + 1;
        assert_eq!(&bytes[20..22], &[0, 1]);
        let (_, Tag::Compound(mut fields)) = mc_nbt::read_named(&mut &bytes[HEADER..]).unwrap()
        else {
            unreachable!()
        };
        fields.push(("Tags".into(), crate::command_snbt::parse("['b']").unwrap()));
        bytes.truncate(HEADER);
        mc_nbt::write_named(&mut bytes, "", &Tag::Compound(fields)).unwrap();
        table
            .insert(uuid.as_bytes().as_slice(), bytes.as_slice())
            .unwrap();
    }
    transaction.commit().unwrap();
    drop(database);
    for _ in 0..2 {
        let error = recovered_fields(&config, id).unwrap_err().to_string();
        assert!(
            error.contains("native checkpoint/NBT/duplicate_field"),
            "{error}"
        );
    }
}

#[test]
fn duplicate_tags_keys_in_wal_refuse_recovery() {
    let (_dir, config, state, id) =
        checkpoint(crate::command_snbt::parse("{Tags:['checkpoint']}").unwrap());
    tail(&config, &state, id, &repeated_keys());
    for _ in 0..2 {
        let error = recovered_fields(&config, id).unwrap_err().to_string();
        assert!(error.contains("duplicate Tags field"), "{error}");
    }
}

#[test]
fn opaque_wal_trailing_bytes_refuse_offline_and_live_recovery() {
    let (_dir, config, state, id) =
        checkpoint(crate::command_snbt::parse("{Tags:['checkpoint']}").unwrap());
    let slot = state.entities.slot_of(id).unwrap();
    let mut image = EntityStateImage::from_row(state.entities.body(slot), state.entities.row(slot));
    image.opaque_entity_fields.clear();
    mc_nbt::write_named(
        &mut image.opaque_entity_fields,
        "",
        &crate::command_snbt::parse("{Tags:['tail']}").unwrap(),
    )
    .unwrap();
    image.opaque_entity_fields.extend_from_slice(&[0, 42]);
    tail_image(&config, id, image);
    for _ in 0..2 {
        let error = recovered_fields(&config, id).unwrap_err().to_string();
        assert!(error.contains("trailing NBT bytes"), "{error}");
        let error =
            crate::core::recover_dimension_state(&config, crate::dimensions::Dimension::Overworld)
                .err()
                .expect("live recovery must refuse trailing opaque bytes")
                .to_string();
        assert!(error.contains("trailing NBT bytes"), "{error}");
    }
}

fn player_tail(tags: &Tag) -> (tempfile::TempDir, crate::CoreConfig, uuid::Uuid) {
    let (dir, config, mut state, _) = checkpoint(Tag::Compound(Vec::new()));
    let player = state.players.values_mut().next().unwrap();
    player.opaque_player_fields.clear();
    mc_nbt::write_named(&mut player.opaque_player_fields, "", tags).unwrap();
    let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
    let entry = crate::core::journal_entries_for_tick(&mut state, &output)
        .unwrap()
        .into_iter()
        .find(|entry| matches!(entry, crate::journal::JournalEntry::Player { .. }))
        .expect("fixture player must have a WAL post-image");
    let crate::journal::JournalEntry::Player { uuid, .. } = &entry else {
        unreachable!()
    };
    let uuid = uuid.parse().unwrap();
    let mut writer = crate::journal::JournalWriter::open_test(&config.world_dir, 1).unwrap();
    writer.append(2, 2, &[entry]).unwrap();
    writer.flush().unwrap();
    (dir, config, uuid)
}

#[test]
fn duplicate_player_tags_in_wal_normalize_before_offline_export_and_live_recovery() {
    let (_dir, config, uuid) =
        player_tail(&crate::command_snbt::parse("{Tags:['tail','tail'],Other:'player'}").unwrap());
    let expected = crate::command_snbt::parse("{Tags:['tail'],Other:'player'}").unwrap();
    for _ in 0..2 {
        let world = crate::native_checkpoint::recover_offline(&config).unwrap();
        for replay in world.dimensions.values() {
            let player = &replay.players[&uuid.to_string()];
            let (_, fields) =
                mc_nbt::read_named(&mut player.opaque_player_fields.as_slice()).unwrap();
            assert_eq!(fields, expected);
        }
        let checkpoint = crate::native_checkpoint::encode_checkpoint(&world).unwrap();
        let player = checkpoint
            .players
            .iter()
            .find(|player| player.uuid == *uuid.as_bytes())
            .unwrap();
        let Tag::Compound(fields) =
            crate::native_checkpoint::decode_tag(&player.record, "player").unwrap()
        else {
            unreachable!()
        };
        assert_eq!(
            mc_data::item_nbt::field(&fields, "Tags"),
            Some(&crate::command_snbt::parse("['tail']").unwrap()),
        );
        drop(world);
        let live =
            crate::core::recover_dimension_state(&config, crate::dimensions::Dimension::Overworld)
                .unwrap();
        let (_, fields) =
            mc_nbt::read_named(&mut live.offline_players[&uuid].opaque_player_fields.as_slice())
                .unwrap();
        assert_eq!(fields, expected);
    }
}

#[test]
fn duplicate_player_tags_keys_in_wal_refuse_offline_export_and_live_recovery() {
    let (_dir, config, _) = player_tail(&repeated_keys());
    for _ in 0..2 {
        let error = crate::native_checkpoint::recover_offline(&config)
            .and_then(|world| crate::native_checkpoint::encode_checkpoint(&world))
            .err()
            .expect("offline export must refuse repeated player Tags keys")
            .to_string();
        assert!(error.contains("duplicate Tags field"), "{error}");
        let error =
            crate::core::recover_dimension_state(&config, crate::dimensions::Dimension::Overworld)
                .err()
                .expect("live recovery must refuse repeated player Tags keys")
                .to_string();
        assert!(error.contains("duplicate Tags field"), "{error}");
    }
}
