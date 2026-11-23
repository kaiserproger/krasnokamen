//! Entity phase: worker row preparation, parallel `step_entity` and pickup
//! intent collection. Moved verbatim from `tick.rs`; stage order unchanged.

use super::*;

mod coreperf;
mod prepare;
mod rest;
mod step;
pub(crate) use coreperf::coreperf_block_region;
use coreperf::{coreperf_active_chunks, coreperf_index, coreperf_region, coreperf_slot_region};
pub(super) use coreperf::{coreperf_damage_observation, coreperf_productive_changes};
pub(super) use prepare::*;
pub(super) use rest::*;
pub(super) use step::*;

#[cfg(test)]
#[path = "../entity_phase_tests.rs"]
mod entity_phase_tests;

#[cfg(test)]
#[path = "../sensor_bench_tests.rs"]
mod sensor_bench_tests;

#[cfg(test)]
#[path = "../entity_prephase_tests.rs"]
mod entity_prephase_tests;

/// Hot worker snapshot; rare state is read by slot while the store is immutable.
#[derive(Debug, Clone, Copy)]
pub(super) struct ReadRow {
    pub(super) slot: EntitySlot,
    pub(super) id: EntityId,
    pub(super) battle: bool,
    /// Current column's ID-staggered activation decision.
    pub(super) think: bool,
    pub(super) pos: [f64; 3],
    pub(super) panic_multiplier: f64,
    /// Velocity written last tick: the standing decision between AI thinks.
    pub(super) vel: [f64; 3],
    pub(super) rotation: [f32; 3],
    pub(super) movement: crate::blueprint::MovementClass,
    pub(super) air_supply: i32,
    pub(super) horizontal_impulse: [f64; 2],
    pub(super) alive: bool,
    pub(super) ground: bool,
    pub(super) half_width: f64,
    pub(super) height: f64,
    pub(super) sight_range: f32,
    pub(super) attack_reach: f32,
    pub(super) attack_damage: f32,
    pub(super) melee_bonus: f32,
    pub(super) attack_cooldown: u32,
    pub(super) love: u32,
    pub(super) is_enderman: bool,
    pub(super) is_iron_golem: bool,
    pub(super) hunts_villagers: bool,
    pub(super) hunts_living: bool,
    pub(super) is_slime_size_one: bool,
    /// Blueprint locomotion capabilities read by the motion step.
    pub(super) climbs_walls: bool,
    pub(super) head_follows_body: bool,
    pub(super) hops: bool,
    pub(super) avoids_armadillos: bool,
    pub(super) aquatic_control: crate::species::AquaticControl,
    /// Resolved from the blueprint's goal set (data, not type names).
    pub(super) chaser: bool,
    /// Breed goal, eligibility, adulthood, cooldown and love checked once.
    pub(super) breed_ready: bool,
    pub(super) temptable: bool,
    /// Blueprint key used for pair compatibility and following.
    pub(super) type_id: u32,
    /// Mature item snapshot (item kind, alive, zero pickup delay).
    pub(super) pickup_ready: bool,
    pub(super) item_entity: bool,
    pub(super) player: bool,
    pub(super) speed: f32,
    pub(super) enderman_target: Option<EntityId>,
    pub(super) levitation: f32,
    pub(super) slow_falling: bool,
    pub(super) no_gravity: bool,
    pub(super) special: Option<crate::hostile::Special>,
    /// Target acquisition uses local light, except recent retaliation.
    pub(super) dark_aggro: bool,
    /// A lit fuse holds the mob still.
    pub(super) standing: bool,
    pub(super) plugin_target: Option<[f64; 3]>,
    pub(super) leash_follow: bool,
    pub(super) gold_target: Option<[f64; 3]>,
    pub(super) has_retained_target: bool,
    pub(super) retain_target: bool,
}

/// Owned movement write from one worker.
pub(super) struct MoveWrite {
    pub(super) slot: EntitySlot,
    pub(super) pos: [f64; 3],
    pub(super) vel: [f64; 3],
    pub(super) rotation: [f32; 3],
    pub(super) air_supply: i32,
    pub(super) ground: bool,
    pub(super) wall_climbing: bool,
    pub(super) horizontal_impulse: [f64; 2],
    /// Only a newly acquired target needs serial memory writeback.
    pub(super) chase_target: crate::target_grid::Nearest,
}

