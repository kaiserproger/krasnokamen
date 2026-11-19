use super::uuids::{is_animal, is_horse, is_mob, is_tameable};
use super::*;

fn apply_at(tag: &mut Tag, kind: NodeKind, version: i32) -> bool {
    let mut report = UpgradeReport::default();
    fix_node(tag, kind, version, &mut report).expect("old_entities never fails");
    after_children(tag, kind, version, &mut report).expect("legacy post-child fix");
    !report.steps.is_empty()
}

fn entity(id: &str, fields: Vec<(&str, Tag)>) -> Tag {
    let mut all = vec![("id", text(id))];
    all.extend(fields);
    nbt::compound(all)
}

#[test]
fn legacy_villager_loot_pickup_is_unconditionally_enabled() {
    for previous in [None, Some(Tag::Byte(0)), Some(Tag::Byte(1))] {
        let mut villager = entity("Villager", vec![]);
        if let Some(previous) = previous {
            nbt::set(&mut villager, "CanPickUpLoot", previous);
        }
        apply_at(&mut villager, NodeKind::Entity, 101);
        assert_eq!(nbt::get(&villager, "CanPickUpLoot"), Some(&Tag::Byte(1)));
    }
    let mut cow = entity("Cow", vec![("CanPickUpLoot", Tag::Byte(0))]);
    apply_at(&mut cow, NodeKind::Entity, 101);
    assert_eq!(nbt::get(&cow, "CanPickUpLoot"), Some(&Tag::Byte(0)));
}

#[test]
fn potion_schema_revision_runs_after_owner_fix_but_skips_revision_zero_source() {
    for from in [2510, 2511] {
        let mut potion = entity(
            "minecraft:potion",
            vec![
                (
                    "Potion",
                    nbt::compound(vec![("id", text("minecraft:splash_potion"))]),
                ),
                (
                    "owner",
                    nbt::compound(vec![("M", Tag::Long(21)), ("L", Tag::Long(22))]),
                ),
            ],
        );
        let mut report = UpgradeReport {
            from,
            ..UpgradeReport::default()
        };
        fix_node(&mut potion, NodeKind::Entity, 2511, &mut report).unwrap();
        assert!(nbt::get(&potion, "Potion").is_none());
        assert!(nbt::get(&potion, "Item").is_some());
        assert_eq!(nbt::get(&potion, "OwnerUUID").is_some(), from == 2510);
        assert_eq!(nbt::get(&potion, "owner").is_some(), from == 2511);
    }
}

#[test]
fn source_808_receives_the_later_shulker_revision() {
    let mut shulker = entity("minecraft:shulker", vec![]);
    let mut report = UpgradeReport {
        from: 808,
        ..UpgradeReport::default()
    };
    fix_node(&mut shulker, NodeKind::Entity, 808, &mut report).unwrap();
    assert_eq!(nbt::get(&shulker, "Color"), Some(&Tag::Byte(10)));
}

#[test]
fn equipment_splits_and_drop_chances() {
    let mut tag = entity(
        "minecraft:zombie",
        vec![
            (
                "Equipment",
                Tag::List(mc_nbt::ListTag {
                    element_type: mc_nbt::tag_type::COMPOUND,
                    elements: vec![
                        entity("minecraft:stone_sword", vec![("Count", Tag::Byte(1))]),
                        entity("minecraft:iron_helmet", vec![("Count", Tag::Byte(1))]),
                    ],
                }),
            ),
            (
                "DropChances",
                Tag::List(mc_nbt::ListTag {
                    element_type: mc_nbt::tag_type::FLOAT,
                    elements: vec![Tag::Float(0.5), Tag::Float(0.0)],
                }),
            ),
        ],
    );
    assert!(apply_at(&mut tag, NodeKind::Entity, 100));
    assert!(nbt::get(&tag, "Equipment").is_none());
    assert!(nbt::get(&tag, "DropChances").is_none());
    let hand = nbt::list(nbt::get(&tag, "HandItems").unwrap()).unwrap();
    assert_eq!(hand.len(), 2);
    let chances = nbt::list(nbt::get(&tag, "HandDropChances").unwrap()).unwrap();
    assert_eq!(chances.len(), 2);
}

