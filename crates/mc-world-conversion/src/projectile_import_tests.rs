use mc_core::projectiles::{ProjectileKind, ProjectileState};
use mc_data::ItemStack;
use mc_data::items::ItemRegistry;
use mc_nbt::Tag;

const RETAINED: &str = "solaris:ProjectileRuntime";

fn items() -> ItemRegistry {
    mc_data::items::solaris_required_items()
}

fn stack(items: &ItemRegistry, name: &str) -> ItemStack {
    ItemStack::new(
        items
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap(),
        1,
    )
}

fn type_id(kind: ProjectileKind) -> u32 {
    mc_data::entity_types::solaris_required_entity_types()
        .id_of(&mc_data::Identifier::parse(kind.name()).unwrap())
        .unwrap()
}

// Explicit valid native envelopes which the refusal cases corrupt below.
// Production projectile field projection remains owned and tested by mc-core.
fn image(state: &ProjectileState, items: &ItemRegistry) -> mc_data::entity_nbt::VanillaEntity {
    assert!(matches!(
        state.kind,
        ProjectileKind::Arrow | ProjectileKind::Snowball
    ));
    let mut fields = vec![
        (
            RETAINED.into(),
            Tag::Compound(vec![
                ("version".into(), Tag::Int(1)),
                (
                    "state".into(),
                    Tag::String(serde_json::to_string(state).unwrap()),
                ),
            ]),
        ),
        ("HasBeenShot".into(), Tag::Byte(1)),
    ];
    if state.kind == ProjectileKind::Arrow {
        fields.extend([
            ("life".into(), Tag::Short(state.age as i16)),
            ("inGround".into(), Tag::Byte(i8::from(state.stuck))),
            ("pickup".into(), Tag::Byte(i8::from(state.pickup))),
            ("damage".into(), Tag::Double(f64::from(state.damage))),
            ("crit".into(), Tag::Byte(i8::from(state.critical))),
            ("PierceLevel".into(), Tag::Byte(state.piercing as i8)),
            (
                "item".into(),
                Tag::Compound(mc_data::item_nbt::stack_to_fields(items, &state.stack).unwrap()),
            ),
        ]);
    }
    mc_data::entity_nbt::VanillaEntity {
        id: state.kind.name().into(),
        uuid: uuid::Uuid::from_u128(123),
        wire_id: 7,
        pos: [-3.0, 70.0, 6.0],
        motion: [0.25, 0.1, -1.5],
        age: state.age as i32,
        item: (state.kind == ProjectileKind::Snowball).then(|| state.stack.clone()),
        extra_fields: fields,
        ..Default::default()
    }
}

fn projectile_anvil_source(root: &std::path::Path) -> std::path::PathBuf {
    let source = root.join("source");
    std::fs::create_dir(&source).unwrap();
    let mut manifest = mc_core::journal::CheckpointManifest {
        domain_versions: mc_core::native_checkpoint::domain_versions(),
        world_spawn: Some([0, 80, 0]),
        ..Default::default()
    };
    for dimension in [
        mc_core::dimensions::Dimension::Nether,
        mc_core::dimensions::Dimension::End,
    ] {
        manifest.dimensions.insert(
            dimension,
            Box::new(mc_core::journal::CheckpointManifest {
                domain_versions: mc_core::native_checkpoint::domain_versions(),
                world_spawn: Some([0, 80, 0]),
                ..Default::default()
            }),
        );
    }
    let mut metadata = mc_core::level_metadata::MetadataTags::new();
    mc_core::level_metadata::checkpoint_tags(&mut metadata, 42, &manifest).unwrap();
    for (path, tag) in metadata {
        crate::write_nbt(&source.join(path), &tag).unwrap();
    }
    source
}

fn projectile_import_refusal(root: &std::path::Path, source: &std::path::Path) -> String {
    let destination = root.join("converted");
    let receipt_path = root.join("receipt.json");
    let error = crate::import_anvil(
        &crate::test_support::config(source.to_owned(), 42),
        source,
        &destination,
        &receipt_path,
    )
    .unwrap_err()
    .to_string();
    let receipt: crate::ConversionReceipt =
        serde_json::from_slice(&std::fs::read(receipt_path).unwrap()).unwrap();
    assert_eq!(receipt.status, "FAIL");
    assert_eq!(receipt.error.as_deref(), Some(error.as_str()));
    assert!(
        !destination.exists(),
        "refused import cannot publish a world"
    );
    error
}

