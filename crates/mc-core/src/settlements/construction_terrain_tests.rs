//! B-min: bounded solid terrain, whole-sweep lot ranking, credited cuts.
use super::site_tests::{Site, storage_lots, street_plateau};
use super::tests::{attempt, complete, fixture, fund, open_plan, request_started};
use super::*;
use crate::settlements::allocator::tests::block;

const FLOOR: i32 = 200;

/// A seven-by-seven-chunk plateau with the fixture chest kept: the street plan
/// then has more than one 64-candidate cursor batch of storage lots.
pub(super) fn wide_plan(state: &mut TickState) {
    let chunks = rustc_hash::FxHashMap::from_iter(
        (-3..=3).flat_map(|x| (-3..=3).map(move |z| (crate::boundary::ChunkPos { x, z }, 0))),
    );
    state.world.ensure_blocking(&chunks);
    for x in -48i32..64 {
        for z in -48i32..64 {
            for y in FLOOR - 10..=FLOOR {
                state.world.set_block(x, y, z, state.registry.states.stone);
            }
            for y in FLOOR + 1..224 {
                if (x, y, z) != (1, 201, 1) {
                    state.world.set_block(x, y, z, state.registry.states.air);
                }
            }
        }
    }
}

/// Re-places and registers the fixture's stock container.
pub(super) fn restore_stock(state: &mut TickState) {
    // Loading the extra chunks re-syncs block entities from the fresh chunks,
    // which drops the fixture's container: re-place and register it canonically.
    let chest = block(state, "minecraft:chest", &[]);
    state.world.set_block(1, 201, 1, chest);
    crate::block_entity_defaults::after_blocks(
        state,
        &[(crate::boundary::BlockPos { x: 1, y: 201, z: 1 }, chest)],
    );
    state.block_entities.insert(
        (1, 201, 1),
        crate::tick::BlockEntityState {
            slots: vec![mc_data::ItemStack::EMPTY; 27],
            furnace: None,
        },
    );
    state.world.fold_overlay();
}

fn item(state: &TickState, name: &str) -> u32 {
    state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap())
        .unwrap()
}

fn stock(state: &TickState, item: u32) -> i64 {
    state.block_entities[&(1, 201, 1)]
        .slots
        .iter()
        .filter(|stack| !stack.is_empty() && stack.item_id == item)
        .map(|stack| i64::from(stack.count))
        .sum()
}

/// What the fixture builder still carries of one item.
fn carried(state: &TickState, slot: crate::entities::EntitySlot, item: u32) -> i64 {
    state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .map(|villager| {
            villager
                .inventory
                .iter()
                .flatten()
                .filter(|stack| stack.item_id == item && stack.count > 0)
                .map(|stack| i64::from(stack.count))
                .sum()
        })
        .unwrap_or(0)
}

fn name_of(state: &TickState, at: [i32; 3]) -> &str {
    state
        .world
        .block_at(at[0], at[1], at[2])
        .and_then(|block| state.registry.blocks.by_id(mc_world::BlockStateId(block)))
        .map_or("unloaded", |definition| definition.block.id.path())
}

/// The lot an owner order takes on the flat fixture: the first free one.
fn first_lot(state: &TickState, id: SettlementId) -> Site {
    let sites = storage_lots(state, id);
    let site = sites
        .iter()
        .find(|site| !site.within(1, 1))
        .copied()
        .expect("a free storage lot");
    assert!(site_free(
        state,
        state.settlements.get(id).unwrap(),
        site.lot.origin,
        site.size
    ));
    site
}

/// One lot only: every other lot cell is solid, so the sweep has no choice and
/// the test's own terrain edit is what the plan must handle.
fn only_lot(state: &mut TickState, id: SettlementId) -> Site {
    open_plan(state);
    fund(state, "storage", 1);
    let site = first_lot(state, id);
    street_plateau(state, id, &[site]);
    site
}

/// A column whose authored blueprint leaves `FLOOR + 1 ..= FLOOR + height`
/// empty, so a test pillar there is terrain, not an authored cell. `skip`
/// takes the n-th such column, which lets a test place a pillar and dig a
/// foundation hole in two different columns of the same lot.
fn free_column(state: &TickState, site: &Site, height: i32, skip: usize) -> [i32; 2] {
    let level = blueprints::get(&state.registry, "storage")
        .unwrap()
        .level(1)
        .unwrap()
        .rotated(Rotation::from_quarter_turns(site.lot.rotation));
    let floor = site.lot.origin[1];
    let [ox, _, oz] = site.lot.origin;
    let mut free = Vec::new();
    for x in ox..ox + i32::from(site.size[0]) {
        for z in oz..oz + i32::from(site.size[2]) {
            let authored_above = level.blocks.iter().any(|(offset, _)| {
                let [ax, ay, az] = level.world(site.lot.origin, *offset);
                ax == x && az == z && ay > floor && ay <= floor + height
            });
            if !authored_above {
                free.push([x, z]);
            }
        }
    }
    free.get(skip)
        .copied()
        .unwrap_or_else(|| panic!("no authored-free column {skip} in the storage lot"))
}

/// The refusal survey's own measure of one lot, floor set included.
fn lot_cost(
    state: &TickState,
    record: &SettlementRecord,
    blueprint: &blueprints::Blueprint,
    site: &Site,
) -> Result<site_refusals::Cost, site_refusals::Reason> {
    lot_cost_at(state, record, blueprint, site.lot.origin, site.lot.rotation)
}

/// The same survey for a bare lot origin and rotation.
fn lot_cost_at(
    state: &TickState,
    record: &SettlementRecord,
    blueprint: &blueprints::Blueprint,
    origin: [i32; 3],
    rotation: u8,
) -> Result<site_refusals::Cost, site_refusals::Reason> {
    let floors = site_refusals::floor_columns(blueprint, rotation);
    site_refusals::check(
        state,
        record,
        origin,
        blueprint.footprint(Rotation::from_quarter_turns(rotation)),
        record.founding.as_ref(),
        &mut Default::default(),
        Some(&floors),
    )
}

/// A cut pillar the plan owns, and the item its cuts credit.
struct Pillar {
    cells: Vec<[i32; 3]>,
    item: u32,
}

fn pillar(state: &mut TickState, site: &Site, name: &str, drop: &str, height: i32) -> Pillar {
    let [x, z] = free_column(state, site, height, 0);
    let placed = block(state, &format!("minecraft:{name}"), &[]);
    let cells: Vec<[i32; 3]> = (1..=height)
        .map(|step| [x, site.lot.origin[1] + step, z])
        .collect();
    for at in &cells {
        state.world.set_block(at[0], at[1], at[2], placed);
    }
    state.world.fold_overlay();
    Pillar {
        cells,
        item: item(state, drop),
    }
}

