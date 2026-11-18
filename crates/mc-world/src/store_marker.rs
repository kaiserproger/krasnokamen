//! Independent durable checkpoint marker.
//!
//! redb's unattended repair can roll the store back to an earlier commit while
//! the WAL covering the lost interval is already pruned (Books R4-P1). The
//! damaged commit slot is gone, so the store itself cannot record that the
//! newer checkpoint ever existed. This module keeps a small fixed-size file
//! next to the store with the world's immutable store id and the tick of the
//! last checkpoint whose WAL was acknowledged and pruned.
//!
//! Writer order (see `mc-core`): redb Immediate commit, then this file
//! (temporary file, `sync_all`, rename over the marker, fsync the directory),
//! then the checkpoint ACK and only then the WAL prune. A store behind the
//! marker therefore means the store lost a committed checkpoint, and opening it
//! is refused instead of serving the older world.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// World-relative name of the marker file.
pub const MARKER_FILE: &str = "checkpoint.marker";

/// `KCMARK` + format version 1.
const MAGIC: &[u8; 8] = b"KCMARK\x01\x00";
const UUID_AT: usize = 8;
const TICK_AT: usize = UUID_AT + 16;
const CHECKSUM_AT: usize = TICK_AT + 8;
const LENGTH: usize = CHECKSUM_AT + 32;

/// One durable checkpoint of one store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreMarker {
    /// Immutable store id written into the native header at creation.
    pub store_uuid: [u8; 16],
    /// Tick of the committed checkpoint this marker witnesses.
    pub checkpoint_tick: u64,
}

/// Why a marker is absent, unreadable or disagrees with the store.
#[derive(Debug, thiserror::Error)]
pub enum MarkerError {
    #[error("checkpoint marker {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("checkpoint marker {} is corrupt: {reason}", path.display())]
    Corrupt { path: PathBuf, reason: &'static str },
    #[error(
        "checkpoint marker {} is missing but this store has checkpoint {}; a store written by \
         this build always has one (a world from an older build or a foreign copy is refused, \
         there is no converter)",
        path.display(),
        store_tick
    )]
    Missing { path: PathBuf, store_tick: u64 },
    #[error(
        "checkpoint marker {} records store id {} but the native store id is {}; the marker and \
         the store belong to different worlds",
        path.display(),
        uuid::Uuid::from_bytes(*marker_store),
        uuid::Uuid::from_bytes(*store)
    )]
    Foreign {
        path: PathBuf,
        marker_store: [u8; 16],
        store: [u8; 16],
    },
    #[error(
        "native store checkpoint {} is behind checkpoint marker {} at {}; the store rolled back \
         to an earlier commit than the marker's acknowledged checkpoint (redb repair), and its \
         WAL for the newer checkpoint is already pruned",
        store_tick,
        marker_tick,
        path.display()
    )]
    Behind {
        path: PathBuf,
        store_tick: u64,
        marker_tick: u64,
    },
}

impl StoreMarker {
    #[must_use]
    pub fn new(store_uuid: [u8; 16], checkpoint_tick: u64) -> Self {
        Self {
            store_uuid,
            checkpoint_tick,
        }
    }

    /// Fixed-size record: magic, store id, tick, SHA-256 of the preceding bytes.
    #[must_use]
    pub fn encode(&self) -> [u8; LENGTH] {
        let mut bytes = [0; LENGTH];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[UUID_AT..TICK_AT].copy_from_slice(&self.store_uuid);
        bytes[TICK_AT..CHECKSUM_AT].copy_from_slice(&self.checkpoint_tick.to_be_bytes());
        let checksum = Sha256::digest(&bytes[..CHECKSUM_AT]);
        bytes[CHECKSUM_AT..].copy_from_slice(&checksum);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() != LENGTH {
            return Err(DecodeError("wrong length"));
        }
        if &bytes[..8] != MAGIC {
            return Err(DecodeError("wrong magic or format version"));
        }
        let expected = Sha256::digest(&bytes[..CHECKSUM_AT]);
        if bytes[CHECKSUM_AT..] != expected[..] {
            return Err(DecodeError("checksum mismatch"));
        }
        let mut store_uuid = [0; 16];
        store_uuid.copy_from_slice(&bytes[UUID_AT..TICK_AT]);
        let mut tick = [0; 8];
        tick.copy_from_slice(&bytes[TICK_AT..CHECKSUM_AT]);
        Ok(Self {
            store_uuid,
            checkpoint_tick: u64::from_be_bytes(tick),
        })
    }
}

