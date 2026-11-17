//! B-min: bounded solid terrain, whole-sweep lot ranking, credited cuts.
use super::site_tests::{Site, storage_lots, street_plateau};
use super::tests::{attempt, complete, fixture, fund, open_plan};
use super::*;
use crate::settlements::allocator::tests::block;

const FLOOR: i32 = 200;

/// A seven-by-seven-chunk plateau with the fixture chest kept: the street plan
/// then has more than one 64-candidate cursor batch of storage lots.
fn wide_plan(state: &mut TickState) {
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
fn restore_stock(state: &mut TickState) {
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
        borrowed_builder: None,
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
        if let Some(project) = state.settlements.get(id).unwrap().project.clone() {
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

/// The window is bounded on both sides: eight blocks of fill and the authored
/// top are the limits; a deeper support or a roof block above them is refused.
#[test]
fn the_terrain_window_is_bounded_below_the_floor_and_above_the_roof() {
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
    // A fifth block is a retaining wall the committee refuses, not a payment.
    state
        .world
        .set_block(6, FLOOR - 4, 6, state.registry.states.air);
    state
        .world
        .set_block(6, FLOOR - 5, 6, state.registry.states.stone);
    state.world.fold_overlay();
    assert_eq!(
        column(&state).err(),
        Some(site_refusals::Reason::HeightVariance),
        "a fill column deeper than four blocks is refused"
    );
    // Restore the flat lot, then bound the cut at the authored top.
    for y in FLOOR - 9..=FLOOR {
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
    request_build(&mut state, id, "storage").expect("authored bill admits the lot");
    let project = state.settlements.get(id).unwrap().project.clone().unwrap();
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

/// Full storage waits: the cut cell and its material stay until space exists.
#[test]
fn a_full_store_waits_without_editing_or_losing_the_excavated_block() {
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
    request_build(&mut state, id, "storage").expect("authored bill admits the lot");
    for _ in 0..3 {
        attempt(&mut state, id, builder);
    }
    assert_eq!(
        name_of(&state, pillar.cells[0]),
        "sand",
        "a full store must not edit the cell"
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

/// A foundation shortfall belongs to the active bill: the project waits as
/// `collecting_material` and finishes after a later delivery.
#[test]
fn a_foundation_shortfall_waits_as_collecting_material_then_finishes() {
    let (mut state, id, builder) = fixture();
    let site = only_lot(&mut state, id);
    dig(&mut state, &site, 2);
    request_build(&mut state, id, "storage")
        .expect("admission is decided on the authored bill, not the foundation");
    let project = state.settlements.get(id).unwrap().project.clone().unwrap();
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
    assert!(state.settlements.get(id).unwrap().project.is_some());
    // Deliver the missing foundation; the same order completes.
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    let free = slots.iter().position(|stack| stack.is_empty()).unwrap();
    slots[free] = mc_data::ItemStack::new(dirt, 8);
    complete(&mut state, id, builder);
    assert!(state.settlements.get(id).unwrap().project.is_none());
    let [x, z] = free_column(&state, &site, 3, 1);
    assert_eq!(
        name_of(&state, [x, FLOOR - 1, z]),
        "dirt",
        "the foundation hole is levelled with real blocks"
    );
}

/// Cut attribution is the player's only if the log says so: it refuses the
/// site at survey and again immediately before the debit.
#[test]
fn player_terrain_is_refused_at_survey_and_again_before_the_debit() {
    let (mut state, id, _) = fixture();
    let record = state.settlements.get(id).unwrap().clone();
    let origin = [5, FLOOR, 5];
    let stone = state.registry.states.stone;
    state.world.set_block(6, FLOOR + 2, 6, stone);
    state.world.fold_overlay();
    let check = |state: &TickState| {
        site_refusals::check(
            state,
            &record,
            origin,
            [3, 5, 3],
            None,
            &mut Default::default(),
            None,
        )
        .err()
    };
    assert_eq!(check(&state), None);
    state.block_log = super::super::forestry::tests::player_history(&state, [6, FLOOR + 2, 6], 0);
    assert_eq!(
        check(&state),
        Some(site_refusals::Reason::ProtectedObstruction)
    );
    // The same cell edited after admission stops the debit.
    let (mut state, id, builder) = fixture();
    let site = only_lot(&mut state, id);
    let pillar = pillar(&mut state, &site, "stone", "cobblestone", 1);
    request_build(&mut state, id, "storage").unwrap();
    state.block_log = super::super::forestry::tests::player_history(&state, pillar.cells[0], 0);
    let before = stock(&state, pillar.item);
    for _ in 0..3 {
        attempt(&mut state, id, builder);
    }
    assert_eq!(name_of(&state, pillar.cells[0]), "stone");
    assert_eq!(stock(&state, pillar.item), before);
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
    request_build(&mut state, id, "storage").unwrap();
    let project = state.settlements.get(id).unwrap().project.clone().unwrap();
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

/// Committee ruling: a ranking measured before the world moved is stale. The
/// sweep restarts instead of admitting the old cheapest lot.
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
    let sites = planner_lots(&state, id);
    let legal: Vec<_> = sites
        .iter()
        .filter(|site| lot_cost(&state, &record, blueprint, site).is_ok())
        .take(2)
        .cloned()
        .collect();
    assert_eq!(legal.len(), 2, "two legal lots");
    let (cheap, other) = (legal[0].clone(), legal[1].clone());
    let mut network = layout::Network::for_construction(&state, &record);
    let mut clearance = super::super::forestry::Clearance::default();
    let mut sweeps = site_refusals::Sweeps::default();
    let mut report = site_refusals::Report::default();
    let mut budget = u32::MAX;
    let first = site_refusals::advance(
        &state,
        &record,
        &mut network,
        blueprint,
        &floors,
        &mut clearance,
        &mut sweeps,
        0,
        &mut budget,
        &mut report,
    );
    let winner = match first {
        site_refusals::Advance::Cheapest(origin, rotation) => (origin, rotation),
        other => panic!("a complete sweep ranks the lots: {other:?}"),
    };
    // The world moves under the ranking: dear up the whole cheap lot's floor.
    let [ox, oy, oz] = cheap.lot.origin;
    for x in ox..ox + i32::from(cheap.size[0]) {
        for z in oz..oz + i32::from(cheap.size[2]) {
            for y in oy - 3..=oy {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    state.world.fold_overlay();
    let _ = other;
    // Same sweep state, current world: the stale ranking must not survive.
    let mut network = layout::Network::for_construction(&state, &record);
    let mut budget = u32::MAX;
    let second = site_refusals::advance(
        &state,
        &record,
        &mut network,
        blueprint,
        &floors,
        &mut clearance,
        &mut sweeps,
        0,
        &mut budget,
        &mut report,
    );
    if winner == (cheap.lot.origin, cheap.lot.rotation) {
        assert!(
            matches!(
                second,
                site_refusals::Advance::Incomplete(_) | site_refusals::Advance::Empty
            ),
            "the stale winner is dropped, not admitted: {second:?}"
        );
    }
}

/// Committee ruling: ties break on the lot's own block position, so a restart
/// in the middle of the sweep cannot change the winner.
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
    let mut clearance = super::super::forestry::Clearance::default();
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
            &mut clearance,
            sweeps,
            start,
            &mut budget,
            &mut report,
        ) {
            site_refusals::Advance::Cheapest(origin, rotation) => (origin, rotation),
            other => panic!("a complete sweep ranks the lots: {other:?}"),
        }
    };
    let mut straight = site_refusals::Sweeps::default();
    let direct = winner(0, &mut straight);
    // A restart walks the same candidates from a later cursor.
    let mut restarted = site_refusals::Sweeps::default();
    let after = winner(7, &mut restarted);
    assert_eq!(
        direct, after,
        "the tie winner does not depend on the cursor"
    );
}
