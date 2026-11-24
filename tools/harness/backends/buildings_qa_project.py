"""Buildings-qa Projects and Buildings menus: certificates, the planner fence, order
and upgrade dialogs with their confirmation, and the lost-echo retries.

The client/report seams live on ``buildings_qa`` (the tests patch them there), so this
module reaches them as ``_qa.<name>`` and never binds them.
"""

from __future__ import annotations

import re
import time as _time
from pathlib import Path
from typing import Any

from ..profiles import ProfileContext
from . import native_client
from .probe_steps import ProbeSteps
from . import buildings_qa as _qa
from .buildings_qa_consts import (
    DIALOG_CLASS, KIND_LABEL, PROJECT_LABEL, PROJECT_STATUS_SLOT, _ORDER_TERMINAL,
)
from .buildings_qa_probe import _building_bounds, _container_titled


def _queue_planner_fence(probe: ProbeSteps, sub: dict[str, Any], tag: str,
                         bp: str, level: int) -> None:
    # Both fence blueprints also exist in the pre-fix catalog. Withhold wool
    # for House, or wheat seeds for Farm when the target itself needs wool.
    fence = "farm" if bp in ("house", "barracks") else "house"
    state = _qa._send_fresh(probe, f"settlement build {fence}",
                        (f"Queued {fence}:", f"Started {fence} ", "Unknown blueprint."))
    lines = _qa._chat_lines(state)
    acknowledgement = next((line for line in lines if line.startswith(f"Queued {fence}:")), None)
    sub[f"L{level}_planner_fence"] = acknowledgement or " | ".join(lines)
    probe.check(f"{tag}_L{level}_planner_fenced", acknowledgement is not None)


def _missing_level_order(probe: ProbeSteps, sub: dict[str, Any], tag: str, bp: str,
                         level: int, bid: int | None) -> None:
    """Ask the server before classifying a local catalog gap as a behavioural failure."""
    command = f"settlement build {bp}" if level == 1 else f"settlement build upgrade {bid}"
    state = _qa._send_fresh(probe, command, (
        "Unknown blueprint.", "Blueprint has no first level.", "No blueprint upgrade available",
        "Started ", "Queued ", "not enough:", "Construction bed material is unavailable.",
        "Building not found.", "Only the owner"))
    lines = _qa._chat_lines(state)
    sub[f"L{level}_missing_level_reply"] = lines
    refused = any(line.startswith(("Unknown blueprint.", "Blueprint has no first level."))
                  for line in lines) if level == 1 else any(
                      line.startswith("No blueprint upgrade available") for line in lines)
    if refused:
        probe.check(f"{tag}_L{level}_{'order' if level == 1 else 'upgrade'}_available", False)
    raise RuntimeError(f"{bp} L{level}: local blueprint unavailable; server reply {lines!r}; cannot verify")


def _order_echo_retry(probe: ProbeSteps, sub: dict[str, Any], tag: str, bp: str, level: int,
                      order_cmd: str, use_dialog: bool, ctx: ProfileContext,
                      dialog_fn=None, upgrade_bid: int | None = None) -> bool:
    """Issue one order; never infer admission from stale chat or an unrelated project.

    A refusal is usually transient planner contention, so retry up to three times about a
    minute apart and re-check admission after each attempt."""
    for attempt in range(1, 4):
        state: dict[str, Any] = {}
        if use_dialog and ctx.windowed and dialog_fn is not None:
            try:
                dialog_fn()
                message = ""
            except Exception as exc:
                message = f"dialog failed: {exc}"
        else:
            state = _qa._send_fresh(
                probe, order_cmd,
                (f"Started {bp} ", f"Queued {bp}", "No blueprint upgrade available",
                 "Unknown blueprint.", "Busy.", "not enough:"))
            message = " | ".join(_qa._chat_lines(state, 12))
        sub[f"L{level}_order_attempt{attempt}"] = message[:300]
        if f"started {PROJECT_LABEL.get(bp, bp)} construction.".lower() in message.lower() or \
                f"started {bp} construction.".lower() in message.lower():
            return True
        probe.call("minecraft_wait_ticks", {"ticks": 20})
        if _qa._project_active_quiet(probe, sub, tag, bp, level, upgrade_bid=upgrade_bid):
            # The planner admitted this level while our order was refused: follow its build.
            return True
        if any(prefix in message for prefix in _ORDER_TERMINAL):
            # A terminal answer is checked against the live project once, never retried:
            # another identical order cannot change it. The caller's `_admitted_or_caught`
            # still credits a certificate the planner produced on its own.
            return False
        if _qa._current_certificate(probe, sub, tag, bp, level) is not None:
            return False
        # A queued order's site is searched in later ticks (owner D42); another
        # identical order would queue a second building, so follow this one.
        queued = f"queued {bp}: waiting for a site" in message.lower()
        # One MCP wait request accepts at most 255 ticks: split the minute.
        for ticks in (255, 255, 255, 255, 180):
            probe.call("minecraft_wait_ticks", {"ticks": ticks})
            if queued and _qa._project_active_quiet(probe, sub, tag, bp, level, upgrade_bid=upgrade_bid):
                return True
        if queued:
            return False
    return False


