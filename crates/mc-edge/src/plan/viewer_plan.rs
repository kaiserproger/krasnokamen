//! The per-viewer publish plan: one tick output plus the cached view to
//! world packets, in credit priority order.
use super::*;

/// Vanilla world-clock resync period (`MinecraftServer` every 20 ticks).
const TIME_SYNC_INTERVAL_TICKS: u64 = 20;

/// Plan every world packet for one viewer from one tick output;
/// `view_distance` sizes the Chebyshev chunk square. Player changes ride
/// the core outbox ([`plan_player_changes`]); weather also reconciles against
/// the authoritative snapshot so skipped projections cannot strand a fade.
///
/// Best-effort frames are bounded by [`ViewerState::world_credit`], in
/// priority order: chunk forgets, block updates, light updates, chunks
/// (nearest first), entity removals, spawns (nearest first), appearance/moves
/// (most overdue, nearest first), visual events. Work that does not fit stays
/// unsent in the viewer cache and is planned again from a newer snapshot.
/// Recipient knockback has no snapshot image and is summed into one reliable
/// explosion per publish. Remaining explosion visuals spend world credit.
#[cfg(test)]
pub(crate) fn plan_viewer(
    output: &TickOutput,
    viewer: &mut ViewerState,
    view_distance: i32,
    chunk_cache: &ChunkPacketCache,
) -> Vec<PlannedPacket> {
    use std::sync::LazyLock;
    static BIOMES: LazyLock<mc_data::Registry> =
        LazyLock::new(|| crate::chunks::test_biome_registry(&["minecraft:plains"]));
    static BLOCKS: LazyLock<mc_world::BlockRegistry> = LazyLock::new(|| {
        mc_world::BlockRegistry::from_report(&mc_data::blocks::krasnokamen_required_blocks_report())
            .expect("required blocks report builds")
    });
    static ITEMS: LazyLock<mc_data::items::ItemRegistry> =
        LazyLock::new(mc_data::items::krasnokamen_required_items);
    static TYPES: LazyLock<mc_data::block_entity_types::BlockEntityTypeRegistry> =
        LazyLock::new(mc_data::block_entity_types::krasnokamen_required_block_entity_types);
    let registries = PlanRegistries {
        biomes: &BIOMES,
        blocks: &BLOCKS,
        items: &ITEMS,
        block_entity_types: &TYPES,
    };
    let Some(output) = output.in_dimension(viewer.dimension) else {
        return Vec::new();
    };
    let index = TickPlanIndex::with_previous(output, viewer.entity_interest.previous())
        .with_reveals(output, chunk_cache.anti_xray());
    plan_viewer_indexed(
        output,
        viewer,
        view_distance,
        chunk_cache,
        &index,
        &registries,
    )
}

pub(crate) fn plan_viewer_indexed(
    output: &TickOutput,
    viewer: &mut ViewerState,
    view_distance: i32,
    chunk_cache: &ChunkPacketCache,
    index: &TickPlanIndex,
    registries: &PlanRegistries<'_>,
) -> Vec<PlannedPacket> {
    let view_distance = viewer
        .view_distance
        .unwrap_or(view_distance)
        .min(view_distance);
    let mut scratch = std::mem::take(&mut viewer.scratch);
    let packets = plan_viewer_inner(
        output,
        viewer,
        view_distance,
        chunk_cache,
        index,
        registries,
        &mut scratch,
    );
    viewer.scratch = scratch;
    packets
}

