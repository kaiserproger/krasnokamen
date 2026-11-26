"""Buildings-qa flows: the sequential per-blueprint flow, the parallel one-settlement-per-
blueprint layout and the per-blueprint blueprint and function flow.

The client/report seams live on ``buildings_qa`` (the tests patch them there), so this
module reaches them as ``_qa.<name>`` and never binds them.
"""

from __future__ import annotations

import math
import os
import time as _time
from pathlib import Path
from typing import Any

from ..profiles import ProfileContext
from . import native_client
from .probe_steps import ProbeSteps
from . import buildings_qa as _qa
from .buildings_qa_build import (
    _bound_count, _building_bounds_for, _building_seq_in, _find_nearby_chest, _hand_storage,
    _site_center, _walk_to,
)
from .buildings_qa_consts import (
    DIALOG_CLASS, LEVELS, PARALLEL_RADIUS, VILLAGE_CENTER, _chunked_fills,
)
from .buildings_qa_probe import _verdict
from .buildings_qa_project import (
    _admitted_or_caught, _missing_level_order, _order_refusal, _order_via_dialog,
    _upgrade_via_dialog, _wait_certified_origin,
)
from .buildings_qa_verify import _restart_resume


def _scenario_sequential(probe: ProbeSteps, detail: dict[str, Any], ctx: ProfileContext,
                         artifact: Path, targets: tuple[str, ...],
                         address: str, config: Path, server_log_path: Path) -> None:
    """Sequential per-blueprint: one founded settlement AT the player (r=64)."""
    def fixture(command: str) -> None:
        probe.fixture(command)
    sub_all = detail.setdefault("blueprints", {})
    fixture("gamemode creative @s")
    fixture("gamerule advance_time false")
    fixture("gamerule spawn_mobs false")
    fixture("gamerule random_tick_speed 0")
    fixture("time set 3000")
    for bp in targets:
        tag = f"buildings_{bp}"
        sub = sub_all.setdefault(bp, {})
        sname = f"bqa_{bp}"
        state = _qa._send(probe, "settlement list")
        if sname in " ".join(_qa._chat_lines(state)):
            sub["resumed_settlement"] = True
        else:
            me = probe.observe().get("player") or {}
            try:
                px = int(math.floor(float(me["x"]))); py = int(math.floor(float(me["y"]))); pz = int(math.floor(float(me["z"])))
            except (KeyError, TypeError, ValueError):
                probe.check(f"{tag}_site", False)
                continue
            # Flatten the FULL r=64 build disc: the planner cursor drifts ~20
            # sites/s and request_build tries 64 spiral positions from wherever
            # the cursor is — a small pad is outside the window by order time.
            # Chunked fills stay under the 32768-block vanilla fill limit.
            for _fx0 in range(px - 64, px + 64, 32):
                for _fz0 in range(pz - 64, pz + 64, 32):
                    _fx1 = min(_fx0 + 31, px + 64); _fz1 = min(_fz0 + 31, pz + 64)
                    fixture(f"fill {_fx0} {py - 2} {_fz0} {_fx1} {py - 2} {_fz1} minecraft:dirt")
                    fixture(f"fill {_fx0} {py - 1} {_fz0} {_fx1} {py - 1} {_fz1} minecraft:grass_block")
                    fixture(f"fill {_fx0} {py} {_fz0} {_fx1} {py + 24} {_fz1} minecraft:air")
            # Drain the fill-confirmation queue first so the create echo is
            # not buried under streaming 'Changed N' confirmations.
            probe.call("minecraft_wait_ticks", {"ticks": 200})
            fixture(f"settlement create {sname} {PARALLEL_RADIUS}")
            probe.call("minecraft_wait_ticks", {"ticks": 40})
            echo = _qa._echo(probe, f"{tag}_founded")
            ok = any(f"Founded {sname}" in line for line in echo.get("recent_chat", []))
            probe.check(f"{tag}_founded", ok)
            _verdict(sub, "settlement_founded", "PASS" if ok else "FAIL", f"{sname} r={PARALLEL_RADIUS}")
            if not ok:
                continue
            sub["site"] = [px, py, pz]
            # Resync AFTER create so the anchor stays at the pad center: the
            # settle-tp must never precede create (anchor = player feet).
            probe.fixture(f"tp @s {px + 20.5} {py + 3.0} {pz + 20.5}")
            probe.call("minecraft_wait_ticks", {"ticks": 60})
        # Resume: read the settlement's REAL anchor from info (never default).
        info = " ".join(_qa._chat_lines(_qa._send(probe, f"settlement info {sname}"), 10))
        import re as _re2
        m = _re2.search(r"anchor=\[(-?\d+),\s*(-?\d+),\s*(-?\d+)\]", info)
        if m:
            sub["site"] = [int(m.group(1)), int(m.group(2)), int(m.group(3))]
            sub["anchor_source"] = "settlement info"
        elif "site" not in sub:
            probe.check(f"{tag}_anchor", False)
            _verdict(sub, "anchor", "FAIL", "no anchor in settlement info and no prior site")
            continue
        site = tuple(sub["site"])
        probe.fixture(f"tp @s {site[0] + 0.5} {site[1] + 1.0} {site[2] + 0.5}")
        shell = _qa._hand_storage_at(probe, sub, tag, sname, site)
        if shell is None:
            found = _find_nearby_chest(probe, sub)
            if found is None:
                probe.check(f"{tag}_shell", False)
                continue
            shell = found
            sub["shell_reused"] = list(shell)
        sub["storage_chest"] = list(shell)
        # Capture a per-run baseline after the hand-built storage exists. A
        # pre-existing certificate, including that storage, is never our build.
        _qa._at_settlement_anchor(probe, sub)
        if not _qa._load_blueprints().get(bp, {}).get("levels"):
            _missing_level_order(probe, sub, tag, bp, 1, None)
        sub["baseline_building_ids"] = sorted(_qa._building_records(probe, bp))
        _qa._queue_planner_fence(probe, sub, tag, bp, 1)
        have = _bound_count(probe, sname) if sub.get("resumed_settlement") else 0
        sub["bound_members"] = have
        if have >= 2:
            pass  # Chain resume: keep existing residents; no population pollution.
        elif not _qa._summon_bind(probe, sub, tag, sname, shell, 6):
            continue
        use_dialog = bp == "house" and ctx.windowed
        for level in range(1, LEVELS[bp] + 1):
            if level > 1:
                previous = _qa._current_certificate(probe, sub, tag, bp, level - 1)
                probe.check(f"{tag}_L{level}_found", previous is not None)
                bid = previous[0]
            else:
                bid = None
            if len(_qa._load_blueprints().get(bp, {}).get("levels", [])) < level:
                _missing_level_order(probe, sub, tag, bp, level, bid)
            # FUNDED-FIRST: stock the chest, assert the delta, THEN order.
            _qa._fund_level(probe, sub, bp, level, tuple(shell))
            if sub.get("behaviour", {}).get(f"L{level}_funded") != "PASS":
                probe.check(f"{tag}_L{level}_built", False)
                _verdict(sub, f"L{level}_upgrade", "FAIL",
                         "funding delta absent; order refused by rule")
                continue
            # All project commands and menus select the nearest settlement.
            _qa._at_settlement_anchor(probe, sub)
            if level == 1 and use_dialog and ctx.windowed:
                probe.fixture(f"tp @s {site[0] + 0.5} {site[1] + 1.0} {site[2] + 0.5}")
            command = f"settlement build {bp}" if level == 1 else f"settlement build upgrade {bid}"
            dialog = (lambda: _order_via_dialog(probe, sub, tag, bp, level, sname)) if level == 1 else \
                (lambda: _upgrade_via_dialog(probe, sub, tag, bp, bid, level, sname))
            admitted = _qa._order_echo_retry(
                probe, sub, tag, bp, level, command, use_dialog, ctx, dialog_fn=dialog,
                upgrade_bid=bid,
                refund=lambda: _qa._fund_level(probe, sub, bp, level, tuple(shell)))
            if level == 1:
                refusal = _order_refusal(sub, 1)
                if refusal is not None:
                    # A terminal server refusal (blueprint not registered: "Unknown
                    # blueprint.") never admits on retry: an ordinary failed check,
                    # never an unexpected exception. Record the verdict first — the
                    # check raises before anything after it runs.
                    _verdict(sub, "L1_built", "FAIL", f"server refused the order: {refusal}")
                    probe.check(f"{tag}_L1_built", False)
                    continue
            # Re-establish the deliberately unfundable pending order immediately
            # after admission, before even the shortest upgrade can complete.
            _qa._queue_planner_fence(probe, sub, tag, bp, level)
            caught = _admitted_or_caught(probe, sub, tag, bp, level, admitted)
            if caught is not None:
                sub[f"L{level}_admission"] = "same-run certificate"
            if level == 1 and use_dialog:
                probe.check(f"{tag}_L1_order_menu", "L1_dialog_order" in sub)
            if os.environ.get("BUILDINGS_QA_STOP_AFTER_ORDER"):
                probe.check(f"{tag}_L{level}_built", True)
                _verdict(sub, f"L{level}_upgrade", "PASS",
                         "order-by-UI admitted (phase-1 receipt); build monitored in phase 2")
                sub[f"L{level}_order_phase1"] = True
                continue
            if level == 2 and caught is None and _qa._adversarial_break(probe, sub, tag, bp, level, bid):
                sub[f"L{level}_adversarial"] = "mid-build block break issued; build waited on site"
                _qa._fund_level(probe, sub, bp, level, tuple(shell))
                _qa._at_settlement_anchor(probe, sub)
            corner = _qa._break_corner(probe, sub, tag, bp) if level == 1 and bp == "house" else None
            origin = _qa._wait_certified_origin_here(probe, sub, tag, bp, level, ctx, artifact)
            if origin is None:
                if bp == "house" and level == 1:
                    # A house whose injected break is never re-placed never certifies: the repair
                    # outcome is a product verdict on this path too, and `probe.check` below raises
                    # immediately, so it must be recorded first (a negative control reads it).
                    _verdict(sub, "broken_cell_replaced", "FAIL",
                             "the injected break was not re-placed; the L1 certificate never appeared")
                    probe.check(f"{tag}_L1_broken_cell_repaired", False)
                probe.check(f"{tag}_L{level}_built", False)
                _verdict(sub, f"L{level}_upgrade", "FAIL",
                         f"L{level} cert never appeared while on site")
                continue
            if level == 1:
                sub["L1_origin"] = list(origin)
                if corner is not None:
                    _qa._broken_replaced(probe, sub, tag, corner)
            if ctx.windowed:
                _qa._capture_level(probe, sub, artifact, tag, bp, level, origin)
            _qa._verify_level(probe, sub, ctx, artifact, bp, level, origin, tag, sname, tuple(shell))
            probe.check(f"{tag}_L{level}_built", True)
            verdict = "L1_built" if level == 1 else f"L{level}_upgrade"
            _verdict(sub, verdict, "PASS",
                     f"same-run #{sub['building_id']} L{level} at {list(origin)}; "
                     "exact-level blocks, entrance and light checked")
            _qa._at_settlement_anchor(probe, sub)
        if use_dialog:
            probe.check(f"{tag}_upgrade_menu",
                        any(f"L{level}_dialog_upgrade" in sub for level in range(2, LEVELS[bp] + 1)))


