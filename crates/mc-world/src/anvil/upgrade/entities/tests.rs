use super::effects::fix_potion_effects;
use super::*;

fn entity(id: &str, mut values: Vec<(&str, Tag)>) -> Tag {
    values.insert(0, ("id", text(id)));
    compound(values)
}

fn tags(elements: Vec<Tag>) -> Tag {
    Tag::List(ListTag {
        element_type: elements.first().map_or(0, Tag::type_id),
        elements,
    })
}

fn apply(tag: &mut Tag, kind: NodeKind, version: i32) {
    fix_node(tag, kind, version, &mut UpgradeReport::default()).unwrap();
}

fn run_entity(tag: &mut Tag, version: i32) {
    apply(tag, NodeKind::Entity, version);
}

#[test]
fn cat_variants_3086_and_british_rename_3097() {
    let mut cat = entity("minecraft:cat", vec![("CatType", Tag::Int(4))]);
    run_entity(&mut cat, 3086);
    assert_eq!(get(&cat, "variant"), Some(&text("minecraft:british")));
    assert_eq!(get(&cat, "CatType"), Some(&Tag::Int(4)));
    run_entity(&mut cat, 3097);
    assert_eq!(
        get(&cat, "variant"),
        Some(&text("minecraft:british_shorthair"))
    );
    let mut unknown = entity("minecraft:cat", vec![("CatType", Tag::Int(100))]);
    run_entity(&mut unknown, 3086);
    assert_eq!(get(&unknown, "variant"), Some(&text("minecraft:tabby")));
}

#[test]
fn frog_variants_3087_reject_unmapped_numeric_ids() {
    let mut frog = entity("minecraft:frog", vec![("Variant", Tag::Int(2))]);
    run_entity(&mut frog, 3087);
    assert_eq!(get(&frog, "variant"), Some(&text("minecraft:cold")));
    let mut unknown = entity("minecraft:frog", vec![("Variant", Tag::Int(3))]);
    let before = unknown.clone();
    assert!(
        fix_node(
            &mut unknown,
            NodeKind::Entity,
            3087,
            &mut UpgradeReport::default()
        )
        .is_err()
    );
    assert_eq!(unknown, before);
}

#[test]
fn painting_fields_3090() {
    let mut painting = entity(
        "minecraft:painting",
        vec![
            ("Motive", text("minecraft:kebab")),
            ("Facing", Tag::Byte(3)),
        ],
    );
    run_entity(&mut painting, 3090);
    assert_eq!(get(&painting, "variant"), Some(&text("minecraft:kebab")));
    assert_eq!(get(&painting, "facing"), Some(&Tag::Byte(3)));
    assert!(get(&painting, "Motive").is_none());
}

#[test]
fn goat_horns_3093_are_unconditional() {
    let mut goat = entity("minecraft:goat", vec![("HasLeftHorn", Tag::Byte(0))]);
    run_entity(&mut goat, 3093);
    assert_eq!(get(&goat, "HasLeftHorn"), Some(&Tag::Byte(1)));
    assert_eq!(get(&goat, "HasRightHorn"), Some(&Tag::Byte(1)));
}

#[test]
fn effect_duration_3322_shared_entity_player_and_item() {
    let effect = compound(vec![
        ("Duration", Tag::Int(40)),
        (
            "FactorCalculationData",
            compound(vec![("effect_changed_timestamp", Tag::Int(95))]),
        ),
        (
            "HiddenEffect",
            compound(vec![(
                "FactorCalculationData",
                compound(vec![("effect_changed_timestamp", Tag::Int(99))]),
            )]),
        ),
    ]);
    for kind in [NodeKind::Entity, NodeKind::Player] {
        let mut living = compound(vec![("ActiveEffects", tags(vec![effect.clone()]))]);
        apply(&mut living, kind, 3322);
        let effect = &super::super::nbt::list(get(&living, "ActiveEffects").unwrap()).unwrap()[0];
        let factor = get(effect, "FactorCalculationData").unwrap();
        assert_eq!(get(factor, "ticks_active"), Some(&Tag::Int(55)));
        assert!(get(factor, "effect_changed_timestamp").is_none());
        assert!(
            get(
                get(
                    get(effect, "HiddenEffect").unwrap(),
                    "FactorCalculationData"
                )
                .unwrap(),
                "effect_changed_timestamp"
            )
            .is_some()
        );
    }
    let mut potion = entity(
        "minecraft:potion",
        vec![(
            "tag",
            compound(vec![("CustomPotionEffects", tags(vec![effect]))]),
        )],
    );
    fix_potion_effects(&mut potion, 3322, &mut UpgradeReport::default()).unwrap();
    let effects = get(get(&potion, "tag").unwrap(), "CustomPotionEffects").unwrap();
    let factor = get(
        &super::super::nbt::list(effects).unwrap()[0],
        "FactorCalculationData",
    )
    .unwrap();
    assert_eq!(get(factor, "ticks_active"), Some(&Tag::Int(55)));
}