#[test]
fn minecart_type_and_riding_and_string_uuid() {
    let mut cart = entity("Minecart", vec![("Type", Tag::Int(1))]);
    assert!(apply_at(&mut cart, NodeKind::Entity, 107));
    assert_eq!(nbt::get(&cart, "id"), Some(&text("MinecartChest")));
    assert!(nbt::get(&cart, "Type").is_none());

    let mut rider = entity(
        "minecraft:pig",
        vec![("Riding", entity("minecraft:cow", vec![]))],
    );
    assert!(apply_at(&mut rider, NodeKind::Entity, 135));
    assert!(nbt::get(&rider, "Riding").is_none());
    assert_eq!(
        nbt::list(nbt::get(&rider, "Passengers").unwrap())
            .unwrap()
            .len(),
        1
    );

    let mut with_uuid = entity(
        "minecraft:pig",
        vec![("UUID", text("12345678-1234-1234-1234-123456789abc"))],
    );
    assert!(apply_at(&mut with_uuid, NodeKind::Entity, 108));
    assert!(nbt::get(&with_uuid, "UUIDMost").is_some());
    assert!(nbt::get(&with_uuid, "UUIDLeast").is_some());
    assert!(nbt::get(&with_uuid, "UUID").is_none());
}

#[test]
fn health_healf_and_horse_saddle_and_redundant_chances() {
    let mut zombie = entity("Zombie", vec![("HealF", Tag::Float(10.0))]);
    assert!(apply_at(&mut zombie, NodeKind::Entity, 109));
    assert_eq!(nbt::get(&zombie, "Health"), Some(&Tag::Float(10.0)));
    assert!(nbt::get(&zombie, "HealF").is_none());

    let mut horse = entity("EntityHorse", vec![("Saddle", Tag::Byte(1))]);
    assert!(apply_at(&mut horse, NodeKind::Entity, 110));
    assert!(nbt::get(&horse, "SaddleItem").is_some());
    assert!(nbt::get(&horse, "Saddle").is_none());

    let mut mob = entity(
        "minecraft:zombie",
        vec![
            (
                "HandDropChances",
                Tag::List(mc_nbt::ListTag {
                    element_type: mc_nbt::tag_type::FLOAT,
                    elements: vec![Tag::Float(0.0), Tag::Float(0.0)],
                }),
            ),
            (
                "ArmorDropChances",
                Tag::List(mc_nbt::ListTag {
                    element_type: mc_nbt::tag_type::FLOAT,
                    elements: vec![
                        Tag::Float(0.0),
                        Tag::Float(0.0),
                        Tag::Float(0.0),
                        Tag::Float(0.0),
                    ],
                }),
            ),
        ],
    );
    assert!(apply_at(&mut mob, NodeKind::Entity, 113));
    assert!(nbt::get(&mob, "HandDropChances").is_none());
    assert!(nbt::get(&mob, "ArmorDropChances").is_none());
}

#[test]
fn splits_and_id_maps() {
    let mut guardian = entity("Guardian", vec![("Elder", Tag::Byte(1))]);
    assert!(apply_at(&mut guardian, NodeKind::Entity, 700));
    assert_eq!(nbt::get(&guardian, "id"), Some(&text("ElderGuardian")));

    let mut skeleton = entity("Skeleton", vec![("SkeletonType", Tag::Int(1))]);
    assert!(apply_at(&mut skeleton, NodeKind::Entity, 701));
    assert_eq!(nbt::get(&skeleton, "id"), Some(&text("WitherSkeleton")));

    let mut zombie = entity("Zombie", vec![("ZombieType", Tag::Int(6))]);
    assert!(apply_at(&mut zombie, NodeKind::Entity, 702));
    assert_eq!(nbt::get(&zombie, "id"), Some(&text("Husk")));

    let mut zv = entity("Zombie", vec![("ZombieType", Tag::Int(2))]);
    assert!(apply_at(&mut zv, NodeKind::Entity, 702));
    assert_eq!(nbt::get(&zv, "id"), Some(&text("ZombieVillager")));
    assert_eq!(nbt::get(&zv, "Profession"), Some(&Tag::Int(1)));

    let mut horse = entity("EntityHorse", vec![("Type", Tag::Int(1))]);
    assert!(apply_at(&mut horse, NodeKind::Entity, 703));
    assert_eq!(nbt::get(&horse, "id"), Some(&text("Donkey")));

    // 502 invalid-profession fallback: deterministic UUIDMost.rem_euclid(6).
    let mut bad_prof = entity(
        "Zombie",
        vec![
            ("IsVillager", Tag::Byte(1)),
            ("VillagerProfession", Tag::Int(9)),
            ("UUIDMost", Tag::Long(14)),
        ],
    );
    assert!(apply_at(&mut bad_prof, NodeKind::Entity, 502));
    assert_eq!(nbt::get(&bad_prof, "ZombieType"), Some(&Tag::Int(2)));
    assert!(nbt::get(&bad_prof, "IsVillager").is_none());
    // Same input twice -> same output (no RNG).
    let mut bad_prof2 = entity(
        "Zombie",
        vec![
            ("IsVillager", Tag::Byte(1)),
            ("VillagerProfession", Tag::Int(9)),
            ("UUIDMost", Tag::Long(14)),
        ],
    );
    assert!(apply_at(&mut bad_prof2, NodeKind::Entity, 502));
    assert_eq!(nbt::get(&bad_prof2, "ZombieType"), Some(&Tag::Int(2)));
    // No UUID -> 0; outcome always in 0..6.
    let mut no_uuid = entity(
        "Zombie",
        vec![
            ("IsVillager", Tag::Byte(1)),
            ("VillagerProfession", Tag::Int(-3)),
        ],
    );
    assert!(apply_at(&mut no_uuid, NodeKind::Entity, 502));
    assert_eq!(nbt::get(&no_uuid, "ZombieType"), Some(&Tag::Int(0)));

    let mut orb = entity("XPOrb", vec![]);
    assert!(apply_at(&mut orb, NodeKind::Entity, 705));
    assert_eq!(nbt::get(&orb, "id"), Some(&text("minecraft:xp_orb")));

    let mut be = entity("Chest", vec![]);
    assert!(apply_at(&mut be, NodeKind::BlockEntity, 704));
    assert_eq!(nbt::get(&be, "id"), Some(&text("minecraft:chest")));
}

