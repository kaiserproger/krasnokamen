//! Books R4-P1: the independent checkpoint marker, on real files.
use super::store_marker::{self, MarkerError, StoreMarker};

const STORE: [u8; 16] = [0x11; 16];
const OTHER: [u8; 16] = [0x22; 16];

#[test]
fn encode_decode_round_trip_and_checksum() {
    let marker = StoreMarker::new(STORE, 1234);
    let bytes = marker.encode();
    assert_eq!(StoreMarker::decode(&bytes).unwrap(), marker);

    let mut damaged = bytes;
    damaged[9] ^= 0x80;
    assert_eq!(
        StoreMarker::decode(&damaged).unwrap_err().to_string(),
        "checksum mismatch"
    );
    assert_eq!(
        StoreMarker::decode(&bytes[..bytes.len() - 1])
            .unwrap_err()
            .to_string(),
        "wrong length"
    );
    let mut wrong_version = bytes;
    wrong_version[7] = 9;
    assert_eq!(
        StoreMarker::decode(&wrong_version).unwrap_err().to_string(),
        "wrong magic or format version"
    );
}

#[test]
fn write_is_durable_and_readable() {
    let root = tempfile::tempdir().unwrap();
    store_marker::write(root.path(), STORE, 7).unwrap();
    assert_eq!(
        store_marker::read(root.path()).unwrap(),
        Some(StoreMarker::new(STORE, 7))
    );
    // No temporary file is left behind.
    assert!(
        !root
            .path()
            .join(format!("{}.tmp", store_marker::MARKER_FILE))
            .exists()
    );
    // A second write replaces the marker.
    store_marker::write(root.path(), STORE, 9).unwrap();
    assert_eq!(
        store_marker::read(root.path()).unwrap(),
        Some(StoreMarker::new(STORE, 9))
    );
}

#[test]
fn corrupt_marker_is_refused() {
    let root = tempfile::tempdir().unwrap();
    store_marker::write(root.path(), STORE, 3).unwrap();
    std::fs::write(store_marker::path(root.path()), b"not a marker").unwrap();
    let error = store_marker::check(root.path(), STORE, Some(3)).unwrap_err();
    assert!(matches!(error, MarkerError::Corrupt { .. }), "{error}");
    assert!(error.to_string().contains("corrupt"), "{error}");
}

#[test]
fn a_new_store_without_checkpoint_or_marker_opens() {
    let root = tempfile::tempdir().unwrap();
    store_marker::check(root.path(), STORE, None).unwrap();
}

#[test]
fn missing_marker_next_to_a_checkpoint_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let error = store_marker::check(root.path(), STORE, Some(12)).unwrap_err();
    assert!(matches!(error, MarkerError::Missing { .. }), "{error}");
    let text = error.to_string();
    assert!(text.contains("missing"), "{text}");
    assert!(text.contains("checkpoint 12"), "{text}");
}

#[test]
fn foreign_store_id_is_refused_and_names_both_ids() {
    let root = tempfile::tempdir().unwrap();
    store_marker::write(root.path(), OTHER, 4).unwrap();
    let error = store_marker::check(root.path(), STORE, Some(4)).unwrap_err();
    assert!(matches!(error, MarkerError::Foreign { .. }), "{error}");
    let text = error.to_string();
    assert!(
        text.contains(&uuid::Uuid::from_bytes(OTHER).to_string()),
        "{text}"
    );
    assert!(
        text.contains(&uuid::Uuid::from_bytes(STORE).to_string()),
        "{text}"
    );
}

#[test]
fn store_behind_the_marker_is_refused_and_names_both_ticks() {
    let root = tempfile::tempdir().unwrap();
    store_marker::write(root.path(), STORE, 40).unwrap();
    let error = store_marker::check(root.path(), STORE, Some(39)).unwrap_err();
    assert!(matches!(error, MarkerError::Behind { .. }), "{error}");
    let text = error.to_string();
    assert!(text.contains("checkpoint 39"), "{text}");
    assert!(text.contains("marker 40"), "{text}");
    // A store with no checkpoint at all is behind a real marker too.
    let error = store_marker::check(root.path(), STORE, None).unwrap_err();
    assert!(matches!(error, MarkerError::Behind { .. }), "{error}");
}

#[test]
fn store_ahead_of_the_marker_opens_and_rewrites_it() {
    let root = tempfile::tempdir().unwrap();
    store_marker::write(root.path(), STORE, 5).unwrap();
    store_marker::check(root.path(), STORE, Some(9)).unwrap();
    assert_eq!(
        store_marker::read(root.path()).unwrap(),
        Some(StoreMarker::new(STORE, 9))
    );
}

#[test]
fn write_failure_reports_the_path() {
    // A regular file where the world directory should be: the temporary marker
    // cannot be created, and the error names the marker path.
    let root = tempfile::tempdir().unwrap();
    let not_a_directory = root.path().join("world");
    std::fs::write(&not_a_directory, b"x").unwrap();
    let error = store_marker::write(&not_a_directory, STORE, 1).unwrap_err();
    assert!(matches!(error, MarkerError::Io { .. }), "{error}");
    assert!(
        error.to_string().contains(store_marker::MARKER_FILE),
        "{error}"
    );
}
