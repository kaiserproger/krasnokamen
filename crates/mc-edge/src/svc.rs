//! Simple Voice Chat (henkelmax/simple-voice-chat, compatibility version 20,
//! branch `26.1`) server wire, ADR 0019. Pure: bytes in, bytes out; no I/O.
//!
//! Two transports: Play plugin channels `voicechat:*` (handshake, player states)
//! and a separate UDP socket whose datagrams are AES-128-GCM encrypted with a
//! per-player secret. Layouts follow SVC's `FriendlyByteBuf` usage: big-endian
//! integers, `VarInt`-prefixed byte arrays/strings, UUID as two big-endian longs.
use aes_gcm::{
    Aes128Gcm, Key, Nonce,
    aead::{Aead, KeyInit},
};
use uuid::Uuid;

/// `Voicechat.COMPATIBILITY_VERSION` of the `26.1` branch (`voicechat_compatibility_version`).
pub(crate) const COMPATIBILITY_VERSION: i32 = 20;
pub(crate) const MAGIC: u8 = 0xFF;
/// `Utils.MAX_VOICE_CHAT_PACKET_SIZE`.
pub(crate) const MAX_DATAGRAM_PAYLOAD: usize = 2048;
/// `AudioUtils.MAX_OPUS_PAYLOAD_SIZE`.
pub(crate) const MAX_OPUS_PAYLOAD: usize = 1275;
/// `PingHandler.PING_V1`: pseudo player id of unauthenticated server pings.
pub(crate) const PING_V1: Uuid = Uuid::from_u128(0x58bc9ae9_c7a8_45e4_a11c_efbb67199425);
const PING_PAYLOAD: usize = 24;
const IV_LEN: usize = 12;

pub(crate) mod channel {
    pub const NAMESPACE: &str = "voicechat";
    pub const REQUEST_SECRET: &str = "voicechat:request_secret";
    pub const UPDATE_STATE: &str = "voicechat:update_state";
    pub const CREATE_GROUP: &str = "voicechat:create_group";
    pub const SET_GROUP: &str = "voicechat:set_group";
    pub const LEAVE_GROUP: &str = "voicechat:leave_group";
    pub const ADD_GROUP: &str = "voicechat:add_group";
    pub const REMOVE_GROUP: &str = "voicechat:remove_group";
    pub const JOINED_GROUP: &str = "voicechat:joined_group";
    pub const SECRET: &str = "voicechat:secret";
    pub const STATE: &str = "voicechat:state";
    pub const STATES: &str = "voicechat:states";
    pub const REMOVE_STATE: &str = "voicechat:remove_state";
    /// Serverbound channels announced to the client in `minecraft:register`.
    pub const SERVERBOUND: &[&str] = &[
        REQUEST_SECRET,
        UPDATE_STATE,
        SET_GROUP,
        CREATE_GROUP,
        LEAVE_GROUP,
    ];
}

/// Per-player AES-128 key.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Secret(pub [u8; 16]);
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}
impl Secret {
    pub(crate) fn random() -> anyhow::Result<Self> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!("secret entropy: {error}"))?;
        Ok(Self(bytes))
    }

    /// `iv(12) || ciphertext || tag(16)`, exactly `Secret.encrypt`.
    pub(crate) fn encrypt(&self, plain: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut iv = [0; IV_LEN];
        getrandom::fill(&mut iv).map_err(|error| anyhow::anyhow!("iv entropy: {error}"))?;
        self.encrypt_with_iv(&iv, plain)
    }

    pub(crate) fn encrypt_with_iv(
        &self,
        iv: &[u8; IV_LEN],
        plain: &[u8],
    ) -> anyhow::Result<Vec<u8>> {
        let cipher = Aes128Gcm::new(&Key::<Aes128Gcm>::from(self.0));
        let sealed = cipher
            .encrypt(&Nonce::from(*iv), plain)
            .map_err(|_| anyhow::anyhow!("voice packet encryption failed"))?;
        let mut out = Vec::with_capacity(IV_LEN + sealed.len());
        out.extend(iv);
        out.extend(sealed);
        Ok(out)
    }

    pub(crate) fn decrypt(&self, payload: &[u8]) -> Option<Vec<u8>> {
        let (iv, sealed) = payload.split_first_chunk::<IV_LEN>()?;
        let cipher = Aes128Gcm::new(&Key::<Aes128Gcm>::from(self.0));
        cipher.decrypt(&Nonce::from(*iv), sealed).ok()
    }
}

