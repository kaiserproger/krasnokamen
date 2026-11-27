use super::*;
use crate::boundary::{Hand, PlayerChange, PlayerInput};
use crate::tick::run_tick;
use mc_data::Identifier;
use mc_inventory::ClickMode;

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
fn fixture() -> (TickState, EntityId, EntityId) {
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
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    for index in 1..=2 {
        run_tick(
            &mut state,
            &[PlayerInput::Join {
                connection: ConnectionId(index),
                uuid: uuid::Uuid::new_v4(),
                name: format!("owner{index}"),
                permission_level: 0,
            }],
        );
    }
    state.world.ensure_blocking(&state.desired_chunks());
    let chest = state
        .registry
        .blocks
        .block(&Identifier::parse("minecraft:ender_chest").unwrap())
        .unwrap()
        .default
        .0;
    state.world.set_block(8, 65, 8, chest);
    state.world.set_block(9, 65, 8, chest);
    state.world.fold_overlay();
    let first = state.connections.entity_of(ConnectionId(1)).unwrap();
    let second = state.connections.entity_of(ConnectionId(2)).unwrap();
    (state, first, second)
}
fn open_at(state: &mut TickState, connection: u64, x: i32) {
    run_tick(
        state,
        &[PlayerInput::UseItemOn {
            connection: ConnectionId(connection),
            hand: Hand::Main,
            position: BlockPos { x, y: 65, z: 8 },
            face: 1,
            cursor: [0.5, 1.0, 0.5],
            sequence: 1,
        }],
    );
}
#[test]
fn ender_chest_same_player_shares_chests_but_never_other_players_or_block_storage() {
    let (mut state, first, second) = fixture();
    let item = ItemStack::new(
        state
            .registry
            .items
            .id_of(&Identifier::parse("minecraft:diamond").unwrap())
            .unwrap(),
        7,
    );
    open_at(&mut state, 1, 8);
    state
        .players
        .get_mut(&first)
        .unwrap()
        .open
        .as_mut()
        .unwrap()
        .carried = item.clone();
    let menu = state.players[&first].open.as_ref().expect("personal menu");
    let input = PlayerInput::ContainerClick {
        connection: ConnectionId(1),
        window: menu.window,
        state_id: menu.state_id,
        slot: 0,
        button: 0,
        mode: ClickMode::Pickup,
    };
    run_tick(&mut state, &[input]);
    assert_eq!(
        contents(
            &state.players[&first].opaque_player_fields,
            &state.registry.items
        )
        .unwrap()[0],
        item
    );
    let image = crate::tick::debug_menu_snapshot(&state, first).unwrap();
    assert_eq!(image["slots"][0]["item"], "minecraft:diamond");
    assert_eq!(image["slots"][0]["count"], 7);
    open_at(&mut state, 2, 8);
    assert!(
        contents(
            &state.players[&second].opaque_player_fields,
            &state.registry.items
        )
        .unwrap()
        .iter()
        .all(ItemStack::is_empty)
    );
    open_at(&mut state, 1, 9);
    assert_eq!(
        contents(
            &state.players[&first].opaque_player_fields,
            &state.registry.items
        )
        .unwrap()[0],
        item
    );
    assert!(!state.block_entities.contains_key(&(8, 65, 8)));
    assert!(!state.block_entities.contains_key(&(9, 65, 8)));
    state.world.set_block(8, 65, 8, state.registry.states.air);
    state.world.set_block(9, 65, 8, state.registry.states.air);
    assert_eq!(
        contents(
            &state.players[&first].opaque_player_fields,
            &state.registry.items
        )
        .unwrap()[0],
        item
    );
    let registry = Arc::clone(&state.registry);
    let player = state.players.get_mut(&first).unwrap();
    let death = player.die(&registry.item_rules(), false, false);
    assert!(
        death
            .dropped
            .iter()
            .all(|stack| stack.item_id != item.item_id)
    );
    player.respawn([8.0, 65.0, 8.0], 0.0, 0.0, Default::default());
    assert_eq!(
        contents(&player.opaque_player_fields, &registry.items).unwrap()[0],
        item
    );
    let record = crate::core::player_record_for(&state, first, state.tick).unwrap();
    let tag = crate::journal::player_to_tag(&record, &state.registry.items).unwrap();
    let recovered = crate::journal::player_from_tag(&tag, &state.registry.items).unwrap();
    assert_eq!(
        contents(&recovered.opaque_player_fields, &state.registry.items).unwrap()[0],
        item
    );
}
#[test]
fn ender_chest_rejects_corrupt_slots_and_preserves_unknown_fields() {
    let items = mc_data::items::krasnokamen_required_items();
    let mut original = Vec::new();
    mc_nbt::write_named(
        &mut original,
        "",
        &Tag::Compound(vec![("CustomOwnerField".into(), Tag::Long(9))]),
    )
    .unwrap();
    let slots = vec![ItemStack::EMPTY; 27];
    let encoded = store(&original, &slots, &items).unwrap();
    assert!(
        root(&encoded)
            .unwrap()
            .contains(&("CustomOwnerField".into(), Tag::Long(9)))
    );
    assert_eq!(contents(&encoded, &items).unwrap(), slots);
    for bad in [
        Tag::Int(3),
        Tag::List(ListTag {
            element_type: tag_type::COMPOUND,
            elements: vec![Tag::Compound(vec![("Slot".into(), Tag::Byte(27))])],
        }),
    ] {
        let mut encoded = Vec::new();
        mc_nbt::write_named(
            &mut encoded,
            "",
            &Tag::Compound(vec![("EnderItems".into(), bad)]),
        )
        .unwrap();
        assert!(contents(&encoded, &items).is_err());
    }
}

