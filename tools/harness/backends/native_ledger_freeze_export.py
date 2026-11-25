"""A frozen world survives the offline Anvil round trip and reaches play.

The row's failing artifact (`20261125T063246-native-v2-ledger-gravity-tick-f4v0b_n_`):
`/tick freeze`, a floating sand block, `save-all`, clean stop, `world export-anvil`
and `world import-anvil` (both reported PASS), then a restart on the imported
store. That process logged `stored chunk unavailable; repair required` for 81
chunks and quarantined them every checkpoint, so the client never entered play.
The export wrote each chunk's simulation-debt anchor (`KrasnokamenSimHeldTicks`,
here 11 at last simulated tick 251) but not the clock's held prefix, so the
imported store restarted that prefix at zero and every anchored chunk became
unservable.

This gate keeps the original fixture as a real-client scenario: the freeze holds
a floating sand block up, the enclosing harness performs the real offline
export/import on the stopped private world (`convert_world`) and restarts on the
imported store, and the reconnected client must

* enter play (`reimport_entered_play`, the assertion that failed before),
* find no quarantine line in the restarted server's log
  (`reimport_no_quarantined_chunks`), and
* read the held sand landed one block below its placed position once the freeze
  is gone (`reimport_sand_fell_one_block`), which also proves the chunk that
  carried the anchor is genuinely simulated rather than merely unquarantined.

A frozen world pushes no block change into the client's model, so the placement
itself is not asserted before the restart; the landing after it carries the
fixture, as the row's notes record.
"""
from __future__ import annotations

from pathlib import Path

from .native_v2 import _block, _echo
from .probe_steps import ProbeSteps

QUARANTINE_MARKERS = ("stored chunk unavailable", "checkpoint skipped quarantined chunks")


def _server_log(steps: ProbeSteps) -> str:
    path = getattr(steps.client, "server_log_path", None)
    if path is None:
        raise RuntimeError("the frozen export gate needs the harness server log path")
    return Path(path).read_text(errors="replace")


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    if not callable(getattr(steps.client, "p4_restart_server", None)):
        raise RuntimeError(
            "the frozen export gate needs the p4_restart_server export/import lifecycle hook")
    x, y, z = origin
    # High enough above the pad that the gap cell is open sky; the support is
    # asserted on the client, so a failed placement cannot fake the landing.
    support = {"x": x + 3, "y": y + 8, "z": z + 3}
    held = {"x": x + 3, "y": y + 9, "z": z + 3}
    placed = {"x": x + 3, "y": y + 10, "z": z + 3}
    steps.fixture(f"setblock {support['x']} {support['y']} {support['z']} minecraft:stone")
    _block(steps, support, "minecraft:stone")
    steps.fixture(f"setblock {held['x']} {held['y']} {held['z']} minecraft:air")
    steps.fixture("tick freeze")
    _echo(steps, "frozen_export_freeze_ordered")
    steps.check("frozen_export_world_frozen",
                steps.observe().get("minecraft", {}).get("server_frozen") is True)
    # A gravity block placed with open air below becomes a falling entity in the
    # same (frozen) tick: the placed cell is already air, which is why the landing
    # carries the fixture.
    steps.fixture(f"setblock {placed['x']} {placed['y']} {placed['z']} minecraft:sand")
    steps.fixture("save-all")
    _echo(steps, "frozen_export_saved")
    steps.call("minecraft_disconnect")
    steps.check("frozen_export_client_disconnected",
                steps.call("minecraft_observe")["in_play"] is False)
    # The harness hook stops the server cleanly, exports the private world,
    # imports it and starts the server on the imported store.
    address = steps.client.p4_restart_server(steps.remaining())
    steps.report["frozen_export_address"] = address
    steps.call("minecraft_connect", {"server_addr": address})
    play = steps.call("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
    steps.check("reimport_entered_play", play["in_play"] is True)
    log = _server_log(steps)
    steps.report["server_log_bytes"] = len(log)
    steps.report["quarantine_lines"] = [line for line in log.splitlines()
                                        if any(marker in line for marker in QUARANTINE_MARKERS)]
    steps.check("reimport_no_quarantined_chunks", not steps.report["quarantine_lines"])
    # The freeze is session state (like vanilla): the imported world runs, so the
    # block the freeze held falls one block onto its fixture support.
    landed = _block(steps, held, "minecraft:sand")
    steps.report["landed"] = {**landed}
    source = steps.call("minecraft_read_block", {**placed})
    steps.report["placed_now"] = {**source}
    steps.check("reimport_sand_fell_one_block",
                landed["block_id"] == "minecraft:sand"
                and source["block_id"] == "minecraft:air")
    _echo(steps, "frozen_export_landed_ordered")
