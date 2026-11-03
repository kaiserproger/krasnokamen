use super::*;
use crate::settlements::SettlementId;
use crate::settlements::allocator::tests::block;
use crate::settlements::buildings::{BuildingId, BuildingKind};
use crate::settlements::construction::tests::{fixture, open_plan};

/// The construction fixture's bell.
const ANCHOR: [i32; 3] = [8, 201, 8];

#[test]
fn street_grid_reserves_roads_verges_lots_and_an_open_plaza() {
    assert_eq!(PERIOD, 28);
    assert_eq!(2 * (LOT_FRONT + LOT_DEPTH), PERIOD);
    for offset in [-1, 0, 1] {
        for along in [-40, 0, 13, 40] {
            assert_eq!(cell(ANCHOR, 8 + offset, along), Cell::Road);
            assert_eq!(cell(ANCHOR, along, 8 + PERIOD + offset), Cell::Road);
        }
    }
    // The square around the bell stays open, verges included.
    assert_eq!(cell(ANCHOR, 12, 12), Cell::Plaza);
    assert_eq!(cell(ANCHOR, 10, 12), Cell::Plaza);
    assert_eq!(cell(ANCHOR, 10, 13), Cell::Verge);
    // East of the avenue: verge, two lot rows of the deepest blueprint with
    // the one-column gap between them, the next verge and road.
    assert_eq!(cell(ANCHOR, 10, 20), Cell::Verge);
    for x in 11..=33 {
        assert_eq!(cell(ANCHOR, x, 20), Cell::Lot, "{x}");
    }
    assert_eq!(cell(ANCHOR, 34, 20), Cell::Verge);
    assert_eq!(cell(ANCHOR, 35, 20), Cell::Road);
}

#[test]
fn stretches_partition_every_road_column_and_own_one_verge_lamp() {
    let mut owner = HashMap::new();
    for x in -40..=60 {
        for z in -40..=60 {
            let found = tile(ANCHOR, x, z);
            if cell(ANCHOR, x, z) != Cell::Road {
                assert_eq!(found, None, "{x} {z}");
                continue;
            }
            let found = found.unwrap_or_else(|| panic!("road {x} {z} has no stretch"));
            assert!(
                (found.min[0]..=found.max[0]).contains(&x)
                    && (found.min[1]..=found.max[1]).contains(&z),
                "{x} {z} outside {found:?}"
            );
            owner.insert((x, z), found);
        }
    }
    let stretches: HashSet<Tile> = owner.values().copied().collect();
    let mut lamps = HashSet::new();
    for stretch in &stretches {
        for x in stretch.min[0]..=stretch.max[0] {
            for z in stretch.min[1]..=stretch.max[1] {
                if let Some(found) = owner.get(&(x, z)) {
                    assert_eq!(found, stretch, "stretches overlap at {x} {z}");
                }
            }
        }
        let [x, z] = stretch.lamp;
        assert!(lamp_at(ANCHOR, x, z), "{stretch:?}");
        assert!(
            matches!(cell(ANCHOR, x, z), Cell::Verge | Cell::Plaza),
            "{stretch:?}"
        );
        assert!(lamps.insert(stretch.lamp), "shared lamp {stretch:?}");
    }
    // The crossing at the bell owns the plaza corner lamp; stretches run eight
    // to fourteen blocks.
    assert!(lamps.contains(&[10, 10]));
    assert!(stretches.iter().all(|stretch| {
        let length = (stretch.max[0] - stretch.min[0]).max(stretch.max[1] - stretch.min[1]) + 1;
        (8..=14).contains(&length)
    }));
}

#[test]
fn walk_reaches_every_loaded_street_and_stops_at_water() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    let loaded_roads = |network: &Network<'_>| {
        let mut reached = 0;
        for x in -16..32 {
            for z in -16..32 {
                if cell(ANCHOR, x, z) == Cell::Road {
                    assert!(network.index.contains_key(&(x, z)), "{x} {z}");
                    reached += 1;
                }
            }
        }
        reached
    };
    {
        let record = state.settlements.get(id).unwrap();
        let network = Network::walk(&state, record);
        assert!(loaded_roads(&network) > 0);
        let report = report(&state, record);
        assert!(report.contains("Cut off: 0 by terrain"), "{report}");
        assert!(report.contains("Objective: "), "{report}");
    }
    // A sealed pool across the avenue south of the plaza.
    let water = block(&state, "minecraft:water", &[]);
    for x in 7..=9 {
        for y in 197..=199 {
            state.world.set_block(x, y, 20, state.registry.states.stone);
        }
        state.world.set_block(x, 200, 20, water);
    }
    state.world.fold_overlay();
    let record = state.settlements.get(id).unwrap();
    let network = Network::walk(&state, record);
    assert!(network.index.contains_key(&(8, 19)));
    for z in 20..32 {
        assert!(!network.index.contains_key(&(8, z)), "{z}");
    }
    let report = report(&state, record);
    // Three pool columns plus the avenue beyond it up to the loaded edge.
    assert!(report.contains("Cut off: 36 by terrain"), "{report}");
}