fn assert_native_projectile_refusal(
    root: &std::path::Path,
    uuid: uuid::Uuid,
    payload: Tag,
    kind: ProjectileKind,
    expected_uuid: &str,
    reason: &str,
) {
    use crate::test_support::{fixture_store, read_manifest, write_manifest};
    use mc_world::native::{NativeCheckpoint, NativeEntitySnapshot};

    let config = crate::test_support::config(root.join("runtime"), 42);
    let registry = crate::test_support::registry().unwrap();
    let store = fixture_store(&config).unwrap();
    write_manifest(&config, &store, &read_manifest(&store).unwrap()).unwrap();
    let image = store.read_checkpoint().unwrap();
    let entities = [NativeEntitySnapshot {
        identity: *uuid.as_bytes(),
        dimension: 0,
        position: mc_world::ChunkPos { x: -33, z: 65 },
        generated_slot: None,
        journal_lsn: image.tick,
        payload: Some(payload),
    }];
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick: image.tick,
                previous_tick: image.tick,
                chunks: &[],
                entities: &entities,
                players: &[],
                globals: &image.globals,
            },
            &registry.blocks,
            &registry.items,
        )
        .unwrap();
    let before = store.read_checkpoint().unwrap();
    drop(store);

    let error = mc_core::Core::open(config.clone()).unwrap_err().to_string();
    for detail in [
        config
            .world_dir
            .join("native/state.redb")
            .display()
            .to_string(),
        "minecraft:overworld".into(),
        "chunk (-33, 65)".into(),
        "entity wire-id 7".into(),
        expected_uuid.into(),
        format!("type-id {} ({})", type_id(kind), kind.name()),
        reason.into(),
    ] {
        assert!(error.contains(&detail), "missing {detail:?} in {error}");
    }
    let after = fixture_store(&config).unwrap().read_checkpoint().unwrap();
    assert_eq!(after.tick, before.tick, "refusal cannot advance checkpoint");
    assert_eq!(after.globals.len(), before.globals.len());
    for (key, expected) in &before.globals {
        let actual = &after.globals[key];
        assert_eq!(actual.version, expected.version, "{key}");
        assert_eq!(actual.captured_tick, expected.captured_tick, "{key}");
        assert_eq!(actual.payload, expected.payload, "{key}");
    }
    assert!(before.players.is_empty() && after.players.is_empty());
    assert_eq!(before.entities.len(), 1);
    assert_eq!(after.entities.len(), 1);
    let expected = &before.entities[0];
    let actual = &after.entities[0];
    assert_eq!(actual.identity, expected.identity);
    assert_eq!(actual.dimension, expected.dimension);
    assert_eq!(actual.position, expected.position);
    assert_eq!(actual.generated_slot, expected.generated_slot);
    assert_eq!(actual.journal_lsn, expected.journal_lsn);
    assert_eq!(
        actual.payload, expected.payload,
        "refusal cannot rewrite custody"
    );
}

#[test]
fn malformed_projectile_native_open_and_offline_import_preserve_custody_and_location() {
    let items = items();
    let uuid = uuid::Uuid::from_u128(0xa770);
    let type_id = type_id(ProjectileKind::Arrow);
    for invalid_ledger in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let source = projectile_anvil_source(dir.path());
        let mut projectile = ProjectileState::new(
            ProjectileKind::Arrow,
            None,
            stack(&items, "minecraft:arrow"),
        );
        let mut malformed = image(&projectile, &items);
        malformed.uuid = uuid;
        malformed.pos = [-520.5, 75.0, 1040.5];
        let expected = if invalid_ledger {
            projectile.hit = vec![uuid, uuid];
            let Some(Tag::Compound(runtime)) = malformed
                .extra_fields
                .iter_mut()
                .find(|(key, _)| key == RETAINED)
                .map(|(_, tag)| tag)
            else {
                panic!("runtime");
            };
            runtime
                .iter_mut()
                .find(|(key, _)| key == "state")
                .unwrap()
                .1 = Tag::String(serde_json::to_string(&projectile).unwrap());
            "solaris:ProjectileRuntime.state.hit: duplicate UUID"
        } else {
            let Some(Tag::Compound(item)) = malformed
                .extra_fields
                .iter_mut()
                .find(|(key, _)| key == "item")
                .map(|(_, tag)| tag)
            else {
                panic!("arrow item");
            };
            item.iter_mut().find(|(key, _)| key == "count").unwrap().1 = Tag::Int(2);
            "item.count: expected 1, got 2"
        };
        assert_native_projectile_refusal(
            dir.path(),
            uuid,
            Tag::Compound(mc_data::entity_nbt::entity_to_fields(&items, &malformed).unwrap()),
            ProjectileKind::Arrow,
            &format!("UUID {uuid}"),
            expected,
        );
        let chunks = std::collections::BTreeMap::from([(
            (-33, 65),
            mc_world::anvil::EntityChunk {
                x: -33,
                z: 65,
                entities: vec![
                    mc_data::entity_nbt::VanillaEntity {
                        id: "minecraft:cow".into(),
                        uuid: uuid::Uuid::from_u128(0xa771),
                        wire_id: 6,
                        ..Default::default()
                    },
                    malformed,
                ],
            },
        )]);
        let regions = mc_core::dimensions::Dimension::Overworld
            .directory(&source)
            .join("entities");
        mc_world::anvil::write_entity_regions(&regions, &items, &chunks, 1).unwrap();
        let path = regions.join("r.-2.2.mca");
        let before = std::fs::read(&path).unwrap();
        let error = projectile_import_refusal(dir.path(), &source);
        for detail in [
            path.display().to_string(),
            "chunk (-33, 65)".into(),
            "entity-index 1".into(),
            format!("UUID {uuid}"),
            format!("type-id {type_id} (minecraft:arrow)"),
            expected.into(),
        ] {
            assert!(error.contains(&detail), "missing {detail:?} in {error}");
        }
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "failed import cannot rewrite custody"
        );
    }
}

