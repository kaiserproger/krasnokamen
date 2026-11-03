//! Icon chest menus for settlement management (issue #1, item 7): the
//! settlement's certified storage as one inventory, and residents and
//! buildings as icons with assignment of a resident to a certified workplace.
//!
//! A menu is a server-owned 9x6 chest window. Its 45 upper slots and the
//! bottom navigation row are display copies that never enter custody; the
//! player's own inventory sits below as in any chest. [`input`] consumes every
//! packet for that window before vanilla menu handling. Each click re-resolves
//! the settlement and permission (record owner or operator, as for the
//! dialogs) and acts on live tick state, never on the icons:
//! - storage moves real stacks through the canonical fenced inventory commit
//!   ([`inventory::commit`]) between the settlement's storage containers and
//!   the clicking player's own inventory or cursor. Reservation floors
//!   (production and construction holds) are never withdrawn, and a refused
//!   commit changes nothing. Moves inside the player's own inventory pass the
//!   tick's reservation admission, and spectators only get a resync for every
//!   stock or inventory move (pages, refresh and navigation still work);
//! - assignment points the resident's job site at a free certified work point
//!   through the allocator's claim authority. The resident takes the
//!   profession on reaching the workstation (vanilla job acquisition), so a
//!   villager who has traded keeps its profession, and builders, soldiers,
//!   nitwits and children are refused.

use std::collections::{BTreeMap, BTreeSet};

use mc_data::inventory_semantics_26_1_2::can_stack;
use mc_data::item_nbt::CUSTOM_NAME_COMPONENT;
use mc_data::{Identifier, ItemStack};
use mc_nbt::Tag;
use serde_json::json;
use uuid::Uuid;

use super::Ctx;
use crate::boundary::{BlockPos, ClickMode, ConnectionId, EntityId, PlayerChange, PlayerInput};
use crate::dialog::ClickPayload;
use crate::inventory::MenuKind;
use crate::settlements::SettlementId;
use crate::settlements::allocator::{Poi, PoiKind};
use crate::settlements::buildings::{self, Building, BuildingId, BuildingKind};
use crate::settlements::inventory;
use crate::settlements::ui_text::{self, Key, Lang};
use crate::tick::TickState;
use crate::villager::VillagerProfession as Profession;

const ROWS: u8 = 6;
const SLOTS: usize = 54;
/// Icons per page: the five upper rows.
const PAGE: usize = 45;
const PLAYER_SLOTS: usize = 36;
const BACK: usize = 45;
const PREVIOUS: usize = 48;
const INFO: usize = 49;
const NEXT: usize = 50;
const ACTION: usize = 52;
const LIST: usize = 53;
/// The building page: its icon, the upgrade/storage buttons, then work places.
const BUILDING_ICON: usize = 4;
const BUILDING_UPGRADE: usize = 6;
const BUILDING_STORAGE: usize = 2;
const BUILDING_PLACES: usize = 18;
/// Same bound as the storage summary on the main page.
const MAX_CONTAINERS: usize = 64;
const MAX_LORE: usize = 10;

/// One open icon menu and what each of its slots showed at the last render.
/// Clicks resolve against these targets, never against a re-derived order.
pub(crate) struct Open {
    window: u8,
    settlement: SettlementId,
    view: View,
    title: String,
    icons: Vec<ItemStack>,
    targets: Vec<Target>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Storage {
        page: usize,
    },
    Residents {
        page: usize,
    },
    Buildings {
        page: usize,
    },
    Building {
        building: BuildingId,
    },
    /// Workplaces for one resident; `from` returns to that building's page.
    Assign {
        worker: Uuid,
        from: Option<BuildingId>,
        page: usize,
    },
    /// Residents who may take a job in `building`.
    Hire {
        building: BuildingId,
        page: usize,
    },
}

#[derive(Debug, Clone)]
enum Target {
    None,
    Refresh,
    /// One kind of stored item (a count-1 template).
    Stock(ItemStack),
    Show(View),
    /// Close the menu and open this settlement dialog page.
    Dialog(&'static str),
    Pick {
        worker: Uuid,
        from: Option<BuildingId>,
    },
    Assign {
        worker: Uuid,
        building: BuildingId,
    },
    Upgrade(BuildingId),
}

/// One rendered window: 54 icons and what clicking each does.
struct Screen {
    title: String,
    icons: Vec<ItemStack>,
    targets: Vec<Target>,
}

impl Screen {
    fn new(title: String) -> Self {
        Self {
            title,
            icons: vec![ItemStack::EMPTY; SLOTS],
            targets: vec![Target::None; SLOTS],
        }
    }

    fn put(&mut self, slot: usize, icon: ItemStack, target: Target) {
        self.icons[slot] = icon;
        self.targets[slot] = target;
    }
}

// ---------------------------------------------------------------- entry points

/// The settlement's storage containers as one inventory.
pub(super) fn storage(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    show(state, connection, ctx, View::Storage { page: 0 });
}

/// Residents as icons; a click chooses the resident's workplace.
pub(super) fn residents(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    show(state, connection, ctx, View::Residents { page: 0 });
}

/// Certified buildings as icons; a click opens the building's page.
pub(super) fn buildings(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    show(state, connection, ctx, View::Buildings { page: 0 });
}

/// One building's page: upgrade, workers and free work places.
pub(super) fn building(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    building: BuildingId,
) {
    show(state, connection, ctx, View::Building { building });
}

pub(super) fn storage_label(ctx: &Ctx) -> &'static str {
    tr(ctx, "Storage", "Склад")
}

pub(super) fn assign_label(ctx: &Ctx) -> &'static str {
    tr(ctx, "Assign crafts", "Ремёсла")
}

pub(super) fn icons_label(ctx: &Ctx) -> &'static str {
    tr(ctx, "Icons", "Иконки")
}

/// Consume every packet for an open settlement icon menu before vanilla menu
/// handling can move its display icons. `Leave` only forgets the menu.
pub(crate) fn input(state: &mut TickState, input: &PlayerInput) -> bool {
    let connection = input.connection();
    if matches!(input, PlayerInput::Leave { .. }) {
        state.settlement_menus.remove(&connection);
        return false;
    }
    let Some(window) = state
        .settlement_menus
        .get(&connection)
        .map(|open| open.window)
    else {
        return false;
    };
    let Some(entity) = state.connections.entity_of(connection) else {
        return false;
    };
    if state
        .players
        .get(&entity)
        .and_then(|player| player.open.as_ref())
        .is_none_or(|menu| menu.window != window)
    {
        // Another menu replaced this one (or the player changed dimension).
        state.settlement_menus.remove(&connection);
        return false;
    }
    match input {
        PlayerInput::ContainerClose { .. } => close(state, connection, entity, false),
        PlayerInput::ContainerClick {
            window: clicked,
            state_id,
            slot,
            button,
            mode,
            ..
        } => {
            // A click naming another window is stale: vanilla ignores it.
            if *clicked == window {
                click(state, connection, entity, *state_id, *slot, *button, *mode);
            }
        }
        PlayerInput::ContainerButton { .. }
        | PlayerInput::PlaceRecipe { .. }
        | PlayerInput::RenameItem { .. }
        | PlayerInput::ContainerSlotState { .. }
        | PlayerInput::SelectTrade { .. } => {}
        _ => return false,
    }
    true
}

// --------------------------------------------------------------------- windows

/// Render `view` and show it: the open window is re-imaged when the title is
/// unchanged (paging, refresh), otherwise a new window opens.
fn show(state: &mut TickState, connection: ConnectionId, ctx: &Ctx, view: View) {
    let (view, screen) = render(state, ctx, view);
    let Some(entity) = state.connections.entity_of(connection) else {
        return;
    };
    let reuse = state
        .settlement_menus
        .get(&connection)
        .filter(|open| open.title == screen.title)
        .map(|open| open.window);
    let registry = state.registry.clone();
    let Some(player) = state.players.get_mut(&entity) else {
        return;
    };
    let current = player.open.as_ref().map(|menu| menu.window);
    let window = if reuse.is_some() && current == reuse {
        let menu = player.open.as_mut().expect("open settlement menu");
        // A click still carrying the old revision is answered with a resync.
        menu.state_id = (menu.state_id + 1) & 0x7FFF;
        let window = menu.window;
        let changes = player.sync_container_view(&screen.icons);
        for change in changes {
            state.push_player(connection, change);
        }
        window
    } else {
        let title = json!({ "text": screen.title }).to_string();
        let effects = player.open_menu(
            &registry.item_rules(),
            MenuKind::Chest { rows: ROWS },
            title,
            &screen.icons,
        );
        let Some(window) = player.open.as_ref().map(|menu| menu.window) else {
            return;
        };
        state.open_containers.remove(&entity);
        publish(state, connection, entity, effects);
        window
    };
    state.settlement_menus.insert(
        connection,
        Open {
            window,
            settlement: ctx.id,
            view,
            title: screen.title,
            icons: screen.icons,
            targets: screen.targets,
        },
    );
}

