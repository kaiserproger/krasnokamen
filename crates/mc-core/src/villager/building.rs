//! Builder navigation and physical hauling. The shared structure executor is
//! the sole placement/progress authority; resident carry is real native stock.
use super::{Activity, VillagerProfession, VillagerState};
use crate::boundary::{BlockPos, ChangeSet};
use crate::entities::EntitySlot;
use crate::plugin_structures::settlement_builds as executor;
use crate::settlements::{SettlementId, blueprints, construction, layout};
use crate::tick::TickState;

/// Perimeter goals sit this many cells outside the future footprint. The
/// brain accepts arrival within 0.3 of a cell centre, and the body half-width
/// is 0.3, so the adjacent cell (centre 0.5 from the edge) can leave the body
/// straddling the footprint and the builder would never place.
const MARGIN: i32 = 2;
/// Placement reach from the builder's feet to the next cell.
const REACH: f64 = 12.0;
/// Perimeter stands the router is asked to walk to before the nearest one wins.
const STAND_ROUTES: usize = 12;
/// Route nodes the real router may expand to confirm a perimeter goal.
const ROUTE_NODES: usize = 512;
/// Route nodes a committed goal is re-confirmed with; a truncated walk is a
/// goal the builder cannot reach.
const WALK_NODES: usize = 4096;
/// Felling and clearing reach from the builder's feet to the cell's footing.
pub(super) const PREPARATION_REACH: f64 = 4.0;

/// Containers are approached; a perimeter cell must actually be reached.
pub(crate) struct WorkTarget {
    pub(crate) pos: BlockPos,
    pub(crate) exact: bool,
}