#[test]
fn mob_effect_ids_3568_and_hidden_instances() {
    let mut living = compound(vec![(
        "ActiveEffects",
        tags(vec![compound(vec![
            ("Id", Tag::Byte(1)),
            ("Duration", Tag::Int(10)),
            ("ShowIcon", Tag::Byte(1)),
            (
                "HiddenEffect",
                compound(vec![("Id", Tag::Byte(2)), ("Amplifier", Tag::Byte(3))]),
            ),
        ])]),
    )]);
    apply(&mut living, NodeKind::Player, 3568);
    let effect = &super::super::nbt::list(get(&living, "active_effects").unwrap()).unwrap()[0];
    assert_eq!(get(effect, "id"), Some(&text("minecraft:speed")));
    assert_eq!(get(effect, "duration"), Some(&Tag::Int(10)));
    let hidden = get(effect, "hidden_effect").unwrap();
    assert_eq!(get(hidden, "id"), Some(&text("minecraft:slowness")));
    assert_eq!(get(hidden, "amplifier"), Some(&Tag::Byte(3)));
}

#[test]
fn effect_3568_mooshroom_stew_and_potion_item() {
    let mut mooshroom = entity(
        "minecraft:mooshroom",
        vec![
            ("EffectId", Tag::Byte(9)),
            ("EffectDuration", Tag::Int(100)),
        ],
    );
    run_entity(&mut mooshroom, 3568);
    let effect = &super::super::nbt::list(get(&mooshroom, "stew_effects").unwrap()).unwrap()[0];
    assert_eq!(get(effect, "id"), Some(&text("minecraft:nausea")));
    assert!(get(&mooshroom, "EffectId").is_none());
    let mut stew = entity(
        "minecraft:suspicious_stew",
        vec![(
            "tag",
            compound(vec![(
                "Effects",
                tags(vec![compound(vec![
                    ("EffectId", Tag::Byte(1)),
                    ("EffectDuration", Tag::Int(50)),
                ])]),
            )]),
        )],
    );
    fix_potion_effects(&mut stew, 3568, &mut UpgradeReport::default()).unwrap();
    let item_tag = get(&stew, "tag").unwrap();
    assert!(get(item_tag, "Effects").is_none());
    let effect = &super::super::nbt::list(get(item_tag, "effects").unwrap()).unwrap()[0];
    assert_eq!(get(effect, "duration"), Some(&Tag::Int(50)));
}

#[test]
fn effect_3568_unknown_ids_and_malformed_lists_fail_closed() {
    for effects in [
        tags(vec![compound(vec![("Id", Tag::Int(99))])]),
        Tag::Int(1),
    ] {
        let mut entity = compound(vec![("ActiveEffects", effects)]);
        assert!(
            fix_node(
                &mut entity,
                NodeKind::Entity,
                3568,
                &mut UpgradeReport::default()
            )
            .is_err()
        );
    }
}

#[test]
fn primed_tnt_3683_keeps_legacy_fuse() {
    let mut tnt = entity("minecraft:tnt", vec![("Fuse", Tag::Short(80))]);
    run_entity(&mut tnt, 3683);
    assert_eq!(get(&tnt, "fuse"), Some(&Tag::Short(80)));
    assert_eq!(get(&tnt, "Fuse"), Some(&Tag::Short(80)));
    assert_eq!(
        get(get(&tnt, "block_state").unwrap(), "Name"),
        Some(&text("minecraft:tnt"))
    );
}

#[test]
fn projectile_stored_items_3685() {
    for (id, potion, expected) in [
        ("minecraft:arrow", "minecraft:empty", "minecraft:arrow"),
        (
            "minecraft:arrow",
            "minecraft:water",
            "minecraft:tipped_arrow",
        ),
        (
            "minecraft:spectral_arrow",
            "minecraft:empty",
            "minecraft:spectral_arrow",
        ),
    ] {
        let mut arrow = entity(id, vec![("Potion", text(potion))]);
        run_entity(&mut arrow, 3685);
        assert_eq!(
            get(get(&arrow, "item").unwrap(), "id"),
            Some(&text(expected))
        );
        assert_eq!(
            get(get(&arrow, "item").unwrap(), "Count"),
            Some(&Tag::Int(1))
        );
    }
    let item = compound(vec![
        ("id", text("minecraft:trident")),
        ("Count", Tag::Byte(1)),
    ]);
    let mut trident = entity("minecraft:trident", vec![("Trident", item.clone())]);
    run_entity(&mut trident, 3685);
    assert_eq!(get(&trident, "item"), Some(&item));
    assert!(get(&trident, "Trident").is_none());
}

#[test]
fn horse_body_armor_3808_and_llama_decor() {
    let item = compound(vec![("id", text("minecraft:diamond_horse_armor"))]);
    let mut horse = entity(
        "minecraft:horse",
        vec![
            ("ArmorItem", item.clone()),
            (
                "ArmorItems",
                tags(vec![
                    compound(vec![]),
                    compound(vec![]),
                    item.clone(),
                    compound(vec![]),
                ]),
            ),
            (
                "ArmorDropChances",
                tags(vec![Tag::Float(0.0), Tag::Float(0.0), Tag::Float(2.0)]),
            ),
        ],
    );
    run_entity(&mut horse, 3808);
    assert_eq!(get(&horse, "body_armor_item"), Some(&item));
    assert_eq!(
        get(&horse, "body_armor_drop_chance"),
        Some(&Tag::Float(2.0))
    );
    assert_eq!(
        super::super::nbt::list(get(&horse, "ArmorItems").unwrap()).unwrap()[2],
        compound(vec![])
    );
    assert_eq!(
        super::super::nbt::list(get(&horse, "ArmorDropChances").unwrap()).unwrap()[2],
        Tag::Float(0.085)
    );
    for id in ["minecraft:llama", "minecraft:trader_llama"] {
        let mut llama = entity(id, vec![("DecorItem", item.clone())]);
        run_entity(&mut llama, 3808);
        assert_eq!(get(&llama, "body_armor_item"), Some(&item));
    }
}