/// Close the menu. A server-initiated close also tells the client; the cursor
/// stack returns to the inventory (or drops) like any closed chest.
fn close(state: &mut TickState, connection: ConnectionId, entity: EntityId, server: bool) {
    let Some(open) = state.settlement_menus.remove(&connection) else {
        return;
    };
    let registry = state.registry.clone();
    let Some(player) = state.players.get_mut(&entity) else {
        return;
    };
    if player
        .open
        .as_ref()
        .is_none_or(|menu| menu.window != open.window)
    {
        return;
    }
    let effects = player.close(&registry.item_rules());
    if server {
        state.push_player(
            connection,
            PlayerChange::MenuClosed {
                window: open.window,
            },
        );
    }
    publish(state, connection, entity, effects);
}

fn publish(
    state: &mut TickState,
    connection: ConnectionId,
    entity: EntityId,
    effects: crate::player::PlayerEffects,
) {
    for change in effects.changes {
        state.push_player(connection, change);
    }
    if let Some(slot) = state.entities.slot_of(entity) {
        let position = state.entities.pos(slot);
        state
            .merge_life_drops
            .extend(effects.dropped.into_iter().map(|stack| (position, stack)));
    }
}

/// Vanilla handling for moves inside the player's own inventory (pick up,
/// place, hotbar swap, throw) while the storage window is open. Only the
/// player's slots and cursor are touched; the icons are a scratch copy.
/// Admission is the tick's container-click authority
/// (`tick/input/inventory.rs`): with active player holds the click is planned,
/// and a plan below a reservation floor is refused with a resync; spectators
/// only get a resync from [`crate::player::PlayerState::click`] itself.
fn vanilla(
    state: &mut TickState,
    connection: ConnectionId,
    entity: EntityId,
    state_id: i32,
    slot: i16,
    button: i8,
    mode: ClickMode,
) {
    let Some(open) = state.settlement_menus.get(&connection) else {
        return;
    };
    let (window, mut scratch) = (open.window, open.icons.clone());
    let registry = state.registry.clone();
    let native = |player: &mut crate::player::PlayerState, icons: &mut [ItemStack]| {
        player.click(
            &registry.item_rules(),
            window,
            state_id,
            slot,
            button,
            mode,
            icons,
        )
    };
    let effects = if crate::plugin_inventories::has_reservations_for_player(state, entity) {
        let Some(player) = state.players.get(&entity) else {
            return;
        };
        let plan = player.plan_inventory_transition(&scratch, native);
        if crate::plugin_inventories::below_player_floors(state, entity, plan.player_slots()) {
            player.inventory_transition_refused(&scratch)
        } else {
            state
                .players
                .get_mut(&entity)
                .expect("planned player")
                .apply_inventory_transition(plan, &mut scratch)
        }
    } else {
        let Some(player) = state.players.get_mut(&entity) else {
            return;
        };
        native(player, &mut scratch)
    };
    publish(state, connection, entity, effects);
}

/// One click on the window: navigation, storage moves or assignment.
fn click(
    state: &mut TickState,
    connection: ConnectionId,
    entity: EntityId,
    state_id: i32,
    slot: i16,
    button: i8,
    mode: ClickMode,
) {
    let Some((settlement, view)) = state
        .settlement_menus
        .get(&connection)
        .map(|open| (open.settlement, open.view))
    else {
        return;
    };
    let Some(ctx) = super::ctx(state, connection, settlement) else {
        let lang = Lang::of(state.connections.locale_of(connection));
        close(state, connection, entity, true);
        super::say(state, connection, ui_text::text(lang, Key::SettlementGone));
        return;
    };
    let current = state
        .players
        .get(&entity)
        .and_then(|player| player.open.as_ref())
        .map(|menu| menu.state_id);
    if current != Some(state_id) {
        return show(state, connection, &ctx, view);
    }
    let storage_view = matches!(view, View::Storage { .. });
    // Spectators only get a resync for every custody move, as for any menu
    // (`PlayerState::click`); navigation and pages stay usable.
    let spectator = state
        .players
        .get(&entity)
        .is_some_and(crate::player::PlayerState::spectator);
    let own = usize::try_from(slot)
        .ok()
        .filter(|index| (SLOTS..SLOTS + PLAYER_SLOTS).contains(index))
        .map(|index| index - SLOTS);
    if let Some(index) = own {
        match mode {
            ClickMode::QuickMove if storage_view && !spectator => {
                deposit_slot(state, connection, &ctx, index);
            }
            ClickMode::Pickup | ClickMode::Swap | ClickMode::Throw if storage_view => {
                return vanilla(state, connection, entity, state_id, slot, button, mode);
            }
            _ => {}
        }
        return show(state, connection, &ctx, view);
    }
    if slot == -999 && mode == ClickMode::Pickup && storage_view {
        // Throw the cursor stack out of the window, as in any chest.
        return vanilla(state, connection, entity, state_id, slot, button, mode);
    }
    let Some(index) = usize::try_from(slot).ok().filter(|index| *index < SLOTS) else {
        return show(state, connection, &ctx, view);
    };
    if !matches!(mode, ClickMode::Pickup | ClickMode::QuickMove) || !matches!(button, 0 | 1) {
        return show(state, connection, &ctx, view);
    }
    let target = state.settlement_menus[&connection].targets[index].clone();
    if storage_view && index < PAGE {
        if spectator {
            return show(state, connection, &ctx, view);
        }
        if !ctx.manage {
            return deny(state, connection, &ctx, view);
        }
        let carried = state.players[&entity]
            .open
            .as_ref()
            .map_or(ItemStack::EMPTY, |menu| menu.carried.clone());
        if mode == ClickMode::Pickup && !carried.is_empty() {
            deposit_cursor(state, connection, entity, &ctx, &carried, button == 1);
        } else if let Target::Stock(template) = target {
            withdraw(
                state,
                connection,
                entity,
                &ctx,
                &template,
                mode,
                button == 1,
            );
        }
        return show(state, connection, &ctx, view);
    }
    match target {
        Target::None | Target::Refresh | Target::Stock(_) => show(state, connection, &ctx, view),
        Target::Show(next) => show(state, connection, &ctx, next),
        Target::Dialog(action) => {
            close(state, connection, entity, true);
            super::dispatch(state, connection, &ctx, action, ClickPayload(None));
        }
        Target::Pick { worker, from } => {
            if !ctx.manage {
                return deny(state, connection, &ctx, view);
            }
            show(
                state,
                connection,
                &ctx,
                View::Assign {
                    worker,
                    from,
                    page: 0,
                },
            );
        }
        Target::Assign { worker, building } => {
            if !ctx.manage {
                return deny(state, connection, &ctx, view);
            }
            match assign(state, ctx.id, worker, building) {
                Ok(()) => {
                    let note = assigned_text(state, &ctx, worker, building);
                    super::say(state, connection, &note);
                    let next = match view {
                        View::Hire { building, .. }
                        | View::Assign {
                            from: Some(building),
                            ..
                        } => View::Building { building },
                        _ => View::Residents { page: 0 },
                    };
                    show(state, connection, &ctx, next);
                }
                Err(refusal) => {
                    let profession = people(state, ctx.id)
                        .iter()
                        .find(|person| person.uuid == worker)
                        .map_or(Profession::None, |person| person.profession);
                    let note = refusal_text(&ctx, refusal, profession);
                    super::say(state, connection, &note);
                    show(state, connection, &ctx, view);
                }
            }
        }
        Target::Upgrade(building) => {
            if !ctx.manage {
                return deny(state, connection, &ctx, view);
            }
            let id = super::short(building).to_string();
            let note = match buildings::upgrade(state, building) {
                Ok(()) => {
                    let level = state
                        .settlements
                        .get(ctx.id)
                        .and_then(|record| record.buildings.get(&building))
                        .map_or(0, |building| building.level);
                    ctx.f(Key::Upgraded, &[("id", &id), ("level", &level.to_string())])
                }
                Err(error) => ctx.f(
                    Key::CannotUpgrade,
                    &[("id", &id), ("missing", &super::missing_text(&error))],
                ),
            };
            super::say(state, connection, &note);
            show(state, connection, &ctx, View::Building { building });
        }
    }
}

