//! Framed chunk cache, chunk encoder and the Chebyshev square order.
//!
//! One framed `LevelChunkWithLight` per resident chunk revision, shared by
//! every viewer; the cache is owned by the pump.
use super::*;

/// Cross-viewer framed chunk cache, owned by the pump. A chunk revision
/// includes its authoritative baked light; neighbour light changes are
/// reflected by the core in the affected chunk's own revision. With anti-xray
/// enabled for a dimension, the frame also depends on the four side
/// neighbours (ore enclosure is judged across chunk borders), so their
/// revisions join the key.
#[derive(Debug, Default)]
pub struct ChunkPacketCache {
    pub(super) entries: HashMap<(mc_core::dimensions::Dimension, (i32, i32)), CachedChunk>,
    pub(super) anti_xray: Option<Arc<crate::anti_xray::AntiXray>>,
    /// Chunk revision deltas between consecutive publishes of this cache.
    pub(crate) deltas: ChunkDeltaLog,
}

/// What an encoded chunk frame was computed from.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ChunkKey {
    revision: u64,
    /// `-x`, `+x`, `-z`, `+z` neighbour revisions (0 = absent); all 0 when the
    /// frame does not depend on neighbours.
    sides: [u64; 4],
}

#[derive(Debug)]
struct CachedChunk {
    key: ChunkKey,
    /// Final framed (compressed) packet; viewers share it by refcount.
    frame: bytes::Bytes,
}

impl ChunkPacketCache {
    /// Empty cache without anti-xray.
    pub fn new() -> Self {
        Self::default()
    }

    /// Empty cache whose chunks are anti-xray processed per `policy`.
    pub fn with_anti_xray(policy: Option<Arc<crate::anti_xray::AntiXray>>) -> Self {
        Self {
            entries: HashMap::default(),
            anti_xray: policy,
            deltas: ChunkDeltaLog::default(),
        }
    }

    /// The anti-xray policy of this cache's chunks, if any dimension has one.
    pub(crate) fn anti_xray(&self) -> Option<&crate::anti_xray::AntiXray> {
        self.anti_xray.as_deref()
    }

    pub(super) fn key(
        &self,
        dimension: mc_core::dimensions::Dimension,
        chunks: &[SnapshotChunk],
        chunk: &SnapshotChunk,
    ) -> ChunkKey {
        chunk_key(self.anti_xray(), dimension, chunks, chunk)
    }

    pub(super) fn get(
        &self,
        dimension: mc_core::dimensions::Dimension,
        pos: (i32, i32),
        key: ChunkKey,
    ) -> Option<&bytes::Bytes> {
        let entry = self.entries.get(&(dimension, pos))?;
        (entry.key == key).then_some(&entry.frame)
    }

    /// Drop entries whose chunk is gone or whose content key moved on.
    pub fn retain_resident(&mut self, output: &TickOutput) {
        let policy = self.anti_xray.as_deref();
        self.entries.retain(|(dimension, pos), entry| {
            output
                .in_dimension(*dimension)
                .and_then(|output| {
                    let chunks = &output.snapshot.chunks;
                    find_chunk(chunks, *pos)
                        .map(|chunk| chunk_key(policy, *dimension, chunks, chunk))
                })
                .is_some_and(|key| key == entry.key)
        });
    }
}

/// Content key of `chunk`'s frame: its own revision plus, when anti-xray
/// applies to `dimension`, the revisions of its four side neighbours.
fn chunk_key(
    policy: Option<&crate::anti_xray::AntiXray>,
    dimension: mc_core::dimensions::Dimension,
    chunks: &[SnapshotChunk],
    chunk: &SnapshotChunk,
) -> ChunkKey {
    let mut sides = [0; 4];
    if policy.is_some_and(|policy| policy.enabled(dimension)) {
        let (x, z) = (chunk.pos.x, chunk.pos.z);
        for (side, pos) in sides
            .iter_mut()
            .zip([(x - 1, z), (x + 1, z), (x, z - 1), (x, z + 1)])
        {
            *side = find_chunk(chunks, pos).map_or(0, |resident| resident.revision);
        }
    }
    ChunkKey {
        revision: chunk.revision,
        sides,
    }
}

/// Resident chunk at `pos` (snapshot chunks are sorted by `(x, z)`).
pub(crate) fn find_chunk(chunks: &[SnapshotChunk], pos: (i32, i32)) -> Option<&SnapshotChunk> {
    chunks
        .binary_search_by_key(&pos, |chunk| (chunk.pos.x, chunk.pos.z))
        .ok()
        .map(|i| &chunks[i])
}

