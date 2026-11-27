use std::collections::HashMap;

use mc_data::Identifier;
use mc_world::{BlockPos, BlockRegistry};

use super::{Click, Face, Reads, place};

/// Placed cells as `((x, y, z), state)`.
type Cells = Vec<((i32, i32, i32), u32)>;

/// The block report, client tags and resolved states `mc-core`'s
/// `CoreRegistry::load(None)` builds in tests, without the core wrapper.
pub(crate) struct TestRegistry {
    pub(crate) blocks: BlockRegistry,
    pub(crate) tags: mc_data::tags::TagsData,
    pub(crate) states: TestStates,
}

pub(crate) struct TestStates {
    pub(crate) air: u32,
    pub(crate) stone: u32,
    pub(crate) water: u32,
}

impl TestRegistry {
    pub(crate) fn load() -> Self {
        let report = mc_data::blocks::krasnokamen_required_blocks_report();
        let blocks = BlockRegistry::from_report(&report).expect("registry loads");
        let items = mc_data::items::krasnokamen_required_items();
        let tags = mc_data::tags::krasnokamen_required_client_tags(&items, &report);
        let default_of = |name: &str| {
            blocks
                .block(&Identifier::parse(name).expect("vanilla block identifier parses"))
                .expect("block")
                .default
                .0
        };
        let states = TestStates {
            air: default_of("minecraft:air"),
            stone: default_of("minecraft:stone"),
            water: default_of("minecraft:water"),
        };
        Self {
            blocks,
            tags,
            states,
        }
    }
}

/// Stone floor at y = 0 with whatever `extra` holds; air elsewhere.
struct World {
    registry: TestRegistry,
    extra: HashMap<(i32, i32, i32), u32>,
    /// The placer sneaks.
    sneaking: bool,
}

impl World {
    fn new() -> Self {
        Self {
            registry: TestRegistry::load(),
            extra: HashMap::new(),
            sneaking: false,
        }
    }

    fn state(&self, name: &str, props: &[(&str, &str)]) -> u32 {
        let id = Identifier::parse(name).expect("id");
        let props: Vec<(String, String)> = props
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        let block = self.registry.blocks.block(&id).expect("block");
        let mut full = self
            .registry
            .blocks
            .by_id(block.default)
            .expect("default")
            .properties
            .clone();
        for (key, value) in props {
            if let Some(entry) = full.iter_mut().find(|(name, _)| *name == key) {
                entry.1 = value;
            }
        }
        self.registry
            .blocks
            .by_name_and_props(&id, &full)
            .expect("state")
            .0
    }

    /// Places `item` clicking `face` of `clicked` with the cursor at
    /// height `cursor_y`, looking along `yaw` (0 = south).
    fn place(
        &self,
        item: &str,
        clicked: (i32, i32, i32),
        face: Face,
        cursor_y: f32,
        yaw: f32,
    ) -> Option<Cells> {
        let air = self.registry.states.air;
        let stone = self.registry.states.stone;
        let block_at = |pos: BlockPos| {
            Some(
                self.extra
                    .get(&(pos.x, pos.y, pos.z))
                    .copied()
                    .unwrap_or(if pos.y <= 0 { stone } else { air }),
            )
        };
        let replaceable = |state: u32| state == air;
        let reads = Reads {
            block_at: &block_at,
            replaceable: &replaceable,
            water_source: self.registry.states.water,
        };
        let item_block = self
            .registry
            .blocks
            .block(&Identifier::parse(item).expect("id"))
            .expect("block");
        let click = Click {
            clicked: BlockPos {
                x: clicked.0,
                y: clicked.1,
                z: clicked.2,
            },
            face,
            cursor: [0.5, cursor_y, 0.5],
            yaw,
            pitch: 0.0,
            sneaking: self.sneaking,
        };
        place(&self.registry.blocks, item_block, &click, &reads).map(|cells| {
            cells
                .into_iter()
                .map(|(pos, state)| ((pos.x, pos.y, pos.z), state))
                .collect()
        })
    }
}

