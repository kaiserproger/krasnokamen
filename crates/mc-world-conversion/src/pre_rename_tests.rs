use super::{Owner, PreRename};
use mc_nbt::{ListTag, Tag, tag_type};

fn compound(fields: Vec<(&str, Tag)>) -> Tag {
    Tag::Compound(
        fields
            .into_iter()
            .map(|(name, value)| (name.into(), value))
            .collect(),
    )
}

fn keys(tag: &Tag) -> Vec<&str> {
    let Tag::Compound(fields) = tag else {
        panic!("compound expected: {tag:?}")
    };
    fields.iter().map(|(name, _)| name.as_str()).collect()
}

fn foreign_payload() -> Tag {
    compound(vec![
        ("my_mod:solaris_flag", Tag::Byte(1)),
        ("my_mod:SolarisFlag", Tag::String("kept".into())),
        ("MY_MOD:SOLARIS_FLAG", Tag::Int(2)),
        ("solaris:unknown_extension", Tag::Int(3)),
        // Even an exact owned key is foreign inside an opaque compound.
        ("solaris:WireId", Tag::Int(9)),
        ("solaris:Rider", Tag::Int(4)),
        ("SolarisUnlockedRecipes", Tag::String("opaque".into())),
        (
            "Entities",
            Tag::List(ListTag {
                element_type: tag_type::COMPOUND,
                elements: vec![compound(vec![("solaris:WireId", Tag::Int(10))])],
            }),
        ),
    ])
}

#[test]
fn foreign_nested_keys_survive_pre_rename_in_every_record_owner() {
    for owner in [
        Owner::Player,
        Owner::Entity,
        Owner::Chunk,
        Owner::EntityRegion,
        Owner::Level,
        Owner::Raids,
        Owner::DragonFight,
    ] {
        let original = compound(vec![
            ("my_mod:solaris_flag", Tag::Byte(1)),
            ("my_mod:SolarisFlag", Tag::Int(2)),
            ("my_mod:SOLARIS_FLAG", Tag::Int(3)),
            ("solaris:unknown_extension", Tag::Int(4)),
            ("my_mod:payload", foreign_payload()),
            (
                "foreign",
                Tag::List(ListTag {
                    element_type: tag_type::COMPOUND,
                    elements: vec![foreign_payload()],
                }),
            ),
        ]);
        let mut tag = original.clone();
        let mut report = PreRename::default();
        report
            .rename(&mut tag, owner, &|| "foreign record".into())
            .unwrap();
        assert_eq!(tag, original);
        assert_eq!(report.keys, 0);
    }
}

