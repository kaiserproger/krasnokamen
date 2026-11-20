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

/// Scratch copy of a store that redb would repair on open, removed on drop.
///
/// It is created on the destination volume beside the archive, never inside it,
/// so a copy's own inventory never contains it and a large world does not need
/// the system temporary directory to hold a second full database. Peak space is
/// therefore the archive plus one database; a process crash can leave the
/// directory behind, and the drop reports a failed removal instead of assuming
/// it succeeded.
struct ScratchStore {
    dir: Option<std::path::PathBuf>,
}

impl ScratchStore {
    fn create(root: &Path) -> Result<Self, CoreError> {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = root.join(format!(
            ".verify-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).map_err(|error| {
            CoreError::Storage(format!(
                "hot backup verification scratch {}: {error}",
                dir.display()
            ))
        })?;
        Ok(Self { dir: Some(dir) })
    }

    fn path(&self) -> &Path {
        self.dir.as_deref().expect("scratch lives until dropped")
    }
}

impl Drop for ScratchStore {
    fn drop(&mut self) {
        let Some(dir) = self.dir.take() else {
            return;
        };
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(
                scratch = %dir.display(),
                %error,
                "hot backup verification could not remove its scratch copy"
            ),
        }
    }
}

/// The copy's store, kept alive while its reader is used: read-only when the
/// copy is clean, or opened writable on a repaired scratch copy when redb says
/// it would repair on the next serve (the normal state of a hot copy of a live
/// store). The archive itself is never written.
enum OpenedStore {
    ReadOnly(mc_world::native::NativeReadOnlyStore),
    Repaired(mc_world::native::NativeStore),
}

/// One verification instance over a copy: the store as the next serve opens it,
/// plus the identity, store id and checkpoint its own header carries. Every
/// fact used to verify the copy comes from this instance, so the archive is
/// opened once, read-only whenever redb allows it.
struct Verification {
    store: OpenedStore,
    identity: [u8; 32],
    store_uuid: [u8; 16],
    checkpoint_tick: Option<u64>,
    /// Held only so the scratch copy outlives the store and is removed after it.
    _scratch: ScratchStore,
}

impl Verification {
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
        mc_world::store_marker::verify(copy, self.store_uuid, self.checkpoint_tick)
            .map_err(|error| pair(error.to_string()))?;
        match &self.store {
            OpenedStore::ReadOnly(store) => store.checkpoint_reader().map_err(storage),
            OpenedStore::Repaired(store) => store.checkpoint_reader().map_err(storage),
        }
    }
}

/// Where a scratch copy goes: the copy's own volume, beside the archive.
fn scratch_root(copy: &Path) -> std::path::PathBuf {
    copy.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(std::env::temp_dir, Path::to_path_buf)
}

/// Open the copy for verification — read-only, or through a repaired scratch
/// copy — and report what its own header says.
fn open_verification(copy: &Path, scratch_root: &Path) -> Result<Verification, CoreError> {
    const CACHE: usize = 16 * 1024 * 1024;
    let store_path = copy.join(NATIVE_DATABASE);
    match mc_world::native::NativeReadOnlyStore::open_learning_identity(&store_path, CACHE) {
        Ok((store, identity)) => {
            let (store_uuid, checkpoint_tick) = store.marker_fields().map_err(|error| {
                CoreError::Storage(format!("hot backup {}: {error}", store_path.display()))
            })?;
            Ok(Verification {
                store: OpenedStore::ReadOnly(store),
                identity,
                store_uuid,
                checkpoint_tick,
                _scratch: ScratchStore { dir: None },
            })
        }
        Err(mc_world::native::NativeError::Source(_)) => {
            let scratch = ScratchStore::create(scratch_root)?;
            let repaired = scratch.path().join("state.redb");
            std::fs::copy(&store_path, &repaired).map_err(|error| {
                CoreError::Storage(format!(
                    "hot backup verification scratch {}: {error}",
                    repaired.display()
                ))
            })?;
            let (store, identity) = mc_world::native::NativeStore::open_learning_identity(
                &repaired, CACHE,
            )
            .map_err(|error| {
                CoreError::Storage(format!(
                    "hot backup {}: the copy does not open: {error}",
                    store_path.display()
                ))
            })?;
            let store_uuid = store.store_uuid();
            let checkpoint_tick = store.checkpoint_tick().map_err(|error| {
                CoreError::Storage(format!("hot backup {}: {error}", store_path.display()))
            })?;
            Ok(Verification {
                store: OpenedStore::Repaired(store),
                identity,
                store_uuid,
                checkpoint_tick,
                _scratch: scratch,
            })
        }
        Err(error) => Err(CoreError::Storage(format!(
            "hot backup {}: the copy does not open: {error}",
            store_path.display()
        ))),
    }
}

/// Highest published batch tick above `checkpoint`, from names alone: the copy
/// owes exactly the batches it published, and continuity is the replay's
/// business (`read_complete_batches_after` refuses a gap). Directory order is
/// never a tick order, so a maximum is taken instead of trusting the walk.
pub fn published_tick_through(copy: &Path, checkpoint: u64) -> Result<u64, CoreError> {
    let journal = copy.join("journal");
    let mut through = checkpoint;
    let entries = match std::fs::read_dir(&journal) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(through),
        Err(error) => {
            return Err(CoreError::Storage(format!(
                "hot backup {}: reading the journal: {error}",
                journal.display()
            )));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            CoreError::Storage(format!(
                "hot backup {}: reading the journal: {error}",
                journal.display()
            ))
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = name
            .strip_prefix("batch-")
            .and_then(|name| name.strip_suffix(".bin"))
        else {
            continue;
        };
        let Some((lo, hi)) = rest.split_once('-') else {
            continue;
        };
        if let (Ok(lo), Ok(hi)) = (lo.parse::<u64>(), hi.parse::<u64>())
            && hi > checkpoint
            && lo <= hi
        {
            through = through.max(hi);
        }
    }
    Ok(through)
}

