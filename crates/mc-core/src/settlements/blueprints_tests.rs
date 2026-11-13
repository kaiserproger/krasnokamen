use super::*;
use crate::settlements::allocator::tests::test_state;
use crate::settlements::buildings::{certify, upgrade};
use crate::tick::TickState;

const IDS: [(&str, usize); 21] = [
    ("house", 3),
    ("storage", 2),
    ("farm", 2),
    ("mason", 2),
    ("shepherd", 1),
    ("lumber_camp", 2),
    ("mine", 2),
    ("barracks", 3),
    ("wall", 3),
    ("watch_tower", 3),
    ("gate", 3),
    ("armorer", 2),
    ("butcher", 2),
    ("cartographer", 2),
    ("cleric", 2),
    ("fisherman", 2),
    ("fletcher", 2),
    ("leatherworker", 2),
    ("librarian", 2),
    ("toolsmith", 2),
    ("weaponsmith", 2),
];

/// The vanilla job-site block each new workshop is built around.
const WORKSHOP_STATIONS: [(&str, &str); 10] = [
    ("armorer", "blast_furnace"),
    ("butcher", "smoker"),
    ("cartographer", "cartography_table"),
    ("cleric", "brewing_stand"),
    ("fisherman", "barrel"),
    ("fletcher", "fletching_table"),
    ("leatherworker", "cauldron"),
    ("librarian", "lectern"),
    ("toolsmith", "smithing_table"),
    ("weaponsmith", "grindstone"),
];

fn state_name(state: &TickState, id: BlockStateId) -> (String, Vec<(String, String)>) {
    let def = state.registry.blocks.by_id(id).unwrap();
    (def.block.id.path().to_owned(), def.properties.clone())
}

#[test]
fn catalog_has_every_blueprint_and_level_within_size_limits() {
    let state = test_state();
    let all = all(&state.registry);
    assert_eq!(all.len(), IDS.len());
    for (id, levels) in IDS {
        let blueprint = get(&state.registry, id).unwrap_or_else(|| panic!("missing {id}"));
        assert_eq!(blueprint.levels.len(), levels, "{id} levels");
        for level in &blueprint.levels {
            assert!(
                level.size.iter().zip(MAX_SIZE).all(|(s, m)| *s <= m),
                "{id} size"
            );
            assert!(!level.blocks.is_empty() && !level.materials.is_empty());
        }
    }
}

/// Most materials bottom out in worker-produced resources. The shepherd's
/// loom also accepts real delivered string, which the planner reports when absent.
/// A workshop's own job-site block (iron, flint, books or a blaze rod behind it)
/// is delivered or crafted from delivered stock, never produced by a worker.
#[test]
fn materials_use_only_worker_producible_items() {
    let state = test_state();
    let wood = ["oak", "spruce", "birch", "dark_oak"];
    let derived = [
        "planks",
        "log",
        "stairs",
        "slab",
        "fence",
        "fence_gate",
        "door",
        "trapdoor",
        "pressure_plate",
    ];
    let plain = [
        "chest",
        "barrel",
        "crafting_table",
        "composter",
        "ladder",
        "loom",
        "torch",
        "cobblestone",
        "cobblestone_wall",
        "furnace",
        "dirt",
        "wheat_seeds",
    ];
    for blueprint in all(&state.registry) {
        for level in &blueprint.levels {
            for (item, count) in &level.materials {
                assert!(*count > 0);
                let name = state
                    .registry
                    .items
                    .name_of(*item)
                    .unwrap()
                    .path()
                    .to_owned();
                let ok = plain.contains(&name.as_str())
                    || WORKSHOP_STATIONS.contains(&(blueprint.id.as_str(), name.as_str()))
                    || name.ends_with("_bed")
                    || name
                        .strip_prefix("cobblestone_")
                        .is_some_and(|s| matches!(s, "stairs" | "slab"))
                    || wood.iter().any(|w| {
                        name.strip_prefix(w)
                            .and_then(|rest| rest.strip_prefix('_'))
                            .is_some_and(|rest| derived.contains(&rest))
                    });
                assert!(ok, "{}: item {name} is not worker-producible", blueprint.id);
            }
        }
    }
}

