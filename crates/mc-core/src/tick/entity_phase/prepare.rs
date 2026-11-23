//! Row preparation and admission: the per-chunk serial and parallel passes
//! that turn admitted slots into `ReadRow`s. Byte-exact move out of
//! `entity_phase.rs`; admission order and the parallel/serial equivalence are
//! unchanged.

use super::*;

/// Live, non-converting resident bodies (villager, wandering trader) in slot
/// order. The resident index replaces a dense scan; slot order is the
/// `TargetGrid` tie-break, so it must match the dense order exactly.
pub(in crate::tick) fn resident_villager_targets(
    entities: &EntityStore,
) -> Vec<(EntityId, [f64; 3])> {
    let mut slots = Vec::new();
    entities.resident_slots_sorted(&mut slots);
    slots
        .into_iter()
        .filter(|&slot| {
            let body = entities.body(slot);
            body.alive
                && entities
                    .row(slot)
                    .villager
                    .as_ref()
                    .is_some_and(|v| v.conversion_time.is_none())
        })
        .map(|slot| (entities.body(slot).id, entities.pos(slot)))
        .collect()
}

/// Prepare every admitted slot (`periods[slot] != 0`) into `out`, in slot
/// order. The parallel path admits disjoint ascending slot ranges over an
/// immutable view (specials, resting and the slots needing rows concatenate in
/// chunk order, which reproduces the serial order), then builds every row
/// straight into `out.rows` with an indexed parallel collect: no per-chunk row
/// vectors and no row copy. Rows needing the `TickState`-only helpers are then
/// rebuilt serially before the combined projection reduction.
pub(in crate::tick) fn prepare_rows(
    state: &TickState,
    periods: &[u8],
    env: &PrepEnv,
    parallel: bool,
    out: &mut PrepChunk,
) {
    // `out` arrives empty (entity-phase scratch is empty between ticks; only
    // its capacity is reused): the parallel collect replaces `out.rows`.
    debug_assert!(out.rows.is_empty() && out.rare.is_empty());
    let mut laps = Laps::new(
        std::env::var_os("MC_CORE_PHASE_LOG").is_some(),
        std::time::Instant::now(),
    );
    let mut view = PrepView::of(state);
    view.rest_proofs = env.rest_proofs;
    laps.lap("view");
    if parallel {
        use rayon::prelude::*;
        let PrepChunk {
            specials,
            resting,
            parts,
            ..
        } = out;
        // The per-chunk admission vectors live in the phase scratch and are
        // reused tick to tick: one buffer set per chunk instead of three fresh
        // `Vec`s per chunk on every tick (about 2 k allocations per tick at
        // 1M slots). The concat order is still chunk order, i.e. slot order.
        // `resize_with` keeps the pooled `AdmissionPart`s (and their inner
        // capacity) across ticks; each is cleared for this tick.
        parts.resize_with(periods.len().div_ceil(PREP_CHUNK), AdmissionPart::default);
        for part in parts.iter_mut() {
            part.kept.clear();
            part.specials.clear();
            part.resting.clear();
        }
        periods
            .par_chunks(PREP_CHUNK)
            .zip(parts.par_iter_mut())
            .enumerate()
            .for_each(|(chunk, (periods, part))| {
                let base = chunk * PREP_CHUNK;
                for (offset, &period) in periods.iter().enumerate() {
                    if period != 0
                        && let Some(kept) = admit_slot(
                            &view,
                            base + offset,
                            period,
                            env,
                            &mut part.specials,
                            &mut part.resting,
                        )
                    {
                        part.kept.push(kept);
                    }
                }
            });
        laps.lap("admit_parallel");
        let mut kept = Vec::with_capacity(parts.iter().map(|part| part.kept.len()).sum());
        for part in parts.iter_mut() {
            kept.append(&mut part.kept);
            specials.append(&mut part.specials);
            resting.append(&mut part.resting);
        }
        laps.lap("admit_concat");
        let mut is_rare = Vec::new();
        kept.par_iter()
            .map(|&(slot, flags)| {
                read_row(
                    &view,
                    slot,
                    flags,
                    periods[slot.0 as usize],
                    &RarePrep::default(),
                )
            })
            .unzip_into_vecs(&mut out.rows, &mut is_rare);
        out.rare.extend(
            is_rare
                .iter()
                .enumerate()
                .filter_map(|(index, &rare): (usize, &bool)| rare.then_some(index)),
        );
        laps.lap("read_rows");
    } else {
        for (index, &period) in periods.iter().enumerate() {
            if period != 0 {
                prepare_slot(&view, index, period, env, out);
            }
        }
        laps.lap("serial_prepare");
    }
    for index in std::mem::take(&mut out.rare) {
        let slot = out.rows[index].slot;
        let flags = view.entities.motion_flags(slot);
        let period = periods[slot.0 as usize];
        out.rows[index] = read_row(&view, slot, flags, period, &RarePrep::of(state, slot)).0;
    }
    laps.lap("rare_fixup");
    // Rare helpers can change `chaser` (notably enderman anger), so project
    // only final rows. Chunk collection and concatenation retain row order.
    out.summary = if parallel {
        use rayon::prelude::*;
        let parts: Vec<_> = out
            .rows
            .par_chunks(PREP_CHUNK)
            .enumerate()
            .map(|(chunk, rows)| PrepSummary::of(rows, chunk * PREP_CHUNK))
            .collect();
        let mut summary = PrepSummary::default();
        for part in parts {
            summary.has_dolphin |= part.has_dolphin;
            summary.temptation_consumers += part.temptation_consumers;
            for (total, count) in summary.consumers.iter_mut().zip(part.consumers) {
                *total += count;
            }
            for (cell, indices) in part.cow_cells {
                summary.cow_cells.entry(cell).or_default().extend(indices);
            }
        }
        summary
    } else {
        PrepSummary::of(&out.rows, 0)
    };
    laps.lap("summary");
    laps.print("prepare rows");
}

