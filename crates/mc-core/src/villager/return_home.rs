//! Runtime-only homeless-resident fallback. Claims and membership stay native;
//! routes use the allocator's router and the brain's committed motion only.
use super::VillagerState;
use crate::boundary::BlockPos;
use crate::entities::EntitySlot;
use crate::settlements::{
    SettlementId,
    allocator::{self, PoiCache, PoiKind, RouteCtx},
};
use crate::tick::TickState;
use crate::tick_budget::Consumer;

const LEG_BLOCKS: f64 = 62.0; // rounding leaves every endpoint within 64 blocks
const ANCHOR_APPROACH_SQUARED: f64 = 4.0 * 4.0;
const HOME_WAKE_BLOCKS: f64 = 32.0;
const NODE_LIMIT: usize = 1024;
// Hold is not persisted: one failure cadence also bounds restored returners.
const RETRY_TICKS: u64 = 1200;
const STAGGER_TICKS: u64 = 40;
const MAX_FAILURES: usize = 3;
const DISPLACEMENT_SQUARED: f64 = 8.0 * 8.0;
const WALK_BLOCKS_PER_SECOND: f64 = 3.0;
const WATCHDOG_MULTIPLIER: f64 = 4.0;
const TICKS_PER_SECOND: f64 = 20.0;

#[derive(Debug)]
pub(crate) struct Memory {
    settlement: SettlementId,
    anchor: [i32; 3],
    radius: u32,
    origin: [f64; 3],
    goal: Option<(BlockPos, bool)>, // bed, otherwise supported anchor approach
    segment: Option<(BlockPos, u64)>,
    waiting: Option<u64>,
    next: u64,
    failures: usize,
    normal: bool,
    rejected: [Option<BlockPos>; MAX_FAILURES],
    hold: bool,
}

impl Memory {
    pub(crate) fn settlement(&self) -> SettlementId {
        self.settlement
    }
}

fn clear(v: &mut VillagerState) {
    v.navigation_target = None;
    v.navigation_path.clear();
    v.navigation_chunks.clear();
    v.navigation_search_failed = false;
}

fn inside(state: &TickState, id: SettlementId, pos: [f64; 3]) -> bool {
    let p = allocator::at(pos);
    state
        .settlements
        .get(id)
        .is_some_and(|r| r.contains([p.x, p.y, p.z]))
}

fn nearest_bed(
    state: &TickState,
    cache: &PoiCache,
    slot: EntitySlot,
    memory: &Memory,
) -> Option<BlockPos> {
    let record = state.settlements.get(memory.settlement)?;
    let pos = state.entities.pos(slot);
    let owner = state.entities.body(slot).id;
    record
        .buildings
        .values()
        .filter(|b| b.kind == crate::settlements::buildings::BuildingKind::House)
        .flat_map(|building| {
            (building.bounds.min[0].div_euclid(16)..=building.bounds.max[0].div_euclid(16))
                .flat_map(move |x| {
                    (building.bounds.min[2].div_euclid(16)..=building.bounds.max[2].div_euclid(16))
                        .filter_map(move |z| cache.chunks.get(&crate::boundary::ChunkPos { x, z }))
                })
                .flat_map(move |chunk| chunk.pois.iter().map(move |poi| (building, *poi)))
        })
        .filter(|(building, poi)| {
            poi.kind == PoiKind::Home
                && !memory.rejected.contains(&Some(poi.pos))
                && [poi.pos.x, poi.pos.y, poi.pos.z]
                    .into_iter()
                    .enumerate()
                    .all(|(i, p)| p >= building.bounds.min[i] && p <= building.bounds.max[i])
                && allocator::claimable_bed(state, cache, owner, Some(memory.settlement), *poi)
        })
        .min_by(|(a, p), (b, q)| {
            allocator::distance(pos, allocator::centre(p.pos))
                .total_cmp(&allocator::distance(pos, allocator::centre(q.pos)))
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| (p.pos.x, p.pos.y, p.pos.z).cmp(&(q.pos.x, q.pos.y, q.pos.z)))
        })
        .map(|(_, p)| p.pos)
}

fn select_goal(
    state: &TickState,
    cache: &PoiCache,
    slot: EntitySlot,
    memory: &Memory,
) -> Option<(BlockPos, bool)> {
    nearest_bed(state, cache, slot, memory)
        .map(|bed| (bed, true))
        .or_else(|| {
            (!inside(state, memory.settlement, state.entities.pos(slot))).then_some((
                BlockPos {
                    x: memory.anchor[0],
                    y: memory.anchor[1],
                    z: memory.anchor[2],
                },
                false,
            ))
        })
}

