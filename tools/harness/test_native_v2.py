"""Static/fail-closed native-v2 probe tests; these are not real-client evidence."""
from __future__ import annotations

import json
import os
import socket
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import scenarios
from .backends import native_v2
from .backends.probe_steps import ProbeSteps


class NativeV2ProbeTests(unittest.TestCase):
    def test_destroyed_chest_scenario_is_graphical_and_registered(self) -> None:
        name = "native-v2-destroyed-chest"
        self.assertIn(name, native_v2.RUNNERS)
        entry = scenarios.SCENARIOS[name]
        self.assertEqual(entry.evidence, "graphical")
        self.assertEqual(entry.profile, "entities")
        self.assertIn("tools/harness/backends/native_custody.py", entry.sources)

    def test_new_core_blocker_scenarios_are_runnable_catalog_entries(self) -> None:
        for name in ("native-v2-destroyed-chest-race", "native-v2-duplicate-login",
                     "native-v2-corrupt-block-entities", "native-v2-corrupt-chunk-near-camera"):
            self.assertIn(name, native_v2.RUNNERS)
            self.assertEqual(scenarios.SCENARIOS[name].evidence, "graphical")

    def test_lifecycle_difficulty_uses_only_fresh_fenced_responses(self) -> None:
        from .backends.native_lifecycle import _difficulty_from_interval

        begin, end = "begin abc", "end abc"
        cases = [
            ([end, "The difficulty is Easy", begin], "easy"),
            (["The difficulty is Easy", end, begin], "easy"),
            ([end, begin, "The difficulty is Easy"], None),
            ([end, begin], None),
            ([end, "You do not have permission", begin], None),
            (["The difficulty is Easy", begin], None),
            (["The difficulty is Easy", end], None),
        ]
        for lines, expected in cases:
            with self.subTest(lines=lines):
                self.assertEqual(_difficulty_from_interval(lines, begin, end), expected)

    def test_standard_inventory_fixture_does_not_assume_auxiliary_equipment(self) -> None:
        auxiliary = [{"slot": 41, "count": None, "state": "unknown"},
                     {"slot": 42, "count": None, "state": "unknown"}]
        self.assertTrue(native_v2._standard_inventory_empty({"inventory": auxiliary}))
        for ordinary in ({"slot": 0, "count": None, "state": "unknown"},
                         {"slot": 40, "count": None, "state": "unknown"},
                         {"slot": 9, "count": 1, "item_id": "minecraft:stone"},
                         {"count": None, "state": "unknown"}):
            with self.subTest(ordinary=ordinary):
                self.assertFalse(native_v2._standard_inventory_empty(
                    {"inventory": [*auxiliary, ordinary]}))

    def test_named_scenarios_route_existing_native_profile(self) -> None:
        self.assertEqual(set(native_v2.RUNNERS), set(native_v2.NATIVE_SCENARIO_IDS))
        for name in native_v2.NATIVE_SCENARIO_IDS:
            entry = scenarios.SCENARIOS[name]
            self.assertEqual(entry.profile, "entities")
            self.assertEqual(entry.scenario_id, name)


    def test_physical_input_catalog_routes_all_four_probes(self) -> None:
        for kind in ("menus", "pick", "sand", "camera"):
            name = f"native-v2-input-{kind}"
            with self.subTest(name=name):
                self.assertIn(name, native_v2.NATIVE_SCENARIO_IDS)
                self.assertIs(native_v2.RUNNERS[name], native_v2._player_input)
                self.assertEqual(scenarios.SCENARIOS[name].evidence, "graphical")

    def test_p1_catalog_covers_all_five_surfaces(self) -> None:
        expected = {f"native-v2-p1-{name}" for name in ("armor-stand", "signs", "piglin", "trader", "leaves")}
        self.assertTrue(expected <= native_v2.NATIVE_SCENARIO_IDS)
        self.assertTrue(expected <= native_v2.RUNNERS.keys())

    def test_sign_wire_varint_waits_for_complete_prefix(self) -> None:
        from .backends.native_p1 import _varint

        self.assertIsNone(_varint(b""))
        self.assertIsNone(_varint(b"\x80"))
        self.assertEqual(_varint(b"\xac\x02"), (300, 2))
        with self.assertRaisesRegex(ValueError, "oversized"):
            _varint(b"\xff" * 5)

    def test_p2_catalog_covers_all_four_surfaces(self) -> None:
        expected = {f"native-v2-p2-{name}" for name in ("slime", "enderman", "fishing", "creaking")}
        self.assertTrue(expected <= native_v2.NATIVE_SCENARIO_IDS)
        self.assertTrue(expected <= native_v2.RUNNERS.keys())

    def test_fishing_trace_requires_exact_pinned_boolean_tail(self) -> None:
        from .backends.native_p1 import _hook_biting

        self.assertEqual(_hook_biting(b"\xac\x02\x08\x01\x00\x09\x08\x01\xff"), (300, True))
        self.assertEqual(_hook_biting(b"\x02\x08\x01\x00\x09\x08\x00\xff"), (2, False))
        self.assertIsNone(_hook_biting(b"\x02\x09\x03\x00\x00\x80\x3f\xff"))
        self.assertIsNone(_hook_biting(b"\x02\x09\x08\x01"))

    def test_p4_catalog_covers_requested_surfaces(self) -> None:
        from .backends import native_p4

        expected = {f"native-v2-p4-{name}" for name in
                    ("nameplates", "advancements", "damage", "identities", "death-protection",
                     "hopper", "statistics")}
        self.assertTrue(expected <= native_v2.NATIVE_SCENARIO_IDS)
        self.assertTrue(expected <= native_v2.RUNNERS.keys())
        self.assertEqual(native_p4.RESTART_SCENARIOS,
                         {"native-v2-p4-nameplates", "native-v2-p4-advancements",
                          "native-v2-p4-statistics"})

    def test_damage_gamerule_fixture_requires_supported_rule_and_ack(self) -> None:
        from .backends.native_p4 import _disable_gamerule

        for rule in ("natural_health_regeneration", "spawn_mobs"):
            with self.subTest(rule=rule):
                steps = mock.Mock()
                _disable_gamerule(steps, rule)
                steps.fixture.assert_called_once_with(f"gamerule {rule} false")
                predicate, label = steps.wait.call_args.args
                self.assertEqual(label, f"{rule}_disabled")
                self.assertFalse(predicate({"recent_chat": ["Unknown game rule"]}))
                self.assertFalse(predicate({"recent_chat": [f"Game rule {rule} is now set to true"]}))
                self.assertTrue(predicate({"recent_chat": [f"Game rule {rule} is now set to false"]}))

    def test_damage_scenario_disables_mob_spawning_before_the_night(self) -> None:
        from .backends import native_p4

        events: list[str] = []
        steps = mock.MagicMock()
        steps.fixture.side_effect = events.append
        with mock.patch.object(native_p4, "_capture", side_effect=RuntimeError("stop")), \
                self.assertRaises(RuntimeError):
            native_p4._damage(steps, (10, 70, 10))
        self.assertLess(events.index("gamerule spawn_mobs false"), events.index("time set 18000"))

    def test_damage_scenario_pins_clear_weather_before_its_rows(self) -> None:
        from .backends import native_p4

        events: list[str] = []
        steps = mock.MagicMock()
        steps.fixture.side_effect = events.append
        with mock.patch.object(native_p4, "_capture", side_effect=RuntimeError("stop")), \
                self.assertRaises(RuntimeError):
            native_p4._damage(steps, (10, 70, 10))
        self.assertIn("gamerule advance_weather false", events)
        self.assertLess(events.index("gamerule spawn_mobs false"),
                        events.index("kill @e[type=!minecraft:player]"))
        self.assertLess(events.index("kill @e[type=!minecraft:player]"), events.index("time set 18000"))
        self.assertLess(events.index("weather clear"), events.index("time set 18000"))

    def test_damage_void_row_observes_outside_play_while_the_player_hangs_below_the_world(self) -> None:
        from .backends import native_p4

        class Stop(Exception):
            pass

        seen: dict[str, object] = {}

        def wait(_predicate, label, **kwargs):
            if label == "thorns_authored_uuid_received":
                self.assertTrue(_predicate({}))
            if label == "void_physical_identity_received":
                seen.update(kwargs)
                raise Stop
            return mock.MagicMock()

        row = {"entity_uuid": "00000000-0000-0000-0000-00000001e240", "entity_id": 7,
               "entity_type": "minecraft:zombie"}
        steps = mock.MagicMock()
        steps.wait.side_effect = wait
        with mock.patch.object(native_p4, "_rows", return_value=[row]), self.assertRaises(Stop):
            native_p4._damage_environment(steps, (10, 70, 10), 1)
        self.assertIs(seen.get("outside_play"), True)
        self.assertNotIn("wait_for_play", seen)

    def test_damage_rows_wait_for_the_douse_water_to_drain_before_their_fixture(self) -> None:
        from .backends import native_p4

        class Stop(Exception):
            pass

        events: list[str] = []
        steps = mock.MagicMock()
        steps.fixture.side_effect = events.append
        steps.call.return_value = {}

        def wait(predicate, label, **_kwargs):
            events.append(f"wait:{label}")
            if label == "douse_water_drained":
                steps.call.return_value = {"blocks": [{"fluid": {"empty": False}}]}
                self.assertFalse(predicate({}))
                steps.call.return_value = {"blocks": [{"fluid": {"empty": True}}]}
                self.assertTrue(predicate({}))
                steps.call.return_value = {}
            if label == "in_wall_fixture_ready":
                raise Stop
            return mock.MagicMock()

        steps.wait.side_effect = wait
        with self.assertRaises(Stop):
            native_p4._damage_environment(steps, (10, 70, 10), 1)
        air = events.index("fill 9 70 9 11 73 11 minecraft:air")
        self.assertLess(air, events.index("wait:douse_water_drained"))
        self.assertLess(events.index("wait:douse_water_drained"), events.index("tp @s 10.5 70 10.5"))

    def test_damage_thorns_row_resets_the_night_before_summoning_its_zombie(self) -> None:
        from .backends import native_p4

        class Stop(Exception):
            pass

        events: list[str] = []
        steps = mock.MagicMock()
        steps.fixture.side_effect = events.append

        def wait(_predicate, label, **_kwargs):
            if label == "thorns_authored_uuid_received":
                raise Stop
            return mock.MagicMock()

        steps.wait.side_effect = wait
        with self.assertRaises(Stop):
            native_p4._damage_environment(steps, (10, 70, 10), 1)
        summon = next(i for i, event in enumerate(events) if event.startswith("summon minecraft:zombie"))
        self.assertEqual("time set 18000", events[summon - 1])

    def test_damage_thorns_row_lets_the_survival_mode_land_before_the_hit(self) -> None:
        from .backends import native_p4

        class Stop(Exception):
            pass

        events: list[str] = []
        steps = mock.MagicMock()
        steps.fixture.side_effect = events.append

        def call(tool, args=None):
            events.append(f"{tool}:{(args or {}).get('ticks')}")
            if tool == "minecraft_attack_entity_once":
                raise Stop
            return {}

        def wait(predicate, label, **_kwargs):
            if label == "thorns_authored_uuid_received":
                predicate({})
            return mock.MagicMock()

        steps.call.side_effect = call
        steps.wait.side_effect = wait
        row = {"entity_uuid": "00000000-0000-0000-0000-00000001e240", "entity_id": 7,
               "entity_type": "minecraft:zombie"}
        with mock.patch.object(native_p4, "_rows", return_value=[row]), self.assertRaises(Stop):
            native_p4._damage_environment(steps, (10, 70, 10), 1)
        attack = events.index("minecraft_attack_entity_once:None")
        survival = max(i for i, event in enumerate(events) if event == "gamemode survival @s")
        self.assertEqual(events[survival + 1], "minecraft_wait_ticks:10")
        self.assertEqual(survival + 2, attack)

    def test_damage_thorns_row_returns_the_player_to_the_origin_before_the_reach_bound_attack(self) -> None:
        from .backends import native_p4

        class Stop(Exception):
            pass

        events: list[str] = []
        steps = mock.MagicMock()
        steps.fixture.side_effect = events.append

        def wait(predicate, label, **_kwargs):
            events.append(f"wait:{label}")
            if label == "thorns_fixture_ready":
                self.assertTrue(predicate({"player": {"health": 20, "x": 10.5, "z": 10.5}}))
                self.assertFalse(predicate({"player": {"health": 20, "x": 12.8, "z": 10.5}}))
                raise Stop
            return mock.MagicMock()

        steps.wait.side_effect = wait
        with self.assertRaises(Stop):
            native_p4._damage_environment(steps, (10, 70, 10), 1)
        authored = events.index("wait:thorns_authored_uuid_received")
        self.assertIn("tp @s 10.5 70 10.5", events[authored:events.index("wait:thorns_fixture_ready")])

    def test_damage_identity_timeout_reports_the_wire_events_it_saw(self) -> None:
        from .backends.native_p4 import _wait_damage_identity

        steps = mock.MagicMock()
        steps.report = {}
        steps.wait.side_effect = TimeoutError("late")
        steps.client.p4_damage_wire.return_value = [{"type": "minecraft:magic"}, {"type": "minecraft:on_fire"}]
        with self.assertRaises(TimeoutError):
            _wait_damage_identity(steps, lambda state: False, "splash", 1, timeout_seconds=3)
        self.assertEqual(steps.report["damage_wire_on_timeout"]["splash"], [{"type": "minecraft:on_fire"}])

    def test_damage_warden_lethal_prep_accepts_a_player_the_sonic_boom_already_killed(self) -> None:
        from .backends import native_p4

        class Stop(Exception):
            pass

        captured = {}

        def wait(predicate, label, **_kwargs):
            if label == "warden_lethal_health_prepared":
                captured["predicate"] = predicate
                raise Stop
            return {"recent_chat": ["x has the following entity data: minecraft:speed"],
                    "player": {"health": 1, "entity_id": 1}, "client_tick": 1,
                    "screen": {"open": True}, "in_play": True}

        steps = mock.MagicMock()
        steps.wait.side_effect = wait
        with mock.patch.object(native_p4, "_capture"), self.assertRaises(Stop):
            native_p4._damage(steps, (10, 70, 10))
        predicate = captured["predicate"]
        self.assertTrue(predicate({"player": {"health": 7.0}}))
        self.assertTrue(predicate({"player": {"health": 0.0}}))
        self.assertFalse(predicate({"player": {"health": 20.0}}))

    def test_damage_scenario_stops_the_zombie_before_slow_capture(self) -> None:
        from .backends import native_p4

        events: list[tuple[str, str]] = []
        steps = mock.MagicMock()
        steps.fixture.side_effect = lambda command: events.append(("fixture", command))
        steps.call.side_effect = lambda tool, _args: events.append(("call", tool))

        def wait(_predicate, label, **_kwargs):
            events.append(("wait", label))
            return mock.MagicMock()

        steps.wait.side_effect = wait

        class Stop(Exception):
            pass

        def capture(_steps: object, name: str) -> None:
            events.append(("capture", name))
            if name == "p4-damage-hit":
                raise Stop

        with mock.patch.object(native_p4, "_capture", capture), self.assertRaises(Stop):
            native_p4._damage(steps, (10, 70, 10))

        hit = events.index(("call", "minecraft_wait_for_health_below"))
        tilt = events.index(("wait", "received_damage_event_nonzero_camera_tilt"))
        kill = events.index(("fixture", f"kill {native_p4._DAMAGE_ZOMBIE_UUID}"))
        # The hurt tilt fades within ten client ticks: it is observed first, the kill follows
        # at once, and both precede every slow capture.
        self.assertEqual((tilt, kill), (hit + 1, hit + 2))
        self.assertLess(kill, events.index(("capture", "p4-damage-hit")))

    def test_java_launcher_validates_platform_before_launch(self) -> None:
        from . import runtime

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                mcp_port = listener.getsockname()[1]
            args = dict(game_dir=root / "game", log_path=root / "client.log",
                        token="unit-test-only", mcp_port=mcp_port,
                        username="HarnessUnit", display=":99")
            with self.assertRaisesRegex(ValueError, "platform"):
                runtime.start_client(**args, platform="invalid")
            with mock.patch.dict(os.environ, {"KRASNOKAMEN_LOADER_ROOT": str(root)}):
                with self.assertRaisesRegex(RuntimeError, "missing executable Gradle wrapper"):
                    runtime.start_client(**args, platform="fabric")

    def test_v2_operator_fixture_uses_observed_offline_uuid(self) -> None:
        from .backends.gameplay import prepare_operator_fixture

        with tempfile.TemporaryDirectory() as directory:
            ctx = mock.Mock(artifact_dir=Path(directory), client_evidence={})
            prepare_operator_fixture(ctx, ["HarnessSurvival"])
            rows = json.loads((Path(directory) / "ops.json").read_text())
            self.assertEqual(rows, [{
                "uuid": "92fea5d4-988a-3331-8b06-2c9858d14b0d", "name": "HarnessSurvival",
                "level": 4, "bypassesPlayerLimit": False,
            }])
            self.assertEqual(ctx.client_evidence["operator_fixture"], rows)


    def test_probe_observe_outside_play_returns_the_state_only_when_declared(self) -> None:
        client = mock.Mock(token="")
        client.call_tool.return_value = {"in_play": False, "state_version": 3}
        with tempfile.TemporaryDirectory() as directory:
            with ProbeSteps(client, Path(directory), 1, "outside") as steps:
                self.assertEqual(steps.observe(outside_play=True)["state_version"], 3)
                with self.assertRaisesRegex(RuntimeError, "in_play"):
                    steps.observe()

    def test_default_probe_observe_stays_strict_outside_play(self) -> None:
        client = mock.Mock(token="")
        client.call_tool.return_value = {"in_play": False}
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "in_play"):
                with ProbeSteps(client, Path(directory), 1, "strict") as steps:
                    steps.observe()
        self.assertEqual([call.args[0] for call in client.call_tool.call_args_list],
                         ["minecraft_observe"])

    def test_transition_wait_awaits_presented_play_after_state_change(self) -> None:
        client = mock.Mock(token="")
        client.call_tool.side_effect = [
            {"in_play": True, "state_version": 1, "dimension": "minecraft:overworld"},
            {}, {"in_play": False, "state_version": 2}, {"in_play": True},
            {"in_play": True, "state_version": 3, "dimension": "minecraft:the_nether"},
        ]
        with tempfile.TemporaryDirectory() as directory:
            with ProbeSteps(client, Path(directory), 1, "transition") as steps:
                state = steps.wait(lambda value: value["dimension"] == "minecraft:the_nether",
                                   "arrived", wait_for_play=True)
                self.assertEqual(state["dimension"], "minecraft:the_nether")
        self.assertEqual([call.args[0] for call in client.call_tool.call_args_list], [
            "minecraft_observe", "minecraft_wait_for_state_change", "minecraft_observe",
            "minecraft_wait_for_play", "minecraft_observe",
        ])

    def test_transition_play_timeout_remains_failed_evidence(self) -> None:
        client = mock.Mock(token="")
        client.call_tool.side_effect = [{"in_play": False}, TimeoutError("world never rebuilt")]
        with tempfile.TemporaryDirectory() as directory:
            artifact = Path(directory)
            with self.assertRaisesRegex(TimeoutError, "world never rebuilt"):
                with ProbeSteps(client, artifact, 1, "transition-timeout") as steps:
                    steps.observe(wait_for_play=True)
            self.assertFalse(json.loads((artifact / "scenario.json").read_text())["passed"])

    def test_title_preplay_wait_and_opt_in_play_observer_share_signature(self) -> None:
        from .backends.title_connect import _TitleProbeSteps

        client = mock.Mock(token="")
        client.call_tool.side_effect = [
            {"in_play": False, "screen": {"class": "TitleScreen"}},
            {"in_play": False}, {"in_play": True}, {"in_play": True},
        ]
        with tempfile.TemporaryDirectory() as directory:
            with _TitleProbeSteps(client, Path(directory), 1, "title-observer") as steps:
                steps.wait(lambda state: state["screen"]["class"] == "TitleScreen", "title")
                self.assertTrue(steps.observe(wait_for_play=True)["in_play"])
        self.assertEqual([call.args[0] for call in client.call_tool.call_args_list], [
            "minecraft_observe", "minecraft_observe", "minecraft_wait_for_play", "minecraft_observe",
        ])


if __name__ == "__main__":
    unittest.main()
