//! Mapping-only snapshots owned by an adjacent step, independent of endpoint
//! availability. Mojang-derived reports are installed locally, never embedded.
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use mc_data::protocol_versions::Version;

use super::adjacent::tables::Reports;

pub(super) fn validate_version(root: &Path, version: Version) -> Result<()> {
    let path = root.join("version.json");
    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    ensure!(
        value["id"].as_str() == Some(version.release)
            && value["protocol_version"].as_i64() == Some(i64::from(version.protocol)),
        "{} does not describe {} (protocol {})",
        path.display(),
        version.release,
        version.protocol
    );
    Ok(())
}

/// Publish independently of either release cache, using the importer's atomic
/// staging/swap. All numeric identities come from the two exact releases.
pub fn install(
    upper_root: &Path,
    lower_root: &Path,
    destination: &Path,
    upper: Version,
    lower: Version,
) -> Result<()> {
    let parent = destination
        .parent()
        .context("step snapshot has no parent")?;
    std::fs::create_dir_all(parent)?;
    let staging = tempfile::Builder::new()
        .prefix(".step-import-")
        .tempdir_in(parent)?;
    for (source, side, version) in [(upper_root, "upper", upper), (lower_root, "lower", lower)] {
        validate_version(source, version)?;
        let target = staging.path().join(side);
        std::fs::create_dir_all(target.join("reports"))?;
        for relative in [
            "version.json",
            "reports/packets.json",
            "reports/blocks.json",
            "reports/registries.json",
        ] {
            std::fs::copy(source.join(relative), target.join(relative)).with_context(|| {
                format!(
                    "snapshot {} for {}-{}",
                    relative, upper.protocol, lower.protocol
                )
            })?;
        }
        let data = mc_data::load(source)?;
        let index: BTreeMap<&str, Vec<&str>> = data
            .registries()
            .map(|registry| {
                (
                    registry.id.as_str(),
                    registry
                        .entries
                        .iter()
                        .map(mc_data::Identifier::as_str)
                        .collect(),
                )
            })
            .collect();
        std::fs::write(target.join("synced.json"), serde_json::to_vec(&index)?)?;
    }
    // Re-read exactly what startup consumes before replacing a good step.
    super::adjacent::Profile::load(staging.path(), upper.protocol, lower.protocol)?;
    crate::content_import::publish(staging.path(), destination)
}

pub(super) fn load_side(root: &Path, protocol: i32) -> Result<(Reports, mc_data::VanillaData)> {
    let version =
        mc_data::protocol_versions::by_protocol(protocol).context("unknown step endpoint")?;
    validate_version(root, version)?;
    let path = root.join("synced.json");
    let index: BTreeMap<String, Vec<String>> = serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    let registries = index
        .into_iter()
        .map(|(id, entries)| {
            Ok(mc_data::Registry {
                id: mc_data::Identifier::parse(id)?,
                entries: entries
                    .into_iter()
                    .map(mc_data::Identifier::parse)
                    .collect::<Result<_, _>>()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((
        Reports::load(root)?,
        mc_data::VanillaData::from_registries(root, registries),
    ))
}
