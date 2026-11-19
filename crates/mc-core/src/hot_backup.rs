//! Hot native backup: a consistent copy of a running world.
//!
//! The native writer thread is the only redb writer and runs jobs FIFO, so while
//! it executes [`copy_world`] the database file is exactly the last committed
//! checkpoint (an Immediate commit is on disk when `commit` returns; no
//! transaction is open). Published WAL batches are immutable and only the
//! writer thread prunes them, so the contiguous batches after that checkpoint
//! are copied up to the tick the journal thread had durably appended when it
//! accepted the request. Recovery from the copy therefore reaches that tick.
//! Reads continue on every other thread; ticks and WAL appends are not paused.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::config::CoreError;
use crate::journal::parse_batch_name;

/// Dirty-page burst limit while copying: sync the output this often so the
/// concurrent WAL fsyncs never queue behind hundreds of MB of dirty pages.
const SYNC_EVERY_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) const NATIVE_DATABASE: &str = "native/state.redb";

/// Test-only damage applied to a finished copy, so a test can prove that
/// `Core::backup` refuses an unrestorable copy instead of reporting it.
/// Inside a `#[cfg(test)]` module on purpose: `ledger promote`'s negative
/// control keeps test hunks and reverses the fix, and the control must still
/// build.
#[cfg(test)]
pub(crate) mod test_hooks {
    use std::sync::atomic::{AtomicU8, Ordering};

    pub(crate) const NONE: u8 = 0;
    /// Corrupt the copied database after it is written.
    pub(crate) const DATABASE: u8 = 1;
    /// Append a CRC-clean batch whose record recovery refuses.
    pub(crate) const UNREPLAYABLE_BATCH: u8 = 2;

    static DAMAGE: AtomicU8 = AtomicU8::new(NONE);

    pub(crate) fn arm(mode: u8) {
        DAMAGE.store(mode, Ordering::SeqCst);
    }

    pub(crate) fn take() -> u8 {
        DAMAGE.swap(NONE, Ordering::SeqCst)
    }
}

/// What a completed hot backup contains. `files` maps world-relative names to
/// lowercase hex SHA-256 digests of the copied bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotBackup {
    /// Tick of the checkpoint stored in the copied database.
    pub checkpoint_tick: u64,
    /// Last tick recoverable from the copy: checkpoint plus copied WAL.
    pub wal_tick: u64,
    pub files: BTreeMap<String, String>,
}

fn io(error: std::io::Error) -> CoreError {
    CoreError::Storage(error.to_string())
}

fn copy_hashed(source: &Path, destination: &Path) -> Result<String, CoreError> {
    let mut input = File::open(source)
        .map_err(|error| CoreError::Storage(format!("{}: {error}", source.display())))?;
    copy_reader_hashed(&mut input, destination)
}

fn copy_reader_hashed(input: &mut impl Read, destination: &Path) -> Result<String, CoreError> {
    let mut output = File::options()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| CoreError::Storage(format!("{}: {error}", destination.display())))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    let mut unsynced = 0;
    loop {
        let count = input.read(&mut buffer).map_err(io)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count]).map_err(io)?;
        hasher.update(&buffer[..count]);
        unsynced += count as u64;
        if unsynced >= SYNC_EVERY_BYTES {
            output.sync_data().map_err(io)?;
            unsynced = 0;
        }
    }
    output.sync_all().map_err(io)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Copy the checkpoint database at `checkpoint_tick` and the contiguous WAL