pub(crate) struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(count)?;
        self.0 = tail;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    pub(crate) fn i32(&mut self) -> Option<i32> {
        Some(i32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_be_bytes(self.take(8)?.try_into().ok()?))
    }
    fn bool(&mut self) -> Option<bool> {
        match self.u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
    fn uuid(&mut self) -> Option<Uuid> {
        Uuid::from_slice(self.take(16)?).ok()
    }
    fn var_int(&mut self) -> Option<u32> {
        let mut value = 0u32;
        for shift in (0..35).step_by(7) {
            let byte = self.u8()?;
            value |= u32::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }
    pub(crate) fn byte_array(&mut self, max: usize) -> Option<&'a [u8]> {
        let len = self.var_int()? as usize;
        if len > max {
            return None;
        }
        self.take(len)
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn put_var_int(out: &mut Vec<u8>, mut value: u32) {
    while value >= 0x80 {
        out.push((value & 0x7f) as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}
fn put_byte_array(out: &mut Vec<u8>, bytes: &[u8]) {
    put_var_int(out, bytes.len() as u32);
    out.extend(bytes);
}
fn put_utf(out: &mut Vec<u8>, text: &str) {
    put_byte_array(out, text.as_bytes());
}

/// A client datagram before decryption: `0xFF, uuid, VarInt len, payload`.
#[derive(Debug, PartialEq)]
pub(crate) struct Datagram<'a> {
    pub player: Uuid,
    /// `iv || ciphertext || tag`, or the raw ping body for [`PING_V1`].
    pub payload: &'a [u8],
}

pub(crate) fn parse_datagram(bytes: &[u8]) -> Option<Datagram<'_>> {
    let mut reader = Reader::new(bytes);
    if reader.u8()? != MAGIC {
        return None;
    }
    let player = reader.uuid()?;
    let max = if player == PING_V1 {
        PING_PAYLOAD
    } else {
        MAX_DATAGRAM_PAYLOAD
    };
    Some(Datagram {
        player,
        payload: reader.byte_array(max)?,
    })
}

/// Reply to a [`PING_V1`] datagram: the 24 raw bytes (uuid + timestamp), no framing.
pub(crate) fn ping_reply(payload: &[u8]) -> Option<Vec<u8>> {
    (payload.len() == PING_PAYLOAD).then(|| payload.to_vec())
}

/// Server datagram: `0xFF, VarInt len, encrypted`.
pub(crate) fn seal_datagram(secret: &Secret, message: &[u8]) -> anyhow::Result<Vec<u8>> {
    let encrypted = secret.encrypt(message)?;
    let mut out = Vec::with_capacity(6 + encrypted.len());
    out.push(MAGIC);
    put_byte_array(&mut out, &encrypted);
    Ok(out)
}

/// Decrypted client -> server UDP message.
#[derive(Debug, PartialEq)]
pub(crate) enum Udp<'a> {
    /// `data` empty = end of speech.
    Mic {
        data: &'a [u8],
        sequence: i64,
        whispering: bool,
    },
    Authenticate {
        player: Uuid,
        secret: Secret,
    },
    ConnectionCheck,
    KeepAlive,
    /// Recognised but not used by the relay (`PingPacket`, ack packets).
    Ignored,
}

mod udp_kind {
    pub const MIC: u8 = 0x1;
    pub const PLAYER_SOUND: u8 = 0x2;
    pub const AUTHENTICATE: u8 = 0x5;
    pub const AUTHENTICATE_ACK: u8 = 0x6;
    pub const PING: u8 = 0x7;
    pub const KEEP_ALIVE: u8 = 0x8;
    pub const CONNECTION_CHECK: u8 = 0x9;
    pub const CONNECTION_CHECK_ACK: u8 = 0xA;
    pub const GROUP_SOUND: u8 = 0x3;
    pub const LOCATION_SOUND: u8 = 0x4;
}

pub(crate) fn parse_udp(plain: &[u8]) -> Option<Udp<'_>> {
    let mut reader = Reader::new(plain);
    match reader.u8()? {
        udp_kind::MIC => {
            let data = reader.byte_array(MAX_OPUS_PAYLOAD)?;
            let sequence = reader.i64()?;
            let whispering = reader.bool()?;
            Some(Udp::Mic {
                data,
                sequence,
                whispering,
            })
        }
        udp_kind::AUTHENTICATE => {
            let player = reader.uuid()?;
            let secret = Secret(reader.take(16)?.try_into().ok()?);
            Some(Udp::Authenticate { player, secret })
        }
        udp_kind::CONNECTION_CHECK => Some(Udp::ConnectionCheck),
        udp_kind::KEEP_ALIVE => Some(Udp::KeepAlive),
        udp_kind::PLAYER_SOUND
        | udp_kind::GROUP_SOUND
        | udp_kind::LOCATION_SOUND
        | udp_kind::AUTHENTICATE_ACK
        | udp_kind::PING
        | udp_kind::CONNECTION_CHECK_ACK => Some(Udp::Ignored),
        _ => None,
    }
}

