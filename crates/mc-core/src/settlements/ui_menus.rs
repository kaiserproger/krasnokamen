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

/// Display carriers cache immutable names, never registry-specific wire IDs.
macro_rules! display_icons {
    ($( $kind:ident => $id:expr ),+ $(,)?) => {
        #[derive(Clone, Copy)]
        enum Icon { $( $kind, )+ }

        impl Icon {
            fn identifier(self) -> &'static Identifier {
                match self {
                    $( Self::$kind => {
                        static ID: std::sync::LazyLock<Identifier> = std::sync::LazyLock::new(|| {
                            Identifier::parse($id).expect("valid display icon id")
                        });
                        &ID
                    }, )+
                }
            }
        }
    };
}

display_icons! {
    Anvil => "minecraft:anvil",
    Arrow => "minecraft:arrow",
    Barrel => mc_data::block_entity_kinds::BlockEntityKind::Barrel.vanilla_id(),
    Barrier => "minecraft:barrier",
    Bell => mc_data::block_entity_kinds::BlockEntityKind::Bell.vanilla_id(),
    BlastFurnace => mc_data::block_entity_kinds::BlockEntityKind::BlastFurnace.vanilla_id(),
    Book => "minecraft:book",
    Bread => "minecraft:bread",
    BrewingStand => mc_data::block_entity_kinds::BlockEntityKind::BrewingStand.vanilla_id(),
    Bricks => "minecraft:bricks",
    CartographyTable => "minecraft:cartography_table",
    Cauldron => "minecraft:cauldron",
    Chest => mc_data::block_entity_kinds::BlockEntityKind::Chest.vanilla_id(),
    Clock => "minecraft:clock",
    CobblestoneWall => "minecraft:cobblestone_wall",
    Composter => "minecraft:composter",
    CraftingTable => "minecraft:crafting_table",
    ExperienceBottle => "minecraft:experience_bottle",
    FilledMap => "minecraft:filled_map",
    FletchingTable => "minecraft:fletching_table",
    GrayStainedGlassPane => "minecraft:gray_stained_glass_pane",
    Grindstone => "minecraft:grindstone",
    IronAxe => "minecraft:iron_axe",
    IronChestplate => "minecraft:iron_chestplate",
    IronHelmet => "minecraft:iron_helmet",
    IronIngot => "minecraft:iron_ingot",
    IronPickaxe => "minecraft:iron_pickaxe",
    IronSword => "minecraft:iron_sword",
    Lectern => mc_data::block_entity_kinds::BlockEntityKind::Lectern.vanilla_id(),
    LightGrayStainedGlassPane => "minecraft:light_gray_stained_glass_pane",
    LimeDye => "minecraft:lime_dye",
    LimeStainedGlassPane => "minecraft:lime_stained_glass_pane",
    Loom => "minecraft:loom",
    OakDoor => "minecraft:oak_door",
    OakFenceGate => "minecraft:oak_fence_gate",
    OakPlanks => "minecraft:oak_planks",
    Paper => "minecraft:paper",
    PlayerHead => "minecraft:player_head",
    RedBed => "minecraft:red_bed",
    Shield => "minecraft:shield",
    SmithingTable => "minecraft:smithing_table",
    Smoker => mc_data::block_entity_kinds::BlockEntityKind::Smoker.vanilla_id(),
    Spyglass => "minecraft:spyglass",
    Stonecutter => "minecraft:stonecutter",
    VillagerSpawnEgg => "minecraft:villager_spawn_egg",
    Wheat => "minecraft:wheat",
    WhiteBanner => "minecraft:white_banner",
    WritableBook => "minecraft:writable_book",
}

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

