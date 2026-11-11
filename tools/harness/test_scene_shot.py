import json
import math
import socket
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.harness.backends import scene_shot


class SceneShotTests(unittest.TestCase):
    def test_server_identity_is_recorded_at_launch(self):
        import hashlib
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifact = root / 'artifact'
            artifact.mkdir()
            binary = root / 'server'
            binary.write_bytes(b'scene-server')
            binary.chmod(0o755)
            (root / 'playable.toml').write_text(
                (scene_shot.runtime.REPO_ROOT / 'playable.toml').read_text())
            ctx = scene_shot.ProfileContext(artifact, client='native', windowed=True)
            (root / 'template').mkdir()
            env = {'SOLARIS_SERVER_BINARY': str(binary), 'XDG_DATA_HOME': str(root / 'xdg')}
            from tools.harness import preflight
            with patch.dict('os.environ', env), patch.object(scene_shot.runtime, 'REPO_ROOT', root), \
                    patch.object(scene_shot, 'prepare_operator_fixture'), \
                    patch.object(preflight, 'client_template', return_value=root / 'template'), \
                    patch.object(scene_shot.subprocess, 'Popen', side_effect=RuntimeError('launch stopped')):
                with self.assertRaisesRegex(RuntimeError, 'launch stopped'):
                    scene_shot.run(ctx, scenario={'steps': []})
            self.assertEqual(ctx.commands[0][0], str(binary))
            self.assertEqual(ctx.client_evidence['server_binary_path'], str(binary))
            self.assertEqual(ctx.client_evidence['server_binary_sha256'],
                             hashlib.sha256(b'scene-server').hexdigest())

    def test_scenario_username(self):
        self.assertEqual(scene_shot.scene_username({}), "SceneShot")
        self.assertEqual(scene_shot.scene_username({"username": "Other_Player"}), "Other_Player")
        for name in ("", "bad name", "a" * 17, 42):
            with self.subTest(name=name), self.assertRaises(ValueError):
                scene_shot.scene_username({"username": name})

    def test_latency_proxy_delays_both_directions(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        def echo():
            with listener:
                connection, _ = listener.accept()
                with connection:
                    connection.sendall(connection.recv(32))
        thread = threading.Thread(target=echo)
        thread.start()
        proxy = scene_shot.LatencyProxy(listener.getsockname()[1], 75)
        try:
            port = proxy.start()
            with socket.create_connection(("127.0.0.1", port), timeout=5) as client:
                started = time.monotonic()
                client.sendall(b"ping")
                self.assertEqual(client.recv(4), b"ping")
                self.assertGreaterEqual(time.monotonic() - started, 0.14)
        finally:
            proxy.close()
            thread.join()

    def test_proxy_eof_is_a_clean_half_close(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        def echo():
            with listener:
                connection, _ = listener.accept()
                with connection:
                    connection.sendall(connection.recv(32))
                    connection.recv(32)
        thread = threading.Thread(target=echo)
        thread.start()
        proxy = scene_shot.LatencyProxy(listener.getsockname()[1], 0)
        try:
            port = proxy.start()
            with socket.create_connection(("127.0.0.1", port), timeout=5) as client:
                client.sendall(b"ping")
                self.assertEqual(client.recv(4), b"ping")
                proxy.send_eof()
                self.assertEqual(client.recv(1), b"")
        finally:
            proxy.close()
            thread.join()

    def test_proxy_eof_closes_every_isolated_connection(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        def echo(connection):
            with connection:
                connection.sendall(connection.recv(32))
                connection.recv(32)
        def serve():
            with listener:
                workers = [threading.Thread(target=echo, args=(listener.accept()[0],)) for _ in range(2)]
                for worker in workers:
                    worker.start()
                for worker in workers:
                    worker.join()
        thread = threading.Thread(target=serve)
        thread.start()
        proxy = scene_shot.LatencyProxy(listener.getsockname()[1], 0)
        try:
            port = proxy.start()
            with socket.create_connection(("127.0.0.1", port), timeout=5) as first, \
                    socket.create_connection(("127.0.0.1", port), timeout=5) as second:
                for client in (first, second):
                    client.sendall(b"ping")
                for client in (first, second):
                    self.assertEqual(client.recv(4), b"ping")
                proxy.send_eof()
                for client in (first, second):
                    self.assertEqual(client.recv(1), b"")
        finally:
            proxy.close()
            thread.join()

    def test_walk_phase_rejects_idle_missing_and_wrong_phase(self):
        spec = {"name": "walk", "entity_type": "minecraft:zombie", "phase": 0.0,
                "tolerance": 0.25, "min_speed": 0.1, "max_ticks": 30}
        step = {"walk_capture": spec}
        scene_shot.validate_steps([step], "native")
        scene_shot.validate_steps([step], "java")
        entity = {"entity_type": "minecraft:zombie", "walk_position": 0.1, "walk_speed": 0.2}
        self.assertIs(scene_shot.matching_walk({"entities": [entity]}, spec), entity)
        for values in ({"walk_speed": 0.0}, {"walk_position": None}, {"walk_position": 1.0},
                       {"walk_position": math.tau + 0.1}):
            self.assertIsNone(scene_shot.matching_walk({"entities": [{**entity, **values}]}, spec))
        self.assertIsNone(scene_shot.matching_walk({"entities": [entity, entity]}, spec))

    def test_walk_capture_checks_render_frame_not_later_observation(self):
        spec = {"name": "walk", "entity_type": "minecraft:zombie", "phase": 1.5,
                "tolerance": 0.25, "min_speed": 0.1, "max_ticks": 30}
        entity = {"entity_type": "minecraft:zombie", "entity_uuid": "mob",
                  "walk_position": 1.5, "walk_speed": 0.2}

        class Probe:
            def __init__(self):
                self.calls = 0
            def call(self, name, args):
                self.calls += 1
                return {"entities": [entity if self.calls == 1 else {**entity, "walk_position": 3.0}]}
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            image = str(root / "walk.png")
            for client in ("java", "native"):
                frame_path = Path(image + ".gait.json") if client == "java" else Path(image).with_suffix(".json")
                def write_frame(position):
                    items = [{**entity, "walk_position": position}]
                    frame_path.write_text(json.dumps(items if client == "java" else {"gait_frame": items}))
                with patch.object(scene_shot, f"capture_{client}", return_value=image):
                    write_frame(1.5)
                    self.assertEqual(scene_shot.execute(Probe(), root, [{"walk_capture": spec}], client), [image])
                    evidence = json.loads((root / "walk-walk.json").read_text())
                    self.assertEqual(evidence["after"]["entities"][0]["walk_position"], 3.0)
                    write_frame(2.5)
                    with self.assertRaisesRegex(AssertionError, "walk_render_phase"):
                        scene_shot.execute(Probe(), root, [{"walk_capture": spec}], client)

    def test_steps_dispatch_and_retain_evidence(self):
        class Probe:
            def __init__(self):
                self.client = object()
                self.calls = []
            def call(self, name, args=None):
                self.calls.append((name, args))
                return {"in_play": True}
            def observe(self):
                return {"in_play": True, "position": [1, 2, 3]}
            def wait(self, predicate, label, *, wait_for_play=False):
                assert wait_for_play
                assert predicate({"player": {"x": 1, "z": 3}})
            def remaining(self):
                return 50
        probe = Probe()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(scene_shot.native_client, "capture", return_value={"retained_path": str(root / "a.png")}):
                shots = scene_shot.execute(probe, root, [{"wait_play": True}, {"cmd": "time set 18000"}, {"tp": [1, 2, 3, 90, 0]}, {"wait_ticks": 5}, {"observe": "state"}, {"shot": "a"}])
            self.assertEqual(shots, [str(root / "a.png")])
            self.assertEqual(json.loads((root / "state.json").read_text())["position"], [1, 2, 3])
            self.assertIn(("minecraft_send_chat", {"message": "tp @s 1 2 3 90 0", "command": True}), probe.calls)
            self.assertIn(("minecraft_wait_ticks", {"ticks": 5}), probe.calls)

    def test_assert_chat_requires_fence_and_rejects_unexpected_feedback(self):
        class Probe:
            def __init__(self, lines):
                self.lines = lines
            def observe(self):
                return {"recent_chat": self.lines}
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)
        step = {"assert_chat": {"name": "disabled", "contains": ["fence"],
                                "excludes": ["Set time"]}}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            scene_shot.execute(Probe(["fence"]), root, [step])
            self.assertEqual(json.loads((root / "disabled.json").read_text())["recent_chat"], ["fence"])
            with self.assertRaisesRegex(AssertionError, "disabled_excludes_0"):
                scene_shot.execute(Probe(["fence", "Set time"]), root, [step])
            with self.assertRaisesRegex(AssertionError, "disabled_contains_0"):
                scene_shot.execute(Probe([]), root, [step])
            with self.assertRaisesRegex(RuntimeError, "omitted recent_chat"):
                scene_shot.execute(Probe(None), root, [step])
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([step], "java")
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([{"assert_chat": {"name": "x", "contains": [], "excludes": []}}], "native")

    def test_named_feedback_scene_rejects_overrides(self):
        class Context:
            client = "native"
            windowed = True
            platform = None
            scenario_id = "native-command-feedback"
            extra_args = []
            timeout_seconds = 180
        with patch.dict("os.environ", {"SCENE_SHOT_SCENARIO": ""}):
            scenario = scene_shot.validate_scenario(Context())[0]
            self.assertTrue(any("assert_chat" in step for step in scenario["steps"]))
            with self.assertRaisesRegex(ValueError, "without overrides"):
                scene_shot.validate_scenario(Context(), {"steps": []})
            with patch.dict("os.environ", {"SCENE_SHOT_SCENARIO": "/tmp/other.json"}):
                with self.assertRaisesRegex(ValueError, "without overrides"):
                    scene_shot.validate_scenario(Context())

    def test_named_frozen_age_scene_rejects_overrides(self):
        class Context:
            client = "native"
            windowed = True
            platform = None
            scenario_id = "native-frozen-game-age"
            extra_args = []
            timeout_seconds = 180
        with patch.dict("os.environ", {"SCENE_SHOT_SCENARIO": ""}):
            scenario = scene_shot.validate_scenario(Context())[0]
            self.assertTrue(any("assert_same_game_time" in step for step in scenario["steps"]))
            with self.assertRaisesRegex(ValueError, "without overrides"):
                scene_shot.validate_scenario(Context(), {"steps": []})

    def test_frozen_age_assertion_rejects_advance_and_missing_clock(self):
        class Probe:
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)
        step = {"assert_same_game_time": {"before": "a", "after": "b", "day_delta": 1000}}
        scene_shot.validate_steps([step], "native")
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.json").write_text(json.dumps({"time": {"game_time": 42, "clock_time": 5}}))
            (root / "b.json").write_text(json.dumps({"time": {"game_time": 42, "clock_time": 1005}}))
            scene_shot.execute(Probe(), root, [step])
            (root / "b.json").write_text(json.dumps({"time": {"game_time": 43, "clock_time": 1005}}))
            with self.assertRaisesRegex(AssertionError, "frozen_game_time"):
                scene_shot.execute(Probe(), root, [step])
            (root / "b.json").write_text(json.dumps({"time": {"game_time": 42, "clock_time": 5}}))
            with self.assertRaisesRegex(AssertionError, "received_fresh_day_time"):
                scene_shot.execute(Probe(), root, [step])
            (root / "b.json").write_text("{}")
            with self.assertRaisesRegex(RuntimeError, "omitted game_time"):
                scene_shot.execute(Probe(), root, [step])
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([step], "java")

    def test_wait_respawn_rejects_dead_or_open_screen(self):
        class Probe:
            def wait(self, predicate, label, *, wait_for_play=False):
                assert label == "respawn_alive_screen_closed"
                assert wait_for_play
                assert not predicate({"player": {"health": 0}, "screen": {"open": True}})
                assert not predicate({"player": {"health": 20}, "screen": {"open": True}})
                assert not predicate({"player": None, "screen": {"open": False}})
                assert predicate({"player": {"health": 20}, "screen": {"open": False}})
        scene_shot.execute(Probe(), Path("/tmp"), [{"wait_respawn": True}])
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([{"wait_respawn": False}], "native")

    def test_pause_dispatches_native_menu_and_rejects_other_inputs(self):
        class Probe:
            def __init__(self):
                self.calls = []
            def call(self, name, args):
                self.calls.append((name, args))
        probe = Probe()
        scene_shot.execute(probe, Path("/tmp"), [{"pause": True}])
        self.assertEqual(probe.calls, [("minecraft_open_pause_menu", {})])
        self.assertEqual(list(scene_shot.scene_mcp_calls([{"pause": True}])),
                         [("minecraft_open_pause_menu", {})])
        for client, value in [("java", True), ("native", False)]:
            with self.assertRaises(ValueError):
                scene_shot.validate_steps([{"pause": value}], client)

    def test_use_phases_checks_state_and_releases_on_failure(self):
        class Probe:
            def __init__(self, states):
                self.states = iter(states)
                self.calls = []
            def observe(self):
                return {"using": next(self.states)}
            def call(self, name, args):
                self.calls.append((name, args))
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)
        step = {"use_phases": {"name": "shield", "ticks": 10}}
        with tempfile.TemporaryDirectory() as tmp, \
                patch.object(scene_shot, "capture_native", side_effect=lambda p, a, n: n):
            root = Path(tmp)
            probe = Probe([None, {"hand": 0}, {"hand": 0}, None])
            self.assertEqual(scene_shot.execute(probe, root, [step]),
                             ["shield-start", "shield-hold", "shield-release"])
            failed = Probe([None, None])
            with self.assertRaisesRegex(AssertionError, "shield_started"):
                scene_shot.execute(failed, root, [step])
            self.assertEqual(failed.calls[-1],
                             ("minecraft_press_inputs", {"keys": ["use"], "ticks": 1, "hold": False}))

    def test_resize_requires_owned_window_and_matching_frame_size(self):
        class Probe:
            def __init__(self):
                self.client = type("Client", (), {"p4_window": {"display": ":99", "window_id": 1}})()
            def call(self, name, args):
                assert (name, args) == ("minecraft_wait_ticks", {"ticks": 5})
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)
        step = {"resize": {"name": "small", "size": [960, 540]}}
        with patch.object(scene_shot, "resize_window") as resize, \
                patch.object(scene_shot, "capture_native", return_value="frame.png"), \
                patch.object(scene_shot, "png_size", return_value=(1280, 720)):
            with self.assertRaisesRegex(AssertionError, "small_dimensions"):
                scene_shot.execute(Probe(), Path("/tmp"), [step])
            resize.assert_called_once_with({"display": ":99", "window_id": 1}, (960, 540))
        with self.assertRaises(ValueError):
            scene_shot.validate_steps([{"resize": {"name": "bad", "size": [0, 540]}}], "native")

    def test_disconnect_wait_rejects_live_or_wrong_eof_state(self):
        class Probe:
            def __init__(self, state):
                self.state = state
            def call(self, name, args=None):
                assert name == "minecraft_observe"
                return self.state
            def check(self, name, value):
                if not value:
                    raise AssertionError(name)
        step = {"wait_disconnect": {"name": "eof", "eof": True}}
        with tempfile.TemporaryDirectory() as tmp:
            state = {"in_play": False, "using": None, "loading_readiness": {"phase": "error"},
                     "disconnect_reason": "not EOF"}
            with self.assertRaisesRegex(AssertionError, "eof_eof"):
                scene_shot.execute(Probe(state), Path(tmp), [step])
            state["in_play"] = True
            with self.assertRaisesRegex(AssertionError, "eof_left_play"):
                scene_shot.execute(Probe(state), Path(tmp), [step])
            state["in_play"] = False
            state["disconnect_reason"] = "server closed connection"
            scene_shot.execute(Probe(state), Path(tmp), [step])
            self.assertEqual(json.loads((Path(tmp) / "eof.json").read_text())["disconnect_reason"],
                             "server closed connection")

    def test_look_is_validated_and_dispatched_to_real_client(self):
        step = {"look": {"yaw_deg": 45, "pitch_deg": -60}}
        self.assertEqual(list(scene_shot.scene_mcp_calls([step])),
                         [("minecraft_look", step["look"])])
        class Probe:
            def __init__(self):
                self.calls = []
            def call(self, name, args):
                self.calls.append((name, args))
        probe = Probe()
        scene_shot.execute(probe, Path("/tmp"), [step])
        self.assertEqual(probe.calls, [("minecraft_look", step["look"])])

    def test_hotbar_is_dispatched_to_real_client(self):
        step = {"hotbar": {"slot": 1}}
        self.assertEqual(list(scene_shot.scene_mcp_calls([step])),
                         [("minecraft_set_hotbar_slot", step["hotbar"])])
        class Probe:
            def __init__(self):
                self.calls = []
            def call(self, name, args):
                self.calls.append((name, args))
        probe = Probe()
        scene_shot.execute(probe, Path("/tmp"), [step])
        self.assertEqual(probe.calls, [("minecraft_set_hotbar_slot", step["hotbar"])])

    def test_mob_motion_gate_rejects_stationary_entities(self):
        class Probe:
            def __init__(self):
                self.checks = {}
            def check(self, name, value):
                self.checks[name] = value
        probe = Probe()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name, tick in zip(("a", "b", "c"), (100, 120, 140)):
                (root / f"{name}.json").write_text(json.dumps({
                    "game_time": tick,
                    "snapshot": {"entities": [
                        {"entity_type": "minecraft:zombie", "entity_uuid": "z", "x": 0, "z": 0},
                        {"entity_type": "minecraft:spider", "entity_uuid": "s", "x": 0, "z": 0},
                    ]},
                }))
            scene_shot.execute(probe, root, [{
                "mob_motion": {"samples": ["a", "b", "c"], "name": "motion"},
            }])
        self.assertEqual(probe.checks, {"mob_moved_zombie": False, "mob_moved_spider": False})

    def test_entity_snapshot_requires_same_thread_game_time(self):
        class Probe:
            def __init__(self, game_time):
                self.game_time = game_time
            def call(self, name, args):
                assert name == "minecraft_list_entities"
                return {"game_time": self.game_time, "entities": [{"x": 1}]}
        step = {"timed_entities": {"name": "mob", "args": {"radius": 24}}}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            scene_shot.execute(Probe(100), root, [step])
            receipt = json.loads((root / "mob.json").read_text())
            self.assertEqual(receipt["game_time"], 100)
            self.assertEqual(receipt["snapshot"]["entities"][0]["x"], 1)
            with self.assertRaisesRegex(RuntimeError, "omitted same-thread game_time"):
                scene_shot.execute(Probe(None), root, [step])

    def test_wait_terrain_fails_closed_on_stale_receipt_and_invalid_input(self):
        class Probe:
            report = {}
            def remaining(self):
                return 30
            def call(self, _name, _args):
                raise AssertionError("invalid input reached the tool")
        valid = {"wait_terrain": {"name": "terrain", "args": {"sections": [[1, 2, 3]]}}}
        class Stale(Probe):
            def call(self, _name, _args):
                return {"status": "ready", "connection_epoch": "1", "dimension_epoch": "1",
                        "resource_epoch": "1", "frame_id": "1", "sections": [
                            {"section": [1, 2, 3], "requested_revision": "8",
                             "mesh_revision": "7", "selected_for_draw": True}]}
        with self.assertRaisesRegex(RuntimeError, "invalid minecraft_wait_for_terrain receipt"):
            scene_shot.execute(Stale(), Path("/tmp"), [valid])
        for args in ({"sections": []}, {"sections": [[1, 2, 3], [1, 2, 3]]},
                     {"sections": [[True, 2, 3]]}, {"sections": [[1, 2, 3]], "timeout_seconds": 121}):
            with self.subTest(args=args), self.assertRaises(ValueError):
                scene_shot.execute(Probe(), Path("/tmp"), [{"wait_terrain": {"name": "terrain", "args": args}}])
        with self.assertRaisesRegex(ValueError, "requires --client native"):
            scene_shot.execute(Probe(), Path("/tmp"), [valid], "java")

    def test_unknown_or_unsafe_step_fails(self):
        for step in ({"shot": "../escape"}, {"unknown": True}, {"wait_play": False}, {"tp": [1, 2]}):
            with self.subTest(step=step), self.assertRaises(ValueError):
                scene_shot.execute(None, Path('/tmp'), [step])

    def test_wait_play_spans_mcp_wait_boundaries_and_detects_disconnect(self):
        class Probe:
            def __init__(self, results):
                self.results = iter(results)
                self.calls = 0
            def remaining(self):
                return 120
            def call(self, name, args):
                self.calls += 1
                return next(self.results)
        probe = Probe([{"in_play": False}, {"in_play": True}])
        scene_shot.wait_play(probe)
        self.assertEqual(probe.calls, 2)
        with self.assertRaisesRegex(RuntimeError, "kicked"):
            scene_shot.wait_play(Probe([{"in_play": False, "disconnect_reason": "kicked"}]))

    def test_long_waits_are_split_for_lease_renewal(self):
        class Probe:
            def __init__(self):
                self.ticks = []
            def call(self, name, args):
                self.ticks.append((name, args["ticks"]))
        probe = Probe()
        scene_shot.execute(probe, Path("/tmp"), [{"wait_ticks": 250}])
        self.assertEqual(probe.ticks, [("minecraft_wait_ticks", 100),
                                      ("minecraft_wait_ticks", 100),
                                      ("minecraft_wait_ticks", 50)])

    def test_screenshot_holds_and_releases_tab_even_on_capture_failure(self):
        events = []
        class Probe:
            client = object()
            def call(self, name, args):
                events.append(args["hold"])
            def wait(self, predicate, label):
                self.assert_visible = predicate({"ui_nodes": [{"id": "tab/player/1"}]})
                assert self.assert_visible
        probe = Probe()
        with tempfile.TemporaryDirectory() as tmp, patch.object(scene_shot.native_client, "capture", side_effect=RuntimeError("capture failed")):
            with self.assertRaisesRegex(RuntimeError, "capture failed"):
                scene_shot.execute(probe, Path(tmp), [{"shot": "tab", "hold_keys": ["key.keyboard.tab"]}])
        self.assertEqual(events, [True, False])

    def test_java_shot_uses_minecraft_screenshot(self):
        class Probe:
            client = object()
        with tempfile.TemporaryDirectory() as tmp, patch.object(scene_shot.java_p5, "capture", return_value="/x/a.png") as java_capture, \
                patch.object(scene_shot, "png_size", return_value=scene_shot.WINDOW), \
                patch.object(scene_shot.native_client, "capture") as native_capture:
            probe = Probe()
            self.assertEqual(scene_shot.execute(probe, Path(tmp), [{"shot": "a"}], "java"), ["/x/a.png"])
            java_capture.assert_called_once_with(probe, "a")
            native_capture.assert_not_called()

    def test_java_shot_with_wrong_window_size_fails(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(scene_shot.java_p5, "capture", return_value="/x/a.png"), \
                patch.object(scene_shot, "png_size", return_value=(854, 480)):
            with self.assertRaisesRegex(RuntimeError, "expected"):
                scene_shot.execute(object(), Path(tmp), [{"shot": "a"}], "java")

    def test_java_rejects_native_only_held_screenshot(self):
        with self.assertRaisesRegex(ValueError, "unsupported for --client java"):
            scene_shot.execute(None, Path("/tmp"), [{"shot": "tab", "hold_keys": ["key.keyboard.tab"]}], "java")

    def test_java_options_pin_window_fov_gui_and_distance(self):
        options = dict(line.split(":", 1) for line in scene_shot.java_options(32, 70).splitlines())
        self.assertEqual((options["overrideWidth"], options["overrideHeight"]), ("1280", "720"))
        self.assertEqual((options["fov"], options["guiScale"], options["renderDistance"]), ("0.0", "3", "32"))
        self.assertEqual(options["graphicsPreset"], '"custom"')
        self.assertEqual(scene_shot.java_options(8, 110).count("fov:1.0"), 1)

    def test_java_effective_render_distance_is_checked(self):
        with tempfile.TemporaryDirectory() as tmp:
            options = Path(tmp) / "options.txt"
            options.write_text("fov:0.0\nrenderDistance:32\n")
            scene_shot.check_java_render_distance(options, 32)
            options.write_text("renderDistance:16\n")
            with self.assertRaisesRegex(RuntimeError, "renderDistance.*16.*32"):
                scene_shot.check_java_render_distance(options, 32)
            options.write_text("fov:0.0\n")
            with self.assertRaisesRegex(RuntimeError, "renderDistance"):
                scene_shot.check_java_render_distance(options, 32)
