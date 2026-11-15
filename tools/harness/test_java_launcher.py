"""The vanilla Java lane must never silently launch a mod loader."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from . import java_launcher as launcher
from .matrix_api import MissingCapability


class VanillaLauncherTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.cache = self.root / "cache"
        self.cache.mkdir()
        self.game = self.root / "game"
        self.agent = self.root / "agent.jar"
        self.agent.write_bytes(b"agent")
        self.client = self.cache / "minecraft-client.jar"
        self.client.write_bytes(b"client")
        self.assets = self.root / "assets"
        (self.assets / "indexes").mkdir(parents=True)
        (self.assets / "objects").mkdir()
        index = b'{"objects":{}}'
        (self.assets / "indexes/26.1.2-30.json").write_bytes(index)
        self.metadata = dict(id="26.1.2", mainClass="net.minecraft.client.main.Main",
                             downloads={"client": self.pin(b"client")}, libraries=[],
                             javaVersion={"majorVersion": 25},
                             assetIndex={"id": "30", **self.pin(index)})
        self.info = self.cache / "mojang_minecraft_info.json"
        self.info.write_text(json.dumps(self.metadata))

    def pin(self, content):
        return dict(size=len(content), sha1=hashlib.sha1(content).hexdigest(), url="https://example.invalid/artifact")

    def command(self):
        return launcher.vanilla_command(game_dir=self.game, username="JMatrix00", agent_jar=self.agent,
                                        metadata_path=self.info, assets_root=self.assets,
                                        gradle_home=self.root / "gradle", library_roots=[])

    def test_true_vanilla_main_and_agent_only(self):
        command = self.command()
        self.assertIn("net.minecraft.client.main.Main", command)
        self.assertIn(f"-javaagent:{self.agent}", command)
        self.assertNotIn("-Dkrasnokamen.client.vanillaEvents=true", command)
        self.assertIn("-Xmx512M", command)
        self.assertFalse(any("KnotClient" in arg or "neoforge" in arg or "gradlew" in arg for arg in command))
        self.assertIn("26.1.2-30", command)

    def test_wrong_version_is_not_a_compatible_fallback(self):
        self.metadata["id"] = "26.2"
        self.info.write_text(json.dumps(self.metadata))
        with self.assertRaisesRegex(MissingCapability, "26.1.2"):
            self.command()

    def test_explicit_263_accepts_only_matching_metadata(self):
        self.metadata["id"] = "26.3"
        self.info.write_text(json.dumps(self.metadata))
        (self.assets / "indexes/26.3-30.json").write_bytes(b'{"objects":{}}')
        with mock.patch.dict("os.environ", {"KRASNOKAMEN_JAVA_VERSION": "26.3"}):
            command = self.command()
            self.assertIn("-Dkrasnokamen.client.vanillaEvents=true", command)
            self.assertEqual(command[command.index("--version") + 1], "26.3")
            self.metadata["id"] = "26.1.2"
            self.info.write_text(json.dumps(self.metadata))
            with self.assertRaisesRegex(MissingCapability, "26.3"):
                self.command()

    def test_explicit_262_requires_matching_metadata(self):
        with mock.patch.dict("os.environ", {"KRASNOKAMEN_JAVA_VERSION": "26.2"}):
            with self.assertRaisesRegex(MissingCapability, "26.2"):
                self.command()
            self.metadata["id"] = "26.2"
            self.info.write_text(json.dumps(self.metadata))
            (self.assets / "indexes/26.2-30.json").write_bytes(b'{"objects":{}}')
            command = self.command()
            self.assertEqual(command[command.index("--version") + 1], "26.2")

    def test_modified_client_bytes_fail_closed(self):
        self.client.write_bytes(b"tampered")
        with self.assertRaisesRegex(MissingCapability, "hash|size"):
            self.command()

    def test_linux_artifact_rules_exclude_other_platforms(self):
        self.assertFalse(launcher.allowed({"rules": [{"action": "allow", "os": {"name": "windows"}}]}))
        self.assertTrue(launcher.allowed({"rules": [{"action": "allow", "os": {"name": "linux"}}]}))
        self.assertTrue(launcher.allowed({}))

    def test_missing_library_reports_exact_published_prerequisite(self):
        self.metadata["libraries"] = [{"name": "a:b:1", "downloads": {"artifact": {
            "path": "a/b/1/b-1.jar", **self.pin(b"jar")}}}]
        self.info.write_text(json.dumps(self.metadata))
        with self.assertRaisesRegex(MissingCapability, "a:b:1"):
            self.command()

class ModdedLauncherTest(unittest.TestCase):

    def test_selectors_never_switch_a_resolved_task(self):
        for task in (":fabric-agent:runClientMcp", ":loader-neoforge:runClientMcp"):
            with self.subTest(task=task), mock.patch.dict(
                    "os.environ", {"KRASNOKAMEN_JAVA_VERSION": "26.1.2",
                                   "KRASNOKAMEN_JAVA_PLATFORM": "fabric"}):
                with self.assertRaisesRegex(MissingCapability, "conflicts with task"):
                    launcher.modded_gradle_arguments(task)

    def test_build_root_uses_owner_targets_or_portable_local_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            loader = work / "loader"
            with mock.patch.dict("os.environ", {"KRASNOKAMEN_WORK_ROOT": str(work),
                                               "KRASNOKAMEN_JAVA_VERSION": "26.1.2"}):
                local = launcher.gradle_project_cache(loader, ":loader-fabric:runClientMcp")
                self.assertTrue(local.is_relative_to(loader / ".analysis/harness-builds"))
                (work / "targets").mkdir()
                owner = launcher.gradle_project_cache(loader, ":loader-fabric:runClientMcp")
                self.assertTrue(owner.is_relative_to(work / "targets/loader-builds"))
                self.assertEqual(owner.parts[-3:], ("26.1.2", "loader-fabric", "project-cache"))
                self.assertNotEqual(owner, launcher.gradle_project_cache(work / "other", ":loader-fabric:runClientMcp"))
                self.assertNotEqual(owner, launcher.gradle_project_cache(loader, ":loader-neoforge:runClientMcp"))
                with mock.patch.dict("os.environ", {"KRASNOKAMEN_JAVA_VERSION": "26.3"}):
                    self.assertNotEqual(owner, launcher.gradle_project_cache(loader, ":loader-fabric:runClientMcp"))



if __name__ == "__main__":
    unittest.main()
