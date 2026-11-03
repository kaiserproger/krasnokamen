"""Real-client gate for Factorio-style settlement autocraft (issue #1, item 10).

Native client, isolated world, server from SOLARIS_SERVER_BINARY (release build).
The operator player levels a pad and certifies a storage hut (one chest) and a
cartographer workshop (cartography table + crafting table; a cartographer never
bakes or deposits). It binds one summoned villager and puts 3 hay blocks into
the chest through the chest menu. Then, only through the server-driven
/settlement dialogs (production page -> New order -> recipe button, with the
default quantity 8 and "Craft missing ingredients: ON"):

- saddle x8 is refused: missing 24 leather, a raw material. No job is created.
- hay_block x8 is refused: recipe cycle hay_block -> open_hay -> hay_block.
  No job is created.
- bread x8 is planned "open_hay x3 -> bread x8"; the bread step shows
  "waiting for ui-1-1".

The real villager then walks to storage and to the table. Owner readbacks
(`/settlement production autocraft job|carry`) prove ui-1-1 makes its 3 crafts
first. The server restarts between the steps, and the chain resumes without
duplicate receipts. The carry ends with exactly 8 bread and 3 wheat, and the
chest holds no hay. Re-ordering id ui-1 by command is refused.

Then, by owner command with one more stored hay block, order qa-cancel
(bread x4) plans "open_hay x1 -> bread x4". Once its dependency qa-cancel-1
is done, cancelling that finished step must cancel the unfinished bread step,
which then crafts nothing more while the finished step keeps its receipt.

Fixture commands (pad, blocks, /give, summon, bind) are recorded separately and
never count as behaviour evidence.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import time
import uuid
from pathlib import Path
from typing import Any

from .. import runtime
from ..profiles import ProfileContext
from . import native_client
from .army_qa import (
    _body,
    _bwait,
    _chest_totals,
    _deposit_all,
    _do_restart,
    _expect,
    _labels,
    _open_at,
    _open_chest,
    _pad_commands,
    _press,
    _villagers,
    _wait_title,
)
from .gameplay import peak_rss_kib, prepare_operator_fixture
from .probe_steps import ProbeSteps

USERNAME = "AutoCraftQA"
SETTLEMENT = "autocraft"
TIMEOUT = 900.0
HAY = 3
COUNT = 8  # the New order slider's initial quantity
CANCEL = "qa-cancel"
CANCEL_COUNT = 4  # carry's 3 wheat + one opened hay block (9) = 12 wheat


def _flat(text: str) -> str:
    """Dialog body with wrapping undone: wrapped lines join at spaces."""
    return " ".join(text.split())


def answer_after_marker(lines: list[str], marker: str, needle: str) -> str | None:
    """`recent_chat` is newest first; the answer is the oldest match newer than the marker."""
    start = next((i for i, line in enumerate(lines) if marker in line), None)
    if start is None:
        return None
    return next((line for line in reversed(lines[:start]) if needle in line), None)


def _readback(probe: ProbeSteps, command: str, needle: str) -> str:
    """Echo a chat marker, then send the owner command and wait for its answer.

    Command answers may arrive after a later chat echo (seen live on the first
    command after a reconnect), so the marker goes first and fences off every
    older line instead of the answer being expected before it.
    """
    marker = f"autocraft-qa {uuid.uuid4().hex[:8]}"
    probe.call("minecraft_send_chat", {"message": marker, "command": False})
    _bwait(probe, lambda s: any(marker in line for line in s.get("recent_chat", [])),
           f"marker_{needle[:24]}")
    probe.call("minecraft_send_chat", {"message": command, "command": True})
    state = _bwait(probe, lambda s: answer_after_marker(s.get("recent_chat", []), marker, needle)
                   is not None, f"readback_{needle[:24]}")
    return answer_after_marker(state.get("recent_chat", []), marker, needle)


def parse_json_line(line: str) -> dict[str, Any]:
    return json.loads(line[line.index("{"):])


def job_state(line: str) -> dict[str, Any]:
    """`job` readback -> the fields this gate asserts."""
    job = parse_json_line(line)
    return {"id": job["id"], "recipe": job["recipe"], "count": job["count"], "done": job["done"],
            "order": job.get("order"), "after": job.get("after"),
            "cancelled": job["cancelled"], "paused": job.get("paused"),
            "receipts": len(job.get("receipts", []))}


def carry_counts(line: str) -> dict[str, int]:
    """`carry` readback -> item id -> count over the eight carry slots."""
    totals: dict[str, int] = {}
    for slot in parse_json_line(line)["slots"]:
        if slot:
            totals[slot["item"]] = totals.get(slot["item"], 0) + int(slot["stack"]["count"])
    return totals


def building_id(lines: list[str], kind: str) -> int | None:
    for line in lines:
        match = re.search(rf"#(\d+) {kind} level", line)
        if match:
            return int(match.group(1))
    return None


def run(ctx: ProfileContext) -> dict[str, Any]:
    if ctx.client != "native":
        raise ValueError("autocraft-qa requires --client native")
    if ctx.extra_args or ctx.platform is not None:
        raise ValueError("autocraft-qa accepts only --client, --windowed and --timeout-seconds")
    timeout = ctx.timeout_seconds if ctx.timeout_seconds is not None else TIMEOUT
    if not 0 < timeout <= 3600:
        raise ValueError("autocraft-qa --timeout-seconds must be in (0, 3600]")
    artifact = ctx.artifact_dir
    override = os.environ.get("SOLARIS_SERVER_BINARY")
    if not override:
        raise RuntimeError("autocraft-qa requires SOLARIS_SERVER_BINARY (a solaris-v2 built from this tree)")
    binary = Path(override).expanduser().resolve()
    if not binary.is_file():
        raise RuntimeError(f"missing executable server binary: {binary}")
    server_port = runtime.reserve_port()
    address = f"127.0.0.1:{server_port}"
    config = artifact / "autocraft-qa.toml"
    source = (runtime.REPO_ROOT / "playable.toml").read_text()
    replacements = (
        ("port = 25565", f"port = {server_port}"),
        ('world_dir = ".analysis/test-world-v12"',
         f'world_dir = "{(artifact / "world").relative_to(runtime.REPO_ROOT).as_posix()}"'),
        ('directory = "plugins"', f"directory = {json.dumps(str(artifact / 'plugins'))}"),
        ("seed = 0", "seed = 81"),
    )
    for old_tok, new_tok in replacements:
        if source.count(old_tok) != 1:
            raise RuntimeError(f"expected one autocraft-qa config token: {old_tok}")
        source = source.replace(old_tok, new_tok, 1)
    config.write_text(source)
    # Empty plugin directory: no auth gate can time the client out mid-scenario.
    (artifact / "plugins").mkdir(parents=True, exist_ok=True)
    prepare_operator_fixture(ctx, [USERNAME])
    evidence = ctx.client_evidence
    with binary.open("rb") as server_executable:
        server_hash = hashlib.file_digest(server_executable, "sha256").hexdigest()
    evidence.update(server_address=address, server_binary_path=str(binary),
                    server_binary_sha256=server_hash, world_dir=str(artifact / "world"), seed=81)
    servers: list[Any] = []
    client_process = xvfb = mcp = None
    server_log = None
    server_log_path = artifact / "server.log"
    try:
        server_log = server_log_path.open("wb")
        env = os.environ.copy()
        env["SOLARIS_HARNESS_LOG_STDOUT"] = "1"
        servers.append(subprocess.Popen([str(binary), "--config", str(config), "--no-console"],
                                        cwd=runtime.REPO_ROOT, stdout=server_log,
                                        stderr=subprocess.STDOUT, start_new_session=True, env=env))
        ctx.logs.append(str(server_log_path))
        runtime.wait_port(server_port, timeout, servers[0], log_path=server_log_path)
        display = None
        if ctx.windowed:
            xvfb, display, _ = runtime.start_xvfb(artifact)
            ctx.logs.append(str(artifact / "xvfb.log"))
        client_process, _client_log, mcp = native_client.start(
            ctx, native_client.resolve_binary(ctx), address, USERNAME, timeout, display=display)
        if ctx.windowed:
            mcp.p4_window = evidence.get("window_focus")
        mcp.request_timeout_seconds = min(timeout + 10, 130.0)
        with ProbeSteps(mcp, artifact / "autocraft-qa", timeout, "autocraft-qa-settlements") as probe:
            probe.call("minecraft_wait_for_play", {"timeout_seconds": probe.remaining()})
            detail: dict[str, Any] = {}
            probe.report["autocraft"] = detail
            _scenario(probe, detail, ctx, artifact, address, config, server_log_path, servers)
        if ctx.windowed:
            evidence["capture"] = native_client.capture(mcp, artifact)
        if client_process is not None:
            rss = peak_rss_kib(client_process.pid)
            if rss is not None:
                evidence["peak_client_rss_kib"] = rss
        return {"scenario": probe.report}
    finally:
        if mcp is not None:
            try:
                mcp.close()
            except Exception:
                pass
        runtime.stop_process(client_process)
        for server in servers:
            runtime.stop_process(server, interrupt=True)
        runtime.stop_process(xvfb)
        if server_log is not None:
            server_log.close()


def _scenario(probe: ProbeSteps, detail: dict[str, Any], ctx: ProfileContext, artifact: Path,
              address: str, config: Path, server_log_path: Path, servers: list[Any]) -> None:
    def shot(name: str) -> None:
        if ctx.windowed:
            native_client.capture(probe.client, artifact, f"autocraft-qa-{name}")

    def job(name: str) -> dict[str, Any]:
        return job_state(_readback(probe, f"settlement production {SETTLEMENT} job {name}",
                                   f'"id":"{name}"'))

    def wait_done(name: str, count: int, limit: float) -> dict[str, Any]:
        deadline = time.monotonic() + limit
        while True:
            state = job(name)
            if state["done"] >= count or state["cancelled"]:
                return state
            if time.monotonic() >= deadline:
                raise RuntimeError(f"autocraft-qa WEDGED: {name} stuck at {state}")
            # Dialogs are closed here, so the client clock runs: wait on game
            # ticks, not wall-clock sleeps.
            probe.call("minecraft_wait_ticks", {"ticks": 40})

    rooted = _bwait(probe, lambda s: isinstance((s.get("player") or {}).get("x"), float),
                    "player_present")
    ox, y0, oz = (int(rooted["player"][axis] // 1) for axis in ("x", "y", "z"))
    detail["origin"] = [ox, y0, oz]
    for command in _pad_commands(ox, y0, oz):
        probe.fixture(command)
    for command in ("gamemode creative @s", "gamerule advance_time false",
                    "gamerule spawn_mobs false", "gamerule random_tick_speed 0", "time set 3000"):
        probe.fixture(command)
    _expect(probe, (ox + 6, y0, oz + 6), "minecraft:air", "pad_cleared", timeout=30.0)

    # Storage hut bounds hold one chest; the workshop holds its profession
    # station and the crafting table. Everything stands on the pad's grass.
    sx, sz = ox + 4, oz + 4
    chest = (sx + 2, y0, sz + 2)
    probe.fixture(f"setblock {chest[0]} {chest[1]} {chest[2]} "
                  "minecraft:chest[facing=north,type=single,waterlogged=false]")
    wx, wz = ox - 8, oz + 4
    table = (wx + 3, y0, wz + 3)
    probe.fixture(f"setblock {wx + 1} {y0} {wz + 1} minecraft:cartography_table")
    probe.fixture(f"setblock {table[0]} {table[1]} {table[2]} minecraft:crafting_table")
    _expect(probe, table, "minecraft:crafting_table", "table_placed")
    probe.fixture(f"settlement create {SETTLEMENT} 32")
    probe.fixture(f"settlement building certify storage {sx} {y0 - 1} {sz} {sx + 4} {y0 + 3} {sz + 4}")
    probe.fixture(f"settlement building certify cartographer {wx} {y0 - 1} {wz} {wx + 4} {y0 + 3} {wz + 4}")
    listing = _readback(probe, "settlement building list", "Buildings:")
    workshop = building_id([listing], "cartographer")
    detail["building_list"] = listing
    probe.check("storage_and_workshop_certified",
                workshop is not None and building_id([listing], "storage") is not None)

    # Real player stock path: /give, then shift-click into the chest menu.
    probe.fixture(f"give @s minecraft:hay_block {HAY}")
    _open_at(probe, chest)
    detail["deposit"] = _deposit_all(probe, chest, ["minecraft:hay_block"])
    stocked = _chest_totals(_open_chest(probe, chest))
    detail["chest_before"] = stocked
    probe.check("chest_holds_hay", stocked.get("minecraft:hay_block") == HAY)

    before = {str(row.get("entity_uuid")) for row in _villagers(probe)}
    player = probe.observe()["player"]
    probe.fixture(f"summon minecraft:villager {player['x']:.1f} {player['y']:.1f} {player['z']:.1f}")
    _bwait(probe, lambda s: any(str(row.get("entity_uuid")) not in before
                                for row in _villagers(probe)), "villager_summoned")
    worker = next(str(row["entity_uuid"]) for row in _villagers(probe)
                  if str(row.get("entity_uuid")) not in before)
    probe.fixture(f"settlement bind {SETTLEMENT} {worker}")
    detail["worker"] = worker

    # --- dialogs only from here on ---
    entry = probe.observe()
    probe.call("minecraft_send_chat", {"message": "settlement menu production", "command": True})
    _wait_title(probe, "Production orders", "orders_open", entry=entry)

    def new_order() -> dict[str, Any]:
        form = _press(probe, "New order")
        probe.check("new_order_form", (form["screen"].get("title") or "") == "New production order")
        return form

    form = new_order()
    labels = _labels(form)
    detail["form_labels"] = labels
    probe.check("autocraft_checkbox_on", "Craft missing ingredients: ON" in labels)
    probe.check(f"quantity_defaults_to_{COUNT}", f"Quantity: {COUNT}" in labels)
    shot("form")

    raw = _flat(_body(_press(probe, "saddle -> saddle x1")))
    detail["refused_raw"] = raw
    probe.check("raw_material_refused",
                "Refused: missing 24 minecraft:leather: not in storage and no recipe makes it." in raw)
    probe.check("raw_refusal_admits_nothing", "No production orders yet." in raw)

    new_order()
    cycle = _flat(_body(_press(probe, "hay_block -> hay_block x1")))
    detail["refused_cycle"] = cycle
    probe.check("cycle_refused",
                "missing 5 minecraft:hay_block: its recipes form a cycle "
                "(hay_block -> open_hay -> hay_block)" in cycle)
    probe.check("cycle_refusal_admits_nothing", "No production orders yet." in cycle)
    shot("refused")

    new_order()
    planned = _flat(_body(_press(probe, "bread -> bread x1")))
    detail["planned"] = planned
    probe.check("chain_planned", "Order ui-1 planned: open_hay x3 -> bread x8." in planned)
    probe.check("product_waits_for_dependency", "waiting for ui-1-1" in planned)
    shot("planned")
    _press(probe, "Close")

    # --- the real villager runs the chain ---
    first = job("ui-1")
    detail["product_admitted"] = first
    probe.check("product_gated_on_dependency",
                first["after"] == "ui-1-1" and first["order"] == "ui-1" and first["done"] == 0)
    dependency = wait_done("ui-1-1", 3, 300.0)
    detail["dependency_done"] = dependency
    probe.check("dependency_crafted_three_times",
                dependency["done"] == 3 and dependency["receipts"] == 3 and not dependency["cancelled"])
    released = job("ui-1")
    detail["product_released"] = released
    probe.check("product_released_after_dependency", released["after"] is None)

    servers.append(_do_restart(ctx, artifact, address, config, probe, server_log_path, servers[-1]))
    resumed = job("ui-1")
    detail["product_after_restart"] = resumed
    probe.check("restart_keeps_order_and_progress",
                resumed["order"] == "ui-1" and resumed["after"] is None
                and resumed["receipts"] == resumed["done"] >= released["done"])
    probe.check("restart_keeps_dependency_receipts", job("ui-1-1")["receipts"] == 3)

    product = wait_done("ui-1", COUNT, 600.0)
    detail["product_done"] = product
    probe.check("product_crafted_exactly", product["done"] == COUNT and product["receipts"] == COUNT)
    carried = carry_counts(_readback(probe, f"settlement production {SETTLEMENT} carry {worker}",
                                     '"worker"'))
    detail["carry_after"] = carried
    probe.check("carry_holds_exact_output",
                carried.get("minecraft:bread") == COUNT
                and carried.get("minecraft:wheat") == HAY * 9 - COUNT * 3
                and "minecraft:hay_block" not in carried)
    after = _chest_totals(_open_chest(probe, chest))
    detail["chest_after"] = after
    probe.check("storage_hay_consumed_once", "minecraft:hay_block" not in after)

    again = _readback(probe, f"settlement production {SETTLEMENT} order ui-1 {worker} "
                             f"solaris-settlements:bread {COUNT} {workshop} "
                             f"{table[0]},{table[1]},{table[2]} {table[0] - 1},{table[1]},{table[2]}",
                      "Autocraft")
    detail["reorder"] = again
    probe.check("reorder_refused", "Autocraft refused: order ui-1 already exists" in again)
    probe.check("reorder_admits_nothing", job("ui-1")["receipts"] == COUNT)

    # --- cancelling a finished dependency stops the rest of its order ---
    probe.fixture("give @s minecraft:hay_block 1")
    _open_at(probe, chest)
    detail["cancel_deposit"] = _deposit_all(probe, chest, ["minecraft:hay_block"])
    planned_cancel = _readback(
        probe, f"settlement production {SETTLEMENT} order {CANCEL} {worker} "
               f"solaris-settlements:bread {CANCEL_COUNT} {workshop} "
               f"{table[0]},{table[1]},{table[2]} {table[0] - 1},{table[1]},{table[2]}",
        "Autocraft")
    detail["cancel_planned"] = planned_cancel
    probe.check("cancel_chain_planned",
                f"Autocraft order {CANCEL} planned: open_hay x1 -> bread x{CANCEL_COUNT}"
                in planned_cancel)
    detail["cancel_dependency_done"] = wait_done(f"{CANCEL}-1", 1, 300.0)
    detail["cancel_answer"] = _readback(
        probe, f"settlement production {SETTLEMENT} cancel {CANCEL}-1", "operation processed")
    stopped = job(CANCEL)
    detail["cancel_product"] = stopped
    probe.check("finished_dependency_cancel_stops_product",
                stopped["cancelled"] and stopped["done"] < CANCEL_COUNT)
    # Two bread portions (120 ticks each); the native tool caps one wait at 255.
    probe.call("minecraft_wait_ticks", {"ticks": 240})
    later = job(CANCEL)
    detail["cancel_product_later"] = later
    probe.check("cancelled_product_crafts_nothing_more",
                later["done"] == stopped["done"] and later["receipts"] == stopped["receipts"])
    probe.check("finished_dependency_keeps_receipt", job(f"{CANCEL}-1")["receipts"] == 1)
    shot("final")
    (artifact / "autocraft-qa").mkdir(parents=True, exist_ok=True)
    (artifact / "autocraft-qa" / "result.json").write_text(json.dumps(detail, indent=2) + "\n")
