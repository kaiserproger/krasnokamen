use super::*;
use crate::chunk::ChunkPos;
use mc_data::blocks::{BlockReport, BlockStateReport};
use std::collections::BTreeMap;
fn project_single_entity(tag: Tag) -> Tag {
    let registry = BlockRegistry::from_report(&[BlockReport {
        id: Identifier::parse("minecraft:air").unwrap(),
        properties: BTreeMap::new(),
        states: vec![BlockStateReport {
            id: 0,
            default: true,
            properties: BTreeMap::new(),
        }],
    }])
    .unwrap();
    let mut chunk = Chunk::empty(
        ChunkPos { x: 0, z: 0 },
        crate::block::BlockStateId(0),
        Identifier::parse("minecraft:plains").unwrap(),
    );
    let pos = BlockPos { x: 1, y: 64, z: 0 };
    let mut bytes = Vec::new();
    mc_nbt::write_network(&mut bytes, &tag).unwrap();
    chunk.block_entities.insert(pos, bytes);
    let result = client_block_entity(&chunk, &registry, pos).unwrap().nbt;
    assert_eq!(client_block_entities(&chunk, &registry)[0].nbt, result);
    result
}

#[test]
fn vault_filters_shared_data_and_keeps_display_item() {
    let display_item = Tag::Compound(vec![("id".into(), Tag::String("minecraft:diamond".into()))]);
    let tag = Tag::Compound(vec![
        ("id".into(), Tag::String("minecraft:vault".into())),
        (
            "shared_data".into(),
            Tag::Compound(vec![
                ("display_item".into(), display_item.clone()),
                ("secret".into(), Tag::String("private".into())),
            ]),
        ),
    ]);
    assert_eq!(
        project_single_entity(tag),
        Tag::Compound(vec![(
            "shared_data".into(),
            Tag::Compound(vec![("display_item".into(), display_item)]),
        )])
    );
}

#[test]
fn banner_filters_unknown_component_types() {
    let custom = Tag::Compound(vec![("visible".into(), Tag::String("allowed".into()))]);
    let tag = Tag::Compound(vec![(
        "components".into(),
        Tag::Compound(vec![
            ("minecraft:custom_data".into(), custom.clone()),
            (
                "minecraft:unknown_private_component".into(),
                Tag::String("private".into()),
            ),
        ]),
    )]);
    let expected = Tag::Compound(vec![(
        "components".into(),
        Tag::Compound(vec![("minecraft:custom_data".into(), custom)]),
    )]);
    assert_eq!(
        super::public_block_entity_update_tag("minecraft:banner", tag),
        expected
    );
}

#[test]
fn banner_filters_unknown_pattern_layer_fields() {
    let pattern = |secret: bool| {
        let mut fields = vec![
            (
                "pattern".into(),
                Tag::String("minecraft:stripe_bottom".into()),
            ),
            ("color".into(), Tag::String("red".into())),
        ];
        if secret {
            fields.push(("secret".into(), Tag::String("private".into())));
        }
        Tag::List(mc_nbt::ListTag {
            element_type: mc_nbt::tag_type::COMPOUND,
            elements: vec![Tag::Compound(fields)],
        })
    };
    let tag = Tag::Compound(vec![
        ("id".into(), Tag::String("minecraft:banner".into())),
        ("patterns".into(), pattern(true)),
    ]);
    let expected = Tag::Compound(vec![("patterns".into(), pattern(false))]);
    assert_eq!(project_single_entity(tag), expected);
}

fn assert_sign_text_filters_unknown_fields(kind: &str) {
    let messages = Tag::List(mc_nbt::ListTag {
        element_type: mc_nbt::tag_type::STRING,
        elements: vec![Tag::String("visible".into()); 4],
    });
    let text = Tag::Compound(vec![
        ("messages".into(), messages.clone()),
        ("color".into(), Tag::String("red".into())),
        ("secret".into(), Tag::String("private".into())),
    ]);
    let tag = Tag::Compound(vec![
        ("id".into(), Tag::String(format!("minecraft:{kind}"))),
        ("front_text".into(), text),
    ]);
    let expected = Tag::Compound(vec![(
        "front_text".into(),
        Tag::Compound(vec![
            ("messages".into(), messages),
            ("color".into(), Tag::String("red".into())),
        ]),
    )]);
    assert_eq!(project_single_entity(tag), expected);
}

