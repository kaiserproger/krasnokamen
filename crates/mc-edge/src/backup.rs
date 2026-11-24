//! Offline native-v2 world backups: one exclusive directory lease, the native
//! database's own lock, and a SHA-256 manifest covering the store, WAL and metadata.
//! Restore validates the entire backup before creating or writing target data,
//! builds the world in a staging directory beside the target and publishes it
//! with one rename.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

const BACKUP_MANIFEST: &str = "manifest.json";
const NATIVE_DATABASE: &str = "native/state.redb";
const BACKUP_FORMAT: u32 = 2;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct BackupManifest {
    format: u32,
    krasnokamen_version: String,
    target_release: String,
    protocol_version: i32,
    world_version: u32,
    world_contract_schema: u32,
    worldgen_revision: u32,
    native_prototype_schema: u32,
    files: BTreeMap<String, String>,
}

impl BackupManifest {
    fn current() -> Self {
        Self {
            format: BACKUP_FORMAT,
            krasnokamen_version: env!("CARGO_PKG_VERSION").to_owned(),
            target_release: mc_protocol::TARGET_RELEASE.to_owned(),
            protocol_version: mc_protocol::PROTOCOL_VERSION,
            world_version: mc_protocol::WORLD_VERSION,
            world_contract_schema: mc_worldgen::world_contract::WORLD_CONTRACT_SCHEMA,
            worldgen_revision: mc_worldgen::WORLDGEN_REVISION,
            native_prototype_schema: mc_world::native::NATIVE_PROTOTYPE_SCHEMA,
            files: BTreeMap::new(),
        }
    }
}

/// Core::open leases the world through `mc_world::storage::open_for_lease`: the
/// directory inode on Unix, its `.krasnokamen-world.lock` on Windows.
/// redb 3 uses the same File::try_lock on state.redb. Locking the raw file also
/// excludes direct NativeStore users without opening/repairing the database.
struct ExclusiveLease(File);

impl ExclusiveLease {
    fn acquire(path: &Path) -> Result<Self> {
        let file = mc_world::storage::open_for_lease(path)
            .with_context(|| format!("opening {}", path.display()))?;
        file.try_lock().map_err(|error| {
            anyhow::anyhow!(
                "{} is already leased or cannot be locked: {error}",
                path.display()
            )
        })?;
        Ok(Self(file))
    }
}

impl Drop for ExclusiveLease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn is_lease_file(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(name) if name == mc_world::storage::WORLD_LEASE_FILE_NAME || name == "session.lock"
    )
}

fn validate_name(name: &str) -> Result<()> {
    // Reject noncanonical names too: aliases such as a//b or a/./b must not
    // resolve to a second manifest entry for the same destination.
    if name.is_empty()
        || name == BACKUP_MANIFEST
        || name.contains(['\\', '\0'])
        || name.split('/').any(|part| matches!(part, "" | "." | ".."))
        || Path::new(name)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || is_lease_file(Path::new(name))
    {
        bail!("backup manifest lists an unsafe path: {name:?}");
    }
    Ok(())
}

fn collect_files(root: &Path, backup: bool) -> Result<BTreeMap<String, PathBuf>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory)
            .with_context(|| format!("reading {}", directory.display()))?
        {
            let path = entry?.path();
            // DirEntry::file_type does not follow links, including directory
            // links. Never copy bytes from outside the world/backup root.
            let kind = std::fs::symlink_metadata(&path)?.file_type();
            if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
                bail!(
                    "backup refuses symlink or non-regular file: {}",
                    path.display()
                );
            }
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            if is_lease_file(&path) {
                continue;
            }
            let name = mc_data::resource_path::slash_relative(root, &path)
                .with_context(|| format!("non-UTF-8 file name under {}", root.display()))?;
            if backup && name == BACKUP_MANIFEST {
                continue;
            }
            validate_name(&name)?;
            files.insert(name, path);
        }
    }
    Ok(files)
}

/// Canonicalize existing ancestors without creating a destination merely to
/// check overlap. In particular, a corrupt backup leaves an absent target absent.
fn resolve_destination(path: &Path) -> Result<PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // A dangling symlink is not an absent destination.
            if std::fs::symlink_metadata(path).is_ok() {
                bail!("destination is a dangling symlink: {}", path.display());
            }
            let absolute = std::path::absolute(path)?;
            let name = absolute
                .file_name()
                .context("destination has no file name")?;
            let parent = absolute.parent().context("destination has no parent")?;
            Ok(resolve_destination(parent)?.join(name))
        }
        Err(error) => Err(error).with_context(|| format!("resolving {}", path.display())),
    }
}