impl Open {
    pub(crate) fn snapshot(&self) -> (u8, &[ItemStack]) {
        (self.window, &self.icons)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Dashboard,
    Resources,
    Resource {
        group: u8,
        page: usize,
    },
    Projects {
        page: usize,
    },
    Project {
        page: usize,
    },
    Census {
        filter: u8,
        page: usize,
    },
    Resident {
        worker: Uuid,
    },
    Households {
        page: usize,
    },
    Household {
        id: Option<(u64, u64)>,
        page: usize,
    },
    Production {
        page: usize,
    },
    Recipes {
        building: BuildingId,
        at: [i32; 3],
        page: usize,
    },
    FilteredBuildings {
        kind: Option<BuildingKind>,
        page: usize,
    },
    BuildingContext {
        building: BuildingId,
        origin: Origin,
    },
    BuildingResidents {
        building: BuildingId,
        origin: Origin,
        page: usize,
    },
    Storage {
        page: usize,
    },
    Buildings {
        page: usize,
    },
    /// Workplaces for one resident, preserving the opening building's origin.
    Assign {
        worker: Uuid,
        from: Option<(BuildingId, Origin)>,
        page: usize,
    },
    /// Residents who may take a job in `building`.
    Hire {
        building: BuildingId,
        origin: Origin,
        page: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    List {
        kind: Option<BuildingKind>,
        page: usize,
    },
    Map {
        centre: [i32; 2],
        scale: i32,
    },
}

impl Origin {
    fn fields(self) -> Vec<(String, Tag)> {
        match self {
            Self::List { kind, page } => vec![
                ("from".to_owned(), Tag::String("list".to_owned())),
                (
                    "bk".to_owned(),
                    Tag::String(kind.map_or("", BuildingKind::name).to_owned()),
                ),
                ("bp".to_owned(), Tag::String(page.to_string())),
            ],
            Self::Map { centre, scale } => vec![
                ("from".to_owned(), Tag::String("map".to_owned())),
                ("mx".to_owned(), Tag::String(centre[0].to_string())),
                ("mz".to_owned(), Tag::String(centre[1].to_string())),
                ("ms".to_owned(), Tag::String(scale.to_string())),
            ],
        }
    }
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
        from: Option<(BuildingId, Origin)>,
    },
    Assign {
        worker: Uuid,
        building: BuildingId,
    },
    Blueprint(String),
    Material(String),
    StockDetail(ItemStack),
    MapAt([i32; 3]),
    Recipe {
        building: BuildingId,
        at: [i32; 3],
        recipe: String,
    },
    Job(String),
    MapView {
        centre: [i32; 2],
        scale: i32,
    },
    Review {
        building: BuildingId,
        origin: Origin,
        certificate: bool,
    },
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

pub(super) fn dashboard(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    show(state, connection, ctx, View::Dashboard);
}

pub(super) fn resources(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    show(state, connection, ctx, View::Resources);
}

pub(super) fn projects(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    show(state, connection, ctx, View::Projects { page: 0 });
}

pub(super) fn project(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    show(state, connection, ctx, View::Project { page: 0 });
}

pub(super) fn production(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    show(state, connection, ctx, View::Production { page: 0 });
}

pub(super) fn recipes(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    building: BuildingId,
    at: [i32; 3],
) {
    show(
        state,
        connection,
        ctx,
        View::Recipes {
            building,
            at,
            page: 0,
        },
    );
}

pub(super) fn production_workers(state: &TickState, id: SettlementId) -> Vec<(String, Tag)> {
    let busy = super::busy_workers(state, id);
    people(state, id)
        .into_iter()
        .filter(|person| {
            person.adult
                && person.soldier.is_none()
                && !matches!(person.profession, Profession::Nitwit | Profession::Builder)
                && !busy.contains(&person.uuid)
        })
        .map(|person| {
            (
                person.uuid.to_string(),
                Tag::String(super::super::presentation::villager_name(person.uuid).to_owned()),
            )
        })
        .collect()
}

/// The settlement's storage containers as one inventory.
pub(super) fn storage(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    show(state, connection, ctx, View::Storage { page: 0 });
}

/// Residents as icons; a click chooses the resident's workplace.
pub(super) fn residents(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    show(state, connection, ctx, View::Census { filter: 0, page: 0 });
}

/// Certified buildings as icons; a click opens the building's page.
pub(super) fn buildings(state: &mut TickState, connection: ConnectionId, ctx: &Ctx) {
    show(state, connection, ctx, View::Buildings { page: 0 });
}

pub(super) fn building_from(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    building: BuildingId,
    payload: ClickPayload<'_>,
) {
    let origin = if payload.string("from") == Some("map") {
        Origin::Map {
            centre: ["mx", "mz"].map(|key| {
                payload
                    .string(key)
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0)
            }),
            scale: payload
                .string("ms")
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
        }
    } else {
        Origin::List {
            kind: payload.string("bk").and_then(BuildingKind::parse),
            page: payload
                .string("bp")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
        }
    };
    show(
        state,
        connection,
        ctx,
        View::BuildingContext { building, origin },
    );
}

pub(super) fn filtered_buildings(
    state: &mut TickState,
    connection: ConnectionId,
    ctx: &Ctx,
    payload: ClickPayload<'_>,
) {
    show(
        state,
        connection,
        ctx,
        View::FilteredBuildings {
            kind: payload.string("bk").and_then(BuildingKind::parse),
            page: payload
                .string("bp")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
        },
    );
}

pub(super) fn storage_label(ctx: &Ctx) -> &'static str {
    tr(ctx, "Storage", "Склад")
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
        Target::MapView { centre, scale } => {
            close(state, connection, entity, true);
            let payload = Tag::Compound(Origin::Map { centre, scale }.fields());
            super::dispatch(state, connection, &ctx, "map", ClickPayload(Some(&payload)));
        }
        Target::Review {
            building,
            origin,
            certificate,
        } => {
            close(state, connection, entity, true);
            let mut fields = origin.fields();
            fields.push(("b".to_owned(), Tag::String(building.0.to_string())));
            let payload = Tag::Compound(fields);
            super::dispatch(
                state,
                connection,
                &ctx,
                if certificate {
                    "certificate_review"
                } else {
                    "project_upgrade_review"
                },
                ClickPayload(Some(&payload)),
            );
        }
        Target::Recipe {
            building,
            at,
            recipe,
        } => {
            close(state, connection, entity, true);
            let payload = Tag::Compound(vec![
                ("b".to_owned(), Tag::String(building.0.to_string())),
                ("p".to_owned(), Tag::String(super::corner_text(at))),
                ("r".to_owned(), Tag::String(recipe)),
            ]);
            super::dispatch(
                state,
                connection,
                &ctx,
                "recipe_review",
                ClickPayload(Some(&payload)),
            );
        }
        Target::Job(job) => {
            close(state, connection, entity, true);
            super::production_detail(state, connection, &ctx, &job, "");
        }
        Target::MapAt(at) => {
            close(state, connection, entity, true);
            let payload = Tag::Compound(vec![
                ("mx".to_owned(), Tag::String(at[0].to_string())),
                ("mz".to_owned(), Tag::String(at[2].to_string())),
                ("ms".to_owned(), Tag::String("1".to_owned())),
            ]);
            super::dispatch(state, connection, &ctx, "map", ClickPayload(Some(&payload)));
        }
        Target::Blueprint(blueprint) => {
            close(state, connection, entity, true);
            let payload = Tag::Compound(vec![("blueprint".to_owned(), Tag::String(blueprint))]);
            super::dispatch(
                state,
                connection,
                &ctx,
                "project_order_review",
                ClickPayload(Some(&payload)),
            );
        }
        Target::Material(item) => {
            close(state, connection, entity, true);
            super::material_detail(state, connection, &ctx, &item);
        }
        Target::StockDetail(template) => {
            close(state, connection, entity, true);
            let stores = stores(state, ctx.id);
            let entry = stock(&stores)
                .into_iter()
                .find(|entry| can_stack(&entry.template, &template));
            let text = entry.map_or_else(
                || tr(&ctx, "Stack no longer present.", "Этого стака больше нет.").to_owned(),
                |entry| {
                    format!(
                        "{} / {} / {}",
                        entry.total,
                        entry.total - entry.free,
                        entry.free
                    )
                },
            );
            let text = if stores.unavailable {
                format!(
                    "{}\n{text}",
                    tr(
                        &ctx,
                        "Incomplete — approach storage; totals unknown",
                        "Неполные данные — подойдите к складу; итог неизвестен"
                    )
                )
            } else {
                text
            };
            let dialog = crate::dialog::Dialog::multi_action(
                tr(
                    &ctx,
                    "Stored / reserved / free",
                    "Запас / резерв / свободно",
                ),
                vec![
                    super::button(storage_label(&ctx), "storage", ctx.id),
                    super::button(ctx.t(Key::Back), "resources", ctx.id),
                ],
            )
            .text(item_label(state, &template))
            .text(text);
            super::present(state, connection, dialog);
        }
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
                        View::Hire {
                            building, origin, ..
                        }
                        | View::Assign {
                            from: Some((building, origin)),
                            ..
                        } => View::BuildingContext { building, origin },
                        _ => View::Resident { worker },
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
    unavailable: bool,
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
        unavailable: false,
    };
    let mut physical = BTreeSet::new();
    let points = buildings::storage_points(state, id);
    stores.unavailable = points.len() > MAX_CONTAINERS;
    for position in points.into_iter().take(MAX_CONTAINERS) {
        let Ok(cells) = crate::plugin_inventories::container_cells(
            state,
            &mc_script::ScriptBlockPosition::new(position[0], position[1], position[2]),
        ) else {
            stores.unavailable = true;
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
            stores.unavailable = true;
            continue;
        };
        physical.extend(cells.iter().map(|cell| [cell.x, cell.y, cell.z]));
        // Unreadable floors fail closed: nothing in that container is free.
        let free = match inventory::unreserved_stock(state, &image) {
            Ok(free) => free,
            Err(_) => {
                stores.unavailable = true;
                BTreeMap::new()
            }
        };
        stores.free.push(free);
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
    home: Option<BlockPos>,
    household: Option<(u64, u64)>,
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
                home: villager
                    .home
                    .filter(|_| villager.home_dimension == state.dimension.name()),
                household: villager.household.as_deref().and_then(|id| {
                    let (founder, sequence) = id.split_once('-')?;
                    Some((founder.parse().ok()?, sequence.parse().ok()?))
                }),
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
        View::Dashboard => (view, dashboard_screen(state, ctx)),
        View::Resources => (view, resources_screen(state, ctx)),
        View::Resource { group, page } => (view, resource_screen(state, ctx, group, page)),
        View::Projects { page } => (view, projects_screen(state, ctx, page)),
        View::Project { page } => (view, project_screen(state, ctx, page)),
        View::Census { filter, page } => (view, census_screen(state, ctx, filter, page)),
        View::Resident { worker } => (view, resident_screen(state, ctx, worker)),
        View::Households { page } => (view, households_screen(state, ctx, page)),
        View::Household { id, page } => (view, household_screen(state, ctx, id, page)),
        View::Production { page } => (view, production_screen(state, ctx, page)),
        View::Recipes { building, at, page } => {
            (view, recipes_screen(state, ctx, building, at, page))
        }
        View::Storage { page } => (view, storage_screen(state, ctx, &name, page)),
        View::Buildings { page } => (view, buildings_screen(state, ctx, &name, page)),
        View::FilteredBuildings { kind, page } => (
            view,
            filtered_buildings_screen(state, ctx, &name, kind, page),
        ),
        View::BuildingContext { building, origin } => {
            (view, building_context_screen(state, ctx, building, origin))
        }
        View::BuildingResidents {
            building,
            origin,
            page,
        } => (
            view,
            building_residents_screen(state, ctx, building, origin, page),
        ),
        View::Assign { worker, from, page } => {
            match assign_screen(state, ctx, worker, from, page) {
                Some(screen) => (view, screen),
                None => (
                    View::Census { filter: 0, page: 0 },
                    census_screen(state, ctx, 0, 0),
                ),
            }
        }
        View::Hire {
            building,
            origin,
            page,
        } => match hire_screen(state, ctx, building, origin, page) {
            Some(screen) => (view, screen),
            None => (
                View::BuildingContext { building, origin },
                building_context_screen(state, ctx, building, origin),
            ),
        },
    }
}

