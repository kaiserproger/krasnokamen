//! Clean stop and restart of an in-process world for the vanilla oracle's
//! restart phases (`crate::quirk_parity_tests`, docs/QUIRK_PARITY.md).
//!
//! A world is created and reopened the way [`Core::open`] does it: the native
//! store is opened from `native/state.redb`, the checkpoint marker is checked
//! against it, and [`recover_tick_state`] reads the checkpoint and replays the
//! WAL from disk. A stop writes the checkpoint the journal writes at a clean
//! shutdown (capture, WAL gate, native commit, marker). The old in-memory
//! state is dropped before the reopen, so nothing but the files survives.
//! The in-process driver journals no per-tick entries: the checkpoint carries
//! the whole state and the WAL tail it replays is empty.

use super::*;

pub(crate) struct OracleWorld {
    config: CoreConfig,
    /// The startup registry `Core::open` builds (no plugin deployment).
    registry: Arc<CoreRegistry>,
    manifest: CheckpointManifest,
    started_tick: u64,
}

fn storage(error: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(error.to_string())
}

fn startup_registry(config: &CoreConfig) -> Result<Arc<CoreRegistry>, CoreError> {
    let mut registry = CoreRegistry::load(config.vanilla_data_dir.as_deref())?;
    crate::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )?;
    Ok(Arc::new(registry))
}

/// The store as `Core::open` opens an existing world: refused when missing,
/// unreadable or behind its checkpoint marker.
fn open_store(config: &CoreConfig) -> Result<Arc<mc_world::native::NativeStore>, CoreError> {
    let path = config.world_dir.join("native/state.redb");
    let store = Arc::new(
        mc_world::native::NativeStore::open(
            &path,
            crate::native_checkpoint::native_identity(config),
        )
        .map_err(|e| crate::native_checkpoint::open_refusal(config, &path, e))?,
    );
    crate::native_checkpoint::tests::register_store(config, &store)?;
    let store_tick = store.checkpoint_tick().map_err(storage)?;
    mc_world::store_marker::check(&config.world_dir, store.store_uuid(), store_tick).map_err(
        |error| {
            CoreError::Storage(format!(
                "world {}: {error}; refusing to open",
                config.world_dir.display()
            ))
        },
    )?;
    Ok(store)
}

impl OracleWorld {
    /// First start in an empty `config.world_dir`: the store, the recovered
    /// fresh state and its tick-0 checkpoint, as `Core::open` creates them.
    pub(crate) fn create(config: CoreConfig) -> Result<(Self, TickState), CoreError> {
        let path = config.world_dir.join("native/state.redb");
        std::fs::create_dir_all(path.parent().expect("native store directory")).map_err(storage)?;
        drop(
            mc_world::native::NativeStore::create(
                &path,
                crate::native_checkpoint::native_identity(&config),
            )
            .map_err(|e| crate::native_checkpoint::open_refusal(&config, &path, e))?,
        );
        let store = open_store_unmarked(&config)?;
        let registry = startup_registry(&config)?;
        let (mut state, manifest, _) =
            recover_tick_state(&config, &registry, &store, true, None, None)?;
        drop(store);
        let mut world = Self {
            config,
            registry,
            manifest,
            started_tick: 0,
        };
        world.checkpoint(&mut state, true)?;
        Ok((world, state))
    }

    /// Clean stop, drop every in-memory owner, then start from disk.
    pub(crate) fn restart(
        &mut self,
        mut state: TickState,
        between: impl FnOnce(&std::path::Path),
    ) -> Result<TickState, CoreError> {
        self.checkpoint(&mut state, false)?;
        drop(state);
        if crate::native_checkpoint::tests::store_open(&self.config) {
            return Err(storage(
                "the native store is still open after the stop: in-memory state survived",
            ));
        }
        between(&self.config.world_dir);
        let store = open_store(&self.config)?;
        let (state, manifest, _) =
            recover_tick_state(&self.config, &self.registry, &store, false, None, None)?;
        self.manifest = manifest;
        self.started_tick = state.tick;
        Ok(state)
    }

    /// The journal's checkpoint of `state` at its current tick.
    fn checkpoint(&mut self, state: &mut TickState, first: bool) -> Result<(), CoreError> {
        let world_dir = self.config.world_dir.clone();
        let (work_tx, work_rx) = journal_work_channel();
        let (done, _) = std::sync::mpsc::channel();
        if !start_checkpoint(state, &work_tx, &done, state.tick) {
            return Err(storage("native checkpoint capture failed"));
        }
        let Ok(JournalWork::Checkpoint(job)) = work_rx.recv() else {
            return Err(storage("native checkpoint capture sent no job"));
        };
        let store = job
            .regions
            .store()
            .ok_or_else(|| storage("checkpoint has no native store"))?;
        let identity = crate::journal::WalIdentity::of_world(
            &world_dir,
            store.store_uuid(),
            &self.registry.blocks,
        )?;
        let mut writer = if first {
            JournalWriter::open(&world_dir, 1, identity)?
        } else {
            JournalWriter::open_recovered(
                &world_dir,
                1,
                self.manifest.tick,
                self.started_tick,
                identity,
            )?
        };
        if state.tick > self.started_tick && !first {
            writer.append(self.started_tick + 1, state.tick, &[])?;
        }
        write_checkpoint(
            &world_dir,
            &job,
            &state.plugins,
            &mut writer,
            &mut self.manifest,
        )?;
        Ok(())
    }
}

/// A just-created store has no checkpoint or marker yet (`Core::open`'s fresh path).
fn open_store_unmarked(
    config: &CoreConfig,
) -> Result<Arc<mc_world::native::NativeStore>, CoreError> {
    let path = config.world_dir.join("native/state.redb");
    let store = Arc::new(
        mc_world::native::NativeStore::open(
            &path,
            crate::native_checkpoint::native_identity(config),
        )
        .map_err(|e| crate::native_checkpoint::open_refusal(config, &path, e))?,
    );
    crate::native_checkpoint::tests::register_store(config, &store)?;
    Ok(store)
}