#[test]
fn chested_inventory_3809_is_byte_narrowing() {
    let mut donkey = entity(
        "minecraft:donkey",
        vec![(
            "Items",
            tags(vec![
                compound(vec![("Slot", Tag::Byte(2))]),
                compound(vec![("Slot", Tag::Int(130))]),
                compound(vec![]),
            ]),
        )],
    );
    run_entity(&mut donkey, 3809);
    let items = super::super::nbt::list(get(&donkey, "Items").unwrap()).unwrap();
    assert_eq!(get(&items[0], "Slot"), Some(&Tag::Byte(0)));
    assert_eq!(get(&items[1], "Slot"), Some(&Tag::Byte(-128)));
    assert!(get(&items[2], "Slot").is_none());
}

#[test]
fn wolf_health_3812_only_doubles_twenty_base() {
    for (base, expected_base, expected_health) in [(20.0, 40.0, 24.0), (19.0, 19.0, 12.0)] {
        let mut wolf = entity(
            "minecraft:wolf",
            vec![
                ("Health", Tag::Float(12.0)),
                (
                    "Attributes",
                    tags(vec![compound(vec![
                        ("Name", text("generic.max_health")),
                        ("Base", Tag::Double(base)),
                    ])]),
                ),
            ],
        );
        run_entity(&mut wolf, 3812);
        assert_eq!(get(&wolf, "Health"), Some(&Tag::Float(expected_health)));
        let attribute = &super::super::nbt::list(get(&wolf, "Attributes").unwrap()).unwrap()[0];
        assert_eq!(get(attribute, "Base"), Some(&Tag::Double(expected_base)));
    }
}

#[test]
fn entity_block_positions_3813_and_uuid_leash() {
    let pos = compound(vec![
        ("X", Tag::Int(1)),
        ("Y", Tag::Int(-2)),
        ("Z", Tag::Int(3)),
    ]);
    for (id, old, new) in [
        ("minecraft:bee", "HivePos", "hive_pos"),
        ("minecraft:bee", "FlowerPos", "flower_pos"),
        ("minecraft:end_crystal", "BeamTarget", "beam_target"),
        (
            "minecraft:wandering_trader",
            "WanderTarget",
            "wander_target",
        ),
        ("minecraft:pillager", "PatrolTarget", "patrol_target"),
    ] {
        let mut entity = entity(id, vec![(old, pos.clone())]);
        run_entity(&mut entity, 3813);
        assert_eq!(get(&entity, new), Some(&Tag::IntArray(vec![1, -2, 3])));
        assert!(get(&entity, old).is_none());
    }
    let leash = compound(vec![("UUID", Tag::IntArray(vec![1, 2, 3, 4]))]);
    let mut cow = entity("minecraft:cow", vec![("Leash", leash.clone())]);
    run_entity(&mut cow, 3813);
    assert_eq!(get(&cow, "leash"), Some(&leash));
}

#[test]
fn legacy_jump_strength_3814_shared_player() {
    let mut player = compound(vec![(
        "Attributes",
        tags(vec![compound(vec![("Name", text("horse.jump_strength"))])]),
    )]);
    apply(&mut player, NodeKind::Player, 3814);
    let attribute = &super::super::nbt::list(get(&player, "Attributes").unwrap()).unwrap()[0];
    assert_eq!(
        get(attribute, "Name"),
        Some(&text("minecraft:generic.jump_strength"))
    );
}

#[test]
fn arrow_and_cloud_potion_transfer_3818() {
    let mut arrow = entity(
        "minecraft:arrow",
        vec![
            ("Potion", text("minecraft:water")),
            ("Color", Tag::Int(42)),
            ("custom_potion_effects", tags(vec![])),
            (
                "item",
                compound(vec![("id", text("minecraft:tipped_arrow"))]),
            ),
        ],
    );
    run_entity(&mut arrow, 3818);
    assert!(get(&arrow, "Potion").is_none());
    let item_tag = get(get(&arrow, "item").unwrap(), "tag").unwrap();
    assert_eq!(get(item_tag, "Potion"), Some(&text("minecraft:water")));
    assert_eq!(get(item_tag, "CustomPotionColor"), Some(&Tag::Int(42)));
    let mut cloud = entity(
        "minecraft:area_effect_cloud",
        vec![
            ("Color", Tag::Int(42)),
            ("Potion", text("minecraft:water")),
            ("effects", tags(vec![])),
        ],
    );
    run_entity(&mut cloud, 3818);
    let contents = get(&cloud, "potion_contents").unwrap();
    assert_eq!(get(contents, "custom_color"), Some(&Tag::Int(42)));
    assert_eq!(get(contents, "potion"), Some(&text("minecraft:water")));
    assert!(get(contents, "custom_effects").is_some());
}

