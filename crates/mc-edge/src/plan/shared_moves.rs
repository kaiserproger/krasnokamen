//! Shared per-snapshot entity move frames across viewers.
//!
//! One [`SharedMoves`] covers a single immutable snapshot publication:
//! every viewer plans from the same entity images, so the wire frame for
//! one `(entity id, exact viewer baseline)` pair is identical and encoded
//! once. Viewers keep their own baselines (credit deferral and late joins
//! diverge); only identical `(id, baseline)` pairs share a frame.
//!
//! Sharded `parking_lot::Mutex<FxHashMap>` keeps rayon viewers off a single
//! lock. The lock does not poison: a panic inside a shard leaves the cached
//! frames usable for the rest of that snapshot (the panic itself still ends the
//! pump task, which the accept loop treats as a server-wide failure).
//! Below-threshold (`None`) moves are cached too, skipping repeated threshold
//! math. A cache MUST be dropped with its snapshot: the entity image is the
//! implicit half of every key.

use super::*;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

/// Shard count (power of two; the mask keeps shard selection cheap).
const SHARDS: usize = 64;

/// Exact `(entity id, viewer baseline)` key. Floats compare by bits: a
/// conservative exact match, so only truly identical baselines share.
/// All baseline fields are in the key even though the move math ignores
/// the head baseline; a stale-sharer would corrupt someone's delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct MoveKey {
    id: i32,
    px: u64,
    py: u64,
    pz: u64,
    yaw: u32,
    pitch: u32,
    head: u32,
}

impl MoveKey {
    fn new(id: EntityId, baseline: MoveBaseline) -> Self {
        let (pos, yaw, pitch, head) = baseline;
        Self {
            id: id.0,
            px: pos[0].to_bits(),
            py: pos[1].to_bits(),
            pz: pos[2].to_bits(),
            yaw: yaw.to_bits(),
            pitch: pitch.to_bits(),
            head: head.to_bits(),
        }
    }
}

/// One shared movement frame: packet id, framed wire bytes, and the
/// baseline the client holds once the frame is delivered.
#[derive(Debug, Clone)]
pub(super) struct SharedMove {
    pub(super) id: i32,
    pub(super) data: bytes::Bytes,
    pub(super) next: MoveBaseline,
}

/// One move shard: its map plus, when counting is on, that shard's hit/miss tallies. The
/// tallies sit under the lock the lookup already takes, so counting adds no atomic and no extra
/// contended cache line — a viewer's rayon thread never waits on a shared counter.
#[derive(Debug, Default)]
struct Shard {
    frames: FxHashMap<MoveKey, Option<SharedMove>>,
    hits: u64,
    misses: u64,
    counting: bool,
}

/// Thread-safe move/head frame cache for one snapshot publication.
/// Share `&self` across rayon viewers; drop with the snapshot.
#[derive(Debug)]
pub(super) struct SharedMoves {
    moves: [Mutex<Shard>; SHARDS],
    heads: [Mutex<FxHashMap<i32, (i32, bytes::Bytes)>>; SHARDS],
}

impl SharedMoves {
    /// A cache that counts its hits and misses. The pump asks for this once per publish from
    /// `mc_edge::server::plan`'s DEBUG state, so a server with that line off pays nothing.
    pub(crate) fn counting() -> Self {
        Self::with_tallies(true)
    }

    fn with_tallies(counting: bool) -> Self {
        Self {
            moves: std::array::from_fn(|_| {
                Mutex::new(Shard {
                    counting,
                    ..Shard::default()
                })
            }),
            heads: std::array::from_fn(|_| Mutex::new(FxHashMap::default())),
        }
    }
}

impl Default for SharedMoves {
    fn default() -> Self {
        Self::with_tallies(false)
    }
}

impl SharedMoves {
    /// Shared movement frame for `entity` against a viewer `baseline`;
    /// `None` when the move is below vanilla thresholds. A hit clones no
    /// body bytes; a miss encodes once through the shared `MovePacket`
    /// path and caches the frame (including `None`).
    pub(super) fn movement(
        &self,
        entity: &EntityView,
        baseline: MoveBaseline,
    ) -> Option<SharedMove> {
        let key = MoveKey::new(entity.id, baseline);
        let shard = &self.moves[(key.id as usize) & (SHARDS - 1)];
        let mut guard = shard.lock();
        let counting = guard.counting;
        if let Some(cached) = guard.frames.get(&key).cloned() {
            if counting {
                guard.hits += 1;
            }
            return cached;
        }
        if counting {
            guard.misses += 1;
        }
        let value = entity_movement(entity, baseline).map(|(packet, next)| {
            let (id, data) = match packet {
                MovePacket::Pos(packet) => frame(&packet),
                MovePacket::Rot(packet) => frame(&packet),
                MovePacket::PosRot(packet) => frame(&packet),
                MovePacket::Sync(packet) => frame(&packet),
            };
            SharedMove { id, data, next }
        });
        guard.frames.insert(key, value.clone());
        value
    }

    /// (hit, miss) move-frame tallies since this snapshot's cache was created.
    pub(crate) fn move_tallies(&self) -> (u64, u64) {
        self.moves.iter().fold((0u64, 0u64), |(hits, misses), shard| {
            let guard = shard.lock();
            (hits + guard.hits, misses + guard.misses)
        })
    }

    /// Shared `RotateHead` frame for `entity`'s current head yaw, keyed by
    /// entity id only: it never depends on a viewer's movement baseline.
    /// The caller wraps `(id, data)` into its own `PlannedPacket`.
    pub(super) fn head(&self, entity: &EntityView) -> (i32, bytes::Bytes) {
        let shard = &self.heads[(entity.id.0 as usize) & (SHARDS - 1)];
        let mut guard = shard.lock();
        guard
            .entry(entity.id.0)
            .or_insert_with(|| {
                frame(&RotateHead {
                    entity_id: entity.id.0,
                    head_yaw: entity.head_yaw,
                })
            })
            .clone()
    }

    /// Test support: panic while holding a move shard's lock, the way an
    /// unexpected encode panic would. A poisonable lock would then refuse
    /// every later viewer's frame; parking-lot keeps the shard usable.
    #[cfg(test)]
    pub(super) fn probe_panic_in_move_shard(&self, id: i32) {
        let shard = &self.moves[(id as usize) & (SHARDS - 1)];
        let _guard = shard.lock();
        panic!("shared move encoder panics with its shard locked");
    }
}

/// Movement/head bodies are fixed-size numeric fields (at most 68 bytes).
/// Encode into stack storage, allocating only the final shared wire frame.
fn frame<P: Packet>(packet: &P) -> (i32, bytes::Bytes) {
    let mut body = [0_u8; 128];
    let mut remaining = &mut body[..];
    packet
        .encode(&mut remaining)
        .expect("numeric entity packet encodes");
    let len = 128 - remaining.len();
    let mut data = BytesMut::with_capacity(len + 11);
    append_frame(&mut data, P::ID, &body[..len], Compression::Threshold(256))
        .expect("numeric entity frame fits");
    (P::ID, data.freeze())
}