#[test]
fn sign_filters_unknown_text_fields() {
    assert_sign_text_filters_unknown_fields("sign");
}

#[test]
fn hanging_sign_filters_unknown_text_fields() {
    assert_sign_text_filters_unknown_fields("hanging_sign");
}

fn assert_spawn_data_filters_unknown(kind: &str, key: &str) {
    let entity = Tag::Compound(vec![("id".into(), Tag::String("minecraft:zombie".into()))]);
    let rules = Tag::Compound(vec![(
        "block_light_limit".into(),
        Tag::Compound(vec![
            ("min_inclusive".into(), Tag::Int(0)),
            ("max_inclusive".into(), Tag::Int(4)),
        ]),
    )]);
    let equipment = Tag::Compound(vec![
        ("loot_table".into(), Tag::String("minecraft:test".into())),
        (
            "slot_drop_chances".into(),
            Tag::Compound(vec![("head".into(), Tag::Float(0.5))]),
        ),
    ]);
    let mut private_rules = match rules.clone() {
        Tag::Compound(fields) => fields,
        _ => unreachable!(),
    };
    if let Tag::Compound(range) = &mut private_rules[0].1 {
        range.push(("range_secret".into(), Tag::String("private".into())));
    }
    private_rules.push(("secret".into(), Tag::String("private".into())));
    let mut private_equipment = match equipment.clone() {
        Tag::Compound(fields) => fields,
        _ => unreachable!(),
    };
    if let Tag::Compound(chances) = &mut private_equipment[1].1 {
        chances.push(("slot_secret".into(), Tag::Float(0.8)));
    }
    private_equipment.push(("secret".into(), Tag::String("private".into())));
    let tag = Tag::Compound(vec![
        ("id".into(), Tag::String(format!("minecraft:{kind}"))),
        (
            key.into(),
            Tag::Compound(vec![
                ("entity".into(), entity.clone()),
                ("custom_spawn_rules".into(), Tag::Compound(private_rules)),
                ("equipment".into(), Tag::Compound(private_equipment)),
                ("secret".into(), Tag::String("private".into())),
            ]),
        ),
    ]);
    let expected = Tag::Compound(vec![(
        key.into(),
        Tag::Compound(vec![
            ("entity".into(), entity),
            ("custom_spawn_rules".into(), rules),
            ("equipment".into(), equipment),
        ]),
    )]);
    assert_eq!(project_single_entity(tag), expected);
}

#[test]
fn mob_spawner_filters_unknown_spawn_data() {
    assert_spawn_data_filters_unknown("mob_spawner", "SpawnData");
}

#[test]
fn trial_spawner_filters_unknown_spawn_data() {
    assert_spawn_data_filters_unknown("trial_spawner", "spawn_data");
}

#[test]
fn test_instance_filters_unknown_nested_fields() {
    let data = Tag::Compound(vec![
        ("size".into(), Tag::IntArray(vec![1, 2, 3])),
        ("rotation".into(), Tag::String("none".into())),
        ("ignore_entities".into(), Tag::Byte(0)),
        ("status".into(), Tag::String("cleared".into())),
    ]);
    let mut private_data = match data.clone() {
        Tag::Compound(fields) => fields,
        _ => unreachable!(),
    };
    private_data.push(("secret".into(), Tag::String("private".into())));
    let error = Tag::Compound(vec![
        ("pos".into(), Tag::IntArray(vec![1, 2, 3])),
        ("text".into(), Tag::String("failure".into())),
    ]);
    let mut private_error = match error.clone() {
        Tag::Compound(fields) => fields,
        _ => unreachable!(),
    };
    private_error.push(("secret".into(), Tag::String("private".into())));
    let errors = |element| {
        Tag::List(mc_nbt::ListTag {
            element_type: mc_nbt::tag_type::COMPOUND,
            elements: vec![element],
        })
    };
    let tag = Tag::Compound(vec![
        (
            "id".into(),
            Tag::String("minecraft:test_instance_block".into()),
        ),
        ("data".into(), Tag::Compound(private_data)),
        ("errors".into(), errors(Tag::Compound(private_error))),
    ]);
    let expected = Tag::Compound(vec![
        ("data".into(), data),
        ("errors".into(), errors(error)),
    ]);
    assert_eq!(project_single_entity(tag), expected);
}