#[derive(Default)]
pub(super) struct EntityPhaseScratch {
    pub(super) rows: Vec<ReadRow>,
    pub(super) writes: Vec<Option<(MoveWrite, Vec<RawIntent>)>>,
    pub(super) specials: Vec<(EntityId, crate::hostile::Special)>,
    pub(super) players: Vec<(EntityId, [f64; 3])>,
    pub(super) activation: HashMap<(i32, i32), u64>,
    pub(super) periods: Vec<u8>,
    pub(super) resting: Vec<EntitySlot>,
    pub(super) coreperf: Option<std::sync::Arc<crate::boundary::CorePerfTick>>,
    pub(super) rest_proofs: RestProofs,
    /// Zombie-arms decisions of the serial writeback (capacity reused tick to tick).
    pub(super) arms: Vec<Option<(EntityId, i8)>>,
}

/// Memo of the idle-rest physics proof ([`idle_wander_motion_unchanged`]).
///
/// Bottleneck: every decision tick of every resting, wander-quiet mob re-ran
/// the full motion step to prove it is a bit-exact no-op, although neither the
/// mob nor the blocks around it had changed since the last proof (about 6.9 %
/// of the native-100 cycles; admission 2.8 ms per tick). Boundary: one stamp
/// per dense slot, written and read only by the worker that admits that slot,
/// valid for no longer than the blocks of its footprint chunks stay unchanged.
/// Correctness fence: the step is a pure function of the key inputs (body,
/// pose, velocity, impulse, air, gravity and effect inputs: see
/// [`rest_proof_key`]) and the blocks of the footprint chunks, which is the
/// premise `unchanged_entity_footprint` already uses on non-decision ticks. A
/// stamp is honoured only for an equal 64-bit key and while no footprint chunk
/// is newer than the proof's revision or has a pending write; stamps are
/// written only for footprints with no pending write; swimmers never use it
/// (their step reads the tick). The entity id is part of the key, so a reused
/// slot never inherits a stamp. Removal path: delete the memo, keeping the
/// proof call (identical result).
#[derive(Default)]
pub(super) struct RestProofs {
    /// Per slot: key, revision the proof was made at.
    stamps: Vec<[std::sync::atomic::AtomicU64; 2]>,
    /// World revision at the start of this phase: the revision a new proof is
    /// made at.
    revision: u64,
}

impl RestProofs {
    /// Start a phase over `len` slots at world revision `revision`.
    pub(super) fn begin(&mut self, len: usize, revision: u64) {
        if self.stamps.len() < len {
            self.stamps.resize_with(len, Default::default);
        }
        self.revision = revision;
    }

    fn get(&self, slot: EntitySlot) -> Option<(u64, u64)> {
        use std::sync::atomic::Ordering::Relaxed;
        let [key, revision] = self.stamps.get(slot.0 as usize)?;
        Some((key.load(Relaxed), revision.load(Relaxed)))
    }

    fn set(&self, slot: EntitySlot, key: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        if let Some([stored, revision]) = self.stamps.get(slot.0 as usize) {
            stored.store(key, Relaxed);
            revision.store(self.revision, Relaxed);
        }
    }
}