#[test]
fn spawner_identifiers_and_shulker_and_trades() {
    let mut spawner = entity("MobSpawner", vec![("EntityId", text("Creeper"))]);
    assert!(apply_at(&mut spawner, NodeKind::BlockEntity, 106));
    assert_eq!(nbt::get(&spawner, "id"), Some(&text("MobSpawner")));
    let data = nbt::get(&spawner, "SpawnData").unwrap();
    assert_eq!(nbt::get(data, "id"), Some(&text("Creeper")));

    // MinecartSpawner entity: nested UNTAGGED_SPAWNER fields transform,
    // outer entity id is untouched (pre-705 legacy id kept for EntityIdFix).
    let mut cart = entity("MinecartSpawner", vec![("EntityId", text("Creeper"))]);
    assert!(apply_at(&mut cart, NodeKind::Entity, 106));
    assert_eq!(nbt::get(&cart, "id"), Some(&text("MinecartSpawner")));
    assert!(nbt::get(&cart, "EntityId").is_none());

    // Untouched spawner-minecart (no EntityId/Potentials): no-op, id kept.
    let mut bare = entity("minecraft:spawner_minecart", vec![]);
    assert!(!apply_at(&mut bare, NodeKind::Entity, 106));
    assert_eq!(
        nbt::get(&bare, "id"),
        Some(&text("minecraft:spawner_minecart"))
    );

    let mut shulker = entity("minecraft:shulker", vec![]);
    assert!(apply_at(&mut shulker, NodeKind::Entity, 808));
    assert_eq!(nbt::get(&shulker, "Color"), Some(&Tag::Byte(10)));

    let mut box_be = entity("minecraft:shulker_box", vec![("Color", Tag::Byte(3))]);
    assert!(apply_at(&mut box_be, NodeKind::BlockEntity, 813));
    assert!(nbt::get(&box_be, "Color").is_none());

    let mut villager = entity(
        "minecraft:villager",
        vec![(
            "Offers",
            nbt::compound(vec![(
                "Recipes",
                Tag::List(mc_nbt::ListTag {
                    element_type: mc_nbt::tag_type::COMPOUND,
                    elements: vec![nbt::compound(vec![(
                        "sell",
                        nbt::compound(vec![("id", text("minecraft:carved_pumpkin"))]),
                    )])],
                }),
            )]),
        )],
    );
    assert!(apply_at(&mut villager, NodeKind::Entity, 1451));
    let recipes =
        nbt::list(nbt::get(nbt::get(&villager, "Offers").unwrap(), "Recipes").unwrap()).unwrap();
    assert_eq!(
        nbt::get(nbt::get(&recipes[0], "sell").unwrap(), "id"),
        Some(&text("minecraft:pumpkin"))
    );
}

#[test]
fn directions_motives_names_and_wolf() {
    let mut frame = entity("minecraft:item_frame", vec![("Facing", Tag::Byte(0))]);
    assert!(apply_at(&mut frame, NodeKind::Entity, 1456));
    assert_eq!(nbt::get(&frame, "Facing"), Some(&Tag::Byte(3)));

    let mut painting = entity("minecraft:painting", vec![("Motive", text("donkeykong"))]);
    assert!(apply_at(&mut painting, NodeKind::Entity, 1460));
    assert_eq!(
        nbt::get(&painting, "Motive"),
        Some(&text("minecraft:donkey_kong"))
    );

    let mut named = entity("minecraft:pig", vec![("CustomName", text("Bob"))]);
    assert!(apply_at(&mut named, NodeKind::Entity, 1458));
    assert_eq!(
        nbt::get(&named, "CustomName"),
        Some(&Tag::String("{\"text\":\"Bob\"}".into()))
    );

    let mut wolf = entity("minecraft:wolf", vec![("CollarColor", Tag::Int(3))]);
    assert!(apply_at(&mut wolf, NodeKind::Entity, 1451));
    assert_eq!(nbt::get(&wolf, "CollarColor"), Some(&Tag::Byte(12)));
}