/// Dig one lot column: its support drops `depth` blocks under the floor. The
/// column is a different one from the test pillar's, so the pillar never
/// becomes the column's own surface.
fn dig(state: &mut TickState, site: &Site, depth: i32) {
    let [x, z] = free_column(state, site, 3, 1);
    let floor = site.lot.origin[1];
    state
        .world
        .set_block(x, floor - depth, z, state.registry.states.stone);
    for y in floor - depth + 1..=floor {
        state.world.set_block(x, y, z, state.registry.states.air);
    }
    state.world.fold_overlay();
}

fn manifest(
    state: &TickState,
    project: &Project,
) -> (ScriptWorldBounds, Vec<executor::PlannedCell>) {
    preparation::planned_cells(
        state,
        project,
        "oak",
        &mut super::super::forestry::Clearance::default(),
    )
    .expect("the admitted lot still plans")
}

fn project_at(id: SettlementId, site: &Site) -> Project {
    Project {
        settlement: id,
        blueprint: "storage".into(),
        level: 1,
        origin: site.lot.origin,
        rotation: site.lot.rotation,
        building: None,
        structure: String::new(),
        ordered: true,
        builder: None,
        order_seq: None,
    }
}

/// Storage lots in the order the construction planner's cursor walks them.
fn planner_lots(state: &TickState, id: SettlementId) -> Vec<Site> {
    let record = state.settlements.get(id).unwrap();
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let mut network = layout::Network::for_construction(state, record);
    network
        .frontage()
        .into_iter()
        .enumerate()
        .filter_map(|(front, (at, normal))| {
            let lot = network.lot(blueprint, at, normal)?;
            Some(Site {
                front: u32::try_from(front).unwrap(),
                lot,
                size: blueprint.footprint(Rotation::from_quarter_turns(lot.rotation)),
            })
        })
        .collect()
}

/// Main's correction: only a completed sweep may admit, and then the cheapest
/// lot over the whole cursor sweep — never the first legal one in the batch.
#[test]
fn cheapest_lot_beyond_the_first_cursor_batch_wins_the_completed_sweep() {
    let (mut state, id, _) = fixture();
    wide_plan(&mut state);
    restore_stock(&mut state);
    fund(&mut state, "storage", 1);
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let images = executor::source_images(&state, id);
    assert_eq!(images.len(), 1, "the fixture stock is the only source");
    let record = state.settlements.get(id).unwrap().clone();
    // A batch-sized budget cannot finish the sweep: nothing is admitted and the
    // cursor parks inside the remaining candidates.
    let mut batch = site_refusals::Sweeps::default();
    let short = site_project(
        &state,
        &record,
        std::iter::once(blueprint),
        &mut batch,
        0,
        SITE_ATTEMPTS,
        &images,
    );
    assert!(short.site.is_none(), "a partial sweep admits nothing");
    assert!(short.running, "a partial sweep keeps searching");
    assert!(short.cursor > 0, "a partial sweep parks its cursor");
    // Dear the first three legal lots up by digging under their floors, which
    // leaves the walkable surface (the frontage walk) untouched.
    let sites = planner_lots(&state, id);
    assert!(
        sites.len() > SITE_ATTEMPTS as usize,
        "the plan needs lots beyond one batch: {}",
        sites.len()
    );
    // An interior column that is neither an authored floor cell nor authored
    // above the floor, so the dug depth is the lot's own fill.
    let dig = |site: &Site| -> [i32; 2] {
        let floors = site_refusals::floor_columns(blueprint, site.lot.rotation);
        let [ox, _, oz] = site.lot.origin;
        for dx in 1..i32::from(site.size[0]) - 1 {
            for dz in 1..i32::from(site.size[2]) - 1 {
                if !floors.contains(&[dx, dz]) {
                    return [ox + dx, oz + dz];
                }
            }
        }
        panic!("no plain floor cell");
    };
    // Every lot the first cursor batch can reach pays for a dug floor, so the
    // sweep's minimum cannot come from inside that batch.
    let legal: Vec<_> = sites
        .iter()
        .filter(|site| site.front < SITE_ATTEMPTS)
        .filter(|site| lot_cost(&state, &record, blueprint, site).is_ok())
        .map(|site| (site.lot.origin, site.lot.rotation, dig(site)))
        .collect();
    assert!(!legal.is_empty(), "the first batch holds legal lots");
    let air = state.registry.states.air;
    for (origin, _, [x, z]) in &legal {
        for y in origin[1] - 3..=origin[1] {
            state.world.set_block(*x, y, *z, air);
        }
    }
    state.world.fold_overlay();
    // The walkable surface is unchanged, so the planner walks the same lots.
    assert_eq!(planner_lots(&state, id).len(), sites.len());
    let first_legal = sites
        .iter()
        .find(|site| lot_cost(&state, &record, blueprint, site).is_ok())
        .expect("a legal lot");
    assert!(
        lot_cost(&state, &record, blueprint, first_legal)
            .expect("still legal")
            .fill
            > 0,
        "the first legal lot now pays for its dug floor"
    );
    // The completed sweep ranks the whole cursor and admits the cheapest lot.
    let mut sweeps = site_refusals::Sweeps::default();
    let pass = site_project(
        &state,
        &record,
        std::iter::once(blueprint),
        &mut sweeps,
        0,
        u32::MAX,
        &images,
    );
    let site = pass
        .site
        .expect("the completed sweep admits its cheapest lot");
    assert!(!pass.running, "the sweep completed");
    assert_eq!(pass.cursor, 0, "a completed sweep restarts its cursor");
    assert_ne!(
        (site.project.origin, site.project.rotation),
        (first_legal.lot.origin, first_legal.lot.rotation),
        "the cheapest lot wins over the first legal one"
    );
    let winner_front = sites
        .iter()
        .find(|lot| {
            lot.lot.origin == site.project.origin && lot.lot.rotation == site.project.rotation
        })
        .expect("the winner is a planner lot")
        .front;
    assert!(
        winner_front >= SITE_ATTEMPTS,
        "the winner sits beyond the first batch: {winner_front}"
    );
    let winner_cost = lot_cost_at(
        &state,
        &record,
        blueprint,
        site.project.origin,
        site.project.rotation,
    )
    .expect("the winner is legal")
    .total();
    for other in &sites {
        if let Ok(cost) = lot_cost(&state, &record, blueprint, other) {
            assert!(
                winner_cost <= cost.total(),
                "no legal lot beats the winner at {:?}",
                other.lot.origin
            );
        }
    }
    // Admission: the planner runs the same sweep and takes the same winner.
    let mut admitted = None;
    for _ in 0..64 {
        plan(&mut state, id);
        if let Some(project) = state.settlements.get(id).unwrap().projects.first().cloned() {
            admitted = Some(project);
            break;
        }
    }
    let project = admitted.expect("the completed sweep admits its cheapest lot");
    assert_eq!(
        (project.origin, project.rotation),
        (site.project.origin, site.project.rotation)
    );
}