fn building_context_screen(
    state: &TickState,
    ctx: &Ctx,
    building: BuildingId,
    origin: Origin,
) -> Screen {
    let back_target = match origin {
        Origin::List { kind, page } => Target::Show(View::FilteredBuildings { kind, page }),
        Origin::Map { centre, scale } => Target::MapView { centre, scale },
    };
    let Some(mut screen) = building_screen(state, ctx, building, origin) else {
        let mut screen = Screen::new(ctx.t(Key::BuildingGone).to_owned());
        screen.put(BACK, back(state, ctx), back_target);
        return screen;
    };
    screen.targets[BACK] = back_target;
    screen.put(
        INFO,
        icon(
            state,
            Icon::Book,
            1,
            text(ctx.t(Key::CertificateCheck)),
            &ctx.t(Key::CertificatePolicy)
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        ),
        Target::Review {
            building,
            origin,
            certificate: true,
        },
    );
    screen
}

fn has_home_in(person: &Person, building: &Building) -> bool {
    person.home.is_some_and(|home| {
        [home.x, home.y, home.z]
            .iter()
            .enumerate()
            .all(|(axis, value)| {
                (building.bounds.min[axis]..=building.bounds.max[axis]).contains(value)
            })
    })
}

fn building_residents_screen(
    state: &TickState,
    ctx: &Ctx,
    building: BuildingId,
    origin: Origin,
    page: usize,
) -> Screen {
    let Some(home) = state
        .settlements
        .get(ctx.id)
        .and_then(|record| record.buildings.get(&building))
    else {
        return building_context_screen(state, ctx, building, origin);
    };
    let people = people(state, ctx.id);
    let residents: Vec<_> = people
        .iter()
        .filter(|person| has_home_in(person, home))
        .collect();
    let pages = pages(residents.len());
    let page = page.min(pages - 1);
    let busy = super::busy_workers(state, ctx.id);
    let mut screen = Screen::new(tr(ctx, "Home claims", "Назначенные дома").to_owned());
    for (slot, person) in residents.iter().skip(page * PAGE).take(PAGE).enumerate() {
        screen.put(
            slot,
            person_icon(state, ctx, person, &busy, ""),
            Target::Show(View::Resident {
                worker: person.uuid,
            }),
        );
    }
    screen.put(
        INFO,
        icon(
            state,
            Icon::RedBed,
            1,
            text(format!(
                "{}: {}",
                tr(ctx, "Home claims", "Назначенные дома"),
                residents.len()
            )),
            &[tr(
                ctx,
                "Bed validity and free capacity unavailable.",
                "Исправность кроватей и свободная вместимость недоступны.",
            )
            .to_owned()],
        ),
        Target::Refresh,
    );
    navigation(&mut screen, state, ctx, page, pages, |page| {
        View::BuildingResidents {
            building,
            origin,
            page,
        }
    });
    screen.put(
        BACK,
        back(state, ctx),
        Target::Show(View::BuildingContext { building, origin }),
    );
    screen
}

fn production_screen(state: &TickState, ctx: &Ctx, page: usize) -> Screen {
    let mut screen = Screen::new(ctx.t(Key::ProductionOrders).to_owned());
    let Some(record) = state.settlements.get(ctx.id) else {
        return screen;
    };
    let mut jobs: Vec<_> = record.economy.production.jobs.values().collect();
    jobs.sort_by_key(|job| (!job.unfinished(), job.accepted_tick, &job.id));
    let pages = pages(jobs.len());
    let page = page.min(pages - 1);
    for (slot, job) in jobs.into_iter().skip(page * PAGE).take(PAGE).enumerate() {
        let output = Identifier::parse(&job.recipe)
            .ok()
            .and_then(|id| state.registry.item_facts.work_recipe(&id))
            .and_then(|recipe| recipe.outputs.first())
            .and_then(|output| output.to_stack(&state.registry.items, &state.registry.item_facts))
            .unwrap_or_else(|| icon(state, Icon::Barrier, 1, text(ctx.t(Key::StateChanged)), &[]));
        screen.put(
            slot,
            decorate(
                state,
                output,
                text(format!("{} · {}/{}", job.id, job.done, job.count)),
                &[
                    super::job_status(ctx, job),
                    super::super::presentation::villager_name(job.worker).to_owned(),
                ],
            ),
            Target::Job(job.id.clone()),
        );
    }
    navigation(&mut screen, state, ctx, page, pages, |page| {
        View::Production { page }
    });
    if ctx.manage {
        screen.put(
            ACTION,
            icon(
                state,
                Icon::CraftingTable,
                1,
                text(ctx.t(Key::NewOrder)),
                &[],
            ),
            Target::Dialog("order_start"),
        );
    }
    screen.put(BACK, back(state, ctx), Target::Show(View::Dashboard));
    screen
}

fn recipes_screen(
    state: &TickState,
    ctx: &Ctx,
    building: BuildingId,
    at: [i32; 3],
    page: usize,
) -> Screen {
    let mut screen = Screen::new(ctx.t(Key::TitleNewOrder).to_owned());
    let recipes: Vec<_> = state
        .registry
        .item_facts
        .work_recipes()
        .filter(|recipe| Some(recipe.station.as_str()) == super::block_name(state, at))
        .collect();
    let pages = pages(recipes.len());
    let page = page.min(pages - 1);
    for (slot, recipe) in recipes.into_iter().skip(page * PAGE).take(PAGE).enumerate() {
        let Some(output) = recipe
            .outputs
            .first()
            .and_then(|output| output.to_stack(&state.registry.items, &state.registry.item_facts))
        else {
            continue;
        };
        screen.put(
            slot,
            decorate(
                state,
                output,
                text(recipe.id.path()),
                &[super::corner_text(at)],
            ),
            Target::Recipe {
                building,
                at,
                recipe: recipe.id.as_str().to_owned(),
            },
        );
    }
    navigation(&mut screen, state, ctx, page, pages, |page| View::Recipes {
        building,
        at,
        page,
    });
    screen.put(
        BACK,
        back(state, ctx),
        Target::Show(View::Production { page: 0 }),
    );
    screen
}