#[test]
fn material_costs_follow_block_halves_and_variants() {
    let state = test_state();
    let block = |text: &str| parse_state(&state.registry, text).unwrap();
    let cost = |text: &str| {
        material_for(&state.registry, block(text)).map(|(item, n)| {
            (
                state
                    .registry
                    .items
                    .name_of(item)
                    .unwrap()
                    .path()
                    .to_owned(),
                n,
            )
        })
    };
    assert_eq!(cost("oak_door[half=lower]"), Some(("oak_door".into(), 1)));
    assert_eq!(cost("oak_door[half=upper]"), None);
    assert_eq!(cost("red_bed[part=head]"), None);
    assert_eq!(cost("red_bed[part=foot]"), Some(("red_bed".into(), 1)));
    assert_eq!(
        cost("spruce_slab[type=double]"),
        Some(("spruce_slab".into(), 2))
    );
    assert_eq!(
        cost("spruce_slab[type=bottom]"),
        Some(("spruce_slab".into(), 1))
    );
    assert_eq!(cost("wall_torch[facing=east]"), Some(("torch".into(), 1)));
    assert_eq!(cost("farmland[moisture=7]"), Some(("dirt".into(), 1)));
    assert_eq!(cost("air"), None);
}

#[test]
fn level_totals_equal_the_sum_of_per_block_costs() {
    let state = test_state();
    for blueprint in all(&state.registry) {
        for level in &blueprint.levels {
            let mut expected: BTreeMap<ItemId, u32> = BTreeMap::new();
            for (_, id) in &level.blocks {
                if let Some((item, n)) = material_for(&state.registry, *id) {
                    *expected.entry(item).or_default() += n;
                }
            }
            let got: BTreeMap<_, _> = level.materials.iter().copied().collect();
            assert_eq!(got, expected, "{}", blueprint.id);
        }
    }
}

#[test]
fn upgrade_levels_are_diffs_that_clear_replaced_cells() {
    let state = test_state();
    let house = get(&state.registry, "house").unwrap();
    let (l1, l2) = (&house.levels[0], &house.levels[1]);
    // Growing the house clears cells that level 1 had built (the old end wall).
    assert!(!l2.removals.is_empty());
    assert!(
        l2.removals
            .iter()
            .all(|r| l1.blocks.iter().any(|(o, _)| o == r))
    );
    // Unchanged cells are not placed again.
    for (offset, id) in &l2.blocks {
        assert!(
            !l1.blocks.iter().any(|(o, i)| o == offset && i == id),
            "{offset:?} re-placed"
        );
    }
    assert!(l2.blocks.len() < l1.blocks.len() + 400);
}

/// Structure writes refuse a stocked container, so a level may grow around
/// the previous level's containers but never replace or remove one.
#[test]
fn upgrade_levels_keep_previous_containers() {
    let state = test_state();
    let raw = |second: &str, width: u8| {
        serde_json::from_str::<RawBlueprint>(&format!(
            r#"{{"id": "crate", "kind": "storage",
                "palette": {{"C": "chest[facing=south]", "b": "barrel[facing=up]", "p": "cobblestone"}},
                "levels": [
                    {{"cert": [1, 1, 1], "entrance": [0, 0, 0], "layers": [["C"]]}},
                    {{"cert": [{width}, 1, 1], "entrance": [0, 0, 0], "layers": [["{second}"]]}}
                ]}}"#
        ))
        .unwrap()
    };
    assert!(load_blueprint(&state.registry, raw("Cp", 2)).is_ok());
    for second in ["pp", "bp", ".p"] {
        let error = load_blueprint(&state.registry, raw(second, 2))
            .err()
            .unwrap_or_else(|| panic!("{second} replaced the chest"));
        assert!(
            error.contains("level 2: replaces the level 1 container at 0,0,0"),
            "{error}"
        );
    }
}

