//! Local entity census for redstone sensors (pressure plates, detector rails,
//! wooden buttons).
//!
//! Bottleneck: a poll of one idle plate used to sort every live body into an
//! AABB tree first (about 5.6 % of the native-100 tick thread, profiled
//! 2026-11-12), although a poll needs the bodies inside one block.
//! Boundary: candidates come from the per-chunk membership index that
//! `EntityStore` already maintains on every spawn, removal and chunk-crossing
//! move, so a query costs the bodies of the 1 to 4 chunks its box reaches.
//! Correctness fence: a body is indexed under the chunk of its feet position and
//! overlaps a query only if its position lies within `max_half_width` of the
//! box, so that chunk range holds every overlapping body; the overlap test,
//! liveness, session and spectator filters are the former tree's, and the
//! result is a count, which does not depend on visit order. Removal path:
//! `entity_query_tests` compares this against the former full-population tree.
use crate::boundary::ChunkPos;
use crate::entities::EntitySlot;
use crate::tick::TickState;

#[derive(Default)]
pub(super) struct EntityQuery {
    /// Candidate slots of the current query; the allocation is reused.
    candidates: Vec<EntitySlot>,
    #[cfg(test)]
    pub(super) visited: usize,
}

impl EntityQuery {
    /// Number of accepted bodies overlapping the open box `(min, max)`, at most `limit`.
    pub(super) fn count(
        &mut self,
        state: &TickState,
        min: [f64; 3],
        max: [f64; 3],
        limit: usize,
        accepts: impl Fn(EntitySlot) -> bool,
    ) -> usize {
        let reach = state.entities.max_half_width();
        let chunk = |coordinate: f64| (coordinate.floor() as i32).div_euclid(16);
        self.candidates.clear();
        for z in chunk(min[2] - reach)..=chunk(max[2] + reach) {
            for x in chunk(min[0] - reach)..=chunk(max[0] + reach) {
                state
                    .entities
                    .for_each_slot_in_chunk(ChunkPos { x, z }, |slot| {
                        let body = state.entities.body(slot);
                        let pos = state.entities.pos(slot);
                        let lo = [pos[0] - body.half_width, pos[1], pos[2] - body.half_width];
                        let hi = [
                            pos[0] + body.half_width,
                            pos[1] + body.height,
                            pos[2] + body.half_width,
                        ];
                        let valid = (0..3).all(|axis| {
                            lo[axis].is_finite() && hi[axis].is_finite() && lo[axis] <= hi[axis]
                        });
                        if valid && (0..3).all(|axis| hi[axis] > min[axis] && lo[axis] < max[axis])
                        {
                            self.candidates.push(slot);
                        }
                    });
            }
        }
        #[cfg(test)]
        {
            self.visited = self.candidates.len();
        }
        // An id whose kind changed can list under both membership maps of its chunk.
        self.candidates.sort_unstable();
        self.candidates.dedup();
        let mut count = 0;
        for &slot in &self.candidates {
            if count == limit {
                break;
            }
            let body = state.entities.body(slot);
            if body.alive
                && !state.entity_session_locked(body.id)
                && !state
                    .players
                    .get(&body.id)
                    .is_some_and(|player| player.spectator())
                && accepts(slot)
            {
                count += 1;
            }
        }
        count
    }
}