fn deny(state: &mut TickState, connection: ConnectionId, ctx: &Ctx, view: View) {
    super::say(state, connection, ctx.t(Key::Denied));
    show(state, connection, ctx, view);
}

// --------------------------------------------------------------------- storage

/// The settlement's storage containers as fenced images, one per physical
/// container (a double chest's two certified halves are read once), with the
/// unreserved quantity of each item id in each.
struct Stores {
    images: Vec<inventory::Image>,
    free: Vec<BTreeMap<u32, u64>>,
}

/// One kind of stored item: stacks that stack together, summed.
struct Stock {
    template: ItemStack,
    total: u64,
    free: u64,
}

fn stores(state: &TickState, id: SettlementId) -> Stores {
    let mut stores = Stores {
        images: Vec::new(),
        free: Vec::new(),
    };
    let mut physical = BTreeSet::new();
    for position in buildings::storage_points(state, id)
        .into_iter()
        .take(MAX_CONTAINERS)
    {
        let Ok(cells) = crate::plugin_inventories::container_cells(
            state,
            &mc_script::ScriptBlockPosition::new(position[0], position[1], position[2]),
        ) else {
            continue;
        };
        if cells
            .iter()
            .any(|cell| physical.contains(&[cell.x, cell.y, cell.z]))
        {
            continue;
        }
        let endpoint = inventory::Endpoint::Container {
            dimension: state.dimension.name().to_owned(),
            position,
        };
        let Ok(image) = inventory::load(state, &endpoint) else {
            continue;
        };
        physical.extend(cells.iter().map(|cell| [cell.x, cell.y, cell.z]));
        // Unreadable floors fail closed: nothing in that container is free.
        stores
            .free
            .push(inventory::unreserved_stock(state, &image).unwrap_or_default());
        stores.images.push(image);
    }
    stores
}

fn stock(stores: &Stores) -> Vec<Stock> {
    let mut stock: Vec<Stock> = Vec::new();
    for (image, free) in stores.images.iter().zip(&stores.free) {
        let mut here: Vec<(usize, u64)> = Vec::new();
        for stack in image.slots.iter().filter(|stack| !stack.is_empty()) {
            let index = match stock
                .iter()
                .position(|entry| can_stack(&entry.template, stack))
            {
                Some(index) => index,
                None => {
                    let mut template = stack.clone();
                    template.count = 1;
                    stock.push(Stock {
                        template,
                        total: 0,
                        free: 0,
                    });
                    stock.len() - 1
                }
            };
            let count = u64::try_from(stack.count).unwrap_or(0);
            stock[index].total += count;
            match here.iter_mut().find(|(entry, _)| *entry == index) {
                Some((_, held)) => *held += count,
                None => here.push((index, count)),
            }
        }
        let mut left = free.clone();
        for (index, held) in here {
            let budget = left.entry(stock[index].template.item_id).or_default();
            let take = held.min(*budget);
            *budget -= take;
            stock[index].free += take;
        }
    }
    // Registry order keeps an icon in place while counts change.
    stock.sort_by_key(|entry| entry.template.item_id);
    stock
}

/// Take up to `want` of `template` out of the stores, never below a floor.
fn debit(stores: &Stores, template: &ItemStack, want: u64) -> (Vec<Vec<ItemStack>>, u64) {
    let mut after: Vec<Vec<ItemStack>> = stores
        .images
        .iter()
        .map(|image| image.slots.clone())
        .collect();
    let mut taken = 0;
    for (slots, free) in after.iter_mut().zip(&stores.free) {
        let mut budget = free.get(&template.item_id).copied().unwrap_or(0);
        for stack in slots.iter_mut() {
            if taken == want || budget == 0 {
                break;
            }
            if !can_stack(template, stack) {
                continue;
            }
            let take = u64::try_from(stack.count)
                .unwrap_or(0)
                .min(want - taken)
                .min(budget);
            stack.count -= take as i32;
            if stack.is_empty() {
                *stack = ItemStack::EMPTY;
            }
            taken += take;
            budget -= take;
        }
    }
    (after, taken)
}

/// Put `stack` into the stores (vanilla shift-click order: top up matching
/// stacks, then fill empty slots). Returns the new images and the count placed.
fn credit(state: &TickState, stores: &Stores, stack: &ItemStack) -> (Vec<Vec<ItemStack>>, i32) {
    let max = state.registry.item_rules().max_stack(stack).min(99);
    let mut after: Vec<Vec<ItemStack>> = stores
        .images
        .iter()
        .map(|image| image.slots.clone())
        .collect();
    let mut left = stack.count;
    for empty in [false, true] {
        for slot in after.iter_mut().flatten() {
            put(slot, stack, max, empty, &mut left);
        }
    }
    (after, stack.count - left)
}

/// Vanilla chest shift-click into the player: hotbar end first.
fn fill_player(state: &TickState, slots: &mut [ItemStack], stack: &ItemStack) -> i32 {
    let max = state.registry.item_rules().max_stack(stack).min(99);
    let mut left = stack.count;
    for empty in [false, true] {
        for slot in slots.iter_mut().rev() {
            put(slot, stack, max, empty, &mut left);
        }
    }
    stack.count - left
}

/// One `moveItemStackTo` step: top up a matching stack (first pass) or fill an
/// empty slot (second pass).
fn put(slot: &mut ItemStack, stack: &ItemStack, max: i32, empty: bool, left: &mut i32) {
    if *left <= 0 || slot.is_empty() != empty || (!empty && !can_stack(slot, stack)) {
        return;
    }
    let room = if empty { max } else { max - slot.count };
    let count = room.min(*left);
    if count <= 0 {
        return;
    }
    if empty {
        *slot = stack.clone();
        slot.count = count;
    } else {
        slot.count += count;
    }
    *left -= count;
}

fn changes(stores: Stores, after: Vec<Vec<ItemStack>>) -> Vec<inventory::Change> {
    stores
        .images
        .into_iter()
        .zip(after)
        .filter(|(before, after)| before.slots != *after)
        .map(|(before, after)| inventory::Change { before, after })
        .collect()
}

/// Left click: a full stack to the cursor; right click: half of it. Shift: a
/// full stack straight into the inventory (partial when it is nearly full).
fn withdraw(
    state: &mut TickState,
    connection: ConnectionId,
    entity: EntityId,
    ctx: &Ctx,
    template: &ItemStack,
    mode: ClickMode,
    half: bool,
) {
    let stores = stores(state, ctx.id);
    let (total, free) = stock(&stores)
        .iter()
        .find(|entry| can_stack(&entry.template, template))
        .map_or((0, 0), |entry| (entry.total, entry.free));
    if free == 0 {
        let note = if total == 0 {
            tr(
                ctx,
                "That item is no longer in storage.",
                "Этого предмета на складе уже нет.",
            )
        } else {
            tr(
                ctx,
                "All of it is reserved for production or construction.",
                "Всё это зарезервировано под производство или стройку.",
            )
        };
        return super::say(state, connection, note);
    }
    let size = u64::try_from(state.registry.item_rules().max_stack(template).min(99)).unwrap_or(1);
    let mut want = size.min(free);
    let mut stack = template.clone();
    if mode == ClickMode::QuickMove {
        let Some(uuid) = state.connections.uuid_of(connection) else {
            return;
        };
        let Ok(before) = inventory::load(state, &inventory::Endpoint::Player { uuid }) else {
            return;
        };
        let mut after = before.slots.clone();
        stack.count = want as i32;
        let placed = fill_player(state, &mut after, &stack);
        if placed <= 0 {
            return super::say(
                state,
                connection,
                tr(ctx, "Your inventory is full.", "Ваш инвентарь полон."),
            );
        }
        let (stored, taken) = debit(&stores, template, placed as u64);
        if taken != placed as u64 {
            return;
        }
        let mut changes = changes(stores, stored);
        changes.push(inventory::Change { before, after });
        if let Err(failure) = inventory::commit(state, changes) {
            super::say(state, connection, &super::failure_text(ctx, failure));
        }
        return;
    }
    if half {
        want = want.div_ceil(2);
    }
    // The cursor must be free and stay in this window: the debit and the
    // cursor stack are one custody move within this tick.
    if state
        .players
        .get(&entity)
        .and_then(|player| player.open.as_ref())
        .is_none_or(|menu| !menu.carried.is_empty())
    {
        return;
    }
    let (stored, taken) = debit(&stores, template, want);
    if taken == 0 {
        return;
    }
    if let Err(failure) = inventory::commit(state, changes(stores, stored)) {
        return super::say(state, connection, &super::failure_text(ctx, failure));
    }
    stack.count = taken as i32;
    if let Some(menu) = state
        .players
        .get_mut(&entity)
        .and_then(|player| player.open.as_mut())
    {
        menu.carried = stack;
    }
}