/// Inside the territory the fill below the floor is levelled down to the edge
/// of the builder's reach; the cut stays bounded by the authored top, and a roof
/// block above it is refused.
#[test]
fn the_fill_below_the_floor_is_levelled_within_reach_and_the_roof_block_is_refused() {
    let (mut state, id, _) = fixture();
    let record = state.settlements.get(id).unwrap().clone();
    let origin = [5, FLOOR, 5];
    let size = [3, 5, 3];
    let column = |state: &TickState| {
        site_refusals::check(
            state,
            &record,
            origin,
            size,
            None,
            &mut Default::default(),
            None,
        )
    };
    assert_eq!(column(&state).map(|cost| cost.total()), Ok(0));
    // Four below the floor is the deepest foundation this slice pays for.
    for y in FLOOR - 3..=FLOOR {
        state.world.set_block(6, y, 6, state.registry.states.air);
    }
    state
        .world
        .set_block(6, FLOOR - 4, 6, state.registry.states.stone);
    state.world.fold_overlay();
    assert_eq!(
        column(&state).map(|cost| cost.fill),
        Ok(4),
        "four fill cells below the floor"
    );
    // Inside the territory a fifth block and anything down to the edge of the
    // eight-block band is filled with real material (owner 2026-11-17).
    for (depth, filled) in [(5, 5), (8, 8)] {
        for y in FLOOR - 14..=FLOOR {
            state.world.set_block(6, y, 6, state.registry.states.air);
        }
        state
            .world
            .set_block(6, FLOOR - depth, 6, state.registry.states.stone);
        state.world.fold_overlay();
        assert_eq!(
            column(&state).map(|cost| cost.fill),
            Ok(filled),
            "a fill column {depth} blocks deep is levelled inside the territory"
        );
    }
    // A support below the eight-block band is out of what a builder at the lot's
    // perimeter reaches: a named refusal, not a shaft the executor stalls on.
    for y in FLOOR - 14..=FLOOR {
        state.world.set_block(6, y, 6, state.registry.states.air);
    }
    state
        .world
        .set_block(6, FLOOR - 12, 6, state.registry.states.stone);
    state.world.fold_overlay();
    assert_eq!(
        column(&state).err(),
        Some(site_refusals::Reason::SurfaceUnavailable)
    );
    // Restore the flat lot, then bound the cut at the authored top.
    for y in FLOOR - 14..=FLOOR {
        state.world.set_block(6, y, 6, state.registry.states.air);
    }
    state
        .world
        .set_block(6, FLOOR, 6, state.registry.states.stone);
    state
        .world
        .set_block(6, FLOOR + 4, 6, state.registry.states.stone);
    state.world.fold_overlay();
    assert_eq!(column(&state).map(|cost| cost.cut), Ok(1));
    state
        .world
        .set_block(6, FLOOR + 4, 6, state.registry.states.air);
    state
        .world
        .set_block(6, FLOOR + 5, 6, state.registry.states.stone);
    state.world.fold_overlay();
    assert_eq!(
        column(&state).err(),
        Some(site_refusals::Reason::CutBeyondEnvelope),
        "a roof block above the authored top is refused, not silently buried"
    );
}

/// Liquid in the work volume and lava in the one-cell shell are named refusals.
#[test]
fn liquid_in_the_volume_and_lava_in_the_shell_refuse_the_site() {
    let (mut state, id, _) = fixture();
    let record = state.settlements.get(id).unwrap().clone();
    let origin = [5, FLOOR, 5];
    let size = [3, 5, 3];
    let refuse = |state: &TickState| {
        site_refusals::check(
            state,
            &record,
            origin,
            size,
            None,
            &mut Default::default(),
            None,
        )
        .err()
    };
    assert_eq!(refuse(&state), None);
    let water = state.registry.states.water;
    state.world.set_block(6, FLOOR + 2, 6, water);
    state.world.fold_overlay();
    assert_eq!(refuse(&state), Some(site_refusals::Reason::LiquidInWork));
    state
        .world
        .set_block(6, FLOOR + 2, 6, state.registry.states.air);
    // The shell is one cell wide, outside the lot's own footprint.
    let lava = state.registry.states.lava_levels[0];
    state.world.set_block(4, FLOOR + 1, 6, lava);
    state.world.fold_overlay();
    assert_eq!(refuse(&state), Some(site_refusals::Reason::LavaShell));
    state
        .world
        .set_block(4, FLOOR + 1, 6, state.registry.states.air);
    state.world.fold_overlay();
    assert_eq!(refuse(&state), None);
}

/// A cut's solid is credited once, and the plan's own foundation is bought
/// from real stock in the same bill.
#[test]
fn excavated_solids_are_credited_once_and_the_fill_is_debited() {
    let (mut state, id, builder) = fixture();
    let site = only_lot(&mut state, id);
    let pillar = pillar(&mut state, &site, "sand", "sand", 2);
    dig(&mut state, &site, 2);
    let project = project_at(id, &site);
    let (_, cells) = manifest(&state, &project);
    let [dx, dz] = free_column(&state, &site, 3, 1);
    assert_eq!(
        super::surface_y_for_lot(
            &state,
            site.lot.origin[1],
            dx,
            dz,
            None,
            &mut Default::default()
        ),
        Some(site.lot.origin[1] - 2),
        "the dug column's own support is the premise of this test"
    );
    let cuts = cells
        .iter()
        .filter(|cell| pillar.cells.contains(&cell.at))
        .count();
    assert_eq!(cuts, pillar.cells.len(), "every pillar cell is a cut");
    let fills = cells
        .iter()
        .filter(|cell| cell.resource.as_deref() == Some("minecraft:dirt"))
        .count();
    assert!(fills > 0, "the dug column needs foundation dirt");
    let dirt_item = item(&state, "dirt");
    let (sand, dirt) = (stock(&state, pillar.item), stock(&state, dirt_item));
    // The foundation is bought from real stock: deliver it before admission.
    {
        let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
        let free = slots
            .iter()
            .position(|stack| stack.is_empty())
            .expect("the bill leaves a free slot");
        slots[free] = mc_data::ItemStack::new(dirt_item, 8);
    }
    let dirt = dirt + 8;
    request_started(&mut state, id, "storage");
    let project = state
        .settlements
        .get(id)
        .unwrap()
        .projects
        .first()
        .cloned()
        .unwrap();
    assert_eq!(project.origin, site.lot.origin);
    complete(&mut state, id, builder);
    assert_eq!(
        stock(&state, pillar.item),
        sand + cuts as i64,
        "excavated solids credited exactly once"
    );
    // Conservation: what storage and the builder's hands hold together equals
    // the stock after delivery minus what the plan placed.
    assert_eq!(
        stock(&state, dirt_item) + carried(&state, builder, dirt_item),
        dirt - fills as i64,
        "the fill is bought from storage, never conjured"
    );
    for at in &pillar.cells {
        assert_ne!(name_of(&state, *at), "sand", "no sand survives at {at:?}");
    }
}

