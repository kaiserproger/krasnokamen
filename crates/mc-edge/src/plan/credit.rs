//! World-queue credit and best-effort framing.
//!
//! One publish's room in the outbound world queue: a group that does not fit
//! is removed again and planned from a newer snapshot, never dropped.
use super::*;

/// One publish's world-queue room ([`crate::server::WorldSender`]): free
/// frame slots, a hard limit, and free bytes, a soft one. Any remaining byte
/// credit admits one more packet group, so the queue overshoots its byte
/// bound by at most one group and a group larger than the bound still goes
/// out once the queue drains. Chunk cache misses are framed after planning
/// and charged by the next publish's credit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorldCredit {
    pub(crate) frames: usize,
    pub(crate) bytes: usize,
}

impl WorldCredit {
    /// Frame-slot credit without a byte limit.
    #[cfg(test)]
    pub(crate) const fn frames(frames: usize) -> Self {
        Self {
            frames,
            bytes: usize::MAX,
        }
    }

    /// No further world frame fits this publish.
    pub(crate) const fn spent(self) -> bool {
        self.frames == 0 || self.bytes == 0
    }

    pub(super) fn charge(&mut self, packets: &[PlannedPacket]) {
        self.frames -= packets.len();
        self.bytes = self
            .bytes
            .saturating_sub(packets.iter().map(|packet| packet.data.len()).sum());
    }
}

/// Keep the best-effort frames `push` adds only if they fit the sampled credit.
/// A group that does not fit is removed and planned again from a newer snapshot.
/// A later queue race can still refuse a planned frame; the pump then repairs
/// retained geometry/entity state, while one-shot visuals may be lost.
pub(crate) fn try_push(
    out: &mut Vec<PlannedPacket>,
    credit: &mut WorldCredit,
    push: impl FnOnce(&mut Vec<PlannedPacket>),
) -> bool {
    if credit.spent() {
        return false;
    }
    let before = out.len();
    push(out);
    if out.len() - before > credit.frames {
        out.truncate(before);
        return false;
    }
    credit.charge(&out[before..]);
    true
}

/// Frame `body` once (threshold-256 compression) and push it; nothing is
/// pushed when it does not frame (oversize), like an encode failure.
fn push_framed(
    out: &mut Vec<PlannedPacket>,
    connection: ConnectionId,
    id: i32,
    body: &[u8],
    reliable: bool,
) {
    let mut data = BytesMut::new();
    if append_frame(&mut data, id, body, Compression::Threshold(256)).is_err() {
        return;
    }
    out.push(PlannedPacket {
        connection,
        id,
        reliable,
        data: data.freeze(),
        chunk: None,
        chunk_dimension: mc_core::dimensions::Dimension::Overworld,
    });
}

pub(crate) fn push_reliable<P: Packet>(
    out: &mut Vec<PlannedPacket>,
    connection: ConnectionId,
    packet: &P,
) {
    let mut body = Vec::new();
    if crate::vanilla_items::encode(connection, packet, &mut body).is_ok() {
        push_framed(out, connection, P::ID, &body, true);
    }
}

pub(crate) fn push_best_effort<P: Packet>(
    out: &mut Vec<PlannedPacket>,
    connection: ConnectionId,
    packet: &P,
) {
    let mut body = Vec::new();
    if crate::vanilla_items::encode(connection, packet, &mut body).is_ok() {
        push_framed(out, connection, P::ID, &body, false);
    }
}