/// Shift-click on one of the player's own slots stores that stack.
fn deposit_slot(state: &mut TickState, connection: ConnectionId, ctx: &Ctx, index: usize) {
    if !ctx.manage {
        return super::say(state, connection, ctx.t(Key::Denied));
    }
    let Some(uuid) = state.connections.uuid_of(connection) else {
        return;
    };
    let Ok(before) = inventory::load(state, &inventory::Endpoint::Player { uuid }) else {
        return;
    };
    let Some(stack) = before
        .slots
        .get(index)
        .filter(|stack| !stack.is_empty())
        .cloned()
    else {
        return;
    };
    let stores = stores(state, ctx.id);
    if stores.images.is_empty() {
        return super::say(state, connection, no_storage(ctx));
    }
    let (stored, placed) = credit(state, &stores, &stack);
    if placed <= 0 {
        return super::say(state, connection, storage_full(ctx));
    }
    let mut after = before.slots.clone();
    after[index].count -= placed;
    if after[index].is_empty() {
        after[index] = ItemStack::EMPTY;
    }
    let mut changes = changes(stores, stored);
    changes.push(inventory::Change { before, after });
    if let Err(failure) = inventory::commit(state, changes) {
        super::say(state, connection, &super::failure_text(ctx, failure));
    }
}

/// A click on the storage grid with a held stack stores it (right: one item).
fn deposit_cursor(
    state: &mut TickState,
    connection: ConnectionId,
    entity: EntityId,
    ctx: &Ctx,
    carried: &ItemStack,
    one: bool,
) {
    let stores = stores(state, ctx.id);
    if stores.images.is_empty() {
        return super::say(state, connection, no_storage(ctx));
    }
    let mut stack = carried.clone();
    if one {
        stack.count = 1;
    }
    let (stored, placed) = credit(state, &stores, &stack);
    if placed <= 0 {
        return super::say(state, connection, storage_full(ctx));
    }
    if let Err(failure) = inventory::commit(state, changes(stores, stored)) {
        return super::say(state, connection, &super::failure_text(ctx, failure));
    }
    if let Some(menu) = state
        .players
        .get_mut(&entity)
        .and_then(|player| player.open.as_mut())
    {
        menu.carried.count -= placed;
        if menu.carried.is_empty() {
            menu.carried = ItemStack::EMPTY;
        }
    }
}

fn no_storage(ctx: &Ctx) -> &'static str {
    tr(
        ctx,
        "No certified storage: certify a building with chests or barrels as Storage.",
        "Нет сертифицированного склада: сертифицируйте здание с сундуками или бочками как склад.",
    )
}

fn storage_full(ctx: &Ctx) -> &'static str {
    tr(ctx, "Storage is full.", "Склад заполнен.")
}

// ------------------------------------------------------------------ assignment

/// A live bound resident as the assignment rules see it.
struct Person {
    uuid: Uuid,
    entity: EntityId,
    profession: Profession,
    /// Job site in this dimension.
    job: Option<BlockPos>,
    building: Option<BuildingId>,
    adult: bool,
    /// Barracks of a soldier.
    soldier: Option<BuildingId>,
    traded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    Gone,
    NotWorkplace,
    Child,
    Soldier,
    Nitwit,
    Builder,
    Traded,
    Busy,
    Already,
    Full,
}

/// Live bound residents, deterministic order (uuid).
fn people(state: &TickState, id: SettlementId) -> Vec<Person> {
    let mut people = Vec::new();
    for entities in state.entities.villager_chunks().values() {
        for &entity in entities {
            let Some(slot) = state.entities.slot_of(entity) else {
                continue;
            };
            if !state.entities.body(slot).alive {
                continue;
            }
            let row = state.entities.row(slot);
            let Some(villager) = row.villager.as_ref() else {
                continue;
            };
            if villager.settlement != Some(id)
                || villager.wandering
                || villager.conversion_time.is_some()
            {
                continue;
            }
            people.push(Person {
                uuid: row.uuid,
                entity,
                profession: villager.data.profession,
                job: villager
                    .job_site
                    .filter(|_| villager.job_site_dimension == state.dimension.name()),
                building: buildings::building_for_job(state, villager).map(|building| building.id),
                adult: villager.age >= 0,
                soldier: villager.soldier.as_ref().map(|soldier| soldier.barracks),
                traded: crate::settlements::allocator::traded(villager),
            });
        }
    }
    people.sort_by_key(|person| person.uuid);
    people
}

/// A certified work point of `building` with its live workstation that no
/// other resident claims.
fn free_point(
    state: &TickState,
    people: &[Person],
    building: &Building,
    owner: EntityId,
) -> Option<BlockPos> {
    let profession = building.kind.profession()?;
    building
        .work_points
        .iter()
        .take(buildings::job_slots(building.kind, building.level))
        .map(|&[x, y, z]| BlockPos { x, y, z })
        .find(|&pos| {
            live_station(state, building, pos)
                && !state.villager_pois.claimed(
                    owner,
                    Poi {
                        pos,
                        kind: PoiKind::Job(profession),
                    },
                )
                && !people
                    .iter()
                    .any(|other| other.entity != owner && other.job == Some(pos))
        })
}

/// The allocator's own workstation test (natural zones have none).
fn live_station(state: &TickState, building: &Building, pos: BlockPos) -> bool {
    state
        .world
        .block_at(pos.x, pos.y, pos.z)
        .is_some_and(|block| {
            building.natural_zone
                || buildings::workstation_matches(&state.registry, building.kind, block)
        })
}

/// Why `person` cannot be assigned to `building`, if anything.
fn refusal(
    state: &TickState,
    people: &[Person],
    busy: &BTreeSet<Uuid>,
    person: &Person,
    building: &Building,
) -> Option<Refusal> {
    let Some(profession) = building.kind.profession() else {
        return Some(Refusal::NotWorkplace);
    };
    if !person.adult {
        return Some(Refusal::Child);
    }
    if person.soldier.is_some() || person.profession == Profession::Soldier {
        return Some(Refusal::Soldier);
    }
    if person.profession == Profession::Nitwit {
        return Some(Refusal::Nitwit);
    }
    if person.profession == Profession::Builder && profession != Profession::Builder {
        return Some(Refusal::Builder);
    }
    if person.traded && person.profession != profession {
        return Some(Refusal::Traded);
    }
    if busy.contains(&person.uuid) {
        return Some(Refusal::Busy);
    }
    if person.building == Some(building.id) {
        return Some(Refusal::Already);
    }
    if free_point(state, people, building, person.entity).is_none() {
        return Some(Refusal::Full);
    }
    None
}

/// Point `worker`'s job site at a free work point of `building`. The previous
/// claim is released through the allocator's claim index; the brain walks the
/// resident there and changes the profession on arrival.
fn assign(
    state: &mut TickState,
    settlement: SettlementId,
    worker: Uuid,
    building: BuildingId,
) -> Result<(), Refusal> {
    let target = state
        .settlements
        .get(settlement)
        .and_then(|record| record.buildings.get(&building))
        .cloned()
        .ok_or(Refusal::Gone)?;
    let people = people(state, settlement);
    let person = people
        .iter()
        .find(|person| person.uuid == worker)
        .ok_or(Refusal::Gone)?;
    let busy = super::busy_workers(state, settlement);
    if let Some(refusal) = refusal(state, &people, &busy, person, &target) {
        return Err(refusal);
    }
    let pos = free_point(state, &people, &target, person.entity).ok_or(Refusal::Full)?;
    let owner = person.entity;
    let slot = state.entities.slot_of(owner).ok_or(Refusal::Gone)?;
    let dimension = state.dimension.name().to_owned();
    let villager = state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .ok_or(Refusal::Gone)?;
    let old = [villager.home, villager.job_site, villager.meeting_point];
    villager.job_site = Some(pos);
    villager.job_site_dimension = dimension;
    villager.poi_validation.sites[2] = None;
    villager.navigation_path.clear();
    villager.navigation_target = None;
    villager.navigation_chunks.clear();
    villager.navigation_repath_at = 0;
    let new = [villager.home, villager.job_site, villager.meeting_point];
    state.villager_pois.update_claims(owner, old, new);
    Ok(())
}

