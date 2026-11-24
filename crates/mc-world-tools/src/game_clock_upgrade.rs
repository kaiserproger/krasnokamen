//! Copy-only, explicitly requested legacy frozen-clock conversion.
use std::path::Path;
use std::sync::Arc;

use mc_core::native_checkpoint::{game_clock_upgrade::Conversion, native_identity};
use mc_core::{CoreConfig, CoreError};
use mc_world::native::NativeStore;

/// The caller must supply a disposable copy, never a live or original world.
/// Holds the exclusive world lease through recovery, commit and WAL archive.
/// The archive takes the files of `journal/`, never the directory itself: a
/// restored copy (a branch of its own) is refused without `journal/`.
pub fn upgrade_game_clock(config: &CoreConfig, copy: &Path) -> Result<Conversion, CoreError> {
    let storage = |error: &dyn std::fmt::Display| {
        CoreError::Storage(format!("game-clock upgrade {}: {error}", copy.display()))
    };
    if std::fs::canonicalize(copy).map_err(|error| storage(&error))?
        == std::fs::canonicalize(&config.world_dir).map_err(|error| storage(&error))?
    {
        return Err(storage(
            &"--copy must name a disposable copy, not the configured original world",
        ));
    }
    let mut config = config.clone();
    config.world_dir = copy.to_path_buf();
    mc_world::store_marker::refuse_unpublished(copy).map_err(|error| storage(&error))?;
    let lease = mc_world::storage::open_for_lease(copy).map_err(|error| storage(&error))?;
    lease.try_lock().map_err(|error| storage(&error))?;
    let store = Arc::new(
        NativeStore::open_checked(copy, native_identity(&config))
            .map_err(|error| storage(&error))?,
    );
    let tick = store
        .checkpoint_tick()
        .map_err(|error| storage(&error))?
        .ok_or_else(|| storage(&"missing native checkpoint"))?;
    let archive = copy.join(format!("journal-before-game-clock-{tick}"));
    if archive.exists() {
        return Err(storage(&format!(
            "WAL archive already exists: {}",
            archive.display()
        )));
    }
    let report =
        mc_core::native_checkpoint::game_clock_upgrade::write_converted_checkpoint(&config, store)?;
    archive_covered_journal(copy, &archive).map_err(|error| storage(&error))?;
    Ok(report)
}

/// Move every file of `journal/` unchanged into `archive`, keeping `journal/`
/// itself, now empty. The committed checkpoint covers every batch, and replay
/// skips a covered batch, so an interruption between two moves leaves a copy
/// that opens: no window exists in which the directory is missing. The archive
/// is made durable before the first move and both directories after the last.
fn archive_covered_journal(copy: &Path, archive: &Path) -> std::io::Result<()> {
    let journal = copy.join("journal");
    if !journal.is_dir() {
        return Ok(());
    }
    std::fs::create_dir(archive)?;
    mc_world::atomic_file::sync_parent_dir(archive)?;
    for entry in std::fs::read_dir(&journal)? {
        let name = entry?.file_name();
        std::fs::rename(journal.join(&name), archive.join(&name))?;
    }
    mc_core::journal::fsync_dir(archive)?;
    mc_core::journal::fsync_dir(&journal)
}

#[cfg(test)]
#[path = "game_clock_upgrade_tests.rs"]
mod tests;
