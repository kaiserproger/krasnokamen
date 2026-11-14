use super::*;
use crate::boundary::PlayerInput;
use crate::tick::run_tick;
use mc_data::Identifier;
use mc_nbt::Tag;
use std::sync::Arc;
struct Empty(u32);
impl mc_world::ChunkGenerator for Empty {
    fn generate(&self, pos: mc_world::ChunkPos) -> mc_world::Chunk {
        mc_world::Chunk::empty(
            pos,
            mc_world::BlockStateId(self.0),
            Identifier::parse("minecraft:plains").unwrap(),
        )
    }
}
fn fixture() -> (TickState, EntityId) {
    let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let world = crate::world::ChunkWorld::new(
        42,
        Arc::new(Empty(registry.states.air)),
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [8, 65, 8],
        mc_world::OVERWORLD_GEOMETRY,
    );
    let mut state = TickState::new(registry, world, 2, 512);
    run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection: ConnectionId(1),
            uuid: uuid::Uuid::new_v4(),
            name: "Author".into(),
            permission_level: 0,
        }],
    );
    let actor = state.connections.entity_of(ConnectionId(1)).unwrap();
    let mut book = mc_data::ItemStack::new(
        state
            .registry
            .items
            .id_of(&Identifier::parse("minecraft:writable_book").unwrap())
            .unwrap(),
        1,
    );
    book.custom_name = Some("kept name".into());
    state.players.get_mut(&actor).unwrap().inventory.slots[0] = book;
    (state, actor)
}
#[test]
fn book_edit_sign_and_player_persistence_preserve_author_and_pages() {
    let (mut state, actor) = fixture();
    let pages = vec!["literal @a text".into(), "second page".into()];
    run_tick(
        &mut state,
        &[PlayerInput::EditBook {
            connection: ConnectionId(1),
            slot: 0,
            pages: pages.clone(),
            title: None,
        }],
    );
    assert_eq!(
        state.players[&actor].inventory.slots[0]
            .extended()
            .preserved_components[0]
            .name,
        "minecraft:writable_book_content"
    );
    run_tick(
        &mut state,
        &[PlayerInput::EditBook {
            connection: ConnectionId(1),
            slot: 0,
            pages: pages.clone(),
            title: Some("Title".into()),
        }],
    );
    let book = &state.players[&actor].inventory.slots[0];
    assert_eq!(
        state.registry.items.name_of(book.item_id).unwrap().as_str(),
        "minecraft:written_book"
    );
    assert_eq!(book.custom_name.as_deref(), Some("kept name"));
    let component = &book.extended().preserved_components[0];
    assert_eq!(component.name, "minecraft:written_book_content");
    let Tag::Compound(fields) =
        mc_nbt::read_network_value(&mut component.value.as_slice()).unwrap()
    else {
        panic!("book compound")
    };
    assert_eq!(
        mc_data::item_nbt::field(&fields, "author"),
        Some(&Tag::String("Author".into()))
    );
    assert_eq!(
        mc_data::item_nbt::field(&fields, "generation"),
        Some(&Tag::Int(0))
    );
    let record = crate::core::player_record_for(&state, actor, state.tick).unwrap();
    let tag = crate::journal::player_to_tag(&record, &state.registry.items).unwrap();
    let recovered = crate::journal::player_from_tag(&tag, &state.registry.items).unwrap();
    assert_eq!(&recovered.inventory[0], book);
    let before = book.clone();
    edit(
        &mut state,
        ConnectionId(1),
        0,
        &["rewrite signed book".into()],
        None,
    );
    assert_eq!(state.players[&actor].inventory.slots[0], before);
}
#[test]
fn book_edit_refuses_invalid_slot_page_and_spectator_without_mutation() {
    let (mut state, actor) = fixture();
    let before = state.players[&actor].inventory.slots[0].clone();
    for slot in [-1, 9, 39, 41] {
        edit(&mut state, ConnectionId(1), slot, &["text".into()], None);
    }
    edit(&mut state, ConnectionId(1), 0, &["x".repeat(1025)], None);
    edit(
        &mut state,
        ConnectionId(1),
        0,
        &vec![String::new(); 101],
        None,
    );
    edit(&mut state, ConnectionId(1), 0, &[], Some(&"x".repeat(33)));
    state.players.get_mut(&actor).unwrap().game_mode = crate::commands::GameMode::Spectator;
    edit(&mut state, ConnectionId(1), 0, &["text".into()], None);
    assert_eq!(state.players[&actor].inventory.slots[0], before);
}

