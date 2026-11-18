use super::*;
use crate::settlements::SettlementId;
use crate::settlements::allocator::tests::block;
use crate::settlements::blueprints::{self, Rotation};
use crate::settlements::construction::tests::{attempt, complete, fixture, fund, open_plan};
use crate::settlements::forestry::tests::player_history;

/// Ground level of the flat fixture plateau.
const FLOOR: i32 = 200;

/// One storage lot of the live street plan, as the planner walks it: the
/// reached road column behind the door, the normal into the lot, the lot.
fn storage_lot(state: &TickState, id: SettlementId) -> ([i32; 3], [i32; 2], layout::Lot, [u8; 3]) {
    let record = state.settlements.get(id).unwrap();
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let mut network = layout::Network::walk(state, record);
    for (front, normal) in network.frontage() {
        if let Some(lot) = network.lot(blueprint, front, normal) {
            let size = blueprint.footprint(Rotation::from_quarter_turns(lot.rotation));
            let bounds = ScriptWorldBounds::new(
                lot.origin,
                std::array::from_fn(|axis| lot.origin[axis] + i32::from(size[axis]) - 1),
            )
            .unwrap();
            // The fixture's bootstrap chest is a container the site search
            // refuses, so the planner never takes this lot.
            if (bounds.min[0]..=bounds.max[0]).contains(&1)
                && (bounds.min[2]..=bounds.max[2]).contains(&1)
            {
                continue;
            }
            return (front, normal, lot, size);
        }
    }
    panic!("the street plan has a storage lot");
}

fn bounds_of(lot: layout::Lot, size: [u8; 3]) -> ScriptWorldBounds {
    ScriptWorldBounds::new(
        lot.origin,
        std::array::from_fn(|axis| lot.origin[axis] + i32::from(size[axis]) - 1),
    )
    .unwrap()
}

