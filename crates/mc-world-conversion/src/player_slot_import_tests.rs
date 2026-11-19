//! Slot admission must precede every lossy projection, including old equipment DFU.
use super::*;
use mc_core::dimensions::Dimension;
use mc_core::native_checkpoint as bridge;
use mc_nbt::{ListTag, tag_type};
use mc_world::native::{NativeCheckpoint, NativeStore};

const PLAYER: &str = "00000000-0000-0000-0000-000000000003";
const PLAYER_PATH: &str = "players/data/00000000-0000-0000-0000-000000000003.dat";

fn list(entries: Vec<Tag>) -> Tag {
    Tag::List(ListTag {
        element_type: tag_type::COMPOUND,
        elements: entries,
    })
}

fn item(slot: Option<Tag>, count: i32) -> Tag {
    let mut fields = vec![
        ("id".into(), Tag::String("minecraft:diamond".into())),
        ("count".into(), Tag::Int(count)),
    ];
    if let Some(slot) = slot {
        fields.push(("Slot".into(), slot));
    }
    Tag::Compound(fields)
}

fn replace(tag: &mut Tag, key: &str, value: Tag) {
    let Tag::Compound(fields) = tag else {
        panic!("player compound")
    };
    fields.retain(|(name, _)| name != key);
    fields.push((key.into(), value));
}

fn refused(edit: impl FnOnce(&mut Tag), detail: &str) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("anvil");
    let native = temp.path().join("native");
    let config = crate::test_support::config(native.clone(), 42);
    crate::roundtrip_tests::modern_fixture(&source, &config);
    // An ordinary earlier player must not be published when this player fails.
    let ordinary = crate::read_nbt(&source.join(PLAYER_PATH)).unwrap();
    crate::roundtrip_tests::save(
        &source,
        "players/data/00000000-0000-0000-0000-000000000002.dat",
        &ordinary,
    );
    let mut player = ordinary;
    edit(&mut player);
    crate::roundtrip_tests::save(&source, PLAYER_PATH, &player);
    let before = hashes(&source).unwrap();
    let receipt_path = temp.path().join("import.json");
    let result = import_anvil(&config, &source, &native, &receipt_path);
    let error = result
        .expect_err("ambiguous custody must refuse import before load/save")
        .to_string();
    for expected in [PLAYER, detail] {
        assert!(error.contains(expected), "missing {expected}: {error}");
    }
    assert!(
        !native.exists(),
        "refusal published a loadable/saveable world"
    );
    assert_eq!(hashes(&source).unwrap(), before, "source custody changed");
    let receipt: ConversionReceipt =
        serde_json::from_slice(&fs::read(receipt_path).unwrap()).unwrap();
    assert_eq!(receipt.status, "FAIL");
    assert!(receipt.output_hashes.is_empty());
    assert!(receipt.checkpoint_tick.is_none());
    assert!(receipt.error.unwrap().contains(detail));
    assert!(
        fs::read_dir(temp.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".krasnokamen-conversion-")
        }),
        "refusal retained a partial staging world"
    );
}

#[test]
fn missing_inventory_slot_refuses_import_before_load_save() {
    refused(
        |player| replace(player, "Inventory", list(vec![item(None, 7)])),
        "Inventory entry 0 Slot: missing byte Slot",
    );
}

#[test]
fn repeated_inventory_slot_refuses_import_before_load_save() {
    refused(
        |player| {
            replace(
                player,
                "Inventory",
                list(vec![
                    item(Some(Tag::Byte(4)), 7),
                    item(Some(Tag::Byte(4)), 11),
                ]),
            )
        },
        "Inventory entry 1 Slot: duplicate 4",
    );
}

