"""Vanilla `/defaultgamemode` persistence and `/loot` block tables on the real client.

A fresh non-operator client must join in the mode `/defaultgamemode creative` set *before a server restart*
(vanilla `WorldData.GameType`), and `/loot give @s loot minecraft:blocks/stone` must give cobblestone.
Checks are recorded under their own names and the scenario keeps going after a miss; at the end the first
miss fails it as `<scenario> assertion failed: <name>`.
"""
from __future__ import annotations

import uuid
from typing import Any

from .native_command_rows import _lines, _record, _seen
from .native_v2 import _echo
from .probe_steps import ProbeSteps, item_count

LATE_PEER = "CmdPersistLate"


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    start_peer = getattr(steps.client, "start_peer", None)
    restart = getattr(steps.client, "p4_restart_server", None)
    if not callable(start_peer) or not callable(restart):
        raise RuntimeError("command persistence requires the peer and restart hooks")
    token = uuid.uuid4().hex[:8]
    steps.fixture("gamemode creative @s")

    # --- /loot with a block table -------------------------------------------------------------
    before = item_count(steps.observe(), "minecraft:cobblestone")
    steps.fixture("loot give @s loot minecraft:blocks/stone")
    try:
        steps.wait(lambda s: item_count(s, "minecraft:cobblestone") > before, "loot_block_table_gives_cobblestone",
                   timeout_seconds=15)
    except TimeoutError:
        pass
    state = steps.observe()
    _record(steps, "loot_block_table_gives_cobblestone", item_count(state, "minecraft:cobblestone") > before)
    _record(steps, "loot_block_table_is_not_reported_unsupported",
            not any("Unknown or unsupported loot table" in line for line in _lines(state)))

    # --- /defaultgamemode, then a clean restart ----------------------------------------------
    steps.fixture("defaultgamemode creative")
    _seen(steps, "The default game mode is now", "defaultgamemode_reports_vanilla_feedback")
    _echo(steps, "defaultgamemode_ordered")
    steps.call("minecraft_disconnect")
    address = restart(steps.remaining())
    steps.call("minecraft_connect", {"server_addr": address})
    play = steps.call("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    _record(steps, "operator_rejoined_after_restart", play.get("in_play") is True)

    # --- a fresh client after the restart joins in the persisted default ----------------------
    peer = start_peer(LATE_PEER)
    play = peer.call_tool("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    _record(steps, "fresh_client_entered_play_after_restart", play.get("in_play") is True)
    steps.fixture(f"execute if entity @a[name={LATE_PEER},gamemode=creative] run say PERSISTCREATIVE{token}")
    _seen(steps, f"PERSISTCREATIVE{token}", "fresh_client_joins_in_the_persisted_default_game_mode")

    failed = steps.report.get("failed_checks", [])
    if failed:
        steps.check(failed[0], False)