#[test]
fn potion_transfer_3818_rejects_missing_item_and_unsupported_snbt() {
    for mut tag in [
        entity("minecraft:arrow", vec![("Potion", text("minecraft:water"))]),
        entity(
            "minecraft:area_effect_cloud",
            vec![("Particle", text("minecraft:item stone{Damage:1}"))],
        ),
    ] {
        let before = tag.clone();
        assert!(
            fix_node(
                &mut tag,
                NodeKind::Entity,
                3818,
                &mut UpgradeReport::default()
            )
            .is_err()
        );
        assert_eq!(tag, before);
    }
}

#[test]
fn stored_weapon_3938_is_schema_identity() {
    let mut arrow = entity(
        "minecraft:arrow",
        vec![("weapon", compound(vec![("id", text("minecraft:bow"))]))],
    );
    let before = arrow.clone();
    run_entity(&mut arrow, 3938);
    assert_eq!(arrow, before);
}

#[test]
fn attributes_3945_convert_entity_fields_and_operations() {
    let mut player = compound(vec![(
        "Attributes",
        tags(vec![compound(vec![
            ("Name", text("minecraft:generic.movement_speed")),
            ("Base", Tag::Double(0.1)),
            (
                "Modifiers",
                tags(vec![compound(vec![
                    ("UUID", Tag::IntArray(vec![0, 0, 0, 1])),
                    ("Name", text("test")),
                    ("Amount", Tag::Double(0.2)),
                    ("Operation", Tag::Int(2)),
                ])]),
            ),
        ])]),
    )]);
    apply(&mut player, NodeKind::Player, 3945);
    assert!(get(&player, "Attributes").is_none());
    let attribute = &super::super::nbt::list(get(&player, "attributes").unwrap()).unwrap()[0];
    assert_eq!(
        get(attribute, "id"),
        Some(&text("minecraft:generic.movement_speed"))
    );
    assert_eq!(get(attribute, "base"), Some(&Tag::Double(0.1)));
    let modifier = &super::super::nbt::list(get(attribute, "modifiers").unwrap()).unwrap()[0];
    assert_eq!(
        get(modifier, "operation"),
        Some(&text("add_multiplied_total"))
    );
    assert_eq!(get(modifier, "amount"), Some(&Tag::Double(0.2)));
    assert!(get(modifier, "id").is_some());
    assert!(get(modifier, "UUID").is_none());
}

#[test]
fn attribute_prefix_4055_preserves_modded_namespaces() {
    let mut entity = compound(vec![(
        "attributes",
        tags(vec![
            compound(vec![("id", text("minecraft:generic.max_health"))]),
            compound(vec![("id", text("horse.jump_strength"))]),
            compound(vec![("id", text("mod:generic.max_health"))]),
        ]),
    )]);
    run_entity(&mut entity, 4055);
    let attributes = super::super::nbt::list(get(&entity, "attributes").unwrap()).unwrap();
    assert_eq!(
        get(&attributes[0], "id"),
        Some(&text("minecraft:max_health"))
    );
    assert_eq!(
        get(&attributes[1], "id"),
        Some(&text("minecraft:jump_strength"))
    );
    assert_eq!(
        get(&attributes[2], "id"),
        Some(&text("mod:generic.max_health"))
    );
}

#[test]
fn boat_split_4067_variants_and_defaults() {
    for (id, wood, expected) in [
        ("minecraft:boat", "birch", "minecraft:birch_boat"),
        (
            "minecraft:chest_boat",
            "bamboo",
            "minecraft:bamboo_chest_raft",
        ),
        ("minecraft:boat", "bamboo", "minecraft:bamboo_raft"),
        ("minecraft:boat", "unknown", "minecraft:oak_boat"),
    ] {
        let mut boat = entity(id, vec![("Type", text(wood))]);
        run_entity(&mut boat, 4067);
        assert_eq!(get(&boat, "id"), Some(&text(expected)));
        assert!(get(&boat, "Type").is_none());
    }
}

#[test]
fn creaking_schema_4071_preserves_unmodified_entity_data() {
    for id in ["minecraft:creaking", "minecraft:creaking_transient"] {
        let mut creaking = entity(id, vec![("home_pos", Tag::IntArray(vec![1, 2, 3]))]);
        let before = creaking.clone();
        run_entity(&mut creaking, 4071);
        assert_eq!(creaking, before);
    }
}

#[test]
fn salmon_size_4081_medium_unless_large() {
    for (old, new) in [
        ("small", "medium"),
        ("large", "large"),
        ("unknown", "medium"),
    ] {
        let mut salmon = entity("minecraft:salmon", vec![("type", text(old))]);
        run_entity(&mut salmon, 4081);
        assert_eq!(get(&salmon, "type"), Some(&text(new)));
    }
}

#[test]
fn minecart_fuse_4173() {
    let mut cart = entity("minecraft:tnt_minecart", vec![("TNTFuse", Tag::Int(10))]);
    run_entity(&mut cart, 4173);
    assert_eq!(get(&cart, "fuse"), Some(&Tag::Int(10)));
    assert!(get(&cart, "TNTFuse").is_none());
}