#[test]
fn unsupported_player_slots_refuse_without_partial_publication() {
    for slot in [Tag::Byte(-1), Tag::Byte(36), Tag::Byte(100), Tag::Int(4)] {
        refused(
            |player| replace(player, "Inventory", list(vec![item(Some(slot), 7)])),
            "Inventory entry 0 Slot:",
        );
    }
    for (entries, detail) in [
        (vec![item(None, 7)], "missing byte Slot"),
        (
            vec![item(Some(Tag::Byte(27)), 7)],
            "27 outside supported range",
        ),
        (
            vec![item(Some(Tag::Byte(2)), 7), item(Some(Tag::Byte(2)), 11)],
            "duplicate 2",
        ),
    ] {
        refused(
            |player| replace(player, "EnderItems", list(entries)),
            detail,
        );
    }
    for name in ["unknown", "mainhand", "body", "saddle"] {
        refused(
            |player| {
                replace(
                    player,
                    "equipment",
                    Tag::Compound(vec![(name.into(), item(None, 7))]),
                )
            },
            &format!("slot {name}"),
        );
    }
    refused(
        |player| {
            replace(
                player,
                "equipment",
                Tag::Compound(vec![
                    ("head".into(), item(None, 7)),
                    ("head".into(), item(None, 11)),
                ]),
            )
        },
        "equipment slot head: duplicate",
    );
    for key in ["Inventory", "EnderItems", "equipment"] {
        refused(
            |player| {
                let Tag::Compound(fields) = player else {
                    unreachable!()
                };
                let value = if key == "equipment" {
                    Tag::Compound(vec![])
                } else {
                    list(vec![])
                };
                if !fields.iter().any(|(name, _)| name == key) {
                    fields.push((key.into(), value.clone()));
                }
                fields.push((key.into(), value));
            },
            "duplicate",
        );
    }
    for key in ["Inventory", "EnderItems"] {
        refused(
            |player| {
                let mut entry = item(Some(Tag::Byte(2)), 7);
                let Tag::Compound(fields) = &mut entry else {
                    unreachable!()
                };
                fields.push(("Slot".into(), Tag::Byte(3)));
                replace(player, key, list(vec![entry]));
            },
            "duplicate",
        );
    }
}

#[test]
fn repeated_legacy_equipment_slot_refuses_before_dfu_fold() {
    for slot in [100, 101, 102, 103, -106] {
        refused(
            |player| {
                replace(player, "DataVersion", Tag::Int(4311));
                replace(
                    player,
                    "Inventory",
                    list(vec![
                        item(Some(Tag::Byte(slot)), 7),
                        item(Some(Tag::Byte(slot)), 11),
                    ]),
                );
            },
            &format!("Inventory entry 1 Slot: duplicate {slot}"),
        );
    }
}

#[test]
fn ordinary_player_inventory_survives_import_load_save_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("anvil");
    let native = temp.path().join("native");
    let config = crate::test_support::config(native.clone(), 42);
    crate::roundtrip_tests::modern_fixture(&source, &config);
    let mut player = crate::read_nbt(&source.join(PLAYER_PATH)).unwrap();
    replace(
        &mut player,
        "Inventory",
        list(vec![
            item(Some(Tag::Byte(0)), 7),
            item(Some(Tag::Byte(35)), 11),
        ]),
    );
    replace(
        &mut player,
        "equipment",
        Tag::Compound(vec![
            ("head".into(), item(None, 1)),
            ("offhand".into(), item(None, 3)),
        ]),
    );
    replace(
        &mut player,
        "EnderItems",
        list(vec![item(Some(Tag::Byte(26)), 13)]),
    );
    crate::roundtrip_tests::save(&source, PLAYER_PATH, &player);
    let before = hashes(&source).unwrap();
    assert_eq!(
        import_anvil(&config, &source, &native, &temp.path().join("import.json"))
            .unwrap()
            .status,
        "PASS"
    );
    let registry = crate::test_support::registry().unwrap();
    let expected = mc_core::journal::player_from_tag(&player, &registry.items).unwrap();
    let mut recovered = bridge::recover_offline(&config).unwrap();
    let loaded = &recovered.dimensions[&Dimension::Nether].players[PLAYER];
    let diamond = registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
        .unwrap();
    let mut slots = vec![mc_data::ItemStack::EMPTY; 41];
    for (slot, count) in [(0, 7), (35, 11), (39, 1), (40, 3)] {
        slots[slot] = mc_data::ItemStack::new(diamond, count);
    }
    assert_eq!(loaded.inventory, slots);
    assert_eq!(loaded.inventory, expected.inventory);
    assert_eq!(loaded.opaque_player_fields, expected.opaque_player_fields);
    for replay in recovered.dimensions.values_mut() {
        replay.tick = 80;
        for player in replay.players.values_mut() {
            player.tick = 80;
        }
    }
    let image = bridge::encode_checkpoint(&recovered).unwrap();
    drop(recovered);
    let store = NativeStore::open(
        &native.join("native/state.redb"),
        bridge::native_identity(&config),
    )
    .unwrap();
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick: 80,
                previous_tick: 79,
                chunks: &[],
                entities: &image.entities,
                players: &image.players,
                globals: &image.globals,
            },
            &registry.blocks,
            &registry.items,
        )
        .unwrap();
    drop(store);
    let reopened = bridge::recover_offline(&config).unwrap();
    let saved = &reopened.dimensions[&Dimension::Nether].players[PLAYER];
    assert_eq!(saved.inventory, expected.inventory);
    assert_eq!(saved.inventory, slots);
    let saved_tag = mc_core::journal::player_to_tag(saved, &registry.items).unwrap();
    let Tag::Compound(fields) = saved_tag else {
        panic!("saved player compound")
    };
    assert_eq!(
        mc_data::item_nbt::field(&fields, "EnderItems"),
        Some(&list(vec![item(Some(Tag::Byte(26)), 13)]))
    );
    assert_eq!(saved.opaque_player_fields, expected.opaque_player_fields);
    assert_eq!(hashes(&source).unwrap(), before);
}

