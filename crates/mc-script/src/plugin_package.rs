//! A deployed plugin package: its root directory and validated manifest.

use std::path::{Path, PathBuf};

/// One deployed package the script host discovered.
///
/// The host already parses each package manifest; this exposes the facts core
/// needs to trigger authored-data subsystems (an authored catalog directory and
/// the manifest's declared required features) without a second manifest reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPackage {
    plugin_id: String,
    package_dir: PathBuf,
    required_features: Vec<String>,
}

impl PluginPackage {
    #[must_use]
    pub fn new(
        plugin_id: impl Into<String>,
        package_dir: impl Into<PathBuf>,
        required_features: Vec<String>,
    ) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            package_dir: package_dir.into(),
            required_features,
        }
    }

    #[must_use]
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    /// Root directory of the deployed package (`plugin.toml`'s parent).
    #[must_use]
    pub fn package_dir(&self) -> &Path {
        &self.package_dir
    }

    #[must_use]
    pub fn required_features(&self) -> &[String] {
        &self.required_features
    }

    /// Whether the deployed manifest declared `feature` in `required_features`.
    #[must_use]
    pub fn declares_feature(&self, feature: &str) -> bool {
        self.required_features.iter().any(|entry| entry == feature)
    }
}