#[test]
fn follow_ranges_4187_exact_defaults_only() {
    for (id, old, new) in [
        ("minecraft:villager", 48.0, 16.0),
        ("minecraft:bee", 48.0, 16.0),
        ("minecraft:allay", 48.0, 16.0),
        ("minecraft:llama", 40.0, 16.0),
        ("minecraft:piglin_brute", 16.0, 12.0),
        ("minecraft:warden", 16.0, 24.0),
    ] {
        let mut entity = entity(
            id,
            vec![(
                "attributes",
                tags(vec![compound(vec![
                    ("id", text("follow_range")),
                    ("base", Tag::Double(old)),
                ])]),
            )],
        );
        run_entity(&mut entity, 4187);
        let attribute = &super::super::nbt::list(get(&entity, "attributes").unwrap()).unwrap()[0];
        assert_eq!(get(attribute, "base"), Some(&Tag::Double(new)));
    }
    let mut bee = entity(
        "minecraft:bee",
        vec![(
            "attributes",
            tags(vec![compound(vec![
                ("id", text("follow_range")),
                ("base", Tag::Double(49.0)),
            ])]),
        )],
    );
    run_entity(&mut bee, 4187);
    let attribute = &super::super::nbt::list(get(&bee, "attributes").unwrap()).unwrap()[0];
    assert_eq!(get(attribute, "base"), Some(&Tag::Double(49.0)));
}

#[test]
fn drop_chances_4293_omits_defaults_and_preserves_slot_mapping() {
    let mut entity = compound(vec![
        (
            "ArmorDropChances",
            tags(vec![
                Tag::Float(0.085),
                Tag::Float(0.2),
                Tag::Float(0.3),
                Tag::Float(0.4),
            ]),
        ),
        (
            "HandDropChances",
            tags(vec![Tag::Float(0.5), Tag::Float(0.6)]),
        ),
        ("body_armor_drop_chance", Tag::Float(2.0)),
    ]);
    run_entity(&mut entity, 4293);
    let chances = get(&entity, "drop_chances").unwrap();
    assert!(get(chances, "feet").is_none());
    for (slot, chance) in [
        ("legs", 0.2),
        ("chest", 0.3),
        ("head", 0.4),
        ("mainhand", 0.5),
        ("offhand", 0.6),
        ("body", 2.0),
    ] {
        assert_eq!(get(chances, slot), Some(&Tag::Float(chance)));
    }
    assert!(get(&entity, "ArmorDropChances").is_none());
}

#[test]
fn cloud_duration_scale_4296_overwrites_previous_scale() {
    let mut cloud = entity(
        "minecraft:area_effect_cloud",
        vec![("potion_duration_scale", Tag::Float(1.0))],
    );
    run_entity(&mut cloud, 4296);
    assert_eq!(
        get(&cloud, "potion_duration_scale"),
        Some(&Tag::Float(0.25))
    );
}

#[test]
fn saddle_equipment_4300_flags_and_items() {
    let mut pig = entity("minecraft:pig", vec![("Saddle", Tag::Byte(1))]);
    run_entity(&mut pig, 4300);
    assert_eq!(
        get(get(&pig, "saddle").unwrap(), "id"),
        Some(&text("minecraft:saddle"))
    );
    assert_eq!(
        get(get(&pig, "drop_chances").unwrap(), "saddle"),
        Some(&Tag::Float(2.0))
    );
    assert!(get(&pig, "Saddle").is_none());
    let mut horse = entity(
        "minecraft:horse",
        vec![(
            "SaddleItem",
            compound(vec![("id", text("minecraft:saddle"))]),
        )],
    );
    run_entity(&mut horse, 4300);
    assert!(get(&horse, "SaddleItem").is_none());
    assert!(get(&horse, "saddle").is_some());
    let mut strider = entity("minecraft:strider", vec![("Saddle", Tag::Byte(0))]);
    run_entity(&mut strider, 4300);
    assert!(get(&strider, "Saddle").is_none());
    assert!(get(&strider, "saddle").is_none());
}

#[test]
fn equipment_4301_discards_only_missing_id_list_placeholders() {
    let boots = compound(vec![
        ("id", text("minecraft:iron_boots")),
        ("count", Tag::Int(1)),
    ]);
    let mut zombie = entity(
        "minecraft:zombie",
        vec![
            ("ArmorItems", tags(vec![boots.clone(), compound(vec![])])),
            (
                "HandItems",
                tags(vec![
                    compound(vec![]),
                    compound(vec![("id", text("minecraft:air"))]),
                ]),
            ),
            ("body_armor_item", compound(vec![])),
        ],
    );
    run_entity(&mut zombie, 4301);
    let equipment = get(&zombie, "equipment").unwrap();
    assert_eq!(get(equipment, "feet"), Some(&boots));
    assert!(get(equipment, "legs").is_none());
    assert!(get(equipment, "mainhand").is_none());
    assert_eq!(
        get(get(equipment, "offhand").unwrap(), "id"),
        Some(&text("minecraft:air"))
    );
    assert_eq!(get(equipment, "body"), Some(&compound(vec![])));
    assert!(get(&zombie, "ArmorItems").is_none());
}

