use super::*;

#[test]
fn walked_paths_are_portable_metadata_keys_and_exclude_world_lease() {
    let root = tempfile::tempdir().unwrap();
    let nested = root
        .path()
        .join("dimensions")
        .join("minecraft")
        .join("the_end")
        .join("data")
        .join("minecraft");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("raids.dat"), b"retained saved data").unwrap();
    fs::write(
        root.path().join(mc_world::storage::WORLD_LEASE_FILE_NAME),
        b"lease, not world data",
    )
    .unwrap();
    let expected = "dimensions/minecraft/the_end/data/minecraft/raids.dat";
    let paths = files(root.path()).unwrap();
    assert_eq!(
        paths.len(),
        1,
        "the Windows lease must not enter import extras"
    );
    assert_eq!(paths[0].to_str(), Some(expected));
    assert!(safe_extra_path(&paths[0]));
    assert_eq!(
        auxiliary_destination(&paths[0]).unwrap().to_str(),
        Some(expected)
    );
    let hashes = hashes(root.path()).unwrap();
    assert_eq!(
        hashes.keys().map(String::as_str).collect::<Vec<_>>(),
        [expected]
    );
    assert_eq!(
        hashes[expected],
        format!("{:08x}", crc32fast::hash(b"retained saved data"))
    );
}

#[cfg(unix)]
#[test]
fn backslash_in_unix_filename_is_not_treated_as_a_windows_separator() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(r"data\escape.dat"), b"unsafe name").unwrap();
    assert!(files(root.path()).is_err());
}

#[test]
fn syncing_staged_nested_files_preserves_every_byte() {
    let staging = tempfile::tempdir().unwrap();
    let nested = staging
        .path()
        .join("dimensions")
        .join("minecraft")
        .join("the_end");
    fs::create_dir_all(&nested).unwrap();
    let payload = b"existing staging bytes, never truncate";
    fs::write(nested.join("saved.dat"), payload).unwrap();
    fs::write(staging.path().join("empty.dat"), []).unwrap();
    let before = hashes(staging.path()).unwrap();
    // On Windows, the historical read-only handle fails FlushFileBuffers.
    sync_tree(staging.path()).unwrap();
    assert_eq!(hashes(staging.path()).unwrap(), before);
    assert_eq!(fs::read(nested.join("saved.dat")).unwrap(), payload);
    assert_eq!(files(staging.path()).unwrap().len(), 2);
}