fn malformed_item_id_refuses_import(id: Option<Tag>) {
    for location in [
        "Inventory",
        "equipment",
        "krasnokamen:Carried",
        "krasnokamen:CraftingGrid",
    ] {
        refused(
            |player| {
                let mut fields = vec![
                    ("count".into(), Tag::Int(7)),
                    (
                        "components".into(),
                        Tag::Compound(vec![(
                            "plugin:payload".into(),
                            Tag::String("must not disappear".into()),
                        )]),
                    ),
                ];
                if let Some(id) = id.clone() {
                    fields.push(("id".into(), id));
                }
                let value = match location {
                    "Inventory" => {
                        fields.push(("Slot".into(), Tag::Byte(0)));
                        list(vec![Tag::Compound(fields)])
                    }
                    "equipment" => Tag::Compound(vec![("head".into(), Tag::Compound(fields))]),
                    "krasnokamen:Carried" => Tag::Compound(fields),
                    _ => list(vec![Tag::Compound(fields)]),
                };
                replace(player, location, value);
            },
            "missing or non-string id",
        );
    }
}

#[test]
fn missing_item_id_with_payload_refuses_import_before_load_save() {
    malformed_item_id_refuses_import(None);
}

#[test]
fn wrong_type_item_id_with_payload_refuses_import_before_load_save() {
    malformed_item_id_refuses_import(Some(Tag::Int(7)));
}

fn duplicate_custody_refuses_import(location: &str) {
    refused(
        |player| {
            let Tag::Compound(fields) = player else {
                unreachable!()
            };
            fields.retain(|(name, _)| name != location);
            let empty_item = Tag::Compound(vec![]);
            let malformed_item = Tag::Compound(vec![
                ("count".into(), Tag::Int(7)),
                (
                    "components".into(),
                    Tag::Compound(vec![(
                        "plugin:payload".into(),
                        Tag::String("must not disappear".into()),
                    )]),
                ),
            ]);
            let (first, second) = if location == "krasnokamen:Carried" {
                (empty_item, malformed_item)
            } else {
                (list(vec![empty_item; 4]), list(vec![malformed_item]))
            };
            fields.push((location.into(), first));
            fields.push((location.into(), second));
        },
        &format!("{location}: duplicate owned custody field"),
    );
}

#[test]
fn duplicate_carried_refuses_import_before_first_match() {
    duplicate_custody_refuses_import("krasnokamen:Carried");
}

#[test]
fn duplicate_crafting_grid_refuses_import_before_first_match() {
    duplicate_custody_refuses_import("krasnokamen:CraftingGrid");
}

