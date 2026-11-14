//! `krasnokamen:voice` proximity relay (ADR 0019). Pure: no I/O and no clock;
//! callers pass `Instant`s and deliver the returned payloads. The server never
//! decodes Opus: it bounds, rate-limits and forwards each packet to listeners in
//! the same dimension within range of the speaker's published pose.
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use bytes::Bytes;
use mc_core::boundary::{ConnectionId, EntityId};
use serde::Deserialize;
use uuid::Uuid;

use crate::screen::Pose;

pub(crate) const CHANNEL: &str = "krasnokamen:voice";
const VERSION: u8 = 1;
pub(crate) const MAX_OPUS_BYTES: usize = 1000;

const PACKETS_PER_SECOND: f64 = 60.0;
const PACKET_BURST: f64 = 20.0;
const BYTES_PER_SECOND: f64 = 65_536.0;
const BYTES_BURST: f64 = 16_384.0;

mod client_kind {
    pub const AUDIO: u8 = 0;
    pub const LISTEN: u8 = 1;
}
mod server_kind {
    pub const AUDIO: u8 = 0;
    pub const VOICE_STATE: u8 = 1;
}

/// `[voice]`: host-owned configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    pub enabled: bool,
    /// Hearing range in blocks, eye to eye.
    pub range_blocks: u16,
    /// Serve Simple Voice Chat clients (ADR 0019): a UDP socket next to the game port.
    pub svc: bool,
    pub svc_port: u16,
    pub svc_bind_address: String,
    /// Host (optionally `host:port`) SVC clients should use for UDP; empty =
    /// the address they reached the game server on.
    pub svc_voice_host: String,
}
impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            range_blocks: 48,
            svc: false,
            svc_port: 24454,
            svc_bind_address: "0.0.0.0".into(),
            svc_voice_host: String::new(),
        }
    }
}
impl VoiceConfig {
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=1024).contains(&self.range_blocks),
            "voice.range_blocks must be 1..=1024"
        );
        if self.svc {
            anyhow::ensure!(self.enabled, "voice.svc requires voice.enabled");
            self.svc_bind_address
                .parse::<std::net::IpAddr>()
                .map_err(|error| anyhow::anyhow!("voice.svc_bind_address: {error}"))?;
        }
        Ok(())
    }
}

/// Client -> server message.
#[derive(Debug, PartialEq)]
pub(crate) enum Message<'a> {
    Audio { seq: u32, opus: &'a [u8] },
    Listen(bool),
}

pub(crate) fn decode(bytes: &[u8]) -> Option<Message<'_>> {
    let ([version, kind], body) = bytes.split_first_chunk::<2>()?;
    if *version != VERSION {
        return None;
    }
    match *kind {
        client_kind::AUDIO => {
            let (seq, opus) = body.split_first_chunk::<4>()?;
            (1..=MAX_OPUS_BYTES)
                .contains(&opus.len())
                .then(|| Message::Audio {
                    seq: u32::from_be_bytes(*seq),
                    opus,
                })
        }
        client_kind::LISTEN => match body {
            [0] => Some(Message::Listen(false)),
            [1] => Some(Message::Listen(true)),
            _ => None,
        },
        _ => None,
    }
}

/// `VOICE_STATE`: `u8 enabled, u16 range_blocks`.
pub(crate) fn voice_state(enabled: bool, range_blocks: u16) -> Bytes {
    let mut out = vec![VERSION, server_kind::VOICE_STATE, u8::from(enabled)];
    out.extend(range_blocks.to_be_bytes());
    Bytes::from(out)
}

pub(crate) fn audio(speaker: Uuid, eye: [f64; 3], seq: u32, opus: &[u8]) -> Bytes {
    let mut out = Vec::with_capacity(34 + opus.len());
    out.extend([VERSION, server_kind::AUDIO]);
    out.extend(speaker.as_bytes());
    for axis in eye {
        out.extend((axis as f32).to_be_bytes());
    }
    out.extend(seq.to_be_bytes());
    out.extend(opus);
    Bytes::from(out)
}

