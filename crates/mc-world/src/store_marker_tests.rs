//! Books R4-P1: the independent checkpoint marker, on real files.
use super::store_marker::{self, MarkerError, StoreMarker};

const STORE: [u8; 16] = [0x11; 16];
const OTHER: [u8; 16] = [0x22; 16];
const BRANCH: [u8; 8] = [0x33; 8];

#[test]
fn encode_decode_round_trip_and_checksum() {
    let marker = StoreMarker::new(STORE, store_marker::ROOT_BRANCH, 1234);
    let bytes = marker.encode();
    assert_eq!(StoreMarker::decode(&bytes).unwrap(), marker);

    let mut damaged = bytes.clone();
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
    let mut wrong_version = bytes.clone();
    wrong_version[7] = 9;
    assert_eq!(
        StoreMarker::decode(&wrong_version).unwrap_err().to_string(),
        "wrong magic or format version"
    );
}

/// A world as created or imported keeps the 64-byte root layout: magic,
/// store id, big-endian tick, SHA-256 of those 32 bytes. Only a restored copy
/// writes the branched layout, which carries the branch after the store id.
#[test]
fn root_and_restored_branches_have_one_layout_each() {
    let root = StoreMarker::new(STORE, store_marker::ROOT_BRANCH, 1234).encode();
    let mut expected = b"KCMARK\x01\x00".to_vec();
    expected.extend_from_slice(&STORE);
    expected.extend_from_slice(&1234_u64.to_be_bytes());
    let checksum = <sha2::Sha256 as sha2::Digest>::digest(&expected);
    expected.extend_from_slice(&checksum);
    assert_eq!(root, expected, "the root layout is the format-1 record");

    let restored = StoreMarker::new(STORE, BRANCH, 1234);
    let bytes = restored.encode();
    assert_eq!(bytes.len(), 72);
    assert_eq!(&bytes[..8], b"KCMARK\x02\x00");
    assert_eq!(&bytes[8..24], &STORE);
    assert_eq!(&bytes[24..32], &BRANCH);
    assert_eq!(StoreMarker::decode(&bytes).unwrap(), restored);

    // The root branch has exactly one encoding: format 2 naming it is corrupt.
    let mut root_in_format_two = b"KCMARK\x02\x00".to_vec();
    root_in_format_two.extend_from_slice(&STORE);
    root_in_format_two.extend_from_slice(&store_marker::ROOT_BRANCH);
    root_in_format_two.extend_from_slice(&1234_u64.to_be_bytes());
    let checksum = <sha2::Sha256 as sha2::Digest>::digest(&root_in_format_two);
    root_in_format_two.extend_from_slice(&checksum);
    assert_eq!(
        StoreMarker::decode(&root_in_format_two)
            .unwrap_err()
            .to_string(),
        "format 2 names the root branch"
    );
}

#[test]
fn a_minted_branch_is_never_the_root_or_the_previous_one() {
    let first = store_marker::mint_branch(store_marker::ROOT_BRANCH);
    assert_ne!(first, store_marker::ROOT_BRANCH);
    let second = store_marker::mint_branch(first);
    assert_ne!(second, store_marker::ROOT_BRANCH);
    assert_ne!(second, first);
}

/// The branch is the only thing that tells two restores of one backup apart, so
/// every one of its 64 bits is random: no bit position keeps one value across
/// mints (a UUIDv4 prefix would pin the version nibble and leave 60).
#[test]
fn a_minted_branch_uses_all_64_bits() {
    let (mut ones, mut zeros) = (0_u64, 0_u64);
    for _ in 0..256 {
        let branch = u64::from_be_bytes(store_marker::mint_branch(store_marker::ROOT_BRANCH));
        ones |= branch;
        zeros |= !branch;
    }
    assert_eq!((ones, zeros), (u64::MAX, u64::MAX), "a bit never varied");
}

#[test]
fn write_is_durable_and_readable() {
    let root = tempfile::tempdir().unwrap();
    store_marker::write(root.path(), STORE, store_marker::ROOT_BRANCH, 7).unwrap();
    assert_eq!(
        store_marker::read(root.path()).unwrap(),
        Some(StoreMarker::new(STORE, store_marker::ROOT_BRANCH, 7))
    );
    // No temporary file is left behind.
    assert!(
        !root
            .path()
            .join(format!("{}.tmp", store_marker::MARKER_FILE))
            .exists()
    );
    // A second write replaces the marker.
    store_marker::write(root.path(), STORE, store_marker::ROOT_BRANCH, 9).unwrap();
    assert_eq!(
        store_marker::read(root.path()).unwrap(),
        Some(StoreMarker::new(STORE, store_marker::ROOT_BRANCH, 9))
    );
}

#[test]
fn corrupt_marker_is_refused() {
    let root = tempfile::tempdir().unwrap();
    store_marker::write(root.path(), STORE, store_marker::ROOT_BRANCH, 3).unwrap();
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
    store_marker::write(root.path(), OTHER, store_marker::ROOT_BRANCH, 4).unwrap();
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
    store_marker::write(root.path(), STORE, store_marker::ROOT_BRANCH, 40).unwrap();
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
    store_marker::write(root.path(), STORE, store_marker::ROOT_BRANCH, 5).unwrap();
    store_marker::check(root.path(), STORE, Some(9)).unwrap();
    assert_eq!(
        store_marker::read(root.path()).unwrap(),
        Some(StoreMarker::new(STORE, store_marker::ROOT_BRANCH, 9))
    );
}

/// The crash-window rewrite keeps the copy on its branch: falling back to the
/// root would let the original world's tail replay into a restored copy.
#[test]
fn the_crash_window_rewrite_keeps_the_branch() {
    let root = tempfile::tempdir().unwrap();
    store_marker::write(root.path(), STORE, BRANCH, 5).unwrap();
    assert_eq!(store_marker::branch(root.path()).unwrap(), BRANCH);
    store_marker::check(root.path(), STORE, Some(9)).unwrap();
    assert_eq!(
        store_marker::read(root.path()).unwrap(),
        Some(StoreMarker::new(STORE, BRANCH, 9))
    );
    let fresh = tempfile::tempdir().unwrap();
    assert_eq!(
        store_marker::branch(fresh.path()).unwrap(),
        store_marker::ROOT_BRANCH,
        "a store before its first marker is the root branch"
    );
}

#[test]
fn write_failure_reports_the_path() {
    // A regular file where the world directory should be: the temporary marker
    // cannot be created, and the error names the marker path.
    let root = tempfile::tempdir().unwrap();
    let not_a_directory = root.path().join("world");
    std::fs::write(&not_a_directory, b"x").unwrap();
    let error =
        store_marker::write(&not_a_directory, STORE, store_marker::ROOT_BRANCH, 1).unwrap_err();
    assert!(matches!(error, MarkerError::Io { .. }), "{error}");
    assert!(
        error.to_string().contains(store_marker::MARKER_FILE),
        "{error}"
    );
}
