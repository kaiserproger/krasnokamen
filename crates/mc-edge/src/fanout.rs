//! Slow-consumer fanout: [`TickOutput`] -> per-connection framed bytes.
//!
//! Split traffic classes:
//!
//! - **World state** (chunks, entity snapshots, block deltas) and one-shot
//!   visuals, including visual-only explosions: best-effort within world
//!   credit. A competing producer can still refuse a planned send; snapshot
//!   state is re-planned, but visuals may drop.
//! - **Player changes and explosion knockback**: reliable FIFO; overflow
//!   disconnects rather than silently dropping an authoritative effect.
//!   Each publish sums a recipient's explosion impulses into one packet, so
//!   a large visual burst cannot fill that FIFO. On broadcast lag, cumulative
//!   knockback epochs identify affected recipients for an explicit disconnect;
//!   unaffected viewers reconcile against retained snapshots.
//!
//! `publish` plans with `rayon` (one viewer per thread); each packet is
//! framed once at plan time with threshold-256 compression, and encoded
//! chunk frames are shared across viewers by refcount.
//! The adaptation point for a future reliable-channel split in
//! `TickOutput` delivery is here: the caller already routes by
//! [`Priority`], so a split source only changes the input side.

use std::collections::HashMap;

use mc_core::boundary::{ConnectionId, TickOutput};
use rayon::prelude::*;

use crate::plan::{
    ChunkPacketCache, PlanRegistries, PlannedPacket, TickPlanIndex, ViewerState,
    plan_viewer_indexed,
};

/// Per-connection reliable queue bound, in items. World queues are bounded
/// in frames and bytes ([`crate::server::world_queue()`]).
pub const RELIABLE_QUEUE_BOUND: usize = 512;

/// Routing priority for one framed packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// Never dropped; overflow disconnects the connection.
    Reliable,
    /// World queue; snapshot state is repaired on refusal, visuals may drop.
    BestEffort,
    /// Batch start followed by exactly `count` chunks and one finished frame.
    /// The pump queues the entire group atomically, charging every frame slot.
    ChunkBatch(usize),
}

/// One framed, addressed packet ready for the per-connection queues.
#[derive(Debug, Clone)]
pub struct AddressedPacket {
    pub connection: ConnectionId,
    pub data: bytes::Bytes,
    pub priority: Priority,
}

/// Plan + encode one tick for every viewer, in parallel.
///
/// `viewers` is consumed as an iterator of `&mut ViewerState` (the server
/// passes `viewers.values_mut()`); each viewer plans against the same
/// [`TickOutput`] on its own rayon thread. `view_distance` is the server
/// limit; negotiated per-connection radii take precedence within that limit.
///
/// Chunk encoding needs block/biome registries; the 3-arg server call site
/// carries none, so `publish` uses the process-wide required-reports
/// fallback (`krasnokamen_required_*`). Biomes not in the fallback (custom
/// datapack biomes) skip that chunk rather than failing the tick.
pub fn publish<'a>(
    output: &TickOutput,
    viewers: impl Iterator<Item = &'a mut ViewerState> + Send,
    view_distance: i32,
) -> Vec<AddressedPacket> {
    use std::sync::LazyLock;
    struct Fallback {
        biomes: mc_data::Registry,
        blocks: mc_world::BlockRegistry,
        items: mc_data::items::ItemRegistry,
        block_entity_types: mc_data::block_entity_types::BlockEntityTypeRegistry,
    }
    static FALLBACK: LazyLock<Fallback> = LazyLock::new(|| {
        let biomes = mc_data::Registry {
            id: mc_data::Identifier::parse("minecraft:worldgen/biome").expect("static id parses"),
            entries: vec![
                mc_data::Identifier::parse("minecraft:plains").expect("static id parses"),
            ],
        };
        let blocks = mc_world::BlockRegistry::from_report(
            &mc_data::blocks::krasnokamen_required_blocks_report(),
        )
        .expect("required blocks report builds");
        Fallback {
            biomes,
            blocks,
            items: mc_data::items::krasnokamen_required_items(),
            block_entity_types:
                mc_data::block_entity_types::krasnokamen_required_block_entity_types(),
        }
    });
    let registries = PlanRegistries {
        biomes: &FALLBACK.biomes,
        blocks: &FALLBACK.blocks,
        items: &FALLBACK.items,
        block_entity_types: &FALLBACK.block_entity_types,
    };
    // Fallback path has no shared cache (tests/single calls); per-call
    // cache still dedups viewers within this publish.
    let mut cache = ChunkPacketCache::new();
    publish_with_registries(output, viewers, view_distance, &registries, &mut cache)
}