fn refuse_overlap(source: &Path, destination: &Path) -> Result<()> {
    if source.starts_with(destination) || destination.starts_with(source) {
        bail!(
            "destination {} overlaps source {}; use a separate directory",
            destination.display(),
            source.display(),
        );
    }
    Ok(())
}

fn require_empty(path: &Path) -> Result<()> {
    match std::fs::read_dir(path) {
        Ok(entries) => {
            // A Windows directory lease is a file inside the directory.
            for entry in entries {
                if !is_lease_file(&entry?.path()) {
                    bail!("destination is not empty: {}", path.display());
                }
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

fn digest_file(path: &Path) -> Result<String> {
    let mut source = File::open(path).with_context(|| format!("reading {}", path.display()))?;
    digest_reader(&mut source)
}

fn digest_reader(source: &mut impl Read) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Copy without replacing any existing file. Hash the bytes actually copied,
/// not a separately opened view of a potentially different source file.
fn copy_file_durable(source: &Path, destination: &Path) -> Result<String> {
    copy_reader_durable(&mut File::open(source)?, destination)
}

fn copy_reader_durable(input: &mut impl Read, destination: &Path) -> Result<String> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut output = File::options()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
    }
    output.sync_all()?;
    mc_world::atomic_file::sync_parent_dir(destination)?;
    #[cfg(test)]
    tests::interruption_point(&tests::Step::Copied(destination));
    Ok(format!("{:x}", hasher.finalize()))
}

/// Copy a stopped native-v2 world, including its WAL and world metadata.
/// Neither an active Core nor a separately opened native store may own it.
pub fn run_backup(world_dir: &Path, out: &Path) -> Result<usize> {
    let world = std::fs::canonicalize(world_dir)
        .with_context(|| format!("resolving world {}", world_dir.display()))?;
    if !world.is_dir() {
        bail!("backup source is not a directory: {}", world.display());
    }
    // A staging directory is no world, even with a valid marker; refuse it
    // before a lease file or redb repair can touch it.
    mc_world::store_marker::refuse_unpublished(&world)
        .map_err(|error| anyhow::anyhow!("backup refused: {error}"))?;
    let out = resolve_destination(out)?;
    refuse_overlap(&world, &out)?;
    let _world_lease = ExclusiveLease::acquire(&world)?;
    let files = collect_files(&world, false)?;
    if !files.contains_key(NATIVE_DATABASE) {
        bail!("backup requires a native-v2 world with {NATIVE_DATABASE}");
    }
    // Books R4-P1: a backup of a store that disagrees with its checkpoint marker
    // would preserve an already lost world. The pair is verified, not rewritten;
    // a store redb must repair is repaired here first, as the next serve would.
    // This opens the database through its own handle, so it runs before the
    // exclusive byte-range lease on that file (which also denies a second handle
    // on Windows).
    let (store_uuid, store_tick) =
        mc_world::native::NativeStore::read_marker_fields(&world.join(NATIVE_DATABASE))
            .map_err(|error| anyhow::anyhow!("backup source store: {error}"))?;
    mc_world::store_marker::verify(&world, store_uuid, store_tick)
        .map_err(|error| anyhow::anyhow!("backup refused: {error}"))?;
    // A restored copy always has `journal/` and replay refuses one without it,
    // its tail being lost. Copying such a copy would hand restore a backup it
    // republishes with a fresh empty journal, so refuse it here, independent of
    // the content check that may or may not follow.
    let branch = mc_world::store_marker::branch(&world)
        .map_err(|error| anyhow::anyhow!("backup refused: {error}"))?;
    if branch != mc_world::store_marker::ROOT_BRANCH && !world.join("journal").is_dir() {
        bail!(
            "backup refused: {} is a restored copy on branch {:016x} without its journal/ \
             directory; its WAL tail is lost, so it is not backed up as a valid world",
            world.display(),
            u64::from_be_bytes(branch),
        );
    }
    let mut database_lease = ExclusiveLease::acquire(&world.join(NATIVE_DATABASE))?;
    require_empty(&out)?;
    std::fs::create_dir_all(&out)?;
    let _output_lease = ExclusiveLease::acquire(&out)?;
    require_empty(&out)?;
    // The journal directory belongs to the world even when it is empty: replay
    // refuses a restored copy without one, so a backup of such a copy keeps it.
    if world.join("journal").is_dir() {
        std::fs::create_dir(out.join("journal"))?;
    }
    let mut manifest = BackupManifest::current();
    for (name, source) in files {
        // Windows byte-range locks also deny reads through a second handle in
        // this process. Read the database through the handle holding its lease.
        let digest = if name == NATIVE_DATABASE {
            copy_reader_durable(&mut database_lease.0, &out.join(&name))
        } else {
            copy_file_durable(&source, &out.join(&name))
        }
        .with_context(|| format!("backing up {name}"))?;
        manifest.files.insert(name, digest);
    }
    write_manifest(&out, &manifest)?;
    // The caller prints the final status: a backup is not "complete" until the
    // copy it just wrote has been verified or explicitly marked unverified.
    Ok(manifest.files.len())
}

/// Create the manifest last: a directory without it is never a valid backup.
fn write_manifest(out: &Path, manifest: &BackupManifest) -> Result<()> {
    let path = out.join(BACKUP_MANIFEST);
    let mut file = File::options().write(true).create_new(true).open(&path)?;
    file.write_all(&serde_json::to_vec_pretty(manifest)?)?;
    file.sync_all()?;
    mc_world::atomic_file::sync_parent_dir(&path)?;
    mc_world::atomic_file::sync_parent_dir(out)?;
    Ok(())
}

/// Prove a written backup restores with this build (Books R5 G3): open the copy
/// read-only, derive the world's own seed from its store identity, and run the
/// ordinary recovery reader and WAL replay over it, requiring the tick the copy
/// claims. `content` supplies the content root and every non-world input; the
/// seed comes from the copy itself, so a backup of any world can be verified.
/// A copy that needs redb repair or cannot replay is reported, never called a
/// verified backup.
pub fn verify_backup(dir: &Path, content: &mc_core::config::CoreConfig) -> Result<()> {
    // One verification instance opens the copy as the next serve would
    // (read-only, or through a repaired scratch copy when redb would repair a
    // live image) and supplies the identity, store id and checkpoint from
    // itself: no second read-only open, which a hot copy fails by design.
    let tick = mc_core::hot_backup::verify_copy(dir, content)
        .map_err(|error| anyhow::anyhow!("backup {} does not restore: {error}", dir.display()))?;
    println!(
        "backup of {} verified: restores to tick {tick}",
        dir.display()
    );
    Ok(())
}

/// Verify a backup this command just wrote and, on failure, withdraw it: the
/// manifest is written last, so a directory without one is never a valid backup.
pub fn verify_written_backup(out: &Path, content: &mc_core::config::CoreConfig) -> Result<()> {
    if let Err(error) = verify_backup(out, content) {
        let _ = std::fs::remove_file(out.join(BACKUP_MANIFEST));
        bail!("{error}; the backup was withdrawn (no manifest kept)");
    }
    Ok(())
}

/// `YYYYMMDDTHHMMSSZ` for Unix seconds (proleptic Gregorian, civil-from-days).
fn utc_stamp(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64 + 719_468;
    let seconds = unix_seconds % 86_400;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

/// Hot backup of a running core into `backup_dir/backup-<UTC>-tick<N>/`, the
/// same manifest format `run_restore` verifies. The copy is built in a hidden
/// `.partial` directory and renamed only once the manifest is durable, so an
/// interrupted or failed backup never looks like a valid one.
pub fn run_hot_backup(
    core: &mc_core::Core,
    backup_dir: &Path,
) -> Result<(PathBuf, mc_core::hot_backup::HotBackup)> {
    std::fs::create_dir_all(backup_dir)
        .with_context(|| format!("creating {}", backup_dir.display()))?;
    let backup_dir = std::fs::canonicalize(backup_dir)?;
    let stamp = utc_stamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
    );
    let partial = backup_dir.join(format!(".backup-{stamp}.partial"));
    std::fs::create_dir(&partial).with_context(|| format!("creating {}", partial.display()))?;
    let result = (|| {
        let backup = core.backup(&partial)?;
        let mut manifest = BackupManifest::current();
        manifest.files.clone_from(&backup.files);
        write_manifest(&partial, &manifest)?;
        let finished = backup_dir.join(format!("backup-{stamp}-tick{}", backup.wal_tick));
        std::fs::rename(&partial, &finished)
            .with_context(|| format!("publishing {}", finished.display()))?;
        mc_world::atomic_file::sync_parent_dir(&finished)?;
        Ok((finished, backup))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&partial);
    }
    result
}

fn read_manifest(from: &Path) -> Result<BackupManifest> {
    let path = from.join(BACKUP_MANIFEST);
    if !std::fs::symlink_metadata(&path)?.file_type().is_file() {
        bail!("backup manifest is not a regular file: {}", path.display());
    }
    let manifest: BackupManifest = serde_json::from_slice(&std::fs::read(&path)?)
        .with_context(|| format!("parsing {}", path.display()))?;
    let current = BackupManifest::current();
    if manifest.format != BACKUP_FORMAT {
        bail!(
            "unsupported native backup format {}; expected {BACKUP_FORMAT}",
            manifest.format
        );
    }
    if manifest.target_release != current.target_release
        || manifest.protocol_version != current.protocol_version
        || manifest.world_version != current.world_version
        || manifest.world_contract_schema != current.world_contract_schema
        || manifest.worldgen_revision != current.worldgen_revision
        || manifest.native_prototype_schema != current.native_prototype_schema
    {
        bail!("backup release or native world schema does not match this binary; restore refused");
    }
    if !manifest.files.contains_key(NATIVE_DATABASE) {
        bail!("backup manifest has no {NATIVE_DATABASE}; restore refused");
    }
    for name in manifest.files.keys() {
        validate_name(name)?;
    }
    Ok(manifest)
}

/// Verify every manifest path and hash before writing any world data, then
/// build the world in a staging directory beside the target ([`stage_world`])
/// and publish it with one rename ([`publish`]): an interrupted restore leaves
/// the target as it was, or the complete restored world, never a partial one.
/// Restore never merges into a nonempty target, follows links, or overlaps its
/// source, and it runs on Unix only (see [`publish`]).
pub fn run_restore(from: &Path, world_dir: &Path) -> Result<()> {
    if cfg!(not(unix)) {
        bail!(
            "restore runs on Unix (Linux, macOS) only: it publishes the restored world by \
             renaming it over the empty target it keeps leased, which this platform cannot do; \
             restore on Linux or macOS and move the finished world directory whole"
        );
    }
    let from = std::fs::canonicalize(from)
        .with_context(|| format!("resolving backup {}", from.display()))?;
    let target = resolve_destination(world_dir)?;
    refuse_overlap(&from, &target)?;
    mc_world::store_marker::refuse_unpublished(&target)
        .map_err(|error| anyhow::anyhow!("restore target refused: {error}"))?;
    let staging = staging_path(&target)?;
    let _source_lease = ExclusiveLease::acquire(&from)?;
    let manifest = read_manifest(&from)?;
    let files = collect_files(&from, true)?;
    if !files.keys().eq(manifest.files.keys()) {
        bail!("backup files do not match manifest; restore refused");
    }
    let mut database_lease = ExclusiveLease::acquire(&from.join(NATIVE_DATABASE))?;
    // Lease an existing target before inspecting it; a live core must fail on
    // its actual lock even when it has not yet published any files.
    let target_lease = if target.try_exists()? {
        Some(ExclusiveLease::acquire(&target)?)
    } else {
        None
    };
    require_empty(&target)?;
    #[cfg(unix)]
    if target_lease.is_some() {
        refuse_mount_point(&target)?;
    }
    for (name, source) in &files {
        let actual = if name == NATIVE_DATABASE {
            digest_reader(&mut database_lease.0)?
        } else {
            digest_file(source)?
        };
        let expected = &manifest.files[name];
        if &actual != expected {
            bail!(
                "backup file {name} failed verification: expected {expected}, actual {actual}; nothing was restored"
            );
        }
    }
    // A staging directory is never reused or removed silently: it holds what an
    // interrupted restore copied, which the operator may still want to inspect.
    if std::fs::symlink_metadata(&staging).is_ok() {
        bail!(
            "{} is left by an interrupted restore into {}; remove it and run restore again",
            staging.display(),
            target.display()
        );
    }
    // Only a target this restore's own exclusive mkdir created is its own; one
    // that appeared since the check above belongs to someone else.
    let created_target = target_lease.is_none() && create_target(&target)?;
    let target_lease = match target_lease {
        Some(lease) => lease,
        None => ExclusiveLease::acquire(&target)?,
    };
    require_empty(&target)?;
    database_lease.0.rewind()?;
    // The staging directory is created under the target lease: it is this
    // restore's own until the lease drops, after cleanup.
    std::fs::create_dir(&staging).with_context(|| format!("creating {}", staging.display()))?;
    let published = stage_world(&from, &staging, &files, &manifest, &mut database_lease.0)
        .and_then(|()| {
            #[cfg(test)]
            tests::interruption_point(&tests::Step::Publishing);
            publish(&staging, &target, &target_lease)
        });
    if let Err(error) = published {
        // Cleanup still holds the target lease and removes only what this
        // restore owns: its staging directory, and the target only if this
        // restore created it and it is still empty (`remove_dir` refuses
        // anything else). Whatever else is at the target stays as found.
        let _ = std::fs::remove_dir_all(&staging);
        if created_target {
            let _ = std::fs::remove_dir(&target);
        }
        drop(target_lease);
        return Err(error);
    }
    #[cfg(test)]
    tests::interruption_point(&tests::Step::Published);
    mc_world::atomic_file::sync_parent_dir(&target).with_context(|| {
        format!(
            "the restored world is published at {}, but its directory entry was not synced",
            target.display()
        )
    })?;
    println!(
        "restore of {} into {} complete: {} files verified",
        from.display(),
        target.display(),
        files.len(),
    );
    Ok(())
}

/// `<target>.restore-staging`: beside the target, so on its filesystem and one
/// rename away from it. The name is reserved: every open refuses it
/// (`store_marker::refuse_unpublished`), whatever marker the copy holds.
fn staging_path(target: &Path) -> Result<PathBuf> {
    let mut name = target
        .file_name()
        .context("destination has no file name")?
        .to_os_string();
    name.push(mc_world::store_marker::RESTORE_STAGING_SUFFIX);
    Ok(target.with_file_name(name))
}

/// Create `target` with an exclusive mkdir, its parents durably first, and
/// report whether this call created it: an existing directory is not ours.
fn create_target(target: &Path) -> Result<bool> {
    let parent = target.parent().context("destination has no parent")?;
    mc_world::atomic_file::create_dir_all_durable(parent)?;
    match std::fs::create_dir(target) {
        Ok(()) => {
            mc_world::atomic_file::sync_parent_dir(target)?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error).with_context(|| format!("creating {}", target.display())),
    }
}

/// The restored world is published by renaming a directory beside the target
/// over it, which cannot replace a mount point: refuse one before copying.
#[cfg(unix)]
fn refuse_mount_point(target: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let parent = target.parent().context("destination has no parent")?;
    if std::fs::metadata(target)?.dev() != std::fs::metadata(parent)?.dev() {
        bail!(
            "destination {} is a mount point; restore into a new directory inside it",
            target.display()
        );
    }
    Ok(())
}

/// Build the restored world in `staging` (Books R4-P1 pair check, then the
/// copy's own branch) and make every name in it durable.
///
/// The staging directory never opens as a world: its name is reserved
/// ([`staging_path`]), so every open refuses it even once the copy's own marker
/// is written. The backup's marker is also verified from memory and never
/// copied, so until that marker is written as the last step the staged store
/// has a checkpoint and no marker, which every open refuses on its own.
fn stage_world(
    from: &Path,
    staging: &Path,
    files: &BTreeMap<String, PathBuf>,
    manifest: &BackupManifest,
    database: &mut File,
) -> Result<()> {
    // A restored copy always has a journal directory, an empty one included:
    // replay refuses a copy on a branch of its own without one (a lost tail).
    std::fs::create_dir(staging.join("journal"))?;
    let mut marker = None;
    for (name, source) in files {
        let expected = &manifest.files[name];
        let actual = if name == mc_world::store_marker::MARKER_FILE {
            let bytes = std::fs::read(source).with_context(|| format!("restoring {name}"))?;
            let digest = digest_reader(&mut bytes.as_slice())?;
            marker = Some(bytes);
            digest
        } else if name == NATIVE_DATABASE {
            copy_reader_durable(database, &staging.join(name))
                .with_context(|| format!("restoring {name}"))?
        } else {
            copy_file_durable(source, &staging.join(name))
                .with_context(|| format!("restoring {name}"))?
        };
        if &actual != expected {
            bail!("backup file {name} changed while restoring; nothing was restored");
        }
    }
    // The backup's marker and store must agree — the same store id and a store
    // checkpoint at or after the marker's tick — and the marker must be
    // present: every world this build writes has one, so a backup without it is
    // not restorable. A backup that pairs one world's marker with another's
    // store, or a store rolled back behind its marker, is refused instead of
    // restored as a valid world.
    let marker_path = from.join(mc_world::store_marker::MARKER_FILE);
    let database = staging.join(NATIVE_DATABASE);
    let paired = marker
        .ok_or_else(|| "the backup has no checkpoint marker".to_owned())
        .and_then(|bytes| {
            mc_world::store_marker::StoreMarker::decode(&bytes).map_err(|error| {
                format!(
                    "checkpoint marker {} is corrupt: {error}",
                    marker_path.display()
                )
            })
        })
        .and_then(|copied| {
            let (store_uuid, store_tick) =
                mc_world::native::NativeStore::read_marker_fields(&database)
                    .map_err(|error| format!("restored {}: {error}", database.display()))?;
            copied
                .admits(&marker_path, store_uuid, store_tick)
                .map_err(|error| error.to_string())?;
            // A backup of a restored copy carries that copy's `journal/`, an
            // empty one included; without it the copy's tail was lost before
            // the backup, and a fresh empty journal must not stand in for it.
            if copied.branch != mc_world::store_marker::ROOT_BRANCH
                && !from.join("journal").is_dir()
            {
                return Err(format!(
                    "the backup is of a restored copy on branch {:016x} and has no journal/ \
                     directory: that copy's WAL tail was lost",
                    u64::from_be_bytes(copied.branch)
                ));
            }
            start_branch(staging, copied, store_tick.unwrap_or(0))
        });
    if let Err(error) = paired {
        bail!("restored world is inconsistent: {error}; nothing was kept");
    }
    sync_tree(staging)?;
    mc_world::atomic_file::sync_parent_dir(staging)?;
    Ok(())
}

/// Give the staged copy its own branch (DIVERGED-BACKUP-COPIES-SHARE-STORE-IDENTITY).
///
/// The store id is the world's identity and travels with every backup, so two
/// copies of one backup — or a copy and the world it came from — would otherwise
/// replay each other's WAL tails. A random branch that is neither the root nor
/// the backup's own is minted, the copied tail above the store's checkpoint
/// moves onto it, and the copy's first marker, naming it, is written last.
fn start_branch(
    staging: &Path,
    copied: mc_world::store_marker::StoreMarker,
    store_tick: u64,
) -> Result<(), String> {
    let branch = mc_world::store_marker::mint_branch(copied.branch);
    mc_core::journal::rebind_tail(
        staging,
        store_tick,
        copied.store_uuid,
        copied.branch,
        branch,
        |_| {
            #[cfg(test)]
            tests::interruption_point(&tests::Step::Rebinding);
        },
    )
    .map_err(|error| error.to_string())?;
    mc_world::store_marker::write(staging, copied.store_uuid, branch, copied.checkpoint_tick)
        .map_err(|error| error.to_string())
}

/// Fsync every directory of the staged tree, `directory` itself last. Each file
/// was synced by the step that wrote it (copy, rebind, marker), but a copy syncs
/// only its own parent: the links of `journal/`, `native/` and nested data
/// directories inside the staging directory are made durable here.
fn sync_tree(directory: &Path) -> Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sync_tree(&entry.path())?;
        }
    }
    mc_core::journal::fsync_dir(directory)
        .with_context(|| format!("syncing {}", directory.display()))
}

/// Publish the staged world at `target` with one rename. The target is an
/// empty directory this restore leases (lease files at most), and `_lease` is
/// that lease: Unix renames a directory over an empty one atomically, so the
/// leased empty target stands until the rename and nobody else can take it in
/// between. The caller keeps the lease until it is done with the target. No
/// Windows variant exists: there the lease is a file inside the target and no
/// directory is renamed over another, so the target would have to be released
/// and removed first, and another process could take it in that gap.
fn publish(staging: &Path, target: &Path, _lease: &ExclusiveLease) -> Result<()> {
    for entry in std::fs::read_dir(target)? {
        let path = entry?.path();
        if !is_lease_file(&path) {
            bail!("destination is not empty: {}", target.display());
        }
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }
    std::fs::rename(staging, target)
        .with_context(|| format!("publishing the restored world at {}", target.display()))
}

#[cfg(test)]
#[path = "backup_tests.rs"]
mod tests;
