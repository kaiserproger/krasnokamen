"""solaris-auth register and login complete on a native client (AUTH-BUDGET).

The plugin hashes passwords with Argon2id inside a host import, which takes far more
than the old 100 ms (four 25 ms epochs) wall-clock allowance of one callback; the
host then retired the plugin and the locked player never authenticated. The real
`solaris-auth` package runs behind the native login gate: ordinary chat is refused
until `/register` (and, after a reconnect, `/login`) unlocks the session, so a chat
echo that arrives proves the unlock.
"""
from __future__ import annotations

import contextlib
import os
import subprocess
import sys
import uuid
from collections.abc import Iterator
from typing import Any

from .probe_steps import ProbeSteps

SCENARIO = "native-v2-auth-budget"
PASSWORD = "hunter2-pass"
REQUIRED_TOOLS = frozenset({
    "minecraft_observe", "minecraft_wait_for_state_change", "minecraft_send_chat",
    "minecraft_wait_for_play", "minecraft_connect", "minecraft_disconnect",
})


HOGS = 7


@contextlib.contextmanager
def contended_plugin_worker(server_pid: int) -> Iterator[int]:
    """Deschedule the plugin worker the way a loaded host does: pin it to one core shared with CPU hogs.

    Argon2 then takes several times its idle wall time (the hogs hold the core), far beyond the
    retired 100 ms allowance, while the work stays the same; no sleeps, the contention lasts
    exactly as long as the block.
    """
    worker = next((int(tid) for tid in os.listdir(f"/proc/{server_pid}/task")
                   if open(f"/proc/{server_pid}/task/{tid}/comm").read().strip() == "mc-plugin-host"), None)
    if worker is None:
        raise RuntimeError("server has no mc-plugin-host worker thread")
    before = os.sched_getaffinity(worker)
    core = max(before)
    os.sched_setaffinity(worker, {core})
    hogs = [subprocess.Popen([sys.executable, "-c", "while True: pass"],
                             preexec_fn=lambda: os.sched_setaffinity(0, {core})) for _ in range(HOGS)]
    try:
        yield core
    finally:
        for hog in hogs:
            hog.kill()
        for hog in hogs:
            hog.wait()
        os.sched_setaffinity(worker, before)


def _unlocked(steps: ProbeSteps, label: str) -> dict[str, Any]:
    """After the plugin's reply, ordinary chat (refused while locked) is echoed back."""
    marker = f"auth echo {uuid.uuid4().hex}"
    steps.call("minecraft_send_chat", {"message": marker, "command": False})
    return steps.wait(lambda state: marker in "\n".join(state["recent_chat"]), label)


def run(client: Any, artifact: Any, timeout: float, address: str, server_pid: int) -> dict[str, Any]:
    with ProbeSteps(client, artifact / SCENARIO, timeout, SCENARIO) as steps:
        first = steps.observe()
        steps.report["player_uuid"] = first["player"]["entity_uuid"]
        steps.report["join_chat"] = first["recent_chat"]
        steps.report["fixture_commands"].append("register <password> <password>")
        with contended_plugin_worker(server_pid) as core:
            steps.report["contended_core"] = core
            steps.call("minecraft_send_chat", {"message": f"register {PASSWORD} {PASSWORD}", "command": True})
            steps.wait(lambda state: any("Successfully registered" in line for line in state["recent_chat"]),
                       "register_reply_under_contention")
        _unlocked(steps, "register_unlocks_the_session")
        steps.call("minecraft_disconnect")
        steps.check("client_left_play", steps.call("minecraft_observe")["in_play"] is False)
        steps.call("minecraft_connect", {"server_addr": address})
        steps.check("relogin_entered_play", steps.call("minecraft_wait_for_play", {
            "timeout_seconds": steps.remaining()})["in_play"] is True)
        steps.report["fixture_commands"].append("login <password>")
        with contended_plugin_worker(server_pid):
            steps.call("minecraft_send_chat", {"message": f"login {PASSWORD}", "command": True})
            steps.wait(lambda state: any("Successfully logged in" in line for line in state["recent_chat"]),
                       "login_reply_under_contention")
        after = _unlocked(steps, "login_unlocks_the_session")
        steps.check("same_account", after["player"]["entity_uuid"] == steps.report["player_uuid"])
        steps.report["passed_chat"] = after["recent_chat"]
    return steps.report
