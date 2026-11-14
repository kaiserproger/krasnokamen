//! Simple Voice Chat bridge glue (ADR 0019): `voicechat:*` plugin channels, the
//! encrypted UDP socket, keep-alives and delivery of relayed audio. Routing
//! policy is `voice.rs`; wire/crypto is `svc.rs`. Never on the tick.
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use mc_core::boundary::{ConnectionId, EntityId};
use mc_protocol::packets::{CustomPayload, Packet, play::ServerboundCustomPayload};
use uuid::Uuid;

use crate::{
    server::EdgeServer,
    svc::{self, Secret, Udp, channel},
    voice::{Delivery, VoiceConfig},
    voice_service::frame_on,
};

/// SVC drops a validated connection after ten missed keep-alives.
const TIMEOUT: Duration = Duration::from_millis(svc::KEEP_ALIVE_MS as u64 * 10);
const RECV_BUFFER: usize = 4096;

struct Peer {
    uuid: Uuid,
    secret: Secret,
    /// Set by the first valid `Authenticate`.
    addr: Option<SocketAddr>,
    validated: bool,
    last_keep_alive: Instant,
}

#[derive(Default)]
struct State {
    peers: HashMap<ConnectionId, Peer>,
    by_uuid: HashMap<Uuid, ConnectionId>,
    by_addr: HashMap<SocketAddr, ConnectionId>,
}
impl State {
    fn remove(&mut self, id: ConnectionId) {
        if let Some(peer) = self.peers.remove(&id) {
            self.by_uuid.remove(&peer.uuid);
            if let Some(addr) = peer.addr {
                self.by_addr.remove(&addr);
            }
        }
    }
}

pub(crate) struct Svc {
    enabled: bool,
    bind: String,
    port: u16,
    voice_host: String,
    distance: f64,
    socket: OnceLock<Arc<tokio::net::UdpSocket>>,
    state: parking_lot::Mutex<State>,
}

impl Svc {
    pub(crate) fn new(config: &VoiceConfig) -> Self {
        Self {
            enabled: config.enabled && config.svc,
            bind: config.svc_bind_address.clone(),
            port: config.svc_port,
            voice_host: config.svc_voice_host.clone(),
            distance: f64::from(config.range_blocks),
            socket: OnceLock::new(),
            state: parking_lot::Mutex::new(State::default()),
        }
    }

    /// UDP port clients are told (the bound port).
    fn public_port(&self) -> u16 {
        self.socket
            .get()
            .and_then(|socket| socket.local_addr().ok())
            .map_or(self.port, |addr| addr.port())
    }

    fn send_raw(&self, bytes: &[u8], to: SocketAddr) {
        if let Some(socket) = self.socket.get() {
            // Voice is droppable: a full socket buffer loses the packet.
            let _ = socket.try_send_to(bytes, to);
        }
    }

    fn send_sealed(&self, secret: &Secret, to: SocketAddr, message: &[u8]) {
        match svc::seal_datagram(secret, message) {
            Ok(bytes) => self.send_raw(&bytes, to),
            Err(error) => tracing::warn!(%error, "voice datagram not encodable"),
        }
    }

    /// Relayed audio for SVC listeners; each listener has its own key.
    pub(crate) fn send_sound(&self, delivery: &Delivery, opus: &[u8]) {
        for (ids, group) in [(&delivery.svc, false), (&delivery.group_svc, true)] {
            if ids.is_empty() {
                continue;
            }
            let message = if group {
                svc::group_sound(delivery.speaker, opus, delivery.sequence)
            } else {
                svc::player_sound(
                    delivery.speaker,
                    opus,
                    delivery.sequence,
                    delivery.whispering,
                    delivery.range,
                )
            };
            let targets: Vec<(SocketAddr, Secret)> = {
                let state = self.state.lock();
                ids.iter()
                    .filter_map(|id| {
                        let peer = state.peers.get(id)?;
                        (peer.validated).then(|| Some((peer.addr?, peer.secret.clone())))?
                    })
                    .collect()
            };
            for (addr, secret) in targets {
                self.send_sealed(&secret, addr, &message);
            }
        }
    }
}

/// The `voicechat:*` Play custom payload, if it is one.
pub(crate) fn payload(body: &[u8]) -> Option<(String, Vec<u8>)> {
    let mut body = body;
    match ServerboundCustomPayload::decode(&mut body).ok()?.payload {
        CustomPayload::Unknown { channel, payload }
            if channel.as_str().split_once(':').map(|(ns, _)| ns) == Some(channel::NAMESPACE) =>
        {
            Some((channel.as_str().to_owned(), payload))
        }
        _ => None,
    }
}