fn assigned_text(state: &TickState, ctx: &Ctx, worker: Uuid, building: BuildingId) -> String {
    let place = state
        .settlements
        .get(ctx.id)
        .and_then(|record| record.buildings.get(&building))
        .map_or_else(String::new, |building| building_name(ctx, building));
    let name = crate::settlements::presentation::villager_name(worker);
    match ctx.lang {
        Lang::En => format!(
            "{name} is assigned to {place}: they walk to the workstation and take the craft there."
        ),
        Lang::Ru => {
            format!("{name} назначен(а) в {place}: дойдёт до рабочего места и освоит там ремесло.")
        }
    }
}

/// Why an assignment was refused; `profession` is the resident's current one.
fn refusal_text(ctx: &Ctx, refusal: Refusal, profession: Profession) -> String {
    match refusal {
        Refusal::Traded => match ctx.lang {
            Lang::En => format!(
                "Has traded and keeps the {} craft.",
                profession_name(ctx, profession)
            ),
            Lang::Ru => format!(
                "Уже торговал(а) и сохраняет ремесло: {}.",
                profession_name(ctx, profession)
            ),
        },
        Refusal::Gone => tr(
            ctx,
            "The resident or building no longer exists.",
            "Жителя или постройки больше нет.",
        )
        .to_owned(),
        Refusal::NotWorkplace => tr(
            ctx,
            "This building has no jobs.",
            "В этой постройке нет рабочих мест.",
        )
        .to_owned(),
        Refusal::Child => tr(ctx, "Children do not work.", "Дети не работают.").to_owned(),
        Refusal::Soldier => tr(
            ctx,
            "Soldiers serve in the army: dismiss them on the Army page first.",
            "Солдаты служат в армии: сначала распустите их на странице «Армия».",
        )
        .to_owned(),
        Refusal::Nitwit => tr(
            ctx,
            "A nitwit cannot learn a craft.",
            "Бездельник не может освоить ремесло.",
        )
        .to_owned(),
        Refusal::Builder => tr(
            ctx,
            "Builders work only in builder huts.",
            "Строители работают только в мастерских строителя.",
        )
        .to_owned(),
        Refusal::Busy => tr(
            ctx,
            "Busy with a production order: let it finish or cancel it first.",
            "Занят(а) заказом производства: дождитесь его или отмените.",
        )
        .to_owned(),
        Refusal::Already => tr(ctx, "Already works here.", "Уже работает здесь.").to_owned(),
        Refusal::Full => tr(
            ctx,
            "No free work place with a workstation here.",
            "Здесь нет свободного места с рабочим блоком.",
        )
        .to_owned(),
    }
}

// ------------------------------------------------------------------- rendering

fn render(state: &TickState, ctx: &Ctx, view: View) -> (View, Screen) {
    let name = state
        .settlements
        .get(ctx.id)
        .map(|record| record.name.clone())
        .unwrap_or_default();
    match view {
        View::Storage { page } => (view, storage_screen(state, ctx, &name, page)),
        View::Residents { page } => (view, residents_screen(state, ctx, &name, page)),
        View::Buildings { page } => (view, buildings_screen(state, ctx, &name, page)),
        View::Building { building } => match building_screen(state, ctx, building) {
            Some(screen) => (view, screen),
            None => (
                View::Buildings { page: 0 },
                buildings_screen(state, ctx, &name, 0),
            ),
        },
        View::Assign { worker, from, page } => {
            match assign_screen(state, ctx, worker, from, page) {
                Some(screen) => (view, screen),
                None => (
                    View::Residents { page: 0 },
                    residents_screen(state, ctx, &name, 0),
                ),
            }
        }
        View::Hire { building, page } => match hire_screen(state, ctx, building, page) {
            Some(screen) => (view, screen),
            None => (
                View::Buildings { page: 0 },
                buildings_screen(state, ctx, &name, 0),
            ),
        },
    }
}

fn storage_screen(state: &TickState, ctx: &Ctx, name: &str, page: usize) -> Screen {
    let stores = stores(state, ctx.id);
    let stock = stock(&stores);
    let pages = pages(stock.len());
    let page = page.min(pages - 1);
    let mut screen = Screen::new(format!("{} · {name}", tr(ctx, "Storage", "Склад")));
    for (slot, entry) in stock.iter().skip(page * PAGE).take(PAGE).enumerate() {
        let mut lore = vec![format!(
            "{}: {}",
            tr(ctx, "In storage", "На складе"),
            entry.total
        )];
        if entry.free < entry.total {
            lore.push(format!(
                "{}: {}",
                tr(ctx, "Reserved", "В резерве"),
                entry.total - entry.free
            ));
        }
        if ctx.manage {
            lore.push(
                tr(
                    ctx,
                    "Left: take a stack, right: half",
                    "ЛКМ — стак, ПКМ — половина",
                )
                .into(),
            );
            lore.push(
                tr(
                    ctx,
                    "Shift: straight to inventory",
                    "Shift — сразу в инвентарь",
                )
                .into(),
            );
        }
        let mut shown = entry.template.clone();
        shown.count = i32::try_from(entry.total.min(99)).unwrap_or(99);
        let label = Tag::Compound(vec![
            ("text".to_owned(), Tag::String(String::new())),
            (
                "extra".to_owned(),
                compounds(vec![
                    item_label(state, &entry.template),
                    Tag::Compound(vec![
                        ("text".to_owned(), Tag::String(format!(" ×{}", entry.total))),
                        ("color".to_owned(), Tag::String("gray".to_owned())),
                    ]),
                ]),
            ),
        ]);
        screen.put(
            slot,
            decorate(state, shown, label, &lore),
            Target::Stock(entry.template.clone()),
        );
    }
    let total: u64 = stock.iter().map(|entry| entry.total).sum();
    let (title, mut lore) = if stores.images.is_empty() {
        (
            tr(ctx, "No certified storage", "Склад не сертифицирован").to_owned(),
            vec![
                tr(
                    ctx,
                    "Certify a building with chests as Storage",
                    "Сертифицируйте здание с сундуками как склад",
                )
                .to_owned(),
            ],
        )
    } else {
        let title = match ctx.lang {
            Lang::En => format!(
                "{} containers, {} kinds, {total} items",
                stores.images.len(),
                stock.len()
            ),
            Lang::Ru => format!(
                "Хранилищ: {}, видов: {}, предметов: {total}",
                stores.images.len(),
                stock.len()
            ),
        };
        let lore = if ctx.manage {
            vec![
                tr(
                    ctx,
                    "Shift-click your items to store them",
                    "Shift+клик по своему предмету — на склад",
                )
                .to_owned(),
                tr(
                    ctx,
                    "or put the held stack on the grid",
                    "или положите стак с курсора в сетку",
                )
                .to_owned(),
            ]
        } else {
            vec![ctx.t(Key::ViewOnly).to_owned()]
        };
        (title, lore)
    };
    lore.push(page_text(ctx, page, pages));
    lore.push(tr(ctx, "Click: refresh", "Клик — обновить").to_owned());
    screen.put(
        INFO,
        icon(state, "minecraft:chest", 1, text(title), &lore),
        Target::Refresh,
    );
    screen.put(BACK, back(state, ctx), Target::Dialog("main"));
    navigation(&mut screen, state, ctx, page, pages, |page| View::Storage {
        page,
    });
    screen
}