fn dashboard_screen(state: &TickState, ctx: &Ctx) -> Screen {
    let mut screen = Screen::new(ctx.t(Key::Settlement).to_owned());
    let Some(record) = state.settlements.get(ctx.id) else {
        return screen;
    };
    screen.put(
        4,
        icon(
            state,
            Icon::Bell,
            1,
            text(&record.name),
            &[record
                .owner_name
                .as_deref()
                .or(record.owner.as_deref())
                .unwrap_or(ctx.t(Key::Autonomous))
                .to_owned()],
        ),
        Target::Dialog("settings"),
    );
    for (slot, item, key, route) in [
        (10, Icon::FilledMap, Key::Map, "map"),
        (12, Icon::OakDoor, Key::Buildings, "buildings"),
        (14, Icon::PlayerHead, Key::ResidentsJobs, "residents"),
        (16, Icon::Chest, Key::KindStorage, "resources"),
        (28, Icon::CraftingTable, Key::ProductionOrders, "orders"),
        (30, Icon::IronPickaxe, Key::Projects, "projects"),
        (32, Icon::IronHelmet, Key::Army, "army"),
        (34, Icon::WhiteBanner, Key::Neighbours, "neighbours"),
        (49, Icon::Book, Key::Checklist, "checklist"),
        (52, Icon::LimeDye, Key::NextStep, "next"),
        (53, Icon::Book, Key::Help, "help"),
    ] {
        screen.put(
            slot,
            icon(state, item, 1, text(ctx.t(key)), &[]),
            Target::Dialog(route),
        );
    }
    screen.put(BACK, back(state, ctx), Target::Dialog("main"));
    screen
}

fn resource_group(state: &TickState, stack: &ItemStack, group: u8) -> bool {
    let Some(name) = state.registry.items.name_of(stack.item_id) else {
        return false;
    };
    let name = name.path();
    match group {
        0 => matches!(
            name,
            "bread"
                | "wheat"
                | "carrot"
                | "potato"
                | "beetroot"
                | "apple"
                | "cooked_beef"
                | "cooked_porkchop"
                | "cooked_mutton"
                | "cooked_chicken"
                | "cooked_cod"
                | "cooked_salmon"
        ),
        1 => {
            name.ends_with("_planks")
                || name.ends_with("_log")
                || name.ends_with("_wood")
                || name.ends_with("_stone")
                || name.ends_with("_bricks")
                || matches!(
                    name,
                    "stone" | "cobblestone" | "dirt" | "sand" | "glass" | "gravel"
                )
        }
        2 => {
            ["_pickaxe", "_axe", "_shovel", "_hoe"]
                .iter()
                .any(|suffix| name.ends_with(suffix))
                || matches!(name, "shears" | "fishing_rod")
        }
        3 => {
            name.ends_with("_ingot")
                || name.ends_with("_nugget")
                || matches!(name, "diamond" | "emerald" | "netherite_scrap")
        }
        4 => ["_helmet", "_chestplate", "_leggings", "_boots"]
            .iter()
            .any(|suffix| name.ends_with(suffix)),
        5 => {
            name.ends_with("_sword")
                || name.ends_with("_spear")
                || name.ends_with("_arrow")
                || matches!(
                    name,
                    "bow" | "crossbow" | "trident" | "mace" | "arrow" | "shield"
                )
        }
        _ => true,
    }
}

fn resources_screen(state: &TickState, ctx: &Ctx) -> Screen {
    let mut screen = Screen::new(tr(ctx, "Resources", "Ресурсы").to_owned());
    let stores = stores(state, ctx.id);
    let stock = stock(&stores);
    for (slot, item, en, ru, group) in [
        (10, Icon::Bread, "Food", "Еда", 0),
        (
            12,
            Icon::OakPlanks,
            "Building materials",
            "Стройматериалы",
            1,
        ),
        (14, Icon::IronPickaxe, "Tools", "Инструменты", 2),
        (16, Icon::IronIngot, "Metals", "Металлы", 3),
        (28, Icon::IronChestplate, "Armour", "Броня", 4),
        (
            30,
            Icon::IronSword,
            "Weapons / ammunition",
            "Оружие / боезапас",
            5,
        ),
        (32, Icon::Chest, "All stock", "Все запасы", 6),
        (34, Icon::Clock, "Reservations", "Резервы", 7),
    ] {
        let entries: Vec<_> = stock
            .iter()
            .filter(|entry| {
                resource_group(state, &entry.template, group)
                    && (group != 7 || entry.free < entry.total)
            })
            .collect();
        let total: u64 = entries.iter().map(|entry| entry.total).sum();
        let free: u64 = entries.iter().map(|entry| entry.free).sum();
        let lore = if stores.unavailable {
            vec![
                tr(
                    ctx,
                    "Unavailable — approach storage",
                    "Недоступно — подойдите к складу",
                )
                .to_owned(),
                tr(
                    ctx,
                    "Partial readable stock only; totals unknown.",
                    "Только доступные запасы; итог неизвестен.",
                )
                .to_owned(),
            ]
        } else {
            vec![
                format!("{total} / {} / {free}", total - free),
                tr(ctx, "Stored / reserved / free", "Запас / резерв / свободно").to_owned(),
            ]
        };
        screen.put(
            slot,
            icon(state, item, 1, text(tr(ctx, en, ru)), &lore),
            Target::Show(View::Resource { group, page: 0 }),
        );
    }
    screen.put(
        INFO,
        icon(
            state,
            Icon::Book,
            1,
            text(storage_label(ctx)),
            &[tr(
                ctx,
                "Open storage for real stack transfers.",
                "Откройте склад для переноса настоящих стаков.",
            )
            .to_owned()],
        ),
        Target::Dialog("storage"),
    );
    screen.put(BACK, back(state, ctx), Target::Show(View::Dashboard));
    screen.put(
        ACTION,
        icon(state, Icon::Chest, 1, text(storage_label(ctx)), &[]),
        Target::Dialog("storage"),
    );
    screen
}

fn resource_screen(state: &TickState, ctx: &Ctx, group: u8, page: usize) -> Screen {
    let stores = stores(state, ctx.id);
    let stock: Vec<_> = stock(&stores)
        .into_iter()
        .filter(|entry| {
            resource_group(state, &entry.template, group)
                && (group != 7 || entry.free < entry.total)
        })
        .collect();
    let pages = pages(stock.len());
    let page = page.min(pages - 1);
    let mut screen = Screen::new(tr(ctx, "Stock · details", "Запасы · детали").to_owned());
    if stores.unavailable {
        screen.put(
            INFO,
            icon(
                state,
                Icon::Barrier,
                1,
                text(tr(
                    ctx,
                    "Unavailable — approach storage",
                    "Недоступно — подойдите к складу",
                )),
                &[tr(
                    ctx,
                    "Partial readable stock only; totals unknown.",
                    "Только доступные запасы; итог неизвестен.",
                )
                .to_owned()],
            ),
            Target::Dialog("storage"),
        );
    }
    for (slot, entry) in stock.iter().skip(page * PAGE).take(PAGE).enumerate() {
        let mut shown = entry.template.clone();
        shown.count = entry.total.min(99) as i32;
        screen.put(
            slot,
            decorate(
                state,
                shown,
                item_label(state, &entry.template),
                &[
                    format!(
                        "{} / {} / {}",
                        entry.total,
                        entry.total - entry.free,
                        entry.free
                    ),
                    tr(
                        ctx,
                        "Stored / reserved / free; click for exact detail",
                        "Запас / резерв / свободно; клик — точные данные",
                    )
                    .to_owned(),
                ],
            ),
            Target::StockDetail(entry.template.clone()),
        );
    }
    navigation(&mut screen, state, ctx, page, pages, |page| {
        View::Resource { group, page }
    });
    screen.put(BACK, back(state, ctx), Target::Show(View::Resources));
    screen.put(
        ACTION,
        icon(state, Icon::Chest, 1, text(storage_label(ctx)), &[]),
        Target::Dialog("storage"),
    );
    screen
}