#[test]
fn cat_villager_profession_and_rebuilds() {
    let mut ocelot = entity(
        "minecraft:ocelot",
        vec![("CatType", Tag::Int(2)), ("Owner", text("Steve"))],
    );
    assert!(apply_at(&mut ocelot, NodeKind::Entity, 1904));
    assert_eq!(nbt::get(&ocelot, "id"), Some(&text("minecraft:cat")));

    let mut cat = entity("minecraft:cat", vec![("CatType", Tag::Int(9))]);
    assert!(apply_at(&mut cat, NodeKind::Entity, 1917));
    assert_eq!(nbt::get(&cat, "CatType"), Some(&Tag::Int(10)));

    let mut villager = entity(
        "minecraft:villager",
        vec![("Profession", Tag::Int(0)), ("Career", Tag::Int(2))],
    );
    assert!(apply_at(&mut villager, NodeKind::Entity, 1918));
    let data = nbt::get(&villager, "VillagerData").unwrap();
    assert_eq!(
        nbt::get(data, "profession"),
        Some(&text("minecraft:fisherman"))
    );

    let mut ravager = entity("minecraft:illager_beast", vec![]);
    assert!(apply_at(&mut ravager, NodeKind::Entity, 1928));
    assert_eq!(nbt::get(&ravager, "id"), Some(&text("minecraft:ravager")));

    let mut gossip_villager = entity(
        "minecraft:villager",
        vec![(
            "Gossips",
            Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::COMPOUND,
                elements: vec![
                    nbt::compound(vec![("Type", text("golem"))]),
                    nbt::compound(vec![("Type", text("major_positive"))]),
                ],
            }),
        )],
    );
    assert!(apply_at(&mut gossip_villager, NodeKind::Entity, 1963));
    assert_eq!(
        nbt::list(nbt::get(&gossip_villager, "Gossips").unwrap())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn uuid_families_and_projectile_and_gossip() {
    let mut horse = entity(
        "minecraft:horse",
        vec![("UUIDMost", Tag::Long(1)), ("UUIDLeast", Tag::Long(2))],
    );
    assert!(apply_at(&mut horse, NodeKind::Entity, 2514));
    assert!(matches!(nbt::get(&horse, "UUID"), Some(Tag::IntArray(_))));
    assert!(nbt::get(&horse, "UUIDMost").is_none());

    let mut arrow = entity(
        "minecraft:arrow",
        vec![
            ("OwnerUUIDMost", Tag::Long(3)),
            ("OwnerUUIDLeast", Tag::Long(4)),
        ],
    );
    assert!(apply_at(&mut arrow, NodeKind::Entity, 2511));
    assert!(matches!(
        nbt::get(&arrow, "OwnerUUID"),
        Some(Tag::IntArray(_))
    ));

    let mut gossip = entity(
        "minecraft:villager",
        vec![(
            "Gossips",
            Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::COMPOUND,
                elements: vec![nbt::compound(vec![
                    ("TargetMost", Tag::Long(5)),
                    ("TargetLeast", Tag::Long(6)),
                ])],
            }),
        )],
    );
    assert!(apply_at(&mut gossip, NodeKind::Entity, 2516));
    let entries = nbt::list(nbt::get(&gossip, "Gossips").unwrap()).unwrap();
    assert!(matches!(
        nbt::get(&entries[0], "Target"),
        Some(Tag::IntArray(_))
    ));

    let mut conduit = entity(
        "minecraft:conduit",
        vec![(
            "target_uuid",
            nbt::compound(vec![("M", Tag::Long(7)), ("L", Tag::Long(8))]),
        )],
    );
    assert!(apply_at(&mut conduit, NodeKind::BlockEntity, 2514));
    assert!(matches!(
        nbt::get(&conduit, "Target"),
        Some(Tag::IntArray(_))
    ));

    let mut jigsaw = entity(
        "minecraft:jigsaw",
        vec![
            ("attachement_type", text("minecraft:empty")),
            ("target_pool", text("minecraft:empty")),
        ],
    );
    assert!(apply_at(&mut jigsaw, NodeKind::BlockEntity, 2518));
    assert_eq!(nbt::get(&jigsaw, "pool"), Some(&text("minecraft:empty")));

    let mut shulker = entity(
        "minecraft:shulker",
        vec![(
            "Rotation",
            Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::DOUBLE,
                elements: vec![Tag::Double(200.0), Tag::Double(0.0)],
            }),
        )],
    );
    assert!(apply_at(&mut shulker, NodeKind::Entity, 2535));
    assert_eq!(
        nbt::list(nbt::get(&shulker, "Rotation").unwrap()).unwrap()[0],
        Tag::Double(20.0)
    );
}

