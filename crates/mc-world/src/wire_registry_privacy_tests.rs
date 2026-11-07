use super::*;
use crate::chunk::ChunkPos;
use mc_data::block_entity_kinds::BlockEntityKind as K;
use mc_data::blocks::{BlockReport, BlockStateReport};
use mc_nbt::{ListTag, tag_type};
use std::collections::BTreeMap;

#[test]
fn registry_block_entities_exclude_unrelated_saved_fields() {
    let report_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/vanilla/reports/registries.json");
    let report = mc_data::block_entity_types::load_block_entity_types_report(report_path)
        .expect("the 26.3 block-entity registry report is required");
    assert_eq!(
        report.len(),
        K::ALL.len(),
        "registry and explicit expectations differ"
    );
    for entry in report {
        let kind = K::ALL
            .iter()
            .copied()
            .find(|kind| kind.vanilla_id() == entry.id.as_str())
            .expect("registry type has no explicit privacy expectation");
        assert_eq!(kind.protocol_id(), entry.protocol_id, "{}", entry.id);
        let registry = BlockRegistry::from_report(&[
            BlockReport {
                id: Identifier::parse("minecraft:air").unwrap(),
                properties: BTreeMap::new(),
                states: vec![BlockStateReport {
                    id: 0,
                    default: true,
                    properties: BTreeMap::new(),
                }],
            },
            BlockReport {
                id: Identifier::parse(format!("minecraft:{}", kind.block_paths()[0])).unwrap(),
                properties: BTreeMap::new(),
                states: vec![BlockStateReport {
                    id: 1,
                    default: true,
                    properties: BTreeMap::new(),
                }],
            },
        ])
        .unwrap();
        let expected_keys: &[&str] = match kind {
            K::Furnace
            | K::Chest
            | K::TrappedChest
            | K::EnderChest
            | K::Jukebox
            | K::Dispenser
            | K::Dropper
            | K::BrewingStand
            | K::EnchantingTable
            | K::EndPortal
            | K::DaylightDetector
            | K::Hopper
            | K::Comparator
            | K::CommandBlock
            | K::ShulkerBox
            | K::Barrel
            | K::Smoker
            | K::BlastFurnace
            | K::Lectern
            | K::Bell
            | K::Beehive
            | K::SculkSensor
            | K::CalibratedSculkSensor
            | K::SculkCatalyst
            | K::SculkShrieker
            | K::ChiseledBookshelf
            | K::Crafter
            | K::CopperGolemStatue
            | K::PotentSulfur
            | K::BrushableBlock => &[],
            K::Sign | K::HangingSign => {
                &["is_waxed", "allow_op_features", "front_text", "back_text"]
            }
            K::MobSpawner => &[
                "SpawnData",
                "Delay",
                "MinSpawnDelay",
                "MaxSpawnDelay",
                "SpawnCount",
                "MaxNearbyEntities",
                "RequiredPlayerRange",
                "SpawnRange",
            ],
            K::CreakingHeart => &["creaking"],
            K::Piston => &["facing", "blockState", "progress", "extending", "source"],
            K::Beacon => &["Levels", "CustomName", "primary_effect", "secondary_effect"],
            K::Skull => &["note_block_sound", "profile", "custom_name"],
            K::Banner => &["components", "patterns", "CustomName"],
            K::StructureBlock => &[
                "name",
                "author",
                "metadata",
                "posX",
                "posY",
                "posZ",
                "sizeX",
                "sizeY",
                "sizeZ",
                "rotation",
                "mirror",
                "mode",
                "ignoreEntities",
                "strict",
                "powered",
                "showair",
                "showboundingbox",
                "integrity",
                "seed",
            ],
            K::Jigsaw => &[
                "name",
                "target",
                "pool",
                "final_state",
                "joint",
                "placement_priority",
                "selection_priority",
            ],
            K::EndGateway => &["Age", "exit_portal", "ExactTeleport"],
            K::Conduit => &["Target"],
            K::Campfire => &["Items"],
            K::Shelf => &["Items", "align_items_to_bottom"],
            K::DecoratedPot => &["sherds"],
            K::TrialSpawner => &["spawn_data"],
            K::Vault => &["shared_data"],
            K::TestBlock => &["mode", "message", "powered"],
            K::TestInstanceBlock => &["data", "errors"],
        };
        let empty_list = || {
            Tag::List(ListTag {
                element_type: tag_type::COMPOUND,
                elements: vec![],
            })
        };
        let public_shared =
            Tag::Compound(vec![("connected_particles_range".into(), Tag::Double(3.0))]);
        let fields: Vec<(String, Tag)> = vec![
            (
                "Items".into(),
                Tag::List(ListTag {
                    element_type: tag_type::COMPOUND,
                    elements: vec![Tag::Compound(vec![
                        ("Slot".into(), Tag::Byte(0)),
                        ("id".into(), Tag::String("minecraft:diamond".into())),
                        ("count".into(), Tag::Int(3)),
                    ])],
                }),
            ),
            ("LootTable".into(), Tag::String("minecraft:secret".into())),
            ("LootTableSeed".into(), Tag::Long(42)),
            ("Lock".into(), Tag::String("secret".into())),
            (
                "lock".into(),
                Tag::Compound(vec![(
                    "items".into(),
                    Tag::String("minecraft:diamond".into()),
                )]),
            ),
            ("Command".into(), Tag::String("say secret".into())),
            (
                "bees".into(),
                Tag::List(ListTag {
                    element_type: tag_type::COMPOUND,
                    elements: vec![Tag::Compound(vec![
                        (
                            "entity_data".into(),
                            Tag::Compound(vec![("id".into(), Tag::String("minecraft:bee".into()))]),
                        ),
                        ("ticks_in_hive".into(), Tag::Int(30)),
                        ("min_ticks_in_hive".into(), Tag::Int(600)),
                    ])],
                }),
            ),
            (
                "Book".into(),
                Tag::Compound(vec![
                    ("id".into(), Tag::String("minecraft:written_book".into())),
                    ("count".into(), Tag::Int(1)),
                    (
                        "components".into(),
                        Tag::Compound(vec![(
                            "minecraft:written_book_content".into(),
                            Tag::Compound(vec![
                                ("title".into(), Tag::String("Private".into())),
                                ("author".into(), Tag::String("Author".into())),
                                (
                                    "pages".into(),
                                    Tag::List(ListTag {
                                        element_type: tag_type::STRING,
                                        elements: vec![Tag::String("private page".into())],
                                    }),
                                ),
                            ]),
                        )]),
                    ),
                ]),
            ),
            (
                "components".into(),
                Tag::Compound(vec![(
                    "minecraft:custom_data".into(),
                    Tag::Compound(vec![("secret".into(), Tag::Int(1))]),
                )]),
            ),
            (
                "normal_config".into(),
                Tag::String("minecraft:trial_chamber/melee/zombie/normal".into()),
            ),
            (
                "SpawnPotentials".into(),
                Tag::List(ListTag {
                    element_type: tag_type::COMPOUND,
                    elements: vec![Tag::Compound(vec![
                        ("weight".into(), Tag::Int(1)),
                        (
                            "data".into(),
                            Tag::Compound(vec![(
                                "entity".into(),
                                Tag::Compound(vec![(
                                    "id".into(),
                                    Tag::String("minecraft:zombie".into()),
                                )]),
                            )]),
                        ),
                    ])],
                }),
            ),
            (
                "server_data".into(),
                Tag::Compound(vec![
                    (
                        "rewarded_players".into(),
                        Tag::List(ListTag {
                            element_type: tag_type::INT_ARRAY,
                            elements: vec![Tag::IntArray(vec![0, 0, 0, 1])],
                        }),
                    ),
                    ("state_updating_resumes_at".into(), Tag::Long(123)),
                    (
                        "items_to_eject".into(),
                        Tag::List(ListTag {
                            element_type: tag_type::COMPOUND,
                            elements: vec![Tag::Compound(vec![
                                ("id".into(), Tag::String("minecraft:diamond".into())),
                                ("count".into(), Tag::Int(1)),
                            ])],
                        }),
                    ),
                    ("total_ejections_needed".into(), Tag::Int(1)),
                ]),
            ),
            (
                "shared_data".into(),
                Tag::Compound(vec![
                    ("connected_particles_range".into(), Tag::Double(3.0)),
                    ("secret".into(), Tag::Int(1)),
                ]),
            ),
            (
                "SpawnData".into(),
                Tag::Compound(vec![(
                    "entity".into(),
                    Tag::Compound(vec![("id".into(), Tag::String("minecraft:zombie".into()))]),
                )]),
            ),
            (
                "spawn_data".into(),
                Tag::Compound(vec![(
                    "entity".into(),
                    Tag::Compound(vec![("id".into(), Tag::String("minecraft:zombie".into()))]),
                )]),
            ),
            ("sherds".into(), Tag::Compound(vec![])),
            ("is_waxed".into(), Tag::Byte(1)),
            ("Levels".into(), Tag::Int(3)),
            ("Target".into(), Tag::IntArray(vec![0, 0, 0, 1])),
            ("creaking".into(), Tag::IntArray(vec![1, 64, 2])),
            (
                "note_block_sound".into(),
                Tag::String("minecraft:block.note_block.harp".into()),
            ),
            ("name".into(), Tag::String("minecraft:house".into())),
            ("facing".into(), Tag::Int(1)),
            ("message".into(), Tag::String("visible".into())),
            ("data".into(), Tag::Compound(vec![])),
            ("Age".into(), Tag::Long(5)),
            ("align_items_to_bottom".into(), Tag::Byte(1)),
            ("patterns".into(), empty_list()),
            ("CustomName".into(), Tag::String("visible".into())),
            ("BrewTime".into(), Tag::Short(5)),
            ("Page".into(), Tag::Int(3)),
            ("lit_time_remaining".into(), Tag::Short(5)),
            ("lit_total_time".into(), Tag::Short(6)),
            ("cooking_time_spent".into(), Tag::Short(7)),
            ("cooking_total_time".into(), Tag::Short(8)),
            ("speed_multiplier".into(), Tag::Float(1.5)),
            (
                "RecipesUsed".into(),
                Tag::Compound(vec![("minecraft:iron_ingot".into(), Tag::Int(1))]),
            ),
            ("flower_pos".into(), Tag::IntArray(vec![1, 64, 2])),
            ("total_brew_time".into(), Tag::Int(10)),
            ("Fuel".into(), Tag::Byte(2)),
            ("total_fuel".into(), Tag::Int(10)),
            (
                "item".into(),
                Tag::Compound(vec![
                    ("id".into(), Tag::String("minecraft:diamond".into())),
                    ("count".into(), Tag::Int(1)),
                ]),
            ),
            ("CookingTimes".into(), Tag::IntArray(vec![1, 2, 3, 4])),
            ("CookingTotalTimes".into(), Tag::IntArray(vec![5, 6, 7, 8])),
            ("last_interacted_slot".into(), Tag::Int(2)),
            ("powered".into(), Tag::Byte(1)),
            ("conditionMet".into(), Tag::Byte(1)),
            ("auto".into(), Tag::Byte(1)),
            ("OutputSignal".into(), Tag::Int(12)),
            ("crafting_ticks_remaining".into(), Tag::Int(3)),
            ("TransferCooldown".into(), Tag::Int(7)),
            ("target".into(), Tag::String("minecraft:target".into())),
            ("pool".into(), Tag::String("minecraft:pool".into())),
            ("final_state".into(), Tag::String("minecraft:stone".into())),
            ("joint".into(), Tag::String("rollable".into())),
            ("placement_priority".into(), Tag::Int(1)),
            ("selection_priority".into(), Tag::Int(2)),
            (
                "RecordItem".into(),
                Tag::Compound(vec![
                    ("id".into(), Tag::String("minecraft:music_disc_13".into())),
                    ("count".into(), Tag::Int(1)),
                ]),
            ),
            ("ticks_since_song_started".into(), Tag::Long(20)),
            ("countdown".into(), Tag::Int(10)),
            ("last_vibration_frequency".into(), Tag::Int(5)),
            (
                "listener".into(),
                Tag::Compound(vec![("event_delay".into(), Tag::Int(3))]),
            ),
            ("warning_level".into(), Tag::Int(1)),
            ("allow_op_features".into(), Tag::Byte(1)),
            (
                "front_text".into(),
                Tag::Compound(vec![
                    (
                        "messages".into(),
                        Tag::List(ListTag {
                            element_type: tag_type::STRING,
                            elements: vec![Tag::String("visible".into()); 4],
                        }),
                    ),
                    (
                        "filtered_messages".into(),
                        Tag::List(ListTag {
                            element_type: tag_type::STRING,
                            elements: vec![Tag::String("filtered".into()); 4],
                        }),
                    ),
                    ("color".into(), Tag::String("red".into())),
                    ("has_glowing_text".into(), Tag::Byte(1)),
                ]),
            ),
            (
                "back_text".into(),
                Tag::Compound(vec![
                    (
                        "messages".into(),
                        Tag::List(ListTag {
                            element_type: tag_type::STRING,
                            elements: vec![Tag::String("back".into()); 4],
                        }),
                    ),
                    ("color".into(), Tag::String("blue".into())),
                    ("has_glowing_text".into(), Tag::Byte(1)),
                ]),
            ),
            (
                "profile".into(),
                Tag::Compound(vec![("name".into(), Tag::String("Player".into()))]),
            ),
            ("custom_name".into(), Tag::String("head".into())),
            ("author".into(), Tag::String("author".into())),
            ("metadata".into(), Tag::String("metadata".into())),
            ("posX".into(), Tag::Int(1)),
            ("posY".into(), Tag::Int(2)),
            ("posZ".into(), Tag::Int(3)),
            ("sizeX".into(), Tag::Int(4)),
            ("sizeY".into(), Tag::Int(5)),
            ("sizeZ".into(), Tag::Int(6)),
            ("rotation".into(), Tag::String("NONE".into())),
            ("mirror".into(), Tag::String("NONE".into())),
            ("mode".into(), Tag::String("SAVE".into())),
            ("ignoreEntities".into(), Tag::Byte(1)),
            ("strict".into(), Tag::Byte(1)),
            ("showair".into(), Tag::Byte(1)),
            ("showboundingbox".into(), Tag::Byte(1)),
            ("integrity".into(), Tag::Float(0.5)),
            ("seed".into(), Tag::Long(77)),
            ("errors".into(), empty_list()),
            ("exit_portal".into(), Tag::IntArray(vec![1, 64, 2])),
            ("ExactTeleport".into(), Tag::Byte(1)),
            (
                "config".into(),
                Tag::Compound(vec![
                    (
                        "loot_table".into(),
                        Tag::String("minecraft:chests/simple_dungeon".into()),
                    ),
                    ("activation_range".into(), Tag::Double(3.0)),
                    ("deactivation_range".into(), Tag::Double(4.0)),
                    (
                        "key_item".into(),
                        Tag::Compound(vec![
                            ("id".into(), Tag::String("minecraft:trial_key".into())),
                            ("count".into(), Tag::Int(1)),
                        ]),
                    ),
                    (
                        "override_loot_table_to_display".into(),
                        Tag::String("minecraft:chests/simple_dungeon".into()),
                    ),
                ]),
            ),
            ("Delay".into(), Tag::Short(2)),
            ("MinSpawnDelay".into(), Tag::Short(3)),
            ("MaxSpawnDelay".into(), Tag::Short(4)),
            ("SpawnCount".into(), Tag::Short(5)),
            ("MaxNearbyEntities".into(), Tag::Short(6)),
            ("RequiredPlayerRange".into(), Tag::Short(7)),
            ("SpawnRange".into(), Tag::Short(8)),
            (
                "blockState".into(),
                Tag::Compound(vec![("Name".into(), Tag::String("minecraft:air".into()))]),
            ),
            ("progress".into(), Tag::Float(0.5)),
            ("extending".into(), Tag::Byte(1)),
            ("source".into(), Tag::Byte(1)),
            (
                "primary_effect".into(),
                Tag::String("minecraft:speed".into()),
            ),
            (
                "secondary_effect".into(),
                Tag::String("minecraft:regeneration".into()),
            ),
            ("next_mob_spawns_at".into(), Tag::Long(42)),
            ("SuccessCount".into(), Tag::Int(2)),
            ("TrackOutput".into(), Tag::Byte(1)),
            ("LastOutput".into(), Tag::String("private output".into())),
            ("UpdateLastExecution".into(), Tag::Byte(1)),
            ("LastExecution".into(), Tag::Long(100)),
            (
                "disabled_slots".into(),
                Tag::List(ListTag {
                    element_type: tag_type::INT,
                    elements: vec![Tag::Int(1)],
                }),
            ),
            ("triggered".into(), Tag::Byte(1)),
            ("cursors".into(), empty_list()),
            (
                "ominous_config".into(),
                Tag::String("minecraft:trial_chamber/melee/zombie/ominous".into()),
            ),
            (
                "registered_players".into(),
                Tag::List(ListTag {
                    element_type: tag_type::INT_ARRAY,
                    elements: vec![Tag::IntArray(vec![0, 0, 0, 1])],
                }),
            ),
            (
                "current_mobs".into(),
                Tag::List(ListTag {
                    element_type: tag_type::INT_ARRAY,
                    elements: vec![Tag::IntArray(vec![0, 0, 0, 2])],
                }),
            ),
            ("cooldown_ends_at".into(), Tag::Long(90)),
            ("total_mobs_spawned".into(), Tag::Int(2)),
            (
                "ejecting_loot_table".into(),
                Tag::String("minecraft:chests/test".into()),
            ),
            ("target_cooldown_length".into(), Tag::Int(100)),
            ("required_player_range".into(), Tag::Int(12)),
        ];
        let selected = fields;
        let mut expected = Vec::new();
        for (key, value) in &selected {
            if expected_keys.contains(&key.as_str()) {
                expected.push((
                    key.clone(),
                    if key == "shared_data" {
                        public_shared.clone()
                    } else {
                        value.clone()
                    },
                ));
            }
        }
        let mut saved = vec![("id".into(), Tag::String(kind.vanilla_id().into()))];
        saved.extend(selected);
        let mut bytes = Vec::new();
        mc_nbt::write_network(&mut bytes, &Tag::Compound(saved)).unwrap();
        let mut chunk = Chunk::empty(
            ChunkPos { x: 0, z: 0 },
            crate::block::BlockStateId(0),
            Identifier::parse("minecraft:plains").unwrap(),
        );
        let pos = BlockPos { x: 1, y: 64, z: 0 };
        chunk
            .set_block(1, 64, 0, crate::block::BlockStateId(1))
            .unwrap();
        chunk.block_entities.insert(pos, bytes);
        let expected = Tag::Compound(expected);
        let record = client_block_entity(&chunk, &registry, pos).expect("projected entity");
        assert_eq!(record.nbt, expected, "{}", kind.vanilla_id());
        assert_eq!(
            client_block_entities(&chunk, &registry)[0].nbt,
            expected,
            "{}",
            kind.vanilla_id()
        );
    }
}