#[test]
fn gap_blocks_floor_support_distinguishes_torches_and_diodes() {
    let world = World::new();
    let pos = BlockPos { x: 0, y: 2, z: 0 };
    for name in ["torch", "soul_torch", "redstone_torch", "copper_torch"] {
        for base in ["oak_fence", "cobblestone_wall"] {
            let support = world.state(base, &[]);
            assert!(
                super::supported(
                    &world.registry.blocks,
                    &world.registry.tags,
                    pos,
                    world.state(name, &[]),
                    &|_| Some(support),
                ),
                "{name} on {base}"
            );
        }
    }
    for name in ["repeater", "comparator"] {
        let state = world.state(name, &[]);
        assert_eq!(
            super::support(&world.registry.blocks, &world.registry.tags, pos, state),
            Some(BlockPos { y: 1, ..pos }),
        );
        assert!(!super::supported(
            &world.registry.blocks,
            &world.registry.tags,
            pos,
            state,
            &|_| Some(world.registry.states.air),
        ));
    }
}

#[test]
fn gap_blocks_connections_update_both_neighbors_and_disconnect_after_removal() {
    let world = World::new();
    let left = BlockPos { x: 0, y: 1, z: 0 };
    let right = BlockPos { x: 1, ..left };
    for (name, connected, disconnected) in [
        ("oak_fence", "true", "false"),
        ("glass_pane", "true", "false"),
        ("cobblestone_wall", "low", "none"),
    ] {
        let default = world.state(name, &[]);
        let mut writes = vec![(right, default)];
        super::shape_updates(&world.registry.blocks, &mut writes, &|pos| {
            Some(if pos == left {
                default
            } else {
                world.registry.states.air
            })
        })
        .unwrap();
        assert!(writes.contains(&(left, world.state(name, &[("east", connected)]))));
        assert!(writes.contains(&(right, world.state(name, &[("west", connected)]))));
        let connected_left = world.state(name, &[("east", connected)]);
        let mut removal = vec![(right, world.registry.states.air)];
        super::shape_updates(&world.registry.blocks, &mut removal, &|pos| {
            Some(if pos == left {
                connected_left
            } else {
                world.registry.states.air
            })
        })
        .unwrap();
        assert!(removal.contains(&(left, world.state(name, &[("east", disconnected)]))));
    }
}

#[test]
fn gap_blocks_door_hinges_pair_and_avoid_obstructions() {
    let mut world = World::new();
    let first = world
        .place("oak_door", (0, 0, 0), Face::Up, 1.0, 0.0)
        .unwrap();
    world.extra.extend(first);
    let second = world
        .place("oak_door", (-1, 0, 0), Face::Up, 1.0, 0.0)
        .unwrap();
    assert!(second.contains(&(
        (-1, 1, 0),
        world.state(
            "oak_door",
            &[("facing", "south"), ("half", "lower"), ("hinge", "right"),]
        )
    )));
    world.extra.clear();
    world.extra.insert((-1, 1, 0), world.registry.states.stone);
    world.extra.insert((-1, 2, 0), world.registry.states.stone);
    let door = world
        .place("oak_door", (0, 0, 0), Face::Up, 1.0, 0.0)
        .unwrap();
    assert!(door.contains(&(
        (0, 1, 0),
        world.state(
            "oak_door",
            &[("facing", "south"), ("half", "lower"), ("hinge", "right"),]
        )
    )));
}

#[test]
fn gap_blocks_isolated_rails_follow_player_axis() {
    let world = World::new();
    for name in ["rail", "powered_rail", "detector_rail", "activator_rail"] {
        for (yaw, shape) in [(0.0, "north_south"), (90.0, "east_west")] {
            assert!(
                world
                    .place(name, (0, 0, 0), Face::Up, 1.0, yaw)
                    .unwrap()
                    .contains(&((0, 1, 0), world.state(name, &[("shape", shape)])))
            );
        }
    }
}