pub(crate) fn authenticate_ack() -> Vec<u8> {
    vec![udp_kind::AUTHENTICATE_ACK]
}
pub(crate) fn connection_check_ack() -> Vec<u8> {
    vec![udp_kind::CONNECTION_CHECK_ACK]
}
pub(crate) fn keep_alive() -> Vec<u8> {
    vec![udp_kind::KEEP_ALIVE]
}

/// `PlayerSoundPacket`: the client plays it at the position of the entity whose
/// uuid is `sender`. `data` empty = end of speech; `sequence` must be
/// non-negative (negative marks a client-side audio channel).
pub(crate) fn player_sound(
    sender: Uuid,
    data: &[u8],
    sequence: i64,
    whispering: bool,
    distance: f32,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(60 + data.len());
    out.push(udp_kind::PLAYER_SOUND);
    out.extend(sender.as_bytes()); // channel id = sender
    out.extend(sender.as_bytes());
    put_byte_array(&mut out, data);
    out.extend(sequence.to_be_bytes());
    out.extend(distance.to_be_bytes());
    out.push(u8::from(whispering));
    out
}

/// Non-positional group sound, matching SVC's GroupSoundPacket.
pub(crate) fn group_sound(sender: Uuid, data: &[u8], sequence: i64) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + data.len());
    out.push(udp_kind::GROUP_SOUND);
    out.extend(sender.as_bytes());
    out.extend(sender.as_bytes());
    put_byte_array(&mut out, data);
    out.extend(sequence.to_be_bytes());
    out.push(0);
    out
}

/// `RequestSecretPacket` body: the client's compatibility version.
pub(crate) fn parse_request_secret(payload: &[u8]) -> Option<i32> {
    let mut reader = Reader::new(payload);
    let version = reader.i32()?;
    reader.is_empty().then_some(version)
}

/// `UpdateStatePacket` body: `disabled`.
pub(crate) fn parse_update_state(payload: &[u8]) -> Option<bool> {
    let mut reader = Reader::new(payload);
    let disabled = reader.bool()?;
    reader.is_empty().then_some(disabled)
}

pub(crate) struct Group {
    pub id: Uuid,
    pub name: String,
    pub password: Option<String>,
    /// 0 normal, 1 open, 2 isolated.
    pub kind: i16,
}

fn group_string(reader: &mut Reader<'_>, max: usize) -> Option<String> {
    let text = std::str::from_utf8(reader.byte_array(max * 4)?).ok()?;
    (text.encode_utf16().count() <= max && !text.chars().any(char::is_control))
        .then(|| text.to_owned())
}