#[test]
fn nested_owned_entity_block_and_metadata_keys_are_renamed_only_at_their_owner() {
    let mut chunk = compound(vec![
        ("SolarisSimDebt", Tag::Long(1)),
        (
            "Entities",
            Tag::List(ListTag {
                element_type: tag_type::COMPOUND,
                elements: vec![compound(vec![
                    ("solaris:WireId", Tag::Int(7)),
                    ("my_mod:payload", foreign_payload()),
                    (
                        "solaris:VillagerRuntime",
                        compound(vec![("SolarisSoldier", Tag::String("kept state".into()))]),
                    ),
                ])],
            }),
        ),
        (
            "block_entities",
            Tag::List(ListTag {
                element_type: tag_type::COMPOUND,
                elements: vec![compound(vec![
                    ("solaris:TransferCooldownUntil", Tag::Long(10)),
                    ("my_mod:payload", foreign_payload()),
                ])],
            }),
        ),
    ]);
    let mut report = PreRename::default();
    report
        .rename(&mut chunk, Owner::Chunk, &|| "chunk".into())
        .unwrap();
    assert_eq!(report.keys, 5);
    let Tag::Compound(fields) = &chunk else {
        unreachable!()
    };
    assert_eq!(fields[0].0, "KrasnokamenSimDebt");
    let Tag::List(entities) = &fields[1].1 else {
        unreachable!()
    };
    assert_eq!(
        keys(&entities.elements[0]),
        [
            "krasnokamen:WireId",
            "my_mod:payload",
            "krasnokamen:VillagerRuntime"
        ]
    );
    let Tag::Compound(entity) = &entities.elements[0] else {
        unreachable!()
    };
    assert_eq!(entity[1].1, foreign_payload());
    assert_eq!(keys(&entity[2].1), ["KrasnokamenSoldier"]);
    let Tag::List(blocks) = &fields[2].1 else {
        unreachable!()
    };
    assert_eq!(
        keys(&blocks.elements[0]),
        ["krasnokamen:TransferCooldownUntil", "my_mod:payload"]
    );
    let Tag::Compound(block) = &blocks.elements[0] else {
        unreachable!()
    };
    assert_eq!(block[1].1, foreign_payload());

    for (owner, container, old, current) in [
        (
            Owner::Level,
            "Data",
            "SolarisTerrainEpoch",
            "KrasnokamenTerrainEpoch",
        ),
        (
            Owner::Raids,
            "data",
            "solaris:clock_tick",
            "krasnokamen:clock_tick",
        ),
        (
            Owner::DragonFight,
            "data",
            "SolarisFightState",
            "KrasnokamenFightState",
        ),
    ] {
        let mut tag = compound(vec![(
            container,
            compound(vec![(old, Tag::Long(1)), ("foreign", foreign_payload())]),
        )]);
        let mut report = PreRename::default();
        report
            .rename(&mut tag, owner, &|| "metadata".into())
            .unwrap();
        assert_eq!(report.keys, 1);
        let Tag::Compound(fields) = &tag else {
            unreachable!()
        };
        assert_eq!(keys(&fields[0].1), [current, "foreign"]);
        let Tag::Compound(data) = &fields[0].1 else {
            unreachable!()
        };
        assert_eq!(data[1].1, foreign_payload());
    }
}

/// An RC5 export of `tag`: every Krasnokamen field name back in its pre-rename
/// spelling. Returns how many names changed.
fn rc5_names(tag: &mut Tag) -> u64 {
    match tag {
        Tag::Compound(fields) => fields
            .iter_mut()
            .map(|(key, value)| {
                let old = key
                    .replace("krasnokamen", "solaris")
                    .replace("Krasnokamen", "Solaris")
                    .replace("KRASNOKAMEN", "SOLARIS");
                let changed = u64::from(old != *key);
                *key = old;
                changed + rc5_names(value)
            })
            .sum(),
        Tag::List(list) => list.elements.iter_mut().map(rc5_names).sum(),
        _ => 0,
    }
}

fn mentions_pre_rename(tag: &Tag) -> bool {
    match tag {
        Tag::Compound(fields) => fields.iter().any(|(key, value)| {
            key.to_ascii_lowercase().contains("solaris") || mentions_pre_rename(value)
        }),
        Tag::List(list) => list.elements.iter().any(mentions_pre_rename),
        _ => false,
    }
}

#[test]
fn nested_pre_rename_fields_take_their_current_names_and_values_stay() {
    let mut tag = compound(vec![
        ("solaris:PortalTime", Tag::Int(7)),
        (
            "SolarisUnlockedRecipes",
            Tag::List(ListTag {
                element_type: tag_type::STRING,
                elements: Vec::new(),
            }),
        ),
        (
            "Item",
            compound(vec![(
                "components",
                compound(vec![("solaris:entity_loot", Tag::Byte(1))]),
            )]),
        ),
        ("Tags", {
            Tag::List(ListTag {
                element_type: tag_type::STRING,
                elements: vec![Tag::String("solaris:keep".into())],
            })
        }),
        ("CustomName", Tag::String("Solaris".into())),
    ]);
    let mut report = PreRename::default();
    report
        .rename(&mut tag, Owner::Player, &|| "players/data/x.dat".into())
        .unwrap();
    assert_eq!(
        keys(&tag),
        [
            "krasnokamen:PortalTime",
            "KrasnokamenUnlockedRecipes",
            "Item",
            "Tags",
            "CustomName"
        ]
    );
    let Tag::Compound(fields) = &tag else {
        unreachable!()
    };
    let Tag::Compound(item) = &fields[2].1 else {
        unreachable!()
    };
    assert_eq!(keys(&item[0].1), ["solaris:entity_loot"]);
    assert_eq!(
        fields[3].1,
        Tag::List(ListTag {
            element_type: tag_type::STRING,
            elements: vec![Tag::String("solaris:keep".into())],
        })
    );
    assert_eq!(fields[4].1, Tag::String("Solaris".into()));
    assert_eq!(report.keys, 2);
    assert_eq!(report.values, 1);
    assert_eq!(
        report.first_value.as_deref(),
        Some("players/data/x.dat field Tags")
    );
}