#[test]
fn logs_follow_the_clicked_face_axis() {
    let mut world = World::new();
    let log = |world: &World, axis| world.state("minecraft:oak_log", &[("axis", axis)]);
    assert_eq!(
        world.place("minecraft:oak_log", (0, 0, 0), Face::Up, 1.0, 0.0),
        Some(vec![((0, 1, 0), log(&world, "y"))])
    );
    world.extra.insert((0, 1, 0), world.registry.states.stone);
    assert_eq!(
        world.place("minecraft:oak_log", (0, 1, 0), Face::East, 0.5, 0.0),
        Some(vec![((1, 1, 0), log(&world, "x"))])
    );
    assert_eq!(
        world.place("minecraft:oak_log", (0, 1, 0), Face::North, 0.5, 0.0),
        Some(vec![((0, 1, -1), log(&world, "z"))])
    );
}

/// Stairs face where the placer looks, upside down from a face's upper
/// half; furnaces face the placer.
#[test]
fn stairs_and_furnaces_face_from_the_placer() {
    let mut world = World::new();
    let stairs = |world: &World, half| {
        world.state(
            "minecraft:oak_stairs",
            &[("facing", "south"), ("half", half)],
        )
    };
    assert_eq!(
        world.place("minecraft:oak_stairs", (0, 0, 0), Face::Up, 1.0, 0.0),
        Some(vec![((0, 1, 0), stairs(&world, "bottom"))])
    );
    world.extra.insert((0, 1, 0), world.registry.states.stone);
    assert_eq!(
        world.place("minecraft:oak_stairs", (0, 1, 0), Face::South, 0.8, 0.0),
        Some(vec![((0, 1, 1), stairs(&world, "top"))])
    );
    world.extra.remove(&(0, 1, 0));
    assert_eq!(
        world.place("minecraft:furnace", (0, 0, 0), Face::Up, 1.0, 90.0),
        Some(vec![(
            (0, 1, 0),
            world.state("minecraft:furnace", &[("facing", "east")])
        )])
    );
}

#[test]
fn torches_stand_on_floors_and_hang_on_walls_only() {
    let mut world = World::new();
    assert_eq!(
        world.place("minecraft:torch", (0, 0, 0), Face::Up, 1.0, 0.0),
        Some(vec![((0, 1, 0), world.state("minecraft:torch", &[]))])
    );
    world.extra.insert((0, 1, 0), world.registry.states.stone);
    assert_eq!(
        world.place("minecraft:torch", (0, 1, 0), Face::West, 0.5, 0.0),
        Some(vec![(
            (-1, 1, 0),
            world.state("minecraft:wall_torch", &[("facing", "west")])
        )])
    );
    assert_eq!(
        world.place("minecraft:torch", (0, 2, 0), Face::Down, 0.0, 0.0),
        None
    );
}

/// Doors and beds claim two cells, or none when the second is taken.
#[test]
fn doors_and_beds_place_both_halves() {
    let mut world = World::new();
    let door = |half| {
        world.state(
            "minecraft:oak_door",
            &[("facing", "south"), ("half", half), ("hinge", "left")],
        )
    };
    assert_eq!(
        world.place("minecraft:oak_door", (0, 0, 0), Face::Up, 1.0, 0.0),
        Some(vec![((0, 1, 0), door("lower")), ((0, 2, 0), door("upper"))])
    );
    let bed = |part| world.state("minecraft:red_bed", &[("facing", "west"), ("part", part)]);
    assert_eq!(
        world.place("minecraft:red_bed", (0, 0, 0), Face::Up, 1.0, 90.0),
        Some(vec![((0, 1, 0), bed("foot")), ((-1, 1, 0), bed("head"))])
    );
    let stone = world.registry.states.stone;
    world.extra.insert((0, 2, 0), stone);
    assert_eq!(
        world.place("minecraft:oak_door", (0, 0, 0), Face::Up, 1.0, 0.0),
        None
    );
}