/// Run the ordinary recovery reader and WAL replay over an opened copy and
/// require the tick it claims.
fn verify_opened(
    copy: &Path,
    config: &crate::config::CoreConfig,
    registry: std::sync::Arc<crate::registry::CoreRegistry>,
    expected_tick: u64,
    opened: Verification,
) -> Result<u64, CoreError> {
    let store_path = copy.join(NATIVE_DATABASE);
    let reader = opened.reader(copy, &store_path)?;
    let mut copy_config = config.clone();
    copy_config.world_dir = copy.to_path_buf();
    let startup_identity_registry = std::sync::Arc::clone(&registry);
    let recovered = crate::native_checkpoint::recover_with_reader(&copy_config, registry, reader)
        .map_err(|error| {
        CoreError::Storage(format!(
            "hot backup {}: the copy does not recover: {error}",
            copy.display()
        ))
    })?;
    let recovered_tick = recovered
        .dimensions
        .values()
        .map(|replay| replay.tick)
        .max()
        .unwrap_or(recovered.manifest.tick);
    // The plugin startup identity a serve enforces (`plugin::validate_startup`):
    // a copy whose record names other plugins is refused here, so "verified"
    // never means "verified, except the plugins".
    let startup_identity = crate::plugin_startup::world_identity(&startup_identity_registry)
        .map_err(|error| {
            CoreError::Storage(format!(
                "hot backup {}: plugin startup identity: {error}",
                copy.display()
            ))
        })?;
    let carries = crate::plugin::check_startup_identity(
        recovered.checkpoint.globals.get("plugin_startup"),
        &startup_identity,
        recovered_tick,
    )
    .map_err(|error| {
        CoreError::Storage(format!(
            "hot backup {}: the copy is refused by the plugin startup policy a serve applies: {error}",
            copy.display()
        ))
    })?;
    if !carries {
        return Err(CoreError::Storage(format!(
            "hot backup {}: native world is missing its required plugin startup identity; a serve \
             would refuse it",
            copy.display()
        )));
    }
    // The recovered world still borrows the copy's store through its reader.
    drop(recovered);
    // Close the store before the scratch directory goes away, on every path.
    drop(opened);
    if recovered_tick != expected_tick {
        return Err(CoreError::Storage(format!(
            "hot backup {}: the copy recovers to tick {recovered_tick}, not the {expected_tick} \
             it claims",
            copy.display()
        )));
    }
    Ok(recovered_tick)
}

/// Prove the copy at `copy` actually restores, instead of reporting a backup
/// whose bytes and CRCs match the source (Books R5-G3).
///
/// Opens exactly that copy — read-only, or through a repaired scratch copy on
/// the destination volume when redb would repair it on the next serve, so the
/// archive itself is never written — checks the copied store against the copied
/// checkpoint marker and against `config`'s identity, and runs the ordinary
/// recovery reader and WAL replay over it to the tick the copy claims.
pub fn verify_restorable(
    copy: &Path,
    config: &crate::config::CoreConfig,
    registry: std::sync::Arc<crate::registry::CoreRegistry>,
    expected_tick: u64,
) -> Result<(), CoreError> {
    let opened = open_verification(copy, &scratch_root(copy))?;
    let expected_identity = crate::native_checkpoint::native_identity(config);
    if opened.identity != expected_identity {
        return Err(CoreError::Storage(format!(
            "hot backup {}: the copy is another world: its header identity is {} and this world's \
             is {}",
            copy.display(),
            crate::journal::identity_hex(&opened.identity),
            crate::journal::identity_hex(&expected_identity),
        )));
    }
    let tick = verify_opened(copy, config, registry, expected_tick, opened)?;
    tracing::info!(
        copy = %copy.display(),
        tick,
        "hot backup verified restorable"
    );
    Ok(())
}

/// Verify a finished backup copy offline, from the copy itself, and return the
/// tick it restores to (Books R5-G3).
///
/// The copy supplies everything it can: its header identity (and so the seed),
/// the store id its marker must pair with, and its checkpoint. `content`
/// supplies the content root and every non-world input; the registry is loaded
/// from it so replay decodes block states exactly as a serve of this build
/// would. Nothing is written to the archive.
pub fn verify_copy(copy: &Path, content: &crate::config::CoreConfig) -> Result<u64, CoreError> {
    let opened = open_verification(copy, &scratch_root(copy))?;
    let checkpoint = opened.checkpoint_tick.ok_or_else(|| {
        CoreError::Storage(format!(
            "hot backup {}: the copy has no committed checkpoint",
            copy.display()
        ))
    })?;
    let expected_tick = published_tick_through(copy, checkpoint)?;
    let identity = opened.identity;
    let mut config = content.clone();
    config.seed = i64::from_be_bytes(identity[16..24].try_into().expect("identity seed bytes"));
    config.world_dir = copy.to_path_buf();
    // The registry a serve would run on, including the plugin startup
    // contribution: a bare content registry refuses worlds whose native types
    // come from the deployment (see `native_checkpoint::recovery_registry`).
    let registry = std::sync::Arc::new(
        crate::native_checkpoint::recovery_registry(&config).map_err(|error| {
            CoreError::Storage(format!("content for backup verification: {error}"))
        })?,
    );
    verify_opened(copy, &config, registry, expected_tick, opened)
}
