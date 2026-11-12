"""Vanilla command parity on the real client: command-block source name and admin broadcast,
translated global-command failures, /defaultgamemode for a fresh client, and /tellraw with a bare string.

Every check is a bounded wait recorded under its own name in the report, and the scenario keeps going
after a miss so each behaviour is an independent red/green verdict; at the end the first miss fails as
`<scenario> assertion failed: <name>`.
"""
from __future__ import annotations

import uuid
from typing import Any

from .native_v2 import _echo
from .probe_steps import ProbeSteps

PEER = "CmdRowsLate"
WAIT_SECONDS = 15


def _lines(state: dict[str, Any]) -> list[str]:
    return [str(line) for line in state["recent_chat"]]


def _record(steps: ProbeSteps, name: str, ok: bool) -> bool:
    steps.report["assertions"][name] = bool(ok)
    if not ok:
        steps.report.setdefault("failed_checks", []).append(name)
    return ok


def _seen(steps: ProbeSteps, needle: str, name: str, *, absent: str | None = None) -> bool:
    """Wait (bounded) for a chat line containing `needle`; record the verdict under `name`.

    `absent` additionally records `<name>_not_raw_key`: no line may contain that text (a raw lang key).
    """
    try:
        steps.wait(lambda s: any(needle in line for line in _lines(s)), name, timeout_seconds=WAIT_SECONDS)
    except TimeoutError:
        pass
    lines = _lines(steps.observe())
    found = _record(steps, name, any(needle in line for line in lines))
    if absent is not None:
        _record(steps, name + "_not_raw_key", not any(absent in line for line in lines))
    return found


def _absent(steps: ProbeSteps, name: str, lines: list[str], text: str, *, prefix: str = "") -> None:
    _record(steps, name, not any(line.startswith(prefix) and text in line for line in lines))


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    start_peer = getattr(steps.client, "start_peer", None)
    if not callable(start_peer):
        raise RuntimeError("command rows require the peer client hook")
    x, y, z = origin
    token = uuid.uuid4().hex[:8]
    steps.fixture("gamemode creative @s")
    steps.fixture("gamerule send_command_feedback true")
    steps.fixture("gamerule command_block_output true")

    # --- tellraw with a bare JSON string -----------------------------------------------------
    steps.fixture(f'tellraw @s "TRSTR{token}"')
    _seen(steps, f"TRSTR{token}", "tellraw_plain_string_arrives")

    # --- command block placed by /setblock with auto:1b --------------------------------------
    steps.fixture(f'setblock {x + 3} {y} {z} minecraft:command_block{{auto:1b,Command:"say CBSAY{token}"}}')
    _seen(steps, f"[@] CBSAY{token}", "setblock_auto_command_block_runs_and_says_at")
    _absent(steps, "command_block_say_is_not_named_server", _lines(steps.observe()), f"CBSAY{token}",
            prefix="[Server]")
    steps.fixture(f'setblock {x + 3} {y + 1} {z} minecraft:command_block{{auto:1b,Command:"time set 4100"}}')
    _seen(steps, "[@: Set", "command_block_output_reaches_operator_as_admin_line")

    # --- commandBlockOutput off: the block still runs, the operator hears nothing -------------
    steps.fixture("gamerule command_block_output false")
    steps.fixture(f'setblock {x + 4} {y} {z} minecraft:command_block{{auto:1b,Command:"time set 4200"}}')
    steps.fixture(f'setblock {x + 4} {y + 1} {z} minecraft:command_block{{auto:1b,Command:"say CBOFF{token}"}}')
    _seen(steps, f"[@] CBOFF{token}", "output_off_command_block_still_runs")
    ordered = _echo(steps, "output_off_ordered")
    _absent(steps, "output_off_sends_no_admin_line", _lines(ordered), "4200", prefix="[@: ")
    steps.fixture("gamerule command_block_output true")

    # --- failing global commands show translated text ----------------------------------------
    steps.fixture("kill @e[type=minecraft:cow,distance=..0]")
    _seen(steps, "No entity was found", "kill_without_match_shows_translated_text",
          absent="argument.entity.notfound")
    steps.fixture("tp @s NobodyHere")
    _echo(steps, "tp_failure_ordered")
    _absent(steps, "tp_to_unknown_name_shows_no_raw_key", _lines(steps.observe()), "argument.entity.notfound")

    # --- /defaultgamemode, then a fresh client -----------------------------------------------
    steps.fixture("defaultgamemode creative")
    _seen(steps, "The default game mode is now", "defaultgamemode_reports_vanilla_feedback")
    peer = start_peer(PEER)
    play = peer.call_tool("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    _record(steps, "fresh_client_entered_play", play.get("in_play") is True)
    steps.fixture(f"execute if entity @a[name={PEER},gamemode=creative] run say PEERCREATIVE{token}")
    _seen(steps, f"PEERCREATIVE{token}", "fresh_client_joins_in_the_default_game_mode")

    failed = steps.report.get("failed_checks", [])
    if failed:
        steps.check(failed[0], False)
