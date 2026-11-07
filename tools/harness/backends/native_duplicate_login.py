"""Vanilla duplicate login: the newer connection of one account replaces the older.

A second native client with the same identity joins while the first is in play. The
older session is kicked, the account's inventory moves to the new session exactly once,
and the first client then logs back in to replace the second (the same rule both ways).
"""
from __future__ import annotations

from typing import Any

from .native_v2 import _echo, _give
from .probe_steps import ProbeSteps, item_count


def _left_play(client: Any, steps: ProbeSteps, label: str) -> dict[str, Any]:
    """Block on state changes until the client has left Play (bounded by the scenario deadline)."""
    while True:
        state = client.call_tool("minecraft_observe")
        if state.get("in_play") is not True:
            steps.check(label, True)
            return state
        client.call_tool("minecraft_wait_for_state_change", {
            "observed_version": state["state_version"], "timeout_seconds": steps.remaining()})


def _peer_diamonds(peer: Any, steps: ProbeSteps, label: str, count: int) -> dict[str, Any]:
    play = peer.call_tool("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    steps.check(label + "_entered_play", play.get("in_play") is True)
    state = peer.call_tool("minecraft_observe")
    steps.check(label + "_same_account", state["player"]["entity_uuid"] == steps.report["player_uuid"])
    steps.check(label + "_inventory_moved_not_copied", item_count(state, "minecraft:diamond") == count)
    return state


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    start = getattr(steps.client, "start_second_login", None)
    if not callable(start):
        raise RuntimeError("duplicate login requires the second-login client hook")
    first = steps.client
    address = steps.client.server_address
    _give(steps, "minecraft:diamond")
    steps.check("first_session_holds_one_diamond", item_count(steps.observe(), "minecraft:diamond") == 1)
    second = start()
    _peer_diamonds(second, steps, "second_login", 1)
    kicked = _left_play(first, steps, "first_session_replaced")
    steps.report["first_session_after_second_login"] = {
        key: kicked.get(key) for key in ("in_play", "disconnect_reason", "disconnected", "state_version")}
    # The other direction: the first client logs in again and replaces the second session.
    first.call_tool("minecraft_connect", {"server_addr": address})
    play = first.call_tool("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    steps.check("first_client_relogin_entered_play", play.get("in_play") is True)
    _left_play(second, steps, "second_session_replaced_by_relogin")
    back = steps.observe()
    steps.check("relogin_same_account", back["player"]["entity_uuid"] == steps.report["player_uuid"])
    steps.check("relogin_inventory_moved_not_copied", item_count(back, "minecraft:diamond") == 1)
    _echo(steps, "relogin_ordered_chat")

