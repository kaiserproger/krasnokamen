//! One local world's tick phases: input intake, entity phase, merge, block
//! phase, commit and publish. Byte-exact move out of `tick.rs`; the dimension
//! coordinator still owns the global tick and the phase order is unchanged.

use super::*;

struct DimensionPhaseLog {
    start: std::time::Instant,
    before_return: std::time::Duration,
    logging: std::time::Duration,
    allocator_us: u64,
}

/// One local world's phases; the dimension coordinator owns the global tick.
pub(crate) fn run_dimension_tick<'a>(
    state: &mut TickState,
    inputs: impl IntoIterator<Item = &'a PlayerInput>,
) -> (
    TickOutput,
    Vec<(ConnectionId, Vec<PlayerChange>)>,
    TickTimes,
) {
    let mut phase_log =
        std::env::var_os("MC_CORE_PHASE_LOG")
            .is_some()
            .then(|| DimensionPhaseLog {
                start: std::time::Instant::now(),
                before_return: std::time::Duration::ZERO,
                logging: std::time::Duration::ZERO,
                allocator_us: 0,
            });
    let result = run_dimension_tick_inner(state, inputs, phase_log.as_mut());
    if let Some(log) = phase_log {
        // The inner call has returned, so this includes destruction of every
        // phase-local allocation, unlike the phase print inside that call.
        let total = log.start.elapsed();
        eprintln!(
            "dimension envelope ms: tick={} dimension={:?} total={:.2} return_cleanup={:.2} phase_log={:.2}",
            state.tick,
            state.dimension,
            total.as_secs_f64() * 1000.0,
            total
                .saturating_sub(log.before_return)
                .saturating_sub(log.logging)
                .as_secs_f64()
                * 1000.0,
            log.logging.as_secs_f64() * 1000.0,
        );
    }
    result
}