#[test]
fn lectern_insert_read_turn_take_preserves_exact_book() {
    let (mut state, actor) = fixture();
    let want = state.desired_chunks();
    state.world.ensure_blocking(&want);
    let position = crate::boundary::BlockPos { x: 8, y: 65, z: 9 };
    let lectern = state
        .registry
        .blocks
        .block(&Identifier::parse("minecraft:lectern").unwrap())
        .unwrap()
        .default
        .0;
    state
        .world
        .set_block(position.x, position.y, position.z, lectern);
    state.world.fold_overlay();
    let player_slot = state.entities.slot_of(actor).unwrap();
    state.entities.set_pos(player_slot, [8.5, 65.0, 8.5]);
    edit(
        &mut state,
        ConnectionId(1),
        0,
        &["first".into(), "second".into()],
        None,
    );
    let book = state.players[&actor].inventory.slots[0].clone();
    let use_on = PlayerInput::UseItemOn {
        connection: ConnectionId(1),
        hand: Hand::Main,
        position,
        face: 1,
        cursor: [0.5; 3],
        sequence: 1,
    };
    run_tick(&mut state, std::slice::from_ref(&use_on));
    assert!(state.players[&actor].inventory.slots[0].is_empty());
    run_tick(&mut state, &[use_on]);
    let menu = state.players[&actor].open.as_ref().expect("lectern opens");
    assert_eq!(menu.kind.menu_type(), Some(17));
    let window = menu.window;
    assert_eq!(crate::lectern::comparator(&state, position), Some(1));
    run_tick(
        &mut state,
        &[PlayerInput::ContainerButton {
            connection: ConnectionId(1),
            window,
            button: 2,
        }],
    );
    assert_eq!(crate::lectern::comparator(&state, position), Some(15));
    run_tick(
        &mut state,
        &[PlayerInput::ContainerButton {
            connection: ConnectionId(1),
            window: window + 1,
            button: 3,
        }],
    );
    assert_eq!(
        crate::lectern::comparator(&state, position),
        Some(15),
        "stale window cannot take"
    );
    run_tick(
        &mut state,
        &[PlayerInput::ContainerButton {
            connection: ConnectionId(1),
            window,
            button: 3,
        }],
    );
    assert_eq!(
        state.players[&actor]
            .inventory
            .slots
            .iter()
            .filter(|stack| **stack == book)
            .count(),
        1
    );
    assert_eq!(crate::lectern::comparator(&state, position), Some(0));
}

#[test]
fn book_input_enforces_utf16_bounds_and_accepts_exact_offhand_limits() {
    let (mut state, actor) = fixture();
    let before = state.players[&actor].inventory.slots[0].clone();
    for (pages, title) in [
        (vec!["\u{1f600}".repeat(513)], None),
        (vec!["text".into()], Some("\u{1f600}".repeat(17))),
    ] {
        run_tick(
            &mut state,
            &[PlayerInput::EditBook {
                connection: ConnectionId(1),
                slot: 0,
                pages,
                title,
            }],
        );
        assert_eq!(state.players[&actor].inventory.slots[0], before);
    }
    state.players.get_mut(&actor).unwrap().inventory.slots[40] = before.clone();
    let pages = vec!["\u{1f600}".repeat(512); 100];
    let title = "\u{1f600}".repeat(16);
    let (_, batches, _) = run_tick(
        &mut state,
        &[PlayerInput::EditBook {
            connection: ConnectionId(1),
            slot: 40,
            pages: pages.clone(),
            title: Some(title.clone()),
        }],
    );
    let book = &state.players[&actor].inventory.slots[40];
    assert_eq!(state.players[&actor].inventory.slots[0], before);
    assert_eq!(
        state.registry.items.name_of(book.item_id).unwrap().as_str(),
        "minecraft:written_book"
    );
    let component = &book.extended().preserved_components[0];
    let Tag::Compound(fields) =
        mc_nbt::read_network_value(&mut component.value.as_slice()).unwrap()
    else {
        panic!("book compound")
    };
    assert_eq!(
        mc_data::item_nbt::field(&fields, "title"),
        Some(&Tag::String(title))
    );
    assert_eq!(
        mc_data::item_nbt::field(&fields, "author"),
        Some(&Tag::String("Author".into()))
    );
    assert_eq!(
        mc_data::item_nbt::field(&fields, "pages"),
        Some(&Tag::List(mc_nbt::ListTag {
            element_type: mc_nbt::tag_type::STRING,
            elements: pages.into_iter().map(Tag::String).collect(),
        }))
    );
    assert!(batches.iter().filter(|(connection, _)| *connection == ConnectionId(1))
        .flat_map(|(_, changes)| changes).any(|change| matches!(change,
            PlayerChange::MenuContents { window: 0, slots, .. } if slots.get(45) == Some(&Some(book.clone()))
        )));
}

