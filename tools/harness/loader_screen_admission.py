"""SERVER-FABRIC-SCREEN-ADMISSION real-client check through the Fabric Loader.

The release server starts the `krasnokamen:screen` service only for admitted
loader sessions (native or Fabric). This scenario runs the real Krasnokamen
Loader (Fabric) client against the server candidate and asserts from the
server log that the screen service was started for that session with
platform Fabric. On a server build where Fabric admission is reverted the
log line is absent and the scenario fails (negative control).
"""
from __future__ import annotations

from pathlib import Path
from typing import Any

from .backends.probe_steps import ProbeSteps

SCREEN_SERVICE_LOG = "screen service started for admitted loader session"


def run(client: Any, runtime: Any, artifact_dir: Path, server_log: Path) -> dict[str, Any]:
    steps = ProbeSteps(client, artifact_dir / "screen-admission", 120.0, "loader-fabric-screen-admission")
    with steps:
        steps.wait(lambda state: state.get("in_play") and state.get("player", {}).get("on_ground"), "grounded")
        log_text = server_log.read_text(errors="replace")
        lines = [line for line in log_text.splitlines() if SCREEN_SERVICE_LOG in line]
        steps.report["screen_service_log_lines"] = lines
        steps.check("fabric_screen_service_admitted", any("Fabric" in line for line in lines))
        steps.report["visual_review"] = "server log line is the asserted surface; client reached Play"
    return steps.report