/// Entity phase: every simulated entity reads the snapshot and emits its own
/// next state plus intents. Parallel by data over scoped threads; workers
/// return owned writes applied in slot order afterwards. No locks: the only
/// synchronization is the scope join.
pub(super) fn run_entity_phase(state: &mut TickState, sim: bool) -> Vec<Intent> {
    #[cfg(all(test, debug_assertions))]
    sensor_bench_tests::COUNTS.set((0, 0));
    let pre_start = std::time::Instant::now();
    let mut laps = Laps::new(std::env::var_os("MC_CORE_PHASE_LOG").is_some(), pre_start);
    if sim {
        crate::contact_hazards::hurt_by_water(state);
        crate::enderman::tick(state);
    }
    let battle_intents = if sim {
        crate::battle::step(state)
    } else {
        Vec::new()
    };
    laps.lap("battle");
    laps.lap("pre");
    state.entities.prepare_motion_hints(&state.registry);
    laps.lap("motion_hints");
    let unchanged_since = state.entity_phase_revision;
    state.entity_phase_revision = state.world.revision_now();
    let EntityPhaseScratch {
        mut rows,
        writes: mut ordered,
        mut specials,
        mut players,
        mut activation,
        mut periods,
        mut resting,
        coreperf: _,
        mut rest_proofs,
        mut arms,
    } = std::mem::take(&mut state.entity_phase_scratch);
    periods.resize(state.entities.len(), 0);
    rest_proofs.begin(state.entities.len(), state.entity_phase_revision);
    let mut admitted = 0;
    let perf_index = coreperf_index();
    let mut perf_counts = perf_index.map(|_| HashMap::default());
    if let (Some(index), Some(counts)) = (perf_index, &mut perf_counts) {
        coreperf_active_chunks(
            index,
            state
                .world
                .active_positions()
                .filter(|chunk| state.world.is_active(chunk.x, chunk.z))
                .map(|chunk| (chunk.x, chunk.z)),
            counts,
        );
    }
    // Compact flags preserve synthetic player bodies without PlayerState,
    // including global motion admission for players outside active chunks.
    for slot in state.entities.motion_slots(MOTION_OWNED | MOTION_PLAYER) {
        periods[slot.0 as usize] = 1;
        admitted += 1;
        if state.entities.motion_flags(slot) & MOTION_ALIVE != 0 {
            players.push((state.entities.body(slot).id, state.entities.pos(slot)));
        }
    }
    laps.lap("player_motion");
    // Mob decisions run every 1/2/4 ticks at near/mid/far chunk distances.
    for (_, pos) in &players {
        let cx = mc_physics::mth::floor_i32(pos[0]).div_euclid(16);
        let cz = mc_physics::mth::floor_i32(pos[2]).div_euclid(16);
        for dz in -MOB_MID_CHUNKS..=MOB_MID_CHUNKS {
            for dx in -MOB_MID_CHUNKS..=MOB_MID_CHUNKS {
                let period = if dx.abs().max(dz.abs()) <= MOB_NEAR_CHUNKS {
                    1
                } else {
                    2
                };
                activation
                    .entry((cx + dx, cz + dz))
                    .and_modify(|best| *best = (*best).min(period))
                    .or_insert(period);
            }
        }
    }
    laps.lap("activation");
    // Exact, mostly active membership lets hot flags/positions replace per-ID
    // index traversal even when a few occupied chunks are inactive.
    if !sim {
        // Frozen (`/tick freeze`): only player bodies step; every other
        // entity keeps its position, velocity and timers.
    } else if prefer_dense_entity_admission(state) {
        if perf_index.is_some() {
            // Per-region attribution needs the serial visit order and `state`.
            admitted += dense_period_marks(
                &state.entities,
                &state.world,
                &activation,
                0,
                &mut periods,
                |slot| {
                    if let (Some(index), Some(counts)) = (perf_index, &mut perf_counts)
                        && let Some(region) = coreperf_slot_region(index, state, slot)
                    {
                        coreperf_region(counts, region).motion_admitted += 1;
                    }
                },
            );
        } else {
            let parallel = periods.len() >= PARALLEL_PREP_MIN;
            admitted += mark_dense_admission(
                &state.entities,
                &state.world,
                &activation,
                &mut periods,
                parallel,
            );
        }
    } else {
        // Preserve the indexed union and period order for old type-transition
        // entries rather than silently replacing them with current positions.
        for chunk in state.world.active_positions() {
            if !state.world.is_active(chunk.x, chunk.z) {
                continue;
            }
            let period = activation
                .get(&(chunk.x, chunk.z))
                .copied()
                .unwrap_or(MOB_FAR_PERIOD) as u8;
            state.entities.for_each_slot_in_chunk(chunk, |slot| {
                let flags = state.entities.motion_flags(slot);
                if flags & MOTION_OWNED != 0 && flags & MOTION_PLAYER == 0 {
                    let mark = &mut periods[slot.0 as usize];
                    if *mark == 0
                        && let (Some(index), Some(counts)) = (perf_index, &mut perf_counts)
                        && let Some(region) = coreperf_slot_region(index, state, slot)
                    {
                        coreperf_region(counts, region).motion_admitted += 1;
                    }
                    admitted += usize::from(*mark == 0);
                    *mark = period;
                }
            });
        }
    }
    laps.lap("admission");
    crate::hostile::refresh_targets(state);
    let targets_empty = !crate::hostile::has_live_targets(state);
    // Reuse the sensor's current resident table before admission. Any
    // candidate conservatively wakes decision ticks, even out of range or
    // occluded: normal sensor cadence catches same-chunk range crossings,
    // spawn, teleport and dimension travel without a cached wake dependency.
    let villagers = if admitted == 0 {
        Vec::new()
    } else {
        resident_villager_targets(&state.entities)
    };
    laps.lap("targets_villagers");
    rows.reserve(admitted);
    let mut prepared = PrepChunk {
        rows: std::mem::take(&mut rows),
        specials: std::mem::take(&mut specials),
        resting: std::mem::take(&mut resting),
        rare: Vec::new(),
        summary: PrepSummary::default(),
    };
    prepare_rows(
        state,
        &periods,
        &PrepEnv {
            targets_empty,
            villagers_empty: villagers.is_empty(),
            unchanged_since,
            rest_proofs: Some(&rest_proofs),
        },
        admitted >= PARALLEL_PREP_MIN,
        &mut prepared,
    );
    laps.lap("prepare_rows");
    let PrepSummary {
        has_dolphin,
        consumers,
        temptation_consumers,
        cow_cells,
    } = std::mem::take(&mut prepared.summary);
    PrepChunk {
        rows,
        specials,
        resting,
        ..
    } = prepared;
    if let (Some(index), Some(counts)) = (perf_index, &mut perf_counts) {
        for row in &rows {
            if let Some(region) = coreperf_slot_region(index, state, row.slot) {
                coreperf_region(counts, region).motion_full += 1;
            }
        }
        for &slot in &resting {
            if let Some(region) = coreperf_slot_region(index, state, slot) {
                coreperf_region(counts, region).motion_resting += 1;
            }
        }
        for &(id, special) in &specials {
            if special == crate::hostile::Special::Projectile
                && let Some(slot) = state.entities.slot_of(id)
            {
                let chunk = state.entities.chunk_of(slot);
                if let Some(&region) = index.chunks.get(&(chunk.x, chunk.z)) {
                    coreperf_region(counts, region).projectile_candidates += 1;
                }
            }
        }
    }
    let coreperf = perf_counts.map(|counts| {
        std::sync::Arc::new(crate::boundary::CorePerfTick {
            tick: state.tick + 1,
            resident_entities: state.entities.len(),
            xp_orbs_processed: 0,
            regions: counts.into_values().collect(),
            ..Default::default()
        })
    });
    laps.lap("prep_summary");
    if rows.is_empty() && resting.is_empty() {
        periods.clear();
        activation.clear();
        state.entity_phase_scratch = EntityPhaseScratch {
            rows,
            writes: ordered,
            specials,
            players,
            activation,
            periods,
            resting,
            coreperf,
            rest_proofs,
            arms,
        };
        return Vec::new();
    }
    let golem_targets = if consumers[3] != 0 {
        crate::villager::defense::targets(state)
    } else {
        Vec::new()
    };
    laps.lap("golem_targets");
    let world = &state.world;
    let registry = &state.registry;
    let chunk_loaded = |x: i32, z: i32| world.has_chunk(x.div_euclid(16), z.div_euclid(16));
    let tick = state.tick;
    let chunk_revision = |cx: i32, cz: i32| world.chunk_revision(cx, cz);
    let rows_ref = &rows;
    let dolphin_toys: Vec<[f64; 3]> = if has_dolphin {
        rows.iter()
            .filter(|row| row.item_entity && row.alive && row.pickup_ready)
            .map(|row| row.pos)
            .collect()
    } else {
        Vec::new()
    };
    // Chase targets: vanilla monsters ignore creative and spectator players.
    let targets: Vec<(EntityId, [f64; 3])> = players
        .iter()
        .filter(|(id, _)| {
            state.players.get(id).is_some_and(|player| {
                !player.creative()
                    && !player.spectator()
                    && !state.session_locked(player.connection)
            })
        })
        .copied()
        .collect();
    let piglin_targets: Vec<_> = if crate::piglin::piglin_present(state) {
        targets
            .iter()
            .filter(|(id, _)| !crate::piglin::wears_gold(state, *id))
            .copied()
            .collect()
    } else {
        Vec::new()
    };
    let zoglin_targets: Vec<_> = if consumers[4] != 0 {
        state
            .entities
            .slots()
            .filter_map(|slot| {
                let body = state.entities.body(slot);
                let EntityKind::Mob { type_id } = body.kind else {
                    return None;
                };
                (body.alive
                    && state.entities.row(slot).health > 0.0
                    && !crate::axolotl::playing_dead(state.entities.row(slot))
                    && mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(
                        crate::blueprint::wire_type_id(type_id),
                    )
                    .is_some_and(|contract| {
                        contract.behavior.archetype
                            == mc_data::entity_types::EntityArchetype::Living
                    })
                    && !state
                        .registry
                        .blueprint(type_id)
                        .is_some_and(|bp| bp.capabilities.ignored_by_living_hunters))
                .then(|| (body.id, state.entities.pos(slot)))
            })
            .collect()
    } else {
        Vec::new()
    };
    laps.lap("zoglin_targets");
    // Chase searches per class this tick (over-counted: not every chaser
    // thinks). Only a class with many consumers pays for an index; the rest
    // scan linearly as before (see target_grid.rs for boundary and fence).
    let grid = crate::target_grid::TargetGrid::for_consumers;
    let mut player_grid = grid(&targets, consumers[0]);
    let mut piglin_grid = grid(&piglin_targets, consumers[1]);
    let mut villager_grid = grid(&villagers, consumers[2]);
    let mut golem_grid = grid(&golem_targets, consumers[3]);
    let mut zoglin_grid = grid(&zoglin_targets, consumers[4]);
    let searches = rows.iter().filter(|row| {
        row.alive
            && !row.battle
            && row.chaser
            && row.think
            && !row.breed_ready
            && !row.standing
            && row.plugin_target.is_none()
            && row.panic_multiplier == 0.0
            && !row.has_retained_target
    });
    let query = |row: &ReadRow| {
        (
            row.pos,
            if row.is_enderman {
                64.0
            } else {
                f64::from(row.sight_range)
            },
        )
    };
    player_grid.batch(
        searches
            .clone()
            .filter(|row| !row.is_iron_golem && !crate::piglin::is_piglin_type(row.type_id))
            .map(query),
    );
    piglin_grid.batch(
        searches
            .clone()
            .filter(|row| crate::piglin::is_piglin_type(row.type_id))
            .map(query),
    );
    villager_grid.batch(
        searches
            .clone()
            .filter(|row| row.hunts_villagers)
            .map(query),
    );
    golem_grid.batch(searches.clone().filter(|row| row.is_iron_golem).map(query));
    zoglin_grid.batch(searches.filter(|row| row.hunts_living).map(query));
    laps.lap("target_grids");
    let cows_ref = &cow_cells;
    // Work-stealing over the core's long-lived rayon workers (no threads
    // spawned per tick); `collect` keeps slot order.
    let ctx = StepCtx {
        dolphin_targets: std::sync::Arc::new(state.locate.dolphin_targets(state.dimension)),
        rows: rows_ref,
        entities: &state.entities,
        battle: &state.hostile.battle,
        retained_targets: &state.hostile.targets,
        players: player_grid,
        passive: passive_follow::Context {
            players: grid(&players, temptation_consumers),
            states: &state.players,
            locked: state
                .require_plugin_login
                .then_some(&*state.locked_sessions),
        },
        dolphin_toys: &dolphin_toys,
        villagers: villager_grid,
        golem_targets: golem_grid,
        piglin_targets: piglin_grid,
        zoglin_targets: zoglin_grid,
        cow_cells: cows_ref,
        world,
        chunk_loaded: &chunk_loaded,
        registry,
        tick,
        chunk_revision: &chunk_revision,
        unchanged_since,
        difficulty: state.difficulty,
        sky_darken: if state.dimension.has_weather() {
            crate::light::sky_darken(state.day_time, state.rain_level, state.thunder_level)
        } else {
            15
        },
    };
    system_graph::run(&ctx, &mut ordered);
    laps.lap("step_scope");
    #[cfg(all(test, debug_assertions))]
    {
        let grids = [
            &ctx.players,
            &ctx.piglin_targets,
            &ctx.villagers,
            &ctx.golem_targets,
            &ctx.zoglin_targets,
        ];
        sensor_bench_tests::COUNTS.set((
            grids.iter().map(|grid| grid.query_count()).sum(),
            grids.iter().map(|grid| grid.batch_count()).sum(),
        ));
    }
    let mut seq = std::mem::take(&mut state.seq_counter);
    let mut intents = battle_intents;
    let physical_profiles = state.registry.item_facts.has_physical_profiles();
    retain_resting_writeback(state, &mut resting);
    for slot in resting.drain(..) {
        let body = state.entities.body(slot);
        if !crate::battle::battle_row(body.faction) {
            let aggressive = state.hostile.targets.contains_key(&body.id);
            crate::hostile::set_zombie_aggressive(state, slot, aggressive);
        }
        // A quiet row's writeback is a no-op; the hot flag spares the cold
        // row (a cache miss per resting body).
        if state.entities.motion_flags(slot) & crate::entities::MOTION_QUIET_TIMERS != 0 {
            continue;
        }
        let body = state.entities.body(slot);
        state.entities.set_motion_state(
            slot,
            [0.0; 2],
            body.air_supply,
            crate::entities::tick_pickup_delay(body.pickup_delay),
            body.attack_cooldown.saturating_sub(1),
        );
    }
    laps.lap("resting_writeback");
    // Zombie arms (PerfEntity2). Bottleneck: the serial writeback read the body,
    // cold row and appearance of every stepped zombie to learn that nothing
    // changes (3 ms of 12 at 48,000 rows in the native-100-like micro). Boundary:
    // the decision only; the writes below stay serial and in row order. Fence:
    // `zombie_arms_update` is a pure read of the store and the target table;
    // a row's own earlier target insert is folded in as `chase_target.is_some()`
    // and no other row's writeback touches this row's appearance or id, so the
    // decision equals the in-loop one. Removal path: call `set_zombie_aggressive`
    // per row as the small-population path below does.
    if ordered.len() >= PARALLEL_PREP_MIN {
        use rayon::prelude::*;
        let (entities, registry, targets) =
            (&state.entities, &*state.registry, &state.hostile.targets);
        ordered
            .par_iter()
            .map(|entry| {
                let (write, _) = entry.as_ref()?;
                crate::hostile::zombie_arms_update(
                    entities,
                    registry,
                    write.slot,
                    write.chase_target.is_some()
                        || targets.contains_key(&entities.body(write.slot).id),
                )
            })
            .collect_into_vec(&mut arms);
    } else {
        arms.clear();
    }
    for (index, entry) in ordered.drain(..).enumerate() {
        let Some((write, mut raw)) = entry else {
            continue;
        };
        if let Some(target) = write.chase_target {
            state
                .hostile
                .targets
                .insert(state.entities.body(write.slot).id, target);
            crate::hostile::set_zombie_aggressive(state, write.slot, true);
        }
        if arms.is_empty() {
            let aggressive = write.chase_target.is_some()
                || state
                    .hostile
                    .targets
                    .contains_key(&state.entities.body(write.slot).id);
            crate::hostile::set_zombie_aggressive(state, write.slot, aggressive);
        } else if let Some((id, next)) = arms[index] {
            crate::hostile::set_zombie_arms(state, write.slot, id, next);
        }
        let attack_cooldown = raw
            .iter()
            .any(|intent| {
                matches!(
                    intent.kind,
                    IntentKind::DamageEntity {
                        kind: crate::survival::DamageKind::Melee,
                        ..
                    }
                )
            })
            .then_some(MELEE_ATTACK_INTERVAL);
        let was_ground = state.entities.ground(write.slot);
        let old = state.entities.apply_motion_result(
            write.slot,
            &crate::entities::MotionResult {
                pos: write.pos,
                vel: write.vel,
                yaw: write.rotation[0],
                pitch: write.rotation[1],
                head_yaw: write.rotation[2],
                ground: write.ground,
                horizontal_impulse: write.horizontal_impulse,
                air_supply: write.air_supply,
                attack_cooldown,
            },
        );
        if matches!(
            state.entities.body(write.slot).kind,
            EntityKind::Item { .. }
        ) {
            crate::redstone::containers::item_moved(&mut state.redstone_containers, write.pos);
        }
        let (fall, stalagmite) = crate::mob_fall::after_move(
            state,
            write.slot,
            old,
            write.pos,
            was_ground,
            write.ground,
            write.wall_climbing,
        );
        if fall > 0.0 {
            let actor = state.entities.body(write.slot).id;
            raw.push(RawIntent {
                actor,
                kind: IntentKind::DamageEntity {
                    target: actor,
                    amount: fall,
                    kind: if stalagmite {
                        crate::survival::DamageKind::Stalagmite
                    } else {
                        crate::survival::DamageKind::Fall
                    },
                    source: None,
                },
                footprint: Footprint {
                    entity_writes: vec![actor],
                    ..Default::default()
                },
            });
        }
        // Only a player or a body wearing armor can spend movement stamina;
        // the hot flags answer that without dereferencing the row's
        // appearance (a cache miss per unarmored mob).
        if physical_profiles
            && state.entities.motion_flags(write.slot)
                & (crate::entities::MOTION_PLAYER | crate::entities::MOTION_ARMORED)
                != 0
        {
            crate::physical_defense::accepted_movement(
                state,
                write.slot,
                mth::length2(write.pos[0] - old[0], write.pos[2] - old[2]),
            );
        }
        for raw_intent in raw {
            let next = seq.get(&raw_intent.actor).copied().unwrap_or(0);
            seq.insert(raw_intent.actor, next + 1);
            intents.push(Intent {
                actor: raw_intent.actor,
                seq: next,
                kind: raw_intent.kind,
                footprint: raw_intent.footprint,
            });
        }
    }
    laps.lap("ordered_writeback");
    state.seq_counter = seq;
    if sim {
        crate::item_motion::merge_neighbours(state);
    }
    laps.lap("item_merge");
    let post_start = std::time::Instant::now();
    state.projectiles.post = crate::projectiles::PostTimes::default();
    // Pickup intents: players collect mature item entities in reach.
    // Exact kind lanes, authoritative maturity and post-move positions.
    let pickup_start = std::time::Instant::now();
    if sim {
        collect_pickup_intents(state, &mut intents);
    }
    state.projectiles.post.pickup_us = pickup_start.elapsed().as_micros() as u64;
    laps.lap("pickup_intents");
    // Shooting skeletons, flying arrows, swelling creepers (serial: they
    // read the world and every entity).
    if sim {
        let hostile_start = std::time::Instant::now();
        crate::hostile::run(state, &mut intents, &specials);
        state.projectiles.post.hostile_us = hostile_start.elapsed().as_micros() as u64;
        laps.lap("hostile");
        crate::projectiles::run(state, &mut intents, &specials);
        laps.lap("projectiles");
    }
    let post_time = post_start.elapsed();
    state.projectiles.post.total_us = post_time.as_micros() as u64;
    if std::env::var("MC_CORE_PHASE_LOG").is_ok() {
        laps.print("entity phase");
        let post = state.projectiles.post;
        eprintln!(
            "  post detail us: pickup={} hostile={} weapons={} hittables={} candidates={} shots={} projectiles={} grounded={}",
            post.pickup_us,
            post.hostile_us,
            post.weapons_us,
            post.hittables_us,
            post.candidates_us,
            post.shots_us,
            post.shots,
            post.grounded,
        );
        let (battle, probe) = crate::battle::phase_log_detail(state);
        eprintln!(
            "  battle detail us: total={} counters={} formation={} columns={} targets={} retaliation={} steering={} combat={} morale={} projection={} emit={} stepped_battle_rows={} battle_slots={} {}",
            battle.total_us,
            battle.counters_us,
            battle.formation_us,
            battle.columns_us,
            battle.targeting_us,
            battle.retaliation_us,
            battle.steering_us,
            battle.combat_us,
            battle.morale_us,
            battle.projection_us,
            battle.emit_us,
            rows.iter().filter(|row| row.battle).count(),
            state.entities.battle_slots().count(),
            probe,
        );
    }
    rows.clear();
    specials.clear();
    players.clear();
    activation.clear();
    periods.clear();
    state.entity_phase_scratch = EntityPhaseScratch {
        rows,
        writes: ordered,
        specials,
        players,
        activation,
        periods,
        resting,
        coreperf,
        rest_proofs,
        arms,
    };
    let _ = tick;
    intents
}