fn projects_screen(state: &TickState, ctx: &Ctx, page: usize) -> Screen {
    let blueprints = super::super::blueprints::all(&state.registry);
    let pages = pages(blueprints.len());
    let page = page.min(pages - 1);
    let mut screen = Screen::new(ctx.t(Key::Projects).to_owned());
    for (slot, blueprint) in blueprints.iter().skip(page * PAGE).take(PAGE).enumerate() {
        screen.put(
            slot,
            icon(
                state,
                kind_item(blueprint.kind),
                1,
                text(format!("{} L1", super::blueprint_label(ctx, &blueprint.id))),
                &ctx.t(Key::RequestPolicy)
                    .lines()
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            ),
            Target::Blueprint(blueprint.id.clone()),
        );
    }
    let access = (!ctx.manage).then(|| ctx.t(Key::ViewOnly).to_owned());
    screen.put(
        INFO,
        icon(
            state,
            Icon::Book,
            1,
            text(ctx.t(Key::Status)),
            access.as_slice(),
        ),
        Target::Show(View::Project { page: 0 }),
    );
    screen.put(
        ACTION,
        icon(state, Icon::IronPickaxe, 1, text(ctx.t(Key::Status)), &[]),
        Target::Show(View::Project { page: 0 }),
    );
    screen.put(BACK, back(state, ctx), Target::Show(View::Dashboard));
    navigation(&mut screen, state, ctx, page, pages, |page| {
        View::Projects { page }
    });
    screen
}

fn project_screen(state: &TickState, ctx: &Ctx, page: usize) -> Screen {
    let mut screen = Screen::new(tr(ctx, "Construction", "Строительство").to_owned());
    let Some(record) = state.settlements.get(ctx.id) else {
        return screen;
    };
    let mut status = super::pending_order_text(state, ctx).unwrap_or_else(|| {
        if record.project.is_some() {
            tr(ctx, "No queued order", "Нет заказа в очереди")
        } else {
            tr(
                ctx,
                "No active project; no queued order",
                "Нет активной стройки и заказа в очереди",
            )
        }
        .to_owned()
    });
    if let Some(project) = &record.project {
        screen.put(
            4,
            icon(
                state,
                super::super::blueprints::get(&state.registry, &project.blueprint)
                    .map_or(Icon::IronPickaxe, |bp| kind_item(bp.kind)),
                1,
                text(format!(
                    "{} L{}",
                    super::blueprint_label(ctx, &project.blueprint),
                    project.level
                )),
                &[],
            ),
            Target::Refresh,
        );
        let builder = project
            .borrowed_builder
            .map(|(worker, _)| super::super::presentation::villager_name(worker).to_owned())
            .unwrap_or_else(|| {
                tr(
                    ctx,
                    "Builder cause unavailable",
                    "Причина состояния строителя недоступна",
                )
                .to_owned()
            });
        screen.put(
            12,
            icon(state, Icon::IronPickaxe, 1, text(builder), &[]),
            Target::Dialog("residents"),
        );
        screen.put(
            14,
            icon(
                state,
                Icon::FilledMap,
                1,
                text(ctx.t(Key::Map)),
                &[super::corner_text(project.origin)],
            ),
            Target::MapAt(project.origin),
        );
        match crate::plugin_structures::settlement_builds::status(state, ctx.id, &project.structure)
        {
            Ok(snapshot) => {
                let total: u64 = snapshot.stages.iter().map(|stage| stage.work_units).sum();
                let done = snapshot
                    .stages
                    .iter()
                    .take(snapshot.current_stage_index as usize)
                    .map(|stage| stage.work_units)
                    .sum::<u64>()
                    .saturating_add(snapshot.completed_work_units);
                let phase = super::project_phase(ctx, snapshot.state.as_str());
                status = format!("{phase} · {done}/{total}\n{status}");
                if let Some(reason) = &snapshot.pause_reason {
                    status.push('\n');
                    status.push_str(reason);
                }
                for segment in 0..9 {
                    let full = total > 0
                        && u128::from(done) * 9 >= u128::from(total) * (segment + 1) as u128;
                    screen.put(
                        18 + segment,
                        icon(
                            state,
                            if full {
                                Icon::LimeStainedGlassPane
                            } else {
                                Icon::GrayStainedGlassPane
                            },
                            1,
                            text(format!("{done}/{total}")),
                            &[],
                        ),
                        Target::Refresh,
                    );
                }
            }
            Err(_) => {
                status = tr(
                    ctx,
                    "Project records unavailable; cause unknown.",
                    "Данные стройки недоступны; причина неизвестна.",
                )
                .to_owned()
            }
        }
    } else if let Some(order) = &record.pending_order {
        screen.put(
            4,
            icon(
                state,
                super::super::blueprints::get(&state.registry, &order.blueprint)
                    .map_or(Icon::IronPickaxe, |bp| kind_item(bp.kind)),
                1,
                text(format!(
                    "{} L{}",
                    super::blueprint_label(ctx, &order.blueprint),
                    order.level
                )),
                &[],
            ),
            Target::Refresh,
        );
        status = format!(
            "{}\n{}",
            tr(
                ctx,
                "Queued — site not assigned",
                "В очереди — место не назначено"
            ),
            status
        );
    }
    screen.put(
        10,
        icon(state, Icon::Clock, 1, text(status), &[]),
        Target::Dialog("project_status_details"),
    );
    screen.put(
        16,
        icon(state, Icon::Chest, 1, text(storage_label(ctx)), &[]),
        Target::Dialog("storage"),
    );
    let mut known = true;
    let needs = match super::project_needs(state, ctx) {
        Ok(needs) => needs,
        Err(error) => {
            known = false;
            screen.put(
                27,
                icon(
                    state,
                    Icon::Barrier,
                    1,
                    text(tr(ctx, "Supply unavailable", "Снабжение недоступно")),
                    &[error],
                ),
                Target::Dialog("project_status_details"),
            );
            Vec::new()
        }
    };
    let pages = needs.len().div_ceil(18).max(1);
    let page = page.min(pages - 1);
    for (slot, (item, missing, reason, _)) in (27..45).zip(needs.iter().skip(page * 18).take(18)) {
        screen.put(
            slot,
            item_icon(
                state,
                item,
                (*missing).min(99) as i32,
                super::material_text(state, item, format!(" ×{missing}")),
                std::slice::from_ref(reason),
            ),
            Target::Material(item.clone()),
        );
    }
    navigation(&mut screen, state, ctx, page, pages, |page| View::Project {
        page,
    });
    screen.put(
        INFO,
        icon(state, Icon::Book, 1, text(ctx.t(Key::Status)), &[]),
        Target::Dialog("project_status_details"),
    );
    let funded = known && needs.is_empty() && record.project.is_some();
    screen.put(
        ACTION,
        icon(
            state,
            if funded { Icon::FilledMap } else { Icon::Chest },
            1,
            text(if funded {
                ctx.t(Key::Map)
            } else {
                storage_label(ctx)
            }),
            &[],
        ),
        if funded {
            record
                .project
                .as_ref()
                .map_or(Target::Refresh, |project| Target::MapAt(project.origin))
        } else {
            Target::Dialog("storage")
        },
    );
    screen.put(
        LIST,
        icon(state, Icon::Book, 1, text(ctx.t(Key::Projects)), &[]),
        Target::Show(View::Projects { page: 0 }),
    );
    screen.put(
        BACK,
        back(state, ctx),
        Target::Show(View::Projects { page: 0 }),
    );
    screen
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
    let (title, mut lore) = if stores.unavailable {
        (
            tr(
                ctx,
                "Unavailable — approach storage",
                "Недоступно — подойдите к складу",
            )
            .to_owned(),
            vec![
                tr(
                    ctx,
                    "Only loaded readable containers are shown; totals are incomplete.",
                    "Показаны только доступные хранилища; итоги неполные.",
                )
                .to_owned(),
            ],
        )
    } else if stores.images.is_empty() {
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
        icon(state, Icon::Chest, 1, text(title), &lore),
        Target::Refresh,
    );
    screen.put(BACK, back(state, ctx), Target::Dialog("main"));
    navigation(&mut screen, state, ctx, page, pages, |page| View::Storage {
        page,
    });
    screen
}

