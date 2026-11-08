use super::*;
use std::io::{Read, Write};

#[test]
fn snapshot_reads_lease_holding_handle_without_releasing_database_custody() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("state.redb");
    let copy = root.path().join("copy.redb");
    let store = NativeStore::create(&source, [42; 32]).unwrap();
    let mut first = store.snapshot_reader();
    let mut prefix = [0; 16];
    first.read_exact(&mut prefix).unwrap();
    // Independent snapshot cursors do not change redb's positioned IO or the
    // cursor of another snapshot, and never drop/reacquire the database lock.
    let mut bytes = Vec::new();
    store.snapshot_reader().read_to_end(&mut bytes).unwrap();
    assert_eq!(&bytes[..16], &prefix);
    let mut remainder = Vec::new();
    first.read_to_end(&mut remainder).unwrap();
    assert_eq!(&bytes[16..], remainder);
    assert!(NativeStore::open(&source, [42; 32]).is_err());
    let mut output = std::fs::File::create(&copy).unwrap();
    output.write_all(&bytes).unwrap();
    output.sync_all().unwrap();
    drop(output);
    let restored = NativeStore::open(&copy, [42; 32]).unwrap();
    assert_eq!(restored.identity(), [42; 32]);
    assert_eq!(
        restored.record_counts().unwrap(),
        store.record_counts().unwrap()
    );
}

#[test]
fn opening_an_empty_database_never_initializes_it() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("empty.redb");
    std::fs::write(&path, []).unwrap();
    assert!(NativeStore::open(&path, [42; 32]).is_err());
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    // Failed admission must not retain the exclusive lease.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    file.try_lock().unwrap();
}
