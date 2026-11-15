//! Per-connection world queue: a bounded frame FIFO that also counts the
//! bytes it holds.
//!
//! Before each publish the pump reads both as the viewer's world credit.
//! Frame slots are a hard limit. Queued bytes are a soft one: any remaining
//! byte credit admits one more packet group, so a frame larger than the
//! bound still goes out once the queue drains. A competing producer may still
//! consume the last frame slot after planning, so a planned send can fail.
//! Bytes, not frames, are what a slow socket backs up on: the byte bound caps
//! world-view backlog, while the frame bound caps per-frame queue memory.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use tokio::sync::mpsc;
use tokio::sync::mpsc::error::{TryRecvError, TrySendError};

/// Frame slots of a player's world queue. A full view of a 20,000-unit
/// battle needs about 6,700 moves plus as many head turns per tick (vanilla
/// 3-tick cadence, about 150 KB); on a slower link the byte bound decides
/// how many go out.
pub const WORLD_QUEUE_FRAMES: usize = 16_384;
/// Queued-byte bound of a player's world queue: about 0.8 s of backlog on a
/// 10 Mbit/s link before planning waits for the socket.
pub const WORLD_QUEUE_BYTES: usize = 1 << 20;
/// Frame slots of a recorder's world queue: one complete 20,000-unit tick
/// (moves, heads, swings, hurts) with room for the next.
pub const RECORDER_WORLD_QUEUE_FRAMES: usize = 65_536;

// A move and a head turn per due unit (vanilla 3-tick cadence) for two ticks.
const _: () = assert!(RECORDER_WORLD_QUEUE_FRAMES >= 2 * 2 * 20_000_usize.div_ceil(3));

#[derive(Debug)]
struct WorldFrame {
    bytes: bytes::Bytes,
    additional: Vec<bytes::Bytes>,
    byte_len: usize,
    generation: u64,
    // A batch is one indivisible writer item, but consumes all wire frame
    // slots. These unused permits stay charged until the item is taken.
    reservations: Vec<mpsc::OwnedPermit<WorldFrame>>,
    chunk_batch: Option<(
        Arc<parking_lot::Mutex<crate::plan::chunk_batch::ChunkBatchState>>,
        u64,
    )>,
}

/// Producer half; clones share the byte count.
#[derive(Debug, Clone)]
pub struct WorldSender {
    frames: mpsc::Sender<WorldFrame>,
    queued: Arc<AtomicUsize>,
    byte_bound: usize,
    generation: Arc<AtomicU64>,
}

/// Consumer half, owned by the connection's writer task.
#[derive(Debug)]
pub struct WorldReceiver {
    frames: mpsc::Receiver<WorldFrame>,
    queued: Arc<AtomicUsize>,
    generation: Arc<AtomicU64>,
    batch: std::vec::IntoIter<bytes::Bytes>,
}

/// A world queue of `frames` slots and a soft `bytes` bound.
#[must_use]
pub fn world_queue(frames: usize, bytes: usize) -> (WorldSender, WorldReceiver) {
    let (sender, receiver) = mpsc::channel(frames);
    let queued = Arc::new(AtomicUsize::new(0));
    let generation = Arc::new(AtomicU64::new(0));
    (
        WorldSender {
            frames: sender,
            queued: Arc::clone(&queued),
            byte_bound: bytes,
            generation: Arc::clone(&generation),
        },
        WorldReceiver {
            frames: receiver,
            queued,
            generation,
            batch: Vec::new().into_iter(),
        },
    )
}

/// The world queue of a new connection: recorder bounds for a recorder
/// ([`crate::recorder`]), player bounds otherwise.
#[must_use]
pub fn world_queue_for(
    config: &crate::recorder::RecorderSection,
    recorder: bool,
) -> (WorldSender, WorldReceiver) {
    if recorder {
        world_queue(RECORDER_WORLD_QUEUE_FRAMES, config.world_queue_bytes)
    } else {
        world_queue(WORLD_QUEUE_FRAMES, WORLD_QUEUE_BYTES)
    }
}

