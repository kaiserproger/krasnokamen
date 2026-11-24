"""Vanilla duplicate login: the newer connection of one account replaces the older.

A second native client with the same identity joins while the first is in play. The
older session is kicked, the account's inventory moves to the new session exactly once,
and the first client then logs back in to replace the second (the same rule both ways).
"""
from __future__ import annotations

import os
import time
from typing import Any

from .native_v2 import _click, _echo, _give, _open, _point, _set
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


def _kicked(client: Any, steps: ProbeSteps, label: str, timeout_seconds: float) -> dict[str, Any]:
    """The state of a client a duplicate login replaced: out of Play with the session's own error.

    `in_play` alone is not the observation: a world reload also leaves Play for a moment, and a
    transient would both misread the reason and stand as a false control. The kicked session ends in
    the client's `error` phase, which is what the row's assertion reads."""
    deadline = time.monotonic() + timeout_seconds
    while True:
        state = client.call_tool("minecraft_observe")
        phase = (state.get("loading_readiness") or {}).get("phase")
        if state.get("in_play") is not True and phase == "error":
            steps.check(label, True)
            return state
        if time.monotonic() >= deadline:
            steps.report["kick_wait_state"] = {
                "in_play": state.get("in_play"), "phase": phase,
                "disconnect_reason": state.get("disconnect_reason")}
            steps.check(label, False)
        client.call_tool("minecraft_wait_for_state_change", {
            "observed_version": state["state_version"],
            "timeout_seconds": max(1.0, min(deadline - time.monotonic(), steps.remaining()))})