fn run_dimension_tick_inner<'a>(
    state: &mut TickState,
    inputs: impl IntoIterator<Item = &'a PlayerInput>,
    mut phase_log: Option<&mut DimensionPhaseLog>,
) -> (
    TickOutput,
    Vec<(ConnectionId, Vec<PlayerChange>)>,
    TickTimes,
) {
    let input_start = std::time::Instant::now();
    let mut input_laps = Laps::new(phase_log.is_some(), input_start);
    // Chunks already resident on entry must be validated before vibration and
    // input systems can hydrate or otherwise inspect their block entities.
    sync_chunk_block_entities(state);
    input_laps.lap("sync_block_entities");
    refresh_player_attributes(state, false);
    input_laps.lap("player_attributes");
    // False while `/tick freeze` holds the world: only players, input, chunk
    // loading and merge/commit run; see `tick_control`.
    let sim = state.tick_control.runs_normally();
    let previous_weather = crate::weather::view(state);
    for player in state.players.values_mut() {
        player.statistics.advance_tick(state.tick);
        player.chat_spam_ticks = player.chat_spam_ticks.saturating_sub(1);
    }
    input_laps.lap("player_counters");
    crate::chunk_tickets::refresh(state);
    state.refresh_active_chunks(state.tick + 1);
    input_laps.lap("tickets_active");
    // 1. Natural spawns/despawns, then inputs: player join/leave/move/dig/
    // attack/chat; intents staged.
    let mut intents: Vec<Intent> = Vec::new();
    let mut changes = ChangeSet::default();
    if sim {
        crate::datapacks::tick(state, &mut changes);
        input_laps.lap("datapacks");
    }
    if sim {
        crate::natural_spawn::advance_ages_and_run(state, &mut changes);
        input_laps.lap("natural_spawn");
        crate::mob_lanes::traders(state, &mut changes);
        input_laps.lap("traders");
        crate::vibration::tick(state);
        input_laps.lap("vibration");
        crate::plugin_npcs::advance(state);
        input_laps.lap("plugin_npcs");
    }
    changes.entities_spawned.append(&mut state.plugin_spawned);
    state
        .merge_block_writes
        .append(&mut state.plugin_block_writes);
    let mut deferred = std::mem::take(&mut state.deferred_uses);
    // MC-28289: snapshot each player's held main-hand stack BEFORE this
    // tick's inputs run, so the melee path resolves attack_damage /
    // attack_speed from the pre-tick item when a swap+attack share a tick.
    for player in state.players.values_mut() {
        let held = player
            .inventory
            .slots
            .get(usize::from(player.inventory.selected.min(8)))
            .cloned()
            .unwrap_or(mc_data::ItemStack::EMPTY);
        player.attribute_snapshot_held = Some(held);
    }
    input_laps.lap("held_snapshot");
    mining::advance(state);
    mining::take_finished(state, &mut deferred);
    input_laps.lap("mining");
    apply_inputs(state, &deferred, &mut intents, &mut changes);
    input_laps.lap("inputs_deferred");
    apply_inputs(state, inputs, &mut intents, &mut changes);
    input_laps.lap("inputs");
    state.refresh_active_chunks(state.tick + 1);
    crate::physical_attacks::advance(state, &mut intents);
    input_laps.lap("physical_attacks");
    if sim {
        crate::shrieker::run(state);
        input_laps.lap("shrieker");
    }
    tick_players(state, &mut changes, &mut intents);
    input_laps.lap("tick_players");
    if sim {
        crate::raids::raid_omens(state);
        input_laps.lap("raid_omens");
    }
    crate::effects::run(state, &mut intents);
    input_laps.lap("effects");
    let contact_times = crate::contact_hazards::run(state, &mut intents);
    input_laps.lap("contact_hazards");
    crate::dimensions::rules::void_intents(state, &mut intents);
    input_laps.lap("void_intents");
    crate::enchantment_locations::run(state, &mut intents);
    input_laps.lap("enchantment_locations");
    if sim {
        crate::end_fight::prepare(state);
        input_laps.lap("end_fight_prepare");
    }
    let input_time = input_start.elapsed();

    // Desired chunks from players + tickets; load/unload.
    let chunk_start = std::time::Instant::now();
    let mut chunk_laps = Laps::new(phase_log.is_some(), chunk_start);
    let want = state.desired_chunks();
    let loaded = state.world.ensure(&want);
    chunk_laps.lap("ensure");
    // A dragon egg's sampled target may have arrived with this admission; its
    // move is written through `merge_block_writes` for this tick's block phase.
    state.complete_pending_egg_teleports();
    chunk_laps.lap("egg_teleports");
    state.refresh_active_chunks(state.tick + 1);
    state.accumulate_inhabited_time();
    chunk_laps.lap("active_inhabited");
    // Inputs can mutate a resident-inactive container immediately before it
    // leaves demand. Fence its old time before eviction can forget authority.
    crate::abstract_sim::cancel_external_writes(state);
    for &(pos, _) in &state.merge_block_writes {
        state.world.cancel_simulation_debt(ChunkPos {
            x: pos.x.div_euclid(16),
            z: pos.z.div_euclid(16),
        });
    }
    chunk_laps.lap("cancel_external_writes");
    let unloaded = state.world.evict_outside(&want);
    chunk_laps.lap("evict_outside");
    state
        .block_ticks
        .evict_flushed(|x, z| state.world.unloaded_clean(x, z));
    state
        .fluid_ticks
        .evict_flushed(|x, z| state.world.unloaded_clean(x, z));
    chunk_laps.lap("evict_ticks");
    crate::fluids::hydrate(state, &loaded);
    chunk_laps.lap("fluids_hydrate");
    crate::villager::generated::load(state, &loaded, &mut changes);
    chunk_laps.lap("villager_generated");
    changes.chunks_loaded.extend(loaded);
    changes.chunks_unloaded.extend(unloaded);
    // Block entities ride inside chunk regions: pull newly resident chunks'
    // typed maps into live storage (journal folds apply over them).
    sync_chunk_block_entities(state);
    chunk_laps.lap("sync_block_entities");
    if sim {
        crate::settlements::autonomy::tick(state, &mut changes);
        chunk_laps.lap("settlement_autonomy");
        crate::conduit::run(state, &mut intents);
        chunk_laps.lap("conduit");
        crate::beacon::run(state);
        chunk_laps.lap("beacon");
        crate::dolphin::run(state);
        chunk_laps.lap("dolphin");
    }
    // Immediate respawn retries after off-tick chunk loading; unavailable
    // personal spawn chunks must never turn into a fallback-world respawn.
    if crate::command_rules::boolean(state, crate::command_rules::BoolRule::IMMEDIATE_RESPAWN) {
        for index in 0..state.entities.len() as u32 {
            let body = state.entities.body(EntitySlot(index));
            if !body.alive
                && let EntityKind::Player { connection } = body.kind
            {
                respawn_player(state, connection, &mut changes);
            }
        }
    }
    chunk_laps.lap("immediate_respawn");
    let chunk_time = chunk_start.elapsed();

    // 2. Entity phase (parallel by data over the fixed pool).
    let entity_start = std::time::Instant::now();
    let mut entity_laps = Laps::new(phase_log.is_some(), entity_start);
    let mut entity_life_time = std::time::Duration::ZERO;
    if sim {
        dying::tick(state, &mut changes);
        crate::entity_life::advance(state);
        entity_laps.lap("entity_life");
        entity_life_time = entity_start.elapsed();
        crate::brushing::run(state);
        entity_laps.lap("brushing");
        crate::axolotl::run(state);
        entity_laps.lap("axolotl");
        crate::turtle::run(state);
        entity_laps.lap("turtle");
        crate::frog::run(state);
        entity_laps.lap("frog");
        crate::sniffer::run(state);
        entity_laps.lap("sniffer");
        crate::strider::run(state);
        entity_laps.lap("strider");
        crate::parrot::run(state);
        entity_laps.lap("parrot");
        crate::goat::run(state, &mut intents, &mut changes);
        entity_laps.lap("goat");
        crate::piglin::run(state, &mut changes);
        entity_laps.lap("piglin");
        crate::llama::run(state);
        entity_laps.lap("llama");
        crate::tnt::run(state, &mut intents);
        entity_laps.lap("tnt");
        crate::mob_lanes::snowballs(state);
        entity_laps.lap("snowballs");
    }
    let entity_species_time = entity_start.elapsed().saturating_sub(entity_life_time);
    let entity_intents = run_entity_phase(state, sim);
    entity_laps.lap("run_entity_phase");
    let mut coreperf = state.entity_phase_scratch.coreperf.take();
    let entity_time = entity_start.elapsed();
    let entity_tail_start = std::time::Instant::now();
    intents.extend(entity_intents);
    entity_laps.lap("collect_intents");
    if sim {
        crate::special_mobs::raiders::run(state, &mut intents);
        entity_laps.lap("raiders");
    }
    crate::vehicles::run(state, &mut intents, &mut changes);
    entity_laps.lap("vehicles");
    crate::mounts::menus::maintain(state, &mut changes);
    entity_laps.lap("mount_menus");
    if sim {
        crate::end_fight::run(state, &mut intents, &mut changes);
        entity_laps.lap("end_fight");
        crate::villager::soldier::attacks(state, &mut intents, &mut changes);
        entity_laps.lap("soldier_attacks");
    }
    let entity_tail_time = entity_tail_start.elapsed();

    // 3. Merge (deterministic, fixed order, whole-intent rejection).
    let merge_start = std::time::Instant::now();
    state.merge(&mut intents);
    state.merge_damage.append(&mut state.plugin_damage);
    crate::end_fight::crystal_hits(state);
    let merge_time = merge_start.elapsed();
    // Beds follow their sleeper (vanilla `LivingEntity.startSleeping`): derived
    // after the merge, so an accepted write to a bed this tick wins over the
    // mirror (a broken bed is never re-occupied), and before the block phase,
    // so the mirror's own write lands in this tick.
    crate::sleep::mirror_occupied_beds(state);
    let block_prepare_start = std::time::Instant::now();

    // 4. Block phase: ordered merge writes, retained falling lifecycle.
    let mut block_writes = std::mem::take(&mut state.block_phase_writes);
    block_writes.extend_from_slice(&state.merge_block_writes);
    let early_merge_writes = state.merge_block_writes.len();
    let block_prepare_time = block_prepare_start.elapsed();
    let block_start = std::time::Instant::now();
    let mut block_laps = Laps::new(phase_log.is_some(), block_start);
    for (pos, write_state) in &block_writes {
        state.world.set_block(pos.x, pos.y, pos.z, *write_state);
    }
    block_laps.lap("apply_writes");
    // Single merge owner: accepted writes win over old chunk time. Catch-up
    // writes are appended only after normal staged writes have been applied.
    crate::abstract_sim::cancel_external_writes(state);
    crate::abstract_sim::run(state, &mut block_writes);
    block_laps.lap("abstract_sim");
    // Attached blocks whose support went away pop off with their loot.
    let mut popped_drops = Vec::new();
    pop_unsupported(state, &mut block_writes, &mut popped_drops);
    block_laps.lap("pop_unsupported");
    containers::expire_native_openers(state);
    if sim {
        crate::weather::settle(state, &mut block_writes);
        block_laps.lap("weather_settle");
        // Random ticks: crops, farmland, saplings, grass.
        crate::farming::run(state, &mut block_writes, &mut popped_drops);
        block_laps.lap("farming");
        // Due water/lava ticks, then schedule the fluid around every write.
        crate::fluids::run(state, &mut block_writes, &mut popped_drops);
        block_laps.lap("fluids_run");
        // Furnace cooking over live block-entity storage (input/fuel/output).
        tick_furnaces(state, &mut block_writes, None);
        block_laps.lap("furnaces");
        crate::sculk::run(state, &mut block_writes);
        block_laps.lap("sculk");
        crate::crafter::advance(state, &mut block_writes);
        block_laps.lap("crafter_advance");
        crate::redstone::run(state, &mut block_writes, &mut popped_drops);
        block_laps.lap("redstone");
        crate::command_blocks::run(state, &mut changes);
        block_laps.lap("command_blocks");
        crate::jukebox::run(state, &mut block_writes, &mut popped_drops);
        block_laps.lap("jukebox");
        if let Some(sample) = &mut coreperf {
            crate::brewing::run_counted(
                state,
                &mut block_writes,
                &mut popped_drops,
                Arc::get_mut(sample).expect("coreperf sample is not shared before publication"),
            );
        } else {
            crate::brewing::run(state, &mut block_writes, &mut popped_drops);
        }
        block_laps.lap("brewing");
        crate::crafter::run(state);
        block_laps.lap("crafter_run");
    }
    crate::shelf::run(state);
    block_laps.lap("shelf");
    crate::shulker::run(state);
    block_laps.lap("shulker");
    // Scheduled ignition and dispenser actions prime after the early merge
    // writes were copied. Their air post-images must commit in this same tick.
    for &(pos, write_state) in &state.merge_block_writes[early_merge_writes..] {
        state.world.set_block(pos.x, pos.y, pos.z, write_state);
        block_writes.push((pos, write_state));
    }
    block_laps.lap("late_writes");
    blocks::run(
        state,
        &mut block_writes,
        &mut popped_drops,
        &mut changes.entities_removed,
    );
    block_laps.lap("blocks_run");
    if sim {
        crate::special_mobs::after_blocks(state, &mut block_writes);
        block_laps.lap("special_mobs");
        crate::bees::run(state, &mut block_writes, &mut changes);
        block_laps.lap("bees");
        crate::armadillo::run(state);
        block_laps.lap("armadillo");
    }
    if sim {
        crate::copper_golem::after_blocks(state, &mut block_writes);
        block_laps.lap("copper_golem_after");
    }
    crate::golems::after_blocks(state, &mut block_writes);
    block_laps.lap("golems");
    if sim {
        crate::copper_golem::tick(state, &mut block_writes);
        block_laps.lap("copper_golem_tick");
        crate::mob_lanes::snow_trails(state, &mut block_writes);
        block_laps.lap("snow_trails");
    }
    crate::portals::nether::after_blocks(state, &mut block_writes);
    block_laps.lap("portals_nether");
    crate::sponge::after_blocks(state, &mut block_writes, &mut popped_drops);
    block_laps.lap("sponge");
    crate::fluids::schedule(state, &block_writes);
    block_laps.lap("fluid_schedule");
    let block_time = block_start.elapsed();

    // 5. Commit: fold blocks, apply entity effects.
    let commit_start = std::time::Instant::now();
    let mut commit_laps = Laps::new(phase_log.is_some(), commit_start);
    state.world.fold_overlay();
    commit_laps.lap("fold_overlay");
    crate::shrieker::after_blocks(state, &block_writes);
    crate::end_portals::after_blocks(state, &block_writes);
    crate::conduit::after_blocks(state, &block_writes);
    crate::campfire::run(state, &mut popped_drops);
    commit_laps.lap("after_blocks_hooks");
    let commit_write_start = state.merge_block_writes.len();
    #[cfg(test)]
    let effects_start = crate::horde_bench::merge_cost_tests::start();
    let (deaths, mut events, spawned_drops, committed_removals) =
        if let Some(sample) = &mut coreperf {
            let sample =
                Arc::get_mut(sample).expect("coreperf sample is not shared before publication");
            state.commit_merge_effects_observed(Some(&mut |state, slot, observation| {
                entity_phase::coreperf_damage_observation(sample, state, slot, observation);
            }))
        } else {
            state.commit_merge_effects()
        };
    commit_laps.lap("commit_merge_effects");
    #[cfg(test)]
    crate::horde_bench::merge_cost_tests::record(effects_start, |probe, ns| {
        probe.effects_ns += ns;
    });
    block_writes.extend_from_slice(&state.merge_block_writes[commit_write_start..]);
    if state.merge_block_writes.len() != commit_write_start {
        state.world.fold_overlay();
    }
    commit_laps.lap("writes_fold");
    crate::shulker::commit_placements(state);
    crate::block_entity_defaults::after_blocks(state, &block_writes);
    crate::trial_spawner::after_blocks(state, &block_writes);
    crate::vault::after_blocks(state, &block_writes);
    post_write_consequences(state, &block_writes, &mut popped_drops);
    crate::block_log::after_blocks(state);
    commit_laps.lap("post_write");
    if sim {
        crate::trial_spawner::run(state, &mut block_writes, &mut changes);
        crate::monster_spawner::run(state, &mut changes);
        crate::vault::run(state, &mut block_writes, &mut changes);
    }
    commit_laps.lap("spawners_vault");
    changes.entities_spawned.extend(spawned_drops);
    changes.entities_spawned.extend(popped_drops);
    changes.entities_spawned.append(&mut state.hostile.spawned);
    changes
        .entities_spawned
        .append(&mut state.projectiles.spawned);
    if sim {
        if let Some(sample) = &mut coreperf {
            crate::experience_orbs::run_counted(
                state,
                &mut changes,
                &mut events,
                Arc::get_mut(sample).expect("coreperf sample is not shared before publication"),
            );
        } else {
            crate::experience_orbs::run(state, &mut changes, &mut events);
        }
    }
    commit_laps.lap("experience_orbs");
    changes.explosions.append(&mut state.hostile.explosions);
    events.append(&mut state.hostile.events);
    changes.entities_removed.append(&mut state.hostile.removed);
    changes
        .entities_removed
        .append(&mut state.projectiles.removed);
    // Committed full-pickup removals journal their tombstones in this same
    // tick (durable pickup: credit + tombstone share one batch).
    changes.entities_removed.extend(committed_removals);
    state.merge_block_writes.clear();
    if sim {
        crate::bees::commit(state, &mut changes);
        crate::villager::conversion::tick(state);
    }
    commit_laps.lap("bees_conversion");
    // Settlements allocator: sole POI authority, immediately before the brain,
    // sharing the global tick budget with its own telemetry phase.
    let allocator_us = if sim {
        let allocator_us = crate::settlements::tick(state, &mut changes);
        commit_laps.lap("settlements");
        let crop_start = coreperf.as_ref().map(|_| changes.blocks.len());
        crate::villager::brain::tick(state, &mut changes);
        commit_laps.lap("villager_brain");
        if let (Some(sample), Some(crop_start)) = (&mut coreperf, crop_start) {
            let sample =
                Arc::get_mut(sample).expect("coreperf sample is not shared before publication");
            entity_phase::coreperf_productive_changes(&changes.blocks[crop_start..], sample);
        }
        allocator_us
    } else {
        0
    };
    if let Some(log) = phase_log.as_mut() {
        log.allocator_us += allocator_us;
    }
    if sim {
        merchant::maintain(state, &mut changes);
        commit_laps.lap("merchant");
        crate::mounts::tick(state, &mut changes);
        commit_laps.lap("mounts");
        crate::allay::run(state, &mut changes);
        commit_laps.lap("allay");
        crate::mob_equipment_pickup::run(state, &mut changes);
        commit_laps.lap("equipment_pickup");
    }
    crate::entity_life::commit_drops(state, &mut changes.entities_spawned);
    commit_laps.lap("commit_drops");
    if sim {
        crate::end_fight::commit(state, &mut changes);
        crate::raids::run(state, &mut changes);
    }
    commit_laps.lap("end_fight_raids");

    // NOTE: Health(0) + Died emit inside commit_merge_effects (death-screen
    // order, same tick as the drops) — no second emission here; commit is
    // the single owner of the death outbox sequence.
    if crate::command_rules::boolean(state, crate::command_rules::BoolRule::IMMEDIATE_RESPAWN) {
        for entity in deaths {
            if let Some(connection) = state.connections.entity_connection(entity) {
                respawn_player(state, connection, &mut changes);
            }
        }
    }
    commit_laps.lap("respawn");

    state.tick += 1;
    crate::statistics::tick(state, state.tick);
    commit_laps.lap("statistics");
    if sim {
        crate::end_portals::tick(state);
    }
    commit_laps.lap("end_portals");
    if crate::tick_control::advances_day(state) {
        state.day_time = state.day_time.wrapping_add(1);
    }
    if !sim {
        // Frozen: the weather clock, sleep and fades hold with the world.
    } else if state.dimension.has_weather() {
        crate::weather::advance_cycle(state);
        crate::sleep::run(state);
        // Sleep consumes the cat's gift state; spawn its item in the same journal batch.
        crate::entity_life::commit_drops(state, &mut changes.entities_spawned);
        crate::weather::fade_and_publish(state, previous_weather);
    } else {
        state.weather = (crate::commands::Weather::Clear, 0);
        state.rain_level = 0.0;
        state.thunder_level = 0.0;
    }
    commit_laps.lap("weather_sleep");
    // Vanilla `WorldBorder.tick` advances the interpolation extent every
    // game tick, independent of weather sleep or dimension.
    if sim {
        state.border.advance_tick();
    }
    crate::scoreboard::automatic(state, &[], &[]);
    crate::maps::tick(state);
    crate::command_locate::drain(state);
    commit_laps.lap("scoreboard_maps_locate");
    changes
        .blocks
        .extend(
            block_writes
                .iter()
                .map(|(pos, write_state)| crate::boundary::BlockChange {
                    position: *pos,
                    state: *write_state,
                }),
        );
    block_writes.clear();
    state.block_phase_writes = block_writes;
    commit_laps.lap("changes_blocks");
    // Player inventory is the equipment authority. Project changes through
    // the same appearance path used by all living entities/SetEquipment.
    for (&entity, player) in &state.players {
        let Some(slot) = state.entities.slot_of(entity) else {
            continue;
        };
        let slots = [
            usize::from(player.inventory.selected.min(8)),
            crate::inventory::OFF_HAND,
            36,
            37,
            38,
            39,
        ];
        let row = state.entities.row_mut(slot);
        let mut changed = false;
        for (index, inventory_slot) in slots.into_iter().enumerate() {
            let stack = &player.inventory.slots[inventory_slot];
            let expected = (!stack.is_empty()).then_some(stack);
            if row.appearance.equipment[index].as_ref() != expected {
                Arc::make_mut(&mut row.appearance).equipment[index] = expected.cloned();
                changed = true;
            }
        }
        if changed {
            state.appearance_changed.push(entity);
        }
    }
    commit_laps.lap("equipment_projection");
    // Effects, commands, sprint changes and joins explicitly invalidate the
    // cache during this tick; publish those without rescanning clean players.
    refresh_player_attributes(state, true);
    commit_laps.lap("player_attributes");
    changes.entity_events.append(&mut events);
    changes
        .entities_appearance_changed
        .append(&mut state.appearance_changed);

    crate::advancements::run(state, &mut changes);
    commit_laps.lap("advancements");
    changes.entities_spawned.append(&mut state.plugin_spawned);
    let plugin_events = crate::plugin_player::tick(state);
    if let Some(runtime) = &mut state.plugin_runtime {
        runtime.pending_events.extend(plugin_events);
    }
    commit_laps.lap("plugin_player");
    spectator::tick(state);
    containers::animate_openers(state);
    commit_laps.lap("container_animations");
    let commit_time = commit_start.elapsed();
    // 6. Publish: chunk list + entity wire views + per-connection outbox
    // drain. Readers hold the snapshot `Arc`; unchanged chunks share theirs.
    let publish_start = std::time::Instant::now();
    // Native commit-tail owners (including villager work and doors) can write
    // after the main block fold. Publish their actual chunk afterimages, not
    // an older chunk beside newer block deltas.
    // Canonical structure work may run in the resident commit tail, after
    // the early plugin-write drain. Seal those writes with debit/cursor now,
    // using final owner post-images (a door/shape may have evolved meanwhile).
    changes
        .blocks
        .extend(state.plugin_block_writes.drain(..).map(|(position, _)| {
            crate::boundary::BlockChange {
                position,
                state: state
                    .world
                    .block_at(position.x, position.y, position.z)
                    .expect("committed native block remains loaded through tick seal"),
            }
        }));
    state.world.fold_overlay();
    let chunks = state.world.snapshot();
    let snapshot_time = publish_start.elapsed();
    let views_start = std::time::Instant::now();
    let views = crate::battle::drain_entity_views(state);
    let views_time = views_start.elapsed();
    let snapshot = Arc::new(WorldSnapshot {
        tick: state.tick,
        dimension: state.dimension,
        dimensions: Default::default(),
        player_knockback_ticks: Default::default(),
        game_time: state.tick_control.game_ticks(state.tick),
        day_time: state.day_time,
        day_time_rate: if crate::command_rules::boolean(
            state,
            crate::command_rules::BoolRule::ADVANCE_TIME,
        ) {
            1.0
        } else {
            0.0
        },
        raining: state.weather.0 != crate::commands::Weather::Clear,
        rain_level: state.rain_level,
        thunder_level: state.thunder_level,
        border: state.border,
        chunks,
        entities: views,
        raid_bars: crate::raids::bars(state),
        settlement_labels: crate::settlements::presentation::snapshot(state),
        player_factions: crate::battle::live_players::player_factions(state),
        cameras: Arc::clone(&state.cameras),
        player_roster: Arc::clone(&state.dimension_globals.roster),
        team_names: crate::scoreboard::snapshot_team_names(state),
    });
    let output = TickOutput {
        tick: state.tick,
        snapshot,
        changes: Arc::new(changes),
        times: TickTimes::default(),
    };
    let mut drains = Vec::new();
    let connections: Vec<ConnectionId> = state.outbox.keys().copied().collect();
    for connection in connections {
        if let Some(queue) = state.outbox.remove(&connection)
            && !queue.is_empty()
        {
            drains.push((connection, queue));
        }
    }
    let publish_tail_time = publish_start
        .elapsed()
        .saturating_sub(snapshot_time)
        .saturating_sub(views_time);
    if let Some(log) = phase_log {
        log.before_return = log.start.elapsed();
        let measured = input_time
            + chunk_time
            + entity_time
            + entity_tail_time
            + merge_time
            + block_prepare_time
            + block_time
            + commit_time
            + snapshot_time
            + views_time
            + publish_tail_time;
        let log_start = std::time::Instant::now();
        input_laps.print("input");
        chunk_laps.print("chunks");
        entity_laps.print("entity systems");
        block_laps.print("block");
        commit_laps.print("commit");
        let mut contact_laps = Laps::new(true, log.start);
        contact_laps.record("classify", contact_times.classify);
        contact_laps.record("serial", contact_times.serial);
        contact_laps.record("cauldron", contact_times.cauldron);
        contact_laps.print("contact hazards");
        let mut summary = Laps::new(true, log.start);
        summary.record("input", input_time);
        summary.record("chunks", chunk_time);
        summary.record("entity", entity_time);
        summary.record("merge", merge_time);
        summary.record("block", block_time);
        summary.record("commit", commit_time);
        summary.record("snapshot", snapshot_time);
        summary.record("views", views_time);
        summary.record("entity_tail", entity_tail_time);
        summary.record("block_prepare", block_prepare_time);
        summary.record("publish_tail", publish_tail_time);
        summary.record("boundary_gaps", log.before_return.saturating_sub(measured));
        summary.record("before_return", log.before_return);
        summary.record(
            "allocator",
            std::time::Duration::from_micros(log.allocator_us),
        );
        summary.record("entity_life", entity_life_time);
        summary.record("entity_species", entity_species_time);
        summary.print_summary(
            "phase",
            format_args!(
                "tick={} intents={} dimension={:?}",
                state.tick,
                intents.len(),
                state.dimension,
            ),
        );
        log.logging = log_start.elapsed();
    }
    let battle_counts_start = std::time::Instant::now();
    let battle = crate::battle::publication_counts(state);
    let battle_counts_time = battle_counts_start.elapsed();
    let times = TickTimes {
        input_us: input_time.as_micros() as u64,
        chunks_us: chunk_time.as_micros() as u64,
        entity_us: (entity_time + entity_tail_time).as_micros() as u64,
        merge_us: merge_time.as_micros() as u64,
        block_us: (block_prepare_time + block_time).as_micros() as u64,
        commit_us: commit_time.as_micros() as u64,
        snapshot_us: snapshot_time.as_micros() as u64,
        views_us: views_time.as_micros() as u64,
        publish_us: (publish_tail_time + battle_counts_time).as_micros() as u64,
        coreperf,
        battle,
        total_us: 0,
        ..Default::default()
    };
    (output, drains, times)
}