/// [`publish`] with explicit registries (tests + future server wiring).
/// `chunk_cache` is owned by the caller across publishes: framed chunk
/// packets (including authoritative baked light) are reused by revision.
/// Viewers plan in parallel against the read-only cache; misses are then
/// encoded once per chunk and filled in (`plan::fill_chunk_packets`).
pub fn publish_with_registries<'a>(
    output: &TickOutput,
    viewers: impl Iterator<Item = &'a mut ViewerState> + Send,
    view_distance: i32,
    registries: &PlanRegistries<'_>,
    chunk_cache: &mut crate::plan::ChunkPacketCache,
) -> Vec<AddressedPacket> {
    publish_with_registries_chain(
        output,
        viewers,
        view_distance,
        registries,
        chunk_cache,
        None,
    )
}

/// [`publish_with_registries`] with the pump's coalesced publication history:
/// a viewer whose publish was skipped stays contiguous through `chain` instead
/// of paying a full rescan, and history the chain cannot reach still rescans
/// with the reason recorded.
pub(crate) fn publish_with_registries_chain<'a>(
    output: &TickOutput,
    viewers: impl Iterator<Item = &'a mut ViewerState> + Send,
    view_distance: i32,
    registries: &PlanRegistries<'_>,
    chunk_cache: &mut crate::plan::ChunkPacketCache,
    history: Option<&crate::plan::PublicationHistory>,
) -> Vec<AddressedPacket> {
    let probe = crate::plan::item_probe::enabled().then(std::time::Instant::now);
    chunk_cache.retain_resident(output);
    chunk_cache.deltas.begin();
    // One delta per dimension, even if the root also lists its own projection.
    let mut chunk_deltas: HashMap<_, Option<std::sync::Arc<_>>> = HashMap::new();
    for projection in std::iter::once(output).chain(output.snapshot.dimensions.values()) {
        chunk_deltas
            .entry(projection.snapshot.dimension)
            .or_insert_with(|| {
                chunk_cache
                    .deltas
                    .delta(projection)
                    .map(std::sync::Arc::new)
            });
    }
    let mut viewers: Vec<&mut ViewerState> = viewers.collect();
    let index_start = probe.map(|_| std::time::Instant::now());
    let cache = &*chunk_cache;
    let indices: HashMap<_, _> = std::iter::once(output)
        .chain(output.snapshot.dimensions.values())
        .map(|projection| {
            let dimension = projection.snapshot.dimension;
            let entities = &projection.snapshot.entities;
            let previous = viewers
                .iter()
                .filter(|viewer| viewer.dimension == dimension)
                .filter_map(|viewer| viewer.previous_entities())
                .find(|previous| previous.revision() == entities.base_revision())
                .or_else(|| {
                    viewers
                        .iter()
                        .filter(|viewer| viewer.dimension == dimension)
                        .filter_map(|viewer| viewer.previous_entities())
                        .find(|previous| previous.revision() == entities.revision())
                });
            let mut index = TickPlanIndex::with_previous(projection, previous);
            let chain = history.map(|history| history.chain(dimension));
            if index.entity_base.is_none()
                && let Some(chain) = chain
            {
                // The chain can keep the viewers whose publish the pump
                // skipped contiguous: bridge the base most of them hold, and
                // let the rest rescan with a named reason.
                let bridge = TickPlanIndex::bridge(
                    chain,
                    entities.revision(),
                    viewers
                        .iter()
                        .filter(|viewer| viewer.dimension == dimension)
                        .filter_map(|viewer| viewer.previous_entities()),
                );
                if let Some((base, _, changed)) = bridge {
                    index = TickPlanIndex::with_chain(projection, base, &changed);
                }
            }
            (
                dimension,
                index
                    .with_reveals(projection, cache.anti_xray())
                    .with_chunk_delta(chunk_deltas.get(&dimension).cloned().flatten())
                    .with_chain_info(chain.map(|chain| chain.info()).unwrap_or_default()),
            )
        })
        .collect();
    // A recorder's tick marker is the first world frame of the publish. Its
    // slot and bytes are reserved before planning so the bounds hold;
    // without a free slot nothing else is planned either, and that tick has
    // no marker.
    let marker = viewers
        .iter()
        .any(|viewer| viewer.recorder)
        .then(|| crate::recorder::tick_marker_frame(output.tick, output.times.wall_start_unix_ns));
    let marker_bytes = marker.as_ref().map_or(0, bytes::Bytes::len);
    // Counters for the plan debug line: a rescan is per-viewer O(chunk square), so its
    // per-publish delta says whether a fanout step comes from rescans or from move volume.
    // The guard must name the target the line is emitted on: a bare `enabled!(DEBUG)` asks the
    // default target (this module, mc_edge::fanout), which a `mc_edge::server::plan=debug` filter
    // leaves off, and the line would never be written.
    let plan_debug = tracing::enabled!(target: "mc_edge::server::plan", tracing::Level::DEBUG);
    let rescans_before = if plan_debug {
        viewers.iter().map(|viewer| viewer.rescans).sum::<u64>()
    } else {
        0
    };
    let scanned_before = if plan_debug {
        viewers
            .iter()
            .map(|viewer| viewer.entities_scanned)
            .sum::<u64>()
    } else {
        0
    };
    // Independent of the item probe: plan_us must be measured whenever the line is going to
    // be written, and MC_CORE_PHASE_LOG is never set on a measured leg.
    let plan_clock = plan_debug.then(std::time::Instant::now);
    let mut planned: Vec<(ConnectionId, Vec<PlannedPacket>)> = Vec::with_capacity(viewers.len());
    let mut marked: Vec<ConnectionId> = Vec::new();
    for (connection, packets, reserved) in viewers
        .par_iter_mut()
        .map(|viewer| {
            let reserved = viewer.recorder && viewer.world_credit > 0;
            if reserved {
                viewer.world_credit -= 1;
                viewer.world_bytes = viewer.world_bytes.saturating_sub(marker_bytes);
            }
            viewer.deferred_updates = 0;
            let dimension = viewer.dimension;
            let packets = output
                .in_dimension(dimension)
                .map_or_else(Vec::new, |projection| {
                    plan_viewer_indexed(
                        projection,
                        viewer,
                        view_distance,
                        cache,
                        &indices[&dimension],
                        registries,
                    )
                });
            (viewer.connection, packets, reserved)
        })
        .collect::<Vec<_>>()
    {
        if reserved {
            marked.push(connection);
        }
        planned.push((connection, packets));
    }
    if plan_debug {
        let packets: usize = planned.iter().map(|(_, packets)| packets.len()).sum();
        let chunk_packets: usize = planned
            .iter()
            .flat_map(|(_, packets)| packets.iter())
            .filter(|packet| packet.chunk.is_some())
            .count();
        let viewers_unplanned = planned
            .iter()
            .filter(|(_, packets)| packets.is_empty())
            .count();
        let (hit, miss) = indices.values().fold((0u64, 0u64), |(hit, miss), index| {
            let (h, m) = index.move_tallies();
            (hit + h, miss + m)
        });
        tracing::debug!(
            target: "mc_edge::server::plan",
            tick = output.tick,
            viewers = viewers.len(),
            viewers_unplanned,
            packets,
            chunk_packets,
            plan_ns = plan_clock.map_or(0, |start| start.elapsed().as_nanos() as u64),
            plan_us = plan_clock.map_or(0, |start| start.elapsed().as_micros() as u64),
            rescans = viewers.iter().map(|viewer| viewer.rescans).sum::<u64>() - rescans_before,
            entities_scanned = viewers.iter().map(|viewer| viewer.entities_scanned).sum::<u64>()
                - scanned_before,
            rescan_gap = viewers.iter().map(|viewer| viewer.rescans_gap).sum::<u64>(),
            rescan_no_history = viewers
                .iter()
                .map(|viewer| viewer.rescans_no_history)
                .sum::<u64>(),
            move_hits = hit,
            move_misses = miss,
            "pump plan"
        );
    }
    let fill_start = probe.map(|_| std::time::Instant::now());
    let failed = crate::plan::fill_chunk_packets(output, &mut planned, registries, chunk_cache);
    let fill_time = fill_start.map(|start| start.elapsed());
    for (connection, pos) in failed {
        if let Some(viewer) = viewers
            .iter_mut()
            .find(|viewer| viewer.connection == connection)
        {
            viewer.chunk_unsent(pos);
        }
    }
    // Cache misses had no encoded bytes during planning. Apply the actual
    // sizes now, admitting one oversize world frame (or chunk plus markers)
    // for progress, but no later world dependency after the first deferral.
    for ((connection, packets), viewer) in planned.iter_mut().zip(viewers.iter_mut()) {
        debug_assert_eq!(*connection, viewer.connection);
        let mut bytes = viewer.world_bytes;
        let mut first_chunk = true;
        let mut deferred = false;
        packets.retain(|packet| {
            if packet.reliable {
                return true;
            }
            if bytes == 0 {
                deferred = true;
                return false;
            }
            let framing = if packet.chunk.is_some() && first_chunk {
                first_chunk = false;
                7
            } else {
                0
            };
            bytes = bytes.saturating_sub(packet.data.len() + framing);
            true
        });
        if deferred {
            viewer.world_frame_unsent();
        }
    }
    if let (Some(start), Some(index_start), Some(plan_clock), Some(fill_start), Some(fill_time)) =
        (probe, index_start, plan_clock, fill_start, fill_time)
    {
        let total_time = start.elapsed();
        let (packets, bytes) = planned
            .iter()
            .fold((0u64, 0u64), |(count, bytes), (_, row)| {
                (
                    count + row.len() as u64,
                    bytes
                        + row
                            .iter()
                            .map(|packet| packet.data.len() as u64)
                            .sum::<u64>(),
                )
            });
        let (item_pairs, all_pairs) =
            crate::plan::item_probe::tracked_item_pairs(viewers.iter().map(|viewer| &**viewer));
        eprintln!(
            "edge items: tick={} viewers={} packets={packets} bytes={bytes} item_pairs={item_pairs} \
             all_pairs={all_pairs} {} prelude_ms={:.3} index_ms={:.3} plan_ms={:.3} fill_ms={:.3} \
             total_ms={:.3} probe_ms={:.3}",
            output.tick,
            viewers.len(),
            crate::plan::item_probe::take_line(),
            index_start.duration_since(start).as_secs_f64() * 1000.0,
            plan_clock.duration_since(index_start).as_secs_f64() * 1000.0,
            fill_start.duration_since(plan_clock).as_secs_f64() * 1000.0,
            fill_time.as_secs_f64() * 1000.0,
            total_time.as_secs_f64() * 1000.0,
            (start.elapsed() - total_time).as_secs_f64() * 1000.0,
        );
    }
    let mut addressed = Vec::new();
    for (connection, packets) in planned {
        if let Some(marker) = marker.as_ref().filter(|_| marked.contains(&connection)) {
            addressed.push(AddressedPacket {
                connection,
                data: marker.clone(),
                priority: Priority::BestEffort,
            });
        }
        let count = packets
            .iter()
            .filter(|packet| packet.chunk.is_some())
            .count();
        let mut chunks_left = count;
        let mut started = false;
        for packet in packets {
            if packet.chunk.is_some() && !started {
                use mc_protocol::packets::{Packet, play::ClientboundChunkBatchStart};
                addressed.push(AddressedPacket {
                    connection,
                    data: mc_protocol::frame::encode_frame(
                        ClientboundChunkBatchStart::ID,
                        &[],
                        mc_protocol::frame::Compression::Threshold(256),
                    )
                    .expect("empty batch start frames"),
                    priority: Priority::ChunkBatch(count),
                });
                started = true;
            }
            let chunk = packet.chunk.is_some();
            addressed.push(AddressedPacket {
                connection,
                data: packet.data,
                priority: if packet.reliable {
                    Priority::Reliable
                } else {
                    Priority::BestEffort
                },
            });
            if chunk {
                chunks_left -= 1;
                if chunks_left == 0 {
                    use mc_protocol::packets::{Packet, play::ClientboundChunkBatchFinished};
                    let mut body = Vec::new();
                    ClientboundChunkBatchFinished {
                        batch_size: count as i32,
                    }
                    .encode(&mut body)
                    .expect("bounded batch count encodes");
                    addressed.push(AddressedPacket {
                        connection,
                        data: mc_protocol::frame::encode_frame(
                            ClientboundChunkBatchFinished::ID,
                            &body,
                            mc_protocol::frame::Compression::Threshold(256),
                        )
                        .expect("batch finished frames"),
                        priority: Priority::BestEffort,
                    });
                }
            }
        }
    }
    addressed
}

/// Connections whose reliable output exceeds the bound. The caller must
/// disconnect them: reliable packets are never skipped, so an over-full
/// reliable queue means the consumer is gone.
///
/// Counts reliable packets per connection; any connection with more than
/// [`RELIABLE_QUEUE_BOUND`] queued reliable packets overflows. (The
/// live queue depth is owned by the server layer; this pure helper lets
/// the pump decide from a single `publish` batch.)
pub fn reliable_overflow(packets: &[AddressedPacket]) -> Vec<ConnectionId> {
    let mut counts: HashMap<ConnectionId, usize> = HashMap::new();
    for packet in packets.iter() {
        if packet.priority == Priority::Reliable {
            *counts.entry(packet.connection).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > RELIABLE_QUEUE_BOUND)
        .map(|(connection, _)| connection)
        .collect()
}

#[cfg(test)]
#[path = "fanout_tests.rs"]
mod fanout_tests;

#[cfg(test)]
#[path = "chunk_batch_tests.rs"]
mod chunk_batch_tests;