#[cfg(test)]
thread_local! {
    /// Construction admissions this thread ran. The four-tick clock is a
    /// contract of `construction::planner_tick`, so a cadence test counts the
    /// calls instead of inferring them from placed cells.
    pub(crate) static ADMISSIONS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn block_pos([x, y, z]: [i32; 3]) -> BlockPos {
    BlockPos { x, y, z }
}

pub(crate) fn working(state: &TickState, v: &VillagerState) -> bool {
    v.data.profession == VillagerProfession::Builder
        && v.age >= 0
        && !v.sleeping
        && v.conversion_time.is_none()
        && v.activity == Activity::Work
        && (2_000..9_000).contains(&state.day_time.rem_euclid(24_000))
        && crate::command_rules::boolean(state, crate::command_rules::BoolRule::MOB_GRIEFING)
}

fn carried(v: &VillagerState, item: u32, count: u32) -> bool {
    v.inventory
        .iter()
        .flatten()
        .filter(|stack| stack.item_id == item && stack.count > 0)
        .map(|stack| stack.count as u32)
        .sum::<u32>()
        >= count
}

fn storage_target(
    state: &TickState,
    slot: EntitySlot,
    id: SettlementId,
    item: u32,
) -> Option<BlockPos> {
    executor::pickup_point(state, id, item, state.entities.pos(slot)).map(block_pos)
}

fn preparation_footing(
    state: &TickState,
    project: &construction::Project,
    next: [i32; 3],
) -> Option<BlockPos> {
    (project.building.is_none()
        && executor::clearing(state, project.settlement, &project.structure)
        && state
            .world
            .physics_block_at(next[0], next[1], next[2])
            .is_some_and(|block| {
                // Planned tree cells keep their footing after decay or removal.
                block == state.registry.states.air
                    || crate::settlements::forestry::tree(state, block)
            }))
    .then(|| {
        crate::settlements::forestry::footing_of(
            state,
            project.settlement,
            &project.structure,
            next,
        )
    })
    .flatten()
}

fn preparation_target(
    state: &TickState,
    slot: EntitySlot,
    project: &construction::Project,
    next: [i32; 3],
) -> Option<BlockPos> {
    let base = preparation_footing(state, project, next)?;
    preparation_stand(state, slot, project, next, base)
}

/// Whether a body standing with its feet at `pos` and these dimensions overlaps
/// another active project's authored site. A villager is a box, not a column:
/// 0.6 wide it straddles a cell edge, and 1.95 tall it owns the cell above, so
/// a feet-column test lets a neighbour lot's walls cross it (D8 review SOL-4,
/// R3-4/R3-5). The site comes from the canonical reservation, so a road, lamp
/// or standalone furnace protects its site too (D8 review S2-2).
fn in_other_body(
    state: &TickState,
    project: &construction::Project,
    pos: [f64; 3],
    half_width: f64,
    height: f64,
) -> bool {
    executor::body_in_other_site(
        state,
        project.settlement,
        &project.structure,
        pos,
        half_width,
        height,
    )
}

/// The same test for a stand on the centre of cell `at`. An arrived builder
/// settles on that centre ([`settle_on_stand`]), so this is the body it works in.
fn standing_in_other_body(
    state: &TickState,
    project: &construction::Project,
    slot: EntitySlot,
    at: BlockPos,
) -> bool {
    let body = state.entities.body(slot);
    in_other_body(
        state,
        project,
        [
            f64::from(at.x) + 0.5,
            f64::from(at.y),
            f64::from(at.z) + 0.5,
        ],
        body.half_width,
        body.height,
    )
}

/// A dry footing near the cell `next` that the builder's body can stand on
/// and from which, once arrived, it is within felling reach of `base`, the
/// cell's own footing.
pub(super) fn preparation_stand(
    state: &TickState,
    slot: EntitySlot,
    project: &construction::Project,
    next: [i32; 3],
    base: BlockPos,
) -> Option<BlockPos> {
    // Seed 24 parked a builder for 42,000 ticks on a hill cell three blocks
    // above and three rows from a cleared pit's footing: the nearest stand to
    // itself, out of reach, so it never moved and never advanced the cell.
    let here = state.entities.pos(slot);
    let body = state.entities.body(slot);
    let mut best: Option<(f64, BlockPos)> = None;
    for dx in -3_i32..=3 {
        for dz in -3_i32..=3 {
            if dx * dx + dz * dz > 9 {
                continue;
            }
            let Some(at) = crate::settlements::forestry::footing_of(
                state,
                project.settlement,
                &project.structure,
                [next[0] + dx, next[1], next[2] + dz],
            ) else {
                continue;
            };
            // A neighbour lot's future body is not a stand: that project would
            // place a wall through this builder (D8 review SOL-4/R3-4).
            if standing_in_other_body(state, project, slot, at) {
                continue;
            }
            // Seed 24 then parked a builder for 51,000 ticks beside a pen,
            // walking toward the top of a fence post: `footing` counts the
            // 1.5-high fence as ground, but no body can stand on it. The stand
            // must be a cell the navigator itself can end a route on.
            let stands = crate::settlements::allocator::standing(
                &state.world,
                &state.registry,
                at,
                body.half_width,
                body.height,
            )
            .is_some();
            let dry = [at.y, at.y + 1].into_iter().all(|y| {
                state
                    .world
                    .physics_block_at(at.x, y, at.z)
                    .is_some_and(|block| {
                        state
                            .registry
                            .blocks
                            .by_id(mc_world::BlockStateId(block))
                            .is_some_and(|definition| !definition.has_fluid())
                    })
            });
            // Arrival leaves the feet within 0.3 of the centre on each axis.
            let reaches = (f64::from(at.x - base.x).powi(2)
                + f64::from(at.y - base.y).powi(2)
                + f64::from(at.z - base.z).powi(2))
            .sqrt()
                + 0.5
                <= PREPARATION_REACH;
            let distance = (here[0] - f64::from(at.x) - 0.5).powi(2)
                + (here[1] - f64::from(at.y)).powi(2)
                + (here[2] - f64::from(at.z) - 0.5).powi(2);
            // D10: a felling stand beside lava is unsafe even when the cell
            // itself is dry; the body rows and their four neighbours must be
            // lava-free.
            let lava_safe = crate::settlements::construction::preparation::liquid::stand_safe(
                state,
                [at.x, at.y, at.z],
            );
            if stands
                && dry
                && lava_safe
                && reaches
                && best.as_ref().is_none_or(|(known, _)| distance < *known)
            {
                best = Some((distance, at));
            }
        }
    }
    best.map(|(_, at)| at)
}

/// Whether the real router walks a body of `width` by `height` from the point
/// `from` to the stand cell `to` within `nodes` expansions; a truncated search
/// is false. The query runs on its own throwaway budget: choosing a goal is
/// planning, not the tick's navigation spend, and the brain charges the route
/// it actually walks.
fn walks(
    state: &TickState,
    from: [f64; 3],
    to: BlockPos,
    width: f64,
    height: f64,
    nodes: usize,
) -> bool {
    let mut cache = crate::settlements::allocator::PoiCache::default();
    let mut path = Vec::new();
    let mut budget = crate::tick_budget::Budget::default();
    let mut ctx = crate::settlements::allocator::RouteCtx {
        world: &state.world,
        registry: &state.registry,
        dimension: state.dimension,
        consumer: crate::tick_budget::Consumer::Allocator,
        budget: &mut budget,
    };
    ctx.admit()
        && crate::settlements::allocator::find_path_limited_ctx(
            &mut ctx, &mut cache, from, to, width, height, &mut path, true, 64, 64, true, nodes,
        )
}

fn site_target(
    state: &TickState,
    project: &construction::Project,
    next: [i32; 3],
    slot: EntitySlot,
    committed: Option<BlockPos>,
) -> Option<BlockPos> {
    // With no stand in felling reach, the cell is placed from the perimeter
    // like any blueprint cell instead of holding the only project slot.
    if preparation_footing(state, project, next).is_some()
        && let Some(stand) = preparation_target(state, slot, project, next)
    {
        return Some(stand);
    }
    if project.blueprint == construction::furnace::NAME {
        let body = state.entities.body(slot);
        // The veto lives inside the side loop: refusing the first free side
        // without trying the others left the furnace with no target while the
        // opposite side was safe (D8 review S2-1).
        return construction::furnace::stand(
            state,
            project,
            next,
            Some((body.half_width, body.height)),
        )
        .map(block_pos);
    }
    if layout::infra(&project.blueprint) {
        return layout::stand(state, project, next)
            .map(block_pos)
            .filter(|at| !standing_in_other_body(state, project, slot, *at));
    }
    let blueprint = blueprints::get(&state.registry, &project.blueprint)?;
    let size = blueprint.footprint(blueprints::Rotation::from_quarter_turns(project.rotation));
    let min = project.origin;
    let max = [
        min[0] + i32::from(size[0]),
        min[1],
        min[2] + i32::from(size[2]),
    ];
    let body = state.entities.body(slot);
    let here = state.entities.pos(slot);
    let mut candidates: Vec<(f64, BlockPos)> = Vec::new();
    // The nearest horizontal side can be below a hillside site's roof reach.
    // Choose an actual supported perimeter stand, with arrival clearance, not
    // a point at the blueprint's fixed base height above a lower slope.
    let edge_min = [min[0] - MARGIN, min[2] - MARGIN];
    let edge_max = [max[0] - 1 + MARGIN, max[2] - 1 + MARGIN];
    let perimeter = (edge_min[0]..=edge_max[0])
        .flat_map(|x| [[x, edge_min[1]], [x, edge_max[1]]])
        .chain((edge_min[1] + 1..edge_max[1]).flat_map(|z| [[edge_min[0], z], [edge_max[0], z]]));
    for [x, z] in perimeter {
        let Some(at) = crate::settlements::forestry::footing_of(
            state,
            project.settlement,
            &project.structure,
            [x, min[1], z],
        ) else {
            continue;
        };
        let Some(stand) = crate::settlements::allocator::standing(
            &state.world,
            &state.registry,
            at,
            body.half_width,
            body.height,
        ) else {
            continue;
        };
        // A neighbour lot's future body is not a stand: another project would
        // place a wall through this builder. The body, not the feet column, is
        // what must stay clear (D8 review SOL-4/R3-5).
        if in_other_body(state, project, stand, body.half_width, body.height) {
            continue;
        }
        let placement_distance = (stand[0] - f64::from(next[0]) - 0.5).powi(2)
            + (stand[1] - f64::from(next[1])).powi(2)
            + (stand[2] - f64::from(next[2]) - 0.5).powi(2);
        if placement_distance > (REACH - super::brain::WAYPOINT_REACH).powi(2) {
            continue;
        }
        // Keep a still-valid destination while walking around terrain. Picking
        // the nearest stand anew can alternate adjacent goals whose routes
        // begin in opposite directions, so neither route ever advances.
        // A goal the router can no longer walk to (the builder stands beneath a
        // roof it can never reach) is no goal to keep.
        if committed == Some(at) && walks(state, here, at, body.half_width, body.height, WALK_NODES)
        {
            return Some(at);
        }
        let distance: f64 = (0..3).map(|axis| (here[axis] - stand[axis]).powi(2)).sum();
        candidates.push((distance, at));
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    // The nearest stand in a straight line can be the roof of a neighbour's
    // finished building or a ledge the router cannot climb: the builder would
    // arrive beneath it and never place (Spain seed 100, the church beside a
    // house: worksite_unreached for 160,000 ticks). The nearest stand the real
    // router walks to is the goal; with none, the nearest as before.
    // A stand beside one the router could not reach is reached or not with it:
    // a roof strip costs one search, not one per column.
    let mut unreached: Vec<BlockPos> = Vec::new();
    let mut searched = 0;
    for (_, at) in &candidates {
        if searched == STAND_ROUTES {
            break;
        }
        if unreached.iter().any(|miss| {
            miss.x.abs_diff(at.x) + miss.z.abs_diff(at.z) <= 1 && miss.y.abs_diff(at.y) <= 1
        }) {
            unreached.push(*at);
            continue;
        }
        searched += 1;
        if walks(state, here, *at, body.half_width, body.height, ROUTE_NODES) {
            return Some(*at);
        }
        unreached.push(*at);
    }
    candidates.first().map(|(_, at)| *at)
}

/// Brain hook only selects a navigation target while villager state is detached.
/// Inventory/world transactions run below with the real entity row attached.
pub(crate) fn work(
    state: &mut TickState,
    slot: EntitySlot,
    v: &mut VillagerState,
    _changes: &mut ChangeSet,
) -> Option<WorkTarget> {
    if !working(state, v)
        || crate::settlements::economy::worker_reserved(
            state,
            v.settlement,
            state.entities.row(slot).uuid,
        )
    {
        return None;
    }
    let id = v.settlement?;
    let uuid = state.entities.row(slot).uuid;
    let project = state
        .settlements
        .get(id)
        .and_then(|record| crate::settlements::construction::project_of_builder(record, uuid))?;
    let next = executor::next_cell(state, id, &project.structure).ok()??;
    if let Some((item, count)) = executor::next_material(state, id, &project.structure).ok()?
        && !carried(v, item, count)
    {
        return Some(WorkTarget {
            pos: storage_target(state, slot, id, item)?,
            exact: false,
        });
    }
    // Already outside the footprint and in reach of the next cell: keep
    // standing here rather than walking around the site for every cell.
    if in_reach(state, slot, project, next) {
        let here = crate::settlements::allocator::at(state.entities.pos(slot));
        return Some(WorkTarget {
            pos: here,
            exact: false,
        });
    }
    let stand = site_target(
        state,
        project,
        next,
        slot,
        v.navigation_target.filter(|_| !v.navigation_search_failed),
    )?;
    settle_on_stand(state, slot, stand);
    Some(WorkTarget {
        pos: stand,
        exact: true,
    })
}

/// The brain ends a route anywhere within WAYPOINT_REACH of the stand's
/// standing point, but a stand is chosen, and kept clear of neighbour sites,
/// for a body on that point. An arrived builder takes the last sub-cell step
/// onto it, as a sleeper settles on its bed (Spain seed 100: a builder 0.25 off
/// its felling stand's centre stood 0.05 into a neighbour's site for a whole
/// stall window). Testing every stand for the whole arrival tolerance instead
/// left a road segment's only stand, which abuts the next segment, unusable.
fn settle_on_stand(state: &mut TickState, slot: EntitySlot, stand: BlockPos) {
    let pos = state.entities.pos(slot);
    let body = state.entities.body(slot);
    let Some(point) = crate::settlements::allocator::standing(
        &state.world,
        &state.registry,
        stand,
        body.half_width,
        body.height,
    ) else {
        return;
    };
    let arrived = (pos[0] - point[0]).powi(2) + (pos[2] - point[2]).powi(2)
        < super::brain::WAYPOINT_REACH.powi(2)
        && (pos[1] - point[1]).abs() < super::brain::WAYPOINT_VERTICAL_REACH;
    if arrived && pos != point {
        state.entities.set_pos(slot, point);
        state.entities.set_vel(slot, [0.0; 3]);
    }
}

/// Whole body outside every future cell and within placement reach of `next`.
/// The 11×9×11 maximum footprint needs sqrt(7.5² + 8²) ≈ 11 blocks from the
/// nearest perimeter point to its farthest roof cell, plus arrival tolerance.
fn in_reach(
    state: &TickState,
    slot: EntitySlot,
    project: &construction::Project,
    next: [i32; 3],
) -> bool {
    if let Some(base) = preparation_footing(state, project, next) {
        // As with lumberjacks, felling is rooted at the trunk's footing; its
        // crown need not be reachable as an independently placed roof block.
        // Standing inside a neighbour lot's future body is never in reach: that
        // project would place a wall through this builder (SOL-4/R3-4).
        let body = state.entities.body(slot);
        if !in_other_body(
            state,
            project,
            state.entities.pos(slot),
            body.half_width,
            body.height,
        ) && super::mining::near(state, slot, base, PREPARATION_REACH)
        {
            return true;
        }
        // Only a cell with no stand in felling reach falls back to the
        // perimeter rule below, matching `site_target`.
        if preparation_stand(state, slot, project, next, base).is_some() {
            return false;
        }
    }
    let pos = state.entities.pos(slot);
    let body = state.entities.body(slot);
    let (width, height) = (body.half_width, body.height);
    if layout::infra(&project.blueprint) || project.blueprint == construction::furnace::NAME {
        // Roads and meadows are walked on while worked; a lamp or furnace
        // column must stay clear of the builder's whole body.
        let clear = matches!(project.blueprint.as_str(), layout::ROAD | layout::MEADOW)
            || [0, 2].into_iter().any(|axis| {
                let min = f64::from(next[axis]);
                pos[axis] + width <= min || pos[axis] - width >= min + 1.0
            });
        return clear
            && !in_other_body(state, project, pos, width, height)
            && super::mining::near(state, slot, block_pos(next), REACH);
    }
    let Some(blueprint) = blueprints::get(&state.registry, &project.blueprint) else {
        return false;
    };
    let size = blueprint.footprint(blueprints::Rotation::from_quarter_turns(project.rotation));
    // Choosing a perimeter goal does not mean the builder has reached it.
    // Keep the full body outside every future cell, not just the next one:
    // placing a wall around a builder can otherwise strand it inside the site.
    let outside = [0, 2].into_iter().any(|axis| {
        let min = f64::from(project.origin[axis]);
        pos[axis] + width <= min || pos[axis] - width >= min + f64::from(size[axis])
    });
    if !outside {
        return false;
    }
    // Standing inside a neighbour lot's future cells would put a wall through
    // this builder: the body's own box decides, not the feet column
    // (D8 review SOL-4/R3-5).
    !in_other_body(state, project, pos, width, height)
        && super::mining::near(state, slot, block_pos(next), REACH)
}

pub(crate) fn wait_reason(
    state: &TickState,
    slot: EntitySlot,
    project: &construction::Project,
    next: [i32; 3],
) -> &'static str {
    let Some(v) = state.entities.row(slot).villager.as_ref() else {
        return "no_builder";
    };
    if crate::settlements::economy::worker_reserved(
        state,
        Some(project.settlement),
        state.entities.row(slot).uuid,
    ) {
        return "reserved_worker";
    }
    if !working(state, v) {
        return "builder_inactive";
    }
    if executor::next_material(state, project.settlement, &project.structure)
        .ok()
        .flatten()
        .is_some_and(|(item, count)| !carried(v, item, count))
    {
        return "collecting_material";
    }
    // D10: name the world-state wait instead of a generic stalled placement. The
    // gate reads the plan entry's state, so a clear cell beside lava names the
    // wait the debit would take, not a wait that block never takes.
    let planned = executor::next_plan_state(state, project.settlement, &project.structure)
        .ok()
        .flatten()
        .unwrap_or(state.registry.states.air);
    if let Some(wait) =
        crate::settlements::construction::preparation::liquid::write_wait(state, next, planned)
    {
        return wait;
    }
    if !in_reach(state, slot, project, next) {
        return "worksite_unreached";
    }
    "placement_not_advancing"
}

/// A member this admission may advance a project with: alive, not reserved by
/// another job, working, and of this settlement.
fn builder_ready(state: &TickState, id: SettlementId, slot: EntitySlot) -> bool {
    state.entities.body(slot).alive
        && !crate::settlements::economy::worker_reserved(
            state,
            Some(id),
            state.entities.row(slot).uuid,
        )
        && state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .is_some_and(|v| v.settlement == Some(id) && working(state, v))
}

/// Called once per construction admission, before the brain detaches carry.
/// One transfer OR budgeted canonical cell advances, never both in an attempt.
pub(crate) fn tick(state: &mut TickState, id: SettlementId) {
    #[cfg(test)]
    ADMISSIONS.with(|count| count.set(count.get() + 1));
    // One project per admission, round-robin over the active set: a tick still
    // writes one structure-registry image (D8).
    // The settlement is chosen every four ticks by `(tick/4) % S`, so the
    // project turn counts whole settlement rounds: a project index that is not
    // congruent mod gcd(S, P) would otherwise never be visited (D8 review
    // P1-1). An unbound project takes its turn like any other and is bound by
    // the admission below (founding handover, fresh project).
    let count = crate::settlements::construction::projects_of(&state.settlements, id).count();
    if count == 0 {
        return;
    }
    let settlements = state.settlements.records().count().max(1) as u64;
    let turn = (state.tick / 4 / settlements) as usize % count;
    let Some(project) = crate::settlements::construction::projects_of(&state.settlements, id)
        .nth(turn)
        .cloned()
    else {
        return;
    };
    // A founding handover admits a project before the planner binds anyone; the
    // first admission binds the resident it is about to advance, so arrival
    // order never leaves a paid project with no authority to advance it.
    let candidate = match project.builder {
        Some((uuid, _)) => Some(uuid),
        // A lazy bind never takes a resident who already holds a project: one
        // binding per resident, or the record stops opening (P1-2). A free
        // builder goes to the project the planner would give it, so an owner's
        // order is never passed over by autonomous work whose turn came first.
        None if construction::unbound_project(&state.settlements, id).as_deref()
            == Some(project.structure.as_str()) =>
        {
            construction::residents(state, id)
                .find(|slot| {
                    builder_ready(state, id, *slot)
                        && !construction::bound_builder(
                            &state.settlements,
                            state.entities.row(*slot).uuid,
                        )
                })
                .map(|slot| state.entities.row(slot).uuid)
        }
        None => None,
    };
    let bound = match (project.builder.map(|(uuid, _)| uuid), candidate) {
        (Some(uuid), _) => Some(uuid),
        (None, Some(uuid)) => {
            let borrowed = state
                .entities
                .slot_of_uuid(uuid)
                .and_then(|slot| state.entities.row(slot).villager.as_ref())
                .map(|v| v.data.profession);
            let borrowed = match borrowed {
                Some(VillagerProfession::Miner | VillagerProfession::Lumberjack) => borrowed,
                _ => None,
            };
            if let Some(record) = state.settlements.get_mut(id)
                && construction::bind_builder(record, &project.structure, Some((uuid, borrowed)))
            {
                record.revision = record.revision.saturating_add(1);
            }
            Some(uuid)
        }
        (None, None) => None,
    };
    // A paused footprint is re-snapshotted and resumed on the next admission.
    if executor::recover(state, id, &project.structure) == Ok(true) {
        return;
    }
    let steps = state
        .test_acceleration
        .construction_steps(state.world.seed() as u64 ^ state.tick.rotate_left(17) ^ id.0);
    for step in 0..steps {
        let next = match executor::next_cell(state, id, &project.structure) {
            Ok(Some(next)) => next,
            Ok(None) => {
                construction::finish_project(state, id, &project.structure);
                return;
            }
            Err(_) => return,
        };
        // Only the builder bound to this project advances it; an unbound or
        // stale binding is the planner's business, not this tick's.
        // The admitted project's builder is resolved by uuid first: a readiness
        // search would stop at whichever resident is first in the list and drop
        // the project's own builder on the filter (SOL-1).
        let Some(slot) = bound
            .and_then(|uuid| state.entities.slot_of_uuid(uuid))
            .filter(|slot| builder_ready(state, id, *slot))
        else {
            return;
        };
        let Ok(item) = executor::next_material(state, id, &project.structure) else {
            return;
        };
        if let Some((item, count)) = item
            && !carried(
                state.entities.row(slot).villager.as_ref().expect("builder"),
                item,
                count,
            )
        {
            if step == 0
                && let Some(point) = storage_target(state, slot, id, item)
                && super::mining::near(state, slot, point, 2.5)
            {
                let uuid = state.entities.row(slot).uuid;
                if let Err(reason) = executor::fetch(
                    state,
                    uuid,
                    &project.structure,
                    [point.x, point.y, point.z],
                    item,
                ) {
                    construction::note_build_refusal(state, id, &project.structure, reason);
                }
            }
            return;
        }
        if !in_reach(state, slot, &project, next) {
            return;
        }
        let uuid = state.entities.row(slot).uuid;
        match executor::advance(state, id, &project.structure, uuid) {
            Ok(true) => {
                construction::finish_project(state, id, &project.structure);
                return;
            }
            Ok(false) => {}
            Err(reason) => {
                construction::note_build_refusal(state, id, &project.structure, reason);
                return;
            }
        }
    }
}

#[cfg(test)]
#[path = "building_site_reach_tests.rs"]
mod site_reach_tests;

/// Cross-project body and stand safety (D8 review SOL-4/R3-3..R3-5). The file
/// lives beside the settlement tests but is registered here: it asserts on this
/// module's private predicates with the body predicates it exercises.
#[cfg(test)]
#[path = "../settlements/construction_body_safety_tests.rs"]
mod body_safety_tests;
