//! Per-session adjacent protocol chain (ADR 0015). The connection owns the
//! only framing/encoding boundary; core gameplay stays canonical and unaware.
mod adjacent;
pub mod snapshots;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use bytes::BytesMut;
use mc_data::protocol_versions::{SUPPORTED, Version, cache_root, step_root};
use mc_protocol::{Compression, RawFrame};

/// Direction of the client relative to the server for a particular step.
/// Up is deliberately distinct: lossy down mappings must never be inverted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Down,
    Up,
}

trait StepSession: Send {
    fn clientbound(&mut self, direction: Direction, frame: RawFrame) -> Result<Option<RawFrame>>;
    fn serverbound(&mut self, direction: Direction, frame: RawFrame) -> Result<Option<RawFrame>>;
    fn split(&self) -> Box<dyn StepSession>;
}

trait StepFactory: Send + Sync {
    fn session(&self) -> Box<dyn StepSession>;
}

impl StepFactory for Arc<adjacent::Profile> {
    fn session(&self) -> Box<dyn StepSession> {
        Box::new(adjacent::Session::new(Arc::clone(self)))
    }
}

struct Step {
    upper: i32,
    lower: i32,
    factory: Box<dyn StepFactory>,
}

/// Clientbound traversal order, canonical -> peer. Serverbound reverses it.
fn route(steps: &[Step], canonical: i32, peer: i32) -> Result<Vec<(usize, Direction)>> {
    let mut current = canonical;
    let mut result = Vec::new();
    while current != peer {
        ensure!(
            result.len() < steps.len(),
            "protocol chain cycle reaching {peer}"
        );
        let direction = if peer < current {
            Direction::Down
        } else {
            Direction::Up
        };
        let (index, step) = steps
            .iter()
            .enumerate()
            .find(|(_, step)| match direction {
                Direction::Down => step.upper == current && step.lower >= peer,
                Direction::Up => step.lower == current && step.upper <= peer,
            })
            .with_context(|| {
                format!("missing adjacent protocol step from {current} toward {peer}")
            })?;
        current = match direction {
            Direction::Down => step.lower,
            Direction::Up => step.upper,
        };
        result.push((index, direction));
    }
    Ok(result)
}

/// One release owns its wire registry/tag snapshot and known-pack identity.
/// Steps own separate mapping snapshots, so an absent endpoint does not break
/// another release whose route merely passes through that protocol.
pub struct Profile {
    pub version: Version,
    pub data: mc_data::VanillaData,
    pub tags: mc_data::tags::TagsData,
    route: Vec<(Arc<Step>, Direction)>,
    #[cfg(test)]
    tables: adjacent::tables::Tables,
}

impl Profile {
    fn load(root: &Path, version: Version, route: Vec<(Arc<Step>, Direction)>) -> Result<Self> {
        snapshots::validate_version(root, version)?;
        let data = mc_data::load(root).with_context(|| {
            format!(
                "loading {} registries from {}",
                version.release,
                root.display()
            )
        })?;
        ensure!(
            data.has_full_registry_payloads(),
            "{} lacks complete RegistryData payloads",
            root.display()
        );
        let tags = mc_data::tags::load(root, &data)?;
        crate::startup_data::validate_payload_tag_references(root, &data, &tags)?;
        #[cfg(test)]
        let tables = {
            let reports = adjacent::tables::Reports::load(root)?;
            let mut tables = adjacent::tables::Tables::new(&reports, &reports, &data, &data)?;
            tables.formats = adjacent::tables::Formats {
                upper: version.protocol,
                lower: version.protocol,
            };
            tables
        };
        Ok(Self {
            version,
            data,
            tags,
            route,
            #[cfg(test)]
            tables,
        })
    }
}

pub(crate) fn validate_endpoint(root: &Path, release: &str) -> Result<()> {
    let version = SUPPORTED
        .iter()
        .find(|version| version.release == release)
        .context("unsupported endpoint release")?;
    Profile::load(root, *version, Vec::new())?;
    Ok(())
}

pub(crate) fn step_snapshot_valid(root: &Path, upper: i32, lower: i32) -> bool {
    adjacent::Profile::load(root, upper, lower).is_ok()
}

#[derive(Default)]
pub struct Catalog {
    profiles: Vec<Arc<Profile>>,
}