#[test]
fn attributes_memory_follow_range_and_misc_renames() {
    let mut mob = entity(
        "minecraft:zombie",
        vec![(
            "Attributes",
            Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::COMPOUND,
                elements: vec![nbt::compound(vec![("Name", text("generic.maxHealth"))])],
            }),
        )],
    );
    assert!(apply_at(&mut mob, NodeKind::Entity, 2523));
    let attrs = nbt::list(nbt::get(&mob, "Attributes").unwrap()).unwrap();
    assert_eq!(
        nbt::get(&attrs[0], "Name"),
        Some(&text("minecraft:generic.max_health"))
    );

    let mut villager = entity(
        "minecraft:villager",
        vec![(
            "Brain",
            nbt::compound(vec![(
                "memories",
                nbt::compound(vec![("minecraft:meeting_point", Tag::Int(3))]),
            )]),
        )],
    );
    assert!(apply_at(&mut villager, NodeKind::Entity, 2505));
    let memories = nbt::get(nbt::get(&villager, "Brain").unwrap(), "memories").unwrap();
    assert!(
        nbt::get(
            nbt::get(memories, "minecraft:meeting_point").unwrap(),
            "value"
        )
        .is_some()
    );

    let mut follower = entity(
        "minecraft:villager",
        vec![(
            "Attributes",
            Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::COMPOUND,
                elements: vec![nbt::compound(vec![
                    ("Name", text("generic.follow_range")),
                    ("Base", Tag::Double(16.0)),
                ])],
            }),
        )],
    );
    assert!(apply_at(&mut follower, NodeKind::Entity, 2533));
    let attrs = nbt::list(nbt::get(&follower, "Attributes").unwrap()).unwrap();
    assert_eq!(nbt::get(&attrs[0], "Base"), Some(&Tag::Double(48.0)));

    let mut strider = entity("minecraft:strider", vec![("NoGravity", Tag::Byte(1))]);
    assert!(apply_at(&mut strider, NodeKind::Entity, 2529));
    assert_eq!(nbt::get(&strider, "NoGravity"), Some(&Tag::Byte(0)));

    let mut pigman = entity("minecraft:zombie_pigman", vec![]);
    assert!(apply_at(&mut pigman, NodeKind::Entity, 2509));
    assert_eq!(
        nbt::get(&pigman, "id"),
        Some(&text("minecraft:zombified_piglin"))
    );

    let mut puffer = entity("minecraft:puffer_fish", vec![]);
    assert!(apply_at(&mut puffer, NodeKind::Entity, 1483));
    assert_eq!(nbt::get(&puffer, "id"), Some(&text("minecraft:pufferfish")));

    let mut shulker = entity("minecraft:shulker", vec![("Color", Tag::Byte(10))]);
    assert!(apply_at(&mut shulker, NodeKind::Entity, 1474));
    assert_eq!(nbt::get(&shulker, "Color"), Some(&Tag::Byte(16)));
}

#[test]
fn block_state_upgrade_and_numeric_states() {
    // Numeric piston state resolves through the exact getTag table.
    let mut piston = entity(
        "minecraft:piston",
        vec![("blockId", Tag::Int(1)), ("blockData", Tag::Int(0))],
    );
    assert!(apply_at(&mut piston, NodeKind::BlockEntity, 1451));
    assert!(nbt::get(&piston, "blockId").is_none());
    assert!(nbt::get(&piston, "blockState").is_some());
    // Falling-block numeric TileID resolves; Data removed.
    let mut falling = entity(
        "minecraft:falling_block",
        vec![("TileID", Tag::Int(1)), ("Data", Tag::Byte(0))],
    );
    assert!(apply_at(&mut falling, NodeKind::Entity, 1451));
    assert!(nbt::get(&falling, "TileID").is_none());
    assert!(nbt::get(&falling, "Data").is_none());
    assert!(nbt::get(&falling, "BlockState").is_some());
}