/// Each road side places a blueprint with its front edge on the door step
/// two blocks from the road, the entrance lined up with the road column, and
/// the footprint wholly on lot columns.
#[test]
fn lots_line_the_entrance_up_with_the_road_across_the_verge() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    let record = state.settlements.get(id).unwrap();
    let mut network = Network::walk(&state, record);
    let fronts = network.frontage();
    for (name, road, normal, origin, rotation) in [
        ("storage", [9, 200, 16], [1, 0], [11, 200, 13], 1),
        ("storage", [16, 200, 7], [0, -1], [13, 200, 1], 0),
        ("storage", [-1, 200, 9], [0, 1], [-6, 200, 11], 2),
        ("storage", [7, 200, 0], [-1, 0], [1, 200, -5], 3),
        ("house", [9, 200, 16], [1, 0], [11, 200, 13], 1),
        ("farm", [16, 200, 7], [0, -1], [13, 200, -4], 0),
    ] {
        assert!(fronts.contains(&(road, normal)), "{road:?}");
        let blueprint = blueprints::get(&state.registry, name).unwrap();
        let lot = network
            .lot(blueprint, road, normal)
            .unwrap_or_else(|| panic!("{name} at {road:?}"));
        assert_eq!((lot.origin, lot.rotation), (origin, rotation), "{name}");
        let turn = Rotation::from_quarter_turns(rotation);
        let entrance = blueprint.level(1).unwrap().rotated(turn).entrance;
        let at = [
            origin[0] + i32::from(entrance[0]),
            origin[2] + i32::from(entrance[2]),
        ];
        if normal[0] == 0 {
            assert_eq!(at[0], road[0], "{name}");
        } else {
            assert_eq!(at[1], road[2], "{name}");
        }
        // The door faces the road: out of the entrance toward it.
        assert_eq!(
            COMPASS[(front(blueprint).unwrap() + usize::from(rotation)) % 4],
            [-normal[0], -normal[1]],
            "{name}"
        );
        let [sx, _, sz] = blueprint.footprint(turn).map(i32::from);
        let door = [road[0] + 2 * normal[0], road[2] + 2 * normal[1]];
        assert!(
            (origin[0]..origin[0] + sx).contains(&door[0])
                && (origin[2]..origin[2] + sz).contains(&door[1]),
            "{name}: the door step is the front edge"
        );
    }
    // Plaza road columns front nothing.
    assert!(
        fronts
            .iter()
            .all(|(at, _)| { (at[0] - ANCHOR[0]).abs().max((at[2] - ANCHOR[2]).abs()) > PLAZA })
    );
}

/// A player-certified house (no blueprint placement on record) on the lots
/// east of the avenue, two stretches south of the plaza.
fn certified_house(state: &mut TickState, id: SettlementId) {
    let record = state.settlements.get_mut(id).unwrap();
    record.building_sequence += 1;
    let building = BuildingId((id.0 << 32) | record.building_sequence);
    record.buildings.insert(
        building,
        Building {
            id: building,
            settlement: id,
            kind: BuildingKind::House,
            level: 1,
            bounds: ScriptWorldBounds::new([11, 200, 25], [15, 204, 30]).unwrap(),
            work_points: Vec::new(),
            revision: 1,
            natural_zone: false,
        },
    );
}