fn send_plugin(server: &EdgeServer, to: ConnectionId, channel: &str, body: &[u8]) {
    if let Ok(framed) = frame_on(channel, body)
        && let Some(handles) = server.connections.get(to)
    {
        let _ = handles.reliable_tx.try_send(framed);
    }
}

/// A native or SVC player registered or changed state: tell SVC clients.
pub(crate) fn player_changed(server: &EdgeServer, id: ConnectionId) {
    if !server.voice.svc.enabled {
        return;
    }
    let (state, listeners) = {
        let relay = server.voice.relay.lock();
        (relay.state(id), relay.svc_connections())
    };
    if let Some(state) = state {
        let body = svc::state_packet(&state);
        for to in listeners {
            send_plugin(server, to, channel::STATE, &body);
        }
    }
}

/// A player left: SVC clients forget its state.
pub(crate) fn player_left(server: &EdgeServer, uuid: Uuid) {
    if !server.voice.svc.enabled {
        return;
    }
    let listeners = server.voice.relay.lock().svc_connections();
    let body = svc::remove_state_packet(uuid);
    for to in listeners {
        send_plugin(server, to, channel::REMOVE_STATE, &body);
    }
}

fn broadcast_plugin(server: &EdgeServer, channel: &str, body: &[u8]) {
    let listeners = server.voice.relay.lock().svc_connections();
    for to in listeners {
        send_plugin(server, to, channel, body);
    }
}

fn reap_groups(server: &EdgeServer) {
    let removed = server.voice.relay.lock().reap_groups();
    for id in removed {
        broadcast_plugin(server, channel::REMOVE_GROUP, id.as_bytes());
    }
}

/// Per-connection handle for any client: dropping it ends its voice chat peer.
pub(crate) struct Connection {
    server: Arc<EdgeServer>,
    id: ConnectionId,
    entity: EntityId,
    uuid: Uuid,
    name: String,
}
impl Drop for Connection {
    fn drop(&mut self) {
        let svc = &self.server.voice.svc;
        let was_peer = {
            let mut state = svc.state.lock();
            let was_peer = state.peers.contains_key(&self.id);
            state.remove(self.id);
            was_peer
        };
        if was_peer {
            let left = self.server.voice.relay.lock().unregister(self.id);
            if let Some(uuid) = left {
                player_left(&self.server, uuid);
            }
            reap_groups(&self.server);
        }
    }
}
impl Connection {
    /// `None` when SVC support is off. Announces the serverbound channels so a
    /// Fabric/NeoForge client may send its secret request.
    pub(crate) fn start(
        server: Arc<EdgeServer>,
        id: ConnectionId,
        entity: EntityId,
        uuid: Uuid,
        name: String,
    ) -> Option<Self> {
        if !server.voice.svc.enabled {
            return None;
        }
        send_plugin(&server, id, "minecraft:register", &svc::register_body());
        Some(Self {
            server,
            id,
            entity,
            uuid,
            name,
        })
    }

    pub(crate) fn receive(&self, channel: &str, payload: &[u8]) {
        let server = &self.server;
        match channel {
            channel::REQUEST_SECRET => {
                let Some(version) = svc::parse_request_secret(payload) else {
                    return;
                };
                if version != svc::COMPATIBILITY_VERSION {
                    tracing::warn!(
                        connection = self.id.0,
                        client = version,
                        server = svc::COMPATIBILITY_VERSION,
                        "voice chat client version is not compatible"
                    );
                    return;
                }
                self.issue_secret();
            }
            channel::UPDATE_STATE => {
                let Some(disabled) = svc::parse_update_state(payload) else {
                    return;
                };
                {
                    let mut relay = server.voice.relay.lock();
                    if !relay.is_svc(self.id) {
                        return;
                    }
                    relay.set_svc_disabled(self.id, disabled);
                }
                player_changed(server, self.id);
            }
            channel::CREATE_GROUP => {
                let Some(group) = svc::parse_create_group(payload) else {
                    return;
                };
                let id = group.id;
                let body = svc::add_group(&group);
                if !server.voice.relay.lock().create_group(self.id, group) {
                    return;
                }
                broadcast_plugin(server, channel::ADD_GROUP, &body);
                player_changed(server, self.id);
                send_plugin(
                    server,
                    self.id,
                    channel::JOINED_GROUP,
                    &svc::joined_group(Some(id), false),
                );
                reap_groups(server);
            }
            channel::SET_GROUP => {
                let Some((id, password)) = svc::parse_join_group(payload) else {
                    return;
                };
                let result = server
                    .voice
                    .relay
                    .lock()
                    .join_group(self.id, id, password.as_deref());
                match result {
                    Some(true) => {
                        player_changed(server, self.id);
                        send_plugin(
                            server,
                            self.id,
                            channel::JOINED_GROUP,
                            &svc::joined_group(Some(id), false),
                        );
                        reap_groups(server);
                    }
                    Some(false) => send_plugin(
                        server,
                        self.id,
                        channel::JOINED_GROUP,
                        &svc::joined_group(None, true),
                    ),
                    None => {}
                }
            }
            channel::LEAVE_GROUP if payload.is_empty() => {
                let left = server.voice.relay.lock().leave_group(self.id);
                if left {
                    player_changed(server, self.id);
                    send_plugin(
                        server,
                        self.id,
                        channel::JOINED_GROUP,
                        &svc::joined_group(None, false),
                    );
                    reap_groups(server);
                }
            }
            _ => {}
        }
    }