#[test]
fn an_rc5_player_rider_becomes_the_rider_list() {
    let mut tag = compound(vec![("solaris:Rider", Tag::IntArray(vec![0, 0, 0, 3]))]);
    let mut report = PreRename::default();
    report
        .rename(&mut tag, Owner::Entity, &|| "horse".into())
        .unwrap();
    assert_eq!(
        tag,
        compound(vec![(
            "krasnokamen:Riders",
            Tag::List(ListTag {
                element_type: tag_type::INT_ARRAY,
                elements: vec![Tag::IntArray(vec![0, 0, 0, 3])],
            })
        )])
    );
    assert_eq!(report.keys, 1);

    let mut broken = compound(vec![("solaris:Rider", Tag::Int(3))]);
    let error = PreRename::default()
        .rename(&mut broken, Owner::Entity, &|| "horse".into())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("horse: pre-rename solaris:Rider is not a UUID"),
        "{error}"
    );
}

#[test]
fn a_field_under_both_names_refuses_the_import() {
    let mut tag = compound(vec![
        ("krasnokamen:PortalTime", Tag::Int(1)),
        ("solaris:PortalTime", Tag::Int(2)),
    ]);
    let error = PreRename::default()
        .rename(&mut tag, Owner::Player, &|| "players/data/x.dat".into())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(
            "players/data/x.dat: field krasnokamen:PortalTime is present under both its pre-rename and its current name"
        ),
        "{error}"
    );
}

