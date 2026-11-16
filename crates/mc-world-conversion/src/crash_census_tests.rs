use super::*;
fn json_tag(tag: &Tag) -> serde_json::Value {
    match tag {
        Tag::Byte(v) => serde_json::json!(v),
        Tag::Short(v) => serde_json::json!(v),
        Tag::Int(v) => serde_json::json!(v),
        Tag::Long(v) => serde_json::json!(v),
        Tag::Float(v) => serde_json::json!(v),
        Tag::Double(v) => serde_json::json!(v),
        Tag::ByteArray(v) => serde_json::json!(v),
        Tag::String(v) => serde_json::json!(v),
        Tag::List(v) => v.elements.iter().map(json_tag).collect(),
        Tag::Compound(v) => v
            .iter()
            .map(|(key, value)| (key.clone(), json_tag(value)))
            .collect(),
        Tag::IntArray(v) => serde_json::json!(v),
        Tag::LongArray(v) => serde_json::json!(v),
    }
}

fn entity() -> mc_core::journal::ReplayEntity {
    mc_core::journal::ReplayEntity {
        uuid: "00000000-0000-0000-0000-000000000001".into(),
        type_id: 1,
        pos: [0.0; 3],
        vel: [0.0; 3],
        yaw: 0.0,
        pitch: 0.0,
        ground: true,
        health: 1.0,
        alive: true,
        age: 5,
        breed_cooldown: 0,
        pickup_delay: 0,
        item: None,
        appearance: Default::default(),
        active_effects: Default::default(),
        villager: None,
        state: Default::default(),
        projectile: None,
        tick: 10,
    }
}

#[test]
fn replay_custody_contains_non_soldier_item_and_equipment() {
    let items = mc_data::items::krasnokamen_required_items();
    let id = items
        .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
        .unwrap();
    let stack = mc_data::ItemStack::new(id, 1);
    let mut entity = entity();
    entity.item = Some(stack.clone());
    entity.appearance.equipment = vec![Some(stack)];
    let fields = entity_fields(&entity, "minecraft:item", &items).unwrap();
    let custody = holders(&fields);
    assert!(custody["Item"].contains("minecraft:diamond"));
    assert!(custody["equipment"].contains("minecraft:diamond"));
    let data = json_tag(&Tag::Compound(fields));
    assert_eq!(data["Item"]["id"], "minecraft:diamond");
    assert_eq!(data["equipment"]["mainhand"]["id"], "minecraft:diamond");
}

#[test]
fn projectile_stack_keys_are_custody() {
    for (key, id) in [
        ("item", "minecraft:trident"),
        ("FireworksItem", "minecraft:firework_rocket"),
    ] {
        let fields = vec![(
            key.into(),
            Tag::Compound(vec![
                ("id".into(), Tag::String(id.into())),
                ("count".into(), Tag::Int(1)),
            ]),
        )];
        let custody = holders(&fields);
        assert_eq!(
            custody.get(key),
            Some(&format!("{{count: 1, id: \"{id}\"}}")),
            "projectile holder {key} must survive the filter"
        );
    }
}

#[test]
fn typed_custody_replaces_stale_opaque_fields_and_keeps_other_holders() {
    let items = mc_data::items::krasnokamen_required_items();
    let mut entity = entity();
    mc_nbt::write_named(
        &mut entity.state.opaque_entity_fields,
        "",
        &Tag::Compound(vec![
            ("Item".into(), Tag::String("stale".into())),
            ("equipment".into(), Tag::String("stale".into())),
            (
                "Items".into(),
                Tag::List(mc_nbt::ListTag {
                    element_type: mc_nbt::tag_type::COMPOUND,
                    elements: vec![Tag::Compound(vec![
                        ("id".into(), Tag::String("minecraft:shield".into())),
                        ("Slot".into(), Tag::Byte(4)),
                    ])],
                }),
            ),
        ]),
    )
    .unwrap();
    let fields = entity_fields(&entity, "minecraft:chest_minecart", &items).unwrap();
    let data = json_tag(&Tag::Compound(fields));
    assert!(data.get("Item").is_none());
    assert_eq!(data["equipment"], serde_json::json!({}));
    assert_eq!(data["Items"][0]["Slot"], 4);
    assert_eq!(data["Items"][0]["id"], "minecraft:shield");
}

