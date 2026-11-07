//! Adjacent release codecs. Each instance pairs exactly two report snapshots;
//! session state (including wire phase) belongs to that step, not the chain.

mod chunk;
mod displays;
pub(super) mod items;
mod play;
pub(super) mod tables;

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use bytes::Bytes;
#[cfg(test)]
use bytes::BytesMut;
#[cfg(test)]
use mc_protocol::Compression;
use mc_protocol::RawFrame;
use parking_lot::Mutex;

pub use tables::Metadata;
use tables::{PacketNames, Tables};

/// Immutable tables of one adjacent step. Endpoint registry/tag/known-pack
/// snapshots are owned by the version profile in the parent module.
pub struct Profile {
    tables: Tables,
}

impl Profile {
    pub fn load(root: &Path, upper: i32, lower: i32) -> Result<Self> {
        anyhow::ensure!(
            matches!((upper, lower), (777, 776) | (776, 775)),
            "no measured adjacent codec for {upper}-{lower}"
        );
        let (upper_reports, upper_data) = super::snapshots::load_side(&root.join("upper"), upper)?;
        let (lower_reports, lower_data) = super::snapshots::load_side(&root.join("lower"), lower)?;
        let mut tables = Tables::new(&upper_reports, &lower_reports, &upper_data, &lower_data)?;
        tables.formats = tables::Formats { upper, lower };
        tables.configure_metadata(&upper_reports)?;
        Ok(Self { tables })
    }
}

/// Protocol phase of a legacy session (Status never needs translation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Login,
    Configuration,
    Play,
}

/// The last absolute teleport sent and the client's last reported pose: a
/// 26.1.2 accept-teleport carries only the id, 26.3's echoes the pose.
#[derive(Debug, Default)]
struct Teleports {
    sent: HashMap<i32, Teleport>,
    pose: Option<([f64; 3], [f32; 2])>,
}

#[derive(Debug, Clone, Copy)]
struct Teleport {
    position: [f64; 3],
    rotation: [f32; 2],
    relative: i32,
}

/// One connection direction's translator. Pre-play one instance does both
/// directions; Play splits it into reader and writer halves.
pub struct Session {
    profile: Arc<Profile>,
    phase: Phase,
    teleports: Arc<Mutex<Teleports>>,
    /// Last lossy player-inventory images sent on the wire, keyed by menu slot.
    inventory_polyfills: Arc<Mutex<HashMap<i16, Vec<u8>>>>,
    /// Entities whose metadata layout differs (canonical type policy).
    entities: HashMap<i32, Metadata>,
}

impl Session {
    #[must_use]
    pub fn new(profile: Arc<Profile>) -> Self {
        Self {
            profile,
            phase: Phase::Login,
            teleports: Arc::default(),
            inventory_polyfills: Arc::default(),
            entities: HashMap::new(),
        }
    }

    pub fn split_box(&self) -> Box<dyn super::StepSession> {
        Box::new(self.split())
    }
    /// A second translator for the other Play half, sharing teleport state.
    #[must_use]
    pub fn split(&self) -> Self {
        Self {
            profile: Arc::clone(&self.profile),
            phase: self.phase,
            teleports: Arc::clone(&self.teleports),
            inventory_polyfills: Arc::clone(&self.inventory_polyfills),
            entities: HashMap::new(),
        }
    }

    fn names(profile: &Profile, phase: Phase, clientbound: bool) -> &PacketNames {
        let packets = &profile.tables.packets;
        match (phase, clientbound) {
            (Phase::Login, true) => &packets.login_cb,
            (Phase::Login, false) => &packets.login_sb,
            (Phase::Configuration, true) => &packets.config_cb,
            (Phase::Configuration, false) => &packets.config_sb,
            (Phase::Play, true) => &packets.play_cb,
            (Phase::Play, false) => &packets.play_sb,
        }
    }

    /// Upper-endpoint clientbound frame -> lower-endpoint frame. Unchanged
    /// bodies retain their Bytes storage while ids move across the chain.
    fn clientbound_frame(&mut self, mut frame: RawFrame) -> Result<Option<RawFrame>> {
        let profile = Arc::clone(&self.profile);
        let names = Self::names(&profile, self.phase, true);
        let id = frame.id;
        let name = names.canonical_name(id).with_context(|| {
            format!(
                "no canonical clientbound packet {id:#04x} in {:?}",
                self.phase
            )
        })?;
        let Some(legacy_id) = names.legacy_id(play::legacy_name(name)) else {
            return Ok(None);
        };
        match self.phase {
            Phase::Login
                if name == "minecraft:login_finished" && profile.tables.formats.lower < 776 =>
            {
                let len = frame
                    .body
                    .len()
                    .checked_sub(16)
                    .context("short login_finished")?;
                frame.body.truncate(len);
            }
            Phase::Login | Phase::Configuration => {}
            Phase::Play => match play::clientbound_body(self, name, &frame.body)
                .with_context(|| format!("translating upper-endpoint {name}"))?
            {
                None => return Ok(None),
                Some(Cow::Borrowed(_)) => {}
                Some(Cow::Owned(body)) => frame.body = Bytes::from(body),
            },
        }
        if self.phase == Phase::Play && name == "minecraft:start_configuration" {
            self.phase = Phase::Configuration;
        }
        frame.id = legacy_id;
        Ok(Some(frame))
    }

