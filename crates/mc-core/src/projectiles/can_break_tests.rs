use mc_nbt::{ListTag, Tag, tag_type};

use super::*;
use crate::bees::tests::{POS, place, state};

/// An arrow stack whose `minecraft:can_break` component holds `predicate`.
pub(crate) fn arrow(state: &TickState, predicate: Option<Tag>) -> ItemStack {
    let mut fields = vec![
        ("id".to_owned(), Tag::String("minecraft:arrow".into())),
        ("count".to_owned(), Tag::Int(1)),
    ];
    if let Some(predicate) = predicate {
        fields.push((
            "components".to_owned(),
            Tag::Compound(vec![("minecraft:can_break".to_owned(), predicate)]),
        ));
    }
    mc_data::item_nbt::stack_from_fields(&state.registry.items, &fields)
        .unwrap()
        .unwrap()
}

fn compound(fields: &[(&str, Tag)]) -> Tag {
    Tag::Compound(
        fields
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn text(value: &str) -> Tag {
    Tag::String(value.into())
}

fn list(elements: Vec<Tag>) -> Tag {
    let element_type = if matches!(elements.first(), Some(Tag::String(_))) {
        tag_type::STRING
    } else {
        tag_type::COMPOUND
    };
    Tag::List(ListTag {
        element_type,
        elements,
    })
}

fn admits(state: &TickState, predicate: Tag) -> bool {
    can_break(state, &arrow(state, Some(predicate)), POS)
}

#[test]
fn block_ids_lists_and_alternatives_select_the_block() {
    let mut state = state(None);
    place(&mut state, POS, "minecraft:tnt");
    assert!(
        !can_break(&state, &arrow(&state, None), POS),
        "no component"
    );
    assert!(!can_break(&state, &ItemStack::EMPTY, POS));
    assert!(admits(
        &state,
        compound(&[("blocks", text("minecraft:tnt"))])
    ));
    assert!(!admits(
        &state,
        compound(&[("blocks", text("minecraft:stone"))])
    ));
    assert!(admits(
        &state,
        compound(&[(
            "blocks",
            list(vec![text("minecraft:stone"), text("minecraft:tnt")])
        )])
    ));
    assert!(admits(
        &state,
        list(vec![
            compound(&[("blocks", text("minecraft:stone"))]),
            compound(&[("blocks", text("minecraft:tnt"))]),
        ])
    ));
    assert!(admits(&state, compound(&[])), "an empty predicate admits");
    assert!(
        !admits(
            &state,
            compound(&[
                ("blocks", text("minecraft:tnt")),
                (
                    "components",
                    compound(&[("minecraft:custom_name", text("x"))])
                ),
            ])
        ),
        "unmodelled component matchers admit nothing"
    );
}

#[test]
fn state_properties_match_exact_values_and_ordered_ranges() {
    let mut state = state(None);
    place(&mut state, POS, "minecraft:wheat");
    let id = state.world.block_at(POS.x, POS.y, POS.z).unwrap();
    let aged = crate::farming::with_prop(&state.registry.blocks, id, "age", "3").unwrap();
    state.world.set_block(POS.x, POS.y, POS.z, aged);
    let with_state = |properties: Tag| compound(&[("state", properties)]);
    for (properties, admitted) in [
        (compound(&[("age", text("3"))]), true),
        (compound(&[("age", text("4"))]), false),
        (compound(&[("age", text("9"))]), false),
        (
            compound(&[("age", compound(&[("min", text("2")), ("max", text("5"))]))]),
            true,
        ),
        (compound(&[("age", compound(&[("min", text("4"))]))]), false),
        (compound(&[("age", compound(&[("max", text("3"))]))]), true),
        (compound(&[("age", compound(&[("max", text("x"))]))]), false),
        (compound(&[("missing", text("1"))]), false),
    ] {
        assert_eq!(
            admits(&state, with_state(properties.clone())),
            admitted,
            "{properties:?}"
        );
    }
    place(&mut state, POS, "minecraft:tnt");
    for (properties, admitted) in [
        (compound(&[("unstable", text("false"))]), true),
        (
            compound(&[("unstable", compound(&[("max", text("false"))]))]),
            true,
        ),
        (
            compound(&[("unstable", compound(&[("min", text("true"))]))]),
            false,
        ),
    ] {
        assert_eq!(
            admits(&state, with_state(properties.clone())),
            admitted,
            "{properties:?}"
        );
    }
}

#[test]
fn nbt_needs_a_block_entity_whose_image_contains_it() {
    let mut state = state(None);
    place(&mut state, POS, "minecraft:tnt");
    assert!(!admits(&state, compound(&[("nbt", text("{}"))])));
    place(&mut state, POS, "minecraft:chest");
    assert!(admits(&state, compound(&[("nbt", text("{}"))])));
    assert!(admits(
        &state,
        compound(&[("nbt", text("{id:\"minecraft:chest\"}"))])
    ));
    assert!(!admits(
        &state,
        compound(&[("nbt", text("{Lock:\"key\"}"))])
    ));
}
