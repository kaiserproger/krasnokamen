use super::*;

fn tags(state: &TickState, id: EntityId) -> Vec<Tag> {
    let Tag::Compound(fields) = crate::command_data::entity(state, id).unwrap() else {
        panic!("entity compound");
    };
    let Some(Tag::List(list)) = mc_data::item_nbt::field(&fields, "Tags") else {
        panic!("entity tags");
    };
    list.elements.clone()
}

fn command(state: &mut TickState, text: &str) -> i32 {
    let actor = state.connections.entity_of(ConnectionId(1)).unwrap();
    let context = crate::command_context::Context::source(state, ConnectionId(1), actor, 2);
    crate::command_context::run(state, text, context, &mut ChangeSet::default())
}

#[test]
fn duplicate_tags_summon_load_add_and_remove_have_set_semantics() {
    for (kind, nbt) in [
        ("cow", "{Tags:['d1','d1','second','d1']}"),
        ("armor_stand", "{Tags:['d1','d1','second','d1']}"),
        (
            "item",
            "{Tags:['d1','d1','second','d1'],Item:{id:'minecraft:stone'}}",
        ),
        ("experience_orb", "{Tags:['d1','d1','second','d1'],Value:1}"),
    ] {
        let (mut state, id) = summon(kind, nbt);
        assert_eq!(
            tags(&state, id),
            vec![Tag::String("d1".into()), Tag::String("second".into())],
            "{kind}"
        );
        let uuid = state.entities.row(state.entities.slot_of(id).unwrap()).uuid;
        assert_eq!(command(&mut state, &format!("tag {uuid} add d1")), 0);
        assert_eq!(command(&mut state, &format!("tag {uuid} remove d1")), 1);
        assert_eq!(tags(&state, id), vec![Tag::String("second".into())]);
        assert_eq!(command(&mut state, &format!("tag {uuid} add d1")), 1);
        assert_eq!(tags(&state, id).len(), 2);

        let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
        let mut native = chunks
            .values()
            .flat_map(|chunk| &chunk.entities)
            .find(|entity| entity.wire_id == id.0)
            .unwrap()
            .clone();
        // A pre-fix save/import must normalize at load, not only at summon.
        native.extra_fields.retain(|(key, _)| key != "Tags");
        native.extra_fields.push((
            "Tags".into(),
            crate::command_snbt::parse("['d1','d1']").unwrap(),
        ));
        let loaded = EntityStateImage::from_record(&state.registry, &native).unwrap();
        let (_, Tag::Compound(fields)) =
            mc_nbt::read_named(&mut loaded.opaque_entity_fields.as_slice()).unwrap()
        else {
            panic!("retained compound")
        };
        assert_eq!(
            mc_data::item_nbt::field(&fields, "Tags"),
            Some(&Tag::List(ListTag {
                element_type: tag_type::STRING,
                elements: vec![Tag::String("d1".into())]
            }))
        );

        // Existing duplicate opaque data must not survive one /tag remove.
        let mut legacy = Vec::new();
        mc_nbt::write_named(
            &mut legacy,
            "",
            &Tag::Compound(vec![(
                "Tags".into(),
                crate::command_snbt::parse("['d1','d1']").unwrap(),
            )]),
        )
        .unwrap();
        state
            .entities
            .row_mut(state.entities.slot_of(id).unwrap())
            .opaque_entity_fields = legacy.clone();
        assert_eq!(command(&mut state, &format!("tag {uuid} add new")), 1);
        assert_eq!(
            tags(&state, id),
            vec![Tag::String("d1".into()), Tag::String("new".into())]
        );
        state
            .entities
            .row_mut(state.entities.slot_of(id).unwrap())
            .opaque_entity_fields = legacy;
        assert_eq!(command(&mut state, &format!("tag {uuid} remove d1")), 1);
        assert!(tags(&state, id).is_empty());
    }
}