#[test]
fn remove_block_entity_tag_falling_block_branch() {
    // 1451r5 ENTITY branch only: falling_block TileEntityData gated on the
    // TAG's nested id.
    let mut noteblock = entity(
        "minecraft:falling_block",
        vec![(
            "TileEntityData",
            nbt::compound(vec![("id", text("minecraft:noteblock"))]),
        )],
    );
    assert!(apply_at(&mut noteblock, NodeKind::Entity, 1451));
    assert!(nbt::get(&noteblock, "TileEntityData").is_none());

    let mut pot = entity(
        "minecraft:falling_block",
        vec![(
            "TileEntityData",
            nbt::compound(vec![("id", text("minecraft:flower_pot"))]),
        )],
    );
    assert!(apply_at(&mut pot, NodeKind::Entity, 1451));
    assert!(nbt::get(&pot, "TileEntityData").is_none());

    // Non-drop id keeps its TileEntityData.
    let mut chest = entity(
        "minecraft:falling_block",
        vec![(
            "TileEntityData",
            nbt::compound(vec![("id", text("minecraft:chest"))]),
        )],
    );
    let _ = apply_at(&mut chest, NodeKind::Entity, 1451);
    assert!(nbt::get(&chest, "TileEntityData").is_some());

    // Wrong holder: non-falling entity never drops.
    let mut zombie = entity(
        "minecraft:zombie",
        vec![(
            "TileEntityData",
            nbt::compound(vec![("id", text("minecraft:noteblock"))]),
        )],
    );
    let _ = apply_at(&mut zombie, NodeKind::Entity, 1451);
    assert!(nbt::get(&zombie, "TileEntityData").is_some());
}

#[test]
fn furnace_recipe_fold() {
    // Synthetic: RecipesUsedSize 2, one positive + one zero amount.
    let mut furnace = entity(
        "minecraft:furnace",
        vec![
            ("RecipesUsedSize", Tag::Int(2)),
            ("RecipeLocation0", text("minecraft:bread")),
            ("RecipeAmount0", Tag::Int(3)),
            ("RecipeLocation1", text("minecraft:iron_ingot")),
            ("RecipeAmount1", Tag::Int(0)),
        ],
    );
    assert!(apply_at(&mut furnace, NodeKind::BlockEntity, 2501));
    assert!(nbt::get(&furnace, "RecipesUsedSize").is_none());
    assert!(nbt::get(&furnace, "RecipeLocation0").is_none());
    assert!(nbt::get(&furnace, "RecipeAmount1").is_none());
    let used = nbt::get(&furnace, "RecipesUsed").unwrap();
    assert_eq!(nbt::get(used, "minecraft:bread"), Some(&Tag::Int(3)));
    assert!(nbt::get(used, "minecraft:iron_ingot").is_none());

    // Missing size -> still stamps empty RecipesUsed (vanilla unconditional).
    let mut bare = entity("minecraft:smoker", vec![]);
    assert!(apply_at(&mut bare, NodeKind::BlockEntity, 2501));
    assert!(nbt::fields(nbt::get(&bare, "RecipesUsed").unwrap()).is_some());

    // Wrong holder: entity never rewrites.
    let mut mob = entity("minecraft:furnace", vec![("RecipesUsedSize", Tag::Int(1))]);
    let _ = apply_at(&mut mob, NodeKind::Entity, 2501);
    assert!(nbt::get(&mob, "RecipesUsed").is_none());
}

#[test]
fn splash_potion_item_field_rename() {
    // 2511r1: Potion -> Item on potion entities only.
    let mut potion = entity("minecraft:potion", vec![("Potion", nbt::compound(vec![]))]);
    assert!(apply_at(&mut potion, NodeKind::Entity, 2511));
    assert!(nbt::get(&potion, "Potion").is_none());
    assert!(nbt::get(&potion, "Item").is_some());

    // No Potion key -> r1 no-op (r0 may still apply via owner).
    let mut bare = entity("minecraft:potion", vec![]);
    let _ = apply_at(&mut bare, NodeKind::Entity, 2511);
    assert!(nbt::get(&bare, "Item").is_none());

    // Wrong holder: arrow keeps its Potion key.
    let mut arrow = entity("minecraft:arrow", vec![("Potion", nbt::compound(vec![]))]);
    let _ = apply_at(&mut arrow, NodeKind::Entity, 2511);
    assert!(nbt::get(&arrow, "Potion").is_some());
}

#[test]
fn riding_three_deep_preserves_each_mount_edge() {
    let mut chain = entity(
        "Pig",
        vec![(
            "Riding",
            entity("Cow", vec![("Riding", entity("Boat", vec![]))]),
        )],
    );
    assert!(apply_at(&mut chain, NodeKind::Entity, 135));
    assert_eq!(nbt::get(&chain, "id"), Some(&text("Boat")));
    let cow = &nbt::list(nbt::get(&chain, "Passengers").unwrap()).unwrap()[0];
    assert_eq!(nbt::get(cow, "id"), Some(&text("Cow")));
    let pig = &nbt::list(nbt::get(cow, "Passengers").unwrap()).unwrap()[0];
    assert_eq!(nbt::get(pig, "id"), Some(&text("Pig")));
    assert!(nbt::get(pig, "Riding").is_none());
}

