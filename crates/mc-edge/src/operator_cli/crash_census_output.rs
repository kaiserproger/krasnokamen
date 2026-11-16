//! Safe receipt creation before opening the disposable crash-census copy.
use std::fs::{File, OpenOptions};
use std::path::Path;

pub(super) fn create(copy: &Path, receipt: &Path) -> anyhow::Result<File> {
    let root = copy.canonicalize()?;
    let parent = receipt
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let name = receipt.file_name().ok_or_else(|| anyhow::anyhow!("receipt has no filename"))?;
    let destination = parent.join(name);
    if destination.starts_with(&root) {
        anyhow::bail!("crash census receipt must be outside the copied world");
    }
    Ok(OpenOptions::new().write(true).create_new(true).open(destination)?)
}

#[cfg(test)]
#[path = "crash_census_output_tests.rs"]
mod tests;
