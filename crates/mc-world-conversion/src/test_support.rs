//! Conversion fixtures use the public native format, never simulation test hooks.
use mc_core::dimensions::Dimension;
use mc_core::journal::CheckpointManifest;
use mc_core::native_checkpoint::{
    domain_versions, generation_records, native_identity, record, tag_record,
};
use mc_core::registry::CoreRegistry;
use mc_core::{CoreConfig, CoreError};
use mc_nbt::Tag;
use mc_world::native::{NativeCheckpoint, NativeStore};
use std::path::PathBuf;
use std::sync::Arc;

pub(super) fn config(world: PathBuf, seed: i64) -> CoreConfig {
    let mut config = CoreConfig::new(world, seed);
    config.vanilla_data_dir = Some(content_dir());
    config
}

fn content_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla")
}

pub(super) fn registry() -> Result<CoreRegistry, CoreError> {
    CoreRegistry::load(Some(&content_dir()))
}

fn storage(error: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(error.to_string())
}

pub(super) fn fixture_store(config: &CoreConfig) -> Result<Arc<NativeStore>, CoreError> {
    let path = config.world_dir.join("native/state.redb");
    if path.exists() {
        return NativeStore::open(&path, native_identity(config))
            .map(Arc::new)
            .map_err(storage);
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(storage)?;
    let mut registry = CoreRegistry::load(config.vanilla_data_dir.as_deref())?;
    mc_core::plugin_startup::apply(
        &mut registry,
        &mc_plugin_host::DeploymentContribution::default(),
        None,
        &[],
    )?;
    let store = Arc::new(NativeStore::create(&path, native_identity(config)).map_err(storage)?);
    let mut manifest = CheckpointManifest {
        domain_versions: domain_versions(),
        world_spawn: Some([0, 80, 0]),
        ..Default::default()
    };
    for dimension in [Dimension::Nether, Dimension::End] {
        manifest.dimensions.insert(
            dimension,
            Box::new(CheckpointManifest {
                domain_versions: domain_versions(),
                world_spawn: Some([0, 80, 0]),
                ..Default::default()
            }),
        );
    }
    let mut globals = generation_records(config)?;
    globals.insert(
        "checkpoint".into(),
        record(0, serde_json::to_vec(&manifest).unwrap()),
    );
    globals.insert(
        "scoreboard".into(),
        tag_record(0, &mc_core::scoreboard::Scoreboard::default().to_nbt())?,
    );
    globals.insert(
        "bossbars".into(),
        tag_record(0, &mc_core::bossbar::BossBars::default().to_nbt())?,
    );
    for key in ["level_metadata", "conversion_extras"] {
        globals.insert(key.into(), tag_record(0, &Tag::Compound(Vec::new()))?);
    }
    globals.insert(
        "plugins".into(),
        record(
            0,
            serde_json::to_vec(&mc_core::plugin::PluginState::default()).unwrap(),
        ),
    );
    globals.insert(
        "settlement_sequence".into(),
        record(0, serde_json::to_vec(&0u64).unwrap()),
    );
    globals.insert(
        "plugin_startup".into(),
        record(0, mc_core::plugin_startup::world_identity(&registry)?),
    );
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick: 0,
                previous_tick: 0,
                chunks: &[],
                entities: &[],
                players: &[],
                globals: &globals,
            },
            &registry.blocks,
            &registry.items,
        )
        .map_err(storage)?;
    Ok(store)
}

pub(super) fn read_manifest(store: &NativeStore) -> Result<CheckpointManifest, CoreError> {
    let image = store
        .checkpoint_reader()
        .map_err(storage)?
        .read_checkpoint_metadata()
        .map_err(storage)?;
    serde_json::from_slice(&image.globals["checkpoint"].payload).map_err(storage)
}

pub(super) fn write_manifest(
    config: &CoreConfig,
    store: &NativeStore,
    manifest: &CheckpointManifest,
) -> Result<(), CoreError> {
    let registry = CoreRegistry::load(config.vanilla_data_dir.as_deref())?;
    let mut image = store.read_checkpoint().map_err(storage)?;
    let mut manifest = manifest.clone();
    manifest.domain_versions = domain_versions();
    manifest.tick = manifest.tick.max(image.tick).max(1);
    let previous: CheckpointManifest =
        serde_json::from_slice(&image.globals["checkpoint"].payload).map_err(storage)?;
    for dimension in [Dimension::Nether, Dimension::End] {
        if let std::collections::btree_map::Entry::Vacant(entry) =
            manifest.dimensions.entry(dimension)
        {
            let child = previous.dimensions.get(&dimension).ok_or_else(|| {
                CoreError::Storage(format!("native fixture missing {dimension:?} checkpoint"))
            })?;
            entry.insert(child.clone());
        }
    }
    for child in manifest.dimensions.values_mut() {
        child.tick = manifest.tick;
        child.domain_versions = domain_versions();
    }
    for global in image.globals.values_mut() {
        global.captured_tick = manifest.tick;
    }
    image.globals.insert(
        "checkpoint".into(),
        record(manifest.tick, serde_json::to_vec(&manifest).unwrap()),
    );
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick: manifest.tick,
                previous_tick: image.tick,
                chunks: &[],
                entities: &image.entities,
                players: &[],
                globals: &image.globals,
            },
            &registry.blocks,
            &registry.items,
        )
        .map_err(storage)
}

pub(super) struct Running {
    pub core: mc_core::Core,
    output: tokio::sync::broadcast::Receiver<mc_core::boundary::TickOutput>,
}

impl Running {
    pub fn open(config: CoreConfig) -> Self {
        let core = mc_core::Core::open(config).unwrap();
        let output = core.subscribe();
        Self { core, output }
    }

    pub async fn until<T>(
        &mut self,
        mut observe: impl FnMut(
            &mc_core::boundary::TickOutput,
            &[(
                mc_core::boundary::ConnectionId,
                Vec<mc_core::boundary::PlayerChange>,
            )],
        ) -> Option<T>,
    ) -> T {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                let output = self.output.recv().await.expect("live core publication");
                let changes = self.core.try_drain_outbox_through(output.tick);
                if let Some(value) = observe(&output, &changes) {
                    return value;
                }
            }
        })
        .await
        .expect("expected core observation")
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.core.shutdown();
    }
}