pub(crate) fn parse_create_group(payload: &[u8]) -> Option<Group> {
    let mut reader = Reader::new(payload);
    let name = group_string(&mut reader, 24)?;
    if name.is_empty() || name.starts_with(char::is_whitespace) {
        return None;
    }
    let password = if reader.bool()? {
        let password = group_string(&mut reader, 24)?;
        if password.is_empty() {
            return None;
        }
        Some(password)
    } else {
        None
    };
    let kind = i16::from_be_bytes(reader.take(2)?.try_into().ok()?);
    if !reader.is_empty() {
        return None;
    }
    Some(Group {
        id: Uuid::new_v4(),
        name,
        password,
        kind: if (0..=2).contains(&kind) { kind } else { 0 },
    })
}

pub(crate) fn parse_join_group(payload: &[u8]) -> Option<(Uuid, Option<String>)> {
    let mut reader = Reader::new(payload);
    let id = reader.uuid()?;
    let password = if reader.bool()? {
        Some(group_string(&mut reader, 512)?)
    } else {
        None
    };
    reader.is_empty().then_some((id, password))
}

pub(crate) fn add_group(group: &Group) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(group.id.as_bytes());
    put_utf(&mut out, &group.name);
    out.extend([u8::from(group.password.is_some()), 0, 0]);
    out.extend(group.kind.to_be_bytes());
    out
}

pub(crate) fn joined_group(group: Option<Uuid>, wrong_password: bool) -> Vec<u8> {
    let mut out = vec![u8::from(group.is_some())];
    if let Some(group) = group {
        out.extend(group.as_bytes());
    }
    out.push(u8::from(wrong_password));
    out
}

/// Server settings carried by `SecretPacket`.
pub(crate) struct SecretInfo<'a> {
    pub secret: &'a Secret,
    pub port: u16,
    pub player: Uuid,
    pub distance: f64,
    /// Empty: the client uses the address it reached the game server on.
    pub voice_host: &'a str,
}

/// `SecretPacket` body. Codec VOIP, MTU 1275, keep-alive 1000 ms;
/// groups enabled and recording disabled.
pub(crate) fn secret_packet(info: &SecretInfo<'_>) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend(info.secret.0);
    out.extend(i32::from(info.port).to_be_bytes());
    out.extend(info.player.as_bytes());
    out.push(0); // Codec.VOIP
    out.extend((MAX_OPUS_PAYLOAD as i32).to_be_bytes());
    out.extend(info.distance.to_be_bytes());
    out.extend(KEEP_ALIVE_MS.to_be_bytes());
    out.push(1); // groups_enabled
    put_utf(&mut out, info.voice_host);
    out.push(0); // allow_recording
    out
}
pub(crate) const KEEP_ALIVE_MS: i32 = 1000;

/// SVC player state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct State {
    pub uuid: Uuid,
    pub name: String,
    pub disabled: bool,
    pub disconnected: bool,
    pub group: Option<Uuid>,
}
impl State {
    fn write(&self, out: &mut Vec<u8>) {
        out.push(u8::from(self.disabled));
        out.push(u8::from(self.disconnected));
        out.extend(self.uuid.as_bytes());
        put_utf(out, &self.name);
        out.push(u8::from(self.group.is_some()));
        if let Some(group) = self.group {
            out.extend(group.as_bytes());
        }
    }
}
pub(crate) fn state_packet(state: &State) -> Vec<u8> {
    let mut out = Vec::new();
    state.write(&mut out);
    out
}
pub(crate) fn states_packet(states: &[State]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend((states.len() as i32).to_be_bytes());
    for state in states {
        state.write(&mut out);
    }
    out
}
pub(crate) fn remove_state_packet(uuid: Uuid) -> Vec<u8> {
    uuid.as_bytes().to_vec()
}

/// `minecraft:register` body: NUL-separated channel ids.
pub(crate) fn register_body() -> Vec<u8> {
    channel::SERVERBOUND.join("\0").into_bytes()
}

/// Client datagram as the SVC mod writes it (`writeClient`).
#[cfg(test)]
pub(crate) fn client_datagram(player: Uuid, secret: &Secret, message: &[u8]) -> Vec<u8> {
    let mut out = vec![MAGIC];
    out.extend(player.as_bytes());
    put_byte_array(&mut out, &secret.encrypt(message).unwrap());
    out
}

#[cfg(test)]
#[path = "svc_tests.rs"]
mod tests;