#[test]
fn replay_mount_cargo_preserves_slots_and_empty_postimage() {
    let items = mc_data::items::krasnokamen_required_items();
    let id = items
        .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
        .unwrap();
    let mut entity = entity();
    entity.state.entity_life.mount.chested = true;
    entity.state.entity_life.mount.cargo = vec![
        mc_data::ItemStack::new(id, 0),
        mc_data::ItemStack::new(id, 3),
    ];
    let data = json_tag(&Tag::Compound(
        entity_fields(&entity, "minecraft:donkey", &items).unwrap(),
    ));
    assert_eq!(data["Items"][0]["Slot"], 1);
    assert_eq!(data["Items"][0]["count"], 3);
    entity.state.entity_life.mount.cargo.clear();
    let data = json_tag(&Tag::Compound(
        entity_fields(&entity, "minecraft:donkey", &items).unwrap(),
    ));
    assert_eq!(data["Items"], serde_json::json!([]));
}

fn entry(entity: mc_core::journal::ReplayEntity) -> mc_core::journal::JournalEntry {
    mc_core::journal::JournalEntry::Entity {
        id: 1,
        uuid: entity.uuid,
        type_id: entity.type_id,
        health: entity.health,
        alive: entity.alive,
        age: entity.age,
        breed_cooldown: entity.breed_cooldown,
        pickup_delay: entity.pickup_delay,
        item: entity.item,
        appearance: entity.appearance,
        active_effects: entity.active_effects,
        villager: entity.villager,
        state: std::sync::Arc::new(entity.state),
        projectile: entity.projectile,
        tick: entity.tick,
        pos: entity.pos,
        vel: entity.vel,
        yaw: entity.yaw,
        pitch: entity.pitch,
    }
}

#[test]
fn crash_census_folds_ordered_wal_and_refuses_a_gap_or_corrupt_frame() {
    use mc_core::journal::JournalWriter;

    let temporary = tempfile::tempdir().unwrap();
    let config = crate::test_support::config(temporary.path().join("world"), 42);
    drop(crate::test_support::fixture_store(&config).unwrap());
    let registry = crate::test_support::registry().unwrap();
    let id = registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
        .unwrap();
    let mut saved = entity();
    saved.type_id = registry.item_type_id;
    saved.item = Some(mc_data::ItemStack::new(id, 3));
    saved.tick = 1;
    saved.age = 0;
    let uuid = saved.uuid.clone();
    let mut writer = JournalWriter::open(&config.world_dir, 1).unwrap();
    writer.append(1, 1, &[entry(saved.clone())]).unwrap();
    saved.tick = 2;
    saved.item = Some(mc_data::ItemStack::new(id, 2));
    writer.append(2, 2, &[entry(saved)]).unwrap();
    drop(writer);
    let census = crash_census(&config).unwrap();
    assert_eq!((census.checkpoint_tick, census.durable_tick), (0, 2));
    let item = &census.entities["minecraft:overworld"][&uuid];
    assert_eq!(item.wire_id, Some(1));
    assert!(item.holders["Item"].contains("minecraft:diamond"));
    assert!(item.holders["Item"].contains("count: 2"));

    let first = config
        .world_dir
        .join("journal/batch-00000000000000000001-00000000000000000001.bin");
    let held = first.with_extension("held");
    std::fs::rename(&first, &held).unwrap();
    assert!(
        crash_census(&config).is_err(),
        "a published gap must refuse"
    );
    std::fs::rename(&held, &first).unwrap();
    let mut bytes = std::fs::read(&first).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&first, bytes).unwrap();
    assert!(
        crash_census(&config).is_err(),
        "corrupt published payload must refuse"
    );
}

#[test]
fn census_keys_dimension_and_rejects_duplicate_uuid_across_dimensions() {
    use mc_core::dimensions::Dimension;
    let make_entity = || CrashEntity {
        kind: "minecraft:item".into(),
        wire_id: Some(7),
        holders: BTreeMap::new(),
    };
    let mut overworld = BTreeMap::new();
    let mut seen = BTreeSet::new();
    insert_entity(
        &mut overworld,
        &mut seen,
        Dimension::Overworld,
        "u".into(),
        make_entity(),
    )
    .unwrap();
    assert!(
        insert_entity(&mut overworld, &mut seen, Dimension::Nether, "u".into(), make_entity())
            .is_err()
    );
    let mut nether = BTreeMap::new();
    insert_entity(
        &mut nether,
        &mut BTreeSet::new(),
        Dimension::Nether,
        "u".into(),
        make_entity(),
    )
    .unwrap();
    assert_ne!(
        serde_json::to_value(overworld).unwrap(),
        serde_json::to_value(nether).unwrap()
    );
}
