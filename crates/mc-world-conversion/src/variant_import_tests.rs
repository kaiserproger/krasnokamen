//! Anvil import keeps vanilla tolerance for the climate/nautilus `variant`
//! field: an ignored value is dropped and the species default kept, while a
//! valid one survives into the native checkpoint.
use mc_nbt::Tag;

#[test]
fn anvil_import_ignores_a_malformed_variant_and_keeps_a_valid_one() {
    let dir = tempfile::tempdir().unwrap();
    let source = crate::projectile_import_tests::projectile_anvil_source(dir.path());
    let items = crate::test_support::registry().unwrap().items;
    let chicken = uuid::Uuid::from_u128(0xc41c);
    let pig = uuid::Uuid::from_u128(0x9199);
    let animal = |id: &str, uuid, wire_id, variant: Tag| mc_data::entity_nbt::VanillaEntity {
        id: id.into(),
        uuid,
        wire_id,
        extra_fields: vec![("variant".into(), variant)],
        ..Default::default()
    };
    let chunks = std::collections::BTreeMap::from([(
        (0, 0),
        mc_world::anvil::EntityChunk {
            x: 0,
            z: 0,
            entities: vec![
                animal(
                    "minecraft:chicken",
                    chicken,
                    6,
                    Tag::String("minecraft:not_a_variant".into()),
                ),
                animal(
                    "minecraft:pig",
                    pig,
                    7,
                    Tag::String("minecraft:cold".into()),
                ),
            ],
        },
    )]);
    let regions = mc_core::dimensions::Dimension::Overworld
        .directory(&source)
        .join("entities");
    mc_world::anvil::write_entity_regions(&regions, &items, &chunks, 1).unwrap();

    let destination = dir.path().join("converted");
    crate::import_anvil(
        &crate::test_support::config(source.clone(), 42),
        &source,
        &destination,
        &dir.path().join("receipt.json"),
    )
    .unwrap();
    let recovered =
        mc_core::native_checkpoint::recover_offline(&crate::test_support::config(destination, 42))
            .unwrap();
    let entities = &recovered.dimensions[&mc_core::dimensions::Dimension::Overworld].entities;
    let by_uuid = |uuid: uuid::Uuid| {
        entities
            .values()
            .find(|entity| entity.uuid == uuid.to_string())
            .unwrap()
    };
    assert!(by_uuid(chicken).appearance.extra.is_empty());
    assert_eq!(
        by_uuid(pig).appearance.extra,
        vec![(19, mc_core::journal::EntityDataValueImage::PigVariant(0))]
    );
}