fn residents_screen(state: &TickState, ctx: &Ctx, name: &str, page: usize) -> Screen {
    let people = people(state, ctx.id);
    let busy = super::busy_workers(state, ctx.id);
    let pages = pages(people.len());
    let page = page.min(pages - 1);
    let mut screen = Screen::new(format!("{} · {name}", tr(ctx, "Residents", "Жители")));
    let hint = if ctx.manage {
        tr(
            ctx,
            "Click: choose a workplace",
            "Клик — выбрать место работы",
        )
    } else {
        ""
    };
    for (slot, person) in people.iter().skip(page * PAGE).take(PAGE).enumerate() {
        screen.put(
            slot,
            person_icon(state, ctx, person, &busy, hint),
            Target::Pick {
                worker: person.uuid,
                from: None,
            },
        );
    }
    let working = people
        .iter()
        .filter(|person| person.building.is_some())
        .count();
    let free: usize = state
        .settlements
        .get(ctx.id)
        .map(|record| {
            record
                .buildings
                .values()
                .filter(|building| building.kind.profession().is_some())
                .map(|building| {
                    let filled = people
                        .iter()
                        .filter(|person| person.building == Some(building.id))
                        .count();
                    buildings::job_slots(building.kind, building.level).saturating_sub(filled)
                })
                .sum::<usize>()
        })
        .unwrap_or(0);
    let title = match ctx.lang {
        Lang::En => format!(
            "Residents {}, working {working}, free job places {free}",
            people.len()
        ),
        Lang::Ru => format!(
            "Жителей {}, работают {working}, свободных мест {free}",
            people.len()
        ),
    };
    let mut lore = vec![page_text(ctx, page, pages)];
    if !ctx.manage {
        lore.push(ctx.t(Key::ViewOnly).to_owned());
    }
    lore.push(tr(ctx, "Click: refresh", "Клик — обновить").to_owned());
    screen.put(
        INFO,
        icon(state, "minecraft:bell", 1, text(title), &lore),
        Target::Refresh,
    );
    screen.put(BACK, back(state, ctx), Target::Dialog("main"));
    screen.put(
        LIST,
        icon(
            state,
            "minecraft:paper",
            1,
            text(tr(ctx, "Jobs list", "Список работ")),
            &[],
        ),
        Target::Dialog("residents"),
    );
    navigation(&mut screen, state, ctx, page, pages, |page| {
        View::Residents { page }
    });
    screen
}

fn buildings_screen(state: &TickState, ctx: &Ctx, name: &str, page: usize) -> Screen {
    let mut screen = Screen::new(format!("{} · {name}", tr(ctx, "Buildings", "Постройки")));
    let Some(record) = state.settlements.get(ctx.id) else {
        return screen;
    };
    let people = people(state, ctx.id);
    let count = record.buildings.len();
    let pages = pages(count);
    let page = page.min(pages - 1);
    for (slot, building) in record
        .buildings
        .values()
        .skip(page * PAGE)
        .take(PAGE)
        .enumerate()
    {
        let mut lore = vec![super::category(ctx, building.kind).to_owned()];
        lore.extend(staffing(ctx, &people, building));
        lore.push(format!(
            "{} .. {}",
            super::corner_text(building.bounds.min),
            super::corner_text(building.bounds.max)
        ));
        lore.push(tr(ctx, "Click: details", "Клик — подробнее").to_owned());
        screen.put(
            slot,
            building_icon(state, ctx, building, &lore),
            Target::Show(View::Building {
                building: building.id,
            }),
        );
    }
    let mut counts = BTreeMap::<&str, usize>::new();
    for building in record.buildings.values() {
        *counts
            .entry(super::category(ctx, building.kind))
            .or_default() += 1;
    }
    let mut lore: Vec<String> = counts
        .iter()
        .map(|(category, count)| format!("{category}: {count}"))
        .collect();
    lore.push(page_text(ctx, page, pages));
    let title = match ctx.lang {
        Lang::En => format!("Certified buildings: {count}"),
        Lang::Ru => format!("Сертифицированных построек: {count}"),
    };
    screen.put(
        INFO,
        icon(state, "minecraft:bricks", 1, text(title), &lore),
        Target::Refresh,
    );
    screen.put(BACK, back(state, ctx), Target::Dialog("main"));
    if ctx.manage {
        screen.put(
            ACTION,
            icon(
                state,
                "minecraft:writable_book",
                1,
                text(ctx.t(Key::CertifyNew)),
                &[],
            ),
            Target::Dialog("certify"),
        );
    }
    screen.put(
        LIST,
        icon(
            state,
            "minecraft:paper",
            1,
            text(tr(ctx, "List with requirements", "Список с требованиями")),
            &[],
        ),
        Target::Dialog("buildings"),
    );
    navigation(&mut screen, state, ctx, page, pages, |page| {
        View::Buildings { page }
    });
    screen
}

fn building_screen(state: &TickState, ctx: &Ctx, id: BuildingId) -> Option<Screen> {
    let building = state.settlements.get(ctx.id)?.buildings.get(&id)?.clone();
    let people = people(state, ctx.id);
    let busy = super::busy_workers(state, ctx.id);
    let mut screen = Screen::new(building_name(ctx, &building));
    let mut lore = vec![super::category(ctx, building.kind).to_owned()];
    lore.extend(staffing(ctx, &people, &building));
    lore.push(format!(
        "{} .. {}",
        super::corner_text(building.bounds.min),
        super::corner_text(building.bounds.max)
    ));
    let next = building.level + 1;
    let mut upgrade = Vec::new();
    if building.level >= 3 {
        lore.push(tr(ctx, "Maximum level", "Максимальный уровень").to_owned());
    } else {
        match buildings::inspect(state, building.kind, next, building.bounds) {
            Ok(_) => upgrade.push(match ctx.lang {
                Lang::En => format!("Ready for level {next}"),
                Lang::Ru => format!("Готово к уровню {next}"),
            }),
            Err(error) => {
                upgrade.push(match ctx.lang {
                    Lang::En => format!("Level {next} needs:"),
                    Lang::Ru => format!("Для уровня {next} нужно:"),
                });
                upgrade.extend(
                    super::missing_text(&error)
                        .split(", ")
                        .map(|need| format!("- {need}")),
                );
            }
        }
        lore.extend(upgrade.iter().cloned());
    }
    screen.put(
        BUILDING_ICON,
        building_icon(state, ctx, &building, &lore),
        Target::Refresh,
    );
    if ctx.manage && building.level < 3 {
        let label = match ctx.lang {
            Lang::En => format!("Upgrade to level {next}"),
            Lang::Ru => format!("Улучшить до уровня {next}"),
        };
        screen.put(
            BUILDING_UPGRADE,
            icon(
                state,
                "minecraft:experience_bottle",
                1,
                text(label),
                &upgrade,
            ),
            Target::Upgrade(id),
        );
    }
    if building.kind == BuildingKind::Storage {
        screen.put(
            BUILDING_STORAGE,
            icon(
                state,
                "minecraft:chest",
                1,
                text(tr(ctx, "Open storage", "Открыть склад")),
                &[],
            ),
            Target::Show(View::Storage { page: 0 }),
        );
    }
    let places = BUILDING_PLACES..PAGE;
    if building.kind.profession().is_some() {
        let points = building
            .work_points
            .iter()
            .take(buildings::job_slots(building.kind, building.level));
        for (slot, &[x, y, z]) in places.zip(points) {
            let pos = BlockPos { x, y, z };
            if let Some(person) = people.iter().find(|person| person.job == Some(pos)) {
                let hint = if ctx.manage {
                    tr(
                        ctx,
                        "Click: move to another workplace",
                        "Клик — перевести на другое место",
                    )
                } else {
                    ""
                };
                screen.put(
                    slot,
                    person_icon(state, ctx, person, &busy, hint),
                    Target::Pick {
                        worker: person.uuid,
                        from: Some(id),
                    },
                );
            } else if live_station(state, &building, pos) {
                let mut lore = vec![format!("{x} {y} {z}")];
                if ctx.manage {
                    lore.push(tr(ctx, "Click: hire a resident", "Клик — нанять жителя").to_owned());
                }
                screen.put(
                    slot,
                    icon(
                        state,
                        "minecraft:light_gray_stained_glass_pane",
                        1,
                        text(tr(ctx, "Free work place", "Свободное место")),
                        &lore,
                    ),
                    Target::Show(View::Hire {
                        building: id,
                        page: 0,
                    }),
                );
            } else {
                screen.put(
                    slot,
                    icon(
                        state,
                        "minecraft:barrier",
                        1,
                        text(tr(ctx, "Workstation missing", "Нет рабочего блока")),
                        &[format!("{x} {y} {z}")],
                    ),
                    Target::None,
                );
            }
        }
    } else if building.kind == BuildingKind::Barracks {
        let soldiers = people.iter().filter(|person| person.soldier == Some(id));
        for (slot, person) in places.zip(soldiers) {
            screen.put(
                slot,
                person_icon(state, ctx, person, &busy, ""),
                Target::None,
            );
        }
    }
    screen.put(
        BACK,
        back(state, ctx),
        Target::Show(View::Buildings { page: 0 }),
    );
    screen.put(
        LIST,
        icon(
            state,
            "minecraft:paper",
            1,
            text(tr(ctx, "List with requirements", "Список с требованиями")),
            &[],
        ),
        Target::Dialog("buildings"),
    );
    Some(screen)
}