#[test]
fn malformed_shared_projectile_native_open_and_offline_import_preserve_raw_identity_and_custody() {
    let items = items();
    let uuid = uuid::Uuid::from_u128(0x5a0b);
    let type_id = type_id(ProjectileKind::Snowball);
    for (bad_field, reason) in [
        ("Item", "Item.id:"),
        ("Motion", "Motion: expected three-double list"),
        ("UUID", "UUID: expected four-int UUID"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let source = projectile_anvil_source(dir.path());
        let projectile = ProjectileState::new(
            ProjectileKind::Snowball,
            None,
            stack(&items, "minecraft:snowball"),
        );
        let mut entity = image(&projectile, &items);
        entity.uuid = uuid;
        entity.pos = [-520.5, 75.0, 1040.5];
        let mut fields = mc_data::entity_nbt::entity_to_fields(&items, &entity).unwrap();
        let value = &mut fields
            .iter_mut()
            .find(|(key, _)| key == bad_field)
            .unwrap()
            .1;
        *value = match bad_field {
            "Item" => Tag::Compound(vec![
                ("id".into(), Tag::String("minecraft:no_such_item".into())),
                ("count".into(), Tag::Int(1)),
            ]),
            "Motion" => Tag::String("bad".into()),
            "UUID" => Tag::IntArray(vec![1, 2, 3]),
            _ => unreachable!(),
        };
        let expected_uuid = if bad_field == "UUID" {
            "UUID IntArray([1, 2, 3])".into()
        } else {
            format!("UUID {uuid}")
        };
        assert_native_projectile_refusal(
            dir.path(),
            uuid,
            Tag::Compound(fields.clone()),
            ProjectileKind::Snowball,
            &expected_uuid,
            reason,
        );
        let valid = mc_data::entity_nbt::entity_to_fields(
            &items,
            &mc_data::entity_nbt::VanillaEntity {
                id: "minecraft:cow".into(),
                uuid: uuid::Uuid::from_u128(0x5a0c),
                wire_id: 6,
                ..Default::default()
            },
        )
        .unwrap();
        let tag = Tag::Compound(vec![
            (
                "DataVersion".into(),
                Tag::Int(mc_world::anvil::entity_regions::ENTITY_DATA_VERSION),
            ),
            ("Position".into(), Tag::IntArray(vec![-33, 65])),
            (
                "Entities".into(),
                Tag::List(mc_nbt::ListTag {
                    element_type: mc_nbt::tag_type::COMPOUND,
                    elements: vec![Tag::Compound(valid), Tag::Compound(fields)],
                }),
            ),
        ]);
        let mut bytes = Vec::new();
        mc_nbt::write_named(&mut bytes, "", &tag).unwrap();
        let regions = mc_core::dimensions::Dimension::Overworld
            .directory(&source)
            .join("entities");
        std::fs::create_dir_all(&regions).unwrap();
        let path = regions.join("r.-2.2.mca");
        mc_world::anvil::region::write_region(
            &path,
            &[mc_world::anvil::region::ChunkPayload {
                local_x: 31,
                local_z: 1,
                timestamp: 1,
                uncompressed_nbt: bytes,
            }],
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        let error = projectile_import_refusal(dir.path(), &source);
        for detail in [
            path.strip_prefix(&source).unwrap().display().to_string(),
            "chunk (-33, 65)".into(),
            "entity-index 1".into(),
            expected_uuid,
            format!("type-id {type_id} (minecraft:snowball)"),
            reason.into(),
        ] {
            assert!(
                error.contains(&detail),
                "{bad_field}: missing {detail:?} in {error}"
            );
        }
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "shared import decode refusal cannot rewrite custody"
        );
    }
}
