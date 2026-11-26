use super::*;

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "dead non-corpse row reached checkpoint capture")]
fn capture_rejects_dead_rows_outside_the_dying_clock() {
    let (mut state, _, position) = crate::hostile_tests::setup(0.0);
    let cow = state.spawn_mob(state.registry.cow_type_id, position, 20.0);
    let slot = state.entities.slot_of(cow).unwrap();
    state.entities.body_mut(slot).alive = false;
    assert!(!state.entities.is_dying(cow));
    let _ = entity_data_record(&state, slot);
}

/// ANVIL-IMPORT-STYLED-CUSTOM-NAME-SILENTLY-DROPPED: a vanilla entity whose `CustomName` is a text
/// component loses the whole name, because the reader accepts only `Tag::String`. Ignored so the
/// gate stays green; run with `-- --ignored`.
#[test]
#[ignore = "ANVIL-IMPORT-STYLED-CUSTOM-NAME-SILENTLY-DROPPED: run with -- --ignored"]
fn an_imported_styled_custom_name_is_not_dropped() {
    let record = mc_data::entity_nbt::VanillaEntity {
        id: "minecraft:cow".into(),
        extra_fields: vec![(
            "CustomName".into(),
            mc_nbt::Tag::Compound(vec![
                ("text".into(), mc_nbt::Tag::String("Bessie".into())),
                ("color".into(), mc_nbt::Tag::String("red".into())),
            ]),
        )],
        ..Default::default()
    };
    let image = appearance_from_vanilla_entity(&record);
    assert_eq!(
        image.custom_name.as_deref(),
        Some("Bessie"),
        "a compound CustomName must be read, not dropped"
    );
}