fn assign_screen(
    state: &TickState,
    ctx: &Ctx,
    worker: Uuid,
    from: Option<BuildingId>,
    page: usize,
) -> Option<Screen> {
    let record = state.settlements.get(ctx.id)?;
    let people = people(state, ctx.id);
    let person = people.iter().find(|person| person.uuid == worker)?;
    let busy = super::busy_workers(state, ctx.id);
    let name = crate::settlements::presentation::villager_name(worker);
    let mut screen = Screen::new(match ctx.lang {
        Lang::En => format!("Workplace for {name}"),
        Lang::Ru => format!("Работа для: {name}"),
    });
    let workplaces: Vec<&Building> = record
        .buildings
        .values()
        .filter(|building| building.kind.profession().is_some())
        .collect();
    let pages = pages(workplaces.len());
    let page = page.min(pages - 1);
    for (slot, building) in workplaces.iter().skip(page * PAGE).take(PAGE).enumerate() {
        let mut lore = staffing(ctx, &people, building);
        lore.push(match refusal(state, &people, &busy, person, building) {
            None => tr(ctx, "Click: assign here", "Клик — назначить сюда").to_owned(),
            Some(Refusal::Already) => {
                tr(ctx, "Current workplace", "Текущее место работы").to_owned()
            }
            Some(reason) => refusal_text(ctx, reason, person.profession),
        });
        screen.put(
            slot,
            building_icon(state, ctx, building, &lore),
            Target::Assign {
                worker,
                building: building.id,
            },
        );
    }
    let mut info = person_icon(state, ctx, person, &busy, "");
    if workplaces.is_empty() {
        info = person_icon(
            state,
            ctx,
            person,
            &busy,
            tr(
                ctx,
                "No certified workplaces yet: certify a workshop",
                "Нет сертифицированных мастерских: сертифицируйте мастерскую",
            ),
        );
    }
    screen.put(INFO, info, Target::Refresh);
    let back_to = from.map_or(View::Residents { page: 0 }, |building| View::Building {
        building,
    });
    screen.put(BACK, back(state, ctx), Target::Show(back_to));
    navigation(&mut screen, state, ctx, page, pages, |page| View::Assign {
        worker,
        from,
        page,
    });
    Some(screen)
}

fn hire_screen(state: &TickState, ctx: &Ctx, id: BuildingId, page: usize) -> Option<Screen> {
    let building = state.settlements.get(ctx.id)?.buildings.get(&id)?.clone();
    let people = people(state, ctx.id);
    let busy = super::busy_workers(state, ctx.id);
    let mut candidates: Vec<(Option<Refusal>, &Person)> = people
        .iter()
        .map(|person| (refusal(state, &people, &busy, person, &building), person))
        .collect();
    // Assignable residents first, the unemployed among them first of all.
    candidates.sort_by_key(|(refusal, person)| {
        (refusal.is_some(), person.building.is_some(), person.uuid)
    });
    let mut screen = Screen::new(match ctx.lang {
        Lang::En => format!("Hire for {}", building_name(ctx, &building)),
        Lang::Ru => format!("Нанять в {}", building_name(ctx, &building)),
    });
    let pages = pages(candidates.len());
    let page = page.min(pages - 1);
    for (slot, (reason, person)) in candidates.iter().skip(page * PAGE).take(PAGE).enumerate() {
        let hint = match reason {
            None => tr(ctx, "Click: assign here", "Клик — назначить сюда").to_owned(),
            Some(reason) => refusal_text(ctx, *reason, person.profession),
        };
        screen.put(
            slot,
            person_icon(state, ctx, person, &busy, &hint),
            Target::Assign {
                worker: person.uuid,
                building: id,
            },
        );
    }
    let lore = staffing(ctx, &people, &building);
    screen.put(
        INFO,
        building_icon(state, ctx, &building, &lore),
        Target::Refresh,
    );
    screen.put(
        BACK,
        back(state, ctx),
        Target::Show(View::Building { building: id }),
    );
    navigation(&mut screen, state, ctx, page, pages, |page| View::Hire {
        building: id,
        page,
    });
    Some(screen)
}

/// Jobs or soldier beds filled, for buildings that have either.
fn staffing(ctx: &Ctx, people: &[Person], building: &Building) -> Vec<String> {
    let slots = buildings::job_slots(building.kind, building.level);
    if building.kind == BuildingKind::Barracks {
        let filled = people
            .iter()
            .filter(|person| person.soldier == Some(building.id))
            .count();
        return vec![format!(
            "{}: {filled}/{slots}",
            tr(ctx, "Soldiers", "Солдаты")
        )];
    }
    if building.kind.profession().is_none() || slots == 0 {
        return Vec::new();
    }
    let filled = people
        .iter()
        .filter(|person| person.building == Some(building.id))
        .count();
    vec![format!(
        "{}: {filled}/{slots}",
        tr(ctx, "Workers", "Работники")
    )]
}

fn person_icon(
    state: &TickState,
    ctx: &Ctx,
    person: &Person,
    busy: &BTreeSet<Uuid>,
    hint: &str,
) -> ItemStack {
    let record = state.settlements.get(ctx.id);
    let mut lore = vec![profession_name(ctx, person.profession).to_owned()];
    let workplace = person
        .building
        .and_then(|id| record.and_then(|record| record.buildings.get(&id)));
    lore.push(match (person.soldier, workplace) {
        (Some(barracks), _) => format!(
            "{} #{}",
            tr(ctx, "Serves in barracks", "Служит в казармах"),
            super::short(barracks)
        ),
        (None, Some(building)) => format!(
            "{} {}",
            tr(ctx, "Works at", "Работает в"),
            building_name(ctx, building)
        ),
        (None, None) => tr(ctx, "No workplace", "Без работы").to_owned(),
    });
    if !person.adult {
        lore.push(tr(ctx, "Child", "Ребёнок").to_owned());
    }
    if busy.contains(&person.uuid) {
        lore.push(
            tr(
                ctx,
                "Busy with a production order",
                "Занят(а) заказом производства",
            )
            .to_owned(),
        );
    }
    if person.traded && person.soldier.is_none() {
        lore.push(
            tr(
                ctx,
                "Has traded: keeps the craft",
                "Торговал(а): ремесло не сменит",
            )
            .to_owned(),
        );
    }
    lore.push(format!("id {}", &person.uuid.simple().to_string()[..8]));
    if !hint.is_empty() {
        lore.push(hint.to_owned());
    }
    let item = if person.soldier.is_some() {
        "minecraft:iron_sword"
    } else {
        profession_item(person.profession)
    };
    icon(
        state,
        item,
        1,
        text(crate::settlements::presentation::villager_name(person.uuid)),
        &lore,
    )
}

fn building_icon(state: &TickState, ctx: &Ctx, building: &Building, lore: &[String]) -> ItemStack {
    icon(
        state,
        kind_item(building.kind),
        1,
        text(building_name(ctx, building)),
        lore,
    )
}

fn building_name(ctx: &Ctx, building: &Building) -> String {
    format!(
        "#{} {} L{}",
        super::short(building.id),
        super::kind_label(ctx, building.kind),
        building.level
    )
}

fn back(state: &TickState, ctx: &Ctx) -> ItemStack {
    icon(state, "minecraft:oak_door", 1, text(ctx.t(Key::Back)), &[])
}

fn navigation(
    screen: &mut Screen,
    state: &TickState,
    ctx: &Ctx,
    page: usize,
    pages: usize,
    at: impl Fn(usize) -> View,
) {
    if page > 0 {
        screen.put(
            PREVIOUS,
            icon(
                state,
                "minecraft:arrow",
                1,
                text(tr(ctx, "Previous page", "Предыдущая страница")),
                &[],
            ),
            Target::Show(at(page - 1)),
        );
    }
    if page + 1 < pages {
        screen.put(
            NEXT,
            icon(
                state,
                "minecraft:arrow",
                1,
                text(tr(ctx, "Next page", "Следующая страница")),
                &[],
            ),
            Target::Show(at(page + 1)),
        );
    }
}