/// Open the ender chest at `8 65 8` for connection 1 and evict its chunk, so the
/// menu coordinate is cold while the overlay still answers `block_at`.
fn cold_ender_menu(state: &mut TickState) -> u8 {
    open_at(state, 1, 8);
    let chunk = crate::boundary::ChunkPos { x: 0, z: 0 };
    state.world.finish_inhabited_tick(state.tick + 1);
    state
        .world
        .set_active_chunks(Default::default(), state.tick + 1);
    state.world.capture_dirty(&Default::default());
    state
        .world
        .mark_flushed(&[(chunk, state.world.storage_revision(chunk))]);
    assert!(
        state
            .world
            .evict_outside(&Default::default())
            .contains(&chunk)
    );
    assert!(!state.world.has_chunk(0, 0));
    state
        .players
        .values()
        .find_map(|player| player.open.as_ref().map(|menu| menu.window))
        .expect("the menu survives eviction")
}

#[test]
fn ender_chest_slot_state_with_a_cold_menu_closes_and_keeps_cursor_custody() {
    let (mut state, first, _second) = fixture();
    let window = cold_ender_menu(&mut state);
    let carried = ItemStack::new(
        state
            .registry
            .items
            .id_of(&Identifier::parse("minecraft:diamond").unwrap())
            .unwrap(),
        5,
    );
    state
        .players
        .get_mut(&first)
        .unwrap()
        .open
        .as_mut()
        .unwrap()
        .carried = carried.clone();
    let drains = run_tick(
        &mut state,
        &[PlayerInput::ContainerSlotState {
            connection: ConnectionId(1),
            window,
            slot: 1,
            enabled: false,
        }],
    )
    .1;
    assert!(
        state.players[&first].open.is_none(),
        "slot-state: a cold menu source closes the menu"
    );
    assert!(!state.open_containers.contains(&first));
    assert!(
        drains
            .iter()
            .flat_map(|(_, queue)| queue)
            .any(|change| matches!(
                change,
                PlayerChange::MenuClosed { window: closed } if *closed == window
            ))
    );
    assert!(
        state.players[&first].inventory.slots.contains(&carried),
        "the cursor item returns to the inventory"
    );
}

