//! Stateful conservation property for the authoritative inventory path.
//!
//! One or two survival players stand in a flat world next to every menu block
//! the core opens. Random operation sequences enter through
//! [`run_tick`] with the same [`PlayerInput`]s a client packet becomes
//! (container clicks of every mode, quick-craft drags, open, close, drop,
//! hand swap, recipe placement, container buttons, creative packets, block
//! breaking, disconnect/rejoin, `/kill` and respawn) and through plain ticks
//! (item entity pickup, hopper transfers).
//!
//! After every tick the whole world is audited:
//! * the total of each item identity never changes: the pool holds no recipe
//!   input, fuel, smeltable or damageable item, so nothing may legitimately be
//!   created or destroyed; the gold ladder (nugget, ingot, block) converts
//!   only through crafting and is compared by nugget value; items nested in
//!   shulker box items and bundles count;
//! * no stack exceeds its maximum size and no slot holds a zero or negative
//!   count residue.
//!
//! `PROPTEST_CASES=3000 cargo test -p mc-core --lib
//! inventory::conservation_tests -- --nocapture` runs the long form and prints
//! the per-operation coverage table; the default is 64 cases.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

use mc_data::ItemStack;
use mc_nbt::Tag;
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::test_runner::TestRunner;

use crate::boundary::{
    BlockPos, ClickMode, ConnectionId, DigAction, EntityId, Hand, PlayerChange, PlayerInput,
};
use crate::horde_bench::flat_world;
use crate::registry::CoreRegistry;
use crate::tick::{BlockEntityState, TickState, run_tick};

/// Block row the players stand on: the flat world's grass top is y=63.
const FLOOR_Y: i32 = 64;
const PLAYER_POS: [[f64; 3]; 2] = [[8.5, 64.0, 8.5], [9.5, 64.0, 8.5]];

/// The full vanilla catalog (real max stack sizes, recipes, tags), loaded once.
fn registry() -> Arc<CoreRegistry> {
    static REGISTRY: LazyLock<Arc<CoreRegistry>> =
        LazyLock::new(|| Arc::new(crate::tick_tests::full_registry()));
    Arc::clone(&REGISTRY)
}

/// One placed block of the fixture room.
struct Fixture {
    block: &'static str,
    props: &'static [(&'static str, &'static str)],
    at: (i32, i32, i32),
    /// Storage slots of its block entity (0: the menu keeps its own custody).
    slots: usize,
}

const fn fixture(
    block: &'static str,
    props: &'static [(&'static str, &'static str)],
    at: (i32, i32, i32),
    slots: usize,
) -> Fixture {
    Fixture {
        block,
        props,
        at,
        slots,
    }
}

const NORTH: &[(&str, &str)] = &[("facing", "north")];
const CHEST_LEFT: &[(&str, &str)] = &[("facing", "north"), ("type", "left")];
const CHEST_RIGHT: &[(&str, &str)] = &[("facing", "north"), ("type", "right")];
const HOPPER_EAST: &[(&str, &str)] = &[("facing", "east")];

/// Every menu block, within reach of both players. Index = [`Target`].
const FIXTURES: &[Fixture] = &[
    fixture("minecraft:chest", NORTH, (6, 64, 5), 27),
    fixture("minecraft:chest", CHEST_LEFT, (7, 64, 5), 27),
    fixture("minecraft:chest", CHEST_RIGHT, (8, 64, 5), 27),
    fixture("minecraft:barrel", &[], (9, 64, 5), 27),
    fixture("minecraft:shulker_box", &[], (10, 64, 5), 27),
    fixture("minecraft:furnace", NORTH, (11, 64, 5), 3),
    fixture("minecraft:dispenser", NORTH, (6, 64, 6), 9),
    fixture("minecraft:dropper", NORTH, (7, 64, 6), 9),
    fixture("minecraft:crafting_table", &[], (8, 64, 6), 0),
    fixture("minecraft:anvil", NORTH, (9, 64, 6), 0),
    fixture("minecraft:grindstone", &[], (10, 64, 6), 0),
    fixture("minecraft:enchanting_table", &[], (11, 64, 6), 0),
    fixture("minecraft:brewing_stand", &[], (6, 64, 10), 5),
    fixture("minecraft:loom", NORTH, (7, 64, 10), 0),
    fixture("minecraft:stonecutter", NORTH, (8, 64, 10), 0),
    fixture("minecraft:smithing_table", &[], (9, 64, 10), 0),
    fixture("minecraft:cartography_table", &[], (10, 64, 10), 0),
    fixture("minecraft:crafter", &[], (11, 64, 10), 9),
    fixture("minecraft:beacon", &[], (6, 64, 11), 0),
    fixture("minecraft:ender_chest", NORTH, (7, 64, 11), 0),
    fixture("minecraft:blast_furnace", NORTH, (8, 64, 11), 3),
    fixture("minecraft:smoker", NORTH, (9, 64, 11), 3),
    fixture("minecraft:trapped_chest", NORTH, (10, 64, 11), 27),
    // Hopper chain: chest above -> hopper -> chest east of it.
    fixture("minecraft:hopper", HOPPER_EAST, (4, 64, 9), 5),
    fixture("minecraft:chest", NORTH, (4, 65, 9), 27),
    fixture("minecraft:chest", NORTH, (5, 64, 9), 27),
];

/// Items the property moves. None is a recipe input, fuel, smeltable or
/// damageable, except the gold ladder whose crafts conserve nugget value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pool {
    Bedrock,
    EnderPearl,
    Saddle,
    ShulkerBox,
    Totem,
    CarvedPumpkin,
    GoldNugget,
    GoldIngot,
    GoldBlock,
    Bundle,
}

const POOL: [Pool; 10] = [
    Pool::Bedrock,
    Pool::EnderPearl,
    Pool::Saddle,
    Pool::ShulkerBox,
    Pool::Totem,
    Pool::CarvedPumpkin,
    Pool::GoldNugget,
    Pool::GoldIngot,
    Pool::GoldBlock,
    Pool::Bundle,
];

