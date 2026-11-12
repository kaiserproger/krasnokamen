#[test]
fn climate_variant_journal_images_restore_the_same_typed_value() {
    use crate::boundary::EntityDataValue;
    for value in [
        EntityDataValue::ChickenVariant(1),
        EntityDataValue::CowVariant(2),
        EntityDataValue::PigVariant(3),
        EntityDataValue::ZombieNautilusVariant(4),
    ] {
        let image = crate::journal::EntityDataValueImage::from(&value);
        let decoded = serde_json::from_slice(&serde_json::to_vec(&image).unwrap()).unwrap();
        assert_eq!(super::restore_data_value(&decoded), value);
    }
}
