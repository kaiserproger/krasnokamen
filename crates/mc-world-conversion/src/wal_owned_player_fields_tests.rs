//! Real WAL post-images must not carry a second, disposable item authority.
use super::*;
use mc_core::dimensions::Dimension;
use mc_core::journal::{JournalEntry, JournalWriter};
use mc_core::native_checkpoint as bridge;
use mc_nbt::{ListTag, tag_type};

const PLAYER: &str = "00000000-0000-0000-0000-000000000003";
const PLAYER_PATH: &str = "players/data/00000000-0000-0000-0000-000000000003.dat";

fn list(elements: Vec<Tag>) -> Tag {
    Tag::List(ListTag {
        element_type: tag_type::COMPOUND,
        elements,
    })
}

fn stack(slot: Option<i8>, count: i32) -> Tag {
    let mut fields = vec![
        ("id".into(), Tag::String("minecraft:diamond".into())),
        ("count".into(), Tag::Int(count)),
    ];
    if let Some(slot) = slot {
        fields.push(("Slot".into(), Tag::Byte(slot)));
    }
    Tag::Compound(fields)
}

fn entry(opaque: Vec<u8>, inventory: &[mc_data::ItemStack], tick: u64) -> JournalEntry {
    serde_json::from_value(serde_json::json!({
        "Player": {
            "uuid": PLAYER, "opaque_player_fields": opaque,
            "pos": [0.0,80.0,0.0], "yaw":0.0, "pitch":0.0, "health":20.0,
            "food":20, "saturation":5.0, "exhaustion":0.0, "air":300,
            "experience_level":0, "experience_progress":0.0, "experience_total":0,
            "game_mode":0, "selected":0, "inventory": inventory,
            "carried": mc_data::ItemStack::EMPTY, "grid": [],
            "enchantment_offer_seed":0, "tick":tick
        }
    }))
    .unwrap()
}

fn write_opaque(root: &Tag) -> Vec<u8> {
    let mut bytes = Vec::new();
    mc_nbt::write_named(&mut bytes, "", root).unwrap();
    bytes
}

fn rejects_owned_opaque_field(key: &str, value: Tag) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("anvil");
    let native = temp.path().join("native");
    let output = temp.path().join("export");
    let config = crate::test_support::config(native.clone(), 42);
    crate::roundtrip_tests::modern_fixture(&source, &config);
    import_anvil(&config, &source, &native, &temp.path().join("import.json")).unwrap();
    let registry = crate::test_support::registry().unwrap();
    let diamond = registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
        .unwrap();
    let mut inventory = vec![mc_data::ItemStack::EMPTY; 41];
    inventory[0] = mc_data::ItemStack::new(diamond, 3);
    let opaque = write_opaque(&Tag::Compound(vec![(key.into(), value)]));
    let mut writer = JournalWriter::open(&native, 79).unwrap();
    writer
        .append(80, 80, &[entry(Vec::new(), &inventory, 80)])
        .unwrap();
    writer
        .append(81, 81, &[entry(opaque, &inventory, 81)])
        .unwrap();
    writer.flush().unwrap();
    drop(writer);
    let before = hashes(&native).unwrap();
    let error = match bridge::recover_offline(&config) {
        Ok(_) => panic!("owned WAL opaque field {key} published recovered state"),
        Err(error) => error.to_string(),
    };
    for detail in [PLAYER, key, "owned field in WAL player opaque"] {
        assert!(error.contains(detail), "missing {detail}: {error}");
    }
    assert_eq!(
        hashes(&native).unwrap(),
        before,
        "refusal changed checkpoint or WAL"
    );
    let receipt_path = temp.path().join("export.json");
    let error = export_anvil(&config, &native, &output, &receipt_path)
        .unwrap_err()
        .to_string();
    for detail in [PLAYER, key, "owned field in WAL player opaque"] {
        assert!(error.contains(detail), "missing {detail}: {error}");
    }
    assert!(!output.exists(), "refusal published a partial export");
    assert_eq!(
        hashes(&native).unwrap(),
        before,
        "failed export changed native custody"
    );
    let receipt: ConversionReceipt =
        serde_json::from_slice(&fs::read(receipt_path).unwrap()).unwrap();
    assert_eq!(receipt.status, "FAIL");
    assert!(receipt.output_hashes.is_empty());
    assert!(receipt.checkpoint_tick.is_none());
}

#[test]
fn repeated_inventory_slot_in_single_opaque_field_refuses_wal_recovery_and_export() {
    rejects_owned_opaque_field(
        "Inventory",
        list(vec![stack(Some(0), 7), stack(Some(0), 11)]),
    );
}

