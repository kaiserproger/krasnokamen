use super::*;
use crate::journal::{JournalEntry, JournalWriter, ReplayPlayer};

fn player_entry(uuid: uuid::Uuid, player: &ReplayPlayer, tick: u64) -> JournalEntry {
    JournalEntry::Player {
        uuid: uuid.to_string(),
        opaque_player_fields: player.opaque_player_fields.clone(),
        statistics: Box::new(player.statistics.clone()),
        advancement_progress: None,
        unlocked_recipes: None,
        dimension: player.dimension,
        pos: player.pos,
        yaw: player.yaw,
        pitch: player.pitch,
        health: player.health,
        food: player.food,
        saturation: player.saturation,
        exhaustion: player.exhaustion,
        air: player.air,
        experience_level: player.experience_level,
        experience_progress: player.experience_progress,
        experience_total: player.experience_total,
        game_mode: player.game_mode,
        selected: player.selected,
        inventory: player.inventory.clone(),
        carried: player.carried.clone(),
        grid: player.grid.clone(),
        respawn: player.respawn,
        respawn_dimension: player.respawn_dimension,
        respawn_forced: player.respawn_forced,
        respawn_yaw: player.respawn_yaw,
        respawn_pitch: player.respawn_pitch,
        active_effects: player.active_effects.clone(),
        enchantment_seed: player.enchantment_seed,
        enchantment_offer_seed: player.enchantment_offer_seed,
        warden_warning: player.warden_warning,
        current_impulse_impact_pos: player.current_impulse_impact_pos,
        fall_flying: player.fall_flying,
        burn_ticks: player.burn_ticks,
        portal: player.portal,
        tick,
    }
}