fn census_match(person: &Person, filter: u8, busy: &BTreeSet<Uuid>) -> bool {
    match filter {
        1 => person.adult,
        2 => !person.adult,
        3 => person.home.is_none() && person.soldier.is_none(),
        4 => person.building.is_some(),
        5 => {
            person.adult
                && person.soldier.is_none()
                && person.building.is_none()
                && !matches!(person.profession, Profession::Nitwit | Profession::Builder)
                && !busy.contains(&person.uuid)
        }
        6 => person.soldier.is_some(),
        _ => true,
    }
}

fn census_screen(state: &TickState, ctx: &Ctx, filter: u8, page: usize) -> Screen {
    let people = people(state, ctx.id);
    let busy = super::busy_workers(state, ctx.id);
    let selected: Vec<_> = people
        .iter()
        .filter(|person| census_match(person, filter, &busy))
        .collect();
    let pages = selected.len().div_ceil(36).max(1);
    let page = page.min(pages - 1);
    let mut screen =
        Screen::new(tr(ctx, "Residents / households", "Жители / домохозяйства").to_owned());
    for (slot, item, label, which) in [
        (0, Icon::PlayerHead, tr(ctx, "Population", "Население"), 0),
        (
            2,
            Icon::Wheat,
            tr(ctx, "Adults / children", "Взрослые / дети"),
            if filter == 1 { 2 } else { 1 },
        ),
        (
            4,
            Icon::RedBed,
            tr(ctx, "Civilian beds", "Гражданские кровати"),
            3,
        ),
        (
            6,
            Icon::CraftingTable,
            tr(ctx, "Working / eligible idle", "Работают / свободны"),
            if filter == 4 { 5 } else { 4 },
        ),
        (8, Icon::IronHelmet, ctx.t(Key::Army), 6),
    ] {
        let count = people
            .iter()
            .filter(|person| census_match(person, which, &busy))
            .count();
        let lore = if slot == 2 {
            let adults = people.iter().filter(|person| person.adult).count();
            vec![
                format!("{adults} / {}", people.len() - adults),
                tr(ctx, "Click: adults / children", "Клик — взрослые / дети").to_owned(),
            ]
        } else if slot == 6 {
            let employed = people
                .iter()
                .filter(|person| census_match(person, 4, &busy))
                .count();
            let idle = people
                .iter()
                .filter(|person| census_match(person, 5, &busy))
                .count();
            vec![
                format!("{employed} / {idle}"),
                tr(
                    ctx,
                    "Click: working / eligible idle",
                    "Клик — работают / свободны",
                )
                .to_owned(),
            ]
        } else if slot == 4 {
            let claims = people
                .iter()
                .filter(|person| person.soldier.is_none() && person.home.is_some())
                .count();
            vec![
                format!(
                    "{}: {claims}",
                    tr(ctx, "Known home claims", "Известных домашних мест")
                ),
                tr(
                    ctx,
                    "Bed validity / free capacity unavailable here; inspect the home.",
                    "Проверка кроватей / свободные места здесь недоступны; осмотрите дом.",
                )
                .to_owned(),
            ]
        } else {
            vec![
                count.to_string(),
                tr(ctx, "Click: filter", "Клик — фильтр").to_owned(),
            ]
        };
        screen.put(
            slot,
            icon(state, item, 1, text(label), &lore),
            Target::Show(View::Census {
                filter: which,
                page: 0,
            }),
        );
    }
    for (slot, person) in (9..45).zip(selected.into_iter().skip(page * 36).take(36)) {
        screen.put(
            slot,
            person_icon(
                state,
                ctx,
                person,
                &busy,
                tr(ctx, "Click: resident card", "Клик — карточка жителя"),
            ),
            Target::Show(View::Resident {
                worker: person.uuid,
            }),
        );
    }
    navigation(&mut screen, state, ctx, page, pages, |page| View::Census {
        filter,
        page,
    });
    screen.put(BACK, back(state, ctx), Target::Show(View::Dashboard));
    screen.put(
        LIST,
        icon(
            state,
            Icon::RedBed,
            1,
            text(tr(ctx, "Households", "Домохозяйства")),
            &[],
        ),
        Target::Show(View::Households { page: 0 }),
    );
    screen
}