    /// SVC `initializePlayerConnection`: states, then the secret.
    fn issue_secret(&self) {
        issue_secret(&self.server, self.id, self.entity, self.uuid, &self.name);
    }
}

fn issue_secret(
    server: &Arc<EdgeServer>,
    id: ConnectionId,
    entity: EntityId,
    uuid: Uuid,
    name: &str,
) {
    let svc_state = &server.voice.svc;
    let secret = match Secret::random() {
        Ok(secret) => secret,
        Err(error) => {
            tracing::warn!(%error, "voice secret unavailable");
            return;
        }
    };
    {
        let mut state = svc_state.state.lock();
        if state.peers.contains_key(&id) {
            // SVC: "Player already requested secret - ignoring".
            return;
        }
        state.by_uuid.insert(uuid, id);
        state.peers.insert(
            id,
            Peer {
                uuid,
                secret: secret.clone(),
                addr: None,
                validated: false,
                last_keep_alive: Instant::now(),
            },
        );
    }
    let (states, groups) = {
        let mut relay = server.voice.relay.lock();
        if !relay.is_svc(id) {
            relay.register_svc(id, entity, uuid, name.to_owned(), Instant::now());
        }
        (
            relay.states(),
            relay.groups().map(svc::add_group).collect::<Vec<_>>(),
        )
    };
    send_plugin(server, id, channel::STATES, &svc::states_packet(&states));
    for group in groups {
        send_plugin(server, id, channel::ADD_GROUP, &group);
    }
    player_changed(server, id);
    send_plugin(
        server,
        id,
        channel::SECRET,
        &svc::secret_packet(&svc::SecretInfo {
            secret: &secret,
            port: svc_state.public_port(),
            player: uuid,
            distance: svc_state.distance,
            voice_host: &svc_state.voice_host,
        }),
    );
}

impl Svc {
    /// One UDP datagram from `addr`.
    fn on_datagram(&self, server: &Arc<EdgeServer>, addr: SocketAddr, bytes: &[u8]) {
        let Some(datagram) = svc::parse_datagram(bytes) else {
            return;
        };
        if datagram.player == svc::PING_V1 {
            if let Some(reply) = svc::ping_reply(datagram.payload) {
                self.send_raw(&reply, addr);
            }
            return;
        }
        let secret = {
            let state = self.state.lock();
            let Some(id) = state.by_uuid.get(&datagram.player) else {
                return;
            };
            let Some(peer) = state.peers.get(id) else {
                return;
            };
            peer.secret.clone()
        };
        let Some(plain) = secret.decrypt(datagram.payload) else {
            return;
        };
        let Some(message) = svc::parse_udp(&plain) else {
            return;
        };
        let now = Instant::now();
        match message {
            Udp::Authenticate {
                player,
                secret: offered,
            } => {
                if player != datagram.player || offered != secret {
                    return;
                }
                let reply_to = {
                    let mut state = self.state.lock();
                    let Some(id) = state.by_uuid.get(&player).copied() else {
                        return;
                    };
                    let Some(peer) = state.peers.get_mut(&id) else {
                        return;
                    };
                    if peer.addr.is_none() {
                        peer.addr = Some(addr);
                        state.by_addr.insert(addr, id);
                    }
                    state.peers.get(&id).and_then(|peer| peer.addr)
                };
                if let Some(to) = reply_to {
                    self.send_sealed(&secret, to, &svc::authenticate_ack());
                }
            }
            Udp::ConnectionCheck => {
                let newly = {
                    let mut state = self.state.lock();
                    let Some(id) = state.by_addr.get(&addr).copied() else {
                        return;
                    };
                    let Some(peer) = state.peers.get_mut(&id) else {
                        return;
                    };
                    if peer.uuid != datagram.player {
                        return;
                    }
                    let newly = !peer.validated;
                    peer.validated = true;
                    peer.last_keep_alive = now;
                    newly.then_some(id)
                };
                if let Some(id) = newly {
                    server.voice.relay.lock().set_svc_connected(id, true);
                    player_changed(server, id);
                    tracing::info!(connection = id.0, "voice chat client connected");
                }
                self.send_sealed(&secret, addr, &svc::connection_check_ack());
            }
            Udp::KeepAlive => {
                let mut state = self.state.lock();
                if let Some(id) = state.by_addr.get(&addr).copied()
                    && let Some(peer) = state.peers.get_mut(&id)
                    && peer.validated
                {
                    peer.last_keep_alive = now;
                }
            }
            Udp::Mic {
                data,
                sequence,
                whispering,
            } => {
                let id = {
                    let state = self.state.lock();
                    let Some(id) = state.by_addr.get(&addr).copied() else {
                        return;
                    };
                    match state.peers.get(&id) {
                        Some(peer) if peer.validated && peer.uuid == datagram.player => id,
                        _ => return,
                    }
                };
                let delivery = server
                    .voice
                    .relay
                    .lock()
                    .audio(id, sequence, data, whispering, now);
                if let Some(delivery) = delivery {
                    crate::voice_service::dispatch(server, &delivery, data);
                }
            }
            Udp::Ignored => {}
        }
    }