def _peer_diamonds(peer: Any, steps: ProbeSteps, label: str, count: int) -> dict[str, Any]:
    play = peer.call_tool("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    steps.check(label + "_entered_play", play.get("in_play") is True)
    state = peer.call_tool("minecraft_observe")
    steps.check(label + "_same_account", state["player"]["entity_uuid"] == steps.report["player_uuid"])
    steps.check(label + "_inventory_moved_not_copied", item_count(state, "minecraft:diamond") == count)
    return state


def _ground(client: Any, item: str) -> int:
    rows = client.call_tool("minecraft_list_entities", {"radius": 8, "limit": 128})["entities"]
    return sum(row.get("item", {}).get("count", 0) for row in rows if row.get("item", {}).get("item_id") == item)


def _held(state: dict[str, Any], item: str) -> int:
    carried = state["container"]["carried"]
    return item_count(state, item) + (carried["count"] if carried.get("item_id") == item else 0)


def _chat_wait(client: Any, steps: ProbeSteps, text: str) -> bool:
    while True:
        seen = client.call_tool("minecraft_observe")
        if any(text in line for line in seen.get("recent_chat", []) if isinstance(line, str)):
            return True
        if seen.get("in_play") is not True:
            return False
        client.call_tool("minecraft_wait_for_state_change", {
            "observed_version": seen["state_version"], "timeout_seconds": min(steps.remaining(), 60)})


def _auth_attack(steps: ProbeSteps, first: Any, start: Any, address: str,
                 record: dict[str, Any], diamond: str) -> None:
    """Duplicate login behind krasnokamen-auth (require_plugin_login): verified incumbent, then two unverified claimants."""
    second = start()
    try:
        second.call_tool("minecraft_wait_for_play", {"timeout_seconds": 30})
    except Exception as error:  # noqa: BLE001 - refusal is an expected branch; recorded
        record["second_wait_error"] = str(error)[:300]
    seen = second.call_tool("minecraft_observe")
    record["second_vs_verified"] = {"in_play": seen.get("in_play"), "disconnect_reason": seen.get("disconnect_reason")}
    seen = first.call_tool("minecraft_observe")
    record["first_after_second"] = {"in_play": seen.get("in_play"), "held": _held(seen, diamond)}
    steps.check("verified_incumbent_keeps_session_and_item", seen.get("in_play") is True and _held(seen, diamond) == 1)
    first.call_tool("minecraft_disconnect")
    first.call_tool("minecraft_connect", {"server_addr": address})
    first.call_tool("minecraft_wait_for_play", {"timeout_seconds": 60})
    third = start("third-login")
    try:
        third.call_tool("minecraft_wait_for_play", {"timeout_seconds": 60})
    except Exception as error:  # noqa: BLE001
        record["third_wait_error"] = str(error)[:300]
    states = {name: client.call_tool("minecraft_observe") for name, client in (("first", first), ("third", third))}
    record["both_unverified"] = {name: {"in_play": s.get("in_play"), "disconnect_reason": s.get("disconnect_reason")}
                                 for name, s in states.items()}
    if states["third"].get("in_play"):
        third.call_tool("minecraft_send_chat", {"message": "login hunter2-pass", "command": True})
        record["third_logged_in"] = _chat_wait(third, steps, "Successfully logged in")
    if first.call_tool("minecraft_observe").get("in_play"):
        first.call_tool("minecraft_send_chat", {"message": "login hunter2-pass", "command": True})
        record["first_logged_in"] = _chat_wait(first, steps, "Successfully logged in")
    live = [c for c in (first, third) if c.call_tool("minecraft_observe").get("in_play")]
    if live:
        live[0].call_tool("minecraft_wait_ticks", {"ticks": 60})
    final = {}
    for name, client in (("first", first), ("third", third)):
        s = client.call_tool("minecraft_observe")
        final[name] = {"in_play": s.get("in_play"), "disconnect_reason": s.get("disconnect_reason"),
                       "held": _held(s, diamond) if s.get("in_play") else None,
                       "ground": _ground(client, diamond) if s.get("in_play") else None}
    record["final"] = final
    winners = [name for name, data in final.items() if data["in_play"]]
    steps.check("auth_at_most_one_live_session", len(winners) <= 1)
    steps.check("auth_item_conserved", not winners or final[winners[0]]["held"] + final[winners[0]]["ground"] == 1)


def _snapshot(client: Any, diamond: str) -> dict[str, Any]:
    state = client.call_tool("minecraft_observe")
    out: dict[str, Any] = {"in_play": state.get("in_play"), "disconnect_reason": state.get("disconnect_reason"),
                           "screen": state.get("screen", {}).get("class")}
    if state.get("in_play") is True:
        rows = client.call_tool("minecraft_list_entities", {"radius": 64, "limit": 128})["entities"]
        out.update(uuid=state["player"]["entity_uuid"], held=_held(state, diamond),
                   ground=sum(row.get("item", {}).get("count", 0) for row in rows
                              if row.get("item", {}).get("item_id") == diamond),
                   other_players=[{key: row.get(key) for key in ("entity_uuid", "x", "y", "z")} for row in rows
                                  if row.get("entity_type") == "minecraft:player" and not row.get("local_player")])
    return out


def _try_play(client: Any, seconds: float) -> Any:
    try:
        return client.call_tool("minecraft_wait_for_play", {"timeout_seconds": seconds}).get("in_play")
    except Exception as error:  # noqa: BLE001 - a refused or kicked client is a recorded outcome
        return str(error)[:300]


def _hold_attack(steps: ProbeSteps, first: Any, start: Any, address: str, record: dict[str, Any],
                 diamond: str, variant: str) -> None:
    """Second login while the other session of the account is held before (config) or just after
    (chunk) its tick Join by a protocol proxy; reverse: the newcomer is the held one."""
    import time

    from .break_hold_proxy import HoldProxy

    log = steps.client.server_log_path
    record["server_log_offset"] = log.stat().st_size
    clients = {"first": first}
    if variant in ("holdconfig", "holdchunk", "holdconfigdrop"):
        gate = HoldProxy(address, "gate")
        held = HoldProxy(address, "chunk" if variant == "holdchunk" else "config")
        record["proxy_events"] = {"gate_second": gate.events, "held_first": held.events}
        clients["second"] = start("second-login", gate.address)
        steps.check("second_client_waits_at_the_gate", gate.login_seen.wait(120))
        first.call_tool("minecraft_disconnect")
        first.call_tool("minecraft_connect", {"server_addr": held.address})
        steps.check("first_client_held", held.held.wait(60))
        record["held_at"] = time.time()
        gate.release.set()
        record["second_play_while_first_held"] = _try_play(clients["second"], 60)
        if variant == "holdconfigdrop":
            # The newer session moves the item (drops it) while the older one waits in configuration:
            # the older one's later join must see the moved item, not the image it logged in with.
            clients["second"].call_tool("minecraft_set_hotbar_slot", {"slot": 0})
            clients["second"].call_tool("minecraft_drop_selected_item", {"item_id": diamond, "count": 1})
            clients["second"].call_tool("minecraft_wait_ticks", {"ticks": 10})
        if os.environ.get("BREAK_DL_HOLD_TICKS"):
            # Hold length against the server's 10 s configuration read deadline (server ticks, 20/s).
            left = int(os.environ["BREAK_DL_HOLD_TICKS"])
            while left > 0:
                clients["second"].call_tool("minecraft_wait_ticks", {"ticks": min(left, 100)})
                left -= 100
        record["during_hold"] = {name: _snapshot(client, diamond) for name, client in clients.items()}
        record["released_at"] = time.time()
        held.release.set()
        record["first_after_release_wait"] = _try_play(first, 30)
    else:
        held = HoldProxy(address, "config")
        record["proxy_events"] = {"held_second": held.events}
        first.call_tool("minecraft_disconnect")
        clients["second"] = start("second-login", held.address)
        steps.check("second_client_held", held.held.wait(120))
        record["held_at"] = time.time()
        first.call_tool("minecraft_connect", {"server_addr": address})
        record["first_play_while_second_held"] = _try_play(first, 60)
        record["during_hold"] = {name: _snapshot(client, diamond) for name, client in clients.items()}
        record["released_at"] = time.time()
        held.release.set()
        record["second_after_release_wait"] = _try_play(clients["second"], 30)
    live = [client for client in clients.values() if client.call_tool("minecraft_observe").get("in_play") is True]
    if live:
        live[0].call_tool("minecraft_wait_ticks", {"ticks": 100})
    after = {name: _snapshot(client, diamond) for name, client in clients.items()}
    record["after_release"] = after
    record["server_log"] = [line[:260] for line in log.read_text(errors="replace")[record["server_log_offset"]:].splitlines()
                            if "HarnessEntities" in line or "admitted" in line or "refused" in line]
    winners = [name for name, data in after.items() if data["in_play"] is True]
    address = steps.client.p4_restart_server(steps.remaining())
    reader = clients[winners[0]] if len(winners) == 1 else first
    reader.call_tool("minecraft_connect", {"server_addr": address})
    record["restart_reader"] = winners[0] if len(winners) == 1 else "first"
    record["restart_play"] = _try_play(reader, 60)
    reader.call_tool("minecraft_wait_ticks", {"ticks": 40})
    record["after_restart"] = _snapshot(reader, diamond)
    if reader is not first:
        first.call_tool("minecraft_connect", {"server_addr": address})
        record["first_final_play"] = _try_play(first, 60)
    steps.call("minecraft_send_chat", {"message": "list uuids", "command": True})
    echoed = _echo(steps, "final_list_uuids")
    record["final_list"] = [line for line in echoed["recent_chat"] if "players online" in line][:1]
    record["final_first"] = _snapshot(first, diamond)
    steps.check("exactly_one_live_session_after_release", len(winners) == 1)
    steps.check("item_conserved_after_release",
                len(winners) == 1 and after[winners[0]]["held"] + after[winners[0]]["ground"] == 1)
    steps.check("no_stale_player_entity", all(not data.get("other_players") for data in after.values()))
    steps.check("store_after_restart_has_the_item_once",
                record["after_restart"].get("held", 0) + record["after_restart"].get("ground", 0) == 1)
    steps.check("final_one_player_online", bool(record["final_list"]) and "There are 1 of" in record["final_list"][0])
    steps.check("final_item_conserved",
                record["final_first"].get("held", 0) + record["final_first"].get("ground", 0) == 1)


def _config_phase(steps: ProbeSteps, first: Any, start: Any, address: str,
                  record: dict[str, Any], diamond: str) -> None:
    """The incumbent reconnects and is frozen (SIGSTOP) mid-join; the second login completes; then the
    incumbent resumes. At most one live session, the diamond exactly once."""
    import signal

    from .. import runtime

    pid = steps.client.break_client_pid
    log = steps.client.server_log_path
    marker = "screen service started for admitted loader session"
    admitted = log.read_text(errors="replace").count(marker)
    first.call_tool("minecraft_disconnect")
    first.call_tool("minecraft_connect", {"server_addr": address})
    screens = []
    if os.environ.get("BREAK_DL_FREEZE") == "joining":
        # Freeze while the client shows a joining screen (login/configuration/terrain), before Play.
        while True:
            seen = first.call_tool("minecraft_observe")
            screens.append((seen.get("in_play"), seen["screen"].get("class")))
            if seen.get("in_play") is True or any(word in str(seen["screen"].get("class"))
                                                  for word in ("Connect", "Receiving", "Configuration", "Loading", "Progress", "Level")):
                break
            first.call_tool("minecraft_wait_for_state_change", {
                "observed_version": seen["state_version"], "timeout_seconds": 10})
    os.kill(pid, signal.SIGSTOP)
    record["frozen_screens"] = screens[-4:]
    record["frozen_after_admission"] = log.read_text(errors="replace").count(marker) > admitted
    try:
        second = start()
        try:
            play = second.call_tool("minecraft_wait_for_play", {"timeout_seconds": 90})
            record["second_in_play_while_first_frozen"] = play.get("in_play")
        except Exception as error:  # noqa: BLE001 - recorded outcome
            record["second_wait_error"] = str(error)[:300]
        seen = second.call_tool("minecraft_observe")
        record["second_before_resume"] = {"in_play": seen.get("in_play"),
                                          "held": _held(seen, diamond) if seen.get("in_play") else None}
    finally:
        os.kill(pid, signal.SIGCONT)
    for name, client in (("first", first), ("second", second)):
        try:
            client.call_tool("minecraft_wait_for_play", {"timeout_seconds": 30})
        except Exception as error:  # noqa: BLE001
            record[name + "_resume_wait"] = str(error)[:200]
    live = [c for c in (second, first) if c.call_tool("minecraft_observe").get("in_play")]
    if live:
        live[0].call_tool("minecraft_wait_ticks", {"ticks": 100})
    final = {}
    for name, client in (("first", first), ("second", second)):
        s = client.call_tool("minecraft_observe")
        final[name] = {"in_play": s.get("in_play"), "disconnect_reason": s.get("disconnect_reason"),
                       "held": _held(s, diamond) if s.get("in_play") else None,
                       "ground": _ground(client, diamond) if s.get("in_play") else None}
    record["final"] = final
    winners = [name for name, data in final.items() if data["in_play"]]
    steps.check("config_at_most_one_live_session", len(winners) <= 1)
    steps.check("config_item_conserved", bool(winners) and
                final[winners[0]]["held"] + final[winners[0]]["ground"] == 1)


def _nether_feedback(steps: ProbeSteps, first: Any, address: str, record: dict[str, Any]) -> None:
    """COMMAND-SUCCESS-FEEDBACK-GAMERULE in the Nether: a fresh session (reconnect) spawned in the Nether, so
    the client's automation lease survives the dimension change."""
    steps.fixture("execute in minecraft:the_nether run tp @s 8 100 8")
    while True:
        seen = first.call_tool("minecraft_observe")
        if seen.get("dimension") == "minecraft:the_nether" and seen.get("in_play") is True:
            break
        first.call_tool("minecraft_wait_for_state_change", {
            "observed_version": seen["state_version"], "timeout_seconds": steps.remaining()})
    first.call_tool("minecraft_disconnect")
    first.call_tool("minecraft_connect", {"server_addr": address})
    play = first.call_tool("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    steps.check("reconnected_in_the_nether", play.get("dimension") == "minecraft:the_nether")
    for rule, commands, label in ((False, ["time set 1234", "give @s minecraft:dirt 1", "weather rain", "xp add @s 3"],
                                   "nether_rule_false"),
                                  (True, ["time set 1300"], "nether_rule_true")):
        steps.call("minecraft_send_chat", {"message": f"gamerule send_command_feedback {str(rule).lower()}",
                                           "command": True})
        for command in commands:
            steps.call("minecraft_send_chat", {"message": command, "command": True})
        chat = _echo(steps, label)["recent_chat"]
        record[label] = chat[:12]
    success = ("Set ", "Gave ", "weather", "experience", "1234")
    steps.check("nether_success_suppressed_when_false",
                not any(word in line for line in record["nether_rule_false"] for word in success))
    steps.check("nether_success_shown_when_true", any("Set the time" in line or "1300" in line
                                                      for line in record["nether_rule_true"]))


def _attack(steps: ProbeSteps, origin: tuple[int, int, int], variant: str) -> None:
    """BREAK_DL_ATTACK breaker glue (not for merge)."""
    start = steps.client.start_second_login
    first = steps.client
    address = steps.client.server_address
    diamond = "minecraft:diamond"
    record: dict[str, Any] = {"variant": variant}
    steps.report["attack"] = record
    _give(steps, diamond)
    if variant == "auth":
        return _auth_attack(steps, first, start, address, record, diamond)
    if variant == "netherfb":
        return _nether_feedback(steps, first, address, record)
    if variant.startswith("hold"):
        return _hold_attack(steps, first, start, address, record, diamond, variant)
    if variant == "config":
        return _config_phase(steps, first, start, address, record, diamond)
    if variant == "cursor":
        point = _point(origin)
        _set(steps, point, "minecraft:chest")
        state = _open(steps, point, "ChestMenu", 63)
        source = next(stack["slot"] for stack in state["container"]["slots"]
                      if stack["slot"] >= 27 and stack["item_id"] == diamond)
        _click(steps, source)
        state = steps.observe()
        record["first_before"] = {"carried": state["container"]["carried"], "held": _held(state, diamond)}
        steps.check("diamond_on_cursor_before_second_login", state["container"]["carried"].get("count") == 1)
    if variant == "nether":
        steps.fixture("execute in minecraft:the_nether run tp @s 8 100 8")
        while True:
            seen = first.call_tool("minecraft_observe")
            if seen.get("dimension") == "minecraft:the_nether" and seen.get("in_play") is True:
                break
            first.call_tool("minecraft_wait_for_state_change", {
                "observed_version": seen["state_version"], "timeout_seconds": steps.remaining()})
    if variant == "pickup":
        # The diamond lies on the ground with its pickup delay running while the session is replaced.
        steps.call("minecraft_set_hotbar_slot", {"slot": 0})
        steps.call("minecraft_drop_selected_item", {"item_id": diamond, "count": 1})
    second = start()
    play = second.call_tool("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    steps.check("second_login_entered_play", play.get("in_play") is True)
    kicked = _left_play(first, steps, "first_session_replaced")
    record["first_kick"] = {key: kicked.get(key) for key in ("in_play", "disconnect_reason")}
    if variant == "pickup":
        second.call_tool("minecraft_wait_ticks", {"ticks": 80})
    state2 = second.call_tool("minecraft_observe")
    record["second"] = {"dimension": state2.get("dimension"), "held": _held(state2, diamond),
                        "ground": _ground(second, diamond), "carried": state2["container"]["carried"],
                        "same_account": state2["player"]["entity_uuid"] == steps.report["player_uuid"]}
    if variant == "three":
        third = start("third-login")
        first.call_tool("minecraft_connect", {"server_addr": address})
        clients = {"first": first, "second": second, "third": third}
        for name in ("third", "first"):
            try:
                clients[name].call_tool("minecraft_wait_for_play", {"timeout_seconds": 60})
            except Exception as error:  # noqa: BLE001 - a loser may be refused; recorded below
                record[name + "_wait_error"] = str(error)[:300]
        live = []
        for name, client in clients.items():
            seen = client.call_tool("minecraft_observe")
            if seen.get("in_play") is True:
                live.append(name)
        settle = clients[live[-1]] if live else first
        settle.call_tool("minecraft_wait_ticks", {"ticks": 60})
        final = {}
        for name, client in clients.items():
            seen = client.call_tool("minecraft_observe")
            final[name] = {"in_play": seen.get("in_play"), "disconnect_reason": seen.get("disconnect_reason"),
                           "held": _held(seen, diamond) if seen.get("in_play") else None}
        record["three_final"] = final
        winners = [name for name, data in final.items() if data["in_play"]]
        steps.check("three_exactly_one_session_in_play", len(winners) == 1)
        ground = _ground(clients[winners[0]], diamond) if winners else 0
        record["three_ground"] = ground
        steps.check("three_diamond_conserved", sum(final[name]["held"] for name in winners) + ground == 1)
        return
    steps.check("second_inventory_conserved", record["second"]["held"] + record["second"]["ground"] == 1)
    if variant == "nether":
        steps.check("second_spawned_where_first_left", record["second"]["dimension"] == "minecraft:the_nether")
    first.call_tool("minecraft_connect", {"server_addr": address})
    play = first.call_tool("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    steps.check("first_client_relogin_entered_play", play.get("in_play") is True)
    _left_play(second, steps, "second_session_replaced_by_relogin")
    back = first.call_tool("minecraft_observe")
    record["relogin"] = {"dimension": back.get("dimension"), "held": _held(back, diamond),
                         "ground": _ground(first, diamond)}
    steps.check("relogin_inventory_conserved", record["relogin"]["held"] + record["relogin"]["ground"] == 1)


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    start = getattr(steps.client, "start_second_login", None)
    if not callable(start):
        raise RuntimeError("duplicate login requires the second-login client hook")
    if os.environ.get("BREAK_DL_ATTACK"):
        return _attack(steps, origin, os.environ["BREAK_DL_ATTACK"])
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
    # NATIVE-KICK-REASON-BROKEN-PIPE, the row's own repro: a duplicate-login kick while the incumbent
    # is in the Nether. The client is falling from the travel spawn and writes itself to the server
    # every tick, so an outbound write can fail before the kick packet already in the receive buffer
    # is decoded; the reason the server sent must still be what the client shows.
    steps.fixture("execute in minecraft:the_nether run tp @s 8 100 8")
    while True:
        seen = first.call_tool("minecraft_observe")
        if seen.get("dimension") == "minecraft:the_nether" and seen.get("in_play") is True:
            break
        first.call_tool("minecraft_wait_for_state_change", {
            "observed_version": seen["state_version"], "timeout_seconds": steps.remaining()})
    nether = start("nether-login")
    nether.call_tool("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    kicked = _kicked(first, steps, "nether_session_replaced", min(120.0, steps.remaining()))
    steps.report["nether_kick"] = {
        key: kicked.get(key) for key in ("in_play", "disconnect_reason", "disconnected", "state_version")}
    reason = str(kicked.get("disconnect_reason") or "")
    steps.check("kick_reason_preserved", "You logged in from another location" in reason)
    steps.check("kick_reason_not_broken_pipe", "Broken pipe" not in reason)
    # Leave the scenario on a live incumbent session: the ordered chat echo that closes
    # `native_v2.run` must run on a client that is in play, on either side of the fix.
    first.call_tool("minecraft_connect", {"server_addr": address})
    first.call_tool("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})