#[derive(Debug)]
pub struct DecodeError(&'static str);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for DecodeError {}

pub fn path(world_dir: &Path) -> PathBuf {
    world_dir.join(MARKER_FILE)
}

fn io_error(path: &Path, source: io::Error) -> MarkerError {
    MarkerError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Read the marker; `None` means the file does not exist.
pub fn read(world_dir: &Path) -> Result<Option<StoreMarker>, MarkerError> {
    let target = path(world_dir);
    let bytes = match fs::read(&target) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(&target, error)),
    };
    StoreMarker::decode(&bytes)
        .map(Some)
        .map_err(|error| MarkerError::Corrupt {
            path: target,
            reason: error.0,
        })
}

/// Durably install `marker`: write a temporary sibling, `sync_all`, rename it
/// over the marker and fsync the world directory.
pub fn write(
    world_dir: &Path,
    store_uuid: [u8; 16],
    checkpoint_tick: u64,
) -> Result<(), MarkerError> {
    let target = path(world_dir);
    let temporary = world_dir.join(format!("{MARKER_FILE}.tmp"));
    let bytes = StoreMarker::new(store_uuid, checkpoint_tick).encode();
    let mut file = File::create(&temporary).map_err(|error| io_error(&temporary, error))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(&temporary, error))?;
    drop(file);
    crate::atomic_file::replace_file_durable(&temporary, &target)
        .map_err(|error| io_error(&target, error))
}

/// Open-time check of the marker against the store, before any WAL replay.
///
/// Refuses a missing or corrupt marker for a store that has a checkpoint, a
/// different store id, and a store behind the marker. A store ahead of the
/// marker is the crash window between the commit and the marker write: it is
/// accepted and the marker is rewritten. Read-only callers use [`verify`]
/// instead, which never writes.
pub fn check(
    world_dir: &Path,
    store_uuid: [u8; 16],
    store_tick: Option<u64>,
) -> Result<(), MarkerError> {
    verify(world_dir, store_uuid, store_tick)?;
    let Some(marker) = read(world_dir)? else {
        // No checkpoint and no marker: a store created before its first one.
        return Ok(());
    };
    let store_tick = store_tick.unwrap_or(0);
    if store_tick > marker.checkpoint_tick {
        write(world_dir, store_uuid, store_tick)?;
    }
    Ok(())
}

/// The same refusal rules as [`check`], without writing: a store ahead of the
/// marker is accepted and the marker is left byte-identical. Read-only tools
/// (`world census`, `world probe`, backups) must use this one.
pub fn verify(
    world_dir: &Path,
    store_uuid: [u8; 16],
    store_tick: Option<u64>,
) -> Result<(), MarkerError> {
    let target = path(world_dir);
    let Some(marker) = read(world_dir)? else {
        return match store_tick {
            Some(store_tick) => Err(MarkerError::Missing {
                path: target,
                store_tick,
            }),
            None => Ok(()),
        };
    };
    if marker.store_uuid != store_uuid {
        return Err(MarkerError::Foreign {
            path: target,
            marker_store: marker.store_uuid,
            store: store_uuid,
        });
    }
    let store_tick = store_tick.unwrap_or(0);
    if store_tick < marker.checkpoint_tick {
        return Err(MarkerError::Behind {
            path: target,
            store_tick,
            marker_tick: marker.checkpoint_tick,
        });
    }
    Ok(())
}
