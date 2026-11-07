//! Release caches and adjacent-step snapshots installed through the same
//! verified download/datagen/wire-capture path as canonical content.
use super::*;
use mc_data::protocol_versions::{SUPPORTED, cache_root, step_root};

/// The CLI and first start install the entire supported window. A failed
/// companion import is an import failure, never a successful partial install.
pub async fn import_window(request: &ImportRequest) -> Result<Vec<ImportReport>> {
    ensure!(
        request.version == mc_protocol::TARGET_RELEASE,
        "the window's canonical cache is {}; --version must be {}",
        mc_protocol::TARGET_RELEASE,
        mc_protocol::TARGET_RELEASE
    );
    let mut reports = vec![import_content(request).await?];
    for version in SUPPORTED
        .iter()
        .filter(|version| version.release != mc_protocol::TARGET_RELEASE)
    {
        let source = if version.release == GAMEPLAY_DATAPACK_RELEASE {
            request.gameplay_source.clone()
        } else {
            ContentSource::Download
        };
        reports.push(
            import_content(&ImportRequest {
                version: version.release.into(),
                source,
                gameplay_source: request.gameplay_source.clone(),
                cache: cache_root(&request.cache, mc_protocol::TARGET_RELEASE, *version),
            })
            .await?,
        );
    }
    for pair in SUPPORTED.windows(2) {
        let (lower, upper) = (pair[0], pair[1]);
        let destination = step_root(&request.cache, mc_protocol::TARGET_RELEASE, upper, lower);
        // A completed older step is immutable. A canonical re-import/bump does
        // not rebuild or remove it; a damaged snapshot is replaced atomically.
        if crate::translation::step_snapshot_valid(&destination, upper.protocol, lower.protocol) {
            continue;
        }
        crate::translation::snapshots::install(
            &cache_root(&request.cache, mc_protocol::TARGET_RELEASE, upper),
            &cache_root(&request.cache, mc_protocol::TARGET_RELEASE, lower),
            &destination,
            upper,
            lower,
        )?;
    }
    Ok(reports)
}

/// Endpoint caches contain only what that endpoint consumes: protocol reports,
/// the exact release's definitions/tags and captured network registry payloads.
/// Canonical-only gameplay extractors must never validate against another id set.
pub(super) async fn derive_profile(
    request: &ImportRequest,
    staging: &Path,
) -> Result<ImportReport> {
    let work = staging.join(".work");
    fs::create_dir_all(&work)?;
    let version_file = staging.join("version.json");
    let local = stage_local_release(&request.source, &work, &request.version, &version_file)?;
    let java = preflight_java()?;
    let jar = match local {
        Some(jar) => jar,
        None => download_release(&request.version, &work, &version_file).await?,
    };
    let bundle = unpack_bundle(&jar, &work)?;
    copy_data_subset(&bundle.inner_jar, staging)?;
    copy_client_tag_subset(&bundle.inner_jar, staging)?;
    run_datagen(&java, &jar, &work, staging)?;
    let captured = capture_registry_payloads(&java, &jar, &work, staging).await?;
    fs::remove_dir_all(&work)?;
    crate::translation::validate_endpoint(staging, &request.version)?;
    Ok(ImportReport {
        cache: staging.to_path_buf(),
        registries: captured.len(),
        entries: captured.values().map(|names| names.len()).sum(),
    })
}