/// Slot ranges larger than this admission count prepare rows in parallel.
pub(in crate::tick) const PARALLEL_PREP_MIN: usize = 4096;
pub(in crate::tick) const PREP_CHUNK: usize = 2048;

/// Read-only components row preparation needs. `TickState` owns channel
/// endpoints and is not `Sync`, so workers borrow only these.
pub(in crate::tick) struct PrepView<'a> {
    pub(in crate::tick) entities: &'a EntityStore,
    pub(in crate::tick) bodies: &'a [Body],
    pub(in crate::tick) positions: &'a [[f64; 3]],
    pub(in crate::tick) velocities: &'a [[f64; 3]],
    pub(in crate::tick) rotations: &'a [[f32; 3]],
    pub(in crate::tick) ground: &'a [bool],
    pub(in crate::tick) impulses: &'a [[f64; 2]],
    pub(in crate::tick) registry: &'a crate::registry::CoreRegistry,
    pub(in crate::tick) world: &'a crate::world::ChunkWorld,
    pub(in crate::tick) hostile: &'a crate::hostile::HostileState,
    pub(in crate::tick) tick: u64,
    pub(in crate::tick) has_tempting_items: bool,
    pub(in crate::tick) rest_proofs: Option<&'a RestProofs>,
}

impl<'a> PrepView<'a> {
    pub(in crate::tick) fn of(state: &'a TickState) -> Self {
        Self {
            entities: &state.entities,
            bodies: state.entities.bodies(),
            positions: state.entities.positions(),
            velocities: state.entities.velocities(),
            rotations: state.entities.rotations(),
            ground: state.entities.grounds(),
            impulses: state.entities.impulses(),
            registry: &state.registry,
            world: &state.world,
            hostile: &state.hostile,
            tick: state.tick,
            rest_proofs: None,
            has_tempting_items: state.players.values().any(|player| {
                !player.spectator()
                    && [Hand::Main, Hand::Off].into_iter().any(|hand| {
                        let held = &player.inventory.slots[held_slot_index(player, hand)];
                        !held.is_empty()
                            && state.registry.blueprints.iter().any(|(_, blueprint)| {
                                blueprint.capabilities.temptable
                                    && blueprint.breed_food.contains(&held.item_id)
                            })
                    })
            }),
        }
    }
}