/// batches `(checkpoint_tick, through]` into the existing empty directory `out`.
/// Must run on the native writer thread between jobs.
pub(crate) fn copy_world(
    world_dir: &Path,
    out: &Path,
    database: &mut impl Read,
    checkpoint_tick: u64,
    through: u64,
    identity: crate::journal::WalIdentity,
) -> Result<HotBackup, CoreError> {
    let store_uuid = identity.store;
    let through = through.max(checkpoint_tick);
    let mut files = BTreeMap::new();
    std::fs::create_dir(out.join("native")).map_err(io)?;
    std::fs::create_dir(out.join("journal")).map_err(io)?;
    files.insert(
        NATIVE_DATABASE.to_owned(),
        copy_reader_hashed(database, &out.join(NATIVE_DATABASE))?,
    );
    // Books R4-P1: the marker travels with the store it witnesses, so a
    // restored copy is checked against the same (store id, checkpoint tick)
    // pair. It is read here, on the writer thread, after the commit that it
    // describes; a store with a checkpoint always has one.
    let marker_path = mc_world::store_marker::path(world_dir);
    let marker_bytes = std::fs::read(&marker_path)
        .map_err(|error| CoreError::Storage(format!("{}: {error}", marker_path.display())))?;
    let marker = mc_world::store_marker::StoreMarker::decode(&marker_bytes)
        .map_err(|error| CoreError::Storage(format!("{}: {error}", marker_path.display())))?;
    if marker.store_uuid != store_uuid {
        return Err(CoreError::Storage(format!(
            "hot backup: checkpoint marker {} records store id {} but the store is {}; the copy \
             would pair two different worlds",
            marker_path.display(),
            uuid::Uuid::from_bytes(marker.store_uuid),
            uuid::Uuid::from_bytes(store_uuid),
        )));
    }
    if marker.checkpoint_tick > checkpoint_tick {
        return Err(CoreError::Storage(format!(
            "hot backup: checkpoint marker records {}, ahead of the committed checkpoint {}; \
             the store rolled back and the copy would be refused on restore",
            marker.checkpoint_tick, checkpoint_tick
        )));
    }
    files.insert(
        mc_world::store_marker::MARKER_FILE.to_owned(),
        copy_reader_hashed(
            &mut marker_bytes.as_slice(),
            &out.join(mc_world::store_marker::MARKER_FILE),
        )?,
    );

    let mut batches = Vec::new();
    for entry in std::fs::read_dir(world_dir.join("journal")).map_err(io)? {
        let name = entry
            .map_err(io)?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if let Some((lo, hi)) = parse_batch_name(&name)
            && hi > checkpoint_tick
            && lo <= through
        {
            batches.push((lo, hi, name));
        }
    }
    batches.sort();
    let mut covered = checkpoint_tick;
    for (lo, hi, name) in &batches {
        if *lo != covered + 1 {
            return Err(CoreError::Storage(format!(
                "hot backup: journal batch {name} starts at {lo}, expected {}",
                covered + 1
            )));
        }
        let relative = format!("journal/{name}");
        // Books R5 G3: a digest only proves the copy matches the source. Decode
        // and CRC-check every batch so the backup cannot be silently
        // unrestorable because the source tail was damaged.
        crate::journal::verify_batch_file(&world_dir.join(&relative), identity)?;
        let digest = copy_hashed(&world_dir.join(&relative), &out.join(&relative))?;
        files.insert(relative, digest);
        covered = *hi;
    }
    if covered < through {
        return Err(CoreError::Storage(format!(
            "hot backup: journal hole: ticks {}..={through} have no batch",
            covered + 1
        )));
    }
    // Test-only: damage the finished copy so `Core::backup` must refuse it.
    #[cfg(test)]
    match test_hooks::take() {
        test_hooks::DATABASE => {
            let path = out.join(NATIVE_DATABASE);
            let mut bytes = std::fs::read(&path).map_err(io)?;
            for byte in bytes.iter_mut().take(4096) {
                *byte ^= 0xff;
            }
            std::fs::write(&path, &bytes).map_err(io)?;
        }
        test_hooks::UNREPLAYABLE_BATCH => {
            let tick = covered + 1;
            let entry = crate::journal::JournalEntry::InhabitedTime {
                image: crate::journal::InhabitedTimeImage {
                    chunk: [0, 0],
                    value: -1,
                    tick,
                    active: false,
                },
            };
            let payload = format!(
                "{}\n",
                serde_json::to_string(&entry)
                    .map_err(|error| CoreError::Storage(error.to_string()))?
            );
            let frame = crate::journal::frame_batch(tick, tick, payload.as_bytes(), identity)?;
            std::fs::write(
                out.join("journal")
                    .join(format!("batch-{tick:020}-{tick:020}.bin")),
                frame,
            )
            .map_err(io)?;
        }
        _ => {}
    }
    for directory in [out.join("native"), out.join("journal"), out.to_path_buf()] {
        crate::journal::fsync_dir(&directory).map_err(io)?;
    }
    Ok(HotBackup {
        checkpoint_tick,
        wal_tick: covered,
        files,
    })
}

/// Scratch directory for repairing a copy of a store, removed on drop.
struct ScratchStore(Option<std::path::PathBuf>);

