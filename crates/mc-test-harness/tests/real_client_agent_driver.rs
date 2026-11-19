use std::process::Command;

#[test]
fn real_client_gate_rejects_runs_without_gradle_runtime_classes_provenance() {
    let repo_root = repo_root();
    let run_dir = tempfile::tempdir().expect("create run dir");

    std::fs::copy(
        repo_root.join("docs/real-client-regression/manifests/m94-regression-pack.json"),
        run_dir.path().join("manifest.json"),
    )
    .expect("copy real-client manifest");
    std::fs::create_dir(run_dir.path().join("screenshots")).expect("create screenshots dir");
    for artifact in ["client.log", "server.log", "git.txt", "toolchain.txt"] {
        std::fs::write(run_dir.path().join(artifact), "").expect("write required artifact");
    }
    std::fs::write(
        run_dir.path().join("observations.json"),
        r#"{
  "schema": "krasnokamen.real_client_observations.v1",
  "client_gate": "agent-run-real-client",
  "quality_label": "stabilization",
  "result": "passed",
  "scenarios": [{
    "id": "m94-02b-rejected-block-resync",
    "result": "passed",
    "screenshots": ["screenshots/m94-02b-rejected-block-resync.png"]
  }]
}"#,
    )
    .expect("write completed observations");
    std::fs::write(
        run_dir.path().join("automation-driver.txt"),
        "client_kind=gradle-runclient\n\
client_adapter_source=auto-gradle-runclient\n\
client_adapter_task=:fabric-agent:runClientAgent\n\
client_agent_driver_exit_status=0\n\
client_agent_phase_exit_status_m94-02b-rejected-block-resync=0\n\
client_agent_bridge_wait_status_primary=ready\n",
    )
    .expect("write pre-provenance automation driver");

    let output = Command::new("bash")
        .arg(repo_root.join("tools/harness/backends/regression.sh"))
        .arg("--validate-run")
        .arg(run_dir.path())
        .output()
        .expect("run real-client validator");

    assert!(
        !output.status.success(),
        "validator accepted a run without loaded runtime provenance\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("runtime classes provenance"),
        "validator must reject missing Gradle runtime provenance before unrelated artifacts\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn interactive_pause_close_waits_for_the_client_thread_response() {
    let repo_root = repo_root();
    let driver = repo_root.join("tools/harness/backends/driver.py");
    let probe = r#"
import importlib.util
import pathlib
import sys

driver_path = pathlib.Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("krasnokamen_real_client_driver", driver_path)
driver = importlib.util.module_from_spec(spec)
sys.modules["krasnokamen_real_client_driver"] = driver
spec.loader.exec_module(driver)
client_path = driver_path.parent / "driver_client.py"
client_spec = importlib.util.spec_from_file_location("krasnokamen_real_client_client", client_path)
client_mod = importlib.util.module_from_spec(client_spec)
sys.modules["krasnokamen_real_client_client"] = client_mod
client_spec.loader.exec_module(client_mod)
driver.time.monotonic = lambda: 0.0
calls = []

def call_and_record(client, transcript, command, payload, timeout_seconds, actor=None):
    calls.append((command, timeout_seconds))
    if command == "close_screen":
        if timeout_seconds < 10.0:
            raise RuntimeError("simulated busy Minecraft client thread")
        return {"status": "ok"}
    if command == "state":
        return {
            "state_version": 2,
            "in_play": True,
            "current_screen": "none",
        }
    raise AssertionError(f"unexpected command {command}")

client_mod.call_and_record = call_and_record
result = client_mod.wait_for_interactive_play(
    object(),
    [],
    {
        "state_version": 1,
        "in_play": True,
        "current_screen": "net.minecraft.client.gui.screens.PauseScreen",
    },
    30.0,
)

assert result["current_screen"] == "none", result
assert calls[0][0] == "close_screen", calls
assert calls[0][1] >= 10.0, calls
"#;

    let output = Command::new("python3")
        .arg("-c")
        .arg(probe)
        .arg(driver)
        .output()
        .expect("run interactive pause-close probe");

    assert!(
        output.status.success(),
        "pause-close probe failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
