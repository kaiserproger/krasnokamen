import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.harness import profiles, regressions
from tools.harness.backends import scene_shot


class RealClientRefTests(unittest.TestCase):
    def test_gait_refs_dispatch_their_canonical_graphical_fixtures(self):
        for name, client in (("native-gait-spider", "native"), ("java-gait-sidecar", "java")):
            with self.subTest(name=name), patch.dict(os.environ, {}, clear=True):
                argv = regressions._client_command("scene-shot", name)
                self.assertIn("--windowed", argv)
                self.assertEqual(argv[argv.index("--client") + 1], client)
                ctx = profiles.ProfileContext(Path("/tmp"), client=client, windowed=True, scenario_id=name)
                fixture = scene_shot.validate_scenario(ctx)[0]
                self.assertEqual(fixture.get("java_plain", False), client == "java")
                self.assertEqual(sum("walk_capture" in step for step in fixture["steps"]), 2)

    def test_battle_ref_requires_late_capture_result(self):
        import json
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "krasnokamen"
            binary.write_bytes(b"server")
            env = {"KRASNOKAMEN_SERVER_BINARY": str(binary), "KRASNOKAMEN_CLIENT_BINARY": "/recorder",
                   "KRASNOKAMEN_BATTLE_RECORDER_DATA": "/data", "KRASNOKAMEN_BATTLE_WORLD": "/world",
                   "KRASNOKAMEN_BATTLE_PACK_ROOT": "/pack", "KRASNOKAMEN_BATTLE_HIT_COUNTER": "/counter"}
            ctx = profiles.ProfileContext(root, client="java", platform="neoforge", scenario_id="java-battle-end-camera")
            with patch.dict(os.environ, env), patch.object(profiles, "_run_command") as launch:
                (root / "java-battle.json").write_text(json.dumps({"functional_passed": True, "checks": {}}))
                with self.assertRaisesRegex(AssertionError, "late_camera_capture"):
                    profiles.run_java_battle(ctx)
                self.assertIn("--late-capture", launch.call_args.args[0])
                (root / "java-battle.json").write_text(json.dumps({"functional_passed": True, "checks": {"late_camera_capture": True}}))
                self.assertTrue(profiles.run_java_battle(ctx)["checks"]["late_camera_capture"])
                self.assertEqual(ctx.client_evidence["server_binary_path"], str(binary))

    def test_mixed_version_ref_and_live_output_selection(self):
        from types import SimpleNamespace
        from tools.harness.loader_mixed_version import class_directory, class_hashes
        name = "loader-fabric-mixed-version-build"
        argv = regressions._client_command("loader-live", name)
        self.assertIn(name, argv)
        self.assertEqual(argv[argv.index("--platform") + 1], "fabric")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runtime = SimpleNamespace(sibling_root=lambda _: root)
            with patch.dict(os.environ, {"KRASNOKAMEN_JAVA_VERSION": "26.3"}):
                shared = class_directory(runtime, "26.3", root)
                self.assertEqual(shared, root / "loader-fabric/build/classes/java/main")
                (root / "client-init.gradle").write_text("isolated launch")
                isolated = class_directory(runtime, "26.3", root)
                self.assertNotEqual(shared, isolated)
                self.assertIn("build/26.3/loader-fabric/classes", str(isolated))
                shared.mkdir(parents=True)
                (shared / "Unchanged.class").write_bytes(b"same")
                versioned = shared / "ClientVersionApi.class"
                versioned.write_bytes(b"26.3")
                before = class_hashes(shared)
                self.assertEqual(before, class_hashes(shared))
                versioned.write_bytes(b"26.1.2")
                self.assertNotEqual(before, class_hashes(shared))
                versioned.unlink()
                self.assertNotEqual(before, class_hashes(shared))