#[test]
fn ender_chest_button_with_a_cold_menu_closes_and_keeps_cursor_custody() {
    let (mut state, first, _second) = fixture();
    let window = cold_ender_menu(&mut state);
    let carried = ItemStack::new(
        state
            .registry
            .items
            .id_of(&Identifier::parse("minecraft:diamond").unwrap())
            .unwrap(),
        5,
    );
    state
        .players
        .get_mut(&first)
        .unwrap()
        .open
        .as_mut()
        .unwrap()
        .carried = carried.clone();
    let drains = run_tick(
        &mut state,
        &[PlayerInput::ContainerButton {
            connection: ConnectionId(1),
            window,
            button: 0,
        }],
    )
    .1;
    assert!(
        state.players[&first].open.is_none(),
        "button: a cold menu source closes the menu"
    );
    assert!(!state.open_containers.contains(&first));
    assert!(
        drains
            .iter()
            .flat_map(|(_, queue)| queue)
            .any(|change| matches!(
                change,
                PlayerChange::MenuClosed { window: closed } if *closed == window
            ))
    );
    assert!(
        state.players[&first].inventory.slots.contains(&carried),
        "the cursor item returns to the inventory"
    );
}

#[test]
fn ender_chest_slot_state_for_a_despawned_player_leaves_the_cold_menu_alone() {
    let (mut state, first, _second) = fixture();
    let window = cold_ender_menu(&mut state);
    // The connection still maps to the player while its entity row is gone: the
    // crafter slot-state path must not be reached (it would close the ghost menu
    // through `slot_of(..).expect("native menu owner")`).
    let slot = state.entities.slot_of(first).expect("joined player");
    state.entities.remove(slot);
    assert!(state.entities.slot_of(first).is_none());
    let drains = run_tick(
        &mut state,
        &[PlayerInput::ContainerSlotState {
            connection: ConnectionId(1),
            window,
            slot: 1,
            enabled: false,
        }],
    )
    .1;
    assert!(
        state.players[&first].open.is_some(),
        "a ghost owner's slot-state input is ignored"
    );
    assert!(
        !drains
            .iter()
            .flat_map(|(_, queue)| queue)
            .any(|change| matches!(change, PlayerChange::MenuClosed { .. }))
    );
}

#[test]
fn ender_chest_click_with_a_cold_menu_coordinate_closes_and_keeps_cursor_custody() {
    let (mut state, first, _second) = fixture();
    let chest = state
        .registry
        .blocks
        .block(&Identifier::parse("minecraft:ender_chest").unwrap())
        .unwrap()
        .default
        .0;
    open_at(&mut state, 1, 8);
    let carried = ItemStack::new(
        state
            .registry
            .items
            .id_of(&Identifier::parse("minecraft:diamond").unwrap())
            .unwrap(),
        5,
    );
    state
        .players
        .get_mut(&first)
        .unwrap()
        .open
        .as_mut()
        .unwrap()
        .carried = carried.clone();
    let chunk = crate::boundary::ChunkPos { x: 0, z: 0 };
    state.world.finish_inhabited_tick(state.tick + 1);
    state
        .world
        .set_active_chunks(Default::default(), state.tick + 1);
    state.world.capture_dirty(&Default::default());
    state
        .world
        .mark_flushed(&[(chunk, state.world.storage_revision(chunk))]);
    assert!(
        state
            .world
            .evict_outside(&Default::default())
            .contains(&chunk)
    );
    assert!(!state.world.has_chunk(0, 0));
    // The cold overlay still answers `block_at` with an ender chest, which is
    // exactly what must not select the handler.
    state.world.set_block(8, 65, 8, chest);
    assert_eq!(state.world.block_at(8, 65, 8), Some(chest));
    let menu = state.players[&first].open.as_ref().expect("menu survives");
    let (window, state_id) = (menu.window, menu.state_id);
    let drains = run_tick(
        &mut state,
        &[PlayerInput::ContainerClick {
            connection: ConnectionId(1),
            window,
            state_id,
            slot: 0,
            button: 0,
            mode: ClickMode::Pickup,
        }],
    )
    .1;
    assert!(
        state.players[&first].open.is_none(),
        "a cold menu source closes the menu"
    );
    assert!(!state.open_containers.contains(&first));
    assert!(
        drains
            .iter()
            .flat_map(|(_, queue)| queue)
            .any(|change| matches!(
                change,
                PlayerChange::MenuClosed { window: closed } if *closed == window
            ))
    );
    assert!(
        contents(
            &state.players[&first].opaque_player_fields,
            &state.registry.items
        )
        .unwrap()
        .iter()
        .all(ItemStack::is_empty),
        "no click reaches personal ender storage"
    );
    assert!(
        state.players[&first].inventory.slots.contains(&carried),
        "the cursor item returns to the inventory"
    );
}