/// A slab clicked from its open side doubles in place; a side click on the
/// upper half places a top slab.
#[test]
fn slabs_double_or_take_the_clicked_half() {
    let mut world = World::new();
    let [bottom, top, double] = ["bottom", "top", "double"]
        .map(|slab_type| world.state("minecraft:oak_slab", &[("type", slab_type)]));
    world.extra.insert((0, 1, 0), bottom);
    assert_eq!(
        world.place("minecraft:oak_slab", (0, 1, 0), Face::Up, 0.5, 0.0),
        Some(vec![((0, 1, 0), double)])
    );
    assert_eq!(
        world.place("minecraft:oak_slab", (0, 1, 0), Face::East, 0.4, 0.0),
        Some(vec![((1, 1, 0), bottom)])
    );
    world.extra.insert((0, 1, 0), world.registry.states.stone);
    assert_eq!(
        world.place("minecraft:oak_slab", (0, 1, 0), Face::East, 0.8, 0.0),
        Some(vec![((1, 1, 0), top)])
    );
}

/// A hand opens both halves of a wooden door, never an iron one; a fence
/// gate swings away from the user.
#[test]
fn hands_open_doors_and_gates() {
    let mut world = World::new();
    let door = |half, open| {
        world.state(
            "minecraft:oak_door",
            &[("facing", "south"), ("half", half), ("open", open)],
        )
    };
    let (lower, upper) = (door("lower", "false"), door("upper", "false"));
    let (lower_open, upper_open) = (door("lower", "true"), door("upper", "true"));
    let gate = |facing, open| {
        world.state(
            "minecraft:oak_fence_gate",
            &[("facing", facing), ("open", open)],
        )
    };
    let (gate_north, gate_south_open) = (gate("north", "false"), gate("south", "true"));
    let iron = world.state("minecraft:iron_door", &[("half", "lower")]);
    world.extra.insert((0, 1, 0), lower);
    world.extra.insert((0, 2, 0), upper);
    world.extra.insert((5, 1, 0), gate_north);
    world.extra.insert((9, 1, 0), iron);
    let use_at = |world: &World, pos: (i32, i32, i32), yaw: f32| {
        let stone = world.registry.states.stone;
        let air = world.registry.states.air;
        let block_at = |pos: BlockPos| {
            Some(
                world
                    .extra
                    .get(&(pos.x, pos.y, pos.z))
                    .copied()
                    .unwrap_or(if pos.y <= 0 { stone } else { air }),
            )
        };
        super::use_block(
            &world.registry.blocks,
            BlockPos {
                x: pos.0,
                y: pos.1,
                z: pos.2,
            },
            yaw,
            &block_at,
        )
        .map(|cells| {
            cells
                .into_iter()
                .map(|(pos, state)| ((pos.x, pos.y, pos.z), state))
                .collect::<Cells>()
        })
    };
    assert_eq!(
        use_at(&world, (0, 2, 0), 0.0),
        Some(vec![((0, 2, 0), upper_open), ((0, 1, 0), lower_open)])
    );
    // Facing south (yaw 0) at a north-facing gate: it swings south.
    assert_eq!(
        use_at(&world, (5, 1, 0), 0.0),
        Some(vec![((5, 1, 0), gate_south_open)])
    );
    assert_eq!(use_at(&world, (9, 1, 0), 0.0), None);
}

fn chest(world: &World, facing: &str, kind: &str) -> u32 {
    world.state("minecraft:chest", &[("facing", facing), ("type", kind)])
}

