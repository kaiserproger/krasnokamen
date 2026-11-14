use super::*;
use crate::settlements::allocator::tests::{block, test_state, villager};
use crate::tick::run_tick;
use mc_script::ScriptWorldBounds;

const CONN: ConnectionId = ConnectionId(7);
/// The joined player's UUID (nil) as a settlement owner.
const PLAYER: &str = "00000000-0000-0000-0000-000000000000";

fn joined() -> TickState {
    let mut state = test_state();
    run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection: CONN,
            uuid: Uuid::nil(),
            name: "Mayor".into(),
            permission_level: 0,
        }],
    );
    state
}

fn town(state: &mut TickState, owner: &str) -> SettlementId {
    state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "oak".into(),
            owner.into(),
            [8, 201, 8],
            48,
        )
        .unwrap()
}

fn bounds() -> ScriptWorldBounds {
    ScriptWorldBounds::new([3, 200, 3], [7, 204, 7]).unwrap()
}

fn place(state: &mut TickState, at: [i32; 3], name: &str) {
    let id = block(state, name, &[]);
    state.world.set_block(at[0], at[1], at[2], id);
    state.world.fold_overlay();
}

fn item(state: &TickState, name: &str) -> u32 {
    state
        .registry
        .items
        .id_of(&Identifier::parse(name).unwrap())
        .unwrap()
}

fn chest(state: &mut TickState, at: [i32; 3], contents: &[(&str, i32)]) {
    place(state, at, "minecraft:chest");
    let mut slots = vec![ItemStack::EMPTY; 27];
    for (slot, (name, count)) in slots.iter_mut().zip(contents) {
        *slot = ItemStack::new(item(state, name), *count);
    }
    state.block_entities.insert(
        (at[0], at[1], at[2]),
        crate::tick::BlockEntityState {
            slots,
            furnace: None,
        },
    );
}

/// Two single chests certified as one storage building.
fn storage_town(state: &mut TickState, owner: &str) -> SettlementId {
    let id = town(state, owner);
    chest(
        state,
        [4, 201, 4],
        &[("minecraft:cobblestone", 40), ("minecraft:cobblestone", 30)],
    );
    chest(state, [6, 201, 4], &[("minecraft:bread", 5)]);
    buildings::certify(state, id, BuildingKind::Storage, bounds()).unwrap();
    id
}

fn run(state: &mut TickState, inputs: &[PlayerInput]) -> Vec<PlayerChange> {
    let (_, outbox, _) = run_tick(state, inputs);
    outbox
        .into_iter()
        .filter(|(connection, _)| *connection == CONN)
        .flat_map(|(_, changes)| changes)
        .collect()
}

fn dialog(
    state: &mut TickState,
    id: SettlementId,
    action: &str,
    extra: &[(&str, Tag)],
) -> Vec<PlayerChange> {
    let mut entries = vec![("s".to_owned(), Tag::String(id.0.to_string()))];
    entries.extend(extra.iter().map(|(k, v)| ((*k).to_owned(), v.clone())));
    run(
        state,
        &[PlayerInput::DialogClick {
            connection: CONN,
            id: format!("krasnokamen:settlement/{action}"),
            payload: Some(Tag::Compound(entries)),
        }],
    )
}

fn entity(state: &TickState) -> EntityId {
    state.connections.entity_of(CONN).unwrap()
}

/// Click the open window at its current revision.
fn press(state: &mut TickState, slot: i16, button: i8, mode: ClickMode) -> Vec<PlayerChange> {
    let menu = state.players[&entity(state)].open.clone().unwrap();
    run(
        state,
        &[PlayerInput::ContainerClick {
            connection: CONN,
            window: menu.window,
            state_id: menu.state_id,
            slot,
            button,
            mode,
        }],
    )
}

fn open(state: &TickState) -> &Open {
    &state.settlement_menus[&CONN]
}

fn title(changes: &[PlayerChange]) -> String {
    changes
        .iter()
        .rev()
        .find_map(|change| match change {
            PlayerChange::MenuOpened { title, .. } => Some(title.clone()),
            _ => None,
        })
        .expect("a menu opened")
}

fn said(changes: &[PlayerChange], text: &str) -> bool {
    chat(changes).iter().any(|line| line.contains(text))
}