impl Pool {
    fn name(self) -> &'static str {
        match self {
            Self::Bedrock => "minecraft:bedrock",
            Self::EnderPearl => "minecraft:ender_pearl",
            Self::Saddle => "minecraft:saddle",
            Self::ShulkerBox => "minecraft:shulker_box",
            Self::Totem => "minecraft:totem_of_undying",
            Self::CarvedPumpkin => "minecraft:carved_pumpkin",
            Self::GoldNugget => "minecraft:gold_nugget",
            Self::GoldIngot => "minecraft:gold_ingot",
            Self::GoldBlock => "minecraft:gold_block",
            Self::Bundle => "minecraft:bundle",
        }
    }
}

/// Counting key and weight of a stack: the gold ladder shares one key
/// (nugget value); every other pool item counts by its identity, plain or
/// named, so a name lost or gained by a move is a violation. Items outside
/// the pool (a broken fixture block's own item) are not custody under test.
fn ledger_entry(gold: &[(u32, i64); 3], pool: &[u32], stack: &ItemStack) -> Option<(i64, i64)> {
    if let Some((_, weight)) = gold.iter().find(|(id, _)| *id == stack.item_id) {
        return Some((-1, *weight));
    }
    pool.contains(&stack.item_id).then_some((
        i64::from(stack.item_id) * 2 + i64::from(stack.custom_name.is_some()),
        1,
    ))
}

#[derive(Debug, Clone, Copy)]
struct StackSpec {
    item: usize,
    count: u8,
    named: bool,
}

fn stack_spec() -> impl Strategy<Value = StackSpec> {
    (0..POOL.len(), any::<u8>(), any::<bool>()).prop_map(|(item, count, named)| StackSpec {
        item,
        count,
        named,
    })
}

/// Where a click lands, resolved against the menu the player has open.
#[derive(Debug, Clone, Copy)]
enum SlotPick {
    /// `index % slot_count` of the open menu.
    Menu(u16),
    /// Outside the window (-999): throws, ends or starts a drag.
    Outside,
    /// Whatever a client could put on the wire.
    Raw(i16),
}

/// How the click's window id and state id relate to the server's menu.
#[derive(Debug, Clone, Copy)]
enum Sync {
    Fresh,
    StaleState(i32),
    WrongWindow,
}

#[derive(Debug, Clone)]
enum Input {
    Click {
        who: usize,
        slot: SlotPick,
        button: i8,
        mode: ClickMode,
        sync: Sync,
    },
    Open {
        who: usize,
        target: usize,
    },
    Close {
        who: usize,
    },
    DropSelected {
        who: usize,
        whole: bool,
    },
    SwapHands {
        who: usize,
    },
    SelectSlot {
        who: usize,
        slot: u8,
    },
    PlaceRecipe {
        who: usize,
        recipe: usize,
        make_all: bool,
    },
    ContainerButton {
        who: usize,
        button: i32,
    },
    CrafterSlot {
        who: usize,
        slot: i32,
        enabled: bool,
    },
    CreativeSlot {
        who: usize,
        slot: i16,
        spec: StackSpec,
    },
    PickBlock {
        who: usize,
        target: usize,
    },
    Rename {
        who: usize,
        name: String,
    },
    SetBeacon {
        who: usize,
        primary: Option<u32>,
    },
    BundleSelect {
        who: usize,
        slot: i32,
        selected: i32,
    },
}

impl Input {
    /// Coverage key.
    fn name(&self) -> String {
        match self {
            Self::Click { mode, .. } => format!("click:{mode:?}"),
            Self::Open { .. } => "open".to_owned(),
            Self::Close { .. } => "close".to_owned(),
            Self::DropSelected { .. } => "drop-selected".to_owned(),
            Self::SwapHands { .. } => "swap-hands".to_owned(),
            Self::SelectSlot { .. } => "select-slot".to_owned(),
            Self::PlaceRecipe { .. } => "place-recipe".to_owned(),
            Self::ContainerButton { .. } => "container-button".to_owned(),
            Self::CrafterSlot { .. } => "crafter-slot".to_owned(),
            Self::CreativeSlot { .. } => "creative-slot".to_owned(),
            Self::PickBlock { .. } => "pick-block".to_owned(),
            Self::Rename { .. } => "rename".to_owned(),
            Self::SetBeacon { .. } => "set-beacon".to_owned(),
            Self::BundleSelect { .. } => "bundle-select".to_owned(),
        }
    }
}

/// One simulated unit: a tick carrying several inputs (two players clicking
/// in the same tick), or a multi-tick gesture.
#[derive(Debug, Clone)]
enum Step {
    Batch(Vec<Input>),
    /// Quick-craft drag: start, one add per slot, end, each in its own tick.
    Drag {
        who: usize,
        right: bool,
        slots: Vec<u16>,
    },
    /// Idle ticks: item pickup delay, hopper cooldown.
    Wait(u8),
    /// Survival-mine a fixture block; its contents drop as item entities.
    Break {
        who: usize,
        target: usize,
    },
    /// Disconnect and rejoin: the cursor and open grid return to the
    /// inventory, the player image is saved and restored.
    Reconnect {
        who: usize,
    },
    /// `/kill` the player, then respawn: the inventory drops as item entities
    /// unless `keep_inventory` is on.
    Die {
        who: usize,
    },
    /// `/gamerule keep_inventory`.
    KeepInventory(bool),
    /// Place a recipe into the crafting grid, then act on its result slot:
    /// the only way random clicks reliably reach a non-empty result.
    Craft {
        who: usize,
        table: bool,
        recipe: usize,
        make_all: bool,
        mode: ClickMode,
        button: i8,
    },
}

fn click_mode() -> impl Strategy<Value = ClickMode> {
    prop_oneof![
        6 => Just(ClickMode::Pickup),
        4 => Just(ClickMode::QuickMove),
        3 => Just(ClickMode::Swap),
        1 => Just(ClickMode::Clone),
        2 => Just(ClickMode::Throw),
        2 => Just(ClickMode::QuickCraft),
        3 => Just(ClickMode::PickupAll),
    ]
}