/// Helper results that need the whole `TickState` (creaking observers,
/// enderman anger, plugin routes, piglin gold). Only rows passing the cheap
/// gates in `read_row` ever carry them; workers leave the default and the
/// serial fix-up pass rebuilds those rows.
#[derive(Default)]
pub(in crate::tick) struct RarePrep {
    pub(in crate::tick) observed: bool,
    pub(in crate::tick) enderman_target: Option<EntityId>,
    pub(in crate::tick) plugin_target: Option<[f64; 3]>,
    pub(in crate::tick) gold_target: Option<[f64; 3]>,
}

impl RarePrep {
    pub(in crate::tick) fn of(state: &TickState, slot: EntitySlot) -> Self {
        Self {
            observed: crate::creaking::observed(state, slot),
            enderman_target: crate::enderman::target(state, slot),
            plugin_target: crate::plugin_npcs::motion_target(state, slot),
            gold_target: crate::piglin::gold_target(state, slot).map(|(_, pos)| pos),
        }
    }
}

/// Prepared rows, serial special pass entries and resting slots of a slot
/// range, plus indexes (into `rows`) of rows that need the rare helpers.
#[derive(Default)]
pub(in crate::tick) struct PrepChunk {
    pub(in crate::tick) rows: Vec<ReadRow>,
    pub(in crate::tick) specials: Vec<(EntityId, crate::hostile::Special)>,
    pub(in crate::tick) resting: Vec<EntitySlot>,
    pub(in crate::tick) rare: Vec<usize>,
    pub(in crate::tick) summary: PrepSummary,
    /// One admission buffer set per [`PREP_CHUNK`] of the parallel pass,
    /// pooled in the phase scratch (kept after the concat).
    pub(in crate::tick) parts: Vec<AdmissionPart>,
}

/// Admission output of one [`PREP_CHUNK`] range, pooled across ticks.
#[cfg_attr(not(test), derive(Default))]
pub(in crate::tick) struct AdmissionPart {
    pub(in crate::tick) kept: Vec<(EntitySlot, u16)>,
    pub(in crate::tick) specials: Vec<(EntityId, crate::hostile::Special)>,
    pub(in crate::tick) resting: Vec<EntitySlot>,
}