/// A chest faces the placer; beside a single chest of the same facing it
/// joins it (vanilla `ChestBlock.getStateForPlacement`): the new chest and
/// the neighbour get opposite types.
#[test]
fn chest_beside_a_chest_forms_a_double_chest() {
    let mut world = World::new();
    // Facing south (yaw 0), the chest faces north.
    assert_eq!(
        world.place("minecraft:chest", (0, 0, 0), Face::Up, 1.0, 0.0),
        Some(vec![((0, 1, 0), chest(&world, "north", "single"))])
    );
    world
        .extra
        .insert((0, 1, 0), chest(&world, "north", "single"));
    assert_eq!(
        world.place("minecraft:chest", (1, 0, 0), Face::Up, 1.0, 0.0),
        Some(vec![
            ((1, 1, 0), chest(&world, "north", "right")),
            ((0, 1, 0), chest(&world, "north", "left")),
        ])
    );
    assert_eq!(
        world.place("minecraft:chest", (-1, 0, 0), Face::Up, 1.0, 0.0),
        Some(vec![
            ((-1, 1, 0), chest(&world, "north", "left")),
            ((0, 1, 0), chest(&world, "north", "right")),
        ])
    );
    // Another facing, or a chest in line with the facing, stays single.
    assert_eq!(
        world.place("minecraft:chest", (1, 0, 0), Face::Up, 1.0, 90.0),
        Some(vec![((1, 1, 0), chest(&world, "east", "single"))])
    );
    assert_eq!(
        world.place("minecraft:chest", (0, 0, 1), Face::Up, 1.0, 0.0),
        Some(vec![((0, 1, 1), chest(&world, "north", "single"))])
    );
}

/// Sneaking against a chest's side joins it whatever the placer faces, and
/// sneaking on top does not join at all.
#[test]
fn sneaking_chest_joins_the_clicked_side_only() {
    let mut world = World::new();
    world
        .extra
        .insert((0, 1, 0), chest(&world, "north", "single"));
    world.sneaking = true;
    assert_eq!(
        world.place("minecraft:chest", (0, 1, 0), Face::East, 0.5, 180.0),
        Some(vec![
            ((1, 1, 0), chest(&world, "north", "right")),
            ((0, 1, 0), chest(&world, "north", "left")),
        ])
    );
    assert_eq!(
        world.place("minecraft:chest", (1, 0, 0), Face::Up, 1.0, 0.0),
        Some(vec![((1, 1, 0), chest(&world, "north", "single"))])
    );
}

/// A double chest's storage cells run right half first; breaking one half
/// turns the other back into a single chest.
#[test]
fn double_chest_cells_and_break_revert() {
    let mut world = World::new();
    world
        .extra
        .insert((1, 1, 0), chest(&world, "north", "right"));
    world
        .extra
        .insert((0, 1, 0), chest(&world, "north", "left"));
    let air = world.registry.states.air;
    let block_at = |pos: BlockPos| {
        Some(
            world
                .extra
                .get(&(pos.x, pos.y, pos.z))
                .copied()
                .unwrap_or(air),
        )
    };
    let cell = |x| BlockPos { x, y: 1, z: 0 };
    let blocks = &world.registry.blocks;
    assert_eq!(
        super::chests::storage_cells(blocks, cell(0), &block_at),
        vec![cell(1), cell(0)]
    );
    assert_eq!(
        super::chests::storage_cells(blocks, cell(1), &block_at),
        vec![cell(1), cell(0)]
    );
    assert_eq!(
        super::chests::break_partner(blocks, cell(0), chest(&world, "north", "left"), &block_at),
        Some((
            cell(1),
            chest(&world, "north", "right"),
            chest(&world, "north", "single")
        ))
    );
    // A lone half (partner gone) is just itself.
    let lonely = |pos: BlockPos| {
        (pos.x == 0)
            .then(|| chest(&world, "north", "left"))
            .or(Some(air))
    };
    assert_eq!(
        super::chests::storage_cells(blocks, cell(0), &lonely),
        vec![cell(0)]
    );
}

