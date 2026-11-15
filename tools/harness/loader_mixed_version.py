"""Keep Fabric 26.3 live while Fabric/SVC 26.1.2 builds and starts."""
from __future__ import annotations

import contextlib
import hashlib
import json
import os
import secrets
from pathlib import Path
from typing import Any

from . import loader
from .java_launcher import gradle_project_cache
from .mcp import McpClient


def run_candidate(ctx: Any, timeout: float) -> dict[str, Any]:
    from .backends.screens_voice_qa import java_endpoint

    with java_endpoint("screens"):
        result = loader.run(
            "fabric", timeout, ctx.artifact_dir, {"run": run},
            server_bin=ctx.server_bin, plugins_dir=ctx.plugins_dir,
            client_ready_timeout_seconds=None,
        )
    return {"scenario": result}


def class_directory(runtime: Any, version: str, game: Path) -> Path:
    root = runtime.sibling_root("loader")
    # Resolve the output selected by this launch, including the shared output
    # used by the fix-only negative control, not an old isolated build's files.
    if (game / "client-init.gradle").is_file():
        cache = gradle_project_cache(root, ":loader-fabric:runClientMcp")
        output = cache.parent / "build" / version / "loader-fabric"
    else:
        output = root / "loader-fabric/build"
    return output / "classes/java/main"

def class_hashes(directory: Path) -> dict[str, str]:
    return {str(path.relative_to(directory)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(directory.rglob("*.class"))}



def run(client: Any, runtime: Any, artifact_dir: Path, server_log: Path) -> dict[str, Any]:
    report: dict[str, Any] = {"passed": False, "versions": ["26.3", "26.1.2"], "assertions": {}}
    first_classes = class_directory(runtime, "26.3", artifact_dir / "game")
    report_path = artifact_dir / "mixed-version.json"
    first_before = runtime.current_client_state(client, 30)
    if not first_before.get("in_play"):
        raise AssertionError("Fabric 26.3 must be live before the second build")
    first_hashes = class_hashes(first_classes)
    if not first_hashes:
        raise AssertionError(f"Fabric 26.3 class tree is empty: {first_classes}")
    report["first_before_second_build"] = {"in_play": True, "classes": str(first_classes),
                                            "hashes": first_hashes}
    second_root = artifact_dir / "fabric-26.1.2"
    second_root.mkdir()
    previous = os.environ.get("KRASNOKAMEN_JAVA_VERSION")
    try:
        os.environ["KRASNOKAMEN_JAVA_VERSION"] = "26.1.2"
        with contextlib.ExitStack() as stack:
            xvfb, display, xlog = runtime.start_xvfb(second_root)
            stack.callback(xlog.close)
            stack.callback(runtime.stop_process, xvfb)
            token, port = secrets.token_urlsafe(32), runtime.reserve_port()
            process, log = runtime.start_client(
                game_dir=second_root / "game", log_path=second_root / "client.log",
                token=token, mcp_port=port, username="MixedVersion", display=display,
                platform="fabric", client_profile="svc", ready_timeout_seconds=None,
            )
            stack.callback(log.close)
            stack.callback(runtime.stop_process, process)
            second = McpClient(f"http://127.0.0.1:{port}/mcp", token, 120)
            stack.callback(second.close)
            second.initialize()
            runtime.wait_client_ready_for_connect(second, 120)
            second_classes = class_directory(runtime, "26.1.2", second_root / "game")
            report["second_after_build"] = {"classes": str(second_classes), "ready": True,
                                             "hashes": class_hashes(second_classes)}
            if process.poll() is not None:
                raise AssertionError("Fabric 26.1.2 exited after its build")
            after = runtime.current_client_state(client, 30)
            report["assertions"]["first_stays_in_play"] = bool(after.get("in_play"))
            after_hashes = class_hashes(first_classes)
            report["first_after_second_build"] = {"hashes": after_hashes}
            report["assertions"]["first_classes_preserved"] = after_hashes == first_hashes
            report["assertions"]["second_classes_present"] = bool(report["second_after_build"]["hashes"])
            loader.capture_ui(client, artifact_dir, "first-after-second-build")
            # The second endpoint is deliberately at TitleScreen, with no world ticks.
            second.call_tool("minecraft_screenshot", {
                "path": str(artifact_dir / "screenshots/second-ready.png"),
            })
            failed = [name for name, value in report["assertions"].items() if not value]
            if failed:
                raise AssertionError(f"mixed-version Loader build overlap failed: {failed}")
            report["passed"] = True
            return report
    finally:
        if previous is None:
            os.environ.pop("KRASNOKAMEN_JAVA_VERSION", None)
        else:
            os.environ["KRASNOKAMEN_JAVA_VERSION"] = previous
        report_path.write_text(json.dumps(report, indent=2) + "\n")