/// The storage lot on a bare stone plateau, so only the cells a test paints
/// carry soil, water or a path.
fn plateau(state: &mut TickState, id: SettlementId) -> ([i32; 3], [i32; 2], layout::Lot, [u8; 3]) {
    open_plan(state);
    fund(state, "storage", 1);
    let (front, normal, lot, size) = storage_lot(state, id);
    let bounds = bounds_of(lot, size);
    for x in bounds.min[0] - 8..=bounds.max[0] + 8 {
        for z in bounds.min[2] - 8..=bounds.max[2] + 8 {
            state
                .world
                .set_block(x, FLOOR, z, state.registry.states.stone);
            for y in FLOOR + 1..=FLOOR + 4 {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    state.world.fold_overlay();
    (front, normal, lot, size)
}

fn path_state(state: &TickState) -> u32 {
    dirt_path(state).expect("dirt path state")
}

fn soil(state: &mut TickState, x: i32, z: i32) {
    let grass = block(state, "minecraft:grass_block", &[]);
    state.world.set_block(x, FLOOR, z, grass);
}

/// A village path straight through the lot from the road behind its door to
/// four cells past its far side, with a soil shoulder all round the lot so a
/// detour can run outside its footprint.
fn crossing_road(
    state: &mut TickState,
    front: [i32; 3],
    normal: [i32; 2],
    size: [u8; 3],
    bounds: ScriptWorldBounds,
) -> Vec<[i32; 3]> {
    for x in bounds.min[0] - 4..=bounds.max[0] + 4 {
        for z in bounds.min[2] - 4..=bounds.max[2] + 4 {
            soil(state, x, z);
        }
    }
    let path = path_state(state);
    let depth = i32::from(if normal[0] != 0 { size[0] } else { size[2] });
    let mut cells = Vec::new();
    for step in 0..=depth + 4 {
        let (x, z) = (front[0] + normal[0] * step, front[2] + normal[1] * step);
        state.world.set_block(x, FLOOR, z, path);
        cells.push([x, FLOOR, z]);
    }
    state.world.fold_overlay();
    cells
}

fn admit(
    state: &TickState,
    id: SettlementId,
    lot: layout::Lot,
    size: [u8; 3],
) -> Result<(), Reason> {
    super::super::site_refusals::check(
        state,
        state.settlements.get(id).unwrap(),
        lot.origin,
        size,
        None,
        &mut Default::default(),
        None,
    )
    .map(|_cost| ())
}

/// Every node still on the walk reaches the bell after every applied cell.
fn stays_connected(state: &mut TickState, id: SettlementId, cells: &[PlannedCell]) {
    let nodes = walk(state, state.settlements.get(id).unwrap());
    let seeds = bell_columns(state.settlements.get(id).unwrap(), &nodes);
    let before = within(&nodes, &seeds);
    for cell in cells {
        state
            .world
            .set_block(cell.at[0], cell.at[1], cell.at[2], cell.state);
        state.world.fold_overlay();
        let record = state.settlements.get(id).unwrap();
        let nodes = walk(state, record);
        let seeds = bell_columns(record, &nodes);
        let reach = within(&nodes, &seeds);
        for column in &before {
            if !nodes.contains_key(column) {
                continue;
            }
            assert!(
                reach.contains(column),
                "cell {:?} cut the walk at {column:?}",
                cell.at
            );
        }
    }
}

/// The same stable disposition order used by the shared executor.
fn executor_order(_state: &TickState, cells: &[PlannedCell]) -> Vec<PlannedCell> {
    let mut ordered: Vec<_> = cells
        .iter()
        .map(|cell| PlannedCell {
            portion: cell.portion,
            at: cell.at,
            state: cell.state,
            resource: cell.resource.clone(),
            quantity: cell.quantity,
        })
        .collect();
    ordered.sort_by_key(|cell| cell.portion);
    ordered
}

fn manifest(
    state: &TickState,
    id: SettlementId,
    lot: layout::Lot,
    size: [u8; 3],
) -> (ScriptWorldBounds, Vec<PlannedCell>) {
    let project = super::super::Project {
        settlement: id,
        blueprint: "storage".into(),
        level: 1,
        origin: lot.origin,
        rotation: lot.rotation,
        building: None,
        structure: String::new(),
        ordered: true,
        builder: None,
        order_seq: None,
    };
    super::super::planned_cells(state, &project, "oak", &mut Default::default())
        .expect("the crossing lot is prepared")
}

#[test]
fn road_crossing_lot_is_admitted_with_a_detour_and_stays_connected() {
    let (mut state, id, _) = fixture();
    let (front, normal, lot, size) = plateau(&mut state, id);
    let bounds = bounds_of(lot, size);
    let road = crossing_road(&mut state, front, normal, size, bounds);
    admit(&state, id, lot, size).expect("the crossing lot is admitted");
    let plan = plan(&state, state.settlements.get(id).unwrap(), bounds)
        .expect("the lot is admitted")
        .expect("the lot covers village work");
    assert!(
        !plan.detour.is_empty(),
        "a path cut on both sides needs a detour"
    );
    let detour: Vec<[i32; 3]> = plan.detour.iter().map(|cell| cell.at).collect();
    for at in &detour {
        assert!(
            at[0] < bounds.min[0] - 1
                || at[0] > bounds.max[0] + 1
                || at[2] < bounds.min[2] - 1
                || at[2] > bounds.max[2] + 1,
            "the detour never runs over the lot or its clearance: {at:?}"
        );
    }
    // The manifest the executor walks, in its own order: detour first, then the
    // house's cells, then the retired village cells.
    let (_, cells) = manifest(&state, id, lot, size);
    let ordered = executor_order(&state, &cells);
    let first_house = ordered
        .iter()
        .position(|cell| {
            name(&state, cell.state).is_some_and(|name| name != "dirt_path" && name != "dirt")
        })
        .expect("the house places cells");
    let last_detour = ordered
        .iter()
        .rposition(|cell| name(&state, cell.state) == Some("dirt_path"))
        .expect("the detour is in the manifest");
    assert!(
        last_detour < first_house,
        "every detour cell stands before the first house cell"
    );
    stays_connected(&mut state, id, &ordered);
    for at in &detour {
        assert_eq!(
            name(&state, state.world.block_at(at[0], at[1], at[2]).unwrap()),
            Some("dirt_path"),
            "detour cell {at:?}"
        );
    }
    for at in &road {
        if (bounds.min[0]..=bounds.max[0]).contains(&at[0])
            && (bounds.min[2]..=bounds.max[2]).contains(&at[2])
        {
            assert_ne!(
                name(&state, state.world.block_at(at[0], at[1], at[2]).unwrap()),
                Some("dirt_path"),
                "the lot's own path cell is retired or built over: {at:?}"
            );
        }
    }
}

#[test]
fn an_immature_field_leaves_only_its_seed() {
    let (state, _, _) = fixture();
    let seed = item_of(&state, "wheat_seeds");
    let wheat = item_of(&state, "wheat");
    let young = block(&state, "minecraft:wheat", &[("age", "0")]);
    let ripe = block(&state, "minecraft:wheat", &[("age", "7")]);
    let young = crop_drops(&state, young);
    assert_eq!(
        young
            .iter()
            .map(|stack| (stack.item_id, stack.count))
            .collect::<Vec<_>>(),
        vec![(seed, 1)],
        "an unripe plant gives the seed back"
    );
    let ripe = crop_drops(&state, ripe);
    assert!(
        ripe.iter()
            .any(|stack| stack.item_id == wheat && stack.count == 1)
            && ripe.iter().any(|stack| stack.item_id == seed),
        "a ripe plant gives wheat and its seed: {ripe:?}"
    );
}

#[test]
fn a_lot_whose_cut_cannot_be_bridged_is_refused() {
    let (mut state, id, _) = fixture();
    let (front, normal, lot, size) = plateau(&mut state, id);
    let bounds = bounds_of(lot, size);
    crossing_road(&mut state, front, normal, size, bounds);
    // A moat of still water one cell outside the lot's clearance: no column
    // there can carry a path, so the cut has no route.
    let water = block(&state, "minecraft:water", &[]);
    for x in bounds.min[0] - 7..=bounds.max[0] + 7 {
        for z in bounds.min[2] - 7..=bounds.max[2] + 7 {
            if (bounds.min[0] - 1..=bounds.max[0] + 1).contains(&x)
                && (bounds.min[2] - 1..=bounds.max[2] + 1).contains(&z)
            {
                continue;
            }
            if name(&state, state.world.block_at(x, FLOOR, z).unwrap()) == Some("dirt_path") {
                continue;
            }
            for y in FLOOR + 1..=FLOOR + 3 {
                state.world.set_block(x, y, z, water);
            }
        }
    }
    state.world.fold_overlay();
    assert_eq!(
        plan(&state, state.settlements.get(id).unwrap(), bounds).err(),
        Some(Reason::NoPathDetour),
        "a cut with no dry route around it is a named refusal"
    );
    assert_eq!(
        admit(&state, id, lot, size).err(),
        Some(Reason::NoPathDetour),
        "the admission refuses the lot with the same named reason"
    );
}

/// Outside the territory the player's village path stays the player's: the same
/// history on a cell the record does not own is still a refusal (D27 p.4).
#[test]
fn a_player_placed_path_outside_the_territory_is_refused() {
    let (mut state, id, _) = fixture();
    let (front, normal, lot, size) = plateau(&mut state, id);
    let bounds = bounds_of(lot, size);
    let mut road = crossing_road(&mut state, front, normal, size, bounds);
    let at = road.remove(2);
    state.block_log = player_history(&state, at, 0);
    assert!(state.block_log.player_touched(at));
    state.settlements.get_mut(id).unwrap().radius = 1;
    assert!(!state.settlements.get(id).unwrap().contains(at));
    assert_eq!(
        plan(&state, state.settlements.get(id).unwrap(), bounds).err(),
        Some(Reason::ProtectedObstruction),
        "outside the territory the player's block is not the village's"
    );
}

/// Owner D27 p.4: inside the territory the block's history is not consulted, so
/// a player-placed village path the lot covers is re-laid like any other.
#[test]
fn a_player_placed_path_is_relaid() {
    let (mut state, id, _) = fixture();
    let (front, normal, lot, size) = plateau(&mut state, id);
    let bounds = bounds_of(lot, size);
    let mut road = crossing_road(&mut state, front, normal, size, bounds);
    let at = road.remove(2);
    let held = state.world.block_at(at[0], at[1], at[2]).unwrap();
    state.block_log = player_history(&state, at, 0);
    assert!(state.block_log.player_touched(at));
    // D27 p.4 consults history only outside the territory: this lot is inside.
    state.settlements.get_mut(id).unwrap().radius = 512;
    assert!(state.settlements.get(id).unwrap().contains(at));
    let planned = plan(&state, state.settlements.get(id).unwrap(), bounds)
        .expect("a player path is not a refusal")
        .expect("the lot covers village work");
    assert!(
        planned.retires(at),
        "the player-placed path cell is the re-lay's own work"
    );
    assert_eq!(
        state.world.block_at(at[0], at[1], at[2]),
        Some(held),
        "planning does not touch the world"
    );
}

#[test]
fn farmland_removal_conserves_the_harvest() {
    let (mut state, id, builder) = fixture();
    let (_, _, lot, size) = plateau(&mut state, id);
    let bounds = bounds_of(lot, size);
    let farmland = block(&state, "minecraft:farmland", &[("moisture", "7")]);
    let wheat = block(&state, "minecraft:wheat", &[("age", "7")]);
    let mut crops = Vec::new();
    for x in bounds.min[0]..=bounds.max[0] {
        for z in bounds.min[2]..=bounds.max[2] {
            state.world.set_block(x, FLOOR, z, farmland);
            state.world.set_block(x, FLOOR + 1, z, wheat);
            crops.push([x, FLOOR + 1, z]);
            for y in FLOOR + 2..=FLOOR + 3 {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    // Every other lot stays a stone pillar through the whole survey band, so
    // this field is the only legal site the planner can take.
    let anchor = state.settlements.get(id).unwrap().anchor;
    for x in bounds.min[0] - 16..=bounds.max[0] + 16 {
        for z in bounds.min[2] - 16..=bounds.max[2] + 16 {
            if layout::cell(anchor, x, z) != Cell::Lot
                || ((bounds.min[0]..=bounds.max[0]).contains(&x)
                    && (bounds.min[2]..=bounds.max[2]).contains(&z))
                // The fixture's bootstrap chest must keep serving the bill.
                || (x, z) == (1, 1)
            {
                continue;
            }
            for y in FLOOR + 1..=FLOOR + 16 {
                state.world.set_block(x, y, z, state.registry.states.stone);
            }
        }
    }
    state.world.fold_overlay();
    admit(&state, id, lot, size).expect("a farmland lot is admitted");
    let planned = plan(&state, state.settlements.get(id).unwrap(), bounds)
        .expect("farmland is not a refusal")
        .expect("the lot covers village work");
    assert_eq!(
        planned.retired.len(),
        crops.len(),
        "every field cell retires"
    );
    assert!(
        planned.detour.is_empty(),
        "farmland is no road, so no detour is owed"
    );
    assert_eq!(
        planned
            .cleared
            .iter()
            .map(|cell| cell.at)
            .collect::<Vec<_>>(),
        crops,
        "every crop is a clear-stage cell"
    );
    let loot: Vec<_> = crops
        .iter()
        .flat_map(|&at| crop_drops(&state, state.world.block_at(at[0], at[1], at[2]).unwrap()))
        .map(|stack| (stack.item_id, stack.count))
        .collect();
    assert!(
        loot.iter()
            .any(|&(item, count)| item == item_of(&state, "wheat") && count >= 1),
        "a mature field yields wheat: {loot:?}"
    );
    // The same plan executes: the clear stage banks the harvest, the build
    // stage retires the field to the soil it is made of. The site search is
    // covered by the crossing-lot test, so this hands its own lot straight to
    // the canonical executor.
    let project = super::super::Project {
        settlement: id,
        blueprint: "storage".into(),
        level: 1,
        origin: lot.origin,
        rotation: lot.rotation,
        building: None,
        structure: String::new(),
        ordered: true,
        builder: None,
        order_seq: None,
    };
    assert!(
        super::super::begin(&mut state, project, "oak", 0),
        "the field is prepared and admitted"
    );
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .projects
            .first()
            .map(|project| (project.origin, project.rotation)),
        Some((lot.origin, lot.rotation)),
        "the project is the farmland lot"
    );
    complete(&mut state, id, builder);
    for at in &crops {
        assert_ne!(
            name(&state, state.world.block_at(at[0], at[1], at[2]).unwrap()),
            Some("wheat")
        );
    }
    let stored: i32 = state.block_entities[&(1, 201, 1)]
        .slots
        .iter()
        .filter(|stack| stack.item_id == item_of(&state, "wheat"))
        .map(|stack| stack.count)
        .sum();
    assert!(stored >= 1, "the harvest reached settlement storage");
}

fn item_of(state: &TickState, name: &str) -> u32 {
    state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap())
        .unwrap()
}

#[test]
fn a_detour_manifest_opens_after_a_restart() {
    let (mut state, id, builder) = fixture();
    let (front, normal, lot, size) = plateau(&mut state, id);
    let bounds = bounds_of(lot, size);
    crossing_road(&mut state, front, normal, size, bounds);
    assert!(super::super::begin(
        &mut state,
        super::super::Project {
            settlement: id,
            blueprint: "storage".into(),
            level: 1,
            origin: lot.origin,
            rotation: lot.rotation,
            building: None,
            structure: String::new(),
            ordered: true,
            builder: None,
            order_seq: None,
        },
        "oak",
        0
    ));
    // A few cells of the detour stand, then the startup check re-reads the
    // serialized ledger exactly as opening a world does.
    for _ in 0..4 {
        attempt(&mut state, id, builder);
    }
    crate::plugin_structures::validate(&state).expect("the ledger accepts the detour");
}

#[test]
fn a_raised_detour_cell_costs_one_dirt() {
    // The rule itself: natural ground is pavement (free), a cell raised above
    // it stands on material and costs one dirt.
    assert_eq!(path_cost(200, 200), 0, "natural paving is free");
    assert_eq!(path_cost(200, 201), 1, "one block up costs one dirt");
    assert_eq!(path_cost(200, 202), 1, "two blocks up costs one dirt");
    // And the plan never breaks it: every paved cell is paid exactly when it
    // stands above its column's own ground, and the bill matches the count.
    let (mut state, id, _) = fixture();
    let (front, normal, lot, size) = plateau(&mut state, id);
    let bounds = bounds_of(lot, size);
    crossing_road(&mut state, front, normal, size, bounds);
    let plan = plan(&state, state.settlements.get(id).unwrap(), bounds)
        .expect("the lot is admitted")
        .expect("the lot covers village work");
    let anchor = state.settlements.get(id).unwrap().anchor;
    let paths: Vec<_> = plan
        .detour
        .iter()
        .filter(|cell| cell.state == dirt_path(&state).unwrap())
        .collect();
    assert!(!paths.is_empty(), "the detour is paved");
    let mut bill = 0;
    for cell in &paths {
        let natural = super::super::surface_y(&state, anchor[1], cell.at[0], cell.at[2])
            .unwrap_or(cell.at[1]);
        let cost = path_cost(natural, cell.at[1]);
        assert_eq!(
            (cell.resource.is_some(), cell.quantity),
            (cost != 0, cost),
            "paved cell {:?} costs what its height says",
            cell.at
        );
        if cost != 0 {
            let filled = plan.detour.iter().any(|fill| {
                fill.state == state.registry.states.dirt
                    && fill.at == [cell.at[0], cell.at[1] - 1, cell.at[2]]
            });
            assert!(
                filled || cell.at[1] - natural <= 1,
                "a cell two or more above its own fill is a block from nothing"
            );
        }
        bill += cell.quantity;
    }
    let paid: u32 = plan
        .detour
        .iter()
        .filter(|cell| cell.resource.is_some())
        .map(|cell| cell.quantity)
        .sum();
    assert!(
        paid >= bill,
        "the detour pays at least every raised cell it lays"
    );
}

#[test]
fn a_dug_out_path_is_never_rewritten_as_soil() {
    let (mut state, id, builder) = fixture();
    let (front, normal, lot, size) = plateau(&mut state, id);
    let bounds = bounds_of(lot, size);
    crossing_road(&mut state, front, normal, size, bounds);
    let plan = plan(&state, state.settlements.get(id).unwrap(), bounds)
        .expect("the lot is admitted")
        .expect("the lot covers village work");
    let retired = plan.retired[0].at;
    assert!(super::super::begin(
        &mut state,
        super::super::Project {
            settlement: id,
            blueprint: "storage".into(),
            level: 1,
            origin: lot.origin,
            rotation: lot.rotation,
            building: None,
            structure: String::new(),
            ordered: true,
            builder: None,
            order_seq: None,
        },
        "oak",
        0
    ));
    // The player digs the path out before its retirement cell is reached: the
    // footprint changes and the project pauses. Its manifest must not turn the
    // hole into soil.
    state.world.set_block(
        retired[0],
        retired[1],
        retired[2],
        state.registry.states.air,
    );
    state.world.fold_overlay();
    for _ in 0..24 {
        attempt(&mut state, id, builder);
    }
    assert_eq!(
        state
            .world
            .block_at(retired[0], retired[1], retired[2])
            .unwrap(),
        state.registry.states.air,
        "a dug-out path cell stays air, never dirt from nothing"
    );
}

#[test]
fn only_the_retired_cell_is_exempt_from_the_protected_floor() {
    let (mut state, id, _) = fixture();
    let (_, _, lot, size) = plateau(&mut state, id);
    let bounds = bounds_of(lot, size);
    // The column's own surface is a village path, which the plan retires at
    // 201; its lot floor at 200 is a field cell that belongs to nobody. Only
    // the retired cell [x, 201, z] may pass, never the whole column.
    let (x, z) = (bounds.min[0] + 1, bounds.min[2] + 1);
    let path = path_state(&state);
    let farmland = block(&state, "minecraft:farmland", &[("moisture", "7")]);
    state.world.set_block(x, FLOOR, z, farmland);
    state.world.set_block(x, FLOOR + 1, z, path);
    for y in FLOOR + 2..=FLOOR + 4 {
        state.world.set_block(x, y, z, state.registry.states.air);
    }
    state.world.fold_overlay();
    let planned = plan(&state, state.settlements.get(id).unwrap(), bounds)
        .expect("the path on the column is village work")
        .expect("the lot covers village work");
    assert!(
        planned.retires([x, FLOOR + 1, z]),
        "the path cell above the floor is the retired one"
    );
    assert_eq!(
        admit(&state, id, lot, size).err(),
        Some(Reason::ProtectedFloor),
        "a protected block at another height of the same column stays refused"
    );
}

#[test]
fn a_two_block_flower_crop_is_reaped_once_and_leaves_its_seed() {
    let (mut state, id, _) = fixture();
    let (_, _, lot, size) = plateau(&mut state, id);
    let bounds = bounds_of(lot, size);
    let farmland = block(&state, "minecraft:farmland", &[("moisture", "7")]);
    let lower = block(
        &state,
        "minecraft:pitcher_crop",
        &[("age", "4"), ("half", "lower")],
    );
    let upper = block(
        &state,
        "minecraft:pitcher_crop",
        &[("age", "4"), ("half", "upper")],
    );
    let (x, z) = (bounds.min[0] + 2, bounds.min[2] + 2);
    state.world.set_block(x, FLOOR, z, farmland);
    state.world.set_block(x, FLOOR + 1, z, lower);
    state.world.set_block(x, FLOOR + 2, z, upper);
    for y in FLOOR + 3..=FLOOR + 4 {
        state.world.set_block(x, y, z, state.registry.states.air);
    }
    state.world.fold_overlay();
    let planned = plan(&state, state.settlements.get(id).unwrap(), bounds)
        .expect("the field is village work")
        .expect("the lot covers village work");
    let reaped: Vec<_> = planned
        .cleared
        .iter()
        .filter(|cell| cell.at[0] == x && cell.at[2] == z)
        .map(|cell| cell.at)
        .collect();
    assert_eq!(
        reaped,
        vec![[x, FLOOR + 1, z]],
        "a two-block plant is one crop: only its lower half is reaped"
    );
    let loot = crop_drops(&state, lower);
    let pods = loot
        .iter()
        .filter(|stack| stack.item_id == item_of(&state, "pitcher_pod"))
        .map(|stack| stack.count)
        .sum::<i32>();
    assert_eq!(pods, 1, "one pitcher pod, not two: {loot:?}");
    assert!(
        !loot
            .iter()
            .any(|stack| stack.item_id == item_of(&state, "pitcher_plant")),
        "the crop block never drops the flower itself: {loot:?}"
    );
}

#[test]
fn a_mature_torchflower_crop_leaves_seeds_not_the_flower() {
    let (state, _, _) = fixture();
    let crop = block(&state, "minecraft:torchflower_crop", &[("age", "1")]);
    let loot = crop_drops(&state, crop);
    let seeds = item_of(&state, "torchflower_seeds");
    assert_eq!(
        loot.iter()
            .map(|stack| (stack.item_id, stack.count))
            .collect::<Vec<_>>(),
        vec![(seeds, 1)],
        "the crop block leaves its seed at any age: {loot:?}"
    );
    assert!(
        !loot
            .iter()
            .any(|stack| stack.item_id == item_of(&state, "torchflower")),
        "the flower is the separate block's drop, not the crop's"
    );
}

#[test]
fn a_pending_detour_keeps_the_network_walkable_for_the_next_plan() {
    let (mut state, id, _) = fixture();
    let (front, normal, lot, size) = plateau(&mut state, id);
    let bounds = bounds_of(lot, size);
    crossing_road(&mut state, front, normal, size, bounds);
    let (extent, cells) = manifest(&state, id, lot, size);
    // Project A holds a real pending structure: its detour is not laid yet and
    // its retired line is still in the world.
    let handle = crate::plugin_structures::settlement_builds::prepare(
        &mut state,
        id,
        "storage".into(),
        extent,
        bounds_of(lot, size),
        cells,
    )
    .expect("A's structure prepares");
    let project = super::super::Project {
        settlement: id,
        blueprint: "storage".into(),
        level: 1,
        origin: lot.origin,
        rotation: lot.rotation,
        building: None,
        structure: handle,
        ordered: true,
        builder: None,
        order_seq: None,
    };
    state.settlements.get_mut(id).unwrap().projects = vec![project];
    let planned = plan(&state, state.settlements.get(id).unwrap(), bounds)
        .expect("the lot is admitted")
        .expect("the lot covers village work");
    let record = state.settlements.get(id).unwrap();
    let nodes = plan_walk(&state, record).expect("the mounted network reads");
    // The unbuilt detour stands for the next plan...
    for cell in &planned.detour {
        assert_eq!(
            nodes.get(&(cell.at[0], cell.at[2])),
            Some(&cell.at[1]),
            "A's pending detour cell {:?} is a walk node",
            cell.at
        );
    }
    // ...and the line A retires is not a way through any more.
    for cell in &planned.retired {
        assert!(
            !nodes.contains_key(&(cell.at[0], cell.at[2])),
            "A's retired cell {:?} is no longer a walk node",
            cell.at
        );
    }
}
