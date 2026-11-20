//! Explicit legacy-clock conversion bridge; the offline tool owns the lease.
use super::*;
use crate::weather::GameClockImage;
use mc_world::native::{NativeCheckpoint, NativeError, NativeStore};

#[derive(Debug, Default, serde::Serialize)]
pub struct Conversion {
    pub checkpoint_tick: u64,
    pub tail_held_ticks: u64,
    pub tick: u64,
    pub held_ticks: u64,
    pub held_since: Option<u64>,
    pub active_debts: usize,
    pub inactive_debts: usize,
    pub dropped_debt: u64,
    pub debt: Vec<DroppedDebt>,
    pub work_normalized: usize,
    pub work_raw: usize,
    pub sleep_normalized: usize,
    pub sleep_raw: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct DroppedDebt {
    dimension: Dimension,
    chunk: [i32; 2],
    dropped: u64,
}

fn storage(error: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(format!("game-clock conversion: {error}"))
}
fn native(error: impl std::fmt::Display) -> NativeError {
    NativeError::Source(error.to_string())
}

fn normalize_villager(
    state: &mut crate::villager::VillagerState,
    game: u64,
    report: &mut Conversion,
) -> Result<(), CoreError> {
    if game == 0 || !state.brain_clock || state.wandering || state.conversion_time.is_some() {
        report.work_raw += 1;
        report.sleep_raw += 1;
        return Ok(());
    }
    let running = report
        .tick
        .checked_sub(1)
        .ok_or_else(|| storage("hold at tick zero"))?;
    let lower = game.saturating_sub(1);
    let work = i64::try_from(running.saturating_sub(1)).map_err(storage)?;
    if state.activity == crate::villager::Activity::Work {
        if state.last_work >= 0 && state.last_work as u64 > running {
            return Err(storage(
                "eligible last_work exceeds the final persisted running boundary",
            ));
        }
        if state.last_work >= 0 && state.last_work as u64 >= lower {
            state.last_work = work;
            report.work_normalized += 1;
        } else {
            report.work_raw += 1;
        }
    } else {
        report.work_raw += 1;
    }
    let sleep = running.checked_sub(1);
    if state.sleeping
        && let Some(value) = state.last_slept
    {
        if value > running {
            return Err(storage(
                "eligible last_slept exceeds the final persisted running boundary",
            ));
        }
        if value >= lower {
            state.last_slept = sleep;
            report.sleep_normalized += 1;
        } else {
            report.sleep_raw += 1;
        }
    } else {
        report.sleep_raw += 1;
    }
    // panic_until and conversion countdowns deliberately stay raw.
    Ok(())
}

fn convert_images(world: &mut RecoveredWorld) -> Result<Conversion, CoreError> {
    let root = &world.dimensions[&Dimension::Overworld];
    let tick = root.tick;
    let climate = root
        .climate
        .ok_or_else(|| storage("legacy world has no climate image"))?;
    if climate.sim_debt_anchored_since.is_some() {
        return Err(storage("world already has a simulation-debt anchor epoch"));
    }
    let (held_ticks, held_since, game) = match climate.game_clock {
        GameClockImage::Running { held_ticks } if held_ticks <= tick => {
            (held_ticks, None, tick - held_ticks)
        }
        GameClockImage::Held { game_ticks, .. } if tick > 0 && game_ticks < tick => {
            (tick - game_ticks, Some(tick), game_ticks)
        }
        _ => return Err(storage("invalid legacy game clock")),
    };
    let checkpoint_tick = world.manifest.tick;
    let checkpoint_held = match world.manifest.climate.map(|image| image.game_clock) {
        Some(GameClockImage::Held { game_ticks, .. }) => checkpoint_tick
            .checked_sub(game_ticks)
            .ok_or_else(|| storage("invalid checkpoint game clock"))?,
        Some(GameClockImage::Running { held_ticks }) => held_ticks,
        None => 0,
    };
    let tail_held_ticks = held_ticks
        .checked_sub(checkpoint_held)
        .ok_or_else(|| storage("held prefix decreased while folding the legacy tail"))?;
    let mut report = Conversion {
        checkpoint_tick,
        tail_held_ticks,
        tick,
        held_ticks,
        held_since,
        ..Default::default()
    };
    for (&dimension, replay) in &mut world.dimensions {
        replay.recovered_held_ticks = held_ticks;
        replay.sim_debt_anchored_since = Some(tick);
        let image = replay
            .climate
            .as_mut()
            .ok_or_else(|| storage("missing dimension climate"))?;
        image.sim_debt_anchored_since = Some(tick);
        if held_since.is_some() {
            image.game_clock = GameClockImage::Held {
                game_ticks: game,
                held_since,
            };
            for entity in replay.entities.values_mut() {
                if let Some(villager) = &mut entity.villager {
                    normalize_villager(villager, game, &mut report)?;
                }
            }
        }
        for debt in replay.sim_debt.values_mut() {
            if debt.last_sim > tick {
                return Err(storage("simulation-debt boundary is in the future"));
            }
            if debt.active {
                report.active_debts += 1;
                debt.last_sim = tick;
                debt.held_anchor = Some(held_ticks);
            } else {
                report.inactive_debts += 1;
                debt.held_anchor = Some(held_ticks.saturating_sub(tick - debt.last_sim));
            }
            let remaining = debt.remaining.saturating_sub(held_ticks);
            let dropped = debt.remaining - remaining;
            report.dropped_debt = report
                .dropped_debt
                .checked_add(dropped)
                .ok_or_else(|| storage("dropped-debt total overflow"))?;
            report.debt.push(DroppedDebt {
                dimension,
                chunk: debt.chunk,
                dropped,
            });
            debt.remaining = remaining;
            debt.tick = tick;
            crate::debt::validate_anchor(
                mc_world::ChunkPos {
                    x: debt.chunk[0],
                    z: debt.chunk[1],
                },
                debt.last_sim,
                debt.held_anchor,
                tick,
                held_ticks,
            )?;
        }
    }
    Ok(report)
}

/// Fold the legacy clock exactly, convert its recovered boundary, then atomically write
/// all domains at that boundary. Only the copy-only world tool calls this.
/// The original journal is archived by the tool AFTER this checkpoint commits.
pub fn write_converted_checkpoint(
    config: &CoreConfig,
    store: Arc<NativeStore>,
) -> Result<Conversion, CoreError> {
    let registry = Arc::new(recovery_registry(config)?);
    let reader = store.checkpoint_reader().map_err(storage)?;
    let mut world = recover_with_clock(config, registry, reader, true)?;
    // Cold native-only authorities are not present in the manifest. Scan once,
    // adopting only records not superseded by the recovered manifest/WAL.
    world
        .reader
        .visit_chunk_positions(|id, pos| {
            let dimension = match id {
                0 => Dimension::Overworld,
                1 => Dimension::Nether,
                2 => Dimension::End,
                _ => return Err(native("invalid dimension")),
            };
            let replay = world
                .dimensions
                .get_mut(&dimension)
                .expect("three dimensions");
            if replay.sim_debt.contains_key(&[pos.x, pos.z]) {
                return Ok(());
            }
            let chunk = world
                .reader
                .read_chunk(
                    id,
                    pos,
                    &world.registry.blocks,
                    &world.registry.items,
                    &world.registry.chest_loot,
                )?
                .ok_or_else(|| native("stored chunk has no baseline"))?;
            let mut records = rustc_hash::FxHashMap::default();
            crate::debt::adopt_native_extras(pos, &Arc::new(chunk), &mut records, replay.tick, 0)
                .map_err(native)?;
            if let Some(record) = records.get(&pos) {
                replay.sim_debt.insert(
                    [pos.x, pos.z],
                    crate::journal::SimDebtImage {
                        chunk: [pos.x, pos.z],
                        last_sim: record.last_sim,
                        held_anchor: None,
                        remaining: record.remaining,
                        active: false,
                        tick: replay.tick,
                    },
                );
            }
            Ok(())
        })
        .map_err(storage)?;
    let report = convert_images(&mut world)?;
    let mut image = encode_checkpoint(&world)?;
    let root = &world.dimensions[&Dimension::Overworld];
    image.globals.insert(
        "plugins".into(),
        record(
            report.tick,
            serde_json::to_vec(&root.plugins).map_err(storage)?,
        ),
    );
    let mut extras = decode_tag(&image.globals["conversion_extras"], "conversion_extras")?;
    let Tag::Compound(fields) = &mut extras else {
        return Err(storage("conversion extras is not a compound"));
    };
    let key = "data/krasnokamen-game-clock-upgrade.json";
    if fields.iter().any(|(name, _)| name == key) {
        return Err(storage("conversion record already exists"));
    }
    fields.push((
        key.into(),
        Tag::ByteArray(
            serde_json::to_vec_pretty(&report)
                .map_err(storage)?
                .into_iter()
                .map(|byte| byte as i8)
                .collect(),
        ),
    ));
    image.globals.insert(
        "conversion_extras".into(),
        tag_record(report.tick, &extras)?,
    );

    let opened = crate::datapacks::worldgen::open(
        &config.world_dir.join("datapacks"),
        &config.world_dir,
        config.vanilla_data_dir.as_deref(),
        &world.dimensions,
        &mut image.globals,
        &world.reader,
        &world.registry,
    )?;
    let registry = Arc::new(opened.registry);
    let mut states = BTreeMap::new();
    let mut backlog = crate::block_log::Backlog::default();
    for (dimension, replay) in std::mem::take(&mut world.dimensions) {
        let coverage = replay
            .block_log
            .iter()
            .map(|delta| delta.coverage)
            .max()
            .unwrap_or(0);
        backlog.absorb_recovered_tail(dimension, &replay.block_log_tail, coverage);
        states.insert(
            dimension,
            crate::core::load_dimension_state(
                config,
                Arc::clone(&registry),
                dimension,
                &world.manifest,
                replay,
                Arc::clone(&store),
                &world.checkpoint.globals,
                Some(&opened.inputs),
            )?,
        );
    }
    let mut state = states.remove(&Dimension::Overworld).expect("overworld");
    state.other_dimensions = states
        .into_iter()
        .map(|(dimension, state)| (dimension, Box::new(state)))
        .collect();
    // These owners already distinguish native deadlines from game ticks. Their
    // existing legacy converters run once; never blanket-retime absolute fields.
    crate::settlements::persistence::finish_recovery(&mut state)?;
    image.globals.retain(|key, _| {
        !key.starts_with("settlement/")
            && !key.starts_with("transit/")
            && key != "settlement_sequence"
    });
    image
        .globals
        .extend(crate::settlements::persistence::checkpoint_rows(
            &state,
            report.tick,
        )?);
    let mut captures = Vec::new();
    captures.push(capture_chunks(&mut state, report.tick)?);
    for state in state.other_dimensions.values_mut() {
        captures.push(capture_chunks(state, report.tick)?);
    }
    // Entity checkpoint updates are incremental: missing rows need tombstones.
    let live: std::collections::BTreeSet<_> = image
        .entities
        .iter()
        .map(|entity| entity.identity)
        .collect();
    world
        .reader
        .visit_entities(|entity| {
            if entity.payload.is_some() && !live.contains(&entity.identity) {
                image.entities.push(entity_tombstone(
                    uuid::Uuid::from_bytes(entity.identity),
                    report.tick,
                ));
            }
            Ok(())
        })
        .map_err(storage)?;
    backlog.write(&store, &world.registry.blocks, &world.registry.items)?;
    let manifest: CheckpointManifest =
        serde_json::from_slice(&image.globals["checkpoint"].payload).map_err(storage)?;
    let debts: BTreeMap<_, _> = std::iter::once((0, &manifest))
        .chain(
            manifest
                .dimensions
                .iter()
                .map(|(&dimension, image)| (dimension_id(dimension), &**image)),
        )
        .flat_map(|(dimension, image)| {
            image
                .debt
                .iter()
                .map(move |debt| ((dimension, debt.chunk), debt))
        })
        .collect();
    store
        .commit_checkpoint_stream(
            NativeCheckpoint {
                tick: report.tick,
                previous_tick: world.manifest.tick,
                chunks: &[],
                entities: &[],
                players: &image.players,
                globals: &image.globals,
            },
            |write| {
                let mut written = std::collections::BTreeSet::new();
                for capture in &captures {
                    capture
                        .visit_native(|snapshot| {
                            written.insert((
                                snapshot.dimension,
                                [snapshot.effective.pos.x, snapshot.effective.pos.z],
                            ));
                            write(snapshot).map_err(storage)
                        })
                        .map_err(native)?;
                }
                world.reader.visit_chunk_positions(|id, pos| {
                    if written.contains(&(id, [pos.x, pos.z])) {
                        return Ok(());
                    }
                    let mut chunk = world
                        .reader
                        .read_chunk(
                            id,
                            pos,
                            &world.registry.blocks,
                            &world.registry.items,
                            &world.registry.chest_loot,
                        )?
                        .ok_or_else(|| native("stored chunk has no baseline"))?;
                    let base = store
                        .resolve_base(
                            id,
                            pos,
                            &world.registry.blocks,
                            &world.registry.items,
                            &world.registry.chest_loot,
                            || Ok(None),
                        )?
                        .ok_or_else(|| native("stored chunk has no immutable baseline"))?;
                    if let Some(debt) = debts.get(&(id, [pos.x, pos.z])) {
                        crate::debt::project(
                            &mut chunk,
                            debt.last_sim,
                            debt.remaining,
                            debt.held_anchor,
                        )
                        .map_err(native)?;
                    }
                    write(mc_world::native::NativeChunkSnapshot {
                        dimension: id,
                        base: &base,
                        effective: &chunk,
                        dirty_sections: Some(0),
                    })
                })
            },
            image.entities.iter().map(Ok),
            &world.registry.blocks,
            &world.registry.items,
        )
        .map_err(storage)?;
    Ok(report)
}

fn capture_chunks(
    state: &mut crate::tick::TickState,
    tick: u64,
) -> Result<crate::world::ChunkCapture, CoreError> {
    if !crate::core::prepare_container_checkpoint(state) {
        return Err(storage("container checkpoint preparation refused"));
    }
    let (_, _, containers) = crate::core::checkpoint_data_for(state, tick);
    Ok(state.world.capture_dirty(&containers))
}

#[cfg(test)]
#[path = "game_clock_upgrade_tests.rs"]
mod tests;