#[test]
fn hanging_wall_placement_uses_bar_ends_and_perpendicular_looking_axis() {
    let mut world = World::new();
    let stone = world.registry.states.stone;
    world.extra.insert((0, 2, 0), stone);
    let cells = world
        .place(
            "minecraft:oak_hanging_sign",
            (0, 2, 0),
            Face::East,
            0.5,
            0.0,
        )
        .unwrap();
    assert_eq!(
        cells,
        vec![(
            (1, 2, 0),
            world.state("minecraft:oak_wall_hanging_sign", &[("facing", "north")])
        )]
    );
    let pos = BlockPos { x: 1, y: 2, z: 0 };
    let sign = cells[0].1;
    let read = |cell: BlockPos| {
        Some(
            world
                .extra
                .get(&(cell.x, cell.y, cell.z))
                .copied()
                .unwrap_or(world.registry.states.air),
        )
    };
    assert!(super::supported(
        &world.registry.blocks,
        &world.registry.tags,
        pos,
        sign,
        &read
    ));
    assert!(super::support_changed(
        &world.registry.blocks,
        &world.registry.tags,
        pos,
        sign,
        BlockPos { x: 0, ..pos }
    ));
    assert!(super::support_changed(
        &world.registry.blocks,
        &world.registry.tags,
        pos,
        sign,
        BlockPos { x: 2, ..pos }
    ));
    assert!(!super::support_changed(
        &world.registry.blocks,
        &world.registry.tags,
        pos,
        sign,
        BlockPos { z: 1, ..pos }
    ));
    world.extra.remove(&(0, 2, 0));
    world.extra.insert((1, 2, 1), stone);
    let read = |cell: BlockPos| {
        Some(
            world
                .extra
                .get(&(cell.x, cell.y, cell.z))
                .copied()
                .unwrap_or(world.registry.states.air),
        )
    };
    assert!(
        !super::supported(
            &world.registry.blocks,
            &world.registry.tags,
            pos,
            sign,
            &read
        ),
        "the wall behind the text is not bar support"
    );
    let chain = world.state("minecraft:birch_wall_hanging_sign", &[("facing", "south")]);
    world.extra.insert((2, 2, 0), chain);
    let read = |cell: BlockPos| {
        Some(
            world
                .extra
                .get(&(cell.x, cell.y, cell.z))
                .copied()
                .unwrap_or(world.registry.states.air),
        )
    };
    assert!(super::supported(
        &world.registry.blocks,
        &world.registry.tags,
        pos,
        sign,
        &read
    ));
    let crossed = world.state("minecraft:birch_wall_hanging_sign", &[("facing", "east")]);
    world.extra.insert((2, 2, 0), crossed);
    let read = |cell: BlockPos| {
        Some(
            world
                .extra
                .get(&(cell.x, cell.y, cell.z))
                .copied()
                .unwrap_or(world.registry.states.air),
        )
    };
    assert!(
        !super::supported(
            &world.registry.blocks,
            &world.registry.tags,
            pos,
            sign,
            &read
        ),
        "only same-axis hanging bars chain"
    );
}

#[test]
fn hanging_ceiling_attachment_and_rotation_follow_full_underside_and_sneaking() {
    let mut world = World::new();
    world.extra.insert((0, 3, 0), world.registry.states.stone);
    let loose = world
        .place(
            "minecraft:oak_hanging_sign",
            (0, 3, 0),
            Face::Down,
            0.5,
            22.0,
        )
        .unwrap();
    assert_eq!(
        loose,
        vec![(
            (0, 2, 0),
            world.state(
                "minecraft:oak_hanging_sign",
                &[("attached", "false"), ("rotation", "8")]
            )
        )]
    );
    world.sneaking = true;
    let attached = world
        .place(
            "minecraft:oak_hanging_sign",
            (0, 3, 0),
            Face::Down,
            0.5,
            22.0,
        )
        .unwrap();
    assert_eq!(
        attached,
        vec![(
            (0, 2, 0),
            world.state(
                "minecraft:oak_hanging_sign",
                &[("attached", "true"), ("rotation", "9")]
            )
        )]
    );
    world.sneaking = false;
    let aligned = world.state("minecraft:birch_hanging_sign", &[("rotation", "8")]);
    world.extra.insert((0, 3, 0), aligned);
    assert_eq!(
        world
            .place(
                "minecraft:oak_hanging_sign",
                (0, 3, 0),
                Face::Down,
                0.5,
                22.0
            )
            .unwrap(),
        loose
    );
    let diagonal = world.state("minecraft:birch_hanging_sign", &[("rotation", "1")]);
    world.extra.insert((0, 3, 0), diagonal);
    assert_eq!(
        world
            .place(
                "minecraft:oak_hanging_sign",
                (0, 3, 0),
                Face::Down,
                0.5,
                22.0
            )
            .unwrap(),
        attached
    );
    let wall = world.state("minecraft:birch_wall_hanging_sign", &[("facing", "north")]);
    world.extra.insert((0, 3, 0), wall);
    assert_eq!(
        world
            .place(
                "minecraft:oak_hanging_sign",
                (0, 3, 0),
                Face::Down,
                0.5,
                22.0
            )
            .unwrap(),
        loose
    );
}

