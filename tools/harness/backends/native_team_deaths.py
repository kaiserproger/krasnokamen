"""Player death messages as two windowed native clients see them.

The victim is the actor client, in team `red`; the peer is a second windowed client in team `blue`.
Each case kills the actor and reads the rendered chat lines of both clients (the client renders
the received component with its own language file), so a wrong damage identity, a plain English
text instead of the translation component, or a team visibility that is ignored changes what a
client shows.
"""
from __future__ import annotations

import uuid
from typing import Any

from . import native_focus
from .native_p1 import _capture
from .native_v2 import _echo
from .native_p4 import _disable_gamerule
from .probe_steps import ProbeSteps

VICTIM = "HarnessEntities"
PEER = "DeathPeer"

# (team deathMessageVisibility, damage type, death text, victim's client shows it, peer's client shows it)
VISIBILITY_CASES = (
    ("always", "cramming", "was squished too much", True, True),
    ("never", "freeze", "froze to death", False, False),
    ("hideForOtherTeams", "cactus", "was pricked to death", True, False),
    ("hideForOwnTeam", "wither", "withered away", False, True),
)


def _chat(state: dict[str, Any]) -> list[str]:
    return [line for line in state["recent_chat"] if isinstance(line, str)]


def _shows(state: dict[str, Any], text: str) -> bool:
    return any(line.endswith(f"{VICTIM} {text}") for line in _chat(state))


def _flush(steps: ProbeSteps, peer: ProbeSteps, label: str) -> tuple[dict[str, Any], dict[str, Any]]:
    """An ordered chat marker from the respawned victim reaches both clients after every death line."""
    marker = f"team deaths {label} {uuid.uuid4().hex}"
    steps.call("minecraft_send_chat", {"message": marker, "command": False})
    victim_state = steps.wait(lambda state: marker in "\n".join(_chat(state)), f"{label}_victim_marker")
    peer_state = peer.wait(lambda state: marker in "\n".join(_chat(state)), f"{label}_peer_marker")
    return victim_state, peer_state


def _die(steps: ProbeSteps, peer: ProbeSteps, display: str, actor_pid: int, label: str, command: str,
         text: str, victim_sees: bool, peer_sees: bool) -> None:
    steps.fixture(command)
    death = steps.wait(lambda state: state["player"]["health"] == 0 and state["screen"]["open"],
                       f"{label}_death_screen_received")
    steps.report.setdefault("death_screens", {})[label] = death["screen"]
    steps.check(f"{label}_death_screen_class", death["screen"]["class"].endswith("DeathScreen"))
    if label == "kill":
        _capture(steps, "team-deaths-kill-death-screen")
    native_focus.acquire(display, actor_pid, steps.remaining())
    steps.call("minecraft_respawn", {"timeout_seconds": steps.remaining()})
    steps.wait(lambda state: state["player"]["health"] > 0 and not state["screen"]["open"],
               f"{label}_respawn_received", wait_for_play=True)
    victim_state, peer_state = _flush(steps, peer, label)
    rows = steps.report.setdefault("death_chat", {})
    rows[label] = {"command": command, "expect": f"{VICTIM} {text}", "victim_chat": _chat(victim_state),
                   "peer_chat": _chat(peer_state)}
    steps.check(f"{label}_victim_client_chat_{'shows' if victim_sees else 'hides'}_death",
                _shows(victim_state, text) is victim_sees)
    steps.check(f"{label}_peer_client_chat_{'shows' if peer_sees else 'hides'}_death",
                _shows(peer_state, text) is peer_sees)


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    mcp, display, actor_pid, peer_pid = steps.client.death_peer
    x, y, z = origin
    steps.fixture("gamemode survival @s")
    _disable_gamerule(steps, "spawn_mobs")
    with ProbeSteps(mcp, steps.artifact_dir / "peer", steps.remaining(), "native-v2-team-deaths-peer") as peer:
        for command in ("team add red", "team add blue", f"team join red {VICTIM}", f"team join blue {PEER}"):
            steps.fixture(command)
        _echo(steps, "teams_ready")
        # KILL-TYPE: player /kill is generic_kill ("was killed"), not out_of_world.
        _die(steps, peer, display, actor_pid, "kill", f"kill {VICTIM}", "was killed", True, True)
        # DEATH-COMPONENT: a mob kill names the mob, which only the vanilla translation carries.
        steps.fixture(f"summon minecraft:zombie {x + .5} {y} {z + 2.5} {{NoAI:1b,PersistenceRequired:1b}}")
        steps.call("minecraft_wait_ticks", {"ticks": 10})
        _die(steps, peer, display, actor_pid, "mob",
             f"damage {VICTIM} 100 minecraft:mob_attack by @e[type=minecraft:zombie,limit=1]",
             "was slain by Zombie", True, True)
        steps.fixture("kill @e[type=minecraft:zombie]")
        # TEAM-DEATH-VISIBILITY: every deathMessageVisibility value filters the broadcast.
        for visibility, damage_type, text, victim_sees, peer_sees in VISIBILITY_CASES:
            steps.fixture(f"team modify red deathMessageVisibility {visibility}")
            _echo(steps, f"{visibility}_set")
            _die(steps, peer, display, actor_pid, visibility,
                 f"damage {VICTIM} 100 minecraft:{damage_type}", text, victim_sees, peer_sees)
        steps.report["observer"] = peer.report
    native_focus.acquire(display, actor_pid, steps.remaining())
    steps.report["limits"] = [
        "Chat lines are the rendered text of the received components; the death screen is asserted by class "
        "and a retained capture, its message text is not exposed by the client observation.",
        "One victim, one same-team-excluded peer; no teammate peer, spectator or dimension-global roster case.",
    ]