fn button() -> impl Strategy<Value = i8> {
    prop_oneof![
        8 => 0..=1_i8,
        4 => 0..=8_i8,
        1 => Just(40_i8),
        1 => -3..=12_i8,
        1 => any::<i8>(),
    ]
}

fn slot_pick() -> impl Strategy<Value = SlotPick> {
    prop_oneof![
        12 => any::<u16>().prop_map(SlotPick::Menu),
        2 => Just(SlotPick::Outside),
        1 => any::<i16>().prop_map(SlotPick::Raw),
        1 => (-3..=70_i16).prop_map(SlotPick::Raw),
    ]
}

fn sync() -> impl Strategy<Value = Sync> {
    prop_oneof![
        24 => Just(Sync::Fresh),
        2 => (-2..=2_i32).prop_map(Sync::StaleState),
        1 => Just(Sync::WrongWindow),
    ]
}

fn input(players: usize) -> impl Strategy<Value = Input> {
    let who = 0..players;
    prop_oneof![
        40 => (who.clone(), slot_pick(), button(), click_mode(), sync()).prop_map(
            |(who, slot, button, mode, sync)| Input::Click { who, slot, button, mode, sync }
        ),
        6 => (who.clone(), 0..FIXTURES.len()).prop_map(|(who, target)| Input::Open { who, target }),
        2 => who.clone().prop_map(|who| Input::Close { who }),
        3 => (who.clone(), any::<bool>()).prop_map(|(who, whole)| Input::DropSelected { who, whole }),
        1 => who.clone().prop_map(|who| Input::SwapHands { who }),
        1 => (who.clone(), 0..=8_u8).prop_map(|(who, slot)| Input::SelectSlot { who, slot }),
        3 => (who.clone(), 0..GOLD_RECIPES.len(), any::<bool>()).prop_map(
            |(who, recipe, make_all)| Input::PlaceRecipe { who, recipe, make_all }
        ),
        1 => (who.clone(), -2..=10_i32).prop_map(|(who, button)| Input::ContainerButton { who, button }),
        1 => (who.clone(), -1..=10_i32, any::<bool>()).prop_map(
            |(who, slot, enabled)| Input::CrafterSlot { who, slot, enabled }
        ),
        1 => (who.clone(), -1..=45_i16, stack_spec()).prop_map(
            |(who, slot, spec)| Input::CreativeSlot { who, slot, spec }
        ),
        1 => (who.clone(), 0..FIXTURES.len()).prop_map(|(who, target)| Input::PickBlock { who, target }),
        1 => (who.clone(), "[a-z]{0,4}").prop_map(|(who, name)| Input::Rename { who, name }),
        1 => (who.clone(), proptest::option::of(0..4_u32)).prop_map(
            |(who, primary)| Input::SetBeacon { who, primary }
        ),
        1 => (who, -1..=45_i32, -1..=4_i32).prop_map(
            |(who, slot, selected)| Input::BundleSelect { who, slot, selected }
        ),
    ]
}

/// The gold recipes the recipe book places (ids under `minecraft:`).
const GOLD_RECIPES: [&str; 4] = [
    "gold_block",
    "gold_ingot_from_gold_block",
    "gold_ingot_from_nuggets",
    "gold_nugget",
];

fn step(players: usize) -> impl Strategy<Value = Step> {
    prop_oneof![
        30 => vec(input(players), 1..=3).prop_map(Step::Batch),
        3 => (0..players, any::<bool>(), vec(any::<u16>(), 0..=6))
            .prop_map(|(who, right, slots)| Step::Drag { who, right, slots }),
        4 => (1..=12_u8).prop_map(Step::Wait),
        1 => (0..players, 0..FIXTURES.len()).prop_map(|(who, target)| Step::Break { who, target }),
        1 => (0..players).prop_map(|who| Step::Reconnect { who }),
        1 => (0..players).prop_map(|who| Step::Die { who }),
        1 => any::<bool>().prop_map(Step::KeepInventory),
        3 => (0..players, any::<bool>(), 0..GOLD_RECIPES.len(), any::<bool>(), click_mode(), button())
            .prop_map(|(who, table, recipe, make_all, mode, button)| Step::Craft {
                who, table, recipe, make_all, mode, button
            }),
    ]
}

#[derive(Debug, Clone)]
struct Scenario {
    players: usize,
    player_fill: Vec<(usize, u8, StackSpec)>,
    container_fill: Vec<(usize, u8, StackSpec)>,
    steps: Vec<Step>,
}

fn scenario() -> impl Strategy<Value = Scenario> {
    (1..=2_usize).prop_flat_map(|players| {
        (
            vec((0..players, 0..37_u8, stack_spec()), 0..=40),
            vec((0..FIXTURES.len(), any::<u8>(), stack_spec()), 0..=40),
            vec(step(players), 1..=36),
        )
            .prop_map(move |(player_fill, container_fill, steps)| Scenario {
                players,
                player_fill,
                container_fill,
                steps,
            })
    })
}

/// Total of every item the world holds, by [`ledger_entry`] key.
type Ledger = BTreeMap<i64, i64>;

/// Custody stacks, open menus, item entity count and raw gold counts.
type Snapshot = (Vec<ItemStack>, Vec<String>, usize, [i64; 3]);

/// Chest above, hopper, chest beside it: the fixtures a wait moves items through.
const HOPPER_CHAIN: [(i32, i32, i32); 3] = [(4, 65, 9), (4, 64, 9), (5, 64, 9)];

struct Rig {
    state: TickState,
    players: Vec<(ConnectionId, EntityId)>,
    gold: [(u32, i64); 3],
    recipes: Vec<u32>,
    pool_ids: Vec<u32>,
    baseline: Ledger,
    /// How often each operation ran and changed the world.
    coverage: BTreeMap<String, u64>,
}