/// Finite supported candidates; no loading, generation, or second router.
fn waypoint(state: &TickState, slot: EntitySlot, goal: BlockPos) -> Option<BlockPos> {
    let pos = state.entities.pos(slot);
    let dx = f64::from(goal.x) + 0.5 - pos[0];
    let dz = f64::from(goal.z) + 0.5 - pos[2];
    let fraction = (LEG_BLOCKS / mc_physics::mth::length2(dx, dz).max(1.0)).min(1.0);
    let x = (pos[0] + dx * fraction).floor() as i32;
    let z = (pos[2] + dz * fraction).floor() as i32;
    let y = (pos[1] + (f64::from(goal.y) - pos[1]) * fraction).ceil() as i32;
    let body = state.entities.body(slot);
    for (ox, oz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
        let (x, z) = (x + ox, z + oz);
        if mc_physics::mth::length_squared2(
            f64::from(x) + 0.5 - pos[0],
            f64::from(z) + 0.5 - pos[2],
        ) > 64.0 * 64.0
            || !state.world.is_active(x.div_euclid(16), z.div_euclid(16))
            || [-1, 1].into_iter().any(|a| {
                [-1, 1].into_iter().any(|b| {
                    !state
                        .world
                        .has_chunk((x + a).div_euclid(16), (z + b).div_euclid(16))
                })
            })
        {
            continue;
        }
        for dy in [0, 1, -1, 2, -2, 4, -4, 8, -8, 16, -16, 32, -32] {
            let p = BlockPos { x, y: y + dy, z };
            if allocator::standing(
                &state.world,
                &state.registry,
                p,
                body.half_width,
                body.height,
            )
            .is_some()
            {
                return Some(p);
            }
        }
    }
    None
}

fn failed(
    state: &TickState,
    slot: EntitySlot,
    memory: &mut Memory,
    v: &mut VillagerState,
    reason: &'static str,
) {
    if memory.failures < MAX_FAILURES {
        if let Some((bed, true)) = memory.goal {
            memory.rejected[memory.failures] = Some(bed);
        }
        memory.failures += 1;
    }
    memory.goal = None;
    memory.segment = None;
    memory.waiting = None;
    clear(v);
    let outside = !inside(state, memory.settlement, state.entities.pos(slot));
    if memory.failures >= MAX_FAILURES && outside && !memory.hold {
        memory.hold = true;
        memory.origin = state.entities.pos(slot);
        tracing::info!(resident = %state.entities.row(slot).uuid, settlement = memory.settlement.0, reason, "settlement_return_hold");
    }
    if memory.failures >= MAX_FAILURES && !outside {
        memory.normal = true;
    }
    let stagger = state.entities.body(slot).id.0.unsigned_abs() as u64 % STAGGER_TICKS;
    memory.next = state
        .tick_control
        .game_ticks(state.tick)
        .saturating_add(RETRY_TICKS)
        .saturating_add(stagger);
}

