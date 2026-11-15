//! Connection-local chunk flow control (26.3 PlayerChunkSender metadata).

#[derive(Debug)]
pub(crate) struct ChunkBatchState {
    desired: f32,
    quota: f32,
    unacknowledged: usize,
    limit: usize,
}

impl Default for ChunkBatchState {
    fn default() -> Self {
        Self {
            desired: 9.0,
            quota: 0.0,
            unacknowledged: 0,
            limit: 1,
        }
    }
}

impl ChunkBatchState {
    /// One publication advances credit only while another batch is allowed.
    pub(crate) fn allowance(&mut self) -> usize {
        if self.unacknowledged >= self.limit {
            return 0;
        }
        self.quota = (self.quota + self.desired).min(self.desired.max(1.0));
        (self.quota.floor() as usize).min(super::CHUNKS_PER_TICK)
    }

    /// Called only after all queue slots have been reserved, before exposure.
    pub(crate) fn sent(&mut self, chunks: usize) {
        self.unacknowledged += 1;
        self.quota -= chunks as f32;
    }

    pub(crate) fn acknowledge(&mut self, desired: f32) {
        self.unacknowledged = self.unacknowledged.saturating_sub(1);
        self.desired = if desired.is_nan() {
            0.01
        } else {
            desired.clamp(0.01, 64.0)
        };
        if self.unacknowledged == 0 {
            self.quota = 1.0;
        }
        self.limit = 10;
    }

    /// A dimension fence discarded a whole, never-written batch. It cannot ACK.
    pub(crate) fn discarded(&mut self) {
        self.unacknowledged = self.unacknowledged.saturating_sub(1);
    }
}

#[cfg(test)]
#[path = "chunk_batch_tests.rs"]
mod tests;