fn block_state(state: &TickState, name: &str, props: &[(&str, &str)]) -> u32 {
    let id = mc_data::Identifier::parse(name).expect("block id");
    let default = state.registry.blocks.block(&id).expect("block").default;
    let mut full = state
        .registry
        .blocks
        .by_id(default)
        .expect("default state")
        .properties
        .clone();
    for (key, value) in props {
        if let Some(entry) = full.iter_mut().find(|(name, _)| name == key) {
            entry.1 = (*value).to_owned();
        }
    }
    state
        .registry
        .blocks
        .by_name_and_props(&id, &full)
        .unwrap_or_else(|| panic!("no state {name} {props:?}"))
        .0
}

fn pos(at: (i32, i32, i32)) -> BlockPos {
    BlockPos {
        x: at.0,
        y: at.1,
        z: at.2,
    }
}

/// Put player `who` at its post with the recipes the property places unlocked.
fn stand(state: &mut TickState, who: usize, entity: EntityId) {
    let slot = state.entities.slot_of(entity).expect("player slot");
    state.entities.set_pos(slot, PLAYER_POS[who]);
    state.entities.set_ground(slot, true);
    let player = state.players.get_mut(&entity).expect("player");
    assert!(!player.creative(), "conservation runs in survival");
    player.unlocked_recipes = Arc::new(
        GOLD_RECIPES
            .iter()
            .map(|name| format!("minecraft:{name}"))
            .collect(),
    );
}

impl Rig {
    fn new(scenario: &Scenario) -> Self {
        let registry = registry();
        let world = flat_world(&registry);
        // Simulation distance 1: the room sits in one chunk.
        let mut state = TickState::new(Arc::clone(&registry), world, 1, 512);
        let connections: Vec<ConnectionId> = (0..scenario.players)
            .map(|who| ConnectionId(who as u64 + 1))
            .collect();
        let joins: Vec<PlayerInput> = connections
            .iter()
            .enumerate()
            .map(|(who, &connection)| PlayerInput::Join {
                connection,
                uuid: uuid::Uuid::from_u128(who as u128 + 1),
                name: format!("conserve{who}"),
                permission_level: 0,
            })
            .collect();
        run_tick(&mut state, &joins);
        let want = state.desired_chunks();
        state.world.ensure_blocking(&want);
        let confirms: Vec<PlayerInput> = connections
            .iter()
            .map(|&connection| PlayerInput::ConfirmTeleport {
                connection,
                teleport_id: 1,
            })
            .collect();
        run_tick(&mut state, &confirms);
        let players: Vec<(ConnectionId, EntityId)> = connections
            .iter()
            .map(|&connection| {
                (
                    connection,
                    state.connections.entity_of(connection).expect("joined"),
                )
            })
            .collect();
        let air = state.registry.states.air;
        for x in 3..=13 {
            for z in 3..=12 {
                for y in FLOOR_Y..FLOOR_Y + 4 {
                    state.world.set_block(x, y, z, air);
                }
            }
        }
        for fixture in FIXTURES {
            let id = block_state(&state, fixture.block, fixture.props);
            let (x, y, z) = fixture.at;
            state.world.set_block(x, y, z, id);
        }
        state.world.fold_overlay();
        for fixture in FIXTURES.iter().filter(|fixture| fixture.slots > 0) {
            state.block_entities.insert(
                fixture.at,
                BlockEntityState {
                    slots: vec![ItemStack::EMPTY; fixture.slots],
                    furnace: None,
                },
            );
        }
        let item_id = |name: &str| {
            state
                .registry
                .items
                .id_of(&mc_data::Identifier::parse(name).expect("item id"))
                .unwrap_or_else(|| panic!("no item {name}"))
        };
        let gold = [
            (item_id("minecraft:gold_nugget"), 1),
            (item_id("minecraft:gold_ingot"), 9),
            (item_id("minecraft:gold_block"), 81),
        ];
        let pool_ids: Vec<u32> = POOL.iter().map(|pool| item_id(pool.name())).collect();
        let recipes: Vec<u32> = GOLD_RECIPES
            .iter()
            .map(|name| {
                let id = format!("minecraft:{name}");
                state
                    .registry
                    .recipes
                    .iter()
                    .position(|recipe| recipe.id.as_str() == id)
                    .unwrap_or_else(|| panic!("no recipe {id}")) as u32
            })
            .collect();
        for (who, &(_, entity)) in players.iter().enumerate() {
            stand(&mut state, who, entity);
        }
        let mut rig = Self {
            state,
            players,
            gold,
            recipes,
            pool_ids,
            baseline: Ledger::new(),
            coverage: BTreeMap::new(),
        };
        rig.fill(scenario);
        rig.baseline = rig.ledger().expect("fixture is valid");
        rig
    }

    fn fill(&mut self, scenario: &Scenario) {
        // Armor takes only what fits it, so it starts empty; slot 36 of the
        // spec stands for the off hand.
        let gives: Vec<PlayerInput> = scenario
            .player_fill
            .iter()
            .map(|&(who, slot, spec)| PlayerInput::TestGive {
                connection: self.players[who].0,
                slot: if slot == 36 { 40 } else { usize::from(slot) },
                stack: self.stack(spec),
            })
            .collect();
        run_tick(&mut self.state, &gives);
        for &(target, slot, spec) in &scenario.container_fill {
            let stack = self.stack(spec);
            let fixture = &FIXTURES[target];
            if fixture.slots == 0 {
                continue;
            }
            let record = self
                .state
                .block_entities
                .get_mut(&fixture.at)
                .expect("fixture");
            let index = usize::from(slot) % fixture.slots;
            // Furnace output is never fed by hand and a shulker never holds
            // a shulker; seed only what a player could have stored.
            if (fixture.block.contains("furnace") || fixture.block == "minecraft:smoker")
                && index == 2
            {
                continue;
            }
            if fixture.block == "minecraft:shulker_box"
                && self.pool_ids[Pool::ShulkerBox as usize] == stack.item_id
            {
                continue;
            }
            record.slots[index] = stack;
        }
    }