#[test]
fn book_replacement_preserves_unrelated_components_and_removes_old_book_components() {
    use mc_data::item_stack::PreservedItemComponent;
    let (state, actor) = fixture();
    let mut held = state.players[&actor].inventory.slots[0].clone();
    let mut value = Vec::new();
    mc_nbt::write_network_value(
        &mut value,
        &Tag::Compound(vec![("kept".into(), Tag::Int(37))]),
    )
    .unwrap();
    let unrelated = PreservedItemComponent {
        name: "minecraft:custom_data".into(),
        value,
        baseline: None,
    };
    held.extended_mut()
        .preserved_components
        .push(unrelated.clone());
    for name in [
        "minecraft:writable_book_content",
        "!minecraft:writable_book_content",
        "minecraft:written_book_content",
        "!minecraft:written_book_content",
    ] {
        held.extended_mut()
            .preserved_components
            .push(PreservedItemComponent {
                name: name.into(),
                value: Vec::new(),
                baseline: None,
            });
    }
    for title in [None, Some("Title")] {
        let replacement = crate::books::replacement(
            &held,
            &state.registry.items,
            &["page".into()],
            title,
            "Author",
        )
        .unwrap();
        assert_eq!(replacement.custom_name, held.custom_name);
        let components = &replacement.extended().preserved_components;
        assert_eq!(components.len(), 2);
        assert_eq!(components[0], unrelated);
        assert_eq!(
            components[1].name,
            if title.is_some() {
                "minecraft:written_book_content"
            } else {
                "minecraft:writable_book_content"
            }
        );
    }
}

#[test]
fn book_input_refuses_signing_a_custody_reserved_writable_book() {
    use mc_script::{
        ScriptInventoryEndpoint, ScriptInventoryMaterial, ScriptInventoryResourcePlan,
        ScriptInventoryWorkPortion, ScriptOperation, ScriptOperationPayload,
        ScriptOperationRequest, ScriptOwnedInventoryOperation, ScriptOwnedInventoryResult,
    };
    let (mut state, actor) = fixture();
    let before = state.players[&actor].inventory.slots[0].clone();
    let endpoint = ScriptInventoryEndpoint::PlayerInventory { player_id: 1 };
    let inventory = crate::plugin_inventories::load(&state, "book-test", &endpoint).unwrap();
    let request = ScriptOperationRequest::try_new(
        "book-reserve",
        ScriptOperation::Inventory {
            operation: ScriptOwnedInventoryOperation::Reserve {
                operation_id: "book-reserve".into(),
                endpoint,
                resource_plan: ScriptInventoryResourcePlan::new(vec![
                    ScriptInventoryWorkPortion::new(
                        1,
                        vec![ScriptInventoryMaterial::new(
                            "minecraft:writable_book".into(),
                            1,
                        )],
                    ),
                ]),
                expected_revision: inventory.fence,
            },
        },
    )
    .unwrap();
    let reservation = crate::plugin_inventories::execute(&mut state, "book-test", &request);
    assert_eq!(reservation.failure(), None);
    let ScriptOperationPayload::OwnedInventory { result } = reservation.payload() else {
        panic!("native inventory result")
    };
    assert!(matches!(
        result.as_ref(),
        ScriptOwnedInventoryResult::Reservation { .. }
    ));
    let (_, batches, _) = run_tick(
        &mut state,
        &[PlayerInput::EditBook {
            connection: ConnectionId(1),
            slot: 0,
            pages: vec!["reserved".into()],
            title: Some("Title".into()),
        }],
    );
    assert_eq!(state.players[&actor].inventory.slots[0], before);
    assert!(batches.iter().filter(|(connection, _)| *connection == ConnectionId(1)).flat_map(|(_, changes)| changes).any(|change| matches!(change,
        PlayerChange::MenuContents { window: 0, slots, .. } if slots.get(36) == Some(&Some(before.clone()))
    )), "custody refusal republishes authoritative inventory");
}

#[test]
fn book_use_input_opens_only_written_books_in_the_requested_hand() {
    let (mut state, actor) = fixture();
    let use_book = |hand| PlayerInput::UseItem {
        connection: ConnectionId(1),
        hand,
        yaw: 0.0,
        pitch: 0.0,
        sequence: 1,
    };
    let (_, batches, _) = run_tick(&mut state, &[use_book(Hand::Main)]);
    assert!(
        !batches
            .iter()
            .flat_map(|(_, changes)| changes)
            .any(|change| matches!(change, PlayerChange::BookOpened { .. }))
    );
    run_tick(
        &mut state,
        &[PlayerInput::EditBook {
            connection: ConnectionId(1),
            slot: 0,
            pages: vec!["page".into()],
            title: Some("Title".into()),
        }],
    );
    let signed = state.players[&actor].inventory.slots[0].clone();
    state.players.get_mut(&actor).unwrap().inventory.slots[40] = signed;
    for hand in [Hand::Main, Hand::Off] {
        let (_, batches, _) = run_tick(&mut state, &[use_book(hand)]);
        assert!(batches.iter().filter(|(connection, _)| *connection == ConnectionId(1)).flat_map(|(_, changes)| changes).any(|change| matches!(change, PlayerChange::BookOpened { hand: opened } if *opened == hand)));
    }
}