#[test]
fn numeric_entity_states_keep_metadata_and_use_vanilla_defaults() {
    for fields in [
        vec![("TileID", Tag::Int(17)), ("Data", Tag::Byte(3))],
        vec![("Tile", Tag::Byte(17)), ("Data", Tag::Byte(3))],
        vec![
            ("Block", text("minecraft:log")),
            ("TileID", Tag::Int(1)),
            ("Data", Tag::Byte(3)),
        ],
    ] {
        let mut falling = entity("minecraft:falling_block", fields);
        apply_at(&mut falling, NodeKind::Entity, 1451);
        assert_eq!(
            nbt::get(&falling, "BlockState"),
            Some(&super::super::old_block_states::state(17 << 4 | 3))
        );
        for key in ["Block", "TileID", "Tile", "Data"] {
            assert!(nbt::get(&falling, key).is_none());
        }
    }
    let mut piston = entity("minecraft:piston", vec![]);
    apply_at(&mut piston, NodeKind::BlockEntity, 1451);
    assert_eq!(
        nbt::get(&piston, "blockState"),
        Some(&super::super::old_block_states::state(0))
    );
    for name in [
        "egg",
        "ender_pearl",
        "fireball",
        "potion",
        "small_fireball",
        "snowball",
        "wither_skull",
        "xp_bottle",
    ] {
        let mut projectile = entity(
            &format!("minecraft:{name}"),
            vec![("inTile", Tag::Int(1)), ("inData", Tag::Byte(3))],
        );
        apply_at(&mut projectile, NodeKind::Entity, 1451);
        assert!(nbt::get(&projectile, "inTile").is_none());
        assert_eq!(nbt::get(&projectile, "inData"), Some(&Tag::Byte(3)));
    }
}

#[test]
fn uuid_families_inherit_living_rewrites_without_inventing_animal_owners() {
    let attributes = || {
        Tag::List(mc_nbt::ListTag {
            element_type: mc_nbt::tag_type::COMPOUND,
            elements: vec![nbt::compound(vec![(
                "Modifiers",
                Tag::List(mc_nbt::ListTag {
                    element_type: mc_nbt::tag_type::COMPOUND,
                    elements: vec![nbt::compound(vec![
                        ("UUIDMost", Tag::Long(0x1234_5678_1234_5679)),
                        ("UUIDLeast", Tag::Long(0x2345_6789_2345_678b)),
                    ])],
                }),
            )])],
        })
    };
    for id in [
        "minecraft:horse",
        "minecraft:wolf",
        "minecraft:cow",
        "minecraft:zombie",
        "minecraft:armor_stand",
        "mod:opaque",
    ] {
        let mut tag = entity(
            id,
            vec![
                ("Attributes", attributes()),
                ("OwnerUUID", text("1-2-3-4-5")),
                ("LoveCauseMost", Tag::Long(1)),
                ("LoveCauseLeast", Tag::Long(2)),
                (
                    "Leash",
                    nbt::compound(vec![
                        ("UUIDMost", Tag::Long(3)),
                        ("UUIDLeast", Tag::Long(4)),
                    ]),
                ),
            ],
        );
        apply_at(&mut tag, NodeKind::Entity, 2514);
        let attribute = &nbt::list(nbt::get(&tag, "Attributes").unwrap()).unwrap()[0];
        let modifier = &nbt::list(nbt::get(attribute, "Modifiers").unwrap()).unwrap()[0];
        assert_eq!(
            nbt::get(modifier, "UUID").is_some(),
            id != "mod:opaque",
            "{id}"
        );
        assert_eq!(
            nbt::get(&tag, "Owner").is_some(),
            is_horse(id) || is_tameable(id),
            "{id}"
        );
        assert_eq!(nbt::get(&tag, "LoveCause").is_some(), is_animal(id), "{id}");
        let leash = nbt::get(&tag, "Leash").unwrap();
        assert_eq!(nbt::get(leash, "UUID").is_some(), is_mob(id), "{id}");
    }
}

#[test]
fn uuid_compounds_keep_all_long_bits_and_projectile_moves_overwrite_owner() {
    let most = 0x1234_5678_1234_5679_i64;
    let least = 0x2345_6789_2345_678b_i64;
    let expected = Tag::IntArray(vec![0x1234_5678, 0x1234_5679, 0x2345_6789, 0x2345_678b]);
    let mut item = entity(
        "minecraft:item",
        vec![(
            "Owner",
            nbt::compound(vec![("M", Tag::Long(most)), ("L", Tag::Long(least))]),
        )],
    );
    apply_at(&mut item, NodeKind::Entity, 2514);
    assert_eq!(nbt::get(&item, "Owner"), Some(&expected));
    for value in [expected, text("opaque-owner")] {
        let mut arrow = entity(
            "minecraft:arrow",
            vec![("OwnerUUID", value.clone()), ("Owner", text("old-owner"))],
        );
        apply_at(&mut arrow, NodeKind::Entity, 2514);
        assert!(nbt::get(&arrow, "OwnerUUID").is_none());
        assert_eq!(nbt::get(&arrow, "Owner"), Some(&value));
    }
}

