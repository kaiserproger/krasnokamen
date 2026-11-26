"""The landed sand is read only after the client reports it in the landing cell.

These are fake-step contracts, not real-client evidence: the final pin pair's
two failing runs read the landing cell in the same instant the due tick cleared
the source cell, when the falling block had not arrived yet.
"""
from __future__ import annotations

import unittest
from pathlib import Path
from unittest import mock

from .backends import native_ledger_gravity_tick as gravity

SAND, STONE, AIR = "minecraft:sand", "minecraft:stone", "minecraft:air"
# The two failing runs read sand 22 times and air on the 23rd read.
SAND_READS_BEFORE_FALL = 22
PLAYER = "90bc660c-7fd2-3a4c-95d6-3a97ff56611b"


class FakeClient:
    """The p4 export/import lifecycle: it runs the scene's export edit hook."""

    def __init__(self, steps: "FakeSteps") -> None:
        self.steps = steps
        self.export_edit = None

    def p4_restart_server(self, _remaining: float) -> str:
        self.export_edit(Path("/fixture/world-export-0"))
        return "127.0.0.1:25565"


class FakeSteps:
    def __init__(self, *, sand_can_land: bool = True) -> None:
        self.report = {"assertions": {}, "fixture_commands": []}
        self.calls: list[tuple[str, dict]] = []
        self.messages: list[str] = []
        self.sand_can_land = sand_can_land
        self.exported = False
        self.gap_has_sand = False
        self.placed_sand_reads = 0
        self.landing_waits = 0
        self.client = FakeClient(self)
        self.support = (195, 84, -445)
        self.gap = (195, 85, -445)
        self.placed = (195, 86, -445)

    def remaining(self, *, limit: float | None = 120.0) -> float:
        return 60.0

    def fixture(self, command: str) -> None:
        self.report["fixture_commands"].append(command)

    def check(self, name: str, condition: bool) -> None:
        self.report["assertions"][name] = bool(condition)
        if not condition:
            raise RuntimeError(f"{self.report.get('id', 'scene')} assertion failed: {name}")

    def observe(self) -> dict:
        return {"player": {"entity_uuid": PLAYER, "on_ground": True}, "state_version": 1,
                "in_play": True, "recent_chat": list(self.messages)}

    def wait(self, predicate, label: str, **_kwargs) -> dict:
        state = self.observe()
        self.check(label, bool(predicate(state)))
        return state

    def _state(self, key: tuple[int, int, int]) -> str:
        if key == self.support:
            return STONE
        if key == self.gap:
            if self.gap_has_sand:
                return SAND
            return AIR if self.exported else STONE
        if key == self.placed:
            return SAND if self.placed_sand_reads < SAND_READS_BEFORE_FALL else AIR
        raise AssertionError(f"unexpected fixture cell {key}")

    def _key(self, arguments: dict) -> tuple[int, int, int]:
        return (arguments["x"], arguments["y"], arguments["z"])

    def call(self, name: str, arguments: dict | None = None) -> dict:
        arguments = dict(arguments or {})
        self.calls.append((name, arguments))
        if name == "minecraft_read_block":
            key = self._key(arguments)
            if key == self.placed:
                self.placed_sand_reads += 1
            return {"block_id": self._state(key)}
        if name == "minecraft_wait_for_block_state":
            key, expected = self._key(arguments), arguments["block_id"]
            if key == self.gap and expected == SAND and not self.gap_has_sand:
                self.landing_waits += 1
                if not self.sand_can_land:
                    # The pinned client reports an expired wait as a tool error.
                    raise RuntimeError(
                        "client block did not reach expected state at 195, 85, -445")
                self.gap_has_sand = True
            state = self._state(key)
            if state != expected:
                raise AssertionError(f"wait expected {expected} at {key}, holds {state}")
            return {"status": "matched", "block": {"block_id": state}}
        if name == "minecraft_observe":
            return {"in_play": False}
        if name == "minecraft_wait_for_play":
            return {"in_play": True}
        if name == "minecraft_send_chat":
            self.messages.append(str(arguments.get("message", "")))
            return {}
        return {}


def _plant(steps: FakeSteps):
    def plant(_world, *, clear, tick, block, clear_block, delay, priority=0):
        steps.exported = True
        return {
            "region": "r.0.-1.mca",
            "chunk": [12, -28],
            "cleared": {"position": list(clear), "was": clear_block, "now": AIR,
                        "palette": [4, 0]},
            "tick": {"i": block, "x": tick[0], "y": tick[1], "z": tick[2],
                     "t": delay, "p": priority, "palette": 5},
            "block_ticks_before": 0,
        }
    return plant


class LedgerGravityTickLandingTests(unittest.TestCase):
    def run_scene(self, steps: FakeSteps) -> None:
        with mock.patch.object(gravity, "plant_due_gravity_tick", side_effect=_plant(steps)):
            gravity.run(steps, (192, 77, -448))

    def test_the_landing_cell_is_waited_for_before_it_is_read(self) -> None:
        steps = FakeSteps()
        self.run_scene(steps)
        self.assertEqual(steps.landing_waits, 1)
        self.assertIs(steps.report["assertions"]["saved_tick_moved_the_sand"], True)
        self.assertIs(steps.report["assertions"]["sand_landed_on_its_support"], True)
        self.assertEqual(steps.report["landing"]["block_id"], SAND)
        self.assertEqual(steps.report["landing_wait"]["status"], "matched")
        wait_index = next(index for index, (tool, args) in enumerate(steps.calls)
                          if tool == "minecraft_wait_for_block_state"
                          and steps._key(args) == steps.gap and args["block_id"] == SAND)
        read_index = next(index for index, (tool, args) in enumerate(steps.calls)
                          if tool == "minecraft_read_block" and steps._key(args) == steps.gap)
        # A reverted scene reads the gap in the removing tick, before this wait.
        self.assertLess(wait_index, read_index)

    def test_a_sand_that_never_lands_stays_a_red_assertion(self) -> None:
        steps = FakeSteps(sand_can_land=False)
        with self.assertRaisesRegex(RuntimeError, "sand_landed_on_its_support"):
            self.run_scene(steps)
        self.assertIs(steps.report["assertions"]["saved_tick_moved_the_sand"], True)
        self.assertIs(steps.report["assertions"]["sand_landed_on_its_support"], False)
        self.assertIn("landing_wait_error", steps.report)
        self.assertEqual(steps.report["landing"]["block_id"], AIR)


if __name__ == "__main__":
    unittest.main()
