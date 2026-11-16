use super::*;

#[test]
fn receipt_cannot_replace_or_write_inside_copied_world() {
    let temp = tempfile::tempdir().unwrap();
    let world = temp.path().join("world");
    let native = world.join("native");
    std::fs::create_dir_all(&native).unwrap();
    let database = native.join("state.redb");
    std::fs::write(&database, b"original database bytes").unwrap();
    assert!(create(&world, &database).is_err());
    assert!(create(&world, &world.join("new-receipt.json")).is_err());
    assert_eq!(std::fs::read(&database).unwrap(), b"original database bytes");

    let external = temp.path().join("receipt.json");
    std::fs::write(&external, b"existing receipt").unwrap();
    assert!(create(&world, &external).is_err());
    assert_eq!(std::fs::read(&external).unwrap(), b"existing receipt");
    std::fs::remove_file(&external).unwrap();
    assert!(create(&world, &external).is_ok());
}

#[cfg(unix)]
#[test]
fn receipt_symlink_cannot_replace_database() {
    let temp = tempfile::tempdir().unwrap();
    let world = temp.path().join("world");
    std::fs::create_dir(&world).unwrap();
    let database = world.join("state.redb");
    std::fs::write(&database, b"untouched").unwrap();
    let alias = temp.path().join("receipt.json");
    std::os::unix::fs::symlink(&database, &alias).unwrap();
    assert!(create(&world, &alias).is_err());
    assert_eq!(std::fs::read(database).unwrap(), b"untouched");
}
