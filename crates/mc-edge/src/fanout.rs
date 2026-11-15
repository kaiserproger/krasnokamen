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
            (
                dimension,
                TickPlanIndex::with_previous(projection, previous)
                    .with_reveals(projection, cache.anti_xray())
                    .with_chunk_delta(chunk_deltas.get(&dimension).cloned().flatten()),
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
    let plan_start = probe.map(|_| std::time::Instant::now());
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
    if let (Some(start), Some(index_start), Some(plan_start), Some(fill_start), Some(fill_time)) =
        (probe, index_start, plan_start, fill_start, fill_time)
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
            plan_start.duration_since(index_start).as_secs_f64() * 1000.0,
            fill_start.duration_since(plan_start).as_secs_f64() * 1000.0,
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