fn resident_screen(state: &TickState, ctx: &Ctx, worker: Uuid) -> Screen {
    let mut screen = Screen::new(tr(ctx, "Resident · details", "Житель · сведения").to_owned());
    let people = people(state, ctx.id);
    let Some(person) = people.iter().find(|person| person.uuid == worker) else {
        screen.put(
            4,
            icon(state, Icon::Barrier, 1, text(ctx.t(Key::StateChanged)), &[]),
            Target::Show(View::Census { filter: 0, page: 0 }),
        );
        screen.put(
            BACK,
            back(state, ctx),
            Target::Show(View::Census { filter: 0, page: 0 }),
        );
        return screen;
    };
    screen.title = format!(
        "{} · {}",
        tr(ctx, "Resident", "Житель"),
        super::super::presentation::villager_name(worker)
    );
    let busy = super::busy_workers(state, ctx.id);
    screen.put(
        4,
        person_icon(state, ctx, person, &busy, &worker.to_string()),
        Target::Refresh,
    );
    screen.put(
        10,
        icon(
            state,
            Icon::CraftingTable,
            1,
            text(profession_name(ctx, person.profession)),
            &[tr(
                ctx,
                "Assignment is not arrival; craft changes at the workstation.",
                "Назначение не означает прибытие; ремесло меняется у рабочего места.",
            )
            .to_owned()],
        ),
        if ctx.manage {
            Target::Pick { worker, from: None }
        } else {
            Target::Refresh
        },
    );
    for (slot, item, label, position) in [
        (
            12,
            Icon::RedBed,
            tr(ctx, "Home / bed", "Дом / кровать"),
            person.home,
        ),
        (
            14,
            Icon::FilledMap,
            tr(ctx, "Workplace", "Место работы"),
            person.job,
        ),
    ] {
        let detail = position
            .map(|at| super::corner_text([at.x, at.y, at.z]))
            .unwrap_or_else(|| {
                tr(
                    ctx,
                    "Not assigned in this dimension",
                    "Не назначено в этом измерении",
                )
                .to_owned()
            });
        screen.put(
            slot,
            icon(state, item, 1, text(label), &[detail]),
            position.map_or(Target::Refresh, |at| Target::MapAt([at.x, at.y, at.z])),
        );
    }
    screen.put(
        16,
        icon(
            state,
            Icon::Book,
            1,
            text(tr(ctx, "Household", "Домохозяйство")),
            &[tr(
                ctx,
                "Social home is not a bed claim; no family tree is inferred.",
                "Социальный дом не является кроватью; родство не выводится.",
            )
            .to_owned()],
        ),
        Target::Show(View::Household {
            id: person.household,
            page: 0,
        }),
    );
    screen.put(
        BACK,
        back(state, ctx),
        Target::Show(View::Census { filter: 0, page: 0 }),
    );
    screen.put(
        ACTION,
        icon(
            state,
            Icon::CraftingTable,
            1,
            text(tr(ctx, "Assign craft", "Назначить ремесло")),
            &[],
        ),
        if ctx.manage {
            Target::Pick { worker, from: None }
        } else {
            Target::Refresh
        },
    );
    screen
}

fn households_screen(state: &TickState, ctx: &Ctx, page: usize) -> Screen {
    let mut screen = Screen::new(tr(ctx, "Households", "Домохозяйства").to_owned());
    let Some(record) = state.settlements.get(ctx.id) else {
        return screen;
    };
    let pages = pages(record.households.len() + 1);
    let page = page.min(pages - 1);
    let ids = std::iter::once(None).chain(record.households.keys().filter_map(|id| {
        let (founder, sequence) = id.split_once('-')?;
        Some(Some((founder.parse().ok()?, sequence.parse().ok()?)))
    }));
    for (slot, id) in ids.skip(page * PAGE).take(PAGE).enumerate() {
        let name = id.map_or_else(
            || tr(ctx, "No household", "Без домохозяйства").to_owned(),
            |(a, b)| format!("{a}-{b}"),
        );
        screen.put(
            slot,
            icon(state, Icon::RedBed, 1, text(name), &[]),
            Target::Show(View::Household { id, page: 0 }),
        );
    }
    navigation(&mut screen, state, ctx, page, pages, |page| {
        View::Households { page }
    });
    screen.put(
        BACK,
        back(state, ctx),
        Target::Show(View::Census { filter: 0, page: 0 }),
    );
    screen
}

fn household_screen(state: &TickState, ctx: &Ctx, id: Option<(u64, u64)>, page: usize) -> Screen {
    let mut screen =
        Screen::new(tr(ctx, "Household · members", "Домохозяйство · жители").to_owned());
    let key = id.map(|(a, b)| format!("{a}-{b}"));
    let household = key
        .as_ref()
        .and_then(|key| state.settlements.get(ctx.id)?.households.get(key));
    let name = key
        .clone()
        .unwrap_or_else(|| tr(ctx, "No household", "Без домохозяйства").to_owned());
    if key.is_some() && household.is_none() {
        screen.put(
            4,
            icon(state, Icon::Barrier, 1, text(ctx.t(Key::StateChanged)), &[]),
            Target::Show(View::Households { page: 0 }),
        );
    } else {
        screen.put(
            4,
            icon(state, Icon::RedBed, 1, text(name), &[]),
            Target::Refresh,
        );
    }
    let unavailable = tr(ctx, "Not recorded", "Не записано");
    screen.put(
        10,
        icon(
            state,
            Icon::Bread,
            1,
            text(tr(ctx, "Needs policy", "Политика потребностей")),
            &[household
                .and_then(|household| household.needs.as_deref())
                .unwrap_or(unavailable)
                .to_owned()],
        ),
        Target::Refresh,
    );
    screen.put(
        14,
        icon(
            state,
            Icon::Book,
            1,
            text(tr(ctx, "Origin / policy", "Происхождение / политика")),
            &[
                household
                    .map_or(unavailable, |household| household.origin.as_str())
                    .to_owned(),
                household
                    .and_then(|household| household.policy.as_deref())
                    .unwrap_or(unavailable)
                    .to_owned(),
            ],
        ),
        Target::Refresh,
    );
    let home = household.and_then(|household| household.home);
    screen.put(
        12,
        icon(
            state,
            Icon::OakDoor,
            1,
            text(tr(
                ctx,
                "Social home (not a bed)",
                "Социальный дом (не кровать)",
            )),
            &[home
                .map(super::corner_text)
                .unwrap_or_else(|| unavailable.to_owned())],
        ),
        home.map_or(Target::Refresh, Target::MapAt),
    );
    let people = people(state, ctx.id);
    let members: Vec<_> = people
        .iter()
        .filter(|person| person.household == id)
        .collect();
    let pages = members.len().div_ceil(27).max(1);
    let page = page.min(pages - 1);
    let busy = super::busy_workers(state, ctx.id);
    for (slot, person) in (18..45).zip(members.into_iter().skip(page * 27).take(27)) {
        screen.put(
            slot,
            person_icon(state, ctx, person, &busy, ""),
            Target::Show(View::Resident {
                worker: person.uuid,
            }),
        );
    }
    navigation(&mut screen, state, ctx, page, pages, |page| {
        View::Household { id, page }
    });
    screen.put(
        BACK,
        back(state, ctx),
        Target::Show(View::Households { page: 0 }),
    );
    screen
}

fn buildings_screen(state: &TickState, ctx: &Ctx, name: &str, page: usize) -> Screen {
    filtered_buildings_screen(state, ctx, name, None, page)
}

