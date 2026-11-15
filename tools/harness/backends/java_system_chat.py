"""Real loader packet journal to Java matrix snapshot regression."""
from __future__ import annotations

import json
from pathlib import Path
import secrets

from ..java_matrix_client import JavaMatrixClient
from .probe_steps import ProbeSteps


def check(probe: ProbeSteps, artifact: Path) -> None:
    marker = "matrix-system-chat-" + secrets.token_hex(8)
    probe.fixture("say " + marker)
    # GUI receipt is an independent fence: removing journal emission must
    # fail the assertion below, not time out waiting for the missing journal.
    probe.wait(lambda state: any(marker in message for message in state.get("recent_chat", [])),
               "system_chat_packet_received")
    client = JavaMatrixClient("SceneShot", probe.client, probe.client.scene_address, artifact,
                              timeout_seconds=probe.remaining())
    snapshot = client.snapshot()
    (artifact / "system-chat-snapshot.json").write_text(json.dumps(snapshot, indent=2) + "\n")
    probe.check("system_chat_producer_reaches_matrix_events",
                any(marker in message for message in snapshot.get("events") or []))