impl WorldSender {
    /// Queue one item (one or more whole frames) when a slot is free. The
    /// byte bound never refuses: it is credit for the planner, not a drop.
    pub fn try_send(&self, frame: bytes::Bytes) -> Result<(), TrySendError<bytes::Bytes>> {
        let len = frame.len();
        // Counted before the receiver can see the frame, so its decrement
        // never runs ahead of this increment.
        self.queued.fetch_add(len, Ordering::AcqRel);
        self.frames
            .try_send(WorldFrame {
                bytes: frame,
                additional: Vec::new(),
                byte_len: len,
                generation: self.generation.load(Ordering::Acquire),
                reservations: Vec::new(),
                chunk_batch: None,
            })
            .map_err(|error| {
                self.queued.fetch_sub(len, Ordering::AcqRel);
                match error {
                    TrySendError::Full(frame) => TrySendError::Full(frame.bytes),
                    TrySendError::Closed(frame) => TrySendError::Closed(frame.bytes),
                }
            })
    }

    /// Reserve every batch frame or queue nothing. A single writer item keeps
    /// framing intact across reliable dimension resets and competing LOD sends.
    pub(crate) fn try_send_chunk_batch(
        &self,
        frames: &[bytes::Bytes],
        chunks: usize,
        state: &Arc<parking_lot::Mutex<crate::plan::chunk_batch::ChunkBatchState>>,
    ) -> Result<(), ()> {
        let mut batch = state.lock();
        if !batch.can_send(chunks) {
            return Err(());
        }
        let mut reservations = Vec::with_capacity(frames.len());
        for _ in frames {
            reservations.push(self.frames.clone().try_reserve_owned().map_err(|_| ())?);
        }
        let sender = reservations.pop().ok_or(())?;
        let len = frames.iter().map(bytes::Bytes::len).sum();
        self.queued.fetch_add(len, Ordering::AcqRel);
        let id = batch.sent(chunks);
        sender.send(WorldFrame {
            bytes: frames[0].clone(),
            additional: frames[1..].to_vec(),
            byte_len: len,
            generation: self.generation.load(Ordering::Acquire),
            reservations,
            chunk_batch: Some((Arc::clone(state), id)),
        });
        Ok(())
    }

    /// Fence queued world frames before enqueueing a reliable dimension reset.
    /// Frames already taken by the writer precede that reset in its byte stream.
    pub(crate) fn reset(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Free frame slots (includes outstanding reservations, like `mpsc`).
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.frames.capacity()
    }

    #[must_use]
    pub fn max_capacity(&self) -> usize {
        self.frames.max_capacity()
    }

    /// Bytes the planner may still queue before the soft bound.
    #[must_use]
    pub fn free_bytes(&self) -> usize {
        self.byte_bound.saturating_sub(self.queued_bytes())
    }

    /// Bytes queued and not yet taken by the writer.
    #[must_use]
    pub fn queued_bytes(&self) -> usize {
        self.queued.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.frames.is_closed()
    }
}

impl WorldReceiver {
    /// Next current-generation item; `None` once every sender is gone.
    /// Cancel safe: only stale items can be discarded before another await.
    pub async fn recv(&mut self) -> Option<bytes::Bytes> {
        if let Some(frame) = self.next_in_batch() {
            return Some(frame);
        }
        loop {
            let frame = self.frames.recv().await?;
            if let Some(bytes) = self.taken(frame) {
                return Some(bytes);
            }
        }
    }

    pub fn try_recv(&mut self) -> Result<bytes::Bytes, TryRecvError> {
        if let Some(frame) = self.next_in_batch() {
            return Ok(frame);
        }
        loop {
            let frame = self.frames.try_recv()?;
            if let Some(bytes) = self.taken(frame) {
                return Ok(bytes);
            }
        }
    }

    /// Wire frame slots queued or reserved, including grouped batch frames.
    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.max_capacity() - self.frames.capacity() + self.batch.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty() && self.batch.len() == 0
    }

    /// The writer drains a taken batch before selecting another queue, so a
    /// reliable dimension reset cannot split its start/chunks/finish sequence.
    pub(crate) fn next_in_batch(&mut self) -> Option<bytes::Bytes> {
        self.batch.next()
    }

    fn taken(&mut self, frame: WorldFrame) -> Option<bytes::Bytes> {
        self.queued.fetch_sub(frame.byte_len, Ordering::AcqRel);
        // Releasing the reservations here matches an ordinary frame's slot.
        drop(frame.reservations);
        if frame.generation == self.generation.load(Ordering::Acquire) {
            self.batch = frame.additional.into_iter();
            Some(frame.bytes)
        } else {
            if let Some((state, id)) = frame.chunk_batch {
                state.lock().discarded(id);
            }
            None
        }
    }
}

#[cfg(test)]
#[path = "world_queue_tests.rs"]
mod world_queue_tests;
