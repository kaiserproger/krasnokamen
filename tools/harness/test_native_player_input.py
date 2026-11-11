"""The native sand gate against a vanilla-timed model of a frozen falling block."""
from __future__ import annotations

import ctypes
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from PIL import Image

from .backends import native_p1, native_player_input, native_v2

DRAG = ctypes.c_float(.98).value  # (double) 0.98f, Entity.getAirDrag


class FrozenFall:
    """Vanilla 26.3 timing for the gate's falling block.

    Server and client tick at the server tick rate, frozen or not. Stepped ticks
    move the block; ServerEntity.sendChanges runs on every server tick and resends
    a moved falling block every 20 server ticks (UpdateInterval 20), so updates
    follow the wall clock. Each harness request costs `latency` seconds. The client
    draws a frozen block at its previous predicted pose (the native client's
    partial-0 presentation), which the gate must accept along with vanilla's.
    `holds=False` models a server that keeps dropping the block while frozen.
    """

    def __init__(self, latency: float, phase: int, holds: bool = True) -> None:
        self.latency, self.phase, self.holds = latency, phase, holds
        self.now = self.next_tick = 0.
        self.rate, self.loop = 20., 0
        self.commands: list[str] = []
        self.chat: list[str] = []
        self.server_steps = self.client_steps = 0
        self.sand_pending = False
        self.server: list[float] | None = None  # [y, velocity]
        self.client: list[float] | None = None  # predicted [y, velocity]
        self.sent = self.received = self.presented = 0.
        self.tick = 0
        self.report: dict = {"assertions": {}, "fixture_commands": []}

    def advance(self, seconds: float) -> None:
        end = self.now + seconds
        while self.next_tick <= end:
            self.now = self.next_tick
            self.server_tick()
            self.next_tick += 1 / self.rate
        self.now = end

    def server_tick(self) -> None:
        for command in self.commands:
            words = command.split()
            if words[:2] == ["tick", "step"]:
                self.server_steps = self.client_steps = int(words[2])
            elif words[:2] == ["tick", "rate"]:
                self.rate = float(words[2])
            elif words[0] == "setblock":
                self.sand_pending = True
            elif words[:2] == ["data", "get"]:
                self.chat.append(f"Falling Block has the following entity data: [0.0d, {self.server[0]!r}d, 0.0d]"
                                 if self.server else "No entity was found")
        self.commands.clear()
        stepped = self.server_steps > 0
        self.server_steps -= stepped
        if stepped and self.sand_pending:
            self.sand_pending = False
            self.server, self.client = [107., 0.], [107., 0.]
            self.sent = self.received = self.presented = 107.
        elif self.server and (stepped or not self.holds):
            fall(self.server)
        if self.client_steps > 0:
            self.client_steps -= 1
            self.tick += 1
            if self.client:
                self.presented = self.client[0]
                fall(self.client)
        if self.server and (self.loop + self.phase) % 20 == 0 and self.server[0] != self.sent:
            self.sent = self.server[0]
            self.received = self.client[0] = round(self.sent * 4096) / 4096
        self.loop += 1

    def row(self) -> dict:
        return {"entity_uuid": "sand", "y": self.received, "presentation_tick": self.tick,
                "predicted_position": [0., self.client[0], 0.], "velocity": {"y": self.client[1]},
                "presented_position": [0., self.presented, 0.]}

    # ProbeSteps surface used by native_player_input.sand.
    def remaining(self) -> float:
        return 600.

    def fixture(self, command: str) -> None:
        self.advance(self.latency)
        self.report["fixture_commands"].append(command)
        self.commands.append(command)

    def call(self, name: str, arguments: dict | None = None) -> dict:
        self.advance(self.latency)
        if name == "minecraft_send_chat":
            self.chat.append(f"<HarnessEntities> {arguments['message']}")
            return {}
        if name == "minecraft_wait_for_visible_entity":
            while self.client is None:
                self.advance(self.latency)
            return self.row()
        return {"entities": [self.row()]} if self.client else {}

    def wait(self, predicate, label: str, *, timeout_seconds: float | None = None) -> dict:
        deadline = self.now + (timeout_seconds or 600.)
        while self.now < deadline:
            self.advance(self.latency)  # observe; the predicate's own requests add theirs
            state = {"recent_chat": list(self.chat)}
            if predicate(state):
                self.check(label, True)
                return state
        self.check(label, False)
        return {}

    def check(self, name: str, condition: bool) -> None:
        self.report["assertions"][name] = bool(condition)
        if not condition:
            raise RuntimeError(f"assertion failed: {name}")


def fall(body: list[float]) -> None:
    body[1] -= .04
    body[0] += body[1]
    body[1] *= DRAG


class SandGateTests(unittest.TestCase):
    def run_gate(self, latency: float, phase: int, holds: bool = True) -> FrozenFall:
        world = FrozenFall(latency, phase, holds)
        with tempfile.TemporaryDirectory() as directory:
            frames = []
            for color in ("red", "blue"):
                path = Path(directory) / f"{color}.png"
                Image.new("RGB", (2, 2), color).save(path)
                frames.append({"retained_path": path, "entity_extraction": {"entity_indices": 6},
                               "entity_geometry": {"gpu_completed_indices": 6}})
            descended = ("sand-descended", "sand-landed")
            with mock.patch.object(native_player_input, "_shot",
                                   lambda _steps, label: frames[label in descended]), \
                    mock.patch.object(native_player_input, "_echo"), \
                    mock.patch.object(native_p1, "_aim"), mock.patch.object(native_v2, "_block"):
                native_player_input.sand(world, (0, 77, 0))
        return world

    def test_window_fits_between_wall_clock_updates_on_a_slow_harness(self) -> None:
        # 0.25 s per request matches the failed receipts (about five client ticks).
        for latency in (.05, .25, .5):
            for phase in (0, 7, 13, 19):
                with self.subTest(latency=latency, phase=phase):
                    world = self.run_gate(latency, phase)
                    self.assertTrue(all(world.report["assertions"].values()))
                    self.assertIn("tick rate 20", world.report["fixture_commands"])

    def test_a_server_that_keeps_dropping_a_frozen_block_turns_the_gate_red(self) -> None:
        for latency in (.05, .25, .5):
            with self.subTest(latency=latency), \
                    self.assertRaisesRegex(RuntimeError, "sand_server_holds_frozen_block"):
                self.run_gate(latency, 7, holds=False)

    def test_prediction_check_is_vanilla_falling_block_integration(self) -> None:
        # Receipt 20261111T163719-native-v2-input-sand-2gazssgv: two client-predicted ticks.
        before = {"predicted_position": [192.5, 106.88076093673706, -444.5],
                  "velocity": {"y": -0.0776160022583008}}
        self.assertEqual(native_player_input._vanilla_fall(before, 2), 106.60788125002227)


if __name__ == "__main__":
    unittest.main()
