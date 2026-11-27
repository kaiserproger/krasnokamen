//! Edge glue for the `krasnokamen:voice` relay: connection lifetime, native pose
//! publication and best-effort delivery. Policy lives in `voice.rs`.
use std::{collections::HashMap, sync::Arc, time::Instant};

use mc_core::boundary::{ConnectionId, TickOutput};
use mc_data::Identifier;
use mc_domain::EntityId;
use mc_protocol::packets::{CustomPayload, Packet, play::ServerboundCustomPayload};
use uuid::Uuid;

use crate::{
    server::EdgeServer,
    voice::{CHANNEL, Delivery, Relay, audio, decode},
};

pub(crate) struct Service {
    pub(crate) relay: parking_lot::Mutex<Relay>,
    pub(crate) svc: crate::svc_service::Svc,
}

impl Default for Service {
    fn default() -> Self {
        Self::new(&crate::voice::VoiceConfig::default())
    }
}

impl Service {
    pub(crate) fn new(config: &crate::voice::VoiceConfig) -> Self {
        Self {
            relay: parking_lot::Mutex::new(Relay::new(config)),
            svc: crate::svc_service::Svc::new(config),
        }
    }

    fn tick(&self, server: &EdgeServer, tick: &TickOutput) {
        let entities = self.relay.lock().entities();
        let mut poses = HashMap::with_capacity(entities.len());
        for (connection, entity) in entities {
            if let Some(pose) = crate::screen_service::published_pose(tick, entity) {
                poses.insert(connection, pose);
            }
        }
        let ended = {
            let mut relay = self.relay.lock();
            relay.set_poses(poses);
            relay.flush_idle(Instant::now())
        };
        for delivery in ended {
            dispatch(server, &delivery, &[]);
        }
    }
}

/// The payload of a `krasnokamen:voice` Play custom payload, if it is one.
pub(crate) fn payload(body: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
    let mut body = body;
    let packet = ServerboundCustomPayload::decode(&mut body)?;
    if let CustomPayload::Unknown { channel, payload } = packet.payload
        && channel.as_str() == CHANNEL
    {
        return Ok(Some(payload));
    }
    Ok(None)
}

pub(crate) fn frame_on(channel: &str, payload: &[u8]) -> anyhow::Result<bytes::Bytes> {
    crate::lod::frame(Identifier::parse(channel)?, payload.to_vec())
}

/// Per-connection handle: dropping it unregisters the connection.
pub(crate) struct Connection {
    server: Arc<EdgeServer>,
    id: ConnectionId,
}
impl Drop for Connection {
    fn drop(&mut self) {
        let left = self.server.voice.relay.lock().unregister(self.id);
        if let Some(uuid) = left {
            crate::svc_service::player_left(&self.server, uuid);
        }
    }
}
impl Connection {
    pub(crate) fn start(
        server: Arc<EdgeServer>,
        id: ConnectionId,
        entity: EntityId,
        uuid: Uuid,
        name: String,
    ) -> Self {
        let state = server
            .voice
            .relay
            .lock()
            .register(id, entity, uuid, name, Instant::now());
        if let Ok(framed) = frame_on(CHANNEL, &state)
            && let Some(handles) = server.connections.get(id)
        {
            let _ = handles.reliable_tx.try_send(framed);
        }
        crate::svc_service::player_changed(&server, id);
        Self { server, id }
    }

    pub(crate) fn receive(&self, bytes: &[u8]) {
        let Some(message) = decode(bytes) else {
            return;
        };
        let (delivery, opus) = {
            let mut relay = self.server.voice.relay.lock();
            let opus = match &message {
                crate::voice::Message::Audio { opus, .. } => *opus,
                crate::voice::Message::Listen(_) => &[][..],
            };
            (relay.handle(self.id, message, Instant::now()), opus)
        };
        if let Some(delivery) = delivery {
            dispatch(&self.server, &delivery, opus);
        }
    }
}

/// Audio is droppable: it never retires a connection and is not queued when the
/// listener's reliable queue is less than half free (headroom for control packets).
/// `opus` empty = end of speech (SVC listeners only).
pub(crate) fn dispatch(server: &EdgeServer, delivery: &Delivery, opus: &[u8]) {
    if !delivery.native.is_empty()
        && let Ok(framed) = frame_on(
            CHANNEL,
            &audio(
                delivery.speaker,
                delivery.eye,
                delivery.sequence as u32,
                opus,
            ),
        )
    {
        for to in &delivery.native {
            if let Some(handles) = server.connections.get(*to)
                && handles.reliable_tx.capacity() * 2 >= handles.reliable_tx.max_capacity()
            {
                let _ = handles.reliable_tx.try_send(framed.clone());
            }
        }
    }
    if !delivery.svc.is_empty() || !delivery.group_svc.is_empty() {
        server.voice.svc.send_sound(delivery, opus);
    }
}

pub(crate) fn spawn_tick_task(server: Arc<EdgeServer>) -> tokio::task::JoinHandle<()> {
    let mut ticks = server.hub.subscribe();
    tokio::spawn(async move {
        loop {
            match ticks.recv().await {
                Ok(tick) => server.voice.tick(&server, &tick),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}