#[cfg(test)]
static ADMISSION_PARTS_CONSTRUCTED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Test-only: how many [`AdmissionPart`]s were constructed since the last call.
/// The pooled pass constructs none on a repeated prepare; the pre-fix pass
/// built a fresh set per chunk per tick.
#[cfg(test)]
pub(in crate::tick) fn take_admission_parts_constructed() -> usize {
    ADMISSION_PARTS_CONSTRUCTED.swap(0, std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
impl Default for AdmissionPart {
    fn default() -> Self {
        ADMISSION_PARTS_CONSTRUCTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            kept: Vec::new(),
            specials: Vec::new(),
            resting: Vec::new(),
        }
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
pub(in crate::tick) struct PrepSummary {
    pub(in crate::tick) has_dolphin: bool,
    pub(in crate::tick) consumers: [usize; 5],
    pub(in crate::tick) temptation_consumers: usize,
    pub(in crate::tick) cow_cells: HashMap<(i32, i32), Vec<usize>>,
}

impl PrepSummary {
    pub(in crate::tick) fn of(rows: &[ReadRow], base: usize) -> Self {
        let mut summary = Self::default();
        for (offset, row) in rows.iter().enumerate() {
            summary.temptation_consumers += usize::from(row.temptable && row.alive && row.think);
            summary.has_dolphin |= row.movement == crate::blueprint::MovementClass::Swimmer
                && crate::dolphin::is_dolphin(row.type_id);
            if row.chaser && row.alive {
                if row.is_iron_golem {
                    summary.consumers[3] += 1;
                } else if crate::piglin::is_piglin_type(row.type_id) {
                    summary.consumers[1] += 1;
                } else {
                    summary.consumers[0] += 1;
                }
                if row.hunts_villagers {
                    summary.consumers[2] += 1;
                }
                if row.hunts_living {
                    summary.consumers[4] += 1;
                }
            }
            if row.breed_ready && row.alive {
                let cell = (
                    mc_physics::mth::floor_i32(row.pos[0]) / 4,
                    mc_physics::mth::floor_i32(row.pos[2]) / 4,
                );
                summary
                    .cow_cells
                    .entry(cell)
                    .or_default()
                    .push(base + offset);
            }
        }
        summary
    }
}

pub(in crate::tick) struct PrepEnv<'a> {
    pub(in crate::tick) targets_empty: bool,
    pub(in crate::tick) villagers_empty: bool,
    pub(in crate::tick) unchanged_since: u64,
    /// The rest-proof memo, or `None` to run every proof (tests, reference).
    pub(in crate::tick) rest_proofs: Option<&'a RestProofs>,
}

/// Admit one slot: record its serial special entry and either rest it
/// (`None`) or return the slot and its motion flags for a worker row.
/// Reads the immutable store only; the caller owns ordering.
pub(in crate::tick) fn admit_slot(
    view: &PrepView,
    index: usize,
    period: u8,
    env: &PrepEnv,
    specials: &mut Vec<(EntityId, crate::hostile::Special)>,
    resting: &mut Vec<EntitySlot>,
) -> Option<(EntitySlot, u16)> {
    let PrepEnv {
        targets_empty,
        villagers_empty,
        unchanged_since,
        rest_proofs: _,
    } = *env;
    let slot = EntitySlot(index as u32);
    let flags = view.entities.motion_flags(slot);
    let mut special_idle = true;
    if flags & MOTION_SPECIAL != 0 {
        let hint = view.entities.motion_hint(slot);
        if let Some(special) = crate::battle::special(view.bodies[index].faction, hint.special) {
            let id = view.bodies[slot.0 as usize].id;
            special_idle = targets_empty
                && !view.hostile.targets.contains_key(&id)
                && view
                    .hostile
                    .idle_without_targets(id, special, hint.swell_metadata_neutral);
            if !special_idle {
                specials.push((id, special));
            }
        }
    }
    if early_entity_rest(
        view,
        slot,
        period,
        unchanged_since,
        special_idle,
        targets_empty
            && (villagers_empty
                || !matches!(
                    view.bodies[index].kind,
                    EntityKind::Mob { type_id }
                        if view.registry.blueprint(type_id).is_some_and(|bp| bp.capabilities.hunts_villagers)
                )),
    ) {
        resting.push(slot);
        return None;
    }
    Some((slot, flags))
}

/// Serial admission: admit one slot and snapshot its worker row into `out`.
pub(in crate::tick) fn prepare_slot(
    view: &PrepView,
    index: usize,
    period: u8,
    env: &PrepEnv,
    out: &mut PrepChunk,
) {
    let Some((slot, flags)) = admit_slot(
        view,
        index,
        period,
        env,
        &mut out.specials,
        &mut out.resting,
    ) else {
        return;
    };
    let (row, rare) = read_row(view, slot, flags, period, &RarePrep::default());
    if rare {
        out.rare.push(out.rows.len());
    }
    out.rows.push(row);
}

/// The worker snapshot of one admitted slot, and whether the row needs the
/// `TickState`-only helpers in `RarePrep` (every helper below returns its
/// default when that is false).
pub(in crate::tick) fn read_row(
    view: &PrepView,
    slot: EntitySlot,
    flags: u16,
    period: u8,
    rare: &RarePrep,
) -> (ReadRow, bool) {
    let body = &view.bodies[slot.0 as usize];
    let battle =
        crate::battle::battle_row(body.faction) && matches!(body.kind, EntityKind::Mob { .. });
    let row = view.entities.row(slot);
    let observed = rare.observed;
    // Snapshot predicates share one immutable blueprint lookup per mob.
    let blueprint = match body.kind {
        EntityKind::Mob { type_id } => view.registry.blueprint(type_id),
        _ => None,
    };
    // A hop tick must be an AI decision tick even when the activation ring
    // would not decide: otherwise mid/far slimes hop only on decision phases
    // and some never line up at all. NoAI keeps the vanilla no-move-control
    // rule. `MagmaCube` multiplies the delay and lifts by its size.
    let hops = blueprint.is_some_and(|blueprint| blueprint.capabilities.hops);
    let hop_delay_scale =
        blueprint.map_or(1, |blueprint| blueprint.capabilities.hop_delay_scale.max(1));
    // `Slime$SlimeMoveControl.tick` divides the delay by three only while its
    // move control is aggressive: `SlimeAttackGoal` sets that when a target
    // exists and `isDealsDamage` holds (any magma size, slime size > 1;
    // `Slime.isDealsDamage`, `MagmaCube.canDealDamage`). An idle hopper keeps
    // the full delay.
    let has_retained_target = view.hostile.targets.contains_key(&body.id);
    let aggressive = hops
        && has_retained_target
        && blueprint.is_some_and(|blueprint| {
            blueprint.capabilities.hop_lift_per_size > 0.0 || row.loot.slime_size.unwrap_or(1) > 1
        });
    let hop_tick = hops
        && !row.controls.no_ai
        && crate::entity_motion::hop_due(body.id.0, view.tick, hop_delay_scale, aggressive);
    let think = battle || motion_thinks(view.tick, flags, period) || hop_tick;
    let needs_rare = crate::enderman::needs_state(row)
        || crate::plugin_npcs::needs_state(row)
        || crate::creaking::needs_state(body)
        || crate::piglin::eligible(body, row);
    let (chaser, breeder, item_entity, player, special, dark_aggro) = match body.kind {
        EntityKind::Mob { .. } => {
            let special = blueprint.and_then(crate::hostile::special_of);
            (
                blueprint.is_some_and(|blueprint| {
                    blueprint.has_goal(crate::blueprint::GoalKind::MeleeChase)
                }) || matches!(
                    special,
                    Some(crate::hostile::Special::Ranged | crate::hostile::Special::Swell)
                ),
                blueprint
                    .is_some_and(|blueprint| blueprint.has_goal(crate::blueprint::GoalKind::Breed)),
                false,
                false,
                special,
                blueprint.is_some_and(|blueprint| blueprint.capabilities.dark_aggro_only),
            )
        }
        EntityKind::Item { .. } => (false, false, true, false, None, false),
        EntityKind::ExperienceOrb { .. } => (false, false, false, false, None, false),
        EntityKind::Player { .. } => (false, false, false, true, None, false),
    };
    // Kind classification and scalar combat/defense inputs are snapshotted
    // here so `step_entity` never touches the cold row for the moved fields.
    let is_enderman = crate::enderman::is_enderman(body);
    let type_id = match body.kind {
        EntityKind::Mob { type_id }
        | EntityKind::Item { type_id }
        | EntityKind::ExperienceOrb { type_id } => type_id,
        EntityKind::Player { .. } => u32::MAX,
    };
    let is_iron_golem = blueprint.is_some_and(|bp| bp.capabilities.hunts_monsters);
    let is_slime_size_one = row.loot.slime_size == Some(1)
        && blueprint
            .is_some_and(|bp| bp.capabilities.size_profile == crate::blueprint::SizeProfile::Slime);
    let has_effects = flags & MOTION_EFFECTS != 0;
    let chaser = !battle
        && chaser
        && !row.controls.no_ai
        && (!matches!(body.kind, EntityKind::Mob { type_id } if crate::piglin::is_piglin_type(type_id))
            || crate::entity_life::adult(row))
        && (battle || !is_enderman || rare.enderman_target.is_some());
    let breeder = !battle && breeder && !row.controls.no_ai;
    let special = crate::battle::special(body.faction, special).filter(|_| !row.controls.no_ai);
    let panic_multiplier = if !battle
        && (body.burn_ticks > 0
            || body
                .hurt_at
                .is_some_and(|at| view.tick.saturating_sub(at) < 100))
        && blueprint
            .is_some_and(|blueprint| blueprint.has_goal(crate::blueprint::GoalKind::FleeFire))
    {
        if body.burn_ticks > 0 { 2.0 } else { 1.25 }
    } else {
        0.0
    };
    let impulse = view.impulses[slot.0 as usize];
    let (speed, melee_bonus, levitation, slow_falling) = if has_effects {
        let effects = &row.active_effects;
        (
            row.speed
                * ((1.0 + 0.2 * effects.level(crate::effects::EffectId::Speed))
                    * (1.0 - 0.15 * effects.level(crate::effects::EffectId::Slowness)).max(0.0)),
            effects.melee_bonus(),
            effects.level(crate::effects::EffectId::Levitation),
            effects.has(crate::effects::EffectId::SlowFalling),
        )
    } else {
        (row.speed, 0.0, 0.0, false)
    };
    // Panic source stays a rare-branch worker read (see `step_entity`).
    let read = ReadRow {
        think,
        battle,
        has_retained_target,
        retain_target: !battle && blueprint.is_some_and(crate::hostile::retains_target),
        enderman_target: rare.enderman_target,
        plugin_target: rare.plugin_target,
        gold_target: rare.gold_target,
        panic_multiplier,
        slot,
        id: body.id,
        pos: view.positions[slot.0 as usize],
        vel: view.velocities[slot.0 as usize],
        rotation: view.rotations[slot.0 as usize],
        movement: body.movement,
        air_supply: body.air_supply,
        horizontal_impulse: if observed { [0.0; 2] } else { impulse },
        alive: body.alive,
        ground: view.ground[slot.0 as usize],
        half_width: body.half_width,
        height: body.height,
        sight_range: row.sight_range,
        attack_reach: row.attack_reach,
        attack_damage: row.attack_damage,
        melee_bonus,
        attack_cooldown: body.attack_cooldown,
        love: row.love,
        is_enderman,
        is_iron_golem,
        hunts_villagers: blueprint.is_some_and(|bp| bp.capabilities.hunts_villagers),
        hunts_living: blueprint.is_some_and(|bp| bp.capabilities.hunts_living),
        is_slime_size_one,
        climbs_walls: blueprint.is_some_and(|blueprint| blueprint.capabilities.climbs_walls),
        head_follows_body: blueprint
            .is_some_and(|blueprint| blueprint.capabilities.head_follows_body),
        hops: hops && !row.controls.no_ai,
        hop_aggressive: aggressive,
        avoids_armadillos: blueprint
            .is_some_and(|blueprint| blueprint.capabilities.avoids_armadillos),
        aquatic_control: blueprint.map_or(crate::species::AquaticControl::Other, |blueprint| {
            blueprint.capabilities.aquatic
        }),
        chaser,
        breed_ready: breeder
            && crate::entity_life::can_breed_in(view.registry, view.entities, body, row)
            && row.entity_life.child_ticks == 0
            && row.breed_cooldown == 0
            && row.love > 0,
        temptable: blueprint.is_some_and(|blueprint| blueprint.capabilities.temptable),
        type_id,
        pickup_ready: item_entity && body.alive && body.pickup_delay == 0,
        item_entity,
        player,
        speed,
        levitation,
        slow_falling,
        no_gravity: row.controls.no_gravity,
        special,
        dark_aggro: !battle && dark_aggro,
        standing: (special == Some(crate::hostile::Special::Swell)
            && view.hostile.fuse(body.id) > 0)
            || matches!(row.special_mob.as_deref(), Some(crate::special_mobs::Retained::Wither { invulnerable }) if *invulnerable > 0)
            || (matches!(
                special,
                Some(crate::hostile::Special::Native(
                    crate::blueprint::GoalKind::Beam | crate::blueprint::GoalKind::ElderBeam
                ))
            ) && view
                .hostile
                .native
                .attacks
                .get(&body.id)
                .is_some_and(|attack| attack.stage > 0))
            || row.controls.no_ai
            || crate::axolotl::playing_dead(row)
            || observed
            || blueprint.is_some_and(|blueprint| blueprint.goals.is_empty())
            || (!battle && crate::taming::suppress_ai(view.entities, row))
            || crate::warden::suppress_ai(row)
            || crate::piglin::admiring(row),
        leash_follow: row.entity_life.leash.is_some()
            && !row.controls.no_ai
            && !row.entity_life.sitting
            && !crate::mounts::occupied(view.entities, &row.entity_life.mount),
    };
    (read, needs_rare)
}

/// Dense admission of the slots `base..base + marks.len()`: every owned,
/// non-player row in an active chunk gets its chunk's decision period;
/// player rows and unowned rows keep their mark. Returns the number admitted
/// and calls `admitted` for each, in slot order. Reads the store only; the
/// result of a slot depends on nothing but that slot, so disjoint ranges are
/// independent.
pub(super) fn dense_period_marks(
    entities: &EntityStore,
    world: &crate::world::ChunkWorld,
    activation: &HashMap<(i32, i32), u64>,
    base: usize,
    marks: &mut [u8],
    mut admitted: impl FnMut(EntitySlot),
) -> usize {
    let mut count = 0;
    let mut previous: Option<(ChunkPos, u8)> = None;
    for (offset, mark) in marks.iter_mut().enumerate() {
        let slot = EntitySlot((base + offset) as u32);
        let flags = entities.motion_flags(slot);
        if flags & MOTION_OWNED == 0 || flags & MOTION_PLAYER != 0 {
            continue;
        }
        let chunk = entities.chunk_of(slot);
        let period = match previous {
            Some((last, period)) if last == chunk => period,
            _ => {
                let period = if world.is_active_position(chunk.x, chunk.z) {
                    activation
                        .get(&(chunk.x, chunk.z))
                        .copied()
                        .unwrap_or(MOB_FAR_PERIOD) as u8
                } else {
                    0
                };
                previous = Some((chunk, period));
                period
            }
        };
        if period != 0 {
            *mark = period;
            count += 1;
            admitted(slot);
        }
    }
    count
}

/// [`dense_period_marks`] over every slot, split into [`PREP_CHUNK`] ranges
/// on the worker pool when `parallel`.
///
/// Bottleneck: the serial pass over all 100,000 slots cost 1.6 ms of every
/// native-100 tick before any row work started. Boundary: dense admission
/// only (the indexed path and the per-region perf attribution stay serial);
/// each range owns its disjoint marks. Correctness fence: every mark is a
/// function of its own slot (the per-range chunk cache only skips repeated
/// lookups), so any partition yields the serial marks and count; the
/// equivalence test pins it. Removal path: pass `parallel = false`.
pub(super) fn mark_dense_admission(
    entities: &EntityStore,
    world: &crate::world::ChunkWorld,
    activation: &HashMap<(i32, i32), u64>,
    periods: &mut [u8],
    parallel: bool,
) -> usize {
    if !parallel {
        return dense_period_marks(entities, world, activation, 0, periods, |_| {});
    }
    use rayon::prelude::*;
    periods
        .par_chunks_mut(PREP_CHUNK)
        .enumerate()
        .map(|(chunk, marks)| {
            dense_period_marks(
                entities,
                world,
                activation,
                chunk * PREP_CHUNK,
                marks,
                |_| {},
            )
        })
        .sum()
}

/// Keeps in `resting` only the slots whose writeback does anything, in slot
/// order; `resting` keeps its capacity (it is reused tick to tick).
///
/// Bottleneck: almost every resting row writes nothing (quiet timers, no
/// zombie arm change), yet the serial writeback read the body and cold row of
/// all of them (0.6 ms per native-100 tick). Boundary: the resting writeback
/// only; the filter is a pure read of the store and the target table, neither
/// of which the writeback changes before it reaches a slot. Correctness fence:
/// a dropped slot has quiet timers and `zombie_arms_update` is `None` for it,
/// which is exactly the case where the writeback does nothing; kept slots are
/// written serially in slot order. Removal path: write back every resting
/// slot (identical result).
pub(super) fn retain_resting_writeback(state: &TickState, resting: &mut Vec<EntitySlot>) {
    if resting.len() < PARALLEL_PREP_MIN {
        return;
    }
    use rayon::prelude::*;
    let (entities, registry, targets) = (&state.entities, &*state.registry, &state.hostile.targets);
    let rows: Vec<EntitySlot> = resting
        .par_iter()
        .copied()
        .filter(|&slot| {
            entities.motion_flags(slot) & crate::entities::MOTION_QUIET_TIMERS == 0
                || crate::hostile::zombie_arms_update(
                    entities,
                    registry,
                    slot,
                    targets.contains_key(&entities.body(slot).id),
                )
                .is_some()
        })
        .collect();
    resting.clear();
    resting.extend_from_slice(&rows);
}