/// Full storage and carry wait: the cut and its material stay until space exists.
#[test]
fn a_full_store_and_carry_wait_without_editing_or_losing_the_excavated_block() {
    let (mut state, id, builder) = fixture();
    let site = only_lot(&mut state, id);
    let pillar = pillar(&mut state, &site, "sand", "sand", 1);
    let sticks = item(&state, "stick");
    {
        let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
        let free = slots
            .iter()
            .position(|stack| stack.is_empty())
            .expect("the bill leaves room");
        for slot in &mut slots[free..] {
            *slot = mc_data::ItemStack::new(sticks, 64);
        }
    }
    request_started(&mut state, id, "storage");
    for slot in &mut state
        .entities
        .row_mut(builder)
        .villager
        .as_mut()
        .unwrap()
        .inventory
    {
        *slot = Some(mc_data::ItemStack::new(sticks, 64));
    }
    for _ in 0..3 {
        attempt(&mut state, id, builder);
    }
    assert_eq!(
        name_of(&state, pillar.cells[0]),
        "sand",
        "full storage and carry must not edit the cell"
    );
    assert_eq!(stock(&state, pillar.item), 0, "nothing may be lost either");
    // One free slot is enough for the credit.
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    let filled = slots
        .iter()
        .rposition(|stack| !stack.is_empty())
        .expect("filled store");
    slots[filled] = mc_data::ItemStack::EMPTY;
    for _ in 0..3 {
        attempt(&mut state, id, builder);
    }
    assert_eq!(stock(&state, pillar.item), pillar.cells.len() as i64);
    for at in &pillar.cells {
        assert_ne!(
            name_of(&state, *at),
            "sand",
            "the cut applies once space exists"
        );
    }
}

/// The deepest fill a builder at the lot's perimeter reaches (the band's eight
/// blocks) is paid from real stock and executed to completion.
#[test]
fn a_fill_at_the_edge_of_reach_is_paid_from_storage_and_completed() {
    let (mut state, id, builder) = fixture();
    let site = only_lot(&mut state, id);
    dig(&mut state, &site, 8);
    let [x, z] = free_column(&state, &site, 3, 1);
    let floor = site.lot.origin[1];
    let dirt_item = item(&state, "dirt");
    {
        let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
        let free = slots
            .iter()
            .position(|stack| stack.is_empty())
            .expect("the bill leaves a free slot");
        slots[free] = mc_data::ItemStack::new(dirt_item, 16);
    }
    let dirt = stock(&state, dirt_item);
    request_started(&mut state, id, "storage");
    assert_eq!(
        state.settlements.get(id).unwrap().projects[0].origin,
        site.lot.origin
    );
    complete(&mut state, id, builder);
    assert!(state.settlements.get(id).unwrap().projects.is_empty());
    for y in floor - 7..=floor {
        assert_ne!(
            name_of(&state, [x, y, z]),
            "air",
            "the eight-deep shaft is filled at y={y}"
        );
    }
    assert_eq!(
        stock(&state, dirt_item) + carried(&state, builder, dirt_item),
        dirt - 8,
        "eight real dirt blocks, no more"
    );
}

/// A foundation shortfall belongs to the active bill: the project waits as
/// `collecting_material` and finishes after a later delivery.
#[test]
fn a_foundation_shortfall_waits_as_collecting_material_then_finishes() {
    let (mut state, id, builder) = fixture();
    let site = only_lot(&mut state, id);
    dig(&mut state, &site, 2);
    // Admission is decided on the authored bill, not the foundation.
    request_started(&mut state, id, "storage");
    let project = state
        .settlements
        .get(id)
        .unwrap()
        .projects
        .first()
        .cloned()
        .unwrap();
    let next = executor::next_cell(&state, id, &project.structure)
        .unwrap()
        .expect("a planned cell");
    for _ in 0..4 {
        attempt(&mut state, id, builder);
    }
    let (dirt, _) = executor::next_material(&state, id, &project.structure)
        .unwrap()
        .expect("the missing foundation is the bill's head");
    assert_eq!(
        state.registry.items.name_of(dirt).unwrap().as_str(),
        "minecraft:dirt"
    );
    assert_eq!(
        crate::villager::building::wait_reason(&state, builder, &project, next),
        "collecting_material"
    );
    assert!(!state.settlements.get(id).unwrap().projects.is_empty());
    // Deliver the missing foundation; the same order completes.
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    let free = slots.iter().position(|stack| stack.is_empty()).unwrap();
    slots[free] = mc_data::ItemStack::new(dirt, 8);
    complete(&mut state, id, builder);
    assert!(state.settlements.get(id).unwrap().projects.is_empty());
    let [x, z] = free_column(&state, &site, 3, 1);
    assert_eq!(
        name_of(&state, [x, FLOOR - 1, z]),
        "dirt",
        "the foundation hole is levelled with real blocks"
    );
}

/// Move the settlement's territory far away and survey the same lot as a
/// founding site: a founding site is outside the parent's territory (owner
/// D27.4). The state's own record moves, since the clearance reads it.
fn outside_territory(
    state: &mut TickState,
    id: SettlementId,
    cell: [i32; 3],
) -> FoundingExpedition {
    let record = state.settlements.get_mut(id).unwrap();
    assert!(record.contains(cell));
    record.anchor = [1_000, FLOOR, 1_000];
    record.radius = 8;
    assert!(!record.contains(cell));
    FoundingExpedition {
        destination: [0, FLOOR - 2, 0],
        household: String::new(),
    }
}

fn survey(
    state: &TickState,
    id: SettlementId,
    expedition: Option<&FoundingExpedition>,
) -> Option<site_refusals::Reason> {
    site_refusals::check(
        state,
        state.settlements.get(id).unwrap(),
        [5, FLOOR, 5],
        [3, 5, 3],
        expedition,
        &mut Default::default(),
        None,
    )
    .err()
}