#[test]
fn fall_distance_4303_float_rounding_shared_with_player() {
    for kind in [NodeKind::Entity, NodeKind::Player] {
        let mut tag = compound(vec![("FallDistance", Tag::Double(1.00000001))]);
        apply(&mut tag, kind, 4303);
        assert_eq!(get(&tag, "fall_distance"), Some(&Tag::Double(1.0)));
        assert!(get(&tag, "FallDistance").is_none());
    }
}

#[test]
fn thrown_potion_split_4306() {
    for (item, expected) in [
        ("minecraft:lingering_potion", "minecraft:lingering_potion"),
        ("minecraft:potion", "minecraft:splash_potion"),
    ] {
        let mut potion = entity(
            "minecraft:potion",
            vec![("Item", compound(vec![("id", text(item))]))],
        );
        run_entity(&mut potion, 4306);
        assert_eq!(get(&potion, "id"), Some(&text(expected)));
    }
    let mut missing = entity("minecraft:potion", vec![]);
    run_entity(&mut missing, 4306);
    assert_eq!(get(&missing, "id"), Some(&text("minecraft:splash_potion")));
}

#[test]
fn player_equipment_4312_extracts_signed_offhand_and_armor() {
    let mut player = compound(vec![(
        "Inventory",
        tags(vec![
            compound(vec![
                ("Slot", Tag::Byte(0)),
                ("id", text("minecraft:stone")),
            ]),
            compound(vec![
                ("Slot", Tag::Byte(100)),
                ("id", text("minecraft:iron_boots")),
            ]),
            compound(vec![
                ("Slot", Tag::Byte(-106)),
                ("id", text("minecraft:shield")),
            ]),
        ]),
    )]);
    apply(&mut player, NodeKind::Player, 4312);
    assert_eq!(
        super::super::nbt::list(get(&player, "Inventory").unwrap())
            .unwrap()
            .len(),
        1
    );
    let equipment = get(&player, "equipment").unwrap();
    assert_eq!(
        get(get(equipment, "feet").unwrap(), "id"),
        Some(&text("minecraft:iron_boots"))
    );
    assert!(get(get(equipment, "feet").unwrap(), "Slot").is_none());
    assert_eq!(
        get(get(equipment, "offhand").unwrap(), "id"),
        Some(&text("minecraft:shield"))
    );
}

#[test]
fn inline_positions_4314_for_living_attached_and_special_entities() {
    for (id, keys, destination) in [
        (
            "minecraft:cow",
            ["SleepingX", "SleepingY", "SleepingZ"],
            "sleeping_pos",
        ),
        ("minecraft:vex", ["BoundX", "BoundY", "BoundZ"], "bound_pos"),
        ("minecraft:phantom", ["AX", "AY", "AZ"], "anchor_pos"),
        (
            "minecraft:turtle",
            ["HomePosX", "HomePosY", "HomePosZ"],
            "home_pos",
        ),
        (
            "minecraft:painting",
            ["TileX", "TileY", "TileZ"],
            "block_pos",
        ),
    ] {
        let mut tag = entity(
            id,
            vec![
                (keys[0], Tag::Int(1)),
                (keys[1], Tag::Int(2)),
                (keys[2], Tag::Int(3)),
            ],
        );
        run_entity(&mut tag, 4314);
        assert_eq!(get(&tag, destination), Some(&Tag::IntArray(vec![1, 2, 3])));
        assert!(get(&tag, keys[0]).is_none());
    }
    let mut partial = entity(
        "minecraft:vex",
        vec![("BoundX", Tag::Int(1)), ("LifeTicks", Tag::Int(10))],
    );
    run_entity(&mut partial, 4314);
    assert_eq!(get(&partial, "BoundX"), Some(&Tag::Int(1)));
    assert_eq!(get(&partial, "life_ticks"), Some(&Tag::Int(10)));
}

#[test]
fn player_inline_positions_4314_build_respawn_and_entered_nether() {
    let mut player = compound(vec![
        ("SpawnX", Tag::Int(1)),
        ("SpawnY", Tag::Int(2)),
        ("SpawnZ", Tag::Int(3)),
        ("SpawnAngle", Tag::Float(45.0)),
        ("SpawnDimension", text("minecraft:the_nether")),
        ("SpawnForced", Tag::Byte(1)),
        (
            "enteredNetherPosition",
            compound(vec![("x", Tag::Double(1.5)), ("z", Tag::Int(-4))]),
        ),
    ]);
    apply(&mut player, NodeKind::Player, 4314);
    let respawn = get(&player, "respawn").unwrap();
    assert_eq!(get(respawn, "pos"), Some(&Tag::IntArray(vec![1, 2, 3])));
    assert_eq!(get(respawn, "angle"), Some(&Tag::Float(45.0)));
    assert_eq!(get(respawn, "forced"), Some(&Tag::Byte(1)));
    assert_eq!(
        get(&player, "entered_nether_pos"),
        Some(&tags(vec![
            Tag::Double(1.5),
            Tag::Double(0.0),
            Tag::Double(-4.0)
        ]))
    );
    assert!(get(&player, "SpawnX").is_none());
}