def _admitted_or_caught(probe: ProbeSteps, sub: dict[str, Any], tag: str, bp: str, level: int,
                        admitted: bool) -> tuple[int, int, int] | None:
    """Accept a refused order only if this run's tracked building reached this exact level."""
    certificate = None if admitted else _qa._current_certificate(probe, sub, tag, bp, level)
    caught = certificate[1] if certificate is not None else None
    probe.check(f"{tag}_L{level}_project", admitted or caught is not None)
    return caught


# `ui_text::Projects` is "Construction"/"Строительство" since the D63 dialog set, and
# `ui_menus` renders the blueprint catalogue (`projects_screen`) and the project-status page
# (`project_screen`) under that one title in one reused window. The driver therefore recognises
# each page by its rows, never by the page title alone.
PROJECT_TITLES = ("Construction", "Строительство")
_CATALOGUE_ROW = re.compile(r"^.+ L1$")
_PROJECT_CARD = re.compile(r"^(.+?) L(\d+) · ")
_SUMMARY_LINE = re.compile(r"^(?:Active projects|Активные стройки|Pending orders|Заказы|Заказов)\b")


def _page_names(state: dict[str, Any]) -> list[str]:
    return [str(row.get("name")) for row in (state.get("container") or {}).get("slots", [])
            if row.get("name")]


def _project_titled(state: dict[str, Any]) -> bool:
    screen = state.get("screen") or {}
    return screen.get("class") == _qa.CONTAINER_CLASS and str(screen.get("title")) in PROJECT_TITLES


def _catalogue_page(state: dict[str, Any]) -> bool:
    """The blueprint catalogue: `projects_screen` names every orderable blueprint `<label> L1`."""
    return _project_titled(state) and any(_CATALOGUE_ROW.match(name) for name in _page_names(state))


def _status_page(state: dict[str, Any]) -> bool:
    """The project-status page: the summary clock and/or an admitted project's card."""
    names = _page_names(state)
    return _project_titled(state) and (any(_SUMMARY_LINE.match(name) for name in names)
                                       or any(_PROJECT_CARD.match(name) for name in names))


def _project_page(probe: ProbeSteps) -> dict[str, Any]:
    """The open Construction status page of the Projects menu (Projects -> Status icon). The
    caller closes the screen."""
    _qa._close_any_screen(probe)
    probe.call("minecraft_send_chat", {"message": "settlement menu projects", "command": True})
    projects = probe.wait(_catalogue_page, "projects_menu_open", timeout_seconds=60)
    action = next((row for row in projects["container"]["slots"] if row.get("slot") == PROJECT_STATUS_SLOT), None)
    if action is None or not re.fullmatch(r"Status|Статус", str(action.get("name") or "")):
        raise RuntimeError(f"buildings-qa: Construction menu slot {PROJECT_STATUS_SLOT} is {action}, not Status")
    probe.call("minecraft_click_container_slot", {"slot": PROJECT_STATUS_SLOT, "button": "primary",
                                                  "timeout_seconds": probe.remaining()})
    return probe.wait(_status_page, "project_status_open", timeout_seconds=60)


