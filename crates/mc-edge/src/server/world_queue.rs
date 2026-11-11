//! Per-connection world queue: a bounded frame FIFO that also counts the
//! bytes it holds.
//!
//! Before each publish the pump reads both as the viewer's world credit.
//! Frame slots are a hard limit. Queued bytes are a soft one: any remaining
//! byte credit admits one more packet group, so a frame larger than the
//! bound still goes out once the queue drains, and nothing planned is ever
//! dropped. Bytes, not frames, are what a slow socket backs up on: the byte
//! bound caps how stale a slow client's world view can get, while the frame
//! bound caps per-frame queue memory.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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

/// Producer half; clones share the byte count.
#[derive(Debug, Clone)]
pub struct WorldSender {
    frames: mpsc::Sender<bytes::Bytes>,
    queued: Arc<AtomicUsize>,
    byte_bound: usize,
}

/// Consumer half, owned by the connection's writer task.
#[derive(Debug)]
pub struct WorldReceiver {
    frames: mpsc::Receiver<bytes::Bytes>,
    queued: Arc<AtomicUsize>,
}

/// A world queue of `frames` slots and a soft `bytes` bound.
#[must_use]
pub fn world_queue(frames: usize, bytes: usize) -> (WorldSender, WorldReceiver) {
    let (sender, receiver) = mpsc::channel(frames);
    let queued = Arc::new(AtomicUsize::new(0));
    (
        WorldSender {
            frames: sender,
            queued: Arc::clone(&queued),
            byte_bound: bytes,
        },
        WorldReceiver {
            frames: receiver,
            queued,
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
        self.frames.try_send(frame).inspect_err(|_| {
            self.queued.fetch_sub(len, Ordering::AcqRel);
        })
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
    /// Next item; `None` once every sender is gone. Cancel safe: nothing
    /// awaits after the item is taken.
    pub async fn recv(&mut self) -> Option<bytes::Bytes> {
        let frame = self.frames.recv().await?;
        self.taken(&frame);
        Some(frame)
    }

    pub fn try_recv(&mut self) -> Result<bytes::Bytes, TryRecvError> {
        let frame = self.frames.try_recv()?;
        self.taken(&frame);
        Ok(frame)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    fn taken(&self, frame: &bytes::Bytes) {
        self.queued.fetch_sub(frame.len(), Ordering::AcqRel);
    }
}

#[cfg(test)]
#[path = "world_queue_tests.rs"]
mod world_queue_tests;