/// True means this fallback owns this movement call (including a budget
/// deferral or Hold). A useful native target always preempts it.
pub(crate) fn step(
    state: &mut TickState,
    cache: &mut PoiCache,
    slot: EntitySlot,
    v: &mut VillagerState,
    useful: bool,
) -> bool {
    let now = state.tick_control.game_ticks(state.tick);
    let uuid = state.entities.row(slot).uuid;
    let Some(id) = v.settlement.filter(|_| v.home.is_none()) else {
        crate::settlements::migration::return_memory(state).remove(&uuid);
        return false;
    };
    if useful {
        if let Some(memory) = crate::settlements::migration::return_memory(state).get_mut(&uuid) {
            memory.segment = None;
            memory.waiting = None;
            memory.next = memory.next.max(now.saturating_add(STAGGER_TICKS));
        }
        return false;
    }
    let Some(record) = state.settlements.get(id) else {
        return false;
    };
    let (anchor, radius) = (record.anchor, record.radius);
    let owner = state.entities.body(slot).id;
    let pos = state.entities.pos(slot);
    let previous = crate::settlements::migration::return_memory(state).remove(&uuid);
    let mut fresh = previous.is_none();
    let Some(mut memory) = previous.or_else(|| {
        cache
            .home_search
            .get(&owner)
            .filter(|r| r.no_candidates)
            .map(|_| Memory {
                settlement: id,
                anchor,
                radius,
                origin: pos,
                goal: None,
                segment: None,
                waiting: None,
                next: now + owner.0.unsigned_abs() as u64 % STAGGER_TICKS,
                failures: 0,
                rejected: [None; MAX_FAILURES],
                hold: false,
                normal: false,
            })
    }) else {
        return false;
    };
    if (memory.settlement, memory.anchor, memory.radius) != (id, anchor, radius) {
        fresh = true;
        clear(v);
        memory = Memory {
            settlement: id,
            anchor,
            radius,
            origin: pos,
            goal: None,
            segment: None,
            waiting: None,
            next: now + owner.0.unsigned_abs() as u64 % STAGGER_TICKS,
            failures: 0,
            rejected: [None; MAX_FAILURES],
            hold: false,
            normal: false,
        };
    }
    if fresh {
        memory.goal = select_goal(state, cache, slot, &memory);
        if memory.goal.is_none() {
            memory.normal = true;
            memory.next = now + RETRY_TICKS;
        } else {
            clear(v);
        }
    }
    let mut owns = true;
    if memory.hold && inside(state, id, pos) {
        memory.hold = false;
        memory.normal = true;
        memory.next = memory.next.max(now + RETRY_TICKS);
        owns = false;
    } else if now < memory.next {
        owns = !memory.normal;
    } else {
        if memory.normal {
            memory.normal = false;
            memory.failures = 0;
            memory.rejected = [None; MAX_FAILURES];
            memory.goal = None;
        }
        if memory.failures >= MAX_FAILURES
            && allocator::distance(pos, memory.origin) >= DISPLACEMENT_SQUARED
        {
            memory.failures = 0;
            memory.rejected = [None; MAX_FAILURES];
            memory.hold = false;
        }
        if let Some(wake) = memory.waiting {
            if cache
                .home_search
                .get(&owner)
                .is_some_and(|r| r.tick >= wake)
            {
                failed(
                    state,
                    slot,
                    &mut memory,
                    v,
                    "home_search_completed_without_claim",
                );
            }
        } else {
            if let Some((target, deadline)) = memory.segment {
                let supported = allocator::standing(
                    &state.world,
                    &state.registry,
                    target,
                    state.entities.body(slot).half_width,
                    state.entities.body(slot).height,
                );
                let arrived = supported.is_some_and(|p| {
                    mc_physics::mth::length_squared2(p[0] - pos[0], p[2] - pos[2])
                        < super::brain::WAYPOINT_REACH * super::brain::WAYPOINT_REACH
                        && (p[1] - pos[1]).abs() < 0.6
                });
                if arrived {
                    memory.segment = None;
                    memory.goal = nearest_bed(state, cache, slot, &memory)
                        .map(|bed| (bed, true))
                        .or(memory.goal.filter(|(_, bed)| !bed));
                } else if now >= deadline
                    || v.navigation_target != Some(target)
                    || allocator::route_changed(state, v)
                {
                    failed(state, slot, &mut memory, v, "segment_stalled_or_changed");
                }
            }
            if now >= memory.next && memory.segment.is_none() {
                if memory.goal.is_none() {
                    memory.goal = select_goal(state, cache, slot, &memory);
                }
                if let Some((goal, bed)) = memory.goal {
                    if bed
                        && allocator::distance(pos, allocator::centre(goal))
                            <= HOME_WAKE_BLOCKS * HOME_WAKE_BLOCKS
                    {
                        clear(v);
                        cache.search_at.insert(owner, state.tick);
                        cache.allocator_updates.push(owner);
                        memory.waiting = Some(state.tick.saturating_add(1));
                    } else if !bed
                        && inside(state, id, pos)
                        && allocator::distance(pos, allocator::centre(goal))
                            <= ANCHOR_APPROACH_SQUARED
                    {
                        clear(v);
                        memory.goal = None;
                        memory.next = now + RETRY_TICKS;
                        memory.normal = true;
                        owns = false;
                    } else if cache.spend_path(state, Consumer::Brain) {
                        if let Some(target) = waypoint(state, slot, goal) {
                            let (width, height) = (
                                state.entities.body(slot).half_width,
                                state.entities.body(slot).height,
                            );
                            let mut ctx = RouteCtx::tick(state, Consumer::Brain);
                            let found = allocator::find_path_limited_ctx(
                                &mut ctx,
                                cache,
                                pos,
                                target,
                                width,
                                height,
                                &mut v.navigation_path,
                                true,
                                72,
                                64,
                                true,
                                NODE_LIMIT,
                            );
                            if found {
                                v.navigation_target = Some(target);
                                allocator::capture_route(state, v, pos, true);
                                let seconds = allocator::distance(pos, allocator::centre(target))
                                    .sqrt()
                                    / WALK_BLOCKS_PER_SECOND;
                                memory.segment = Some((
                                    target,
                                    now + (WATCHDOG_MULTIPLIER
                                        * seconds.max(1.0)
                                        * TICKS_PER_SECOND)
                                        .ceil() as u64,
                                ));
                            } else {
                                failed(state, slot, &mut memory, v, "route_truncated_or_blocked");
                            }
                        } else {
                            failed(state, slot, &mut memory, v, "no_supported_waypoint");
                        }
                    }
                } else {
                    clear(v);
                    memory.next = now + RETRY_TICKS;
                    memory.normal = true;
                    owns = false;
                }
            }
        }
    }
    crate::settlements::migration::return_memory(state).insert(uuid, memory);
    owns
}

#[cfg(test)]
#[path = "return_home_tests.rs"]
mod tests;