#[test]
fn unconverted_particle_4420_and_icon_default_4424_fail_closed() {
    for (mut tag, kind, version) in [
        (
            entity(
                "minecraft:area_effect_cloud",
                vec![("Particle", text("minecraft:smoke"))],
            ),
            NodeKind::Entity,
            4420,
        ),
        (
            compound(vec![("locator_bar_icon", compound(vec![]))]),
            NodeKind::Player,
            4424,
        ),
        (
            entity(
                "minecraft:cow",
                vec![("locator_bar_icon", compound(vec![]))],
            ),
            NodeKind::Entity,
            4424,
        ),
    ] {
        let before = tag.clone();
        assert!(fix_node(&mut tag, kind, version, &mut UpgradeReport::default()).is_err());
        assert_eq!(tag, before);
    }
    let mut absent = compound(vec![]);
    apply(&mut absent, NodeKind::Player, 4424);
    assert_eq!(absent, compound(vec![]));
}

#[test]
fn copper_golem_weather_4535() {
    for (value, expected) in [
        (0, "unaffected"),
        (1, "exposed"),
        (2, "weathered"),
        (3, "oxidized"),
        (99, "unaffected"),
    ] {
        let mut golem = entity(
            "minecraft:copper_golem",
            vec![("weather_state", Tag::Int(value))],
        );
        run_entity(&mut golem, 4535);
        assert_eq!(get(&golem, "weather_state"), Some(&text(expected)));
    }
}

#[test]
fn villagers_4763_are_finalized_unconditionally() {
    for id in ["minecraft:villager", "minecraft:zombie_villager"] {
        let mut villager = entity(id, vec![("VillagerDataFinalized", Tag::Byte(0))]);
        run_entity(&mut villager, 4763);
        assert_eq!(get(&villager, "VillagerDataFinalized"), Some(&Tag::Byte(1)));
    }
}

#[test]
fn opaque_custom_data_and_unassigned_kinds_are_preserved() {
    let opaque = compound(vec![
        ("ActiveEffects", Tag::Int(9)),
        ("FallDistance", Tag::Float(2.0)),
        ("Motive", text("keep")),
    ]);
    let mut entity = entity("mod:unknown", vec![("custom_data", opaque.clone())]);
    for version in [
        3086, 3090, 3322, 3568, 3813, 3814, 3945, 4055, 4303, 4314, 4424, 4763,
    ] {
        run_entity(&mut entity, version);
    }
    assert_eq!(get(&entity, "custom_data"), Some(&opaque));
    let mut item = opaque.clone();
    apply(&mut item, NodeKind::Item, 4303);
    assert_eq!(item, opaque);
}

#[test]
fn malformed_typed_lists_fail_with_exact_step_receipts() {
    for (id, key, version, expected_fix) in [
        (
            "minecraft:donkey",
            "Items",
            3809,
            "ChestedHorsesInventoryZeroIndexingFix",
        ),
        ("minecraft:wolf", "Attributes", 3812, "FixWolfHealth"),
        (
            "minecraft:cow",
            "Attributes",
            3814,
            "AttributesRenameLegacy",
        ),
        (
            "minecraft:cow",
            "Attributes",
            3945,
            "AttributeModifierIdFix",
        ),
        ("minecraft:cow", "attributes", 4055, "AttributeIdPrefixFix"),
        (
            "minecraft:bee",
            "attributes",
            4187,
            "EntityAttributeBaseValueFix",
        ),
        (
            "minecraft:cow",
            "ArmorDropChances",
            4293,
            "DropChancesFormatFix",
        ),
        ("minecraft:cow", "ArmorItems", 4301, "EquipmentFormatFix"),
    ] {
        let mut value = entity(id, vec![(key, Tag::Int(9))]);
        let error = fix_node(
            &mut value,
            NodeKind::Entity,
            version,
            &mut UpgradeReport::default(),
        )
        .unwrap_err();
        assert!(
            matches!(error, UpgradeError::UnsupportedField { version: actual_version, fix, .. }
            if actual_version == version && fix == expected_fix)
        );
    }
    let mut player = compound(vec![("Inventory", Tag::Int(9))]);
    assert!(matches!(
        fix_node(
            &mut player,
            NodeKind::Player,
            4312,
            &mut UpgradeReport::default()
        )
        .unwrap_err(),
        UpgradeError::UnsupportedField {
            version: 4312,
            fix: "PlayerEquipmentFix",
            ..
        }
    ));
    let mut horse = entity(
        "minecraft:horse",
        vec![("ArmorItem", compound(vec![])), ("ArmorItems", Tag::Int(9))],
    );
    assert!(matches!(
        fix_node(
            &mut horse,
            NodeKind::Entity,
            3808,
            &mut UpgradeReport::default()
        )
        .unwrap_err(),
        UpgradeError::UnsupportedField {
            version: 3808,
            fix: "HorseBodyArmorItemFix",
            ..
        }
    ));
}