    fn stack(&self, spec: StackSpec) -> ItemStack {
        let item = self.pool_ids[spec.item];
        let probe = ItemStack::new(item, 1);
        let max = self.state.registry.item_rules().max_stack(&probe);
        let mut stack = ItemStack::new(item, 1 + i32::from(spec.count) % max);
        // The gold ladder converts by crafting, which would drop the name.
        if spec.named && !self.gold.iter().any(|(id, _)| *id == item) {
            stack.custom_name = Some("named".into());
        }
        stack
    }

    fn rules(&self) -> crate::inventory::ItemRules<'_> {
        self.state.registry.item_rules()
    }

    /// Items stored inside `stack`: a shulker box item's container component
    /// and a bundle's contents.
    fn nested(&self, stack: &ItemStack) -> Result<Vec<ItemStack>, String> {
        let items = &self.state.registry.items;
        let rules = self.rules();
        let mut inside = Vec::new();
        if crate::shulker::is_item(items, stack) {
            let fields = mc_data::item_nbt::stack_to_fields(items, stack)
                .map_err(|error| format!("shulker item unreadable: {error}"))?;
            if let Some(Tag::Compound(components)) = mc_data::item_nbt::field(&fields, "components")
                && let Some(Tag::List(list)) =
                    mc_data::item_nbt::field(components, "minecraft:container")
            {
                for entry in &list.elements {
                    let Tag::Compound(entry) = entry else {
                        return Err("container entry is not a compound".into());
                    };
                    let Some(Tag::Compound(item)) = mc_data::item_nbt::field(entry, "item") else {
                        return Err("container entry has no item".into());
                    };
                    let nested = mc_data::item_nbt::stack_from_fields(items, item)
                        .map_err(|error| format!("container item unreadable: {error}"))?;
                    inside.extend(nested);
                }
            }
        }
        if super::bundle::is_bundle(&rules, stack) {
            inside.extend(
                super::bundle::read(&rules, stack).ok_or_else(|| "bundle unreadable".to_owned())?,
            );
        }
        Ok(inside)
    }

    /// Validate `stack` and everything nested in it, adding pool items to
    /// `ledger`.
    fn audit_stack(
        &self,
        ledger: &mut Ledger,
        place: &str,
        stack: &ItemStack,
    ) -> Result<(), String> {
        if stack.is_empty() {
            if *stack != ItemStack::EMPTY {
                return Err(format!("{place}: empty slot keeps residue {stack:?}"));
            }
            return Ok(());
        }
        let max = self.rules().max_stack(stack);
        if stack.count > max {
            return Err(format!(
                "{place}: {} x{} exceeds max stack {max}",
                self.state
                    .registry
                    .items
                    .name_of(stack.item_id)
                    .map_or("?", |name| name.as_str()),
                stack.count
            ));
        }
        if let Some((key, weight)) = ledger_entry(&self.gold, &self.pool_ids, stack) {
            *ledger.entry(key).or_default() += weight * i64::from(stack.count);
        }
        for inner in self
            .nested(stack)
            .map_err(|error| format!("{place}: {error}"))?
        {
            self.audit_stack(ledger, &format!("{place}/inside"), &inner)?;
        }
        Ok(())
    }

    /// Audit every custody location; returns the world's item ledger.
    fn ledger(&self) -> Result<Ledger, String> {
        let rules = self.rules();
        let mut ledger = Ledger::new();
        let mut add = |place: String, stack: &ItemStack| -> Result<(), String> {
            self.audit_stack(&mut ledger, &place, stack)
        };
        for (n, &(_, entity)) in self.players.iter().enumerate() {
            let player = &self.state.players[&entity];
            for (slot, stack) in player.inventory.slots.iter().enumerate() {
                add(format!("player{n} inventory[{slot}]"), stack)?;
            }
            for (label, menu) in std::iter::once(("window0", Some(&player.inventory_menu)))
                .chain([("open", player.open.as_ref())])
            {
                let Some(menu) = menu else { continue };
                add(format!("player{n} {label} cursor"), &menu.carried)?;
                for (slot, stack) in menu.grid.iter().enumerate() {
                    add(format!("player{n} {label} grid[{slot}]"), stack)?;
                }
                // Derived output: its validity is audited, it is never counted.
                if !menu.result.is_empty() && menu.result.count > rules.max_stack(&menu.result) {
                    return Err(format!(
                        "player{n} {label} result over max: {:?}",
                        menu.result
                    ));
                }
            }
            let ender = crate::ender_chest::contents(
                &player.opaque_player_fields,
                &self.state.registry.items,
            )
            .map_err(|error| format!("player{n} ender chest unreadable: {error}"))?;
            for (slot, stack) in ender.iter().enumerate() {
                add(format!("player{n} ender[{slot}]"), stack)?;
            }
        }
        for (cell, record) in &self.state.block_entities {
            for (slot, stack) in record.slots.iter().enumerate() {
                add(format!("block {cell:?}[{slot}]"), stack)?;
            }
        }
        for slot in self.state.entities.item_slots() {
            let Some(row) = self.state.entities.row(slot).item.as_ref() else {
                continue;
            };
            let full = row.full_stack();
            if row.count != full.count || row.item_id != full.item_id {
                return Err(format!("item entity row disagrees with its stack: {row:?}"));
            }
            add(format!("item entity {slot:?}"), &full)?;
        }
        Ok(ledger)
    }

    fn check(&self, what: &str) -> Result<(), String> {
        let ledger = self.ledger().map_err(|error| format!("{what}: {error}"))?;
        if ledger != self.baseline {
            return Err(format!(
                "{what}: item totals changed\n  before: {}\n  after:  {}",
                self.describe(&self.baseline),
                self.describe(&ledger)
            ));
        }
        Ok(())
    }

    fn describe(&self, ledger: &Ledger) -> String {
        ledger
            .iter()
            .map(|(key, total)| {
                if *key == -1 {
                    format!("gold-nugget-value={total}")
                } else {
                    let name = self
                        .state
                        .registry
                        .items
                        .name_of((*key / 2) as u32)
                        .map_or("?", |name| name.as_str());
                    let named = if *key % 2 == 1 { " (named)" } else { "" };
                    format!("{name}{named}={total}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn menu(&self, who: usize) -> &crate::inventory::Menu {
        let player = &self.state.players[&self.players[who].1];
        player.open.as_ref().unwrap_or(&player.inventory_menu)
    }

    fn click_input(
        &self,
        who: usize,
        slot: i16,
        button: i8,
        mode: ClickMode,
        sync: Sync,
    ) -> PlayerInput {
        let menu = self.menu(who);
        let (window, state_id) = match sync {
            Sync::Fresh => (menu.window, menu.state_id),
            Sync::StaleState(delta) => (menu.window, menu.state_id.wrapping_add(delta)),
            Sync::WrongWindow => (menu.window.wrapping_add(1), menu.state_id),
        };
        PlayerInput::ContainerClick {
            connection: self.players[who].0,
            window,
            state_id,
            slot,
            button,
            mode,
        }
    }

    fn resolve_slot(&self, who: usize, pick: SlotPick) -> i16 {
        match pick {
            SlotPick::Menu(index) => (usize::from(index) % self.menu(who).kind.slot_count()) as i16,
            SlotPick::Outside => -999,
            SlotPick::Raw(slot) => slot,
        }
    }

    /// An operator's chat command from player `who`.
    fn command(&self, who: usize, text: &str) -> PlayerInput {
        PlayerInput::Command {
            connection: self.players[who].0,
            command: text.to_owned(),
            permission_level: 4,
        }
    }

    /// A use-on-block packet at a fixture that was broken: vanilla places the
    /// held block item there, which consumes it. That legitimate destruction
    /// is block placement's own contract, not an inventory operation, so the
    /// property does not send it.
    fn places_held_block(&self, input: &Input) -> bool {
        let Input::Open { target, .. } = input else {
            return false;
        };
        let (x, y, z) = FIXTURES[*target].at;
        self.state.world.block_at(x, y, z) == Some(self.state.registry.states.air)
    }
    fn to_player_input(&self, input: &Input) -> PlayerInput {
        match *input {
            Input::Click {
                who,
                slot,
                button,
                mode,
                sync,
            } => self.click_input(who, self.resolve_slot(who, slot), button, mode, sync),
            Input::Open { who, target } => PlayerInput::UseItemOn {
                connection: self.players[who].0,
                hand: Hand::Main,
                position: pos(FIXTURES[target].at),
                face: 1,
                cursor: [0.5; 3],
                sequence: 1,
            },
            Input::Close { who } => PlayerInput::ContainerClose {
                connection: self.players[who].0,
                window: self.menu(who).window,
            },
            Input::DropSelected { who, whole } => PlayerInput::DropSelected {
                connection: self.players[who].0,
                whole_stack: whole,
            },
            Input::SwapHands { who } => PlayerInput::SwapHands {
                connection: self.players[who].0,
            },
            Input::SelectSlot { who, slot } => PlayerInput::SelectSlot {
                connection: self.players[who].0,
                slot,
            },
            Input::PlaceRecipe {
                who,
                recipe,
                make_all,
            } => PlayerInput::PlaceRecipe {
                connection: self.players[who].0,
                window: self.menu(who).window,
                recipe: self.recipes[recipe],
                make_all,
            },
            Input::ContainerButton { who, button } => PlayerInput::ContainerButton {
                connection: self.players[who].0,
                window: self.menu(who).window,
                button,
            },
            Input::CrafterSlot { who, slot, enabled } => PlayerInput::ContainerSlotState {
                connection: self.players[who].0,
                window: self.menu(who).window,
                slot,
                enabled,
            },
            // A survival client must never gain items from creative packets.
            Input::CreativeSlot { who, slot, spec } => PlayerInput::CreativeSlot {
                connection: self.players[who].0,
                slot,
                stack: self.stack(spec),
            },
            Input::PickBlock { who, target } => PlayerInput::PickItemFromBlock {
                connection: self.players[who].0,
                position: pos(FIXTURES[target].at),
            },
            Input::Rename { who, ref name } => PlayerInput::RenameItem {
                connection: self.players[who].0,
                name: name.clone(),
            },
            Input::SetBeacon { who, primary } => PlayerInput::SetBeacon {
                connection: self.players[who].0,
                primary,
                secondary: primary,
            },
            Input::BundleSelect {
                who,
                slot,
                selected,
            } => PlayerInput::SelectBundleItem {
                connection: self.players[who].0,
                slot,
                selected_index: selected,
            },
        }
    }

    fn tick(&mut self, inputs: &[PlayerInput], what: &str) -> Result<(), String> {
        run_tick(&mut self.state, inputs);
        self.check(what)
    }

    /// Every stack in custody (players, menus, block storage, item entities)
    /// plus which menus are open, how many item entities exist and the raw
    /// gold counts (nugget, ingot, block): what a step can change.
    fn snapshot(&self) -> Snapshot {
        let mut stacks = Vec::new();
        let mut menus = Vec::new();
        for &(_, entity) in &self.players {
            let player = &self.state.players[&entity];
            stacks.extend(player.inventory.slots.iter().cloned());
            for menu in std::iter::once(&player.inventory_menu).chain(player.open.as_ref()) {
                stacks.push(menu.carried.clone());
                stacks.extend(menu.grid.iter().cloned());
            }
            menus.push(format!("{:?}", player.open.as_ref().map(|menu| menu.kind)));
        }
        let mut cells: Vec<_> = self.state.block_entities.iter().collect();
        cells.sort_by_key(|(cell, _)| **cell);
        for (_, record) in cells {
            stacks.extend(record.slots.iter().cloned());
        }
        let mut gold = [0_i64; 3];
        let entity_stacks: Vec<ItemStack> = self
            .state
            .entities
            .item_slots()
            .filter_map(|slot| self.state.entities.row(slot).item.as_ref())
            .map(|row| row.full_stack())
            .collect();
        for stack in stacks.iter().chain(&entity_stacks) {
            if let Some(index) = self.gold.iter().position(|(id, _)| *id == stack.item_id) {
                gold[index] += i64::from(stack.count.max(0));
            }
        }
        (stacks, menus, entity_stacks.len(), gold)
    }

    fn note(&mut self, key: String) {
        *self.coverage.entry(key).or_default() += 1;
    }

    /// Count `name` as exercised, and as effective when it changed the world.
    fn note_step(&mut self, name: &str, before: &Snapshot) {
        self.note(name.to_owned());
        let after = self.snapshot();
        if after != *before {
            self.note(format!("{name}:effective"));
            if after.2 < before.2 {
                self.note(format!("{name}:picked-up-item-entity"));
            }
            if after.2 > before.2 {
                self.note(format!("{name}:spawned-item-entity"));
            }
            let value = |gold: &[i64; 3]| {
                gold.iter()
                    .zip(&self.gold)
                    .map(|(count, (_, weight))| count * weight)
                    .sum::<i64>()
            };
            if after.3 != before.3 && value(&after.3) == value(&before.3) {
                self.note(format!("{name}:gold-converted"));
            }
        }
    }

    fn apply(&mut self, index: usize, step: &Step) -> Result<(), String> {
        let label = |detail: &str| format!("step {index} {detail}");
        let before = self.snapshot();
        match step {
            Step::Batch(inputs) => {
                let packets: Vec<PlayerInput> = inputs
                    .iter()
                    .filter(|input| !self.places_held_block(input))
                    .map(|input| self.to_player_input(input))
                    .collect();
                let menu_kind = match inputs.as_slice() {
                    [Input::Click { who, .. }] => Some(format!("{:?}", self.menu(*who).kind)),
                    _ => None,
                };
                let result = self.tick(&packets, &label(&format!("{packets:?}")));
                if let [only] = inputs.as_slice() {
                    self.note_step(&only.name(), &before);
                    if let Some(kind) = menu_kind
                        && self.snapshot() != before
                    {
                        self.note(format!("click-in-menu:{kind}:effective"));
                    }
                } else {
                    self.note_step("batch-of-several", &before);
                }
                result
            }
            Step::Wait(ticks) => {
                let chain = |rig: &Self| {
                    HOPPER_CHAIN
                        .map(|cell| rig.state.block_entities.get(&cell).map(|r| r.slots.clone()))
                };
                let chain_before = chain(self);
                for n in 0..*ticks {
                    self.tick(&[], &label(&format!("wait tick {n}")))?;
                }
                self.note_step("wait", &before);
                if chain(self) != chain_before {
                    self.note("wait:hopper-chain-moved".to_owned());
                }
                Ok(())
            }
            Step::Craft {
                who,
                table,
                recipe,
                make_all,
                mode,
                button,
            } => {
                if *table {
                    let target = FIXTURES
                        .iter()
                        .position(|fixture| fixture.block == "minecraft:crafting_table")
                        .expect("crafting table fixture");
                    let open = Input::Open { who: *who, target };
                    if !self.places_held_block(&open) {
                        let packet = self.to_player_input(&open);
                        self.tick(&[packet], &label("craft: open table"))?;
                    }
                }
                let place = self.to_player_input(&Input::PlaceRecipe {
                    who: *who,
                    recipe: *recipe,
                    make_all: *make_all,
                });
                self.tick(&[place], &label("craft: place recipe"))?;
                // Slot 0 is the result of both the player menu and the table.
                let take = self.click_input(*who, 0, *button, *mode, Sync::Fresh);
                self.tick(
                    std::slice::from_ref(&take),
                    &label(&format!("craft: {take:?}")),
                )?;
                self.note_step("craft", &before);
                Ok(())
            }
            Step::Drag { who, right, slots } => {
                let (start, add, end) = if *right { (4, 5, 6) } else { (0, 1, 2) };
                let mut clicks = vec![(-999, start)];
                clicks.extend(
                    slots
                        .iter()
                        .map(|&pick| (self.resolve_slot(*who, SlotPick::Menu(pick)), add)),
                );
                clicks.push((-999, end));
                for (slot, button) in clicks {
                    let packet =
                        self.click_input(*who, slot, button, ClickMode::QuickCraft, Sync::Fresh);
                    self.tick(
                        std::slice::from_ref(&packet),
                        &label(&format!("drag {packet:?}")),
                    )?;
                }
                self.note_step("drag", &before);
                Ok(())
            }
            Step::KeepInventory(on) => {
                let command = self.command(0, &format!("gamerule keep_inventory {on}"));
                self.tick(&[command], &label("gamerule keep_inventory"))?;
                self.note_step("keep-inventory", &before);
                Ok(())
            }
            Step::Die { who } => {
                // A dead player may be removed from the live set; one
                // survivor keeps the room's chunk loaded.
                if self.players.len() < 2 {
                    return Ok(());
                }
                let (connection, entity) = self.players[*who];
                let kill = self.command(*who, "kill");
                self.tick(&[kill], &label("kill"))?;
                for attempt in 0..3 {
                    self.tick(&[PlayerInput::Respawn { connection }], &label("respawn"))?;
                    if self.state.entities.slot_of(entity).is_some() {
                        break;
                    }
                    assert!(attempt < 2, "player {who} never respawned");
                }
                stand(&mut self.state, *who, entity);
                self.note_step("die", &before);
                Ok(())
            }
            Step::Reconnect { who } => {
                // With one player the room's chunk could unload and take the
                // fixtures with it; the other player keeps it resident.
                if self.players.len() < 2 {
                    return Ok(());
                }
                let (connection, _) = self.players[*who];
                run_tick(&mut self.state, &[PlayerInput::Leave { connection }]);
                run_tick(
                    &mut self.state,
                    &[PlayerInput::Join {
                        connection,
                        uuid: uuid::Uuid::from_u128(*who as u128 + 1),
                        name: format!("conserve{who}"),
                        permission_level: 0,
                    }],
                );
                let want = self.state.desired_chunks();
                self.state.world.ensure_blocking(&want);
                run_tick(
                    &mut self.state,
                    &[PlayerInput::ConfirmTeleport {
                        connection,
                        teleport_id: 1,
                    }],
                );
                let entity = self
                    .state
                    .connections
                    .entity_of(connection)
                    .expect("rejoined");
                self.players[*who].1 = entity;
                stand(&mut self.state, *who, entity);
                self.check(&label("reconnect"))?;
                self.note_step("reconnect", &before);
                Ok(())
            }
            Step::Break { who, target } => {
                let connection = self.players[*who].0;
                let position = pos(FIXTURES[*target].at);
                let current = self
                    .state
                    .world
                    .block_at(position.x, position.y, position.z);
                if current.is_none_or(|id| id == self.state.registry.states.air) {
                    return Ok(());
                }
                crate::tick::mining_tests::ready_to_finish(&mut self.state, connection, position);
                let packet = PlayerInput::Dig {
                    connection,
                    position,
                    action: DigAction::Finish,
                    sequence: 2,
                };
                run_tick(&mut self.state, std::slice::from_ref(&packet));
                // Breaking a shulker box block creates its own item: the one
                // legitimate creation of a pool item.
                let broken = self
                    .state
                    .world
                    .block_at(position.x, position.y, position.z)
                    .is_some_and(|id| id == self.state.registry.states.air);
                if broken && FIXTURES[*target].block == "minecraft:shulker_box" {
                    *self
                        .baseline
                        .entry(i64::from(self.pool_ids[Pool::ShulkerBox as usize]) * 2)
                        .or_default() += 1;
                }
                self.check(&label(&format!("{packet:?}")))?;
                self.note_step("break", &before);
                if broken {
                    self.note("break:broken".to_owned());
                }
                Ok(())
            }
        }
    }

    fn run(&mut self, steps: &[Step]) -> Result<(), String> {
        steps
            .iter()
            .enumerate()
            .try_for_each(|(index, step)| self.apply(index, step))
    }
}

/// `(minimum cases for the floor to apply, coverage key that must occur)`.
const COVERAGE_FLOORS: [(u32, &str); 14] = [
    (64, "open:effective"),
    (64, "click:Pickup:effective"),
    (64, "click:Swap:effective"),
    (64, "drag:effective"),
    (64, "wait:hopper-chain-moved"),
    (64, "craft:effective"),
    (500, "click:QuickMove:effective"),
    (500, "click:PickupAll:effective"),
    (500, "click:Throw:effective"),
    (500, "drop-selected:spawned-item-entity"),
    (500, "wait:picked-up-item-entity"),
    (500, "craft:gold-converted"),
    (500, "break:spawned-item-entity"),
    (500, "reconnect:effective"),
];

/// 64 cases by default (about 80 s in a debug build, most of it loading the
/// vanilla catalog); `PROPTEST_CASES=3000` for the long form (about 13 min).
fn config() -> ProptestConfig {
    let mut config = ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    };
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = 64;
    }
    config
}

#[test]
fn inventory_conservation_holds_for_random_player_operations() {
    let config = config();
    let cases = config.cases;
    let mut runner = TestRunner::new(config);
    let coverage = std::sync::Mutex::new(BTreeMap::<String, u64>::new());
    let outcome = runner.run(&scenario(), |scenario| {
        let mut rig = Rig::new(&scenario);
        let result = rig.run(&scenario.steps);
        // Failing cases (and their shrinking replays) are not coverage.
        if result.is_ok() {
            let mut total = coverage.lock().expect("coverage lock");
            for (key, count) in std::mem::take(&mut rig.coverage) {
                *total.entry(key).or_default() += count;
            }
        }
        result.map_err(TestCaseError::fail)
    });
    let coverage = coverage.into_inner().expect("coverage lock");
    eprintln!("coverage over {cases} cases: {coverage:#?}");
    if let Err(error) = outcome {
        panic!("{error}");
    }
    // The property must reach what it claims to cover. Each floor is far
    // below the rate measured over 1500 cases (hundreds of hits), so only a
    // generator or fixture regression can trip one.
    for (min_cases, key) in COVERAGE_FLOORS {
        if cases >= min_cases {
            assert!(
                coverage.get(key).copied().unwrap_or(0) > 0,
                "{key} never happened in {cases} cases"
            );
        }
    }
}

/// Every fixture block opens its menu for a survival player (guards the
/// property's room against silently exercising nothing).
#[test]
fn every_fixture_block_opens_a_menu() {
    let scenario = Scenario {
        players: 2,
        player_fill: Vec::new(),
        container_fill: Vec::new(),
        steps: Vec::new(),
    };
    let mut rig = Rig::new(&scenario);
    for (target, fixture) in FIXTURES.iter().enumerate() {
        for who in 0..2 {
            let packet = rig.to_player_input(&Input::Open { who, target });
            let (_, drains) = {
                let (_, drains, _) = run_tick(&mut rig.state, &[packet]);
                ((), drains)
            };
            let opened = drains
                .iter()
                .flat_map(|(_, changes)| changes)
                .any(|change| {
                    matches!(
                        change,
                        PlayerChange::MenuOpened { .. } | PlayerChange::MountMenuOpened { .. }
                    )
                });
            assert!(
                opened,
                "player {who} opens {} at {:?}",
                fixture.block, fixture.at
            );
        }
    }
}

/// The pool's stack sizes come from the vanilla catalog, not defaults.
#[test]
fn pool_items_have_vanilla_stack_sizes() {
    let registry = registry();
    let rules = registry.item_rules();
    for (pool, max) in [
        (Pool::Bedrock, 64),
        (Pool::EnderPearl, 16),
        (Pool::Saddle, 1),
        (Pool::ShulkerBox, 1),
        (Pool::Totem, 1),
        (Pool::CarvedPumpkin, 64),
        (Pool::GoldIngot, 64),
        (Pool::Bundle, 1),
    ] {
        let id = registry
            .items
            .id_of(&mc_data::Identifier::parse(pool.name()).expect("item id"))
            .expect("pool item exists");
        assert_eq!(
            rules.max_stack(&ItemStack::new(id, 1)),
            max,
            "{}",
            pool.name()
        );
    }
}
