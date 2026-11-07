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
            id: format!("solaris:settlement/{action}"),
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
    use mc_script::{
        ScriptInventoryEndpoint, ScriptInventoryMaterial, ScriptInventoryResourcePlan,
        ScriptInventoryWorkPortion, ScriptOperation, ScriptOperationRequest,
        ScriptOwnedInventoryOperation,
    };
    let endpoint = ScriptInventoryEndpoint::PlayerInventory { player_id: CONN.0 };
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
        .position(|target| matches!(target, Target::Pick { worker, .. } if *worker == uuid))
        .expect("resident icon")
}

#[test]
fn residents_menu_assigns_a_free_certified_place_and_explains_refusals() {
    let mut state = joined();
    let (id, building) = farm(&mut state);
    let first = resident(&mut state, id, 5.5);
    let second = resident(&mut state, id, 6.5);
    let opened = dialog(&mut state, id, "residents_icons", &[]);
    assert!(title(&opened).contains("Residents · oak"));

    // Resident icon -> workplace icons -> assignment through the claim index.
    let icon = resident_slot(&state, first);
    press(&mut state, icon as i16, 0, ClickMode::Pickup);
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
    let icon = resident_slot(&state, second);
    press(&mut state, icon as i16, 0, ClickMode::Pickup);
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
        &[("b", Tag::String(building.0.to_string()))],
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
    assert_eq!(open(&state).view, View::Building { building });
    assert!(matches!(
        open(&state).targets[BUILDING_PLACES],
        Target::Pick { worker: w, from: Some(b) } if w == worker && b == building
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