def _order_refusal(sub: dict[str, Any], level: int) -> str | None:
    """The server's terminal refusal of the level's order attempts, if there is one.

    "Unknown blueprint." (the blueprint is not registered on the server) never
    admits on retry; report it as the failure reason instead of looping to the
    deadline."""
    for attempt in (3, 2, 1):
        echo = str(sub.get(f"L{level}_order_attempt{attempt}", ""))
        if "Unknown blueprint." in echo:
            return echo
    return None


def _project_view(probe: ProbeSteps) -> tuple[str, int, bool] | None:
    """Return the first admitted project's label and level, or None.

    `ui_menus::project_screen` puts each admitted project's card at slots 0.. as
    `<label> L<level> · <phase>`; a queued order is a pending-order book at slots 18.. with no
    card, so a page without a card has no active project."""
    page = _qa._project_page(probe)
    slots = page["container"]["slots"]
    probe.call("minecraft_close_screen", {})
    for row in slots:
        match = _PROJECT_CARD.match(str(row.get("name") or ""))
        if match:
            return match.group(1), int(match.group(2)), True
    return None


def _project_status(probe: ProbeSteps) -> tuple[str, int] | None:
    view = _project_view(probe)
    return (view[0], view[1]) if view is not None and view[2] else None


def _project_active_quiet(probe: ProbeSteps, sub: dict[str, Any], tag: str, bp: str,
                          level: int, upgrade_bid: int | None = None) -> bool:
    """Admission check without emitting probe.check verdicts (for retries): an active project
    of this blueprint at this level."""
    try:
        status = _project_status(probe)
    except Exception as exc:
        sub[f"L{level}_project_status_error"] = str(exc)[:200]
        return False
    sub[f"L{level}_project_body"] = f"{status}"
    return status is not None and status[1] == level and PROJECT_LABEL.get(bp, bp).lower() in status[0].lower()


def _project_active_echo(probe: ProbeSteps, sub: dict[str, Any], tag: str, bp: str, level: int,
                         order_cmd: str) -> bool:
    """Headless project oracle: the order chat echo. 'Ordered ... started.'
    or 'Busy.' (a project is already admitted) means active; any refusal
    text means not. Dialog bodies are presentation-gated (empty headless)."""
    state = _qa._send(probe, order_cmd)
    lines = _qa._chat_lines(state, 12)
    echo = " | ".join(lines[:6])
    sub[f"L{level}_project_body"] = echo[:400]
    active = any("Ordered " in line and "started" in line for line in lines) or \
        any(line.strip() == "Busy." for line in lines)
    probe.check(f"{tag}_L{level}_project", active)
    return active


def _wait_certified_origin_here(probe: ProbeSteps, sub: dict[str, Any], tag: str, bp: str,
                                level: int, ctx: ProfileContext,
                                artifact: Path) -> tuple[int, int, int] | None:
    """Poll the listing while the player STAYS on site (active chunks)."""
    budget = probe.deadline - _time.monotonic() - 60.0
    fence = "farm" if bp in ("house", "barracks") else "house"
    deadline = _time.monotonic() + max(budget, 60.0)
    while _time.monotonic() < deadline:
        probe.call("minecraft_wait_ticks", {"ticks": 200})
        certificate = _qa._current_certificate(probe, sub, tag, bp, level)
        if certificate is None:
            continue
        view = _project_view(probe)
        if view is not None and view[2]:
            continue
        expected = PROJECT_LABEL.get(fence, fence.capitalize()).lower()
        fenced = view is not None and not view[2] and view[1] == 1 and view[0].lower() == expected
        probe.check(f"{tag}_L{level}_fence_waiting", fenced)
        # The listing flips on upgrade admission, not completion; check it
        # again after observing the queued fence.
        certificate = _qa._current_certificate(probe, sub, tag, bp, level)
        if certificate is None:
            continue
        origin = certificate[1]
        sub[f"L{level}_origin"] = list(origin)
        return origin
    return None