/// RC5 `world export-anvil` output names its own fields `solaris:*`/`Solaris*`
/// and keeps a player rider as `solaris:Rider` on the vehicle. Import must give
/// this build those fields (wire id, portal time, unlocked recipes) and the
/// ridden horse its rider, not keep them as unread extra data.
#[test]
fn rc5_export_with_a_player_rider_keeps_its_fields_and_the_rider() {
    use crate::roundtrip_tests::{modern_fixture, save};
    use mc_core::dimensions::Dimension;
    use mc_core::native_checkpoint as bridge;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("anvil");
    let native = temp.path().join("native");
    let output = temp.path().join("export");
    std::fs::create_dir(&source).unwrap();
    let config = crate::test_support::config(native.clone(), 42);
    modern_fixture(&source, &config);
    let registry = crate::test_support::registry().unwrap();
    let player = uuid::Uuid::from_u128(3);
    let horse = uuid::Uuid::from_u128(0x5ad);
    let entities = Dimension::Overworld.directory(&source).join("entities");
    let mut chunks = mc_world::anvil::read_entity_regions(&entities, &registry.items).unwrap();
    chunks
        .get_mut(&(-1, 1))
        .unwrap()
        .entities
        .push(mc_data::entity_nbt::VanillaEntity {
            id: "minecraft:horse".into(),
            uuid: horse,
            pos: [-8.0, 64.0, 24.0],
            health: 20.0,
            extra_fields: vec![
                ("Tame".into(), Tag::Byte(1)),
                ("solaris:WireId".into(), Tag::Int(201)),
                ("solaris:PortalTime".into(), Tag::Int(7)),
                ("solaris:Rider".into(), Tag::IntArray(vec![0, 0, 0, 3])),
                ("my_mod:payload".into(), foreign_payload()),
            ],
            ..Default::default()
        });
    mc_world::anvil::write_entity_regions(&entities, &registry.items, &chunks, 79).unwrap();
    let mut renamed = 3;
    let mut level = crate::read_nbt(&source.join("level.dat")).unwrap();
    renamed += rc5_names(&mut level);
    save(&source, "level.dat", &level);
    let player_file = format!("players/data/{player}.dat");
    let mut player_tag = crate::read_nbt(&source.join(&player_file)).unwrap();
    let Tag::Compound(fields) = &mut player_tag else {
        unreachable!()
    };
    fields.push((
        "KrasnokamenUnlockedRecipes".into(),
        Tag::List(ListTag {
            element_type: tag_type::STRING,
            elements: vec![Tag::String("minecraft:crafting_table".into())],
        }),
    ));
    renamed += rc5_names(&mut player_tag);
    save(&source, &player_file, &player_tag);

    let receipt =
        crate::import_anvil(&config, &source, &native, &temp.path().join("import.json")).unwrap();
    assert_eq!(receipt.status, "PASS");
    assert_eq!(receipt.counts.get("pre_rename_keys"), Some(&renamed));
    assert!(
        receipt
            .normalizations
            .iter()
            .any(|line| line.contains("pre-rename solaris/Solaris fields renamed")),
        "{:?}",
        receipt.normalizations
    );

    let recovered = bridge::recover_offline(&config).unwrap();
    let ridden = recovered.dimensions[&Dimension::Overworld]
        .entities
        .values()
        .find(|entity| entity.uuid == horse.to_string())
        .expect("imported horse");
    assert_eq!(ridden.state.entity_life.mount.riders, [player]);

    crate::export_anvil(&config, &native, &output, &temp.path().join("export.json")).unwrap();
    let exported = mc_world::anvil::read_entity_regions(
        &Dimension::Overworld.directory(&output).join("entities"),
        &registry.items,
    )
    .unwrap();
    let exported_horse = exported[&(-1, 1)]
        .entities
        .iter()
        .find(|entity| entity.uuid == horse)
        .expect("exported horse");
    assert!(
        exported_horse
            .extra_fields
            .contains(&("my_mod:payload".into(), foreign_payload()))
    );
    assert!(
        exported_horse
            .extra_fields
            .iter()
            .any(|(name, value)| name == "krasnokamen:PortalTime" && *value == Tag::Int(7)),
        "{:?}",
        exported_horse.extra_fields
    );
    assert!(
        !exported_horse
            .extra_fields
            .iter()
            .any(|(name, _)| name.to_ascii_lowercase().contains("solaris")),
        "{:?}",
        exported_horse.extra_fields
    );
    let exported_player = crate::read_nbt(&output.join(&player_file)).unwrap();
    assert!(
        !mentions_pre_rename(&exported_player),
        "{exported_player:?}"
    );
    // Export writes the vanilla player shape: the native recipe field is folded
    // into `recipeBook.recipes`, which is what a vanilla server reads.
    let Tag::Compound(fields) = &exported_player else {
        unreachable!()
    };
    let Some(Tag::Compound(book)) = fields
        .iter()
        .find(|(name, _)| name == "recipeBook")
        .map(|(_, value)| value)
    else {
        panic!("exported player has no recipeBook: {exported_player:?}")
    };
    assert!(
        book.iter().any(|(name, value)| name == "recipes"
            && matches!(value, Tag::List(list)
                if list.elements.contains(&Tag::String("minecraft:crafting_table".into())))),
        "{exported_player:?}"
    );
    assert!(
        !fields
            .iter()
            .any(|(name, _)| name == "KrasnokamenUnlockedRecipes"),
        "{exported_player:?}"
    );
}

// Apply pre_rename_unscoped_control.patch at the repository root, then run only
// this test: RED must explicitly show the foreign key changed, without a Rider.
#[test]
fn foreign_key_is_preserved_without_a_legacy_rider() {
    let original = compound(vec![(
        "my_mod:payload",
        compound(vec![("my_mod:solaris_flag", Tag::Int(7))]),
    )]);
    let mut tag = original.clone();
    PreRename::default()
        .rename(&mut tag, Owner::Player, &|| "foreign key".into())
        .unwrap();
    assert_eq!(tag, original, "foreign key was renamed");
}

#[path = "pre_rename_bee_tests.rs"]
mod bee_items;
