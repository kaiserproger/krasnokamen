use super::*;

fn valid_identity() -> serde_json::Value {
    serde_json::json!({
        "version": IDENTITY_SCHEMA,
        "digest": "a".repeat(64),
        "sources": {"vanilla cache": "b".repeat(64)},
        // Historical ownership remains valid after its pack has been removed.
        "structure_owners": {"mod:old_start": "file/removed-pack"}
    })
}

fn replaced(field: &str, value: serde_json::Value) -> String {
    let mut saved = valid_identity();
    saved[field] = value;
    saved.to_string()
}

fn assert_rejected(saved: &str) {
    let dir = tempfile::tempdir().unwrap();
    let mut packs = PackSet::load(&dir.path().join("datapacks"), Selection::default()).unwrap();
    let inputs = packs.prepare_worldgen(dir.path(), None).unwrap();
    let mut globals = BTreeMap::from([(
        "level_metadata".into(),
        crate::native_checkpoint::tag_record(0, &mc_nbt::Tag::Compound(vec![])).unwrap(),
    )]);
    crate::terrain::stage_data_field(&mut globals, IDENTITY, saved).unwrap();
    let epoch = "0123456789abcdef0123456789abcdef";
    crate::terrain::stage_epoch(&mut globals, epoch).unwrap();
    let error = inputs
        .stage_identity(&mut globals)
        .err()
        .expect("corrupt identity must refuse, not become a pack change");
    assert!(
        error.to_string().contains("datapack worldgen identity"),
        "{error}"
    );
    assert_eq!(
        crate::terrain::read_data_field(&globals, IDENTITY)
            .unwrap()
            .as_deref(),
        Some(saved)
    );
    assert_eq!(
        crate::terrain::read_data_field(&globals, "KrasnokamenTerrainEpoch")
            .unwrap()
            .as_deref(),
        Some(epoch)
    );
}

#[test]
fn identity_rejects_unsupported_or_missing_schema() {
    for value in [
        serde_json::json!(0),
        serde_json::json!(2),
        serde_json::Value::Null,
    ] {
        assert_rejected(&replaced("version", value));
    }
    let mut saved = valid_identity();
    saved.as_object_mut().unwrap().remove("version");
    assert_rejected(&saved.to_string());
}

#[test]
fn identity_rejects_malformed_sha256_and_source_hashes() {
    for digest in [
        "".to_owned(),
        "a".repeat(63),
        "A".repeat(64),
        "g".repeat(64),
    ] {
        assert_rejected(&replaced("digest", serde_json::json!(digest)));
    }
    assert_rejected(&replaced(
        "sources",
        serde_json::json!({"vanilla cache":"bad"}),
    ));
}

#[test]
fn identity_rejects_unknown_or_invalid_source_and_ownership_state() {
    for (field, value) in [
        ("sources", serde_json::json!({})),
        (
            "sources",
            serde_json::json!({"vanilla cache":"b".repeat(64),"unknown-source":"c".repeat(64)}),
        ),
        (
            "structure_owners",
            serde_json::json!({"bad id":"file/removed-pack"}),
        ),
        (
            "structure_owners",
            serde_json::json!({"unqualified":"file/removed-pack"}),
        ),
        (
            "structure_owners",
            serde_json::json!({"mod:old_start":"vanilla cache"}),
        ),
        (
            "structure_owners",
            serde_json::json!({"mod:old_start":"file/../outside"}),
        ),
    ] {
        assert_rejected(&replaced(field, value));
    }
}

#[test]
fn identity_rejects_unknown_fields_duplicate_fields_and_trailing_state() {
    assert_rejected(&replaced(
        "future_state",
        serde_json::json!({"allow_recovery":true}),
    ));
    let valid = valid_identity().to_string();
    assert_rejected(&valid.replacen("{", "{\"version\":1,", 1));
    assert_rejected(&format!("{valid} trailing-state"));
}

#[test]
fn identity_rejects_oversized_saved_record() {
    assert_rejected(&format!(
        "{}{}",
        valid_identity(),
        " ".repeat(MAX_IDENTITY_BYTES)
    ));
}

#[test]
fn identity_schema_roundtrip_keeps_removed_pack_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut packs = PackSet::load(&dir.path().join("datapacks"), Selection::default()).unwrap();
    let inputs = packs.prepare_worldgen(dir.path(), None).unwrap();
    let mut globals = BTreeMap::from([(
        "level_metadata".into(),
        crate::native_checkpoint::tag_record(0, &mc_nbt::Tag::Compound(vec![])).unwrap(),
    )]);
    crate::terrain::stage_data_field(&mut globals, IDENTITY, &valid_identity().to_string())
        .unwrap();
    let adoption = inputs.stage_identity(&mut globals).unwrap();
    assert!(
        adoption.changed,
        "valid SHA changes stay diagnostic, not locked"
    );
    assert_eq!(
        adoption.catalog.owners["mod:old_start"],
        "file/removed-pack"
    );
    let persisted = crate::terrain::read_data_field(&globals, IDENTITY)
        .unwrap()
        .unwrap();
    let persisted = Identity::decode(&persisted).unwrap();
    assert_eq!(persisted.version, IDENTITY_SCHEMA);
    assert_eq!(persisted.digest, inputs.identity);
    assert_eq!(
        persisted.structure_owners["mod:old_start"],
        "file/removed-pack"
    );
    assert!(!inputs.stage_identity(&mut globals).unwrap().changed);
}

#[test]
fn identity_rejects_pack_ids_forbidden_by_selection() {
    for pack in [
        "file/a\\b".to_owned(),
        "file/a\0b".to_owned(),
        format!("file/{}", "x".repeat(1024)),
    ] {
        assert_rejected(&replaced(
            "sources",
            serde_json::json!({"vanilla cache":"b".repeat(64),pack.clone():"c".repeat(64)}),
        ));
        assert_rejected(&replaced(
            "structure_owners",
            serde_json::json!({"mod:old_start":pack}),
        ));
    }
}

#[test]
fn identity_rejects_duplicate_source_map_keys() {
    assert_rejected(&format!(
        r#"{{"version":1,"digest":"{}","sources":{{"vanilla cache":"{}","vanilla cache":"{}"}},"structure_owners":{{}}}}"#,
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64)
    ));
}

#[test]
fn identity_rejects_duplicate_historical_owner_map_keys() {
    assert_rejected(&format!(
        r#"{{"version":1,"digest":"{}","sources":{{"vanilla cache":"{}"}},"structure_owners":{{"mod:old_start":"file/old-a","mod:old_start":"file/old-b"}}}}"#,
        "a".repeat(64),
        "b".repeat(64)
    ));
}
