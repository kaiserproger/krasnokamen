use super::*;
use crate::boundary::{ConnectionId, Hand, PlayerChange, PlayerInput};

const POS: BlockPos = BlockPos { x: 4, y: 64, z: 4 };

fn open(state: &mut TickState, connection: ConnectionId) {
    let actor = state.connections.entity_of(connection).unwrap();
    let slot = state.entities.slot_of(actor).unwrap();
    state.entities.set_pos(slot, [4.5, 64.0, 2.5]);
    crate::tick::run_tick(
        state,
        &[PlayerInput::UseItemOn {
            connection,
            hand: Hand::Main,
            position: POS,
            face: 1,
            cursor: [0.5; 3],
            sequence: 1,
        }],
    );
    assert!(
        state.players[&actor].open.is_some(),
        "real UseItemOn must open the menu"
    );
}

fn properties(state: &TickState, connection: ConnectionId) -> Vec<(i16, i16)> {
    state
        .outbox
        .get(&connection)
        .into_iter()
        .flatten()
        .filter_map(|change| {
            if let PlayerChange::MenuProperty {
                property, value, ..
            } = change
            {
                Some((*property, *value))
            } else {
                None
            }
        })
        .collect()
}

fn song_events(state: &TickState) -> Vec<i32> {
    state
        .outbox
        .values()
        .flatten()
        .filter_map(|change| {
            if let PlayerChange::ProjectileLevelEvent {
                position, event, ..
            } = change
                && *position == POS
                && matches!(event, 1010 | 1011)
            {
                Some(*event)
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn data_owner_jukebox_clock_publishes_exact_playback_transitions() {
    let (mut state, _, context) = fixture();
    run(&mut state, context, "setblock 4 64 4 minecraft:jukebox");
    state.outbox.clear();
    mutate(&mut state, context,
        "data merge block 4 64 4 {RecordItem:{id:'minecraft:music_disc_cat',count:1},ticks_since_song_started:0L}").unwrap();
    assert!(crate::jukebox::playing(&state, POS));
    assert_eq!(song_events(&state), [1010]);
    assert_eq!(
        state.world.block_at(4, 64, 4),
        crate::bees::property(&state, POS, "has_record", "true")
    );
    state.outbox.clear();
    mutate(
        &mut state,
        context,
        "data merge block 4 64 4 {CustomName:'unchanged song'}",
    )
    .unwrap();
    assert!(song_events(&state).is_empty());
    mutate(
        &mut state,
        context,
        "data remove block 4 64 4 ticks_since_song_started",
    )
    .unwrap();
    assert!(!crate::jukebox::playing(&state, POS));
    assert_eq!(song_events(&state), [1011]);
    state.outbox.clear();
    mutate(
        &mut state,
        context,
        "data modify block 4 64 4 ticks_since_song_started set value 0L",
    )
    .unwrap();
    assert!(crate::jukebox::playing(&state, POS));
    assert_eq!(song_events(&state), [1010]);
    state.outbox.clear();
    let before = block(&state, POS).unwrap();
    let writes = state.opaque_changes.len();
    assert!(mutate(&mut state, context,
        "data merge block 4 64 4 {RecordItem:{id:'minecraft:stone',count:1},ticks_since_song_started:12L}").is_err());
    assert_eq!(block(&state, POS).unwrap(), before);
    assert_eq!(state.opaque_changes.len(), writes);
    assert!(song_events(&state).is_empty());
    mutate(&mut state, context, "data remove block 4 64 4 RecordItem").unwrap();
    assert!(!crate::jukebox::playing(&state, POS));
    assert_eq!(song_events(&state), [1011]);
    assert_eq!(
        state.world.block_at(4, 64, 4),
        crate::bees::property(&state, POS, "has_record", "false")
    );
}

#[test]
fn data_owner_lectern_updates_both_open_readers_and_clamps_page_atomically() {
    let (mut state, actor, context) = fixture();
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    let book = item(&state, "minecraft:writable_book", 1);
    state.players.get_mut(&actor).unwrap().inventory.slots[0] = book;
    crate::tick::run_tick(
        &mut state,
        &[
            PlayerInput::EditBook {
                connection: ConnectionId(1),
                slot: 0,
                pages: vec!["first".into(), "second".into(), "third".into()],
                title: Some("Book".into()),
            },
            PlayerInput::Join {
                connection: ConnectionId(2),
                uuid: uuid::Uuid::new_v4(),
                name: "reader2".into(),
                permission_level: 2,
            },
        ],
    );
    run(&mut state, context, "setblock 4 64 4 minecraft:lectern");
    mutate(
        &mut state,
        context,
        "data modify block 4 64 4 Book set from entity @s Inventory[{Slot:0b}]",
    )
    .unwrap();
    assert_eq!(
        state.world.block_at(4, 64, 4),
        crate::bees::property(&state, POS, "has_book", "true")
    );
    open(&mut state, ConnectionId(1));
    open(&mut state, ConnectionId(2));
    let readers: Vec<_> = [ConnectionId(1), ConnectionId(2)]
        .into_iter()
        .map(|connection| {
            let actor = state.connections.entity_of(connection).unwrap();
            let player = &state.players[&actor];
            let menu = player.open.as_ref().unwrap();
            (
                actor,
                menu.window,
                menu.carried.clone(),
                player.inventory.slots.clone(),
            )
        })
        .collect();
    state.outbox.clear();
    mutate(&mut state, context, "data merge block 4 64 4 {Page:99}").unwrap();
    assert_eq!(
        query(&mut state, context, "data get block 4 64 4 Page").unwrap(),
        2
    );
    for connection in [ConnectionId(1), ConnectionId(2)] {
        assert_eq!(properties(&state, connection), [(0, 2)]);
    }
    for (actor, window, cursor, slots) in readers {
        let player = &state.players[&actor];
        let menu = player.open.as_ref().unwrap();
        assert_eq!(menu.window, window);
        assert_eq!(menu.carried, cursor);
        assert_eq!(player.inventory.slots, slots);
    }
    state.outbox.clear();
    let before = block(&state, POS).unwrap();
    let writes = state.opaque_changes.len();
    assert!(
        mutate(
            &mut state,
            context,
            "data merge block 4 64 4 {Page:0,Book:1}"
        )
        .is_err()
    );
    assert_eq!(block(&state, POS).unwrap(), before);
    assert_eq!(state.opaque_changes.len(), writes);
    assert!(state.outbox.is_empty());
}

#[test]
fn data_owner_idle_brewing_publishes_properties_without_reopening_or_brewing() {
    let (mut state, actor, context) = fixture();
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    run(
        &mut state,
        context,
        "setblock 4 64 4 minecraft:brewing_stand",
    );
    open(&mut state, ConnectionId(1));
    let window = state.players[&actor].open.as_ref().unwrap().window;
    let cursor = item(&state, "minecraft:diamond", 3);
    state
        .players
        .get_mut(&actor)
        .unwrap()
        .open
        .as_mut()
        .unwrap()
        .carried = cursor.clone();
    let slots = state.block_entities[&(4, 64, 4)].slots.clone();
    state.outbox.clear();
    mutate(&mut state, context, "data merge block 4 64 4 {Fuel:20b}").unwrap();
    assert_eq!(properties(&state, ConnectionId(1)), [(0, 0), (1, 20)]);
    assert_eq!(state.brewing[&(4, 64, 4)].fuel, 20);
    assert_eq!(state.block_entities[&(4, 64, 4)].slots, slots);
    let menu = state.players[&actor].open.as_ref().unwrap();
    assert_eq!(menu.window, window);
    assert_eq!(menu.carried, cursor);
    state.outbox.clear();
    let before = block(&state, POS).unwrap();
    let native = state.brewing[&(4, 64, 4)].clone();
    let writes = state.opaque_changes.len();
    assert!(
        mutate(
            &mut state,
            context,
            "data merge block 4 64 4 {Fuel:5b,BrewTime:'invalid'}"
        )
        .is_err()
    );
    assert_eq!(state.brewing[&(4, 64, 4)], native);
    assert_eq!(block(&state, POS).unwrap(), before);
    assert_eq!(state.opaque_changes.len(), writes);
    assert!(state.outbox.is_empty());
}
