use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

const M94_MANIFEST: &str =
    include_str!("../../../docs/real-client-regression/manifests/m94-regression-pack.json");

#[test]
fn playable_real_client_runner_check_selects_gradle_adapter() {
    let repo_root = repo_root();

    let output = Command::new("python3")
        .arg("-m")
        .arg("tools.harness")
        .arg("run")
        .arg("playable")
        .arg("--check")
        .current_dir(&repo_root)
        .output()
        .expect("run playable real-client check");

    assert!(
        output.status.success(),
        "playable runner did not auto-select the repo-native runClient adapter\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("profile=playable"),
        "playable runner check should report its harness profile\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn real_client_runner_run_fails_fast_without_graphical_display() {
    let repo_root = repo_root();
    let output = Command::new("bash")
        .arg(repo_root.join("tools/harness/backends/regression.sh"))
        .arg("--run")
        .env(
            "KRASNOKAMEN_REAL_CLIENT_MANIFEST",
            repo_root.join("docs/real-client-regression/manifests/m94-regression-pack.json"),
        )
        .env(
            "KRASNOKAMEN_REAL_CLIENT_AGENT_SCENARIO",
            "m94-02b-rejected-block-resync",
        )
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("run real-client gate without graphical display");

    assert!(
        !output.status.success(),
        "real-client --run unexpectedly succeeded without a graphical display"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--run requires a graphical display; set DISPLAY or WAYLAND_DISPLAY"),
        "runner must fail on the missing graphical prerequisite before launch\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("running real-client regression into"),
        "missing DISPLAY must be rejected before a run directory/server/client launch\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn bucket_resync_debug_loop_preflight_uses_declared_real_client_scenario() {
    let repo_root = repo_root();
    let output = Command::new("bash")
        .arg(repo_root.join("tools/harness/backends/bucket_resync.sh"))
        .arg("--check")
        .output()
        .expect("run bucket-resync debug-loop preflight");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("gradle-runclient"));
    assert!(stdout.contains("scenario policy: debug commands allowed by manifest"));
}

#[test]
fn core_replay_real_client_prepare_copies_canonical_manifest() {
    let repo_root = repo_root();
    let run_root = tempfile::tempdir().expect("create core replay run root");

    let output = Command::new("python3")
        .arg("-m")
        .arg("tools.harness")
        .arg("run")
        .arg("replay")
        .arg("--prepare")
        .current_dir(&repo_root)
        .env("KRASNOKAMEN_REAL_CLIENT_RUN_ROOT", run_root.path())
        .env(
            "KRASNOKAMEN_REAL_CLIENT_AGENT_SECRET",
            "core-replay-test-secret",
        )
        .output()
        .expect("prepare core replay real-client gate");
    assert!(
        output.status.success(),
        "core replay prepare failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let prepared_dir = prepared_run_dir(run_root.path());
    let copied = std::fs::read(prepared_dir.join("core-replay-manifest.json"))
        .expect("copied core replay manifest exists");
    let canonical =
        std::fs::read(repo_root.join("tools/core-replay-scenarios/core-actions-seed-81.json"))
            .expect("canonical core replay manifest exists");
    assert_eq!(copied, canonical);
    let automation = std::fs::read_to_string(prepared_dir.join("automation-driver.txt"))
        .expect("read prepared automation metadata");
    assert!(automation.contains("core_replay_manifest=core-replay-manifest.json"));
    assert!(automation.contains("client_adapter_task=:fabric-agent:runClientAgent"));
}

#[test]
fn playable_real_client_prepare_uses_playable_manifest_config_and_scenario() {
    let repo_root = repo_root();
    let run_root = tempfile::tempdir().expect("create playable run root");

    let output = Command::new("python3")
        .arg("-m")
        .arg("tools.harness")
        .arg("run")
        .arg("playable")
        .arg("--prepare")
        .current_dir(&repo_root)
        .env("KRASNOKAMEN_REAL_CLIENT_RUN_ROOT", run_root.path())
        .output()
        .expect("prepare playable real-client gate");

    assert!(
        output.status.success(),
        "playable prepare failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run_dir = prepared_run_dir(run_root.path());
    let automation_driver = std::fs::read_to_string(run_dir.join("automation-driver.txt"))
        .expect("read automation driver");
    assert!(
        automation_driver.contains("playable.toml"),
        "prepared playable gate must use playable.toml\n{automation_driver}"
    );
    assert!(
        automation_driver.contains("client_agent_scenario=playable-04-twenty-minute-survival-loop"),
        "prepared playable gate must default to the full loop scenario\n{automation_driver}"
    );
    let operators: Vec<Value> = serde_json::from_slice(
        &std::fs::read(run_dir.join("ops.json")).expect("read prepared operator list"),
    )
    .expect("parse prepared operator list");
    assert!(
        operators.is_empty(),
        "the twenty-minute acceptance gate must remain no-operator"
    );
    assert!(
        automation_driver
            .contains("client_gate=agent-run real-client configured: gradle-runclient")
            && automation_driver.contains("client_kind=gradle-runclient")
            && automation_driver.contains("client_adapter_source=auto-gradle-runclient")
            && automation_driver.contains("client_adapter_task=:fabric-agent:runClientAgent")
            && !automation_driver
                .lines()
                .any(|line| line.contains(&legacy_primary_launcher_metadata_key())),
        "prepared playable gate must auto-select the Gradle runClient adapter\n{automation_driver}"
    );

    let manifest: Value = serde_json::from_slice(
        &std::fs::read(run_dir.join("manifest.json")).expect("read prepared playable manifest"),
    )
    .expect("parse prepared playable manifest");
    assert_eq!(manifest["pack_id"], "playable-real-client-loop");
    assert_eq!(manifest["quality_label"], "playable-spike");
}

#[test]
fn playable_real_client_prepare_ignores_legacy_client_command_env() {
    let repo_root = repo_root();
    let run_root = tempfile::tempdir().expect("create playable run root");
    let legacy_command = "echo legacy-client-command-must-not-run";
    let legacy_command_env = legacy_primary_client_env_names()
        .into_iter()
        .next()
        .expect("legacy primary client command env is listed");

    let output = Command::new("python3")
        .arg("-m")
        .arg("tools.harness")
        .arg("run")
        .arg("playable")
        .arg("--prepare")
        .current_dir(&repo_root)
        .env("KRASNOKAMEN_REAL_CLIENT_RUN_ROOT", run_root.path())
        .env(&legacy_command_env, legacy_command)
        .output()
        .expect("prepare playable real-client gate with legacy command env");

    assert!(
        output.status.success(),
        "playable prepare rejected an ignored legacy command env\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run_dir = prepared_run_dir(run_root.path());
    let automation_driver = std::fs::read_to_string(run_dir.join("automation-driver.txt"))
        .expect("read automation driver");

    assert!(
        automation_driver.contains("client_kind=gradle-runclient")
            && automation_driver.contains("client_adapter_source=auto-gradle-runclient")
            && automation_driver.contains("client_adapter_task=:fabric-agent:runClientAgent")
            && !automation_driver.contains(&legacy_command_env)
            && !automation_driver.contains(legacy_command)
            && !automation_driver
                .lines()
                .any(|line| line.contains(&legacy_primary_launcher_metadata_key())),
        "legacy client command env must not affect the fixed Gradle runClient adapter\n{automation_driver}"
    );
}

#[test]
fn playable_real_client_prepare_picks_free_agent_port_when_default_is_busy() {
    let repo_root = repo_root();
    let run_root = tempfile::tempdir().expect("create playable run root");
    let _busy_default = std::net::TcpListener::bind(("127.0.0.1", 39094)).ok();

    let output = Command::new("python3")
        .arg("-m")
        .arg("tools.harness")
        .arg("run")
        .arg("playable")
        .arg("--prepare")
        .current_dir(&repo_root)
        .env("KRASNOKAMEN_REAL_CLIENT_RUN_ROOT", run_root.path())
        .env("KRASNOKAMEN_REAL_CLIENT_AGENT_SECRET", "s_test_prepare")
        .output()
        .expect("prepare playable real-client gate");

    assert!(
        output.status.success(),
        "playable prepare with agent secret failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run_dir = prepared_run_dir(run_root.path());
    let automation_driver = std::fs::read_to_string(run_dir.join("automation-driver.txt"))
        .expect("read automation driver");
    let port = automation_driver
        .lines()
        .find_map(|line| line.strip_prefix("client_agent_port="))
        .expect("client agent port is recorded")
        .parse::<u16>()
        .expect("client agent port is numeric");
    assert_ne!(
        port, 39094,
        "runner must avoid the occupied historical default port\n{automation_driver}"
    );
    assert!(
        automation_driver.contains(&format!(
            "client_agent_bridge_url=http://127.0.0.1:{port}/rpc"
        )),
        "runner must record a bridge URL matching the selected port\n{automation_driver}"
    );
}

#[test]
fn real_client_prepare_auto_selects_secondary_gradle_runclient_adapter() {
    let repo_root = repo_root();
    let runner_path = repo_root.join("tools/harness/backends/regression.sh");
    let run_root = tempfile::tempdir().expect("create real-client run root");

    let output = Command::new("bash")
        .arg(&runner_path)
        .arg("--prepare")
        .env("KRASNOKAMEN_REAL_CLIENT_RUN_ROOT", run_root.path())
        .env("KRASNOKAMEN_REAL_CLIENT_AGENT_SECRET", "s_primary_prepare")
        .env(
            "KRASNOKAMEN_REAL_CLIENT_SECOND_AGENT_SECRET",
            "s_secondary_prepare",
        )
        .output()
        .expect("prepare two-client real-client gate");

    assert!(
        output.status.success(),
        "two-client prepare failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run_dir_text = String::from_utf8(output.stdout).expect("run dir stdout is utf-8");
    let run_dir = Path::new(run_dir_text.trim());
    let automation_driver = std::fs::read_to_string(run_dir.join("automation-driver.txt"))
        .expect("read automation driver");
    assert!(
        automation_driver.contains("second_client_kind=gradle-runclient")
            && automation_driver.contains("second_client_adapter_source=auto-gradle-runclient")
            && automation_driver
                .contains("second_client_adapter_task=:fabric-agent:runClientAgent")
            && automation_driver.contains("client_username=KrasnoPrimary")
            && automation_driver.contains("second_client_username=KrasnoSecondary")
            && automation_driver.contains("client_game_dir=")
            && automation_driver.contains("/clients/primary")
            && automation_driver.contains("second_client_game_dir=")
            && automation_driver.contains("/clients/secondary")
            && !automation_driver
                .lines()
                .any(|line| line.contains(&legacy_primary_launcher_metadata_key()))
            && automation_driver.contains("second_client_agent_secret=SET_REDACTED"),
        "prepared two-client gate must auto-select the secondary Gradle runClient adapter and isolate client state\n{automation_driver}"
    );
    let primary_port = automation_driver
        .lines()
        .find_map(|line| line.strip_prefix("client_agent_port="))
        .expect("primary port recorded");
    let secondary_port = automation_driver
        .lines()
        .find_map(|line| line.strip_prefix("second_client_agent_port="))
        .expect("secondary port recorded");
    assert_ne!(
        primary_port, secondary_port,
        "primary and secondary bridge ports must differ\n{automation_driver}"
    );
}

#[test]
fn gradle_runclient_adapter_is_the_default_real_client_launcher() {
    let repo_root = repo_root();
    let output = Command::new("python3")
        .arg("-m")
        .arg("tools.harness")
        .arg("run")
        .arg("playable")
        .arg("--check")
        .current_dir(&repo_root)
        .output()
        .expect("run playable real-client check");

    assert!(
        output.status.success(),
        "playable runner failed to auto-select Gradle runClient\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // The Gradle adapter source lives in the sibling krasnokamen-loader repo
    // (KRASNOKAMEN_LOADER_ROOT, default ../solaris-loader). This test pins only
    // the observable harness contract above, never the loader's build text.
}

#[test]
fn p46_prepare_never_reuses_a_colliding_run_directory() {
    let repo_root = repo_root();
    let run_root = tempfile::tempdir().expect("create real-client run root");
    let command_dir = tempfile::tempdir().expect("create fake command directory");
    let date_path = command_dir.path().join("date");
    std::fs::write(
        &date_path,
        "#!/usr/bin/env bash\nprintf '20260718T000000Z\\n'\n",
    )
    .expect("write fake date command");
    let mut permissions = std::fs::metadata(&date_path)
        .expect("read fake date command metadata")
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o755);
    }
    std::fs::set_permissions(&date_path, permissions).expect("make fake date command executable");
    let path = format!(
        "{}:{}",
        command_dir.path().display(),
        std::env::var("PATH").expect("PATH is set")
    );

    let prepare = || {
        Command::new("bash")
            .arg(repo_root.join("tools/harness/backends/regression.sh"))
            .arg("--prepare")
            .env("PATH", &path)
            .env("KRASNOKAMEN_REAL_CLIENT_RUN_ROOT", run_root.path())
            .env(
                "KRASNOKAMEN_REAL_CLIENT_AGENT_SCENARIO",
                "playable-46-generated-ruin-cache",
            )
            .output()
            .expect("prepare P46 real-client run")
    };

    let first = prepare();
    assert!(
        first.status.success(),
        "first P46 prepare failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let first_dir = PathBuf::from(
        String::from_utf8(first.stdout)
            .expect("first run dir is utf-8")
            .trim(),
    );
    std::fs::create_dir(first_dir.join("world")).expect("create sentinel P46 world");
    std::fs::write(first_dir.join("world/sentinel"), "must not be reused")
        .expect("write sentinel P46 world state");

    let second = prepare();
    assert!(
        second.status.success(),
        "second P46 prepare failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let second_dir = PathBuf::from(
        String::from_utf8(second.stdout)
            .expect("second run dir is utf-8")
            .trim(),
    );
    assert_ne!(
        first_dir, second_dir,
        "P46 must create a new run directory rather than reusing a timestamp collision"
    );
    assert!(
        !second_dir.join("world/sentinel").exists(),
        "P46 must never reuse a preexisting world after a run-directory collision"
    );
}

#[test]
fn validate_run_accepts_subsecond_tick_budget_warnings() {
    let repo_root = repo_root();
    let run_dir = tempfile::tempdir().expect("create run dir");
    write_validate_run_artifacts(
        run_dir.path(),
        json!({
            "schema": "krasnokamen.real_client_observations.v1",
            "client_gate": "agent-run-real-client",
            "quality_label": "stabilization",
            "result": "passed",
            "scenarios": [{
                "id": "m94-01-join-rejoin-chunks-movement",
                "result": "passed",
                "screenshots": ["screenshots/m94-01-join-rejoin-chunks-movement.png"]
            }]
        }),
    );
    std::fs::write(
        run_dir
            .path()
            .join("screenshots/m94-01-join-rejoin-chunks-movement.png"),
        valid_png_320x180(),
    )
    .expect("write valid screenshot");
    std::fs::write(
        run_dir.path().join("server.log"),
        "WARN mc_core::runtime: runtime tick exceeded performance budget tick_us=412302\n",
    )
    .expect("write subsecond tick warning");

    let output = validate_run(&repo_root, run_dir.path());

    assert!(
        output.status.success(),
        "functional real-client evidence must not fail on a subsecond budget warning\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn validate_run_rejects_half_second_tick_stall() {
    let repo_root = repo_root();
    let run_dir = tempfile::tempdir().expect("create run dir");
    write_validate_run_artifacts(
        run_dir.path(),
        json!({
            "schema": "krasnokamen.real_client_observations.v1",
            "client_gate": "agent-run-real-client",
            "quality_label": "stabilization",
            "result": "passed",
            "scenarios": [{
                "id": "m94-01-join-rejoin-chunks-movement",
                "result": "passed",
                "screenshots": ["screenshots/m94-01-join-rejoin-chunks-movement.png"]
            }]
        }),
    );
    std::fs::write(
        run_dir
            .path()
            .join("screenshots/m94-01-join-rejoin-chunks-movement.png"),
        valid_png_320x180(),
    )
    .expect("write valid screenshot");
    std::fs::write(
        run_dir.path().join("server.log"),
        "WARN mc_core::runtime: runtime tick exceeded performance budget tick_us=500000\n",
    )
    .expect("write catastrophic tick warning");

    let output = validate_run(&repo_root, run_dir.path());

    assert!(
        !output.status.success(),
        "real-client validator accepted a half-second server stall\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("catastrophic runtime tick"),
        "validator must name the catastrophic tick threshold\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Resolve the single run directory a `--prepare` invocation created under
/// an isolated `KRASNOKAMEN_REAL_CLIENT_RUN_ROOT`. The harness CLI tees backend
/// output into its own artifact log, so callers read the run root instead
/// of parsing a printed run path from stdout.
fn prepared_run_dir(run_root: &Path) -> std::path::PathBuf {
    let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(run_root)
        .expect("read prepared run root")
        .map(|entry| entry.expect("read prepared run entry").path())
        .filter(|path| path.is_dir())
        .collect();
    entries.sort();
    assert_eq!(
        entries.len(),
        1,
        "prepare must create exactly one run directory under {}",
        run_root.display()
    );
    entries.pop().expect("prepared run directory exists")
}

fn legacy_primary_client_env_names() -> Vec<String> {
    vec![
        ["KRASNOKAMEN", "REAL", "CLIENT", "COMMAND"].join("_"),
        ["M94", "CLIENT", "COMMAND"].join("_"),
        ["KRASNOKAMEN", "REAL", "CLIENT", "KIND"].join("_"),
    ]
}

fn legacy_primary_launcher_metadata_key() -> String {
    ["client", "command"].join("_")
}

fn write_validate_run_artifacts(run_dir: &Path, observations: Value) {
    write_validate_run_artifacts_with_manifest(run_dir, M94_MANIFEST, observations);
}

fn write_validate_run_artifacts_with_manifest(run_dir: &Path, manifest: &str, observations: Value) {
    std::fs::write(run_dir.join("manifest.json"), manifest).expect("write manifest");
    for artifact in ["client.log", "server.log", "git.txt", "toolchain.txt"] {
        std::fs::write(run_dir.join(artifact), "").expect("write artifact");
    }
    std::fs::write(
        run_dir.join("automation-driver.txt"),
        gradle_automation_driver_fixture(),
    )
    .expect("write automation driver");
    std::fs::create_dir(run_dir.join("screenshots")).expect("create screenshots dir");
    std::fs::write(
        run_dir.join("observations.json"),
        serde_json::to_vec_pretty(&observations).expect("serialize observations"),
    )
    .expect("write observations");
}

fn gradle_automation_driver_fixture() -> &'static str {
    "client_gate=agent-run real-client configured: gradle-runclient\n\
client_kind=gradle-runclient\n\
client_adapter_source=auto-gradle-runclient\n\
client_adapter_task=:fabric-agent:runClientAgent\n\
client_agent_runtime_kind_primary=compiled-classes\n\
client_agent_runtime_path_primary=/tmp/solaris-client-agent/fabric-agent/build/classes/java/main\n\
client_agent_runtime_sha256_primary=0000000000000000000000000000000000000000000000000000000000000000\n\
client_agent_runtime_file_count_primary=1\n\
client_agent_runtime_validation_primary=verified\n\
client_agent_bridge_wait_status_primary=ready\n\
client_agent_phase_exit_status_m94-01-join-rejoin-chunks-movement=0\n\
client_agent_driver_exit_status=0\n"
}

fn validate_run(repo_root: &Path, run_dir: &Path) -> std::process::Output {
    Command::new("bash")
        .arg(repo_root.join("tools/harness/backends/regression.sh"))
        .arg("--validate-run")
        .arg(run_dir)
        .output()
        .expect("run real-client validator")
}

fn valid_png_320x180() -> Vec<u8> {
    valid_png_rgba(320, 180)
}

fn valid_png_rgba(width: u32, height: u32) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    append_png_chunk(&mut png, b"IHDR", &ihdr);

    let row_len = 1 + (width as usize * 4);
    let mut raw = Vec::with_capacity(row_len * height as usize);
    for _ in 0..height {
        raw.push(0);
        raw.resize(raw.len() + width as usize * 4, 0);
    }

    let mut zlib = Vec::new();
    zlib.extend_from_slice(&[0x78, 0x01]);
    for (index, chunk) in raw.chunks(65_535).enumerate() {
        zlib.push(if index == (raw.len() - 1) / 65_535 {
            0x01
        } else {
            0x00
        });
        let len = chunk.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(chunk);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
    append_png_chunk(&mut png, b"IDAT", &zlib);
    append_png_chunk(&mut png, b"IEND", &[]);
    png
}

fn append_png_chunk(png: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(chunk_type);
    png.extend_from_slice(data);
    let mut crc_data = Vec::with_capacity(chunk_type.len() + data.len());
    crc_data.extend_from_slice(chunk_type);
    crc_data.extend_from_slice(data);
    png.extend_from_slice(&crc32(&crc_data).to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    const MOD_ADLER: u32 = 65_521;
    let mut a = 1u32;
    let mut b = 0u32;
    for byte in data {
        a = (a + u32::from(*byte)) % MOD_ADLER;
        b = (b + a) % MOD_ADLER;
    }
    (b << 16) | a
}