/// Owner D27.4: inside the settlement's territory a player's terrain edit is
/// the village's property and no longer refuses the lot; the same edit on a
/// founding site outside the territory still does.
#[test]
fn player_terrain_is_village_property_inside_the_territory_only() {
    let (mut state, id, _) = fixture();
    let cell = [6, FLOOR + 2, 6];
    let stone = state.registry.states.stone;
    state.world.set_block(cell[0], cell[1], cell[2], stone);
    state.world.fold_overlay();
    assert_eq!(survey(&state, id, None), None);
    state.block_log = super::super::forestry::tests::player_history(&state, cell, 0);
    assert!(state.block_log.player_touched(cell));
    assert_eq!(survey(&state, id, None), None, "inside: village property");
    let expedition = outside_territory(&mut state, id, cell);
    assert_eq!(
        survey(&state, id, Some(&expedition)),
        Some(site_refusals::Reason::ProtectedObstruction),
        "outside: the player's edit stays protected"
    );
    state.block_log = Default::default();
    assert_eq!(
        survey(&state, id, Some(&expedition)),
        None,
        "the founding site itself is admissible"
    );
}

/// Outside the territory a cut cell a player edited after admission stops the
/// debit (owner D27.4 keeps the veto there).
#[test]
fn player_terrain_edited_after_admission_outside_the_territory_stops_the_debit() {
    let (mut state, id, builder) = fixture();
    let site = only_lot(&mut state, id);
    let pillar = pillar(&mut state, &site, "stone", "cobblestone", 1);
    request_started(&mut state, id, "storage");
    let project = state.settlements.get(id).unwrap().projects[0].clone();
    let next = executor::next_cell(&state, id, &project.structure)
        .unwrap()
        .unwrap();
    assert_eq!(next, pillar.cells[0], "the pillar is the only clear cell");
    state.block_log = super::super::forestry::tests::player_history(&state, next, 0);
    let record = state.settlements.get_mut(id).unwrap();
    record.anchor = [next[0] + 1_000, next[1], next[2]];
    record.radius = 8;
    let before = stock(&state, pillar.item);
    super::tests::bind_project(&mut state, id, &project.structure, builder);
    let uuid = state.entities.row(builder).uuid;
    assert_eq!(
        executor::advance(&mut state, id, &project.structure, uuid),
        Err(mc_script::ScriptOperationFailure::Blocked)
    );
    assert_eq!(name_of(&state, next), "stone");
    assert_eq!(stock(&state, pillar.item), before);
}

#[test]
fn player_terrain_edits_inside_the_territory_are_cut_and_credited_at_debit() {
    // Inside village property, a history-only edit after admission is cleared
    // and credits the certified stone once.
    let (mut state, id, builder) = fixture();
    let site = only_lot(&mut state, id);
    let pillar = pillar(&mut state, &site, "stone", "cobblestone", 1);
    request_started(&mut state, id, "storage");
    state.block_log = super::super::forestry::tests::player_history(&state, pillar.cells[0], 0);
    let before = stock(&state, pillar.item);
    let uuid = state.entities.row(builder).uuid;
    let handle = state.settlements.get(id).unwrap().projects[0]
        .structure
        .clone();
    bind_builder(
        state.settlements.get_mut(id).unwrap(),
        &handle,
        Some((uuid, None)),
    );
    executor::advance(&mut state, id, &handle, uuid).unwrap();
    assert_eq!(name_of(&state, pillar.cells[0]), "air");
    assert_eq!(stock(&state, pillar.item), before + 1);
}

/// Lava arriving beside a planned cut blocks the write in the same preflight.
#[test]
fn lava_arriving_before_the_cut_blocks_the_write() {
    let (mut state, id, builder) = fixture();
    let site = only_lot(&mut state, id);
    let pillar = pillar(&mut state, &site, "sand", "sand", 1);
    request_build(&mut state, id, "storage").unwrap();
    let below = [
        pillar.cells[0][0],
        pillar.cells[0][1] - 1,
        pillar.cells[0][2],
    ];
    let lava = state.registry.states.lava_levels[0];
    state.world.set_block(below[0], below[1], below[2], lava);
    state.world.fold_overlay();
    for _ in 0..3 {
        attempt(&mut state, id, builder);
    }
    assert_eq!(name_of(&state, pillar.cells[0]), "sand");
    assert_eq!(stock(&state, pillar.item), 0);
}

/// Cuts run top-down, so no surviving terrain loses its support and the
/// builder never drops more than one block per work unit.
#[test]
fn terrain_is_cut_top_down_and_no_step_drops_more_than_one_block() {
    let (mut state, id, builder) = fixture();
    let site = only_lot(&mut state, id);
    let pillar = pillar(&mut state, &site, "stone", "cobblestone", 3);
    request_started(&mut state, id, "storage");
    let project = state
        .settlements
        .get(id)
        .unwrap()
        .projects
        .first()
        .cloned()
        .unwrap();
    let (_, cells) = manifest(&state, &project);
    let [x, z] = [pillar.cells[0][0], pillar.cells[0][2]];
    let ordered: Vec<i32> = cells
        .iter()
        .filter(|cell| {
            cell.at[0] == x && cell.at[2] == z && cell.state == state.registry.states.air
        })
        .map(|cell| cell.at[1])
        .collect();
    assert_eq!(
        ordered,
        vec![FLOOR + 3, FLOOR + 2, FLOOR + 1],
        "the clear manifest cuts this column from the top"
    );
    let mut previous = state.entities.pos(builder)[1];
    for _ in 0..8 {
        if !executor::clearing(&state, id, &project.structure) {
            break;
        }
        attempt(&mut state, id, builder);
        let y = state.entities.pos(builder)[1];
        assert!(
            previous - y <= 1.000_001,
            "a cut step drops the builder {} blocks: {previous} -> {y}",
            previous - y
        );
        previous = y;
        for at in &pillar.cells {
            if name_of(&state, *at) == "stone" {
                continue;
            }
            for above in at[1] + 1..=FLOOR + 3 {
                assert_ne!(
                    name_of(&state, [x, above, z]),
                    "stone",
                    "terrain above the removed cell {at:?} lost its support"
                );
            }
        }
    }
    assert!(!executor::clearing(&state, id, &project.structure));
    for at in &pillar.cells {
        assert_ne!(name_of(&state, *at), "stone", "{at:?} stays cut");
    }
}

