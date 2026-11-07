use super::super::tables::IdMap;
use super::*;

fn tables() -> Tables {
    let slot_display_names = [
        "empty",
        "any_fuel",
        "with_any_potion",
        "only_with_component",
        "item",
        "item_stack",
        "tag",
        "dyed",
        "smithing_trim",
        "with_remainder",
        "composite",
    ]
    .map(|name| format!("minecraft:{name}"))
    .to_vec();
    let recipe_display_names = [
        "crafting_shapeless",
        "crafting_shaped",
        "furnace",
        "stonecutter",
        "smithing",
    ]
    .map(|name| format!("minecraft:{name}"))
    .to_vec();
    let identity = |names: &[String]| {
        let entries = || {
            names
                .iter()
                .enumerate()
                .map(|(id, name)| (name.as_str(), id as u32))
        };
        IdMap::by_name(entries(), entries(), |_| None)
    };
    let registry_ids = [
        (
            "custom_stat".into(),
            IdMap::by_name([("jump", 9)], [("jump", 8)], |_| None),
        ),
        ("slot_display".into(), identity(&slot_display_names)),
        ("recipe_display".into(), identity(&recipe_display_names)),
    ]
    .into();
    Tables {
        items: IdMap::by_name([("stick", 10)], [("stick", 3)], |_| None),
        argument_types: IdMap::by_name([("uuid", 61)], [("uuid", 58)], |_| None),
        argument_names: {
            let mut names = vec![String::new(); 62];
            names[61] = "minecraft:uuid".into();
            names
        },
        slot_display_names,
        recipe_display_names,
        registry_ids,
        ..Tables::default()
    }
}
fn translate(name: &str, body: &[u8]) -> Vec<u8> {
    let mut r = body;
    let mut out = Vec::new();
    clientbound(&tables(), name, &mut r, &mut out).unwrap();
    assert!(r.is_empty());
    out
}
#[test]
fn command_parser_ids_follow_names_not_ordinals() {
    // One argument node with no children and a UUID singleton parser.
    let body = [1, 2, 0, 1, b'x', 61, 0];
    assert_eq!(translate("commands", &body), [1, 2, 0, 1, b'x', 58, 0]);
}
#[test]
fn recipes_remap_templates_and_ingredients() {
    let body = [1, 1, b'x', 1, 10, 1, 2, 10, 5, 10, 2, 0, 0];
    let expected = [1, 1, b'x', 1, 3, 1, 2, 3, 5, 3, 2, 0, 0];
    assert_eq!(translate("update_recipes", &body), expected);
    // One shapeless entry with item ingredient and template output.
    let body = [
        1, 7, 0, 1, 4, 10, 5, 10, 2, 0, 0, 1, 0, 0, 1, 1, 2, 10, 3, 1,
    ];
    let expected = [1, 7, 0, 1, 4, 3, 5, 3, 2, 0, 0, 1, 0, 0, 1, 1, 2, 3, 3, 1];
    assert_eq!(translate("recipe_book_add", &body), expected);
}

#[test]
fn recipe_tag_slot_removes_native_holder_set_marker() {
    // A ghost shapeless display with one named tag slot. The surrounding
    // composite exercises the same nested slot path used by recipe_book_add.
    let mut body = vec![1, 0, 1, 10, 1, 6, 0];
    body.write_string("minecraft:birch_logs", 32767).unwrap();
    body.extend([0, 1]); // result empty, station any fuel
    let mut expected = vec![1, 0, 1, 10, 1, 6];
    expected
        .write_string("minecraft:birch_logs", 32767)
        .unwrap();
    expected.extend([0, 1]);
    assert_eq!(translate("place_ghost_recipe", &body), expected);
}
#[test]
fn custom_stat_insertions_do_not_relabel_counters() {
    assert_eq!(translate("award_stats", &[1, 8, 9, 12]), [1, 8, 8, 12]);
}
#[test]
fn advancement_coordinates_move_back_inside_display_info() {
    let mut body = vec![1, 1];
    body.write_string("test:a", 32767).unwrap();
    body.extend([0, 1]); // no parent, display present
    mc_nbt::write_network_value(&mut body, &mc_nbt::Tag::String("Title".into())).unwrap();
    mc_nbt::write_network_value(&mut body, &mc_nbt::Tag::String("Description".into())).unwrap();
    body.extend([10, 1, 0, 0, 0]); // icon template, frame
    body.write_i32(0);
    let display_end = body.len();
    body.extend([0, 0]); // no requirements, no telemetry
    body.write_f32(2.5);
    body.write_f32(-1.0);
    body.extend([0, 0, 1]); // removed, progress, show
    let out = translate("update_advancements", &body);
    let mut expected = body[..display_end].to_vec();
    // Icon item id was 10, now 3; it is five bytes before the flags.
    expected[display_end - 9] = 3;
    expected.extend_from_slice(&body[display_end + 2..display_end + 10]);
    expected.extend_from_slice(&body[display_end..display_end + 2]);
    expected.extend_from_slice(&body[display_end + 10..]);
    assert_eq!(out, expected);
}

#[test]
fn display_kinds_and_payload_shapes_follow_names_with_permuted_ids() {
    let mut t = tables();
    t.slot_display_names = ["minecraft:empty", "minecraft:composite", "minecraft:item"]
        .map(str::to_owned)
        .to_vec();
    t.recipe_display_names = ["minecraft:furnace", "minecraft:crafting_shapeless"]
        .map(str::to_owned)
        .to_vec();
    t.registry_ids.insert(
        "slot_display".into(),
        IdMap::by_name(
            [("empty", 0), ("composite", 1), ("item", 2)],
            [("empty", 4), ("composite", 5), ("item", 7)],
            |_| None,
        ),
    );
    t.registry_ids.insert(
        "recipe_display".into(),
        IdMap::by_name(
            [("furnace", 0), ("crafting_shapeless", 1)],
            [("furnace", 2), ("crafting_shapeless", 3)],
            |_| None,
        ),
    );
    let mut input = &[1, 1, 2, 10, 0, 0][..];
    let mut output = Vec::new();
    recipe(&t, &mut input, &mut output).unwrap();
    assert!(input.is_empty());
    assert_eq!(output, [3, 1, 7, 3, 4, 4]);
}

#[test]
fn upgrade_adds_tag_holder_markers_and_new_advancement_coordinates() {
    let mut t = tables();
    t.formats = super::super::tables::Formats {
        upper: 776,
        lower: 777,
    };
    let mut old_tag = vec![6];
    old_tag.write_string("minecraft:birch_logs", 32767).unwrap();
    let mut expected_tag = vec![6, 0];
    expected_tag.extend_from_slice(&old_tag[1..]);
    let mut out = Vec::new();
    slot(&t, &mut old_tag.as_slice(), &mut out, 0).unwrap();
    assert_eq!(out, expected_tag);
    let mut old = vec![1, 1];
    old.write_string("test:a", 32767).unwrap();
    old.extend([0, 0, 0, 0]); // parent, display, requirements, telemetry
    let coordinates = old.len();
    old.extend([0, 0, 1]); // removed, progress, show
    let mut out = Vec::new();
    advancements(&t, &mut old.as_slice(), &mut out).unwrap();
    let mut expected = old[..coordinates].to_vec();
    expected.extend([0; 8]); // no old display means the nearest coordinates (0,0)
    expected.extend_from_slice(&old[coordinates..]);
    assert_eq!(out, expected);
}