#[test]
fn missing_inventory_slot_in_single_opaque_field_refuses_wal_recovery_and_export() {
    rejects_owned_opaque_field("Inventory", list(vec![stack(None, 7)]));
}

#[test]
fn single_nonempty_opaque_carried_refuses_wal_recovery_and_export() {
    rejects_owned_opaque_field("krasnokamen:Carried", stack(None, 7));
}

#[test]
fn single_nested_recipe_list_refuses_wal_recovery_and_export() {
    rejects_owned_opaque_field(
        "recipeBook",
        Tag::Compound(vec![
            (
                "recipes".into(),
                Tag::List(ListTag {
                    element_type: tag_type::STRING,
                    elements: vec![Tag::String("minecraft:crafting_table".into())],
                }),
            ),
            ("isGuiOpen".into(), Tag::Byte(1)),
        ]),
    );
}

#[test]
fn other_owned_fields_refuse_even_with_valid_or_empty_payloads() {
    for (key, value) in [
        (
            "equipment",
            Tag::Compound(vec![("head".into(), stack(None, 1))]),
        ),
        ("krasnokamen:CraftingGrid", list(vec![stack(None, 5)])),
        ("Inventory", list(vec![])),
        ("Health", Tag::Float(20.0)),
    ] {
        rejects_owned_opaque_field(key, value);
    }
}

#[test]
fn clean_wal_keeps_typed_items_and_supported_opaque_fields_through_export() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("anvil");
    let native = temp.path().join("native");
    let output = temp.path().join("export");
    let config = crate::test_support::config(native.clone(), 42);
    crate::roundtrip_tests::modern_fixture(&source, &config);
    import_anvil(&config, &source, &native, &temp.path().join("import.json")).unwrap();
    let registry = crate::test_support::registry().unwrap();
    let diamond = registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
        .unwrap();
    let mut inventory = vec![mc_data::ItemStack::EMPTY; 41];
    inventory[0] = mc_data::ItemStack::new(diamond, 3);
    let opaque = Tag::Compound(vec![
        ("plugin:payload".into(), Tag::String("retained".into())),
        ("EnderItems".into(), list(vec![stack(Some(26), 13)])),
        ("attributes".into(), list(vec![])),
        (
            "Tags".into(),
            Tag::List(ListTag {
                element_type: tag_type::STRING,
                elements: vec![Tag::String("custody".into())],
            }),
        ),
        (
            "recipeBook".into(),
            Tag::Compound(vec![
                ("isGuiOpen".into(), Tag::Byte(1)),
                ("isFilteringCraftable".into(), Tag::Byte(0)),
                ("plugin:retained".into(), Tag::String("book state".into())),
            ]),
        ),
    ]);
    let mut writer = JournalWriter::open(&native, 79).unwrap();
    writer
        .append(80, 80, &[entry(write_opaque(&opaque), &inventory, 80)])
        .unwrap();
    writer.flush().unwrap();
    drop(writer);
    let before = hashes(&native).unwrap();
    let recovered = bridge::recover_offline(&config).unwrap();
    assert_eq!(
        recovered.dimensions[&Dimension::Overworld].players[PLAYER].inventory,
        inventory
    );
    drop(recovered);
    assert_eq!(
        export_anvil(&config, &native, &output, &temp.path().join("export.json"))
            .unwrap()
            .status,
        "PASS"
    );
    let tag = crate::read_nbt(&output.join(PLAYER_PATH)).unwrap();
    let restored = mc_core::journal::player_from_tag(&tag, &registry.items).unwrap();
    assert_eq!(restored.inventory, inventory);
    let Tag::Compound(fields) = tag else {
        unreachable!()
    };
    for (key, expected) in [
        ("plugin:payload", Tag::String("retained".into())),
        ("EnderItems", list(vec![stack(Some(26), 13)])),
    ] {
        assert_eq!(mc_data::item_nbt::field(&fields, key), Some(&expected));
    }
    assert!(mc_data::item_nbt::field(&fields, "attributes").is_some());
    assert!(mc_data::item_nbt::field(&fields, "Tags").is_some());
    let Some(Tag::Compound(book)) = mc_data::item_nbt::field(&fields, "recipeBook") else {
        panic!("retained recipe book");
    };
    for (key, expected) in [
        ("isGuiOpen", Tag::Byte(1)),
        ("isFilteringCraftable", Tag::Byte(0)),
        ("plugin:retained", Tag::String("book state".into())),
    ] {
        assert_eq!(mc_data::item_nbt::field(book, key), Some(&expected));
    }
    assert_eq!(hashes(&native).unwrap(), before);
}