/// Who hears one packet. `opus` stays with the caller; the relay never copies it.
#[derive(Debug, PartialEq)]
pub(crate) struct Delivery {
    pub speaker: Uuid,
    /// Speaker eye position.
    pub eye: [f64; 3],
    pub sequence: i64,
    pub whispering: bool,
    /// Hearing range used, blocks.
    pub range: f32,
    /// Native `krasnokamen:voice` listeners.
    pub native: Vec<ConnectionId>,
    /// Simple Voice Chat listeners (ADR 0019 "Simple Voice Chat bridge").
    pub svc: Vec<ConnectionId>,
    /// SVC group listeners receive non-positional audio.
    pub group_svc: Vec<ConnectionId>,
}

/// Native speakers send nothing between bursts; SVC listeners need an explicit
/// end-of-speech packet, so a burst ends after this much silence.
const SPEECH_GAP: Duration = Duration::from_millis(200);

enum Kind {
    /// `krasnokamen:voice`; hears only after `LISTEN(1)`.
    Native { listening: bool },
    /// Simple Voice Chat client; hears once its UDP connection is validated and
    /// it has not disabled voice chat.
    Svc { connected: bool, disabled: bool },
}
impl Kind {
    fn hears(&self) -> bool {
        match self {
            Self::Native { listening } => *listening,
            Self::Svc {
                connected,
                disabled,
            } => *connected && !*disabled,
        }
    }
}

struct Talk {
    last: Instant,
    sequence: i64,
}

struct Player {
    entity: EntityId,
    uuid: Uuid,
    name: String,
    kind: Kind,
    group: Option<Uuid>,
    packets: f64,
    bytes: f64,
    updated: Instant,
    /// Native speaker mid-burst.
    talking: Option<Talk>,
}

pub(crate) struct Relay {
    enabled: bool,
    range: f64,
    range_blocks: u16,
    players: HashMap<ConnectionId, Player>,
    poses: HashMap<ConnectionId, Pose>,
    groups: HashMap<Uuid, crate::svc::Group>,
}

impl Relay {
    pub(crate) fn new(config: &VoiceConfig) -> Self {
        Self {
            enabled: config.enabled,
            range: f64::from(config.range_blocks),
            range_blocks: config.range_blocks,
            players: HashMap::new(),
            poses: HashMap::new(),
            groups: HashMap::new(),
        }
    }

    fn insert(
        &mut self,
        id: ConnectionId,
        entity: EntityId,
        uuid: Uuid,
        name: String,
        kind: Kind,
        now: Instant,
    ) {
        self.players.insert(
            id,
            Player {
                entity,
                uuid,
                name,
                kind,
                group: None,
                packets: PACKET_BURST,
                bytes: BYTES_BURST,
                updated: now,
                talking: None,
            },
        );
    }

    /// Register a native client and return its `VOICE_STATE`.
    pub(crate) fn register(
        &mut self,
        id: ConnectionId,
        entity: EntityId,
        uuid: Uuid,
        name: String,
        now: Instant,
    ) -> Bytes {
        self.insert(
            id,
            entity,
            uuid,
            name,
            Kind::Native { listening: false },
            now,
        );
        voice_state(self.enabled, self.range_blocks)
    }

    /// Register a Simple Voice Chat client that completed the plugin-channel
    /// handshake (UDP not yet validated).
    pub(crate) fn register_svc(
        &mut self,
        id: ConnectionId,
        entity: EntityId,
        uuid: Uuid,
        name: String,
        now: Instant,
    ) {
        self.insert(
            id,
            entity,
            uuid,
            name,
            Kind::Svc {
                connected: false,
                disabled: false,
            },
            now,
        );
    }

    pub(crate) fn is_svc(&self, id: ConnectionId) -> bool {
        matches!(
            self.players.get(&id),
            Some(Player {
                kind: Kind::Svc { .. },
                ..
            })
        )
    }

    pub(crate) fn set_svc_connected(&mut self, id: ConnectionId, value: bool) {
        if let Some(Player {
            kind: Kind::Svc { connected, .. },
            ..
        }) = self.players.get_mut(&id)
        {
            *connected = value;
        }
    }