#[test]
fn established_villagers_receive_xp_without_rebuilding_level() {
    for (level, xp) in [(2, 10), (3, 50), (4, 100), (5, 150)] {
        for previous in [None, Some(text("invalid"))] {
            let mut tag = entity(
                "minecraft:villager",
                vec![(
                    "VillagerData",
                    nbt::compound(vec![("level", Tag::Int(level))]),
                )],
            );
            if let Some(previous) = previous {
                nbt::set(&mut tag, "Xp", previous);
            }
            apply_at(&mut tag, NodeKind::Entity, 1955);
            assert_eq!(nbt::get(&tag, "Xp"), Some(&Tag::Int(xp)));
            assert_eq!(
                nbt::get(nbt::get(&tag, "VillagerData").unwrap(), "level"),
                Some(&Tag::Int(level))
            );
        }
    }
}

#[test]
fn shulker_rotation_retypes_every_element() {
    let mut tag = entity(
        "minecraft:shulker",
        vec![(
            "Rotation",
            Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::FLOAT,
                elements: vec![Tag::Float(200.0), Tag::Float(45.0)],
            }),
        )],
    );
    apply_at(&mut tag, NodeKind::Entity, 2535);
    assert_eq!(
        nbt::get(&tag, "Rotation"),
        Some(&Tag::List(mc_nbt::ListTag {
            element_type: mc_nbt::tag_type::DOUBLE,
            elements: vec![Tag::Double(20.0), Tag::Double(45.0)],
        }))
    );
}

#[test]
fn jukebox_uses_full_item_tables_and_preserves_original_record() {
    for (record, expected) in [
        (1, Some("minecraft:stone")),
        (2256, Some("minecraft:music_disc_13")),
        (276, None),
    ] {
        let original_item = nbt::compound(vec![("id", text("minecraft:dirt"))]);
        let mut tag = entity(
            "minecraft:jukebox",
            vec![
                ("Record", Tag::Int(record)),
                ("RecordItem", original_item.clone()),
            ],
        );
        apply_at(&mut tag, NodeKind::BlockEntity, 1451);
        assert_eq!(nbt::get(&tag, "Record"), Some(&Tag::Int(record)));
        if let Some(expected) = expected {
            let stack = nbt::get(&tag, "RecordItem").unwrap();
            assert_eq!(nbt::get(stack, "id"), Some(&text(expected)));
            assert_eq!(nbt::get(stack, "Count"), Some(&Tag::Byte(1)));
        } else {
            assert_eq!(nbt::get(&tag, "RecordItem"), Some(&original_item));
        }
    }
}

#[test]
fn item_frame_direction_and_dir_take_distinct_vanilla_branches() {
    let mut directional = entity(
        "ItemFrame",
        vec![
            ("Direction", Tag::Byte(3)),
            ("Dir", Tag::Byte(1)),
            ("ItemRotation", Tag::Byte(3)),
            ("TileX", Tag::Int(10)),
        ],
    );
    apply_at(&mut directional, NodeKind::Entity, 111);
    assert_eq!(nbt::get(&directional, "Facing"), Some(&Tag::Byte(3)));
    assert_eq!(nbt::get(&directional, "TileX"), Some(&Tag::Int(11)));
    assert_eq!(nbt::get(&directional, "ItemRotation"), Some(&Tag::Byte(6)));
    assert_eq!(nbt::get(&directional, "Dir"), Some(&Tag::Byte(1)));
    assert!(nbt::get(&directional, "Direction").is_none());

    let mut legacy = entity(
        "ItemFrame",
        vec![
            ("Dir", Tag::Byte(1)),
            ("ItemRotation", Tag::Byte(3)),
            ("TileX", Tag::Int(10)),
        ],
    );
    apply_at(&mut legacy, NodeKind::Entity, 111);
    assert_eq!(nbt::get(&legacy, "Facing"), Some(&Tag::Byte(1)));
    assert_eq!(nbt::get(&legacy, "TileX"), Some(&Tag::Int(10)));
    assert_eq!(nbt::get(&legacy, "ItemRotation"), Some(&Tag::Byte(3)));
    assert!(nbt::get(&legacy, "Dir").is_none());
}