#[test]
fn ceiling_hanging_sign_requires_center_underside_not_the_whole_face() {
    let mut world = World::new();
    let pos = BlockPos { x: 0, y: 2, z: 0 };
    let sign = world.state("minecraft:oak_hanging_sign", &[]);
    let fence = world.state("minecraft:oak_fence", &[]);
    world.extra.insert((0, 3, 0), fence);
    let read = |cell: BlockPos| {
        Some(
            world
                .extra
                .get(&(cell.x, cell.y, cell.z))
                .copied()
                .unwrap_or(world.registry.states.air),
        )
    };
    assert!(super::supported(
        &world.registry.blocks,
        &world.registry.tags,
        pos,
        sign,
        &read
    ));
    let top_slab = world.state("minecraft:oak_slab", &[("type", "top")]);
    world.extra.insert((0, 3, 0), top_slab);
    let read = |cell: BlockPos| {
        Some(
            world
                .extra
                .get(&(cell.x, cell.y, cell.z))
                .copied()
                .unwrap_or(world.registry.states.air),
        )
    };
    assert!(!super::supported(
        &world.registry.blocks,
        &world.registry.tags,
        pos,
        sign,
        &read
    ));
    world.extra.remove(&(0, 3, 0));
    let read = |cell: BlockPos| {
        Some(
            world
                .extra
                .get(&(cell.x, cell.y, cell.z))
                .copied()
                .unwrap_or(world.registry.states.air),
        )
    };
    assert!(!super::supported(
        &world.registry.blocks,
        &world.registry.tags,
        pos,
        sign,
        &read
    ));
}

#[test]
fn top_face_wall_hanging_sign_tries_secondary_horizontal_looking_axis() {
    let mut world = World::new();
    world.extra.insert((0, 2, 1), world.registry.states.stone);
    let cells = world.place("minecraft:oak_hanging_sign", (0, 0, 0), Face::Up, 0.5, 0.0);
    // This target is y1: neither bar end is supported, despite the floor.
    assert!(cells.is_none());
    world.extra.insert((0, 1, 1), world.registry.states.stone);
    let cells = world
        .place("minecraft:oak_hanging_sign", (0, 0, 0), Face::Up, 0.5, 0.0)
        .unwrap();
    let state = world
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(cells[0].1))
        .unwrap();
    let facing = state
        .properties
        .iter()
        .find(|(key, _)| key == "facing")
        .unwrap()
        .1
        .as_str();
    assert!(
        matches!(facing, "east" | "west"),
        "secondary look axis admits the supported north/south bar ends"
    );
}

#[test]
fn center_only_ceiling_support_keeps_free_rotation_and_attached_chains() {
    let mut world = World::new();
    let fence = world.state("minecraft:oak_fence", &[]);
    world.extra.insert((0, 3, 0), fence);
    let cells = world
        .place(
            "minecraft:oak_hanging_sign",
            (0, 3, 0),
            Face::Down,
            0.5,
            22.0,
        )
        .unwrap();
    assert_eq!(
        cells,
        vec![(
            (0, 2, 0),
            world.state(
                "minecraft:oak_hanging_sign",
                &[("attached", "true"), ("rotation", "9")]
            )
        )]
    );
}