fn plan_viewer_inner(
    output: &TickOutput,
    viewer: &mut ViewerState,
    view_distance: i32,
    chunk_cache: &ChunkPacketCache,
    index: &TickPlanIndex,
    registries: &PlanRegistries<'_>,
    scratch: &mut PlanScratch,
) -> Vec<PlannedPacket> {
    let mut out = Vec::new();
    let snapshot = &output.snapshot;
    let changes = &output.changes;
    let conn = viewer.connection;
    let mut credit = WorldCredit {
        frames: viewer.world_credit,
        bytes: viewer.world_bytes,
    };
    viewer.world_entities.clear();
    viewer.world_removed.clear();
    let desired_camera = snapshot
        .cameras
        .get(&viewer.entity_id)
        .copied()
        .unwrap_or(viewer.entity_id);
    if viewer.camera != Some(desired_camera)
        && !camera::send(viewer, viewer.entity_id, &mut out, &mut credit)
    {
        return out;
    }
    // Resolve the authoritative anchor before range-checking one-shot effects.
    if let Some(own) = own_entity(snapshot, viewer) {
        viewer.position = own.position;
    }
    // Impulses are additive on the client. One reliable packet per recipient
    // preserves their sum without overflowing the FIFO on a large TNT burst.
    index.explosions_near(viewer.position, &mut scratch.events);
    let impulse = index.knockback.get(&viewer.entity_id);
    let carrier = impulse.map(|&(event, _)| event);
    if let Some(&(event, push)) = impulse {
        push_reliable(
            &mut out,
            conn,
            &explode_packet(&changes.explosions[event], Some(push)),
        );
    }
    for &event in &scratch.events {
        if Some(event) == carrier {
            continue;
        }
        let explosion = &changes.explosions[event];
        let [x, y, z] = viewer.position;
        let [cx, cy, cz] = explosion.center;
        if (x - cx).powi(2) + (y - cy).powi(2) + (z - cz).powi(2) <= EXPLOSION_RANGE_SQ
            && !try_push(&mut out, &mut credit, |out| {
                push_best_effort(out, conn, &explode_packet(explosion, None));
            })
        {
            break;
        }
    }
    // Heartbeats and discontinuities share the reliable FIFO: an older queued
    // heartbeat must never overwrite a time command, rate change or sleep skip.
    // Sync before chunk work, including a credit-blocked respawn forget pass.
    if viewer.clock_baseline.is_none_or(|(at, day_time, rate)| {
        let elapsed = snapshot.tick.saturating_sub(at);
        elapsed >= TIME_SYNC_INTERVAL_TICKS
            || snapshot.day_time_rate != rate
            || snapshot.day_time
                != day_time.saturating_add((elapsed as f64 * f64::from(rate)) as i64)
    }) {
        push_reliable(
            &mut out,
            conn,
            &ClientboundSetTime::overworld(
                i64::try_from(snapshot.game_time).unwrap_or(i64::MAX),
                snapshot.day_time,
                snapshot.day_time_rate,
            ),
        );
        viewer.clock_baseline = Some((snapshot.tick, snapshot.day_time, snapshot.day_time_rate));
    }
    #[cfg(test)]
    let mut stats = chunk_delta::ChunkPlanStats::default();
    // Cleared until this plan completes: returning early leaves the next
    // plan a full walk.
    let sync = viewer.chunk_sync.take();
    let delta = index.chunk_delta();
    // A pending forget empties the held set and a skip drops block changes:
    // neither is described by the delta.
    let followed = !viewer.forget_all
        && !viewer.skipped
        && delta.zip(sync).is_some_and(|(delta, sync)| {
            sync.quiet
                && sync.held == viewer.loaded_chunks.len()
                && delta.follows(&sync, snapshot.dimension)
        });
    if !followed {
        chunk_stat!(stats.visited += viewer.light_baseline.len());
        chunk_stat!(stats.probes += viewer.light_baseline.len());
        viewer.light_baseline.retain_held(&viewer.loaded_chunks);
    }
    crate::faction_teams::plan_teams(snapshot, index.faction_delta(), viewer, &mut out);
    if viewer.forget_all && !forget_everything(viewer, &mut out, &mut credit, scratch) {
        return out;
    }
    viewer.weather_baseline.snapshot(
        conn,
        snapshot.raining,
        snapshot.rain_level,
        snapshot.thunder_level,
        &mut out,
    );
    if viewer.border_baseline != Some(snapshot.border)
        && try_push(&mut out, &mut credit, |out| {
            push_best_effort(
                out,
                conn,
                &mc_protocol::packets::play::ClientboundInitializeBorder {
                    center_x: snapshot.border.center_x,
                    center_z: snapshot.border.center_z,
                    old_size: snapshot.border.size,
                    new_size: snapshot.border.lerp_target,
                    lerp_time: snapshot.border.lerp_remaining.saturating_mul(50),
                    absolute_max_size: 29_999_984,
                    warning_blocks: snapshot.border.warning_blocks,
                    // Protocol warning time is seconds; vanilla
                    // ClientboundInitializeBorderPacket carries
                    // getWarningTime (seconds) alongside getLerpTime.
                    warning_time: snapshot.border.warning_time,
                },
            );
        })
    {
        viewer.border_baseline = Some(snapshot.border);
    }
    let center = entity_chunk(viewer.position);

    // Center + radius (reliable: the client discards mis-anchored chunks).
    if viewer.center_sent != Some(center) {
        push_reliable(
            &mut out,
            conn,
            &SetCenterChunk {
                chunk_x: center.0,
                chunk_z: center.1,
            },
        );
        viewer.center_sent = Some(center);
        viewer.radius_sent = None;
    }
    if viewer.radius_sent != Some(view_distance.max(0)) {
        push_reliable(
            &mut out,
            conn,
            &SetChunkCacheRadius {
                radius: view_distance.max(0),
            },
        );
        viewer.radius_sent = Some(view_distance.max(0));
    }

    // Chunk unloads: a chunk left without credit stays held and is
    // forgotten by a later plan.
    let window_moved = viewer.window.update(center, view_distance);
    // The window is unmoved and the viewer held nothing outside it, so only
    // the chunks the publish delta names can have moved on.
    let fast = followed && !window_moved;
    let mut unload_debt = false;
    let mut light_debt = false;
    let mut admission_debt = false;
    let mut admit = !fast;
    chunk_stat!(stats.plans += 1);
    chunk_stat!(stats.delta_plans += u64::from(fast));
    chunk_stat!(stats.full_walks += u64::from(!fast));
    if !fast {
        chunk_stat!(stats.visited += viewer.loaded_chunks.len());
        chunk_stat!(stats.probes += viewer.loaded_chunks.len());
        scratch.chunks.clear();
        scratch.chunks.extend(
            viewer
                .loaded_chunks
                .keys()
                .filter(|pos| !viewer.window.want.contains(pos))
                .copied(),
        );
        for &gone in &scratch.chunks {
            let forgotten = try_push(&mut out, &mut credit, |out| {
                push_best_effort(
                    out,
                    conn,
                    &ForgetLevelChunk {
                        chunk_x: gone.0,
                        chunk_z: gone.1,
                    },
                );
            });
            if forgotten {
                viewer.loaded_chunks.remove(&gone);
                viewer.chunk_repairs.remove(&gone);
                viewer.block_entity_updates.remove(&gone);
                viewer.light_baseline.forget(gone);
            } else {
                unload_debt = true;
            }
        }
    }
    // Public block-entity changes and non-skipped block changes
    // explain content revisions without full resends. A changed baked light
    // baseline covers light-only revisions; skips still require repairs.
    let skipped = std::mem::take(&mut viewer.skipped);
    let skip_base = viewer.skip_base.take();
    scratch.touched.clear();
    scratch.touched.extend(viewer.block_entity_updates.drain());
    if let Some(delta) = delta.filter(|_| fast) {
        // Walk the shared change lists, not this viewer's held chunks.
        for (chunk, changes_in_chunk) in &index.blocks {
            chunk_stat!(stats.visited += 1);
            chunk_stat!(stats.probes += 1);
            if viewer.loaded_chunks.contains_key(chunk) {
                for &change in changes_in_chunk {
                    viewer
                        .pending_blocks
                        .insert(changes.blocks[change].position);
                }
            }
        }
        for (chunk, revealed) in &index.reveals {
            chunk_stat!(stats.visited += 1);
            chunk_stat!(stats.probes += 1);
            if viewer.loaded_chunks.contains_key(chunk) {
                viewer.pending_blocks.extend(revealed.iter().copied());
            }
        }
        scratch.delta_chunks.clear();
        for change in delta.changes() {
            chunk_stat!(stats.visited += 1);
            if change.kind == ChunkChangeKind::Unloaded {
                continue;
            }
            chunk_stat!(stats.probes += 1);
            if !viewer.window.want.contains(&change.pos) {
                continue;
            }
            chunk_stat!(stats.probes += 1);
            if viewer.loaded_chunks.contains_key(&change.pos) {
                scratch.delta_chunks.push(change.pos);
            } else {
                // Newly resident inside the window: admission owns it.
                admit = true;
            }
        }
        for &pos in &scratch.delta_chunks {
            chunk_stat!(stats.visited += 1);
            chunk_stat!(stats.probes += 2);
            let Some(held) = viewer.loaded_chunks.get_mut(&pos) else {
                continue;
            };
            let Some(current) = find_chunk(&snapshot.chunks, pos) else {
                continue;
            };
            if *held == current.revision {
                continue;
            }
            chunk_stat!(stats.probes += 2);
            let light_changed = viewer.light_baseline.observe(pos, &current.chunk);
            if !skipped
                && (scratch.touched.contains(&pos)
                    || index.blocks.contains_key(&pos)
                    || light_changed)
            {
                *held = current.revision;
            } else {
                viewer.chunk_repairs.insert(pos);
            }
        }
        admit |= !viewer.chunk_repairs.is_empty();
    } else {
        for chunk in viewer.loaded_chunks.keys() {
            chunk_stat!(stats.visited += 1);
            chunk_stat!(stats.probes += 2);
            if let Some(changes_in_chunk) = index.blocks.get(chunk) {
                scratch.touched.insert(*chunk);
                for &change in changes_in_chunk {
                    viewer
                        .pending_blocks
                        .insert(changes.blocks[change].position);
                }
            }
            // Ores a block change next to them just exposed (anti-xray sent the
            // rock stand-in): the change may sit in a neighbouring chunk, so the
            // shared index is keyed by the ore's own chunk.
            if let Some(revealed) = index.reveals.get(chunk) {
                viewer.pending_blocks.extend(revealed.iter().copied());
            }
        }
        // Anti-xray judges an ore by its side neighbours too: a skipped block
        // change beside a chunk border may have revealed an ore without moving
        // the ore chunk's own revision (no light change), and that reveal
        // lived only in the skipped changes.
        let side_reveals = skipped
            && chunk_cache
                .anti_xray()
                .is_some_and(|policy| policy.enabled(snapshot.dimension));
        for (pos, held) in &mut viewer.loaded_chunks {
            chunk_stat!(stats.visited += 1);
            chunk_stat!(stats.probes += 1);
            if !viewer.window.want.contains(pos) {
                continue;
            }
            chunk_stat!(stats.probes += 1);
            let Some(current) = find_chunk(&snapshot.chunks, *pos) else {
                continue;
            };
            if *held == current.revision {
                if side_reveals {
                    chunk_stat!(stats.probes += 8);
                    if side_moved(
                        skip_base.as_deref().map(Vec::as_slice),
                        &snapshot.chunks,
                        *pos,
                    ) {
                        viewer.chunk_repairs.insert(*pos);
                    }
                }
                continue;
            }
            chunk_stat!(stats.probes += 1);
            if viewer.chunk_repairs.contains(pos) {
                continue;
            }
            chunk_stat!(stats.probes += 2);
            let light_changed = viewer.light_baseline.observe(*pos, &current.chunk);
            if !skipped && (scratch.touched.contains(pos) || light_changed) {
                *held = current.revision;
            } else {
                viewer.chunk_repairs.insert(*pos);
            }
        }
        chunk_stat!(stats.visited += viewer.light_baseline.len());
        chunk_stat!(stats.probes += viewer.light_baseline.len());
        viewer.light_baseline.retain_held(&viewer.loaded_chunks);
    }
    // Owed block updates use the newest snapshot state. Like vanilla, one
    // block uses BlockUpdate; multiple blocks in a section share one packet
    // and one queue credit. A full repair already covers its chunk's debt.
    let section_of = |position: &mc_world::chunk::BlockPos| {
        (
            position.x.div_euclid(16),
            position.y.div_euclid(16),
            position.z.div_euclid(16),
        )
    };
    scratch.blocks.clear();
    scratch.blocks.extend(viewer.pending_blocks.drain());
    scratch.blocks.sort_unstable_by_key(|position| {
        (section_of(position), position.x, position.y, position.z)
    });
    let mut update = SectionBlocksUpdate {
        section_pos: 0,
        changes: std::mem::take(&mut scratch.section_blocks),
    };
    let mut start = 0;
    while start < scratch.blocks.len() {
        let section = section_of(&scratch.blocks[start]);
        let mut end = start + 1;
        while end < scratch.blocks.len() && section_of(&scratch.blocks[end]) == section {
            end += 1;
        }
        let positions = &scratch.blocks[start..end];
        start = end;
        chunk_stat!(stats.visited += positions.len() as u64);
        chunk_stat!(stats.probes += 3);
        let chunk = (section.0, section.2);
        if !viewer.loaded_chunks.contains_key(&chunk) || viewer.chunk_repairs.contains(&chunk) {
            continue;
        }
        if credit.spent() {
            viewer.pending_blocks.extend(positions.iter().copied());
            continue;
        }
        let Some(resident) = find_chunk(&snapshot.chunks, chunk) else {
            viewer.pending_blocks.extend(positions.iter().copied());
            continue;
        };
        update.section_pos = pack_section_pos(section.0, section.1, section.2);
        update.changes.clear();
        for position in positions {
            if let Some(state) = resident.chunk.get_block(
                position.x.rem_euclid(16) as u8,
                position.y,
                position.z.rem_euclid(16) as u8,
            ) {
                update.changes.push(SectionBlockChange {
                    relative_pos: pack_section_relative_pos(position.x, position.y, position.z),
                    state_id: state.0 as i32,
                });
            }
        }
        if update.changes.is_empty() {
            continue;
        }
        if !try_push(&mut out, &mut credit, |out| {
            if positions.len() == 1 {
                push_block_update(out, conn, positions[0], update.changes[0].state_id as u32);
            } else {
                push_best_effort(out, conn, &update);
            }
        }) {
            viewer.pending_blocks.extend(positions.iter().copied());
        }
    }
    scratch.section_blocks = update.changes;
    // A repair packet contains the latest projection; otherwise send the
    // entity only after its block update and through the same world FIFO.
    if !viewer.pending_block_entities.is_empty() {
        scratch.blocks.clear();
        scratch.blocks.extend(viewer.pending_block_entities.drain());
        scratch
            .blocks
            .sort_unstable_by_key(|pos| (pos.x, pos.y, pos.z));
        for &pos in &scratch.blocks {
            let chunk = (pos.x.div_euclid(16), pos.z.div_euclid(16));
            if !viewer.loaded_chunks.contains_key(&chunk) || viewer.chunk_repairs.contains(&chunk) {
                continue;
            }
            if credit.spent() || viewer.pending_blocks.contains(&pos) {
                viewer.pending_block_entities.insert(pos);
                continue;
            }
            let Some(resident) = find_chunk(&snapshot.chunks, chunk) else {
                continue;
            };
            let Some(record) =
                mc_world::wire::client_block_entity(&resident.chunk, registries.blocks, pos)
            else {
                continue;
            };
            let Some(type_id) = registries
                .block_entity_types
                .id_of(&record.type_name)
                .and_then(|id| i32::try_from(id).ok())
            else {
                continue;
            };
            let packet = mc_protocol::packets::play::ClientboundBlockEntityData {
                position: mc_protocol::packets::play::pack_block_pos(pos.x, pos.y, pos.z),
                block_entity_type: type_id,
                nbt: record.nbt,
            };
            if !try_push(&mut out, &mut credit, |out| {
                push_best_effort(out, conn, &packet)
            }) {
                viewer.pending_block_entities.insert(pos);
            }
        }
    }
    // A full repair covers its own newest baked light. Other light debt
    // survives exhausted credit without replacing a chunk admission. Light
    // moves only with a chunk revision, so the delta names every candidate.
    if fast {
        scratch
            .delta_chunks
            .sort_unstable_by_key(|&(x, z)| ((x - center.0).abs().max((z - center.1).abs()), x, z));
        for &pos in &scratch.delta_chunks {
            chunk_stat!(stats.visited += 1);
            chunk_stat!(stats.probes += 3);
            if !viewer.chunk_repairs.contains(&pos)
                && let Some(resident) = find_chunk(&snapshot.chunks, pos)
            {
                light_debt |= !viewer.light_baseline.update(
                    pos,
                    &resident.chunk,
                    resident.revision,
                    conn,
                    &mut out,
                    &mut credit,
                );
            }
        }
    } else {
        for &pos in &viewer.window.order {
            chunk_stat!(stats.visited += 1);
            chunk_stat!(stats.probes += 1);
            if !viewer.loaded_chunks.contains_key(&pos) {
                continue;
            }
            chunk_stat!(stats.probes += 1);
            if viewer.chunk_repairs.contains(&pos) {
                continue;
            }
            chunk_stat!(stats.probes += 2);
            if let Some(resident) = find_chunk(&snapshot.chunks, pos) {
                light_debt |= !viewer.light_baseline.update(
                    pos,
                    &resident.chunk,
                    resident.revision,
                    conn,
                    &mut out,
                    &mut credit,
                );
            }
        }
    }
    // Labels refer only to chunks held before this plan, never to an unseen
    // chunk admitted below whose packet has not reached the client yet.
    crate::settlement_labels::plan_labels(snapshot, viewer, view_distance, &mut out, &mut credit);

    // Admit unseen chunks first, then repair held chunks. Both share the same
    // bounded send budget; a deferred repair remains owed across publishes.
    let mut chunks_left = viewer.chunk_batches.lock().allowance();
    // Two framing slots travel atomically with the chunks. Reserve them
    // before admission; unused reservations are returned below.
    let batch_credit = credit;
    if credit.frames >= 3 && credit.bytes > 0 {
        credit.frames -= 2;
    } else {
        chunks_left = 0;
    }
    let batch_start = out.len();
    for repairs in [false, true] {
        if !admit {
            break;
        }
        for &pos in &viewer.window.order {
            chunk_stat!(stats.visited += 1);
            chunk_stat!(stats.probes += 1);
            if if repairs {
                !viewer.chunk_repairs.contains(&pos)
            } else {
                viewer.loaded_chunks.contains_key(&pos)
            } {
                continue;
            }
            if chunks_left == 0 || credit.spent() {
                admission_debt = true;
                break;
            }
            // Missing resident chunks do not spend client quota.
            chunk_stat!(stats.probes += 1);
            let Some(resident) = find_chunk(&snapshot.chunks, pos) else {
                continue;
            };
            chunks_left -= 1;
            let data = chunk_cache
                .get(
                    snapshot.dimension,
                    pos,
                    chunk_cache.key(snapshot.dimension, &snapshot.chunks, resident),
                )
                .cloned()
                .unwrap_or_default();
            out.push(PlannedPacket {
                connection: conn,
                id: mc_protocol::packets::play::LevelChunkWithLight::ID,
                reliable: false,
                data,
                chunk: Some(pos),
                chunk_dimension: snapshot.dimension,
            });
            credit.charge(&out[out.len() - 1..]);
            if out.len() == batch_start + 1 {
                // Threshold-256 framing: empty start = 3 bytes, one-byte count
                // finish = 4. Charge with the first chunk so any remaining
                // byte credit still admits one complete (possibly large) group.
                credit.bytes = credit.bytes.saturating_sub(7);
            }
            viewer.loaded_chunks.insert(pos, resident.revision);
            viewer.chunk_repairs.remove(&pos);
            viewer
                .light_baseline
                .record(pos, &resident.chunk, resident.revision);
        }
    }
    if out.len() == batch_start {
        credit = batch_credit;
    }
    if let Some(delta) = delta {
        let quiet = !unload_debt
            && !light_debt
            && !admission_debt
            && viewer.chunk_repairs.is_empty()
            && viewer.pending_blocks.is_empty()
            && viewer.pending_block_entities.is_empty();
        viewer.chunk_sync = Some(delta.sync(snapshot.dimension, viewer.loaded_chunks.len(), quiet));
    }
    #[cfg(test)]
    viewer.chunk_stats.add(stats);

    // One-shot visuals of entities this client has; lost without credit.
    // Before entity removals: vanilla sends the pickup (and death) event
    // while the client still holds the entity, then removes it.
    index.events_for(
        &viewer.window.want,
        viewer
            .tracked_entities
            .keys()
            .copied()
            .chain(std::iter::once(viewer.entity_id)),
        &mut scratch.events,
    );
    for &event in &scratch.events {
        let event = &changes.entity_events[event];
        let subject = event_subject(*event);
        if subject != viewer.entity_id && !viewer.tracked_entities.contains_key(&subject) {
            continue;
        }
        if !try_push(&mut out, &mut credit, |out| {
            push_entity_event(out, conn, *event);
        }) {
            break;
        }
    }

    // Entities: remove / spawn / appearance / move, via the chunk index.
    plan_entities(
        snapshot,
        viewer,
        view_distance,
        scratch,
        index,
        &mut out,
        &mut credit,
    );
    camera::reconcile(snapshot, viewer, &mut out, &mut credit);
    plan_seats(viewer, &mut out, &mut credit);
    out
}

