//! Connection-local chunk flow control (26.3 PlayerChunkSender metadata).

use std::collections::VecDeque;

#[derive(Debug)]
pub(crate) struct ChunkBatchState {
    desired: f32,
    quota: f32,
    unacknowledged: VecDeque<(u64, usize)>,
    next_batch: u64,
    limit: usize,
}

impl Default for ChunkBatchState {
    fn default() -> Self {
        Self {
            desired: 9.0,
            quota: 0.0,
            unacknowledged: VecDeque::new(),
            next_batch: 0,
            limit: 1,
        }
    }
}

impl ChunkBatchState {
    /// One publication advances credit only while another batch is allowed.
    pub(crate) fn allowance(&mut self) -> usize {
        if self.unacknowledged.len() >= self.limit {
            return 0;
        }
        self.quota = (self.quota + self.desired).min(self.desired.max(1.0));
        (self.quota.floor() as usize).min(super::CHUNKS_PER_TICK)
    }

    /// Recheck the actual queue admission, after any intervening ACK.
    pub(crate) fn can_send(&self, chunks: usize) -> bool {
        chunks > 0
            && chunks <= super::CHUNKS_PER_TICK
            && self.unacknowledged.len() < self.limit
            && chunks as f32 <= self.quota
    }

    /// Called under the admission lock, before the queue exposes the group.
    pub(crate) fn sent(&mut self, chunks: usize) -> u64 {
        let id = self.next_batch;
        self.next_batch = self.next_batch.wrapping_add(1);
        self.unacknowledged.push_back((id, chunks));
        self.quota -= chunks as f32;
        id
    }

    pub(crate) fn acknowledge(&mut self, desired: f32) {
        // Vanilla accepts even an unsolicited ACK. With no wire batch ID a
        // duplicate cannot be distinguished from a legitimate oldest ACK.
        self.unacknowledged.pop_front();
        self.desired = if desired.is_nan() {
            0.01
        } else {
            desired.clamp(0.01, 64.0)
        };
        if self.unacknowledged.is_empty() {
            self.quota = 1.0;
        }
        self.limit = 10;
    }

    /// Refund only this never-written group's still-unacknowledged work.
    pub(crate) fn discarded(&mut self, id: u64) {
        if let Some(index) = self
            .unacknowledged
            .iter()
            .position(|&(batch, _)| batch == id)
        {
            let (_, chunks) = self.unacknowledged.remove(index).unwrap();
            self.quota = (self.quota + chunks as f32).min(self.desired.max(1.0));
        }
    }
}

#[cfg(test)]
#[path = "chunk_batch_tests.rs"]
mod tests;