/// Committee ruling: the authored-floor exemption waives only the fill price.
/// A fluid on the written floor cell is still refused, overhang or not.
#[test]
fn an_authored_floor_still_refuses_fluid_on_its_own_cell() {
    let (mut state, id, _) = fixture();
    let site = only_lot(&mut state, id);
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let record = state.settlements.get(id).unwrap().clone();
    let floors = site_refusals::floor_columns(blueprint, site.lot.rotation);
    let [ox, oy, oz] = site.lot.origin;
    let relative = *floors.iter().next().expect("authored floor column");
    let (x, z) = (ox + relative[0], oz + relative[1]);
    assert!(
        lot_cost(&state, &record, blueprint, &site).is_ok(),
        "clean lot"
    );
    // A stone overhang above the floor plus water in the written floor cell.
    state
        .world
        .set_block(x, oy + 2, z, state.registry.states.stone);
    let water = block(&state, "minecraft:water", &[("level", "0")]);
    state.world.set_block(x, oy, z, water);
    state.world.fold_overlay();
    assert_eq!(
        lot_cost(&state, &record, blueprint, &site).err(),
        Some(site_refusals::Reason::LiquidInWork),
        "the floor cell the plan rewrites must not hold fluid"
    );
}

/// Committee ruling: a ranking measured before the world moved is stale. Lot A
/// leads the sweep after the first pass; between passes its foundation is dug
/// out, so lot B, examined later in the same sweep, is now cheaper. The sweep
/// must admit B, never the stale A.
#[test]
fn a_stale_ranking_restarts_instead_of_admitting_the_old_winner() {
    let (mut state, id, _) = fixture();
    wide_plan(&mut state);
    restore_stock(&mut state);
    fund(&mut state, "storage", 1);
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let record = state.settlements.get(id).unwrap().clone();
    let floors: [std::collections::BTreeSet<[i32; 2]>; 4] = std::array::from_fn(|turns| {
        site_refusals::floor_columns(blueprint, u8::try_from(turns).unwrap())
    });
    let mut sweeps = site_refusals::Sweeps::default();
    let mut report = site_refusals::Report::default();
    // A: the flat plan's winner. The first pass examines A alone.
    let a = super::site_tests::ranking_lot(&state, id, "storage");
    assert_eq!(
        lot_cost(&state, &record, blueprint, &a).map(|cost| cost.total()),
        Ok(0)
    );
    let mut network = layout::Network::for_construction(&state, &record);
    let mut budget = 1;
    let first = site_refusals::advance(
        &state,
        &record,
        &mut network,
        blueprint,
        &floors,
        &mut site_refusals::Survey::default(),
        &mut sweeps,
        a.front,
        &mut budget,
        &mut report,
    );
    assert!(
        matches!(first, site_refusals::Advance::Incomplete(_)),
        "one candidate cannot finish the sweep: {first:?}"
    );
    // Between passes the world moves: one plain interior column of A loses
    // three blocks of its support, so A now pays for foundation fill.
    let plain = site_refusals::floor_columns(blueprint, a.lot.rotation);
    let [x, z] = (1..i32::from(a.size[0]) - 1)
        .flat_map(|dx| (1..i32::from(a.size[2]) - 1).map(move |dz| [dx, dz]))
        .find(|column| !plain.contains(column))
        .map(|[dx, dz]| [a.lot.origin[0] + dx, a.lot.origin[2] + dz])
        .expect("a plain interior floor column");
    for y in a.lot.origin[1] - 2..=a.lot.origin[1] {
        state.world.set_block(x, y, z, state.registry.states.air);
    }
    state.world.fold_overlay();
    let stale = lot_cost(&state, &record, blueprint, &a)
        .expect("A stays legal")
        .total();
    assert!(stale > 0, "A now costs foundation fill");
    let b = super::site_tests::ranking_lot(&state, id, "storage");
    assert_ne!(
        (b.lot.origin, b.lot.rotation),
        (a.lot.origin, a.lot.rotation)
    );
    assert!(
        lot_cost(&state, &record, blueprint, &b).unwrap().total() < stale,
        "B is cheaper than A on the current world"
    );
    // The remaining passes finish the sweep over the moved world.
    let mut winner = None;
    for _ in 0..64 {
        let mut network = layout::Network::for_construction(&state, &record);
        let mut budget = SITE_ATTEMPTS;
        match site_refusals::advance(
            &state,
            &record,
            &mut network,
            blueprint,
            &floors,
            &mut site_refusals::Survey::default(),
            &mut sweeps,
            0,
            &mut budget,
            &mut report,
        ) {
            site_refusals::Advance::Incomplete(_) => {}
            site_refusals::Advance::Empty => panic!("the plan keeps legal lots"),
            site_refusals::Advance::Cheapest(origin, rotation) => {
                winner = Some((origin, rotation));
                break;
            }
        }
    }
    assert_eq!(
        winner,
        Some((b.lot.origin, b.lot.rotation)),
        "the stale ranking of A is not admitted; the cheaper B wins"
    );
}

