use super::PreRename;
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
        ("solaris:WireId", Tag::Int(7)),
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
        .rename(&mut tag, &|| "entities/r.0.0.mca#chunk(0, 0)".into())
        .unwrap();
    assert_eq!(
        keys(&tag),
        [
            "krasnokamen:WireId",
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
    assert_eq!(keys(&item[0].1), ["krasnokamen:entity_loot"]);
    assert_eq!(
        fields[3].1,
        Tag::List(ListTag {
            element_type: tag_type::STRING,
            elements: vec![Tag::String("solaris:keep".into())],
        })
    );
    assert_eq!(fields[4].1, Tag::String("Solaris".into()));
    assert_eq!(report.keys, 3);
    assert_eq!(report.values, 1);
    assert_eq!(
        report.first_value.as_deref(),
        Some("entities/r.0.0.mca#chunk(0, 0) field Tags")
    );
}

#[test]
fn an_rc5_player_rider_becomes_the_rider_list() {
    let mut tag = compound(vec![("solaris:Rider", Tag::IntArray(vec![0, 0, 0, 3]))]);
    let mut report = PreRename::default();
    report.rename(&mut tag, &|| "horse".into()).unwrap();
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
        .rename(&mut broken, &|| "horse".into())
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
        .rename(&mut tag, &|| "players/data/x.dat".into())
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