impl Drop for ScratchStore {
    fn drop(&mut self) {
        if let Some(dir) = self.0.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// The copy's store, kept alive while its reader is used: read-only when the
/// copy is clean, or opened writable on a repaired scratch copy when redb says
/// it would repair on the next serve (the normal state of a hot copy of a live
/// store). The backup itself is never written.
enum OpenedCopy {
    ReadOnly(mc_world::native::NativeReadOnlyStore),
    Repaired(mc_world::native::NativeStore),
}

impl OpenedCopy {
    /// The recovery reader, after checking the store against the copied marker.
    fn reader(
        &self,
        copy: &Path,
        store_path: &Path,
    ) -> Result<mc_world::native::NativeCheckpointReader, CoreError> {
        let storage = |error: mc_world::native::NativeError| {
            CoreError::Storage(format!("hot backup {}: {error}", store_path.display()))
        };
        let pair = |error: String| {
            CoreError::Storage(format!(
                "hot backup {}: the copy does not pair with its checkpoint marker: {error}",
                copy.display()
            ))
        };
        match self {
            Self::ReadOnly(store) => {
                store
                    .verify_marker(copy)
                    .map_err(|error| pair(error.to_string()))?;
                store.checkpoint_reader().map_err(storage)
            }
            Self::Repaired(store) => {
                mc_world::store_marker::verify(
                    copy,
                    store.store_uuid(),
                    store.checkpoint_tick().map_err(storage)?,
                )
                .map_err(|error| pair(error.to_string()))?;
                store.checkpoint_reader().map_err(storage)
            }
        }
    }
}

/// Open the copy for verification and hold its scratch directory, if any.
fn open_copy(
    copy: &Path,
    config: &crate::config::CoreConfig,
) -> Result<(OpenedCopy, ScratchStore), CoreError> {
    let store_path = copy.join(NATIVE_DATABASE);
    let identity = crate::native_checkpoint::native_identity(config);
    match mc_world::native::NativeReadOnlyStore::open_with_cache(
        &store_path,
        identity,
        16 * 1024 * 1024,
    ) {
        Ok(store) => Ok((OpenedCopy::ReadOnly(store), ScratchStore(None))),
        Err(mc_world::native::NativeError::Source(_)) => {
            static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "krasnokamen-verify-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let scratch = ScratchStore(Some(dir.clone()));
            std::fs::create_dir_all(&dir).map_err(|error| {
                CoreError::Storage(format!("hot backup verify {}: {error}", dir.display()))
            })?;
            let repaired = dir.join("state.redb");
            std::fs::copy(&store_path, &repaired).map_err(|error| {
                CoreError::Storage(format!("hot backup verify {}: {error}", repaired.display()))
            })?;
            let store = mc_world::native::NativeStore::open(&repaired, identity).map_err(|error| {
                CoreError::Storage(format!(
                    "hot backup {}: the copy does not open: {error}",
                    store_path.display()
                ))
            })?;
            Ok((OpenedCopy::Repaired(store), scratch))
        }
        Err(error) => Err(CoreError::Storage(format!(
            "hot backup {}: the copy does not open: {error}",
            store_path.display()
        ))),
    }
}

/// Prove the copy at `copy` actually restores, instead of reporting a backup
/// whose bytes and CRCs match the source (Books R5-G3).
///
/// Opens exactly that copy — read-only, or through a repaired scratch copy when
/// redb would repair it on the next serve, so the backup itself is never
/// written — checks the copied store against the copied checkpoint marker, and
/// runs the ordinary recovery reader and WAL replay over it: manifest and
/// domain versions, stored records, and every
/// batch of the copied tail under the palette this build registers. The
/// recovered tick must be the `expected_tick` the copy claims. A damaged store
/// or a CRC-clean but semantically unreplayable batch therefore fails here, at
/// backup time, never at disaster time.
pub fn verify_restorable(
    copy: &Path,
    config: &crate::config::CoreConfig,
    registry: std::sync::Arc<crate::registry::CoreRegistry>,
    expected_tick: u64,
) -> Result<(), CoreError> {
    let (store, scratch) = open_copy(copy, config)?;
    let store_path = copy.join(NATIVE_DATABASE);
    let reader = store.reader(copy, &store_path)?;
    let mut copy_config = config.clone();
    copy_config.world_dir = copy.to_path_buf();
    let recovered =
        crate::native_checkpoint::recover_with_reader(&copy_config, registry, reader).map_err(
            |error| {
                CoreError::Storage(format!(
                    "hot backup {}: the copy does not recover: {error}",
                    copy.display()
                ))
            },
        )?;
    let recovered_tick = recovered
        .dimensions
        .values()
        .map(|replay| replay.tick)
        .max()
        .unwrap_or(recovered.manifest.tick);
    // The recovered world still borrows the copy's store through its reader.
    drop(recovered);
    drop(store);
    drop(scratch);
    if recovered_tick != expected_tick {
        return Err(CoreError::Storage(format!(
            "hot backup {}: the copy recovers to tick {recovered_tick}, not the {expected_tick} \
             it was taken at",
            copy.display()
        )));
    }
    tracing::info!(
        copy = %copy.display(),
        tick = recovered_tick,
        "hot backup verified restorable"
    );
    Ok(())
}