/// Committee ruling: the sweep scratch is bound to the ordered frontage list,
/// not to its length. A plaza hole reorders the walk under a running sweep
/// without changing its length; the sweep restarts, so the winner the old
/// cursor would now skip is still found.
#[test]
fn a_reordered_walk_of_the_same_length_restarts_the_sweep() {
    let (mut state, id, _) = fixture();
    wide_plan(&mut state);
    restore_stock(&mut state);
    fund(&mut state, "storage", 1);
    // The winner must lie late in the walk, where a plaza hole can move its
    // frontage earlier: keep the westmost legal lot, the one this walk was
    // authored around, and refuse every other lot.
    let target = {
        let record = state.settlements.get(id).unwrap();
        let blueprint = blueprints::get(&state.registry, "storage").unwrap();
        planner_lots(&state, id)
            .into_iter()
            .filter(|site| lot_cost(&state, record, blueprint, site).is_ok())
            .min_by_key(|site| (site.lot.origin, site.lot.rotation))
            .expect("a legal storage lot")
    };
    super::site_tests::pin_lot(&mut state, id, "storage", &target);
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let record = state.settlements.get(id).unwrap().clone();
    let floors: [std::collections::BTreeSet<[i32; 2]>; 4] = std::array::from_fn(|turns| {
        site_refusals::floor_columns(blueprint, u8::try_from(turns).unwrap())
    });
    let winner = super::site_tests::ranking_lot(&state, id, "storage");
    let fronts = |state: &TickState| layout::Network::for_construction(state, &record).frontage();
    let before = fronts(&state);
    let front = before[winner.front as usize];
    // A hole below the walkable band takes one plaza column out of the walk.
    let (air, stone) = (state.registry.states.air, state.registry.states.stone);
    let hole = |world: &mut crate::world::ChunkWorld, [x, z]: [i32; 2], block: u32| {
        for y in FLOOR - 9..=FLOOR {
            world.set_block(x, y, z, block);
        }
        world.fold_overlay();
    };
    let anchor = record.anchor;
    let mut reorder = None;
    'search: for dx in -8..=8 {
        for dz in -8..=8 {
            let column = [anchor[0] + dx, anchor[2] + dz];
            if layout::cell(anchor, column[0], column[1]) != layout::Cell::Plaza {
                continue;
            }
            hole(&mut state.world, column, air);
            let after = fronts(&state);
            hole(&mut state.world, column, stone);
            if after.len() == before.len()
                && after
                    .iter()
                    .position(|candidate| *candidate == front)
                    .is_some_and(|index| index < winner.front as usize)
            {
                reorder = Some(column);
                break 'search;
            }
        }
    }
    let column = reorder.expect("a plaza hole moves the winner's frontage earlier");
    assert_eq!(fronts(&state), before, "the plaza is whole again");
    // The first pass examines every frontage before the winner's.
    let mut sweeps = site_refusals::Sweeps::default();
    let mut report = site_refusals::Report::default();
    let mut network = layout::Network::for_construction(&state, &record);
    let mut budget = winner.front;
    let first = site_refusals::advance(
        &state,
        &record,
        &mut network,
        blueprint,
        &floors,
        &mut site_refusals::Survey::default(),
        &mut sweeps,
        0,
        &mut budget,
        &mut report,
    );
    assert!(matches!(first, site_refusals::Advance::Incomplete(_)));
    // The walk is reordered, same length; the old cursor would skip the winner.
    hole(&mut state.world, column, air);
    let mut admitted = None;
    for _ in 0..64 {
        let mut network = layout::Network::for_construction(&state, &record);
        let mut budget = u32::MAX;
        match site_refusals::advance(
            &state,
            &record,
            &mut network,
            blueprint,
            &floors,
            &mut site_refusals::Survey::default(),
            &mut sweeps,
            0,
            &mut budget,
            &mut report,
        ) {
            site_refusals::Advance::Incomplete(_) => {}
            site_refusals::Advance::Empty => panic!("the plan keeps legal lots"),
            site_refusals::Advance::Cheapest(origin, rotation) => {
                admitted = Some((origin, rotation));
                break;
            }
        }
    }
    assert_eq!(
        admitted,
        Some((winner.lot.origin, winner.lot.rotation)),
        "the reordered walk restarted the sweep and still found the winner"
    );
}

/// Committee ruling: ties break on the lot alone (its distance to the anchor,
/// then its block position), so a restart in the middle of the sweep cannot
/// change the winner.
#[test]
fn a_restart_keeps_the_tie_winner() {
    let (mut state, id, _) = fixture();
    wide_plan(&mut state);
    restore_stock(&mut state);
    fund(&mut state, "storage", 1);
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let record = state.settlements.get(id).unwrap().clone();
    let floors: [std::collections::BTreeSet<[i32; 2]>; 4] = std::array::from_fn(|turns| {
        site_refusals::floor_columns(blueprint, u8::try_from(turns).unwrap())
    });
    let mut report = site_refusals::Report::default();
    let mut winner = |start: u32, sweeps: &mut site_refusals::Sweeps| {
        let mut network = layout::Network::for_construction(&state, &record);
        let mut budget = u32::MAX;
        match site_refusals::advance(
            &state,
            &record,
            &mut network,
            blueprint,
            &floors,
            &mut site_refusals::Survey::default(),
            sweeps,
            start,
            &mut budget,
            &mut report,
        ) {
            site_refusals::Advance::Cheapest(origin, rotation) => (origin, rotation),
            other => panic!("a complete sweep ranks the lots: {other:?}"),
        }
    };
    // Every legal lot of the flat plan costs the same. A restart resumes from
    // the persisted cursor, here just past the first legal lot in walking
    // order: cursor order alone would then crown a different tie.
    let legal: Vec<_> = planner_lots(&state, id)
        .into_iter()
        .filter_map(|site| {
            let cost = lot_cost(&state, &record, blueprint, &site).ok()?.total();
            Some((site.front, cost))
        })
        .take(2)
        .collect();
    assert!(
        legal.len() == 2 && legal[0].1 == legal[1].1,
        "two equal-cost lots: {legal:?}"
    );
    let mut straight = site_refusals::Sweeps::default();
    let direct = winner(0, &mut straight);
    let mut restarted = site_refusals::Sweeps::default();
    let after = winner(legal[0].0 + 1, &mut restarted);
    assert_eq!(
        direct, after,
        "the tie winner does not depend on the cursor"
    );
}

/// Main's ruling (2026-11-18): on flat ground every legal lot costs the same,
/// and the tie goes to the lot nearest the settlement's anchor. Block position
/// alone would site the first storage at the west edge of the loaded plan.
#[test]
fn a_flat_tie_goes_to_the_lot_nearest_the_anchor() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    let record = state.settlements.get(id).unwrap().clone();
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let near = |origin: [i32; 3]| -> i64 {
        (0..3)
            .map(|axis| (i64::from(origin[axis]) - i64::from(record.anchor[axis])).pow(2))
            .sum()
    };
    let legal: Vec<(u32, [i32; 3], u8)> = planner_lots(&state, id)
        .into_iter()
        .filter_map(|site| {
            let cost = lot_cost(&state, &record, blueprint, &site).ok()?.total();
            Some((cost, site.lot.origin, site.lot.rotation))
        })
        .collect();
    assert!(
        legal.len() > 1 && legal.iter().all(|(cost, ..)| *cost == legal[0].0),
        "flat ground: every legal lot costs the same: {legal:?}"
    );
    let westmost = legal
        .iter()
        .map(|&(_, origin, rotation)| (origin, rotation))
        .min()
        .unwrap();
    let (distance, origin, rotation) = legal
        .iter()
        .map(|&(_, origin, rotation)| (near(origin), origin, rotation))
        .min()
        .unwrap();
    assert!(
        distance < near(westmost.0),
        "premise: the westmost tie {westmost:?} is not the nearest {origin:?}"
    );
    request_started(&mut state, id, "storage");
    let project = state.settlements.get(id).unwrap().projects.first().unwrap();
    assert_eq!(
        (project.origin, project.rotation),
        (origin, rotation),
        "the tie goes to the lot nearest the anchor {:?}, not to {westmost:?}",
        record.anchor
    );
}