fn filtered_buildings_screen(
    state: &TickState,
    ctx: &Ctx,
    name: &str,
    kind: Option<BuildingKind>,
    page: usize,
) -> Screen {
    let mut screen = Screen::new(format!("{} · {name}", tr(ctx, "Buildings", "Постройки")));
    let Some(record) = state.settlements.get(ctx.id) else {
        return screen;
    };
    let people = people(state, ctx.id);
    let count = record
        .buildings
        .values()
        .filter(|building| kind.is_none_or(|kind| kind == building.kind))
        .count();
    let pages = pages(count);
    let page = page.min(pages - 1);
    for (slot, building) in record
        .buildings
        .values()
        .filter(|building| kind.is_none_or(|kind| kind == building.kind))
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
            Target::Show(View::BuildingContext {
                building: building.id,
                origin: Origin::List { kind, page },
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
        icon(state, Icon::Bricks, 1, text(title), &lore),
        Target::Refresh,
    );
    screen.put(BACK, back(state, ctx), Target::Show(View::Dashboard));
    if ctx.manage {
        screen.put(
            ACTION,
            icon(
                state,
                Icon::WritableBook,
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
            Icon::Paper,
            1,
            text(tr(ctx, "Filter by type", "Фильтр по типу")),
            &[],
        ),
        Target::Dialog("building_filters"),
    );
    navigation(&mut screen, state, ctx, page, pages, |page| {
        View::FilteredBuildings { kind, page }
    });
    screen
}

fn building_screen(state: &TickState, ctx: &Ctx, id: BuildingId, origin: Origin) -> Option<Screen> {
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
        let label = ctx.f(Key::Certificate, &[("level", &next.to_string())]);
        screen.put(
            BUILDING_UPGRADE,
            icon(
                state,
                Icon::ExperienceBottle,
                1,
                text(label),
                &ctx.t(Key::CertificatePolicy)
                    .lines()
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            ),
            Target::Review {
                building: id,
                origin,
                certificate: true,
            },
        );
    }
    let construction = super::super::construction::owner_upgrade_level(state, ctx.id, id);
    screen.put(
        ACTION,
        icon(
            state,
            if construction.is_some() {
                Icon::Anvil
            } else {
                Icon::Barrier
            },
            1,
            text(construction.map_or_else(
                || ctx.t(Key::NoBlueprintUpgrade).to_owned(),
                |level| ctx.f(Key::ConstructionUpgrade, &[("level", &level.to_string())]),
            )),
            &ctx.t(Key::RequestPolicy)
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        ),
        if ctx.manage && construction.is_some() {
            Target::Review {
                building: id,
                origin,
                certificate: false,
            }
        } else {
            Target::None
        },
    );
    let centre =
        std::array::from_fn(|axis| building.bounds.min[axis].midpoint(building.bounds.max[axis]));
    screen.put(
        8,
        icon(state, Icon::FilledMap, 1, text(ctx.t(Key::Map)), &[]),
        Target::MapAt(centre),
    );
    if building.kind == BuildingKind::Storage {
        screen.put(
            BUILDING_STORAGE,
            icon(
                state,
                Icon::Chest,
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
                    if ctx.manage {
                        Target::Pick {
                            worker: person.uuid,
                            from: Some((id, origin)),
                        }
                    } else {
                        Target::Show(View::Resident {
                            worker: person.uuid,
                        })
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
                        Icon::LightGrayStainedGlassPane,
                        1,
                        text(tr(ctx, "Free work place", "Свободное место")),
                        &lore,
                    ),
                    if ctx.manage {
                        Target::Show(View::Hire {
                            building: id,
                            origin,
                            page: 0,
                        })
                    } else {
                        Target::Refresh
                    },
                );
            } else {
                screen.put(
                    slot,
                    icon(
                        state,
                        Icon::Barrier,
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
                Target::Show(View::Resident {
                    worker: person.uuid,
                }),
            );
        }
    } else if building.kind == BuildingKind::House {
        let claims = people
            .iter()
            .filter(|person| has_home_in(person, &building))
            .count();
        screen.put(
            BUILDING_PLACES,
            icon(
                state,
                Icon::PlayerHead,
                1,
                text(format!(
                    "{}: {claims}",
                    tr(ctx, "Home claims", "Назначенные дома")
                )),
                &[tr(
                    ctx,
                    "Bed validity and free capacity unavailable.",
                    "Исправность кроватей и свободная вместимость недоступны.",
                )
                .to_owned()],
            ),
            Target::Show(View::BuildingResidents {
                building: id,
                origin,
                page: 0,
            }),
        );
    }
    screen.put(
        BACK,
        back(state, ctx),
        Target::Show(View::Buildings { page: 0 }),
    );
    screen.put(
        LIST,
        icon(state, Icon::Paper, 1, text(ctx.t(Key::Buildings)), &[]),
        Target::Dialog("buildings"),
    );
    Some(screen)
}

fn assign_screen(
    state: &TickState,
    ctx: &Ctx,
    worker: Uuid,
    from: Option<(BuildingId, Origin)>,
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
    let back_to = from.map_or(View::Resident { worker }, |(building, origin)| {
        View::BuildingContext { building, origin }
    });
    screen.put(BACK, back(state, ctx), Target::Show(back_to));
    navigation(&mut screen, state, ctx, page, pages, |page| View::Assign {
        worker,
        from,
        page,
    });
    Some(screen)
}

fn hire_screen(
    state: &TickState,
    ctx: &Ctx,
    id: BuildingId,
    origin: Origin,
    page: usize,
) -> Option<Screen> {
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
        Target::Show(View::BuildingContext {
            building: id,
            origin,
        }),
    );
    navigation(&mut screen, state, ctx, page, pages, |page| View::Hire {
        building: id,
        origin,
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
        Icon::IronSword
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
    icon(state, Icon::Arrow, 1, text(ctx.t(Key::Back)), &[])
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
                Icon::Arrow,
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
                Icon::Arrow,
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

fn kind_item(kind: BuildingKind) -> Icon {
    match kind {
        BuildingKind::House => Icon::RedBed,
        BuildingKind::Storage => Icon::Chest,
        BuildingKind::Workshop(profession) => profession_item(profession),
        BuildingKind::Mine => Icon::IronPickaxe,
        BuildingKind::LumberCamp => Icon::IronAxe,
        BuildingKind::Barracks => Icon::Shield,
        BuildingKind::Wall => Icon::CobblestoneWall,
        BuildingKind::Tower => Icon::Spyglass,
        BuildingKind::Gate => Icon::OakFenceGate,
    }
}

/// The profession's workstation (vanilla job site block), else its tool.
fn profession_item(profession: Profession) -> Icon {
    match profession {
        Profession::None | Profession::Nitwit => Icon::VillagerSpawnEgg,
        Profession::Armorer => Icon::BlastFurnace,
        Profession::Butcher => Icon::Smoker,
        Profession::Cartographer => Icon::CartographyTable,
        Profession::Cleric => Icon::BrewingStand,
        Profession::Farmer => Icon::Composter,
        Profession::Fisherman => Icon::Barrel,
        Profession::Fletcher => Icon::FletchingTable,
        Profession::Leatherworker => Icon::Cauldron,
        Profession::Librarian => Icon::Lectern,
        Profession::Mason => Icon::Stonecutter,
        Profession::Shepherd => Icon::Loom,
        Profession::Toolsmith => Icon::SmithingTable,
        Profession::Weaponsmith => Icon::Grindstone,
        Profession::Miner => Icon::IronPickaxe,
        Profession::Lumberjack => Icon::IronAxe,
        Profession::Builder => Icon::CraftingTable,
        Profession::Soldier => Icon::IronSword,
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
fn icon(state: &TickState, item: Icon, count: i32, name: Tag, lore: &[String]) -> ItemStack {
    let id = state
        .registry
        .items
        .id_of(item.identifier())
        .or_else(|| state.registry.items.id_of(Icon::Paper.identifier()));
    let Some(id) = id else {
        return ItemStack::EMPTY;
    };
    decorate(state, ItemStack::new(id, count.clamp(1, 99)), name, lore)
}

fn item_icon(state: &TickState, item: &str, count: i32, name: Tag, lore: &[String]) -> ItemStack {
    let id = item_id(state, item).or_else(|| state.registry.items.id_of(Icon::Paper.identifier()));
    let Some(id) = id else {
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
