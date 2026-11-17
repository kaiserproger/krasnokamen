use super::*;
use crate::settlements::SettlementId;
use crate::settlements::allocator::tests::block;
use crate::settlements::blueprints::{self, Rotation};
use crate::settlements::construction::tests::{complete, fixture, fund, open_plan};
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
            state.world.set_block(x, FLOOR, z, state.registry.states.stone);
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
    )
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

#[test]
fn road_crossing_lot_is_admitted_with_a_detour_and_stays_connected() {
    let (mut state, id, _) = fixture();
    let (front, normal, lot, size) = plateau(&mut state, id);
    let bounds = bounds_of(lot, size);
    let road = crossing_road(&mut state, front, normal, size, bounds);
    admit(&state, id, lot, size).expect("the crossing lot is admitted");
    let plan = plan(&state, state.settlements.get(id).unwrap(), bounds)
        .expect("a cut village path is admitted")
        .expect("the lot covers village work");
    assert!(
        !plan.detour.is_empty(),
        "a path cut on both sides needs a detour"
    );
    let detour: Vec<[i32; 3]> = plan.detour.iter().map(|cell| cell.at).collect();
    let retired: Vec<[i32; 3]> = plan.retired.iter().map(|cell| cell.at).collect();
    for at in &detour {
        assert!(
            at[0] < bounds.min[0] - 1
                || at[0] > bounds.max[0] + 1
                || at[2] < bounds.min[2] - 1
                || at[2] > bounds.max[2] + 1,
            "the detour never runs over the lot or its clearance: {at:?}"
        );
    }
    for at in &retired {
        assert!(
            (bounds.min[0]..=bounds.max[0]).contains(&at[0])
                && (bounds.min[2]..=bounds.max[2]).contains(&at[2]),
            "only lot cells are retired: {at:?}"
        );
    }
    let mut cells = Vec::new();
    cells.extend(plan.detour);
    cells.extend(plan.cleared);
    cells.extend(plan.retired);
    stays_connected(&mut state, id, &cells);
    for at in &detour {
        assert_eq!(
            name(&state, state.world.block_at(at[0], at[1], at[2]).unwrap()),
            Some("dirt_path"),
            "detour cell {at:?}"
        );
    }
    for at in &road {
        let inside = (bounds.min[0]..=bounds.max[0]).contains(&at[0])
            && (bounds.min[2]..=bounds.max[2]).contains(&at[2]);
        assert_eq!(
            name(&state, state.world.block_at(at[0], at[1], at[2]).unwrap()),
            Some(if inside { "dirt" } else { "dirt_path" }),
            "village path cell {at:?}"
        );
    }
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

#[test]
fn a_player_placed_path_is_refused() {
    let (mut state, id, _) = fixture();
    let (front, normal, lot, size) = plateau(&mut state, id);
    let bounds = bounds_of(lot, size);
    let mut road = crossing_road(&mut state, front, normal, size, bounds);
    let at = road.remove(2);
    let held = state.world.block_at(at[0], at[1], at[2]).unwrap();
    state.block_log = player_history(&state, at, 0);
    assert!(state.block_log.player_touched(at));
    assert_eq!(
        plan(&state, state.settlements.get(id).unwrap(), bounds).err(),
        Some(Reason::ProtectedObstruction),
        "a player path is not the village's to re-lay"
    );
    assert_eq!(state.world.block_at(at[0], at[1], at[2]), Some(held));
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
        .flat_map(|&at| {
            crop_drops(
                &state,
                at,
                state.world.block_at(at[0], at[1], at[2]).unwrap(),
            )
            .unwrap()
        })
        .map(|stack| (stack.item_id, stack.count))
        .collect();
    assert!(
        loot
            .iter()
            .any(|&(item, count)| item == item_of(&state, "wheat") && count >= 1),
        "a mature field yields wheat: {loot:?}"
    );
    // The same plan executes: the clear stage banks the harvest, the build
    // stage retires the field to the soil it is made of.
    crate::settlements::construction::request_build(&mut state, id, "storage")
        .expect("the farmland lot starts");
    complete(&mut state, id, builder);
    for at in &crops {
        assert_ne!(
            name(
                &state,
                state.world.block_at(at[0], at[1], at[2]).unwrap()
            ),
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