/// Direct identity lookup, including teleports and not-yet-admitted chunks.
pub(super) fn own_entity<'a>(
    snapshot: &'a WorldSnapshot,
    viewer: &ViewerState,
) -> Option<&'a EntityView> {
    snapshot.entities.get(viewer.entity_id)
}

/// A side neighbour of `pos` that loaded, unloaded or changed revision since
/// `base`: anti-xray counts an unloaded side as open, so presence changes the
/// ore mask too. Without a base every side counts as moved.
fn side_moved(
    base: Option<&[SnapshotChunk]>,
    chunks: &[SnapshotChunk],
    (x, z): (i32, i32),
) -> bool {
    let Some(base) = base else {
        return true;
    };
    [(x - 1, z), (x + 1, z), (x, z - 1), (x, z + 1)]
        .into_iter()
        .any(|side| {
            find_chunk(base, side).map(|was| was.revision)
                != find_chunk(chunks, side).map(|now| now.revision)
        })
}

fn push_block_update(
    out: &mut Vec<PlannedPacket>,
    connection: ConnectionId,
    position: mc_world::chunk::BlockPos,
    state: u32,
) {
    push_best_effort(
        out,
        connection,
        &BlockUpdate {
            position: mc_protocol::packets::play::pack_block_pos(
                position.x, position.y, position.z,
            ),
            state_id: state as i32,
        },
    );
}