    /// Once a second: keep-alives to validated peers; peers silent for 10 s are
    /// reset and issued a fresh secret (as SVC does).
    fn keep_alive(&self, server: &Arc<EdgeServer>) {
        let now = Instant::now();
        let mut alive = Vec::new();
        let mut timed_out = Vec::new();
        {
            let mut state = self.state.lock();
            for (id, peer) in &state.peers {
                if !peer.validated {
                    continue;
                }
                match peer.addr {
                    Some(addr) if now.duration_since(peer.last_keep_alive) < TIMEOUT => {
                        alive.push((addr, peer.secret.clone()));
                    }
                    _ => timed_out.push((*id, peer.uuid)),
                }
            }
            for (id, _) in &timed_out {
                state.remove(*id);
            }
        }
        let message = svc::keep_alive();
        for (addr, secret) in alive {
            self.send_sealed(&secret, addr, &message);
        }
        for (id, uuid) in timed_out {
            tracing::info!(connection = id.0, "voice chat client timed out");
            let entity_name = {
                let mut relay = server.voice.relay.lock();
                relay.set_svc_connected(id, false);
                relay
                    .entities()
                    .into_iter()
                    .find(|(connection, _)| *connection == id)
                    .zip(relay.state(id))
            };
            player_changed(server, id);
            if let Some(((_, entity), state)) = entity_name {
                issue_secret(server, id, entity, uuid, &state.name);
            }
        }
    }
}

/// Bind the UDP socket and serve it. `None` when SVC support is off or the
/// socket cannot be bound (logged; native voice keeps working).
pub(crate) async fn spawn(server: Arc<EdgeServer>) -> Option<tokio::task::JoinHandle<()>> {
    let svc_state = &server.voice.svc;
    if !svc_state.enabled {
        return None;
    }
    let address = format!("{}:{}", svc_state.bind, svc_state.port);
    let socket = match tokio::net::UdpSocket::bind(&address).await {
        Ok(socket) => Arc::new(socket),
        Err(error) => {
            tracing::error!(%error, %address, "voice chat UDP socket unavailable; SVC bridge disabled");
            return None;
        }
    };
    tracing::info!(%address, "Simple Voice Chat bridge listening (UDP)");
    let _ = svc_state.socket.set(Arc::clone(&socket));
    Some(tokio::spawn(async move {
        let mut buffer = vec![0u8; RECV_BUFFER];
        let mut keep_alive =
            tokio::time::interval(Duration::from_millis(svc::KEEP_ALIVE_MS as u64));
        keep_alive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                received = socket.recv_from(&mut buffer) => match received {
                    Ok((len, addr)) => server.voice.svc.on_datagram(&server, addr, &buffer[..len]),
                    Err(error) => tracing::debug!(%error, "voice chat UDP receive failed"),
                },
                _ = keep_alive.tick() => server.voice.svc.keep_alive(&server),
            }
        }
    }))
}

#[cfg(test)]
#[path = "svc_service_tests.rs"]
mod tests;
