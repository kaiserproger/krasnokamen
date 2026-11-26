use std::io::Write;
use std::path::Path;

use super::{
    ContentSource, GAMEPLAY_DATAPACK_RELEASE, GAMEPLAY_VERSION_FILE, ImportReport, ImportRequest,
    copy_client_tag_subset, finish_import, import_content, install_version_json,
    manifest_version_entry, parse_version_metadata, publish, require_target_version,
    stage_local_jar, staging_dir, wanted_data_entry,
};
use crate::content_cache::tests::write_valid_cache;

fn write_bundle(path: &Path, version: &str) {
    let file = std::fs::File::create(path).unwrap();
    let mut archive = zip::ZipWriter::new(file);
    archive
        .start_file("version.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    write!(
        archive,
        "{{\"id\":\"{version}\",\"world_version\":{},\"protocol_version\":{}}}",
        mc_protocol::WORLD_VERSION,
        mc_protocol::PROTOCOL_VERSION
    )
    .unwrap();
    archive.finish().unwrap();
}
#[test]
fn imports_english_language_from_target_jar() {
    let dir = tempfile::tempdir().unwrap();
    let jar = dir.path().join("target.jar");
    let mut archive = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
    archive
        .start_file(
            "assets/minecraft/lang/en_us.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    write!(archive, "{{\"commands.time.set\":\"Set the time to %s\"}}").unwrap();
    archive.finish().unwrap();
    super::copy_language(&jar, dir.path()).unwrap();
    let data: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("assets/minecraft/lang/en_us.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(data["commands.time.set"], "Set the time to %s");
}

#[test]
fn manifest_selection_uses_the_pinned_version_not_latest() {
    let manifest = r#"{
        "latest": {"release": "26.4"},
        "versions": [
            {"id":"26.4","url":"https://example.invalid/newer","sha1":"newer"},
            {"id":"26.3","url":"https://example.invalid/pinned","sha1":"pinned"},
            {"id":"26.1.2","url":"https://example.invalid/gameplay","sha1":"gameplay"}
        ]
    }"#;
    let entry = manifest_version_entry(manifest, mc_protocol::TARGET_RELEASE).unwrap();
    assert_eq!(entry.url, "https://example.invalid/pinned");
    assert_eq!(entry.sha1, "pinned");
    let gameplay = manifest_version_entry(manifest, GAMEPLAY_DATAPACK_RELEASE).unwrap();
    assert_eq!(gameplay.url, "https://example.invalid/gameplay");
    assert!(manifest_version_entry(manifest, "missing").is_err());
    let metadata = parse_version_metadata(
        r#"{
        "id":"26.3",
        "downloads":{"server":{"url":"https://example.invalid/server.jar","sha1":"abc","size":42}}
    }"#,
    )
    .unwrap();
    assert_eq!(metadata.id, mc_protocol::TARGET_RELEASE);
    assert_eq!(metadata.server.unwrap().size, 42);
    for version in mc_data::protocol_versions::SUPPORTED {
        require_target_version(version.release).unwrap();
    }
    assert!(require_target_version("26.4").is_err());
}

#[tokio::test]
async fn rejected_version_does_not_create_staging_or_change_existing_cache() {
    let parent = tempfile::tempdir().unwrap();
    let cache = parent.path().join("cache");
    write_valid_cache(&cache);
    let before = std::fs::read(cache.join("version.json")).unwrap();
    let error = import_content(&ImportRequest {
        version: "26.4".into(),
        source: ContentSource::Download,
        gameplay_source: ContentSource::Download,
        cache: cache.clone(),
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("cannot derive content"));
    assert_eq!(std::fs::read(cache.join("version.json")).unwrap(), before);
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn actual_local_import_failure_preserves_complete_cache_and_discards_staging() {
    let parent = tempfile::tempdir().unwrap();
    let cache = parent.path().join("cache");
    write_valid_cache(&cache);
    std::fs::write(cache.join("operator-marker"), "keep").unwrap();
    let before = std::fs::read(cache.join("version.json")).unwrap();
    let error = import_content(&ImportRequest {
        version: mc_protocol::TARGET_RELEASE.into(),
        source: ContentSource::LocalJar(parent.path().join("missing.jar")),
        gameplay_source: ContentSource::Download,
        cache: cache.clone(),
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("supplied jar"), "{error:#}");
    assert_eq!(std::fs::read(cache.join("version.json")).unwrap(), before);
    assert_eq!(
        std::fs::read_to_string(cache.join("operator-marker")).unwrap(),
        "keep"
    );
    crate::content_cache::validate_content_cache(&cache).unwrap();
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn foreign_local_bundle_fails_before_java_or_network_and_preserves_cache() {
    let parent = tempfile::tempdir().unwrap();
    let cache = parent.path().join("cache");
    write_valid_cache(&cache);
    let before = std::fs::read(cache.join("version.json")).unwrap();
    let jar = parent.path().join("licensed.jar");
    write_bundle(&jar, "26.2");
    let error = import_content(&ImportRequest {
        version: mc_protocol::TARGET_RELEASE.into(),
        source: ContentSource::LocalJar(jar),
        gameplay_source: ContentSource::Download,
        cache: cache.clone(),
    })
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("supplied jar is 26.2"),
        "{error:#}"
    );
    assert_eq!(std::fs::read(cache.join("version.json")).unwrap(), before);
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 2);
}

#[tokio::test]
async fn foreign_gameplay_bundle_fails_before_java_or_network() {
    let parent = tempfile::tempdir().unwrap();
    let cache = parent.path().join("cache");
    write_valid_cache(&cache);
    let jar = parent.path().join("licensed.jar");
    write_bundle(&jar, mc_protocol::TARGET_RELEASE);
    let gameplay = parent.path().join("gameplay.jar");
    write_bundle(&gameplay, mc_protocol::TARGET_RELEASE);
    let error = import_content(&ImportRequest {
        version: mc_protocol::TARGET_RELEASE.into(),
        source: ContentSource::LocalJar(jar),
        gameplay_source: ContentSource::LocalJar(gameplay),
        cache: cache.clone(),
    })
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains(&format!(
            "supplied jar is {} but this cache needs {GAMEPLAY_DATAPACK_RELEASE}",
            mc_protocol::TARGET_RELEASE
        )),
        "{error:#}"
    );
}

#[test]
fn local_jar_staging_and_version_check_are_offline() {
    let parent = tempfile::tempdir().unwrap();
    let jar = parent.path().join("licensed.jar");
    write_bundle(&jar, mc_protocol::TARGET_RELEASE);
    let gameplay_jar = parent.path().join("gameplay.jar");
    write_bundle(&gameplay_jar, GAMEPLAY_DATAPACK_RELEASE);
    let staging = staging_dir(&parent.path().join("cache")).unwrap();
    let identity = staging.path().join("identity");
    let gameplay = staging.path().join("gameplay");
    std::fs::create_dir_all(&identity).unwrap();
    std::fs::create_dir_all(&gameplay).unwrap();
    let staged = stage_local_jar(&jar, &identity).unwrap();
    install_version_json(
        &staged,
        mc_protocol::TARGET_RELEASE,
        &staging.path().join("version.json"),
    )
    .unwrap();
    let error = crate::startup_data::validate_vanilla_sidecar_version(staging.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("gameplay data-pack version"), "{error}");
    let staged_gameplay = stage_local_jar(&gameplay_jar, &gameplay).unwrap();
    install_version_json(
        &staged_gameplay,
        GAMEPLAY_DATAPACK_RELEASE,
        &staging.path().join(GAMEPLAY_VERSION_FILE),
    )
    .unwrap();
    crate::startup_data::validate_vanilla_sidecar_version(staging.path()).unwrap();
    assert_eq!(std::fs::read(staged).unwrap(), std::fs::read(jar).unwrap());
}

#[test]
fn failed_validation_funnels_through_without_touching_live_cache() {
    let parent = tempfile::tempdir().unwrap();
    let cache = parent.path().join("cache");
    write_valid_cache(&cache);
    let before = std::fs::read(cache.join("version.json")).unwrap();
    let staging = staging_dir(&cache).unwrap();
    std::fs::write(staging.path().join("partial"), "half-derived").unwrap();
    let derived = super::validate_staged_cache(staging.path()).map(|()| ImportReport {
        cache: staging.path().to_path_buf(),
        registries: 0,
        entries: 0,
    });
    let error = finish_import(&cache, staging.path(), derived).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("derived vanilla content cache failed validation")
    );
    assert!(!staging.path().exists());
    assert_eq!(std::fs::read(cache.join("version.json")).unwrap(), before);
    crate::content_cache::validate_content_cache(&cache).unwrap();
}

#[test]
fn publish_replaces_only_after_derivation_and_restores_on_rename_failure() {
    let parent = tempfile::tempdir().unwrap();
    let cache = parent.path().join("cache");
    std::fs::create_dir(&cache).unwrap();
    std::fs::write(cache.join("marker"), "previous").unwrap();
    let staging = parent.path().join("staging");
    std::fs::create_dir(&staging).unwrap();
    std::fs::write(staging.join("marker"), "replacement").unwrap();
    publish(&staging, &cache).unwrap();
    assert_eq!(
        std::fs::read_to_string(cache.join("marker")).unwrap(),
        "replacement"
    );
    assert!(!staging.exists());
    assert!(!parent.path().join(".cache.previous").exists());
    let error = publish(&parent.path().join("missing-staging"), &cache).unwrap_err();
    assert!(error.to_string().contains("publish"));
    assert_eq!(
        std::fs::read_to_string(cache.join("marker")).unwrap(),
        "replacement"
    );
    assert!(!parent.path().join(".cache.previous").exists());
}

#[test]
fn staging_is_unique_and_failed_import_does_not_consume_another_import() {
    let parent = tempfile::tempdir().unwrap();
    let cache = parent.path().join("cache");
    let first = staging_dir(&cache).unwrap();
    std::fs::write(first.path().join("marker"), "first").unwrap();
    let second = staging_dir(&cache).unwrap();
    assert_ne!(first.path(), second.path());
    let _ = finish_import(
        &cache,
        second.path(),
        Err(anyhow::anyhow!("datagen failed")),
    );
    assert_eq!(
        std::fs::read_to_string(first.path().join("marker")).unwrap(),
        "first"
    );
}

#[test]
fn extraction_selects_consumed_content_and_rejects_path_traversal() {
    for relative in [
        "worldgen/structure_set/villages.json",
        "structure/village/house.nbt",
        "tags/block/mineable/pickaxe.json",
        "recipe/stick.json",
        "loot_table/blocks/stone.json",
        "dimension_type/overworld.json",
        "advancement/story/root.json",
    ] {
        assert_eq!(
            wanted_data_entry(&format!("data/minecraft/{relative}")),
            Some(relative.into())
        );
    }
    assert!(wanted_data_entry("data/minecraft/lang/en_us.json").is_none());
    assert!(wanted_data_entry("data/minecraft/tags/../../escape.json").is_none());
    assert!(wanted_data_entry("../data/minecraft/recipe/stick.json").is_none());
}

#[test]
fn identity_tags_publish_new_tags_and_merge_only_client_block_and_item_tags() {
    let source = tempfile::tempdir().unwrap();
    let jar = source.path().join("identity.jar");
    let file = std::fs::File::create(&jar).unwrap();
    let mut archive = zip::ZipWriter::new(file);
    for (name, body) in [
        (
            "data/minecraft/tags/block/deepslate_ore_replaceables.json",
            &br#"{"values":["minecraft:deepslate","minecraft:cinnabar"]}"#[..],
        ),
        (
            "data/minecraft/tags/block/mineable/pickaxe.json",
            &br#"{"values":["minecraft:cinnabar"]}"#[..],
        ),
        (
            "data/minecraft/tags/item/slabs.json",
            &br#"{"values":[]}"#[..],
        ),
        (
            "data/minecraft/tags/item/sulfur_cube_archetype/regular.json",
            &br#"{"values":["minecraft:slime_ball"]}"#[..],
        ),
        (
            "data/minecraft/tags/item/logs_that_burn.json",
            &br#"{"values":["minecraft:oak_log","minecraft:poplar_log"]}"#[..],
        ),
        (
            "data/minecraft/tags/worldgen/biome/is_overworld.json",
            &br#"{"values":[]}"#[..],
        ),
    ] {
        archive
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(body).unwrap();
    }
    archive.finish().unwrap();

    let staging = tempfile::tempdir().unwrap();
    let tags = staging.path().join("data/minecraft/tags");
    std::fs::create_dir_all(tags.join("block")).unwrap();
    std::fs::create_dir_all(tags.join("item")).unwrap();
    std::fs::write(
        tags.join("block/deepslate_ore_replaceables.json"),
        br#"{"values":["minecraft:deepslate","minecraft:tuff"]}"#,
    )
    .unwrap();
    let gameplay_logs = br#"{"values":["minecraft:oak_log"]}"#;
    std::fs::write(tags.join("item/logs_that_burn.json"), gameplay_logs).unwrap();
    copy_client_tag_subset(&jar, staging.path()).unwrap();

    let merged: serde_json::Value = serde_json::from_slice(
        &std::fs::read(tags.join("block/deepslate_ore_replaceables.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        merged["values"],
        serde_json::json!([
            "minecraft:deepslate",
            "minecraft:tuff",
            "minecraft:cinnabar"
        ]),
        "26.1.2 members stay, new 26.3 members are added"
    );
    assert!(tags.join("block/mineable/pickaxe.json").is_file());
    assert!(tags.join("item/slabs.json").is_file());
    // Tags only 26.3 defines are published in every registry; a shared
    // gameplay item tag keeps its 26.1.2 bytes.
    assert!(
        tags.join("item/sulfur_cube_archetype/regular.json")
            .is_file()
    );
    assert!(tags.join("worldgen/biome/is_overworld.json").is_file());
    assert_eq!(
        std::fs::read(tags.join("item/logs_that_burn.json")).unwrap(),
        gameplay_logs
    );
}

#[test]
fn tool_file_name_adds_the_exe_suffix_only_on_windows() {
    assert_eq!(super::tool_file_name("javac", true), "javac.exe");
    assert_eq!(super::tool_file_name("javac", false), "javac");
}

#[test]
fn classpath_round_trips_through_the_platform_separator() {
    let entries = vec![
        std::path::PathBuf::from("classes"),
        std::path::PathBuf::from("server-26.1.2.jar"),
        std::path::PathBuf::from("libs").join("netty.jar"),
    ];
    let joined = super::join_classpath(&entries).unwrap();
    let split: Vec<_> = std::env::split_paths(&joined).collect();
    assert_eq!(split, entries);
    let separator = if cfg!(windows) { ";" } else { ":" };
    assert_eq!(joined.to_string_lossy().matches(separator).count(), 2);
}

/// A child that stands in for a JVM blocked on a lock: it writes one line, then sleeps far past
/// any useful limit. `exec` makes the shell become the sleeper, so killing the child's pid kills
/// the process the wait is actually about.
#[cfg(unix)]
fn stalled_child() -> super::JvmChild {
    let mut command = std::process::Command::new("sh");
    command.arg("-c").arg("echo started; exec sleep 60");
    super::JvmChild::spawn(&mut command, "stalled child").expect("spawn the stand-in JVM")
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(unix)]
#[test]
fn a_stalled_jvm_child_is_killed_at_the_deadline() {
    // CONTENT-IMPORT-JVM-NO-TIMEOUT: the row's symptom is a child that never returns. The wait must
    // end at its own limit with the child killed, not block the import forever.
    let child = stalled_child();
    let pid = child.child.id();
    let started = std::time::Instant::now();
    let error = child
        .output(std::time::Duration::from_millis(300))
        .expect_err("a stalled child must not be waited on forever");
    let message = format!("{error:#}");
    assert!(
        message.contains("did not finish within") && message.contains("stalled child"),
        "the refusal names the child and its limit: {message}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "the wait was bounded by its own limit, not the child's 60 s"
    );
    assert!(!alive(pid), "the timed-out child was killed");
}

#[cfg(unix)]
#[test]
fn a_dropped_jvm_child_is_killed() {
    // Ctrl-C or an earlier error drops the guard: the JVM must not outlive the import.
    let child = stalled_child();
    let pid = child.child.id();
    assert!(alive(pid), "the child runs before the drop");
    drop(child);
    assert!(!alive(pid), "dropping the guard kills the child");
}

#[cfg(unix)]
#[test]
fn a_bounded_child_returns_its_output() {
    // The bounded wait keeps `Command::output`'s contract: status and both pipes of a child that
    // finishes in time, so the datagen/javac/extractor call sites still read their errors.
    let mut command = std::process::Command::new("sh");
    command.arg("-c").arg("echo out; echo err 1>&2; exit 3");
    let output = super::JvmChild::spawn(&mut command, "quick child")
        .expect("spawn")
        .output(std::time::Duration::from_secs(30))
        .expect("a child that finishes inside the limit returns");
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "out");
    assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "err");
}

/// A `java` stand-in that never finishes: the row's stalled JVM (a lock, stdin, a full disk).
#[cfg(unix)]
fn stalled_java(dir: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("stalled-java");
    std::fs::write(&path, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn a_stalled_datagen_returns_a_bounded_error() {
    // CONTENT-IMPORT-JVM-NO-TIMEOUT: `content import` waited on the bundle's datagen/javac/extractors
    // with no deadline, so a stalled JVM blocked the command forever. The production path must
    // return at its own limit; the pre-fix path never returns, which is this test's red.
    let dir = tempfile::tempdir().unwrap();
    let java = stalled_java(dir.path());
    let jar = dir.path().join("bundle.jar");
    let work = dir.path().join("work");
    let staging = dir.path().join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = super::run_datagen_bounded(
            java.as_os_str(),
            &jar,
            &work,
            &staging,
            std::time::Duration::from_millis(300),
        );
        let _ = sender.send(format!(
            "{:#}",
            result.expect_err("a stalled datagen cannot succeed")
        ));
    });
    let message = receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the import path must return instead of waiting for the JVM");
    assert!(
        message.contains("did not finish within"),
        "the refusal names the deadline: {message}"
    );
}