    pub(crate) fn set_svc_disabled(&mut self, id: ConnectionId, value: bool) {
        if let Some(Player {
            kind: Kind::Svc { disabled, .. },
            ..
        }) = self.players.get_mut(&id)
        {
            *disabled = value;
        }
    }

    /// `PlayerState` of one registered player as SVC clients see it.
    pub(crate) fn state(&self, id: ConnectionId) -> Option<crate::svc::State> {
        let player = self.players.get(&id)?;
        let (disabled, disconnected) = match player.kind {
            Kind::Native { .. } => (false, false),
            Kind::Svc {
                connected,
                disabled,
            } => (disabled, !connected),
        };
        Some(crate::svc::State {
            uuid: player.uuid,
            name: player.name.clone(),
            disabled,
            disconnected,
            group: player.group,
        })
    }

    pub(crate) fn states(&self) -> Vec<crate::svc::State> {
        self.players
            .keys()
            .filter_map(|id| self.state(*id))
            .collect()
    }

    pub(crate) fn svc_connections(&self) -> Vec<ConnectionId> {
        self.players
            .iter()
            .filter(|(_, player)| matches!(player.kind, Kind::Svc { .. }))
            .map(|(id, _)| *id)
            .collect()
    }

    pub(crate) fn groups(&self) -> impl Iterator<Item = &crate::svc::Group> {
        self.groups.values()
    }

    pub(crate) fn create_group(&mut self, player: ConnectionId, group: crate::svc::Group) -> bool {
        if !self.is_svc(player) {
            return false;
        }
        let id = group.id;
        self.groups.insert(id, group);
        self.players.get_mut(&player).expect("SVC player").group = Some(id);
        true
    }

    /// None means unknown player/group; false means wrong password.
    pub(crate) fn join_group(
        &mut self,
        player: ConnectionId,
        group: Uuid,
        password: Option<&str>,
    ) -> Option<bool> {
        if !self.is_svc(player) {
            return None;
        }
        let wanted = self.groups.get(&group)?;
        if wanted
            .password
            .as_deref()
            .is_some_and(|expected| Some(expected) != password)
        {
            return Some(false);
        }
        self.players.get_mut(&player)?.group = Some(group);
        Some(true)
    }

    pub(crate) fn leave_group(&mut self, player: ConnectionId) -> bool {
        if !self.is_svc(player) {
            return false;
        }
        self.players.get_mut(&player).expect("SVC player").group = None;
        true
    }

    pub(crate) fn reap_groups(&mut self) -> Vec<Uuid> {
        let used: std::collections::HashSet<Uuid> =
            self.players.values().filter_map(|p| p.group).collect();
        let removed: Vec<Uuid> = self
            .groups
            .keys()
            .filter(|id| !used.contains(id))
            .copied()
            .collect();
        for id in &removed {
            self.groups.remove(id);
        }
        removed
    }

    /// Remove a player; the uuid is returned so SVC peers can drop its state.
    pub(crate) fn unregister(&mut self, id: ConnectionId) -> Option<Uuid> {
        self.poses.remove(&id);
        self.players.remove(&id).map(|player| player.uuid)
    }

    /// Connections and the entity whose pose `set_poses` needs.
    pub(crate) fn entities(&self) -> Vec<(ConnectionId, EntityId)> {
        self.players
            .iter()
            .map(|(id, player)| (*id, player.entity))
            .collect()
    }

    pub(crate) fn set_poses(&mut self, poses: HashMap<ConnectionId, Pose>) {
        self.poses = poses;
    }

    /// A message from a native client.
    pub(crate) fn handle(
        &mut self,
        from: ConnectionId,
        message: Message<'_>,
        now: Instant,
    ) -> Option<Delivery> {
        if !self.enabled {
            return None;
        }
        match message {
            Message::Listen(on) => {
                if let Some(Player {
                    kind: Kind::Native { listening },
                    ..
                }) = self.players.get_mut(&from)
                {
                    *listening = on;
                }
                None
            }
            Message::Audio { seq, opus } => self.audio(from, i64::from(seq), opus, false, now),
        }
    }