/// The credited cut survives the persistence boundary: the storage stack, the
/// changed block and the construction cursor all come back through the
/// settlement record images the journal writes.
#[test]
fn a_cut_credit_block_and_cursor_survive_the_record_round_trip() {
    let (mut state, id, builder) = fixture();
    let site = only_lot(&mut state, id);
    let pillar = pillar(&mut state, &site, "sand", "sand", 1);
    let cuts = pillar.cells.len();
    let sand = stock(&state, pillar.item);
    request_started(&mut state, id, "storage");
    complete(&mut state, id, builder);
    assert_eq!(stock(&state, pillar.item), sand + cuts as i64);
    let credited = stock(&state, pillar.item);
    let cursor = state.settlements.get(id).unwrap().construction_cursor;
    let cut_at = pillar.cells[0];
    let after_cut = name_of(&state, cut_at).to_string();
    assert_ne!(after_cut, "sand", "the column is really cut");
    let slots = state.block_entities[&(1, 201, 1)].slots.clone();
    // Every settlement row the journal writes goes out and comes back.
    let images = state.settlements.take_images();
    assert!(!images.is_empty(), "the settlement row is imaged");
    for image in images {
        state.settlements.apply_image(image);
    }
    assert_eq!(
        state.settlements.get(id).unwrap().construction_cursor,
        cursor,
        "the cursor survives the record round trip"
    );
    assert_eq!(stock(&state, pillar.item), credited, "the credit survives");
    assert_eq!(
        &state.block_entities[&(1, 201, 1)].slots,
        &slots,
        "the storage slots survive"
    );
    assert_eq!(name_of(&state, cut_at), after_cut, "the cut block survives");
}

/// Committee item 6: one credited cut survives a real journal reopen. The
/// native checkpoint holds the admitted project before the cut; only the
/// published WAL frames of the following real ticks carry the cut, and
/// production recovery must replay the storage credit, the cleared cell and
/// the executor's cursor together.
#[test]
fn a_credited_cut_survives_a_journal_reopen() {
    use crate::dimensions::Dimension;
    use crate::journal::JournalEntry;
    let (mut state, id, builder) = fixture();
    let site = only_lot(&mut state, id);
    // `only_lot` walls every other lot column, the fixture's spawn included:
    // the builder starts on the avenue beside the bell instead. A player near
    // the bell keeps the loaded plan simulated, so the builder walks to the
    // lot through real ticks, never by teleport.
    state.entities.set_pos(builder, [8.5, 201.0, 8.5]);
    for x in -1..=1 {
        for z in -1..=1 {
            state.tickets.insert(
                format!("terrain-wal-{x}-{z}"),
                (crate::boundary::ChunkPos { x, z }, 0),
            );
        }
    }
    state.refresh_active_chunks(0);
    let pillar = pillar(&mut state, &site, "sand", "sand", 1);
    let cut_at = pillar.cells[0];
    request_started(&mut state, id, "storage");
    let structure = state
        .settlements
        .get(id)
        .unwrap()
        .projects
        .first()
        .unwrap()
        .structure
        .clone();
    // One frozen tick publishes the admission; the checkpoint then holds the
    // world before any cut, so only the WAL tail can carry it.
    state.tick_control.set_frozen(true);
    let (baseline, _, _) = crate::tick::run_tick(&mut state, &[]);
    crate::core::journal_entries_for_tick(&mut state, &baseline).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let config = crate::CoreConfig::new(directory.path().to_path_buf(), 42);
    super::founding::tests::checkpoint(&mut state, &config);
    state.tick_control.set_frozen(false);
    let checkpointed = state.tick;
    let sand = stock(&state, pillar.item);
    assert_eq!(name_of(&state, cut_at), "sand");
    let mut writer = crate::journal::JournalWriter::open(directory.path(), 1).unwrap();
    let mut cut_frame = None;
    for _ in 0..6000 {
        let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
        let entries = crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
        writer.append(output.tick, output.tick, &entries).unwrap();
        if name_of(&state, cut_at) != "sand" {
            cut_frame = Some(entries);
            break;
        }
    }
    writer.flush().unwrap();
    drop(writer);
    let cut_frame = cut_frame.expect("the builder cuts the pillar through real ticks");
    assert!(
        state.tick > checkpointed,
        "the cut lies after the checkpoint"
    );
    assert_eq!(
        stock(&state, pillar.item),
        sand + 1,
        "the cut credits its own drop"
    );
    let cleared = state
        .world
        .block_at(cut_at[0], cut_at[1], cut_at[2])
        .unwrap();
    assert!(
        cut_frame.iter().any(|entry| matches!(
            entry,
            JournalEntry::Block { pos, state } if *pos == cut_at && *state == cleared
        )),
        "the cleared cell rides the published cut frame"
    );
    assert!(
        cut_frame.iter().any(|entry| matches!(
            entry,
            JournalEntry::BlockEntity { pos, .. } if *pos == [1, 201, 1]
        )),
        "the credited storage rides the same frame"
    );
    let ledger = |state: &TickState| {
        state
            .plugins
            .get_domain("@native-structures", "ledger")
            .unwrap()["structures"][&structure]
            .clone()
    };
    let live_ledger = ledger(&state);
    let live_next = executor::next_cell(&state, id, &structure).unwrap();
    let live_cursor = state.settlements.get(id).unwrap().construction_cursor;
    let live_slots = state.block_entities[&(1, 201, 1)].slots.clone();
    drop(state);

    let mut restored = crate::core::recover_dimension_state(&config, Dimension::Overworld).unwrap();
    let wanted = [[1, 1], [cut_at[0], cut_at[2]]]
        .into_iter()
        .map(|[x, z]| {
            (
                crate::boundary::ChunkPos {
                    x: x.div_euclid(16),
                    z: z.div_euclid(16),
                },
                0,
            )
        })
        .collect();
    restored.world.ensure_blocking(&wanted);
    restored.world.fold_overlay();
    assert_eq!(
        restored.world.block_at(cut_at[0], cut_at[1], cut_at[2]),
        Some(cleared),
        "the cut cell stays cleared after the reopen"
    );
    assert_eq!(
        stock(&restored, pillar.item),
        sand + 1,
        "the storage credit survives the reopen"
    );
    assert_eq!(restored.block_entities[&(1, 201, 1)].slots, live_slots);
    assert_eq!(
        ledger(&restored),
        live_ledger,
        "the executor's cursor and receipts survive the reopen"
    );
    assert_eq!(
        executor::next_cell(&restored, id, &structure).unwrap(),
        live_next,
        "the reopened build resumes after the cut, not at it"
    );
    assert_eq!(
        restored.settlements.get(id).unwrap().construction_cursor,
        live_cursor
    );
    crate::plugin_structures::validate(&restored).expect("the reopened ledger validates");
}