/// Encode the chunk packets `plan_viewer` left empty (cache misses): each
/// distinct chunk once, in parallel; cache and fill every placeholder.
/// Returns placeholders whose chunk failed to encode (removed from
/// `planned`), for the caller to mark unsent.
pub fn fill_chunk_packets(
    output: &TickOutput,
    planned: &mut [(ConnectionId, Vec<PlannedPacket>)],
    registries: &PlanRegistries<'_>,
    cache: &mut ChunkPacketCache,
) -> Vec<(ConnectionId, (i32, i32))> {
    use rayon::prelude::*;
    let mut missing: Vec<(mc_core::dimensions::Dimension, (i32, i32))> = planned
        .iter()
        .flat_map(|(_, packets)| packets.iter())
        .filter_map(|packet| match packet.chunk {
            Some(pos) if packet.data.is_empty() => Some((packet.chunk_dimension, pos)),
            _ => None,
        })
        .collect();
    missing.sort_unstable();
    missing.dedup();
    type Encoded = (
        (mc_core::dimensions::Dimension, (i32, i32)),
        ChunkKey,
        Option<bytes::Bytes>,
    );
    let policy = cache.anti_xray.as_deref();
    let encoded: Vec<Encoded> = missing
        .into_par_iter()
        .map(|(dimension, pos)| {
            let Some(output) = output.in_dimension(dimension) else {
                return ((dimension, pos), ChunkKey::default(), None);
            };
            let chunks = &output.snapshot.chunks;
            let key = find_chunk(chunks, pos).map_or_else(ChunkKey::default, |chunk| {
                chunk_key(policy, dimension, chunks, chunk)
            });
            (
                (dimension, pos),
                key,
                encode_chunk(chunks, pos, dimension, registries, policy),
            )
        })
        .collect();
    let mut failed_chunks = HashSet::default();
    for (pos, key, frame) in encoded {
        match frame {
            Some(frame) => {
                cache.entries.insert(pos, CachedChunk { key, frame });
            }
            None => {
                failed_chunks.insert(pos);
            }
        }
    }
    let mut failed = Vec::new();
    for (connection, packets) in planned.iter_mut() {
        packets.retain_mut(|packet| {
            let Some(pos) = packet.chunk else {
                return true;
            };
            if !packet.data.is_empty() {
                return true;
            }
            if failed_chunks.contains(&(packet.chunk_dimension, pos)) {
                failed.push((*connection, pos));
                return false;
            }
            packet.data = cache.entries[&(packet.chunk_dimension, pos)].frame.clone();
            true
        });
    }
    failed
}

/// Encode and frame (compress) one chunk packet, or `None` when the chunk
/// is not resident or does not encode. `policy` hides enclosed ores.
fn encode_chunk(
    chunks: &[SnapshotChunk],
    pos: (i32, i32),
    dimension: mc_core::dimensions::Dimension,
    registries: &PlanRegistries<'_>,
    policy: Option<&crate::anti_xray::AntiXray>,
) -> Option<bytes::Bytes> {
    let centre = find_chunk(chunks, pos)?;
    let hidden = policy.map_or_else(Vec::new, |policy| {
        policy.hide(
            dimension,
            &crate::anti_xray::Surroundings::of(chunks, centre),
        )
    });
    let packet = crate::chunks::encode_chunk_packet_with_sections(
        centre.pos,
        &centre.chunk,
        crate::chunks::EncodeInputs {
            biomes: registries.biomes,
            blocks: registries.blocks,
            block_entity_types: registries.block_entity_types,
        },
        &hidden,
    )
    .ok()?;
    let mut body = Vec::new();
    packet.encode(&mut body).ok()?;
    mc_protocol::frame::encode_frame(
        mc_protocol::packets::play::LevelChunkWithLight::ID,
        &body,
        Compression::Threshold(256),
    )
    .ok()
}

/// Chunk packets per viewer per publish, nearest-first (vanilla
/// `PlayerChunkSender` paces too). An unpaced first publish encoded a whole
/// view square (289 chunks at view 8: ~11 s in a debug build) on the pump,
/// stalling every viewer.
pub(super) const CHUNKS_PER_TICK: usize = 16;

/// Nearest-first spiral of the Chebyshev square around `center`.
pub(super) fn spiral_square(center: (i32, i32), view_distance: i32) -> Vec<(i32, i32)> {
    let radius = view_distance.max(0);
    let mut cells: Vec<(i32, i32, i32)> = Vec::new();
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            let ring = dx.abs().max(dz.abs());
            cells.push((center.0 + dx, center.1 + dz, ring));
        }
    }
    cells.sort_by_key(|(_, _, ring)| *ring);
    cells.into_iter().map(|(x, z, _)| (x, z)).collect()
}