    #[cfg(test)]
    pub fn clientbound(&mut self, id: i32, body: &[u8]) -> Result<Option<(i32, Vec<u8>)>> {
        self.clientbound_frame(RawFrame {
            id,
            body: Bytes::copy_from_slice(body),
        })
        .map(|frame| frame.map(|frame| (frame.id, frame.body.to_vec())))
    }

    /// Legacy serverbound frame -> canonical frame; `None` drops packets
    /// without a corresponding canonical operation.
    pub fn serverbound(&mut self, mut frame: RawFrame) -> Result<Option<RawFrame>> {
        let profile = Arc::clone(&self.profile);
        let names = Self::names(&profile, self.phase, false);
        let name = names.legacy_name(frame.id).with_context(|| {
            format!(
                "unknown serverbound packet {:#04x} for protocol {} in {:?}",
                frame.id, profile.tables.formats.lower, self.phase
            )
        })?;
        let canonical_name = match name {
            "minecraft:swing" if profile.tables.formats.upper >= 777 => "minecraft:punch",
            "minecraft:spectate_entity" => "minecraft:spectator_action",
            other => other,
        };
        let canonical = names.canonical_id(canonical_name);
        let next = match (self.phase, name) {
            (Phase::Login, "minecraft:login_acknowledged") => Some(Phase::Configuration),
            (Phase::Configuration, "minecraft:finish_configuration") => Some(Phase::Play),
            (Phase::Play, "minecraft:configuration_acknowledged") => Some(Phase::Configuration),
            _ => None,
        };
        let Some(canonical) = canonical else {
            return Ok(None);
        };
        if self.phase == Phase::Play {
            match play::serverbound_body(self, name, &frame.body).with_context(|| {
                format!(
                    "translating protocol {} {name}",
                    profile.tables.formats.lower
                )
            })? {
                None => return Ok(None),
                Some(Cow::Borrowed(_)) => {}
                Some(Cow::Owned(body)) => frame.body = Bytes::from(body),
            }
        }
        if let Some(next) = next {
            self.phase = next;
        }
        frame.id = canonical;
        Ok(Some(frame))
    }

    /// Translate whole pre-framed Play frames (`compression` as on the wire).
    #[cfg(test)]
    pub fn clientbound_frames(
        &mut self,
        framed: &[u8],
        compression: Compression,
    ) -> Result<BytesMut> {
        let mut input = BytesMut::from(framed);
        let mut out = BytesMut::with_capacity(framed.len());
        while let Some(frame) = mc_protocol::frame::try_decode_frame(&mut input, compression)? {
            if let Some((id, body)) = self.clientbound(frame.id, &frame.body)? {
                mc_protocol::frame::append_frame(&mut out, id, &body, compression)?;
            }
        }
        anyhow::ensure!(input.is_empty(), "partial frame in a pre-framed batch");
        Ok(out)
    }
}

impl super::StepSession for Session {
    fn clientbound(
        &mut self,
        direction: super::Direction,
        frame: RawFrame,
    ) -> Result<Option<RawFrame>> {
        anyhow::ensure!(
            direction == super::Direction::Down,
            "this measured step has no clientbound upgrade codec"
        );
        self.clientbound_frame(frame)
    }

    fn serverbound(
        &mut self,
        direction: super::Direction,
        frame: RawFrame,
    ) -> Result<Option<RawFrame>> {
        anyhow::ensure!(
            direction == super::Direction::Down,
            "this measured step has no serverbound downgrade codec"
        );
        self.serverbound(frame)
    }

    fn split(&self) -> Box<dyn super::StepSession> {
        self.split_box()
    }
}

#[cfg(test)]
pub(super) fn test_profile(tables: Tables) -> Arc<Profile> {
    Arc::new(Profile { tables })
}

#[cfg(test)]
fn test_session(tables: Tables) -> Session {
    Session::new(Arc::new(Profile { tables }))
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