fn wal_inventory_recovery(size: usize, duplicate: Option<&str>) {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let registry = crate::registry::CoreRegistry::load(None).unwrap();
    let owner = uuid::Uuid::from_u128(0x5107);
    let bad_owner = uuid::Uuid::from_u128(0x5108);
    let mut player =
        crate::journal::player_from_tag(&Tag::Compound(vec![]), &registry.items).unwrap();
    let stone = registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap();
    player.inventory[0] = mc_data::ItemStack::new(stone, 3);
    write_player(&config, &owner.to_string(), &player, &registry.items).unwrap();
    player.inventory[0].count = 7;
    let mut bad = player.clone();
    bad.inventory
        .resize(size, mc_data::ItemStack::new(stone, 11));
    if let Some(key) = duplicate {
        let empty = Tag::Compound(vec![]);
        let malformed = malformed_item(None);
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
        mc_nbt::write_named(
            &mut bad.opaque_player_fields,
            "",
            &Tag::Compound(
                values
                    .into_iter()
                    .map(|value| (key.into(), value))
                    .collect(),
            ),
        )
        .unwrap();
    }
    let mut writer =
        JournalWriter::open(directory.path(), 1, crate::journal::WalIdentity::fixture()).unwrap();
    writer
        .append(2, 2, &[player_entry(owner, &player, 2)])
        .unwrap();
    writer
        .append(3, 3, &[player_entry(bad_owner, &bad, 3)])
        .unwrap();
    writer.flush().unwrap();
    drop(writer);
    let db_path = directory.path().join("native/state.redb");
    let db_before = std::fs::read(&db_path).unwrap();
    let batches: Vec<_> = [2, 3]
        .into_iter()
        .map(|tick| {
            let path = directory
                .path()
                .join(format!("journal/batch-{tick:020}-{tick:020}.bin"));
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    let recovered = recover_offline(&config);
    if let Some(key) = duplicate {
        let error = match recovered {
            Ok(_) => panic!("duplicate WAL custody {key} published recovered state"),
            Err(error) => error.to_string(),
        };
        for detail in [
            bad_owner.to_string(),
            key.into(),
            "duplicate owned custody field".into(),
        ] {
            assert!(error.contains(&detail), "{error}");
        }
        let save_error = crate::journal::player_to_tag(&bad, &registry.items)
            .unwrap_err()
            .to_string();
        assert!(
            save_error.contains("duplicate owned custody field"),
            "{save_error}"
        );
    } else if size == crate::inventory::PLAYER_SLOTS {
        let recovered = recovered.unwrap();
        let players = &recovered.dimensions[&Dimension::Overworld].players;
        assert_eq!(players[&owner.to_string()].inventory[0].count, 7);
        assert_eq!(players[&bad_owner.to_string()].inventory, bad.inventory);
    } else {
        let error = match recovered {
            Ok(_) => {
                panic!("invalid WAL player inventory published recovered state: length {size}")
            }
            Err(error) => error.to_string(),
        };
        for detail in [
            bad_owner.to_string(),
            "inventory slots: expected 41".into(),
            format!("found {size}"),
        ] {
            assert!(error.contains(&detail), "{error}");
        }
    }
    assert_eq!(
        std::fs::read(db_path).unwrap(),
        db_before,
        "recovery rewrote checkpoint custody"
    );
    // A writable redb open updates its allocation/header bookkeeping even on
    // refusal; prove read-only byte identity first, then live logical custody.
    if let Some(key) = duplicate {
        let live_error = match crate::core::recover_dimension_state(&config, Dimension::Overworld) {
            Ok(_) => panic!("duplicate WAL custody {key} reached live dimension state"),
            Err(error) => error.to_string(),
        };
        assert!(live_error.contains(key), "{live_error}");
        assert_eq!(read_manifest(&config).unwrap().tick, 1);
    }
    for (path, bytes) in batches {
        assert_eq!(
            std::fs::read(path).unwrap(),
            bytes,
            "recovery changed or pruned WAL"
        );
    }
    assert_eq!(
        read_player(&config, &owner.to_string(), &registry.items)
            .unwrap()
            .inventory[0]
            .count,
        3,
        "earlier valid WAL image was partially checkpointed"
    );
}

#[test]
fn short_wal_inventory_refuses_recovery_without_partial_load() {
    wal_inventory_recovery(40, None);
}

#[test]
fn long_wal_inventory_refuses_recovery_without_partial_load() {
    wal_inventory_recovery(42, None);
}

#[test]
fn ordinary_wal_inventory_recovers_all_players_without_rewriting_checkpoint() {
    wal_inventory_recovery(41, None);
}

#[test]
fn duplicate_carried_in_wal_opaque_refuses_recovery() {
    wal_inventory_recovery(41, Some("krasnokamen:Carried"));
}

#[test]
fn duplicate_crafting_grid_in_wal_opaque_refuses_recovery() {
    wal_inventory_recovery(41, Some("krasnokamen:CraftingGrid"));
}

fn malformed_item(id: Option<Tag>) -> Tag {
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
    if let Some(id) = id {
        fields.push(("id".into(), id));
    }
    Tag::Compound(fields)
}

fn malformed_checkpoint_item(id: Option<Tag>, duplicate: Option<&str>) {
    for location in [
        "Inventory",
        "equipment",
        "krasnokamen:Carried",
        "krasnokamen:CraftingGrid",
    ] {
        if duplicate.is_some_and(|key| key != location) {
            continue;
        }
        let directory = tempfile::tempdir().unwrap();
        let config = CoreConfig::new(directory.path().to_path_buf(), 42);
        let registry = crate::registry::CoreRegistry::load(None).unwrap();
        let owner = uuid::Uuid::from_u128(0x5107);
        let bad_owner = uuid::Uuid::from_u128(0x5108);
        let mut player =
            crate::journal::player_from_tag(&Tag::Compound(vec![]), &registry.items).unwrap();
        let stone = registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
            .unwrap();
        player.inventory[0] = mc_data::ItemStack::new(stone, 3);
        write_player(&config, &owner.to_string(), &player, &registry.items).unwrap();
        let store = fixture_store(&config).unwrap();
        let mut image = store.read_checkpoint().unwrap();
        let tick = image.tick + 1;
        player.tick = tick;
        let mut tag = crate::journal::player_to_tag(&player, &registry.items).unwrap();
        let Tag::Compound(fields) = &mut tag else {
            unreachable!()
        };
        let mut item = malformed_item(id.clone());
        let value = match location {
            "Inventory" => {
                let Tag::Compound(fields) = &mut item else {
                    unreachable!()
                };
                fields.push(("Slot".into(), Tag::Byte(0)));
                Tag::List(mc_nbt::ListTag {
                    element_type: mc_nbt::tag_type::COMPOUND,
                    elements: vec![item],
                })
            }
            "equipment" => Tag::Compound(vec![("head".into(), item)]),
            "krasnokamen:Carried" => item,
            _ => Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::COMPOUND,
                elements: vec![item],
            }),
        };
        fields.retain(|(name, _)| name != location);
        if duplicate.is_some() {
            let empty = if location == "krasnokamen:Carried" {
                Tag::Compound(vec![])
            } else {
                Tag::List(mc_nbt::ListTag {
                    element_type: mc_nbt::tag_type::COMPOUND,
                    elements: vec![Tag::Compound(vec![]); 4],
                })
            };
            fields.push((location.into(), empty));
        }
        fields.push((location.into(), value));
        let mut manifest: CheckpointManifest =
            serde_json::from_slice(&image.globals["checkpoint"].payload).unwrap();
        manifest.tick = tick;
        for child in manifest.dimensions.values_mut() {
            child.tick = tick;
        }
        for global in image.globals.values_mut() {
            global.captured_tick = tick;
        }
        image.globals.insert(
            "checkpoint".into(),
            record(tick, serde_json::to_vec(&manifest).unwrap()),
        );
        if duplicate.is_some() {
            let error = crate::journal::player_from_tag(&tag, &registry.items)
                .unwrap_err()
                .to_string();
            assert!(error.contains("duplicate owned custody field"), "{error}");
            let error = store
                .commit_checkpoint(
                    NativeCheckpoint {
                        tick,
                        previous_tick: image.tick,
                        chunks: &[],
                        entities: &[],
                        players: &[mc_world::native::NativePlayerSnapshot {
                            uuid: *bad_owner.as_bytes(),
                            record: tag_record(tick, &tag).unwrap(),
                        }],
                        globals: &image.globals,
                    },
                    &registry.blocks,
                    &registry.items,
                )
                .unwrap_err()
                .to_string();
            assert!(error.contains("NBT/duplicate_field"), "{error}");
            continue;
        }
        let players = [mc_world::native::NativePlayerSnapshot {
            uuid: *bad_owner.as_bytes(),
            record: tag_record(tick, &tag).unwrap(),
        }];
        store
            .commit_checkpoint(
                NativeCheckpoint {
                    tick,
                    previous_tick: image.tick,
                    chunks: &[],
                    entities: &[],
                    players: &players,
                    globals: &image.globals,
                },
                &registry.blocks,
                &registry.items,
            )
            .unwrap();
        drop(store);
        let db_path = directory.path().join("native/state.redb");
        let before = std::fs::read(&db_path).unwrap();
        let error = match recover_offline(&config) {
            Ok(_) => panic!("malformed item {location} published recovered state"),
            Err(error) => error.to_string(),
        };
        for detail in [
            bad_owner.to_string(),
            location.into(),
            if duplicate.is_some() {
                "duplicate owned custody field".into()
            } else {
                "missing or non-string id".into()
            },
        ] {
            assert!(error.contains(&detail), "{error}");
        }
        assert_eq!(
            std::fs::read(db_path).unwrap(),
            before,
            "refused recovery rewrote native custody"
        );
        assert_eq!(
            read_player(&config, &owner.to_string(), &registry.items)
                .unwrap()
                .inventory[0]
                .count,
            3
        );
    }
}