def _scenario_parallel(probe: ProbeSteps, detail: dict[str, Any], ctx: ProfileContext,
                       artifact: Path, targets: tuple[str, ...],
                       address: str, config: Path, server_log_path: Path) -> None:
    """One world, one founded settlement per blueprint (400-grid, r=64)."""
    def fixture(command: str) -> None:
        probe.fixture(command)
    rooted = probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float),
                        "player_present")
    sub_all = detail.setdefault("blueprints", {})
    for bp in targets:
        sub_all.setdefault(bp, {})["site"] = list(_site_center(bp))
    for bp in targets:
        cx, cy, cz = _site_center(bp)
        for cmd in _chunked_fills(cx - 40, cy - 8, cz - 40, cx + 40, cy + 16, cz + 40,
                                  "minecraft:air"):
            fixture(cmd)
        for cmd in _chunked_fills(cx - 40, cy - 2, cz - 40, cx + 40, cy - 2, cz + 40,
                                  "minecraft:dirt"):
            fixture(cmd)
        for cmd in _chunked_fills(cx - 40, cy - 1, cz - 40, cx + 40, cy - 1, cz + 40,
                                  "minecraft:grass_block"):
            fixture(cmd)
    fixture("gamemode creative @s")
    fixture("gamerule advance_time false")
    fixture("gamerule spawn_mobs false")
    fixture("gamerule random_tick_speed 0")
    fixture("time set 3000")
    for bp in targets:
        cx, cy, cz = _site_center(bp)
        tag = f"buildings_{bp}"
        probe.fixture(f"tp @s {cx + 0.5} {cy + 1.0} {cz + 0.5}")
        probe.wait(lambda s, cx=cx, cz=cz: abs(float((s.get("player") or {}).get("x", 0)) - cx) < 3.0,
                   f"{tag}_site_pos")
        fixture(f"settlement create bqa_{bp} {PARALLEL_RADIUS}")
        echo = _qa._echo(probe, f"{tag}_founded")
        ok = any("Founded" in line for line in echo.get("recent_chat", []))
        probe.check(f"{tag}_founded", ok)
        _verdict(sub_all[bp], "settlement_founded", "PASS" if ok else "FAIL",
                 f"center={[cx, cy, cz]} r={PARALLEL_RADIUS}")
        if not ok:
            return
    for bp in targets:
        cx, cy, cz = _site_center(bp)
        tag = f"buildings_{bp}"
        sub = sub_all[bp]
        probe.fixture(f"tp @s {cx + 0.5} {cy + 1.0} {cz + 0.5}")
        probe.wait(lambda s, cx=cx, cz=cz: abs(float((s.get("player") or {}).get("x", 0)) - cx) < 5.0,
                   f"{tag}_shell_pos")
        shell = _qa._hand_storage_at(probe, sub, tag, f"bqa_{bp}", (cx, cy, cz))
        if shell is None:
            continue
        sub["storage_chest"] = list(shell)
        if not _qa._summon_bind(probe, sub, tag, f"bqa_{bp}", shell, 6):
            continue
    for bp in targets:
        sub = sub_all[bp]
        if "storage_chest" not in sub or sub.get("bound_members", 0) < 2:
            continue
        tag = f"buildings_{bp}"
        if bp in ("house", "barracks"):
            probe.fixture(f"tp @s {_site_center(bp)[0] + 0.5} {_site_center(bp)[1] + 1.0} {_site_center(bp)[2] + 0.5}")
            _order_via_dialog(probe, sub, tag, bp, 1, f"bqa_{bp}")
        else:
            probe.fixture(f"tp @s {_site_center(bp)[0] + 0.5} {_site_center(bp)[1] + 1.0} {_site_center(bp)[2] + 0.5}")
            fixture(f"settlement build {bp}")
            probe.call("minecraft_wait_ticks", {"ticks": 20})
    for bp in targets:
        sub = sub_all[bp]
        if "storage_chest" not in sub or sub.get("bound_members", 0) < 2:
            continue
        _qa._fund_level(probe, sub, bp, 1, tuple(sub["storage_chest"]))
    pending = {bp for bp in targets
               if "storage_chest" in sub_all[bp] and sub_all[bp].get("bound_members", 0) >= 2}
    deadline = _time.monotonic() + (probe.deadline - _time.monotonic() - 900.0)
    while pending and _time.monotonic() < deadline:
        probe.call("minecraft_wait_ticks", {"ticks": 200})
        for bp in sorted(pending):
            origin = _building_bounds_for(probe, f"bqa_{bp}", bp, 1,
                                          exclude=[sub_all[bp].get("shell_bounds", [[0, 0, 0]])[0]])
            if origin is not None:
                sub_all[bp]["L1_origin"] = list(origin)
                pending.discard(bp)
    for bp in targets:
        sub = sub_all[bp]
        tag = f"buildings_{bp}"
        if "L1_origin" not in sub:
            probe.check(f"{tag}_L1_built", False)
            _verdict(sub, "L1_built", "FAIL", "L1 cert never appeared in parallel window")
            continue
        probe.check(f"{tag}_L1_built", True)
        if ctx.windowed:
            try:
                native_client.capture(probe.client, artifact, f"{tag}-L1")
            except Exception:
                pass
        _qa._verify_level(probe, sub, ctx, artifact, bp, 1, tuple(sub["L1_origin"]), tag,
                      f"bqa_{bp}", tuple(sub["storage_chest"]))
    for bp in targets:
        if LEVELS[bp] < 2 or "L1_origin" not in sub_all.get(bp, {}):
            continue
        sub = sub_all[bp]
        tag = f"buildings_{bp}"
        bid = _building_seq_in(probe, f"bqa_{bp}", bp, 1,
                               exclude=[sub.get("shell_bounds", [[0, 0, 0]])[0]])
        if bid is None:
            continue
        sub["L2_bid"] = bid
        if bp in ("house", "barracks") and ctx.windowed:
            _upgrade_via_dialog(probe, sub, tag, bp, bid, 2, f"bqa_{bp}")
        else:
            fixture(f"settlement build upgrade {bid}")
            probe.call("minecraft_wait_ticks", {"ticks": 20})
        _qa._fund_level(probe, sub, bp, 2, tuple(sub["storage_chest"]))
    pending_up = {bp for bp in targets if LEVELS[bp] >= 2 and "L2_bid" in sub_all.get(bp, {})}
    deadline_up = _time.monotonic() + (probe.deadline - _time.monotonic() - 400.0)
    while pending_up and _time.monotonic() < deadline_up:
        probe.call("minecraft_wait_ticks", {"ticks": 200})
        for bp in sorted(pending_up):
            origin = _building_bounds_for(probe, f"bqa_{bp}", bp, 2,
                                          exclude=[sub_all[bp].get("shell_bounds", [[0, 0, 0]])[0]])
            if origin is not None:
                sub_all[bp]["L2_origin"] = list(origin)
                pending_up.discard(bp)
    for bp in targets:
        if LEVELS[bp] < 2:
            continue
        sub = sub_all.get(bp, {})
        tag = f"buildings_{bp}"
        if "L2_origin" not in sub:
            probe.check(f"{tag}_L2_built", False)
            _verdict(sub, "L2_upgrade", "FAIL", "L2 cert never appeared")
            continue
        probe.check(f"{tag}_L2_built", True)
        _qa._verify_level(probe, sub, ctx, artifact, bp, 2, tuple(sub["L2_origin"]), tag,
                      f"bqa_{bp}", tuple(sub["storage_chest"]))
    if "house" in targets and LEVELS.get("house", 0) >= 3 and "L2_origin" in sub_all.get("house", {}):
        sub = sub_all["house"]
        bid = _building_seq_in(probe, "bqa_house", "house", 2,
                               exclude=[sub.get("shell_bounds", [[0, 0, 0]])[0]])
        if bid is not None and ctx.windowed:
            _upgrade_via_dialog(probe, sub, "buildings_house", "house", bid, 3, "bqa_house")
            _qa._fund_level(probe, sub, "house", 3, tuple(sub["storage_chest"]))
            origin3 = None
            d3 = _time.monotonic() + (probe.deadline - _time.monotonic() - 120.0)
            while _time.monotonic() < d3 and origin3 is None:
                probe.call("minecraft_wait_ticks", {"ticks": 200})
                origin3 = _building_bounds_for(probe, "bqa_house", "house", 3,
                                              exclude=[sub.get("shell_bounds", [[0, 0, 0]])[0]])
            if origin3 is not None:
                sub["L3_origin"] = list(origin3)
                probe.check("buildings_house_L3_built", True)
                _qa._verify_level(probe, sub, ctx, artifact, "house", 3, origin3,
                              "buildings_house", "bqa_house", tuple(sub["storage_chest"]))
            else:
                probe.check("buildings_house_L3_built", False)
                _verdict(sub, "L3_upgrade", "FAIL", "L3 cert never appeared")