#[test]
fn duplicate_embedded_player_custody_refuses_import_before_publication() {
    for key in ["krasnokamen:Carried", "krasnokamen:CraftingGrid"] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("anvil");
        let native = temp.path().join("native");
        let config = crate::test_support::config(native.clone(), 42);
        crate::roundtrip_tests::modern_fixture(&source, &config);
        let mut player = crate::read_nbt(&source.join(PLAYER_PATH)).unwrap();
        let Tag::Compound(fields) = &mut player else {
            unreachable!()
        };
        fields.push(("UUID".into(), Tag::IntArray(vec![0, 0, 0, 3])));
        let empty = Tag::Compound(vec![]);
        let malformed = Tag::Compound(vec![
            ("count".into(), Tag::Int(7)),
            ("payload".into(), Tag::String("retained".into())),
        ]);
        let values = if key == "krasnokamen:Carried" {
            [empty, malformed]
        } else {
            [empty, malformed].map(|item| list(vec![item]))
        };
        fields.extend(values.into_iter().map(|value| (key.into(), value)));
        let mut level = crate::read_nbt(&source.join("level.dat")).unwrap();
        let Tag::Compound(root) = &mut level else {
            unreachable!()
        };
        let (_, Tag::Compound(data)) = root.iter_mut().find(|(key, _)| key == "Data").unwrap()
        else {
            unreachable!()
        };
        data.push(("Player".into(), player));
        crate::roundtrip_tests::save(&source, "level.dat", &level);
        let before = hashes(&source).unwrap();
        let error = import_anvil(&config, &source, &native, &temp.path().join("import.json"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("level.dat#Data.Player"), "{error}");
        assert!(
            error.contains(&format!("{key}: duplicate owned custody field")),
            "{error}"
        );
        assert!(!native.exists());
        assert_eq!(hashes(&source).unwrap(), before);
    }
}

#[test]
fn duplicate_wal_player_custody_refuses_offline_export_without_publication() {
    for key in ["krasnokamen:Carried", "krasnokamen:CraftingGrid"] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("anvil");
        let native = temp.path().join("native");
        let output = temp.path().join("export");
        let config = crate::test_support::config(native.clone(), 42);
        crate::roundtrip_tests::modern_fixture(&source, &config);
        import_anvil(&config, &source, &native, &temp.path().join("import.json")).unwrap();
        let empty = Tag::Compound(vec![]);
        let malformed = Tag::Compound(vec![
            ("count".into(), Tag::Int(7)),
            ("payload".into(), Tag::String("retained".into())),
        ]);
        let values = if key == "krasnokamen:Carried" {
            [empty, malformed]
        } else {
            [empty, malformed].map(|item| list(vec![item]))
        };
        let mut opaque = Vec::new();
        mc_nbt::write_named(
            &mut opaque,
            "",
            &Tag::Compound(
                values
                    .into_iter()
                    .map(|value| (key.into(), value))
                    .collect(),
            ),
        )
        .unwrap();
        let entry: mc_core::journal::JournalEntry = serde_json::from_value(serde_json::json!({
            "Player": {
                "uuid": PLAYER, "opaque_player_fields": opaque,
                "pos": [0.0,80.0,0.0], "yaw":0.0, "pitch":0.0, "health":20.0,
                "food":20, "saturation":5.0, "exhaustion":0.0, "air":300,
                "experience_level":0, "experience_progress":0.0, "experience_total":0,
                "game_mode":0, "selected":0,
                "inventory": vec![mc_data::ItemStack::EMPTY;41],
                "carried": mc_data::ItemStack::EMPTY, "grid": [],
                "enchantment_offer_seed":0, "tick":80
            }
        }))
        .unwrap();
        let mut writer = mc_core::journal::JournalWriter::open(
            &native,
            79,
            mc_core::journal::WalIdentity::fixture(),
        )
        .unwrap();
        writer.append(80, 80, &[entry]).unwrap();
        writer.flush().unwrap();
        drop(writer);
        let before = hashes(&native).unwrap();
        let error = export_anvil(&config, &native, &output, &temp.path().join("export.json"))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(key) && error.contains("duplicate owned custody field"),
            "{error}"
        );
        assert!(!output.exists());
        assert_eq!(hashes(&native).unwrap(), before);
    }
}
