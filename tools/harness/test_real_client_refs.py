import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.harness import profiles


class RealClientRefTests(unittest.TestCase):
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
            with patch.dict(os.environ, env), patch.object(profiles, "_run_command"):
                (root / "java-battle.json").write_text(json.dumps({"functional_passed": True, "checks": {}}))
                with self.assertRaisesRegex(AssertionError, "late_camera_capture"):
                    profiles.run_java_battle(ctx)
                (root / "java-battle.json").write_text(json.dumps({"functional_passed": True, "checks": {"late_camera_capture": True}}))
                self.assertTrue(profiles.run_java_battle(ctx)["checks"]["late_camera_capture"])

    def test_mixed_version_observer_detects_overwritten_or_deleted_classes(self):
        from tools.harness.loader_mixed_version import class_hashes

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Unchanged.class").write_bytes(b"same")
            versioned = root / "ClientVersionApi.class"
            versioned.write_bytes(b"26.3")
            before = class_hashes(root)
            versioned.write_bytes(b"26.1.2")
            self.assertNotEqual(before, class_hashes(root))
            versioned.unlink()
            self.assertNotEqual(before, class_hashes(root))