fn chat(changes: &[PlayerChange]) -> Vec<String> {
    changes
        .iter()
        .filter_map(|change| match change {
            PlayerChange::Chat { message } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

fn stock_slot(state: &TickState, name: &str) -> usize {
    let wanted = item(state, name);
    open(state)
        .targets
        .iter()
        .position(|target| matches!(target, Target::Stock(template) if template.item_id == wanted))
        .expect("stock icon")
}

fn stored(state: &TickState, name: &str) -> i32 {
    let wanted = item(state, name);
    [[4, 201, 4], [6, 201, 4]]
        .iter()
        .flat_map(|at| &state.block_entities[&(at[0], at[1], at[2])].slots)
        .filter(|stack| stack.item_id == wanted && !stack.is_empty())
        .map(|stack| stack.count)
        .sum()
}

fn held(state: &TickState, name: &str) -> i32 {
    let wanted = item(state, name);
    let player = &state.players[&entity(state)];
    let cursor = player
        .open
        .as_ref()
        .filter(|menu| menu.carried.item_id == wanted && !menu.carried.is_empty())
        .map_or(0, |menu| menu.carried.count);
    player
        .inventory
        .slots
        .iter()
        .filter(|stack| stack.item_id == wanted && !stack.is_empty())
        .map(|stack| stack.count)
        .sum::<i32>()
        + cursor
}

#[test]
fn debug_menu_uses_settlement_icons_and_falls_through_stale_window() {
    let mut state = joined();
    let id = storage_town(&mut state, PLAYER);
    dialog(&mut state, id, "storage", &[]);
    let actor = entity(&state);
    let cobble = stock_slot(&state, "minecraft:cobblestone");
    let image = crate::tick::debug_menu_snapshot(&state, actor).unwrap();
    assert_eq!(image["slots"][cobble]["item"], "minecraft:cobblestone");
    assert_eq!(image["slots"][cobble]["count"], 70);

    let registry = std::sync::Arc::clone(&state.registry);
    let slots = state.block_entities[&(6, 201, 4)].slots.clone();
    state.players.get_mut(&actor).unwrap().open_menu(
        &registry.item_rules(),
        MenuKind::Chest { rows: 3 },
        "Chest".into(),
        &slots,
    );
    state.open_containers.insert(actor, (6, 201, 4));
    let image = crate::tick::debug_menu_snapshot(&state, actor).unwrap();
    assert_eq!(image["slots"][0]["item"], "minecraft:bread");
    assert_eq!(image["slots"][0]["count"], 5);
}

#[test]
fn storage_inventory_moves_real_stacks_both_ways_without_duplication() {
    let mut state = joined();
    let id = storage_town(&mut state, PLAYER);
    let opened = dialog(&mut state, id, "storage", &[]);
    assert!(
        title(&opened).contains("Storage · oak"),
        "{}",
        title(&opened)
    );
    let cobble = stock_slot(&state, "minecraft:cobblestone");
    assert_eq!(
        open(&state).icons[cobble].count,
        70,
        "one icon for both stacks"
    );
    let total = |state: &TickState| {
        stored(state, "minecraft:cobblestone") + held(state, "minecraft:cobblestone")
    };
    assert_eq!(total(&state), 70);

    // Shift: one full stack straight into the inventory.
    press(&mut state, cobble as i16, 0, ClickMode::QuickMove);
    assert_eq!(stored(&state, "minecraft:cobblestone"), 6);
    assert_eq!(held(&state, "minecraft:cobblestone"), 64);
    assert_eq!(total(&state), 70);

    // Click: the rest onto the cursor, then back onto the grid.
    let cobble = stock_slot(&state, "minecraft:cobblestone");
    press(&mut state, cobble as i16, 0, ClickMode::Pickup);
    assert_eq!(stored(&state, "minecraft:cobblestone"), 0);
    let carried = state.players[&entity(&state)]
        .open
        .as_ref()
        .unwrap()
        .carried
        .clone();
    assert_eq!(carried.count, 6);
    assert!(
        !open(&state)
            .targets
            .iter()
            .any(|target| matches!(target, Target::Stock(template) if template.item_id == carried.item_id)),
        "an emptied kind leaves the grid"
    );
    press(&mut state, 0, 0, ClickMode::Pickup);
    assert!(
        state.players[&entity(&state)]
            .open
            .as_ref()
            .unwrap()
            .carried
            .is_empty()
    );
    assert_eq!(stored(&state, "minecraft:cobblestone"), 6);
    assert_eq!(total(&state), 70);

    // Shift-click the player's own stack stores it again.
    let index = state.players[&entity(&state)]
        .inventory
        .slots
        .iter()
        .position(|stack| stack.count == 64)
        .unwrap();
    // Window order: main inventory 9..35, then the hotbar 0..8.
    let window_slot = SLOTS + if index < 9 { 27 + index } else { index - 9 };
    press(&mut state, window_slot as i16, 0, ClickMode::QuickMove);
    assert_eq!(stored(&state, "minecraft:cobblestone"), 70);
    assert_eq!(held(&state, "minecraft:cobblestone"), 0);
    assert_eq!(stored(&state, "minecraft:bread"), 5);
}

fn icon_name(state: &TickState, slot: usize) -> Tag {
    let fields =
        mc_data::item_nbt::stack_to_fields(&state.registry.items, &open(state).icons[slot])
            .unwrap();
    let Some((_, Tag::Compound(components))) =
        fields.into_iter().find(|(key, _)| key == "components")
    else {
        panic!("icon without components");
    };
    components
        .into_iter()
        .find(|(key, _)| key == CUSTOM_NAME_COMPONENT)
        .map(|(_, name)| name)
        .expect("icon custom name")
}

/// `name` as the decorated, non-italic display name `decorate` writes.
fn display_name(parts: Vec<Tag>) -> Tag {
    let label = Tag::Compound(vec![
        ("text".to_owned(), Tag::String(String::new())),
        ("extra".to_owned(), compounds(parts)),
    ]);
    Tag::Compound(vec![
        ("text".to_owned(), Tag::String(String::new())),
        ("italic".to_owned(), Tag::Byte(0)),
        ("extra".to_owned(), compounds(vec![label])),
    ])
}

/// Icon names reach the wire as whole `minecraft:custom_name` components: a
/// literal custom name showed its JSON as the item name in the real client.
#[test]
fn icon_names_are_text_components_never_literal_json() {
    let mut state = joined();
    let id = storage_town(&mut state, PLAYER);
    let mut loaf = ItemStack::new(item(&state, "minecraft:bread"), 2);
    loaf.custom_name = Some("Loaf".into());
    state.block_entities.get_mut(&(6, 201, 4)).unwrap().slots[1] = loaf;
    dialog(&mut state, id, "storage", &[]);
    let total = |text: &str| {
        Tag::Compound(vec![
            ("text".to_owned(), Tag::String(text.to_owned())),
            ("color".to_owned(), Tag::String("gray".to_owned())),
        ])
    };
    let cobble = stock_slot(&state, "minecraft:cobblestone");
    let translated = Tag::Compound(vec![(
        "translate".to_owned(),
        Tag::String("block.minecraft.cobblestone".to_owned()),
    )]);
    assert_eq!(
        icon_name(&state, cobble),
        display_name(vec![translated, total(" ×70")])
    );
    let icon = &open(&state).icons[cobble];
    assert!(
        icon.custom_name
            .as_deref()
            .is_none_or(|name| !name.contains('{')),
        "{:?}",
        icon.custom_name
    );
    assert!(
        icon.extended()
            .preserved_components
            .iter()
            .any(|component| component.name == CUSTOM_NAME_COMPONENT),
        "the wire copies the whole component, not the literal projection"
    );
    let named = open(&state)
        .targets
        .iter()
        .position(|target| {
            matches!(target, Target::Stock(template) if template.custom_name.as_deref() == Some("Loaf"))
        })
        .expect("renamed stock keeps its own icon");
    assert_eq!(
        icon_name(&state, named),
        display_name(vec![text("Loaf"), total(" ×2")])
    );
}

#[test]
fn view_only_full_inventory_and_stale_clicks_move_nothing() {
    let mut state = joined();
    let id = storage_town(&mut state, "someone-else");
    dialog(&mut state, id, "storage", &[]);
    let cobble = stock_slot(&state, "minecraft:cobblestone");
    let refused = press(&mut state, cobble as i16, 0, ClickMode::QuickMove);
    assert!(
        said(&refused, "Only the owner or an operator"),
        "{:?}",
        chat(&refused)
    );
    assert_eq!(stored(&state, "minecraft:cobblestone"), 70);

    let mut state = joined();
    let id = storage_town(&mut state, PLAYER);
    dialog(&mut state, id, "storage", &[]);
    let cobble = stock_slot(&state, "minecraft:cobblestone");
    // A click carrying an old revision only resyncs.
    let menu = state.players[&entity(&state)].open.clone().unwrap();
    run(
        &mut state,
        &[PlayerInput::ContainerClick {
            connection: CONN,
            window: menu.window,
            state_id: menu.state_id.wrapping_sub(1),
            slot: cobble as i16,
            button: 0,
            mode: ClickMode::QuickMove,
        }],
    );
    assert_eq!(stored(&state, "minecraft:cobblestone"), 70);
    // A full inventory refuses the move instead of dropping or duplicating.
    let dirt = item(&state, "minecraft:dirt");
    let player = entity(&state);
    for slot in 0..36 {
        state.players.get_mut(&player).unwrap().inventory.slots[slot] = ItemStack::new(dirt, 64);
    }
    let full = press(&mut state, cobble as i16, 0, ClickMode::QuickMove);
    assert!(
        chat(&full)
            .iter()
            .any(|line| line.contains("inventory is full")),
        "{:?}",
        chat(&full)
    );
    assert_eq!(stored(&state, "minecraft:cobblestone"), 70);
}

/// A plugin-inventory hold on the joined player's inventory (the same
/// operation production and construction holds use).
fn reserve(state: &mut TickState, name: &str, count: u64) {
    reserve_endpoint(
        state,
        name,
        count,
        mc_script::ScriptInventoryEndpoint::PlayerInventory { player_id: CONN.0 },
    );
}

fn reserve_endpoint(
    state: &mut TickState,
    name: &str,
    count: u64,
    endpoint: mc_script::ScriptInventoryEndpoint,
) {
    use mc_script::{
        ScriptInventoryMaterial, ScriptInventoryResourcePlan, ScriptInventoryWorkPortion,
        ScriptOperation, ScriptOperationRequest, ScriptOwnedInventoryOperation,
    };
    let operation_id = Uuid::new_v4().simple().to_string();
    let request = ScriptOperationRequest::try_new(
        &operation_id,
        ScriptOperation::Inventory {
            operation: ScriptOwnedInventoryOperation::Reserve {
                operation_id: operation_id.clone(),
                endpoint: endpoint.clone(),
                resource_plan: ScriptInventoryResourcePlan::new(vec![
                    ScriptInventoryWorkPortion::new(
                        1,
                        vec![ScriptInventoryMaterial::new(name.into(), count)],
                    ),
                ]),
                expected_revision: crate::plugin_inventories::load(state, "native-test", &endpoint)
                    .unwrap()
                    .fence,
            },
        },
    )
    .unwrap();
    let result = crate::plugin_inventories::execute(state, "native-test", &request);
    assert_eq!(result.failure(), None);
}

/// Window slot of player inventory slot `index` (main 9..35, then hotbar 0..8).
fn window_slot(index: usize) -> i16 {
    (SLOTS + if index < 9 { 27 + index } else { index - 9 }) as i16
}

fn own(state: &TickState, index: usize) -> ItemStack {
    state.players[&entity(state)].inventory.slots[index].clone()
}

fn cursor(state: &TickState) -> ItemStack {
    state.players[&entity(state)]
        .open
        .as_ref()
        .map_or(ItemStack::EMPTY, |menu| menu.carried.clone())
}

fn resynced(changes: &[PlayerChange]) -> bool {
    changes
        .iter()
        .any(|change| matches!(change, PlayerChange::MenuContents { .. }))
}

#[test]
fn reserved_own_stack_cannot_be_picked_up_thrown_or_stored_below_its_floor() {
    // A view-only visitor still moves its own items, but never a held unit.
    let mut state = joined();
    let id = storage_town(&mut state, "someone-else");
    let emerald = item(&state, "minecraft:emerald");
    let player = entity(&state);
    state.players.get_mut(&player).unwrap().inventory.slots[0] = ItemStack::new(emerald, 2);
    reserve(&mut state, "minecraft:emerald", 1);
    dialog(&mut state, id, "storage", &[]);

    let refused = press(&mut state, window_slot(0), 0, ClickMode::Pickup);
    assert!(resynced(&refused));
    assert_eq!(
        own(&state, 0).count,
        2,
        "picking up the stack would take the held unit"
    );
    assert!(cursor(&state).is_empty());

    // Throwing the excess unit is admitted; the held unit is not.
    press(&mut state, window_slot(0), 0, ClickMode::Throw);
    assert_eq!(own(&state, 0).count, 1);
    let refused = press(&mut state, window_slot(0), 0, ClickMode::Throw);
    assert!(resynced(&refused));
    assert_eq!(own(&state, 0).count, 1);
    let refused = press(&mut state, window_slot(0), 1, ClickMode::Throw);
    assert!(resynced(&refused));
    assert_eq!(own(&state, 0).count, 1);
    assert!(cursor(&state).is_empty());

    // The owner's shift-deposit goes through the fenced commit's floor check.
    let mut state = joined();
    let id = storage_town(&mut state, PLAYER);
    let player = entity(&state);
    state.players.get_mut(&player).unwrap().inventory.slots[0] = ItemStack::new(emerald, 2);
    reserve(&mut state, "minecraft:emerald", 2);
    dialog(&mut state, id, "storage", &[]);
    press(&mut state, window_slot(0), 0, ClickMode::QuickMove);
    assert_eq!(own(&state, 0).count, 2);
    assert_eq!(stored(&state, "minecraft:emerald"), 0);
    let refused = press(&mut state, window_slot(0), 0, ClickMode::Pickup);
    assert!(resynced(&refused));
    assert_eq!(own(&state, 0).count, 2);
    assert!(cursor(&state).is_empty());
}

#[test]
fn spectator_owner_browses_storage_but_every_move_only_resyncs() {
    let mut state = joined();
    let id = storage_town(&mut state, PLAYER);
    let player = entity(&state);
    let dirt = item(&state, "minecraft:dirt");
    let bread = item(&state, "minecraft:bread");
    state.players.get_mut(&player).unwrap().inventory.slots[0] = ItemStack::new(dirt, 10);
    state.players.get_mut(&player).unwrap().game_mode = crate::commands::GameMode::Spectator;
    let opened = dialog(&mut state, id, "storage", &[]);
    assert!(
        title(&opened).contains("Storage · oak"),
        "the menu opens for spectators"
    );
    let cobble = stock_slot(&state, "minecraft:cobblestone");

    for (button, mode) in [
        (0, ClickMode::QuickMove),
        (0, ClickMode::Pickup),
        (1, ClickMode::Pickup),
    ] {
        let refused = press(&mut state, cobble as i16, button, mode);
        assert!(resynced(&refused));
        assert!(!said(&refused, "Only the owner"), "{:?}", chat(&refused));
        assert_eq!(stored(&state, "minecraft:cobblestone"), 70);
        assert_eq!(held(&state, "minecraft:cobblestone"), 0);
    }
    // No deposit from the inventory or from a cursor stack either.
    let refused = press(&mut state, window_slot(0), 0, ClickMode::QuickMove);
    assert!(resynced(&refused));
    assert_eq!(own(&state, 0), ItemStack::new(dirt, 10));
    assert_eq!(stored(&state, "minecraft:dirt"), 0);
    for (slot, mode) in [
        (window_slot(0), ClickMode::Pickup),
        (window_slot(0), ClickMode::Throw),
    ] {
        press(&mut state, slot, 0, mode);
        assert_eq!(own(&state, 0), ItemStack::new(dirt, 10));
        assert!(cursor(&state).is_empty());
    }
    state
        .players
        .get_mut(&player)
        .unwrap()
        .open
        .as_mut()
        .unwrap()
        .carried = ItemStack::new(bread, 3);
    let empty = (0..PAGE)
        .find(|slot| matches!(open(&state).targets[*slot], Target::None))
        .unwrap();
    let refused = press(&mut state, empty as i16, 0, ClickMode::Pickup);
    assert!(resynced(&refused));
    assert_eq!(cursor(&state), ItemStack::new(bread, 3));
    assert_eq!(stored(&state, "minecraft:bread"), 5);
    state
        .players
        .get_mut(&player)
        .unwrap()
        .open
        .as_mut()
        .unwrap()
        .carried = ItemStack::EMPTY;

    // Navigation stays usable: refresh re-images, Back opens the dialog.
    let refresh = press(&mut state, INFO as i16, 0, ClickMode::Pickup);
    assert!(resynced(&refresh));
    assert_eq!(open(&state).view, View::Storage { page: 0 });

    // Control: the same click withdraws once the owner leaves spectator mode.
    state.players.get_mut(&player).unwrap().game_mode = crate::commands::GameMode::Survival;
    press(&mut state, cobble as i16, 0, ClickMode::QuickMove);
    assert_eq!(stored(&state, "minecraft:cobblestone"), 6);
    assert_eq!(held(&state, "minecraft:cobblestone"), 64);
    state.players.get_mut(&player).unwrap().game_mode = crate::commands::GameMode::Spectator;
    let back = press(&mut state, BACK as i16, 0, ClickMode::Pickup);
    assert!(
        back.iter()
            .any(|change| matches!(change, PlayerChange::ShowDialog { .. }))
    );
    assert!(state.settlement_menus.is_empty());
}

fn farm(state: &mut TickState) -> (SettlementId, BuildingId) {
    let id = town(state, PLAYER);
    place(state, [4, 201, 4], "minecraft:composter");
    let building = buildings::certify(
        state,
        id,
        BuildingKind::Workshop(Profession::Farmer),
        bounds(),
    )
    .unwrap();
    (id, building)
}

fn resident(state: &mut TickState, id: SettlementId, x: f64) -> Uuid {
    let slot = villager(state, [x, 201.0, 7.5]);
    let entity = state.entities.body(slot).id;
    crate::settlements::bind(state, id, entity).unwrap();
    state.entities.row(slot).uuid
}

fn villager_of(state: &mut TickState, uuid: Uuid) -> &mut crate::villager::VillagerState {
    let slot = state.entities.slot_of_uuid(uuid).unwrap();
    state.entities.row_mut(slot).villager.as_mut().unwrap()
}

fn job(state: &TickState, uuid: Uuid) -> Option<BlockPos> {
    let slot = state.entities.slot_of_uuid(uuid).unwrap();
    state.entities.row(slot).villager.as_ref().unwrap().job_site
}

fn resident_slot(state: &TickState, uuid: Uuid) -> usize {
    open(state)
        .targets
        .iter()
        .position(
            |target| matches!(target, Target::Show(View::Resident { worker }) if *worker == uuid),
        )
        .expect("resident icon")
}

#[test]
fn residents_menu_assigns_a_free_certified_place_and_explains_refusals() {
    let mut state = joined();
    let (id, building) = farm(&mut state);
    let first = resident(&mut state, id, 5.5);
    let second = resident(&mut state, id, 6.5);
    let opened = dialog(&mut state, id, "residents", &[]);
    assert!(title(&opened).contains("Residents / households"));

    // Resident icon -> workplace icons -> assignment through the claim index.
    let icon = resident_slot(&state, first);
    press(&mut state, icon as i16, 0, ClickMode::Pickup);
    assert!(matches!(open(&state).view, View::Resident { worker } if worker == first));
    press(&mut state, ACTION as i16, 0, ClickMode::Pickup);
    assert!(matches!(open(&state).view, View::Assign { worker, .. } if worker == first));
    let place = open(&state)
        .targets
        .iter()
        .position(|target| matches!(target, Target::Assign { building: b, .. } if *b == building))
        .unwrap();
    let done = press(&mut state, place as i16, 0, ClickMode::Pickup);
    assert!(said(&done, "is assigned to #1"), "{:?}", chat(&done));
    let point = BlockPos { x: 4, y: 201, z: 4 };
    assert_eq!(job(&state, first), Some(point));
    let other = state
        .entities
        .body(state.entities.slot_of_uuid(second).unwrap())
        .id;
    assert!(state.villager_pois.claimed(
        other,
        Poi {
            pos: point,
            kind: PoiKind::Job(Profession::Farmer),
        }
    ));

    // The single place is taken: the second resident is refused, unchanged.
    press(&mut state, BACK as i16, 0, ClickMode::Pickup);
    let icon = resident_slot(&state, second);
    press(&mut state, icon as i16, 0, ClickMode::Pickup);
    press(&mut state, ACTION as i16, 0, ClickMode::Pickup);
    let full = press(&mut state, place as i16, 0, ClickMode::Pickup);
    assert!(said(&full, "No free work place"), "{:?}", chat(&full));
    assert_eq!(job(&state, second), None);

    // A villager who traded keeps its craft; a nitwit never works.
    villager_of(&mut state, second).xp = 3;
    villager_of(&mut state, second).data.profession = Profession::Mason;
    let traded = press(&mut state, place as i16, 0, ClickMode::Pickup);
    assert!(said(&traded, "Has traded"), "{:?}", chat(&traded));
    villager_of(&mut state, second).xp = 0;
    villager_of(&mut state, second).data.profession = Profession::Nitwit;
    let nitwit = press(&mut state, place as i16, 0, ClickMode::Pickup);
    assert!(said(&nitwit, "nitwit"), "{:?}", chat(&nitwit));
    assert_eq!(job(&state, second), None);
    assert_eq!(job(&state, first), Some(point));
}

#[test]
fn map_click_opens_the_building_page_and_hires_into_its_free_place() {
    let mut state = joined();
    let (id, building) = farm(&mut state);
    let worker = resident(&mut state, id, 5.5);
    let opened = dialog(
        &mut state,
        id,
        "buildings",
        &[
            ("b", Tag::String(building.0.to_string())),
            ("from", Tag::String("map".to_owned())),
            ("mx", Tag::String("42".to_owned())),
            ("mz", Tag::String("-9".to_owned())),
            ("ms", Tag::String("2".to_owned())),
        ],
    );
    assert!(title(&opened).contains("#1"), "{}", title(&opened));
    assert!(matches!(
        open(&state).targets[BUILDING_PLACES],
        Target::Show(View::Hire { .. })
    ));
    press(&mut state, BUILDING_PLACES as i16, 0, ClickMode::Pickup);
    let candidate = resident_or_assign(&state, worker);
    press(&mut state, candidate as i16, 0, ClickMode::Pickup);
    assert_eq!(job(&state, worker), Some(BlockPos { x: 4, y: 201, z: 4 }));
    let origin = Origin::Map {
        centre: [42, -9],
        scale: 2,
    };
    assert_eq!(
        open(&state).view,
        View::BuildingContext { building, origin }
    );
    assert!(matches!(
        open(&state).targets[BUILDING_PLACES],
        Target::Pick { worker: w, from: Some((b, o)) } if w == worker && b == building && o == origin
    ));
    assert!(matches!(
        open(&state).targets[BACK],
        Target::MapView {
            centre: [42, -9],
            scale: 2
        }
    ));
}

fn resident_or_assign(state: &TickState, uuid: Uuid) -> usize {
    open(state)
        .targets
        .iter()
        .position(|target| matches!(target, Target::Assign { worker, .. } if *worker == uuid))
        .expect("hire candidate")
}

#[test]
fn back_closes_the_window_and_returns_the_cursor_before_the_dialog() {
    let mut state = joined();
    let id = storage_town(&mut state, PLAYER);
    dialog(&mut state, id, "storage", &[]);
    let bread = stock_slot(&state, "minecraft:bread");
    press(&mut state, bread as i16, 1, ClickMode::Pickup);
    assert_eq!(
        held(&state, "minecraft:bread") + stored(&state, "minecraft:bread"),
        5
    );
    let back = press(&mut state, BACK as i16, 0, ClickMode::Pickup);
    assert!(
        back.iter()
            .any(|change| matches!(change, PlayerChange::MenuClosed { .. }))
    );
    assert!(
        back.iter()
            .any(|change| matches!(change, PlayerChange::ShowDialog { .. }))
    );
    assert!(state.settlement_menus.is_empty());
    assert!(state.players[&entity(&state)].open.is_none());
    // The half stack taken onto the cursor is now in the inventory, not lost.
    assert_eq!(stored(&state, "minecraft:bread"), 2);
    assert_eq!(held(&state, "minecraft:bread"), 3);
}

fn metadata(screen: &Screen) -> String {
    screen
        .icons
        .iter()
        .flat_map(|stack| &stack.extended().preserved_components)
        .filter(|component| {
            matches!(
                component.name.as_str(),
                "minecraft:custom_name" | "minecraft:lore"
            )
        })
        .map(|component| {
            let mut bytes = component.value.as_slice();
            let tag = mc_nbt::read_network_value(&mut bytes).unwrap();
            assert!(bytes.is_empty());
            format!("{tag:?}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn resources_keep_component_distinct_stacks_and_native_reservation_floors() {
    let mut state = joined();
    let id = storage_town(&mut state, PLAYER);
    let named = decorate(
        &state,
        ItemStack::new(item(&state, "minecraft:cobblestone"), 7),
        text("Marked stone"),
        &[],
    );
    state.block_entities.get_mut(&(4, 201, 4)).unwrap().slots[2] = named.clone();
    let dimension = state.dimension.name().into();
    reserve_endpoint(
        &mut state,
        "minecraft:cobblestone",
        5,
        mc_script::ScriptInventoryEndpoint::Container {
            dimension,
            position: mc_script::ScriptBlockPosition::new(4, 201, 4),
        },
    );
    let snapshot = stores(&state, id);
    assert!(!snapshot.unavailable);
    let entries = stock(&snapshot);
    let stone = entries
        .iter()
        .filter(|entry| entry.template.item_id == named.item_id)
        .collect::<Vec<_>>();
    assert_eq!(stone.len(), 2, "component-distinct stacks must not merge");
    let ordinary = stone
        .iter()
        .find(|entry| !can_stack(&entry.template, &named))
        .unwrap();
    let marked = stone
        .iter()
        .find(|entry| can_stack(&entry.template, &named))
        .unwrap();
    assert_eq!((ordinary.total, ordinary.free), (70, 70));
    assert_eq!((marked.total, marked.free), (7, 2));
    let ctx = Ctx {
        id,
        manage: true,
        lang: Lang::En,
    };
    assert!(metadata(&resources_screen(&state, &ctx)).contains("77 / 5 / 72"));
    let reserved = resource_screen(&state, &ctx, 7, 0);
    assert_eq!(
        reserved
            .targets
            .iter()
            .filter(|target| matches!(target, Target::StockDetail(_)))
            .count(),
        1
    );
    assert!(metadata(&reserved).contains("7 / 5 / 2"));
}

#[test]
fn unreadable_storage_and_container_cap_are_unknown_not_zero_stock() {
    let mut state = joined();
    let id = storage_town(&mut state, PLAYER);
    state.block_entities.remove(&(6, 201, 4));
    let snapshot = stores(&state, id);
    assert!(snapshot.unavailable);
    assert_eq!(
        stock(&snapshot)
            .iter()
            .map(|entry| entry.total)
            .sum::<u64>(),
        70
    );
    let ctx = Ctx {
        id,
        manage: true,
        lang: Lang::En,
    };
    let summary = metadata(&resources_screen(&state, &ctx));
    assert!(summary.contains("Partial readable stock only; totals unknown."));
    assert!(!summary.contains("0 / 0 / 0"));
    let details = resource_screen(&state, &ctx, 6, 0);
    assert!(matches!(details.targets[INFO], Target::Dialog("storage")));
    assert!(metadata(&details).contains("totals unknown"));

    let mut capped = joined();
    let id = town(&mut capped, PLAYER);
    for index in 0..64 {
        chest(
            &mut capped,
            [index % 16, 201, index / 16],
            &[("minecraft:bread", 1)],
        );
    }
    buildings::certify(
        &mut capped,
        id,
        BuildingKind::Storage,
        ScriptWorldBounds::new([0, 200, 0], [15, 204, 3]).unwrap(),
    )
    .unwrap();
    // A single certificate owns at most 64 work points. Exceed the UI's
    // aggregate inspection budget with a second, independently certified store.
    chest(&mut capped, [1, 201, 8], &[("minecraft:bread", 1)]);
    buildings::certify(
        &mut capped,
        id,
        BuildingKind::Storage,
        ScriptWorldBounds::new([0, 200, 7], [2, 204, 9]).unwrap(),
    )
    .unwrap();
    assert_eq!(buildings::storage_points(&capped, id).len(), 65);
    let snapshot = stores(&capped, id);
    assert!(
        snapshot.unavailable,
        "a capped projection cannot claim completeness"
    );
    assert_eq!(snapshot.images.len(), MAX_CONTAINERS);
    let ctx = Ctx {
        id,
        manage: true,
        lang: Lang::En,
    };
    assert!(metadata(&resources_screen(&capped, &ctx)).contains("totals unknown"));
}

#[test]
fn seventy_residents_and_no_household_members_are_paged_without_lost_ids() {
    let mut state = joined();
    let id = town(&mut state, PLAYER);
    let mut expected = (0..70)
        .map(|_| resident(&mut state, id, 8.5))
        .collect::<Vec<_>>();
    expected.sort_unstable();
    assert!(
        people(&state, id)
            .iter()
            .all(|person| person.household.is_none())
    );
    dialog(&mut state, id, "residents", &[]);
    let mut seen = Vec::new();
    for page in 0..2 {
        assert_eq!(open(&state).view, View::Census { filter: 0, page });
        let members = open(&state).targets[9..45]
            .iter()
            .filter_map(|target| match target {
                Target::Show(View::Resident { worker }) => Some(*worker),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(members.len(), if page == 0 { 36 } else { 34 });
        seen.extend(members);
        if page == 0 {
            press(&mut state, NEXT as i16, 0, ClickMode::Pickup);
        }
    }
    seen.sort_unstable();
    assert_eq!(seen, expected);
    let worker = match open(&state).targets[9] {
        Target::Show(View::Resident { worker }) => worker,
        _ => panic!("resident head"),
    };
    press(&mut state, 9, 0, ClickMode::Pickup);
    assert_eq!(open(&state).view, View::Resident { worker });
    press(&mut state, 16, 0, ClickMode::Pickup);
    let mut members = Vec::new();
    for page in 0..3 {
        assert_eq!(open(&state).view, View::Household { id: None, page });
        members.extend(
            open(&state).targets[18..45]
                .iter()
                .filter_map(|target| match target {
                    Target::Show(View::Resident { worker }) => Some(*worker),
                    _ => None,
                }),
        );
        if page < 2 {
            press(&mut state, NEXT as i16, 0, ClickMode::Pickup);
        }
    }
    members.sort_unstable();
    assert_eq!(members, expected);
    press(&mut state, BACK as i16, 0, ClickMode::Pickup);
    assert_eq!(open(&state).view, View::Households { page: 0 });
    press(&mut state, BACK as i16, 0, ClickMode::Pickup);
    assert_eq!(open(&state).view, View::Census { filter: 0, page: 0 });
}

#[test]
fn visitor_resident_and_building_cards_have_no_assignment_actuators() {
    let mut state = joined();
    let (id, building) = farm(&mut state);
    let worker = resident(&mut state, id, 8.5);
    let ctx = Ctx {
        id,
        manage: false,
        lang: Lang::En,
    };
    let person = resident_screen(&state, &ctx, worker);
    assert!(person.targets.iter().all(|target| !matches!(
        target,
        Target::Pick { .. } | Target::Assign { .. } | Target::Show(View::Hire { .. })
    )));
    let building = building_context_screen(
        &state,
        &ctx,
        building,
        Origin::List {
            kind: None,
            page: 0,
        },
    );
    assert!(building.targets.iter().all(|target| !matches!(
        target,
        Target::Pick { .. } | Target::Assign { .. } | Target::Show(View::Hire { .. })
    )));
}

#[test]
fn unavailable_native_project_is_not_a_complete_bar_or_a_funded_map_action() {
    use crate::settlements::construction::tests::{fixture, fund, open_plan};
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    crate::settlements::construction::request_build(&mut state, id, "storage").unwrap();
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .project
        .as_mut()
        .unwrap()
        .structure = "gone-native-site".into();
    let ctx = Ctx {
        id,
        manage: true,
        lang: Lang::En,
    };
    let screen = project_screen(&state, &ctx, 0);
    assert!(metadata(&screen).contains("Project records unavailable; cause unknown."));
    assert!(screen.icons[18..27].iter().all(ItemStack::is_empty));
    assert!(matches!(screen.targets[ACTION], Target::Dialog("storage")));
}

#[test]
fn cached_icon_names_keep_numeric_ids_local_to_the_registry() {
    let mut state = joined();
    let original = icon(&state, Icon::Book, 1, text("Book"), &[]).item_id;
    let changed = original + 100_000;
    let items = mc_data::items::ItemRegistry::from_report(&[
        mc_data::items::ItemReport {
            id: Icon::Book.identifier().clone(),
            protocol_id: changed,
        },
        mc_data::items::ItemReport {
            id: Icon::Paper.identifier().clone(),
            protocol_id: changed + 1,
        },
    ]);
    std::sync::Arc::make_mut(&mut state.registry).items = std::sync::Arc::new(items);
    assert_eq!(
        icon(&state, Icon::Book, 1, text("Book"), &[]).item_id,
        changed
    );
    assert!(std::ptr::eq(
        Icon::Book.identifier(),
        Icon::Book.identifier()
    ));
}

#[test]
fn certificate_and_request_policies_are_multiline_lore_in_both_locales() {
    let mut state = joined();
    let id = storage_town(&mut state, PLAYER);
    let building = *state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .keys()
        .next()
        .unwrap();
    for lang in [Lang::En, Lang::Ru] {
        let ctx = Ctx {
            id,
            manage: true,
            lang,
        };
        let screen = building_context_screen(
            &state,
            &ctx,
            building,
            Origin::List {
                kind: None,
                page: 0,
            },
        );
        for (slot, policy) in [
            (BUILDING_UPGRADE, Key::CertificatePolicy),
            (ACTION, Key::RequestPolicy),
            (INFO, Key::CertificatePolicy),
        ] {
            let component = screen.icons[slot]
                .extended()
                .preserved_components
                .iter()
                .find(|component| component.name == "minecraft:lore")
                .expect("policy is lore, not an oversized item name");
            let mut bytes = component.value.as_slice();
            let Tag::List(lore) = mc_nbt::read_network_value(&mut bytes).unwrap() else {
                panic!("expected encoded lore list");
            };
            assert!(bytes.is_empty());
            let lines = lore
                .elements
                .iter()
                .map(|line| {
                    let Tag::Compound(fields) = line else {
                        panic!("expected text component")
                    };
                    fields
                        .iter()
                        .find_map(|(key, value)| {
                            if key == "text"
                                && let Tag::String(text) = value
                            {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .unwrap()
                })
                .collect::<Vec<_>>();
            assert_eq!(lines.len(), 4, "slot {slot}: {lines:?}");
            assert_eq!(lines.join("\n"), ctx.t(policy));
        }
    }
}