fn pages(count: usize) -> usize {
    count.div_ceil(PAGE).max(1)
}

fn page_text(ctx: &Ctx, page: usize, pages: usize) -> String {
    format!("{} {}/{pages}", tr(ctx, "Page", "Страница"), page + 1)
}

fn kind_item(kind: BuildingKind) -> &'static str {
    match kind {
        BuildingKind::House => "minecraft:red_bed",
        BuildingKind::Storage => "minecraft:chest",
        BuildingKind::Workshop(profession) => profession_item(profession),
        BuildingKind::Mine => "minecraft:iron_pickaxe",
        BuildingKind::LumberCamp => "minecraft:iron_axe",
        BuildingKind::Barracks => "minecraft:shield",
        BuildingKind::Wall => "minecraft:cobblestone_wall",
        BuildingKind::Tower => "minecraft:spyglass",
        BuildingKind::Gate => "minecraft:oak_fence_gate",
    }
}

/// The profession's workstation (vanilla job site block), else its tool.
fn profession_item(profession: Profession) -> &'static str {
    match profession {
        Profession::None | Profession::Nitwit => "minecraft:villager_spawn_egg",
        Profession::Armorer => "minecraft:blast_furnace",
        Profession::Butcher => "minecraft:smoker",
        Profession::Cartographer => "minecraft:cartography_table",
        Profession::Cleric => "minecraft:brewing_stand",
        Profession::Farmer => "minecraft:composter",
        Profession::Fisherman => "minecraft:barrel",
        Profession::Fletcher => "minecraft:fletching_table",
        Profession::Leatherworker => "minecraft:cauldron",
        Profession::Librarian => "minecraft:lectern",
        Profession::Mason => "minecraft:stonecutter",
        Profession::Shepherd => "minecraft:loom",
        Profession::Toolsmith => "minecraft:smithing_table",
        Profession::Weaponsmith => "minecraft:grindstone",
        Profession::Miner => "minecraft:iron_pickaxe",
        Profession::Lumberjack => "minecraft:iron_axe",
        Profession::Builder => "minecraft:crafting_table",
        Profession::Soldier => "minecraft:iron_sword",
    }
}

fn profession_name(ctx: &Ctx, profession: Profession) -> &'static str {
    match profession {
        Profession::None => tr(ctx, "Unemployed", "Безработный"),
        Profession::Nitwit => tr(ctx, "Nitwit", "Бездельник"),
        Profession::Armorer => tr(ctx, "Armorer", "Бронник"),
        Profession::Butcher => tr(ctx, "Butcher", "Мясник"),
        Profession::Cartographer => tr(ctx, "Cartographer", "Картограф"),
        Profession::Cleric => tr(ctx, "Cleric", "Священник"),
        Profession::Farmer => tr(ctx, "Farmer", "Фермер"),
        Profession::Fisherman => tr(ctx, "Fisherman", "Рыбак"),
        Profession::Fletcher => tr(ctx, "Fletcher", "Лучник"),
        Profession::Leatherworker => tr(ctx, "Leatherworker", "Кожевник"),
        Profession::Librarian => tr(ctx, "Librarian", "Библиотекарь"),
        Profession::Mason => tr(ctx, "Mason", "Каменщик"),
        Profession::Shepherd => tr(ctx, "Shepherd", "Пастух"),
        Profession::Toolsmith => tr(ctx, "Toolsmith", "Инструментальщик"),
        Profession::Weaponsmith => tr(ctx, "Weaponsmith", "Оружейник"),
        Profession::Miner => tr(ctx, "Miner", "Шахтёр"),
        Profession::Lumberjack => tr(ctx, "Lumberjack", "Лесоруб"),
        Profession::Builder => tr(ctx, "Builder", "Строитель"),
        Profession::Soldier => tr(ctx, "Soldier", "Солдат"),
    }
}

// ----------------------------------------------------------------------- icons

fn tr(ctx: &Ctx, en: &'static str, ru: &'static str) -> &'static str {
    match ctx.lang {
        Lang::En => en,
        Lang::Ru => ru,
    }
}

/// A literal text component.
fn text(label: impl Into<String>) -> Tag {
    Tag::Compound(vec![("text".to_owned(), Tag::String(label.into()))])
}

/// A component list (`extra`, lore lines): NBT lists hold one tag type.
fn compounds(elements: Vec<Tag>) -> Tag {
    Tag::List(mc_nbt::ListTag {
        element_type: mc_nbt::tag_type::COMPOUND,
        elements,
    })
}

fn item_id(state: &TickState, name: &str) -> Option<u32> {
    Identifier::parse(name)
        .ok()
        .and_then(|id| state.registry.items.id_of(&id))
}

/// A display icon: `count` of `item` named `name` with `lore` lines.
fn icon(state: &TickState, item: &str, count: i32, name: Tag, lore: &[String]) -> ItemStack {
    let Some(id) = item_id(state, item).or_else(|| item_id(state, "minecraft:paper")) else {
        return ItemStack::EMPTY;
    };
    decorate(state, ItemStack::new(id, count.clamp(1, 99)), name, lore)
}

/// Name (not italic, unlike an anvil name) and lore on a display copy, written
/// through the item codec as real `minecraft:custom_name` and `minecraft:lore`
/// components: `ItemStack::custom_name` is literal text on the wire.
fn decorate(state: &TickState, stack: ItemStack, name: Tag, lore: &[String]) -> ItemStack {
    let items = &state.registry.items;
    let Ok(mut fields) = mc_data::item_nbt::stack_to_fields(items, &stack) else {
        return stack;
    };
    let name = Tag::Compound(vec![
        ("text".to_owned(), Tag::String(String::new())),
        ("italic".to_owned(), Tag::Byte(0)),
        ("extra".to_owned(), compounds(vec![name])),
    ]);
    let mut set = vec![(CUSTOM_NAME_COMPONENT.to_owned(), name)];
    if !lore.is_empty() {
        let lines = lore.iter().take(MAX_LORE).map(|line| {
            Tag::Compound(vec![
                ("text".to_owned(), Tag::String(line.clone())),
                ("italic".to_owned(), Tag::Byte(0)),
                ("color".to_owned(), Tag::String("gray".to_owned())),
            ])
        });
        set.push(("minecraft:lore".to_owned(), compounds(lines.collect())));
    }
    match fields.iter_mut().find(|(key, _)| key == "components") {
        Some((_, Tag::Compound(components))) => {
            components.retain(|(key, _)| set.iter().all(|(name, _)| name != key));
            components.extend(set);
        }
        _ => fields.push(("components".to_owned(), Tag::Compound(set))),
    }
    match mc_data::item_nbt::stack_from_fields(items, &fields) {
        Ok(Some(decorated)) => decorated,
        _ => stack,
    }
}

/// The item's own name: its complete custom name component (styled names stay
/// whole), else the translated item name.
fn item_label(state: &TickState, stack: &ItemStack) -> Tag {
    let named = stack.custom_name.is_some()
        || stack
            .extended()
            .preserved_components
            .iter()
            .any(|component| component.name == CUSTOM_NAME_COMPONENT);
    if named && let Ok(fields) = mc_data::item_nbt::stack_to_fields(&state.registry.items, stack) {
        let name = fields
            .into_iter()
            .find(|(key, _)| key == "components")
            .and_then(|(_, components)| match components {
                Tag::Compound(components) => components
                    .into_iter()
                    .find(|(key, _)| key == CUSTOM_NAME_COMPONENT)
                    .map(|(_, name)| name),
                _ => None,
            });
        match name {
            Some(Tag::String(literal)) => return text(literal),
            Some(name) => return name,
            None => {}
        }
    }
    let Some(id) = state.registry.items.name_of(stack.item_id) else {
        return text("?");
    };
    let prefix = if state.registry.blocks.block(id).is_some() {
        "block"
    } else {
        "item"
    };
    Tag::Compound(vec![(
        "translate".to_owned(),
        Tag::String(format!("{prefix}.{}.{}", id.namespace(), id.path())),
    )])
}

#[cfg(test)]
#[path = "ui_menus_tests.rs"]
mod tests;