#[test]
fn tag_refusals_name_element_type_count_and_modified_utf8_limit_before_spawn() {
    // Vanilla Entity.TAG_LIST_CODEC is STRING.sizeLimitedListOf(1024)
    // before Entity.load adds to its Set (26.3 Entity.javap:12054-12060).
    let too_many = format!("[{}]", vec!["'x'"; 1025].join(","));
    for (value, expected) in [
        ("[1b]".to_owned(), "every list element must be a string"),
        ("['ok',2]".to_owned(), "every list element must be a string"),
        ("[{a:1}]".to_owned(), "every list element must be a string"),
        (too_many, "expected at most 1024 tags"),
        (
            format!("['{}']", "C".repeat(70000)),
            "tag at index 0 is 70000 Modified UTF-8 bytes; maximum is 65535",
        ),
        (
            format!("['{}']", "😀".repeat(10923)),
            "tag at index 0 is 65538 Modified UTF-8 bytes; maximum is 65535",
        ),
    ] {
        let mut state = rejected_tag_fixture();
        let count = state.entities.len();
        command(&mut state, &format!("summon cow ~ ~ ~ {{Tags:{value}}}"));
        assert_eq!(state.entities.len(), count, "refusal allocated an entity");
        assert!(state.outbox.iter().any(|(recipient, queue)| *recipient == ConnectionId(1) && queue.iter().any(|change| matches!(change, PlayerChange::Chat { message } if message.contains(&format!("Invalid Tags: {expected}"))))), "missing specific refusal {expected}: {:?}", state.outbox);
    }
    // The same parser boundary applies to the non-mob spawn paths.
    let state = rejected_tag_fixture();
    let nbt = crate::command_snbt::parse(&format!(
        "{{Tags:['{}'],Value:1,Item:{{id:'minecraft:stone'}}}}",
        "C".repeat(70000)
    ))
    .unwrap();
    assert!(
        experience_orb(&nbt)
            .unwrap_err()
            .message()
            .contains("Invalid Tags: tag at index 0")
    );
    assert!(
        prepare_item(&state, &nbt)
            .err()
            .unwrap()
            .message()
            .contains("Invalid Tags: tag at index 0")
    );
    for value in ["C".repeat(65535), "😀".repeat(10922)] {
        let nbt = Tag::List(ListTag {
            element_type: tag_type::STRING,
            elements: vec![Tag::String(value)],
        });
        validate_tags(&nbt).unwrap();
    }
}

#[test]
fn duplicate_tags_keys_are_refused_by_summon_and_data_before_mutation() {
    let (mut state, id) = summon("cow", "{Tags:['original']}");
    let uuid = state.entities.row(state.entities.slot_of(id).unwrap()).uuid;
    let before = state.entities.len();
    state.command_storage.insert(
        "test:duplicate".into(),
        Tag::Compound(vec![
            ("Tags".into(), crate::command_snbt::parse("['a']").unwrap()),
            ("Tags".into(), crate::command_snbt::parse("['b']").unwrap()),
        ]),
    );
    for text in [
        "summon cow ~ ~ ~ {Tags:['a'],Tags:['b']}".to_owned(),
        "summon item ~ ~ ~ {Item:{id:'minecraft:stone'},Tags:['a'],Tags:['b']}".to_owned(),
        "summon experience_orb ~ ~ ~ {Value:1,Tags:['a'],Tags:['b']}".to_owned(),
        format!("data merge entity {uuid} {{Tags:['a'],Tags:['b']}}"),
        format!("data modify entity {uuid} {{}} merge value {{Tags:['a'],Tags:['b']}}"),
        format!("data modify entity {uuid} {{}} merge from storage test:duplicate"),
    ] {
        state.outbox.clear();
        assert_eq!(command(&mut state, &text), 0, "{text}");
        assert_eq!(state.entities.len(), before);
        assert_eq!(tags(&state, id), vec![Tag::String("original".into())]);
        assert!(
            state.outbox.values().flatten().any(|change| matches!(
                change, PlayerChange::Chat { message }
                if message == "Duplicate SNBT key"
                    || message.to_lowercase().contains("duplicate") && message.contains("Tags")
            )),
            "{text}: {:?}",
            state.outbox
        );
    }
}