def _scenario_blueprint(probe: ProbeSteps, detail: dict[str, Any], ctx: ProfileContext,
                        artifact: Path, address: str, config: Path,
                        server_log_path: Path, bp: str, timeout: float) -> None:
    tag = f"buildings_{bp}"
    sub: dict[str, Any] = detail.setdefault("blueprints", {}).setdefault(bp, {})

    def fixture(command: str) -> None:
        probe.fixture(command)

    # --- (1) walk to the seed-100 village; ownerless settlement adopts ---
    rooted = probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float),
                        "player_present")
    sub["spawn_observed"] = [rooted["player"].get("x"), rooted["player"].get("y"),
                             rooted["player"].get("z")]
    try:
        walked = _walk_to(probe, sub, (float(VILLAGE_CENTER[0]), float(VILLAGE_CENTER[1]),
                                       float(VILLAGE_CENTER[2])), "approach")
    except Exception as exc:
        probe.fixture(f"tp @s {VILLAGE_CENTER[0]} {VILLAGE_CENTER[1]} {VILLAGE_CENTER[2]}")
        probe.wait(lambda s: isinstance((s.get("player") or {}).get("x"), float),
                   "player_repositioned")
        sub["walk_fallback"] = f"teleport after navigate failure: {exc}"
        walked = probe.observe()
    player = walked.get("player") or {}
    arrived = math.dist((float(player.get("x", 0)), float(player.get("z", 0))),
                         (float(VILLAGE_CENTER[0]), float(VILLAGE_CENTER[2]))) < 6.0
    probe.check(f"{tag}_walk_to_village", arrived)
    _verdict(sub, "walk_to_village", "PASS" if arrived else "FAIL",
             f"at {[player.get('x'), player.get('y'), player.get('z')]} vs village {VILLAGE_CENTER}")
    if ctx.windowed:
        native_client.capture(probe.client, artifact, f"{tag}-approach")

    settlement_name: str | None = None
    deadline = _time.monotonic() + 180.0
    while _time.monotonic() < deadline:
        state = _qa._send(probe, "settlement list")
        joined = " ".join(_qa._chat_lines(state))
        if "Settlements:" in joined and "No settlements" not in joined:
            for line in _qa._chat_lines(state):
                if line.startswith("Settlements:"):
                    names = [n.strip().rstrip(".") for n in line.split(":", 1)[1].split(",")]
                    settlement_name = next((n for n in names if n), None)
                    break
            if settlement_name:
                break
        probe.call("minecraft_wait_ticks", {"ticks": 100})
    sub["settlement_name"] = settlement_name
    if not settlement_name:
        probe.check(f"{tag}_ownerless", False)
        _verdict(sub, "ownerless_settlement", "FAIL", "no settlement listed")
        return
    state = _qa._send(probe, f"settlement info {settlement_name}")
    info_text = " ".join(_qa._chat_lines(state, 20))
    ownerless = "owner=autonomous" in info_text
    resumed_owned = (not ownerless) and ("BuildQA" in info_text or "owner" in info_text.lower())
    probe.check(f"{tag}_ownerless", ownerless or resumed_owned)
    _verdict(sub, "ownerless_settlement", "PASS" if ownerless else ("RESUMED-OWNED" if resumed_owned else "FAIL"),
             f"name={settlement_name} resumed_owned={resumed_owned}")
    if not ownerless and not resumed_owned:
        return
    if resumed_owned:
        sub["resumed_claim"] = True

    # --- (2) claim (fresh worlds only; resumed worlds are already owned) ---
    if not sub.get("resumed_claim"):
        probe.fixture("tp @s 117 71 5")
        probe.wait(lambda s: abs(float((s.get("player") or {}).get("x", 0)) - 117.0) < 3.0, "claim_position")
        state = _qa._send(probe, "settlement claim")
        claim_chat = _qa._chat_lines(state, 8)
        sub["claim_chat"] = claim_chat
        claim_ok = any("Claimed " in line for line in claim_chat)
        probe.check(f"{tag}_claimed", claim_ok)
        _verdict(sub, "claimed_via_dialog", "PASS" if claim_ok else "FAIL", f"{claim_chat[:4]}")
        if not claim_ok:
            return
        state = _qa._send(probe, "settlement")
        opened = probe.wait(lambda s: s["screen"].get("class") == DIALOG_CLASS, "claim_dialog_open")
        title = opened["screen"].get("title", "")
        sub["claim_dialog_title"] = title
        no_claim = not any("Claim" in str(n.get("label", "")) for n in opened.get("ui_nodes", [])
                           if isinstance(n, dict))
        probe.check(f"{tag}_claim_gone", no_claim)
        _verdict(sub, "claim_button_gone_post_claim", "PASS" if no_claim else "FAIL",
                 "owner dialog shows no Claim button")
        if ctx.windowed:
            native_client.capture(probe.client, artifact, f"{tag}-claimed")
    else:
        probe.check(f"{tag}_claimed", True)
        _verdict(sub, "claimed_via_dialog", "PASS", "resumed world: already owned by BuildQA")

    # --- (3) storage funding source ---
    # Non-storage targets: hand-built SHELL funds the first blueprint build.
    # Storage target: the shell IS the L1 certificate path — order the authored
    # `storage` blueprint L1 via the planner and Builder-build it like the rest.
    # The shell still funds the build (player-funds-storage path).
    if bp == "storage" and not sub.get("resumed_claim"):
        shell = _hand_storage(probe, sub, tag, settlement_name)
        if shell is None:
            return
        storage_chest = shell
        sub["shell_funds_blueprint_storage"] = True
    elif sub.get("resumed_claim"):
        found = _find_nearby_chest(probe, sub)
        storage_chest = tuple(found) if found else (178, 71, 6)
        sub["shell_reused"] = list(storage_chest)
    else:
        shell = _hand_storage(probe, sub, tag, settlement_name)
        if shell is None:
            return
        storage_chest = shell
    sub["storage_chest"] = list(storage_chest)
    if ctx.windowed:
        native_client.capture(probe.client, artifact, f"{tag}-shell-storage")
    # --- (3b) summon + bind Builder candidates (planner needs residents>0;
    # assign_builder picks an unemployed adult). 6 fresh villagers at the shell.
    bound = _qa._summon_bind(probe, sub, tag, settlement_name, storage_chest, 6)
    if not bound:
        return
    # --- (4) order the target blueprint L1 (dialog button for house+barracks) ---
    use_dialog = bp in ("house", "barracks")
    if use_dialog:
        _order_via_dialog(probe, sub, tag, bp, 1, settlement_name)
    else:
        fixture(f"settlement build {bp}")
        probe.call("minecraft_wait_ticks", {"ticks": 40})
        sub[f"L1_order_chat"] = _qa._chat_lines(probe.observe(), 8)
    # Fund from the built storage chest via /give + ordinary shift-click deposit.
    _qa._fund_level(probe, sub, bp, 1, storage_chest)
    origin = _wait_certified_origin(probe, sub, tag, bp, 1, 1500.0, ctx, artifact)
    if origin is None:
        probe.check(f"{tag}_L1_built", False)
        _verdict(sub, "L1_built", "FAIL", "building list never showed the L1 cert")
        return
    probe.check(f"{tag}_L1_built", True)
    _qa._verify_level(probe, sub, ctx, artifact, bp, 1, origin, tag, settlement_name, storage_chest)

    # --- (6) upgrades L(n)->L(n+1) ---
    for level in range(2, LEVELS[bp] + 1):
        bid = _qa._building_seq(probe, bp, level - 1)
        if bid is None:
            probe.check(f"{tag}_L{level}_found", False)
            _verdict(sub, f"L{level}_upgrade", "FAIL", "no building id in listing")
            continue
        if use_dialog and ctx.windowed:
            _upgrade_via_dialog(probe, sub, tag, bp, bid, level, settlement_name)
        else:
            fixture(f"settlement build upgrade {bid}")
            probe.call("minecraft_wait_ticks", {"ticks": 40})
        # Adversarial on the FIRST upgrade only: break one non-structural block mid-build.
        adversarial_done = False
        if level == 2:
            adversarial_done = _qa._adversarial_break(probe, sub, tag, bp, level, bid)
        _qa._fund_level(probe, sub, bp, level, storage_chest)
        origin_up = _wait_certified_origin(probe, sub, tag, bp, level, 1500.0, ctx, artifact)
        if origin_up is None:
            probe.check(f"{tag}_L{level}_built", False)
            _verdict(sub, f"L{level}_upgrade", "FAIL", "upgraded cert never appeared")
            continue
        probe.check(f"{tag}_L{level}_built", True)
        if adversarial_done:
            sub[f"L{level}_adversarial"] = "mid-build block break issued; build waited to completion"
        _qa._verify_level(probe, sub, ctx, artifact, bp, level, origin_up, tag,
                      settlement_name, storage_chest)

    # --- (7) restart mid-idle: server restart on same world, settlement data survives ---
    _restart_resume(probe, sub, ctx, artifact, address, config, server_log_path,
                    tag, settlement_name, bp)