#[test]
fn rotation_turns_facing_swaps_extent_and_keeps_levels_nested() {
    let state = test_state();
    let storage = get(&state.registry, "storage").unwrap();
    assert_eq!(storage.footprint(Rotation(0)), [9, 6, 5]);
    assert_eq!(storage.footprint(Rotation(1)), [5, 6, 9]);
    let origin = [0, 0, 0];
    let (l1, l2) = (&storage.levels[0], &storage.levels[1]);
    assert_eq!(l1.rotated(Rotation(0)).bounds(origin).max, [6, 3, 4]);
    assert_eq!(l1.rotated(Rotation(1)).bounds(origin).max, [4, 3, 6]);
    for turns in 0..4 {
        let (a, b) = (
            l1.rotated(Rotation(turns)).bounds(origin),
            l2.rotated(Rotation(turns)).bounds(origin),
        );
        assert!(
            (0..3).all(|i| b.min[i] <= a.min[i] && a.max[i] <= b.max[i]),
            "turn {turns}"
        );
    }
    let house = get(&state.registry, "house").unwrap();
    let door_facing = |turns| {
        house.levels[0]
            .rotated(Rotation(turns))
            .blocks
            .iter()
            .find_map(|(_, id)| {
                let (name, props) = state_name(&state, *id);
                (name == "oak_door" && props.contains(&("half".into(), "lower".into())))
                    .then(|| props.iter().find(|(k, _)| k == "facing").unwrap().1.clone())
            })
            .unwrap()
    };
    let dirs = ["north", "east", "south", "west"];
    let start = dirs.iter().position(|d| *d == door_facing(0)).unwrap();
    for turns in 0..4 {
        assert_eq!(door_facing(turns), dirs[(start + usize::from(turns)) % 4]);
    }
    for turns in 0..4 {
        let rotated = house.levels[0].rotated(Rotation(turns));
        assert!(
            rotated
                .blocks
                .iter()
                .all(|(o, _)| o.iter().all(|v| *v >= 0))
        );
        assert_eq!(rotated.blocks.len(), house.levels[0].blocks.len());
    }
}