    /// One audio packet (empty `opus` = SVC end of speech) from any speaker.
    pub(crate) fn audio(
        &mut self,
        from: ConnectionId,
        sequence: i64,
        opus: &[u8],
        whispering: bool,
        now: Instant,
    ) -> Option<Delivery> {
        if !self.enabled {
            return None;
        }
        let speaker = self.players.get_mut(&from)?;
        let elapsed = now.saturating_duration_since(speaker.updated).as_secs_f64();
        speaker.updated = now;
        speaker.packets = (speaker.packets + elapsed * PACKETS_PER_SECOND).min(PACKET_BURST);
        speaker.bytes = (speaker.bytes + elapsed * BYTES_PER_SECOND).min(BYTES_BURST);
        let cost = (opus.len() + 6) as f64;
        if speaker.packets < 1.0 || speaker.bytes < cost {
            return None;
        }
        speaker.packets -= 1.0;
        speaker.bytes -= cost;
        if matches!(speaker.kind, Kind::Native { .. }) {
            speaker.talking = Some(Talk {
                last: now,
                sequence,
            });
        }
        let uuid = speaker.uuid;
        let range = if whispering {
            self.range / 2.0
        } else {
            self.range
        };
        self.delivery(from, uuid, sequence, whispering, range, !opus.is_empty())
    }

    /// Native bursts that have been silent for [`SPEECH_GAP`]: an end-of-speech
    /// delivery for their SVC listeners.
    pub(crate) fn flush_idle(&mut self, now: Instant) -> Vec<Delivery> {
        let ended: Vec<(ConnectionId, Uuid, i64)> = self
            .players
            .iter_mut()
            .filter_map(|(id, player)| {
                let talk = player.talking.as_ref()?;
                (now.saturating_duration_since(talk.last) >= SPEECH_GAP)
                    .then(|| (*id, player.uuid, talk.sequence.wrapping_add(1)))
            })
            .collect();
        let mut out = Vec::new();
        for (id, uuid, sequence) in ended {
            if let Some(player) = self.players.get_mut(&id) {
                player.talking = None;
            }
            out.extend(self.delivery(id, uuid, sequence, false, self.range, false));
        }
        out
    }

    fn delivery(
        &self,
        from: ConnectionId,
        uuid: Uuid,
        sequence: i64,
        whispering: bool,
        range: f64,
        to_native: bool,
    ) -> Option<Delivery> {
        let speaker_group = self.players.get(&from)?.group;
        let speaker_kind = speaker_group
            .and_then(|id| self.groups.get(&id))
            .map(|group| group.kind);
        let pose = self.poses.get(&from);
        let range_squared = range * range;
        let (mut native, mut svc, mut group_svc) = (Vec::new(), Vec::new(), Vec::new());
        for (id, player) in &self.players {
            if *id == from || !player.kind.hears() {
                continue;
            }
            if speaker_group.is_some() && player.group == speaker_group {
                if matches!(player.kind, Kind::Svc { .. }) {
                    group_svc.push(*id);
                }
                continue;
            }
            if speaker_kind.is_some_and(|kind| kind != 1) {
                continue;
            }
            if player
                .group
                .and_then(|group| self.groups.get(&group))
                .is_some_and(|group| group.kind == 2)
            {
                continue;
            }
            let (Some(pose), Some(heard)) = (pose, self.poses.get(id)) else {
                continue;
            };
            let distance_squared: f64 = (0..3)
                .map(|axis| (heard.eye[axis] - pose.eye[axis]).powi(2))
                .sum();
            if heard.dimension != pose.dimension || distance_squared > range_squared {
                continue;
            }
            match player.kind {
                Kind::Native { .. } if to_native => native.push(*id),
                Kind::Native { .. } => {}
                Kind::Svc { .. } => svc.push(*id),
            }
        }
        (!native.is_empty() || !svc.is_empty() || !group_svc.is_empty()).then_some(Delivery {
            speaker: uuid,
            eye: pose.map_or([0.0; 3], |pose| pose.eye),
            sequence,
            whispering,
            range: range as f32,
            native,
            svc,
            group_svc,
        })
    }
}

#[cfg(test)]
#[path = "voice_tests.rs"]
mod tests;
