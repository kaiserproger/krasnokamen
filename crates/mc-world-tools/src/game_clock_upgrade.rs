//! Copy-only, explicitly requested legacy frozen-clock conversion.
use std::path::Path;
use std::sync::Arc;

use mc_core::native_checkpoint::{game_clock_upgrade::Conversion, native_identity};
use mc_core::{CoreConfig, CoreError};
use mc_world::native::NativeStore;

/// The caller must supply a disposable copy, never a live or original world.
/// Holds the exclusive world lease through recovery, commit and WAL archive.
pub fn upgrade_game_clock(config: &CoreConfig, copy: &Path) -> Result<Conversion, CoreError> {
    let storage = |error: &dyn std::fmt::Display| {
        CoreError::Storage(format!("game-clock upgrade {}: {error}", copy.display()))
    };
    let mut config = config.clone();
    config.world_dir = copy.to_path_buf();
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
    let journal = copy.join("journal");
    if journal.exists() {
        std::fs::rename(&journal, &archive).map_err(|error| storage(&error))?;
        std::fs::File::open(copy)
            .and_then(|dir| dir.sync_all())
            .map_err(|error| storage(&error))?;
    }
    println!(
        "game-clock conversion: copy={} tick={} H={} held_since={:?} active={} inactive={} dropped_debt={} work_normalized={} work_raw={} sleep_normalized={} sleep_raw={} archive={}",
        copy.display(),
        report.tick,
        report.held_ticks,
        report.held_since,
        report.active_debts,
        report.inactive_debts,
        report.dropped_debt,
        report.work_normalized,
        report.work_raw,
        report.sleep_normalized,
        report.sleep_raw,
        archive.display()
    );
    Ok(report)
}

#[cfg(test)]
#[path = "game_clock_upgrade_tests.rs"]
mod tests;