pub(super) fn clear_site(state: &mut TickState) {
    for x in 0..16 {
        for z in 0..16 {
            state
                .world
                .set_block(x, 200, z, state.registry.states.stone);
            for y in 201..214 {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    state.world.fold_overlay();
}

/// The invariant the construction flow relies on: place level 1 then certify; for
/// each next level clear, place the diff, re-certify the expanded box, upgrade.
#[test]
fn every_level_certifies_and_upgrades_in_every_rotation() {
    let mut state = test_state();
    let registry = state.registry.clone();
    let origin = [2, 200, 2];
    for blueprint in all(&registry) {
        for turns in 0..4 {
            clear_site(&mut state);
            let rotation = Rotation(turns);
            let settlement = state
                .settlements
                .create(
                    &mut state.settlement_sequence,
                    format!("{}-{turns}", blueprint.id),
                    "owner".into(),
                    [8, 201, 8],
                    48,
                )
                .unwrap();
            let mut building = None;
            for (index, level) in blueprint.levels.iter().enumerate() {
                let rotated = level.rotated(rotation);
                for offset in &rotated.removals {
                    let [x, y, z] = rotated.world(origin, *offset);
                    state.world.set_block(x, y, z, registry.states.air);
                }
                for (offset, id) in &rotated.blocks {
                    let [x, y, z] = rotated.world(origin, *offset);
                    state.world.set_block(x, y, z, id.0);
                }
                state.world.fold_overlay();
                let bounds = rotated.bounds(origin);
                let context = format!("{} level {} rotation {turns}", blueprint.id, index + 1);
                let id = certify(&mut state, settlement, blueprint.kind, bounds)
                    .unwrap_or_else(|e| panic!("{context}: certify: {e}"));
                if index > 0 {
                    assert_eq!(building, Some(id), "{context}: expansion keeps identity");
                    upgrade(&mut state, id).unwrap_or_else(|e| panic!("{context}: upgrade: {e}"));
                }
                building = Some(id);
                let record = &state.settlements.get(settlement).unwrap().buildings[&id];
                assert_eq!(usize::from(record.level), index + 1, "{context}");
                assert!(
                    !record.work_points.is_empty()
                        || blueprint.kind == BuildingKind::House
                        || blueprint.kind == BuildingKind::Barracks
                        || blueprint.kind == BuildingKind::Wall
                        || blueprint.kind == BuildingKind::Gate
                        || blueprint.kind == BuildingKind::Tower,
                    "{context}: work points"
                );
            }
        }
    }
}

/// Builds `levels` levels of `blueprint` unrotated at the standard site in a fresh
/// settlement, certifying/upgrading each; returns the settlement and building.
pub(super) fn build(
    state: &mut TickState,
    name: &str,
    blueprint: &Blueprint,
    levels: usize,
) -> (
    super::super::SettlementId,
    super::super::buildings::BuildingId,
) {
    let origin = [2, 200, 2];
    clear_site(state);
    let settlement = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            name.into(),
            "owner".into(),
            [8, 201, 8],
            48,
        )
        .unwrap();
    let mut building = None;
    for (index, level) in blueprint.levels.iter().take(levels).enumerate() {
        let rotated = level.rotated(Rotation(0));
        for offset in &rotated.removals {
            let [x, y, z] = rotated.world(origin, *offset);
            state.world.set_block(x, y, z, state.registry.states.air);
        }
        for (offset, id) in &rotated.blocks {
            let [x, y, z] = rotated.world(origin, *offset);
            state.world.set_block(x, y, z, id.0);
        }
        state.world.fold_overlay();
        let id = certify(state, settlement, blueprint.kind, rotated.bounds(origin)).unwrap();
        if index > 0 {
            upgrade(state, id).unwrap();
        }
        building = Some(id);
    }
    (settlement, building.unwrap())
}

/// Planner, menus and upgrade orders resolve a blueprint by building kind, so
/// every kind may have only one.
#[test]
fn every_blueprint_owns_a_distinct_building_kind() {
    let state = test_state();
    let kinds: Vec<_> = all(&state.registry).iter().map(|b| b.kind).collect();
    for (index, kind) in kinds.iter().enumerate() {
        assert!(
            !kinds[..index].contains(kind),
            "two blueprints build {kind:?}"
        );
    }
    assert!(kinds.contains(&BuildingKind::Gate));
    for (id, _) in WORKSHOP_STATIONS {
        let kind = BuildingKind::parse(id).unwrap();
        assert!(kinds.contains(&kind), "{id} has no blueprint");
        assert_eq!(get(&state.registry, id).unwrap().kind, kind);
    }
}

/// Each new workshop places its profession's vanilla job-site block, one more
/// per level, and the certificate registers exactly the level's job slots.
#[test]
fn workshops_place_their_job_site_and_gain_a_slot_per_level() {
    let mut state = test_state();
    for (id, station) in WORKSHOP_STATIONS {
        let blueprint = get(&state.registry, id).unwrap();
        let profession = BuildingKind::parse(id).unwrap().profession().unwrap();
        for levels in 1..=blueprint.levels.len() {
            let name = format!("{id}-{levels}");
            let (settlement, building) = build(&mut state, &name, blueprint, levels);
            let record = &state.settlements.get(settlement).unwrap().buildings[&building];
            assert_eq!(usize::from(record.level), levels, "{id}");
            assert_eq!(
                record.work_points.len(),
                levels,
                "{id} level {levels} slots"
            );
            for point in &record.work_points {
                let block = state.world.block_at(point[0], point[1], point[2]).unwrap();
                assert_eq!(state_name(&state, BlockStateId(block)).0, station, "{id}");
                assert!(
                    matches!(
                        crate::settlements::allocator::poi_kind(&state.registry, block),
                        Some(crate::settlements::allocator::PoiKind::Job(p)) if p == profession
                    ),
                    "{id}: the work point is not a {profession:?} job site"
                );
            }
        }
    }
}

/// The gate, wall, tower and barracks have a third level with the certificate
/// dimensions the level rules ask for (door count, length x height, tower
/// height, beds).
#[test]
fn gate_and_fortifications_reach_level_three() {
    let mut state = test_state();
    for (id, cert) in [
        ("gate", [9, 4, 3]),
        ("wall", [13, 6, 3]),
        ("watch_tower", [5, 10, 5]),
        ("barracks", [11, 5, 7]),
    ] {
        let blueprint = get(&state.registry, id).unwrap();
        assert_eq!(blueprint.levels.len(), 3, "{id}");
        assert_eq!(blueprint.levels[2].cert, cert, "{id}");
        let (settlement, building) = build(&mut state, id, blueprint, 3);
        let record = &state.settlements.get(settlement).unwrap().buildings[&building];
        assert_eq!(record.level, 3, "{id}");
    }
}

/// Lots are `LOT_DEPTH` deep: the extent across the front a blueprint turns to
/// the road may not exceed it (the wall's 13-block length runs along the road).
#[test]
fn every_blueprint_fits_the_street_lot_across_its_front() {
    let state = test_state();
    for blueprint in all(&state.registry) {
        let [fx, _, fz] = blueprint.footprint(Rotation(0)).map(i32::from);
        let depth = match crate::settlements::layout::front(blueprint).unwrap() {
            0 | 2 => fz,
            _ => fx,
        };
        assert!(
            depth <= crate::settlements::layout::LOT_DEPTH,
            "{} is {depth} deep across its front",
            blueprint.id
        );
    }
}
