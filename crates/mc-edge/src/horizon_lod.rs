//! Optional Horizon VERSION 1 far-terrain presentation.
mod codec;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Instant;

use mc_data::Identifier;
use mc_protocol::packets::{CustomPayload, Packet, play::ServerboundCustomPayload};

use crate::server::EdgeServer;

fn channel() -> Identifier {
    Identifier::parse("horizon:lod").expect("static channel")
}

pub(crate) fn offer(distance: u32) -> CustomPayload {
    CustomPayload::Unknown {
        channel: channel(),
        payload: codec::offer(distance),
    }
}

pub(crate) fn payload(mut body: &[u8]) -> Option<Vec<u8>> {
    match ServerboundCustomPayload::decode(&mut body).ok()?.payload {
        CustomPayload::Unknown {
            channel: id,
            payload,
        } if body.is_empty() && id == channel() => Some(payload),
        _ => None,
    }
}

pub(crate) struct Connection {
    entity: mc_core::boundary::EntityId,
    ticks: tokio::sync::broadcast::Receiver<mc_core::boundary::TickOutput>,
    anchor: [f64; 2],
    inflight: Arc<AtomicUsize>,
    in_overworld: bool,
    tokens: f64,
    last: Instant,
}

impl Connection {
    pub(crate) fn new(
        spawn: [f64; 3],
        entity: mc_core::boundary::EntityId,
        ticks: tokio::sync::broadcast::Receiver<mc_core::boundary::TickOutput>,
        in_overworld: bool,
    ) -> Self {
        Self {
            entity,
            ticks,
            anchor: [spawn[0], spawn[2]],
            inflight: Arc::default(),
            tokens: 256.0,
            last: Instant::now(),
            in_overworld,
        }
    }

    pub(crate) fn request(
        &mut self,
        server: &Arc<EdgeServer>,
        connection: mc_core::boundary::ConnectionId,
        bytes: &[u8],
    ) {
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.last).as_secs_f64() * 64.0).min(256.0);
        self.last = now;
        if self.tokens < 1.0 {
            return;
        }
        self.tokens -= 1.0;
        let Ok(request) = codec::request(bytes) else {
            return;
        };
        loop {
            match self.ticks.try_recv() {
                Ok(tick) => {
                    if let Some(player) = tick.snapshot.entities.get(self.entity) {
                        self.anchor = [player.position[0], player.position[2]];
                        self.in_overworld = true;
                    } else {
                        self.in_overworld = false;
                    }
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                    self.in_overworld = false;
                }
                Err(
                    tokio::sync::broadcast::error::TryRecvError::Empty
                    | tokio::sync::broadcast::error::TryRecvError::Closed,
                ) => break,
            }
        }
        let stride = request.stride;
        let size = stride.saturating_mul(16);
        let near = allowed(request, server.cfg.horizon_lod.distance_chunks, self.anchor);
        let reject = || {
            if let Ok(frame) = crate::lod::frame(channel(), codec::refusal(request.id))
                && let Some(handles) = server.connections.get(connection)
            {
                let _ = handles.world_tx.try_send(frame);
            }
        };
        if !near || !self.in_overworld || server.hub.session_locked(connection) {
            reject();
            return;
        }
        if self.inflight.fetch_add(1, Ordering::AcqRel) >= 8 {
            self.inflight.fetch_sub(1, Ordering::AcqRel);
            reject();
            return;
        }
        let inflight = Arc::clone(&self.inflight);
        let workers = Arc::clone(&server.lod.as_ref().expect("shared lod lane").workers);
        let server = Arc::clone(server);
        tokio::spawn(async move {
            let _done = Done(inflight);
            let Ok(_permit) = workers.acquire_owned().await else {
                return;
            };
            let hub = Arc::clone(&server.hub);
            let result = tokio::task::spawn_blocking(move || {
                hub.lod_tile(
                    request.x.div_euclid(size),
                    request.z.div_euclid(size),
                    stride.trailing_zeros() as u8,
                    true,
                )
            })
            .await;
            let payload = match result {
                Ok(Ok(samples)) => codec::reply(request.id, &samples),
                _ => codec::refusal(request.id),
            };
            if !server.hub.session_locked(connection)
                && let Ok(frame) = crate::lod::frame(channel(), payload)
                && let Some(handles) = server.connections.get(connection)
            {
                let _ = handles.world_tx.try_send(frame);
            }
        });
    }
}
struct Done(Arc<AtomicUsize>);
impl Drop for Done {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn allowed(request: codec::Request, distance: u32, anchor: [f64; 2]) -> bool {
    let stride = request.stride;
    if stride <= 0
        || (stride & (stride - 1)) != 0
        || stride > distance.next_power_of_two().min(1024) as i32
    {
        return false;
    }
    let size = stride * 16;
    if request.x.rem_euclid(size) != 0 || request.z.rem_euclid(size) != 0 {
        return false;
    }
    let gap = |a: f64, b: i32| {
        (f64::from(b) - a)
            .max(a - f64::from(b) - f64::from(size))
            .max(0.0)
    };
    let dx = gap(anchor[0], request.x);
    let dz = gap(anchor[1], request.z);
    dx * dx + dz * dz <= f64::from((distance + 8) * 16).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizon_lod_distance_stride_and_alignment() {
        let mut request = codec::Request {
            id: 1,
            x: 256,
            z: 0,
            stride: 1,
        };
        assert!(allowed(request, 8, [0.0, 0.0]));
        request.x = 272;
        assert!(!allowed(request, 8, [0.0, 0.0]));
        assert!(allowed(request, 8, [16.0, 0.0]));
        request.x = 1;
        assert!(!allowed(request, 8, [0.0, 0.0]));
        request.x = 0;
        for stride in [0, -1, 3, 16] {
            request.stride = stride;
            assert!(!allowed(request, 8, [0.0, 0.0]));
        }
    }
}