impl Catalog {
    /// Missing endpoint data disables that release only; a missing step disables
    /// every route crossing it. Canonical sessions never require a translator.
    #[must_use]
    pub fn discover(canonical: &mc_data::VanillaData) -> Self {
        let Some(root) = canonical.sidecar_root() else {
            return Self::default();
        };
        let mut steps = Vec::new();
        for pair in SUPPORTED.windows(2) {
            let (lower, upper) = (pair[0], pair[1]);
            let path = step_root(root, mc_protocol::TARGET_RELEASE, upper, lower);
            match adjacent::Profile::load(&path, upper.protocol, lower.protocol) {
                Ok(profile) => steps.push(Step {
                    upper: upper.protocol,
                    lower: lower.protocol,
                    factory: Box::new(Arc::new(profile)),
                }),
                Err(error) => {
                    tracing::warn!(step = %format!("{}-{}", upper.protocol, lower.protocol), cache = %path.display(), error = %format!("{error:#}"), "protocol step unavailable; all versions whose route crosses this step are disabled")
                }
            }
        }
        let routes: Vec<_> = SUPPORTED
            .iter()
            .filter(|version| version.protocol != mc_protocol::PROTOCOL_VERSION)
            .map(|version| {
                (
                    *version,
                    route(&steps, mc_protocol::PROTOCOL_VERSION, version.protocol),
                )
            })
            .collect();
        let steps: Vec<_> = steps.into_iter().map(Arc::new).collect();
        let mut profiles = Vec::new();
        for (version, route) in routes {
            let path = cache_root(root, mc_protocol::TARGET_RELEASE, version);
            let loaded = route.and_then(|route| {
                Profile::load(
                    &path,
                    version,
                    route
                        .into_iter()
                        .map(|(index, direction)| (Arc::clone(&steps[index]), direction))
                        .collect(),
                )
            });
            match loaded {
                Ok(profile) => {
                    tracing::info!(
                        release = version.release,
                        protocol = version.protocol,
                        "client version enabled"
                    );
                    profiles.push(Arc::new(profile));
                }
                Err(error) => {
                    tracing::warn!(release = version.release, protocol = version.protocol, cache = %path.display(), error = %format!("{error:#}"), "client version disabled; other available versions remain enabled")
                }
            }
        }
        Self { profiles }
    }

    #[must_use]
    pub fn get(&self, protocol: i32) -> Option<&Arc<Profile>> {
        self.profiles
            .iter()
            .find(|profile| profile.version.protocol == protocol)
    }
}

pub struct Session {
    profile: Arc<Profile>,
    steps: Vec<(Box<dyn StepSession>, Direction)>,
}

impl Session {
    #[must_use]
    pub fn new(profile: Arc<Profile>) -> Self {
        let steps = profile
            .route
            .iter()
            .map(|(step, direction)| (step.factory.session(), *direction))
            .collect();
        Self { profile, steps }
    }

    #[must_use]
    pub fn profile(&self) -> &Arc<Profile> {
        &self.profile
    }

    #[must_use]
    pub fn split(&self) -> Self {
        Self {
            profile: Arc::clone(&self.profile),
            steps: self
                .steps
                .iter()
                .map(|(step, direction)| (step.split(), *direction))
                .collect(),
        }
    }

    pub fn clientbound_frame(&mut self, mut frame: RawFrame) -> Result<Option<RawFrame>> {
        ensure!(!self.steps.is_empty(), "translated session has no route");
        for (step, direction) in &mut self.steps {
            let Some(next) = step.clientbound(*direction, frame)? else {
                return Ok(None);
            };
            frame = next;
        }
        Ok(Some(frame))
    }

    #[cfg(test)]
    pub fn clientbound(&mut self, id: i32, body: &[u8]) -> Result<Option<(i32, Vec<u8>)>> {
        self.clientbound_frame(RawFrame {
            id,
            body: bytes::Bytes::copy_from_slice(body),
        })
        .map(|frame| frame.map(|frame| (frame.id, frame.body.to_vec())))
    }

    pub fn serverbound(&mut self, mut frame: RawFrame) -> Result<Option<RawFrame>> {
        for (step, direction) in self.steps.iter_mut().rev() {
            let Some(next) = step.serverbound(*direction, frame)? else {
                return Ok(None);
            };
            frame = next;
        }
        Ok(Some(frame))
    }

    /// Compression/framing is decoded and re-encoded once for the entire chain,
    /// never once per step (including pre-framed core fanout batches).
    pub fn clientbound_frames(
        &mut self,
        framed: &[u8],
        compression: Compression,
    ) -> Result<BytesMut> {
        let mut input = BytesMut::from(framed);
        let mut out = BytesMut::with_capacity(framed.len());
        while let Some(frame) = mc_protocol::frame::try_decode_frame(&mut input, compression)? {
            if let Some(frame) = self.clientbound_frame(frame)? {
                mc_protocol::frame::append_frame(&mut out, frame.id, &frame.body, compression)?;
            }
        }
        ensure!(input.is_empty(), "partial frame in a pre-framed batch");
        Ok(out)
    }
}

#[cfg(test)]
mod join_tests;
#[cfg(test)]
mod tests;