def _menu_click_named(probe: ProbeSteps, title: str, name: str) -> None:
    """Find an icon across container pages and click its exact name."""
    page = probe.wait(lambda s: _container_titled(s, title), f"{title}_menu_open",
                      timeout_seconds=60)
    while True:
        slots = page["container"]["slots"]
        match = next((row for row in slots if row.get("name") == name), None)
        if match is not None:
            probe.call("minecraft_click_container_slot", {"slot": int(match["slot"]), "button": "primary",
                                                          "timeout_seconds": probe.remaining()})
            return
        if not any(row.get("slot") == 50 and row.get("name") == "Next page" for row in slots):
            raise RuntimeError(f"buildings-qa: {name!r} missing from {title} menu")
        probe.call("minecraft_click_container_slot", {"slot": 50, "button": "primary",
                                                      "timeout_seconds": probe.remaining()})
        page = probe.wait(lambda s: _container_titled(s, title)
                          and s["container"]["slots"] != slots,
                          f"{title}_next_page", timeout_seconds=60)


def _confirm_project(probe: ProbeSteps, label: str) -> str:
    # The container icon opens a real server confirmation, not the retired
    # Projects dialog. Do not bypass that confirmation with a chat command.
    opened = probe.wait(lambda s: (s.get("screen") or {}).get("class") == DIALOG_CLASS
                        and (s.get("screen") or {}).get("title") == label,
                        "project_confirmation_open", timeout_seconds=60)
    probe.call("minecraft_click_screen_button", {"expected_screen_class": DIALOG_CLASS,
                                                "expected_title": opened["screen"]["title"],
                                                "button_label": label})
    result = probe.wait(
        lambda s: (s.get("screen") or {}).get("class", "").endswith("SimpleDialogScreen")
        and any(str(node.get("id", "")).startswith("dialog/body/") for node in s.get("ui_nodes", [])),
        "project_confirmation_result", timeout_seconds=60)
    return " | ".join(str(node.get("label", "")) for node in result.get("ui_nodes", [])
                      if str(node.get("id", "")).startswith("dialog/body/"))


def _order_via_dialog(probe: ProbeSteps, sub: dict[str, Any], tag: str,
                      bp: str, level: int, settlement: str) -> str:
    _qa._send(probe, "settlement menu projects")
    label = PROJECT_LABEL.get(bp, bp.capitalize())
    _qa._menu_click_named(probe, "Projects", f"{label} L1")
    result = _qa._confirm_project(probe, f"Order: {label}")
    sub[f"L{level}_dialog_order"] = result
    return result


def _upgrade_via_dialog(probe: ProbeSteps, sub: dict[str, Any], tag: str,
                        bp: str, bid: int, level: int, settlement: str) -> str:
    _qa._send(probe, "settlement menu buildings")
    kind = KIND_LABEL.get(bp, bp.capitalize())
    _qa._menu_click_named(probe, "Buildings", f"#{bid & 0xFFFFFFFF} {kind} L{level - 1}")
    screen = probe.wait(lambda s: _container_titled(s, f"#{bid & 0xFFFFFFFF} {kind}"),
                        f"{tag}_building_open", timeout_seconds=60)
    action = next((row for row in screen["container"]["slots"] if row.get("slot") == 52), None)
    label = f"Order construction L{level}"
    if not action or action.get("name") != label:
        raise RuntimeError(f"buildings-qa: expected {label!r}, got {action}")
    probe.call("minecraft_click_container_slot", {"slot": 52, "button": "primary",
                                                  "timeout_seconds": probe.remaining()})
    result = _qa._confirm_project(probe, f"{label} #{bid & 0xFFFFFFFF}")
    sub[f"L{level}_dialog_upgrade"] = result
    return result


def _wait_certified_origin(probe: ProbeSteps, sub: dict[str, Any], tag: str, bp: str,
                           level: int, budget: float, ctx: ProfileContext,
                           artifact: Path) -> tuple[int, int, int] | None:
    exclude = [sub["shell_bounds"][0]] if bp == "storage" and "shell_bounds" in sub else None
    deadline = _time.monotonic() + budget
    while _time.monotonic() < deadline:
        probe.call("minecraft_wait_ticks", {"ticks": 200})
        origin = _building_bounds(probe, bp, level, exclude=exclude)
        if origin is not None:
            if ctx.windowed:
                try:
                    native_client.capture(probe.client, artifact, f"{tag}-L{level}")
                except Exception:
                    pass
            sub[f"L{level}_origin"] = list(origin)
            return origin
    return None