/// The map shows the street work the planner owes: only on the walk from the
/// plaza to a standing building, the unpaved stretches and missing lamps.
#[test]
fn street_features_owe_work_on_the_walks_to_standing_buildings() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    let grass = block(&state, "minecraft:grass_block", &[]);
    for x in 7..=9 {
        for z in 13..=23 {
            state.world.set_block(x, 200, z, grass);
        }
    }
    state.world.fold_overlay();
    let features = |state: &TickState| {
        street_features(state, state.settlements.get(id).unwrap())
            .into_iter()
            .map(|(kind, bounds)| (kind, bounds.min, bounds.max))
            .collect::<Vec<_>>()
    };
    assert!(features(&state).is_empty(), "no building, no street work");
    certified_house(&mut state, id);
    // The house's west side faces the avenue's second stretch; its walk runs
    // through the first. Stone needs no paving, so the road owed is exactly the
    // grass columns (z 13..23), not the whole stretch (z 10..23); both lamps are owed.
    assert_eq!(
        features(&state),
        [
            (ROAD, [7, 200, 13], [9, 200, 23]),
            (LAMP, [10, 201, 19], [10, 203, 19]),
            (LAMP, [10, 201, 28], [10, 203, 28]),
        ]
    );
}

/// The street features of one kind as `(kind, min, max)` rectangles (the
/// avenue's first stretch is x 7..9, z 10..23).
fn runs(
    state: &TickState,
    id: SettlementId,
    wanted: &str,
) -> Vec<(&'static str, [i32; 3], [i32; 3])> {
    street_features(state, state.settlements.get(id).unwrap())
        .into_iter()
        .filter(|(kind, _)| *kind == wanted)
        .map(|(kind, bounds)| (kind, bounds.min, bounds.max))
        .collect()
}

fn covers(features: &[(&'static str, [i32; 3], [i32; 3])], x: i32, z: i32) -> bool {
    features
        .iter()
        .any(|(_, min, max)| (min[0]..=max[0]).contains(&x) && (min[2]..=max[2]).contains(&z))
}

/// A finished road is a map feature of its own: the reached columns whose
/// surface is a dirt path, on every stretch (a building or not). A gap in the
/// paving (stone left as it is) splits it in two rectangles; none spans the gap.
#[test]
fn paved_road_is_exact_runs_that_never_span_a_gap() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    let path = block(&state, "minecraft:dirt_path", &[]);
    for x in 7..=9 {
        for z in (10..=12).chain(18..=23) {
            state.world.set_block(x, 200, z, path);
        }
    }
    state.world.fold_overlay();
    assert_eq!(
        runs(&state, id, PAVED),
        [
            (PAVED, [7, 200, 10], [9, 200, 12]),
            (PAVED, [7, 200, 18], [9, 200, 23]),
        ]
    );
    assert!(
        runs(&state, id, ROAD).is_empty(),
        "no building, no road work"
    );
}

/// A column of water inside the band is never drawn as road, though paved
/// columns on both sides of it are reached around it.
#[test]
fn paved_road_is_not_drawn_over_a_blocked_column() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    let path = block(&state, "minecraft:dirt_path", &[]);
    let water = block(&state, "minecraft:water", &[]);
    for x in 7..=9 {
        for z in 10..=23 {
            state.world.set_block(x, 200, z, path);
        }
    }
    for z in 15..=16 {
        state.world.set_block(8, 200, z, water);
    }
    state.world.fold_overlay();
    let paved = runs(&state, id, PAVED);
    for z in [14, 17] {
        assert!(covers(&paved, 8, z), "(8, {z}) is paved: {paved:?}");
    }
    for z in [15, 16] {
        assert!(!covers(&paved, 8, z), "(8, {z}) is water: {paved:?}");
        assert!(covers(&paved, 7, z) && covers(&paved, 9, z), "{paved:?}");
    }
}

/// A stretch part paved, part still owed paving shows both, on disjoint
/// columns: the paved columns are not painted as planned, nor the planned as paved.
#[test]
fn part_paved_part_planned_stretch_shows_each_on_its_own_columns() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    let path = block(&state, "minecraft:dirt_path", &[]);
    let grass = block(&state, "minecraft:grass_block", &[]);
    for x in 7..=9 {
        for z in 13..=16 {
            state.world.set_block(x, 200, z, path);
        }
        for z in 17..=23 {
            state.world.set_block(x, 200, z, grass);
        }
    }
    state.world.fold_overlay();
    certified_house(&mut state, id);
    assert_eq!(
        runs(&state, id, PAVED),
        [(PAVED, [7, 200, 13], [9, 200, 16])]
    );
    assert_eq!(runs(&state, id, ROAD), [(ROAD, [7, 200, 17], [9, 200, 23])]);
}