#[test]
fn malformed_effect_factor_3322_and_hidden_instance_3568_fail_closed() {
    let mut living = compound(vec![(
        "ActiveEffects",
        tags(vec![compound(vec![("FactorCalculationData", Tag::Int(9))])]),
    )]);
    assert!(matches!(
        fix_node(
            &mut living,
            NodeKind::Player,
            3322,
            &mut UpgradeReport::default()
        )
        .unwrap_err(),
        UpgradeError::UnsupportedField {
            version: 3322,
            fix: "EffectDurationFix",
            ..
        }
    ));
    let mut living = compound(vec![(
        "ActiveEffects",
        tags(vec![compound(vec![
            ("Id", Tag::Int(1)),
            ("HiddenEffect", Tag::Int(9)),
        ])]),
    )]);
    assert!(matches!(
        fix_node(
            &mut living,
            NodeKind::Entity,
            3568,
            &mut UpgradeReport::default()
        )
        .unwrap_err(),
        UpgradeError::UnsupportedField {
            version: 3568,
            fix: "MobEffectIdFix",
            ..
        }
    ));
}

#[test]
fn particle_and_default_rejections_include_explicit_fix_receipts() {
    for (mut value, version, expected_fix) in [
        (
            entity(
                "minecraft:area_effect_cloud",
                vec![("Particle", text("minecraft:item stone{Damage:1}"))],
            ),
            3818,
            "ParticleUnflatteningFix",
        ),
        (
            entity(
                "minecraft:area_effect_cloud",
                vec![("Particle", text("minecraft:smoke"))],
            ),
            4420,
            "NamedEntityConvertUncheckedFix",
        ),
        (
            entity(
                "minecraft:cow",
                vec![("locator_bar_icon", compound(vec![]))],
            ),
            4424,
            "AddFieldFix",
        ),
    ] {
        let before = value.clone();
        let error = fix_node(
            &mut value,
            NodeKind::Entity,
            version,
            &mut UpgradeReport::default(),
        )
        .unwrap_err();
        assert!(
            matches!(error, UpgradeError::UnsupportedField { version: actual_version, fix, .. }
            if actual_version == version && fix == expected_fix)
        );
        assert_eq!(value, before);
    }
}

#[test]
fn applied_receipts_are_deduplicated() {
    let mut goat = entity("minecraft:goat", vec![]);
    let mut report = UpgradeReport::default();
    fix_node(&mut goat, NodeKind::Entity, 3093, &mut report).unwrap();
    fix_node(&mut goat, NodeKind::Entity, 3093, &mut report).unwrap();
    assert_eq!(report.steps.len(), 1);
    assert_eq!(report.steps[0].version, 3093);
    assert_eq!(report.steps[0].fix, "EntityGoatMissingStateFix");
}

#[test]
fn horse_armor_3808_rejects_heterogeneous_output_before_mutating() {
    let mut horse = entity(
        "minecraft:horse",
        vec![
            (
                "ArmorItem",
                compound(vec![("id", text("minecraft:iron_horse_armor"))]),
            ),
            (
                "ArmorDropChances",
                tags(vec![Tag::Double(0.0), Tag::Double(0.0), Tag::Double(2.0)]),
            ),
        ],
    );
    let before = horse.clone();
    let error = fix_node(
        &mut horse,
        NodeKind::Entity,
        3808,
        &mut UpgradeReport::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        UpgradeError::UnsupportedField {
            version: 3808,
            fix: "HorseBodyArmorItemFix",
            ..
        }
    ));
    assert_eq!(horse, before);
}

#[test]
fn cloud_particle_3818_then_schema_cast_4420_preserves_simple_particle() {
    let mut cloud = entity(
        "minecraft:area_effect_cloud",
        vec![
            ("Particle", text("entity_effect")),
            ("Duration", Tag::Int(100)),
        ],
    );
    let mut report = UpgradeReport::default();
    fix_node(&mut cloud, NodeKind::Entity, 3818, &mut report).unwrap();
    let particle = get(&cloud, "Particle").unwrap().clone();
    assert_eq!(
        get(&particle, "type"),
        Some(&text("minecraft:entity_effect"))
    );
    fix_node(&mut cloud, NodeKind::Entity, 4420, &mut report).unwrap();
    assert!(get(&cloud, "Particle").is_none());
    assert_eq!(get(&cloud, "custom_particle"), Some(&particle));
    assert_eq!(get(&cloud, "Duration"), Some(&Tag::Int(100)));
    assert!(
        report
            .steps
            .iter()
            .any(|step| step.version == 3818 && step.fix == "ParticleUnflatteningFix")
    );
    assert!(
        report
            .steps
            .iter()
            .any(|step| step.version == 4420 && step.fix == "NamedEntityConvertUncheckedFix")
    );
}

#[test]
fn source_3808_skips_horse_revision_zero_but_runs_llama_revisions() {
    let item = compound(vec![("id", text("minecraft:iron_horse_armor"))]);
    let mut report = UpgradeReport {
        from: 3808,
        ..UpgradeReport::default()
    };
    let mut horse = entity("minecraft:horse", vec![("ArmorItem", item.clone())]);
    fix_node(&mut horse, NodeKind::Entity, 3808, &mut report).unwrap();
    assert_eq!(get(&horse, "ArmorItem"), Some(&item));
    assert!(get(&horse, "body_armor_item").is_none());
    for id in ["minecraft:llama", "minecraft:trader_llama"] {
        let mut llama = entity(id, vec![("DecorItem", item.clone())]);
        fix_node(&mut llama, NodeKind::Entity, 3808, &mut report).unwrap();
        assert_eq!(get(&llama, "body_armor_item"), Some(&item));
        assert!(get(&llama, "DecorItem").is_none());
    }
}
