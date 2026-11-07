"""The vanilla Java lane must never silently launch a mod loader."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

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
        self.assertIn("-Xmx512M", command)
        self.assertFalse(any("KnotClient" in arg or "neoforge" in arg or "gradlew" in arg for arg in command))
        self.assertIn("26.1.2-30", command)

    def test_wrong_version_is_not_a_compatible_fallback(self):
        self.metadata["id"] = "26.2"
        self.info.write_text(json.dumps(self.metadata))
        with self.assertRaisesRegex(MissingCapability, "26.1.2"):
            self.command()

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

    def test_prepared_javaexec_rewrites_only_per_client_identity_and_game(self):
        manifest = self.root / "fabric.json"
        manifest.write_text(json.dumps({
            "java": "/java", "main_class": "net.fabricmc.devlaunchinjector.Main",
            "classpath": [str(self.agent)], "jvm_args": ["-Xmx4G", "-Dsolaris.loader.cacheDir=/prepared/game/cache"],
            "args": ["--username", "PreparedJava", "--gameDir", "/prepared/game"],
            "game_dir": "/prepared/game", "username": "PreparedJava"}))
        command = launcher.prepared_fabric_command(manifest, self.game, "JMatrix07")
        self.assertNotIn("-Xmx4G", command)
        self.assertIn("-Xmx512M", command)
        self.assertIn("JMatrix07", command)
        self.assertIn(str(self.game), command)
        self.assertIn("-Dsolaris.loader.cacheDir=" + str(self.game / "cache"), command)


if __name__ == "__main__":
    unittest.main()
