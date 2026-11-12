//! World-root admission and immutable natural geography shared by native sources.

use std::fs::File;
use std::path::{Path, PathBuf};

use thiserror::Error;

mod native_runtime;
mod world_lease;

pub use native_runtime::{NativeNaturalSource, NaturalTerrainColumn};
pub use world_lease::{WorldRootLease, acquire_exclusive_world_root_lease};

/// The file that carries a world directory's lease on Windows (see
/// [`open_for_lease`]) and in [`acquire_exclusive_world_root_lease`]. It is never
/// world data: fresh-world checks, conversions and backups skip it.
pub const WORLD_LEASE_FILE_NAME: &str = ".krasnokamen-world.lock";

/// Open the file that carries an exclusive `try_lock` lease on `path`.
///
/// Unix locks a world directory itself. Windows can neither `File::open` a
/// directory ("Access is denied", os error 5) nor `LockFileEx` a directory
/// handle (os error 87), so there a directory's lease is its
/// [`WORLD_LEASE_FILE_NAME`]. Files are leased directly on every platform.
pub fn open_for_lease(path: &Path) -> std::io::Result<File> {
    #[cfg(windows)]
    {
        if path.is_dir() {
            return std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path.join(WORLD_LEASE_FILE_NAME));
        }
    }
    File::open(path)
}

#[derive(Debug, Error)]
pub enum WorldError {
    #[error("writable world root is already leased: {root}; holder metadata: {metadata}")]
    WorldLocked { root: PathBuf, metadata: String },
    #[error("world lease {operation} failed at {path}: {source}")]
    WorldLeaseIo {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