#[test]
fn missing_item_id_with_payload_refuses_checkpoint_recovery() {
    malformed_checkpoint_item(None, None);
}

#[test]
fn wrong_type_item_id_with_payload_refuses_checkpoint_recovery() {
    malformed_checkpoint_item(Some(Tag::Int(7)), None);
}

#[test]
fn duplicate_carried_refuses_checkpoint_recovery_before_first_match() {
    malformed_checkpoint_item(None, Some("krasnokamen:Carried"));
}

#[test]
fn duplicate_crafting_grid_refuses_checkpoint_recovery_before_first_match() {
    malformed_checkpoint_item(None, Some("krasnokamen:CraftingGrid"));
}

#[test]
fn duplicate_recipe_book_custody_refuses_even_with_native_recipe_authority() {
    let items = mc_data::items::krasnokamen_required_items();
    let tag = Tag::Compound(vec![
        (
            "KrasnokamenUnlockedRecipes".into(),
            Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::STRING,
                elements: vec![],
            }),
        ),
        (
            "recipeBook".into(),
            Tag::Compound(vec![
                (
                    "recipes".into(),
                    Tag::List(mc_nbt::ListTag {
                        element_type: mc_nbt::tag_type::STRING,
                        elements: vec![],
                    }),
                ),
                (
                    "recipes".into(),
                    Tag::String("unread second custody".into()),
                ),
            ]),
        ),
    ]);
    let error = crate::journal::player_from_tag(&tag, &items)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("recipeBook.recipes: duplicate owned custody field"),
        "{error}"
    );
}

#[test]
fn empty_player_equipment_cursor_and_grid_compounds_remain_empty_slots() {
    let items = mc_data::items::krasnokamen_required_items();
    let tag = Tag::Compound(vec![
        (
            "equipment".into(),
            Tag::Compound(vec![("head".into(), Tag::Compound(vec![]))]),
        ),
        ("krasnokamen:Carried".into(), Tag::Compound(vec![])),
        (
            "krasnokamen:CraftingGrid".into(),
            Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::COMPOUND,
                elements: vec![Tag::Compound(vec![]); 4],
            }),
        ),
    ]);
    let player = crate::journal::player_from_tag(&tag, &items).unwrap();
    assert_eq!(player.inventory.len(), 41);
    assert!(player.inventory.iter().all(mc_data::ItemStack::is_empty));
    assert!(player.carried.is_empty());
    assert_eq!(player.grid.len(), 4);
    assert!(player.grid.iter().all(mc_data::ItemStack::is_empty));
}
