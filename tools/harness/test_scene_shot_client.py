"""Java client language-load guard: a fallback client is not evidence.

Acceptance 2026-11-19 (`receipts/SettlementQuickWinsAccept/java-7209af9fb/run1-26.3-pass`)
ran a ru_ru scene whose asset store lacked every `minecraft/lang/*.json` object. The
client still reported ru_ru, so the server sent Russian literals, while the client
translated item names and vanilla tooltips into the en_us fallback ("Cobblestone ×42"
next to «и ещё 9 видов»). `check_java_language` refuses such a launch.
"""
import io
import shutil
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

# `scene_shot_client` and `scene_shot` import each other; loading `scene_shot` first
# finishes the cycle, so the client module can be bound afterwards.
import tools.harness.backends.scene_shot  # noqa: F401
from tools.harness.backends import scene_shot_client

RUN_HEAD = """\
[05:00:17] [Render thread/INFO]: Environment: authHost='https://authserver.mojang.com'
[05:00:19] [Render thread/INFO]: Reloading ResourceManager: vanilla
"""

BROKEN_RU_RU = """\
[05:00:19] [Render thread/WARN]: Failed to load translations for ru_ru from pack vanilla
java.nio.file.NoSuchFileException: /assets/objects/4c/4cd2190ccb9151a43cfdccb0e0f6d000c4025ade
	at net.minecraft.client.resources.language.ClientLanguage.appendFrom(ClientLanguage.java:57)
	at net.minecraft.client.resources.language.ClientLanguage.loadFrom(ClientLanguage.java:41)
	at net.minecraft.client.resources.language.LanguageManager.onResourceManagerReload(LanguageManager.java:75)
[05:00:22] [Render thread/INFO]: Connecting to 127.0.0.1
"""


class JavaLanguageGuardTest(unittest.TestCase):
    def log(self, text: str) -> Path:
        handle = tempfile.NamedTemporaryFile("w", suffix=".log", delete=False, encoding="utf-8")
        self.addCleanup(Path(handle.name).unlink, missing_ok=True)
        with handle:
            handle.write(text)
        return Path(handle.name)

    def test_a_client_that_could_not_load_the_requested_language_fails_the_run(self):
        with self.assertRaises(RuntimeError) as raised:
            scene_shot_client.check_java_language(self.log(RUN_HEAD + BROKEN_RU_RU), "ru_ru")
        message = str(raised.exception)
        self.assertIn("asset store is incomplete", message)
        self.assertIn("minecraft/lang/ru_ru.json", message)

    def test_a_healthy_language_load_passes(self):
        log = self.log(RUN_HEAD + "[05:00:20] [Render thread/INFO]: Connecting to 127.0.0.1\n")
        self.assertFalse(scene_shot_client.java_language_failure(log, "ru_ru"))
        scene_shot_client.check_java_language(log, "ru_ru")

    def test_another_languages_failure_does_not_fail_this_run(self):
        log = self.log(RUN_HEAD + BROKEN_RU_RU)
        scene_shot_client.check_java_language(log, "en_us")
        self.assertTrue(scene_shot_client.java_language_failure(log, "ru_ru"))

    def test_a_missing_or_unwritten_log_is_not_a_language_failure(self):
        self.assertFalse(scene_shot_client.java_language_failure(Path("/nonexistent/client.log"), "ru_ru"))
        scene_shot_client.check_java_language(self.log(""), "ru_ru")


class FakeMcp:
    instances: list["FakeMcp"] = []

    def __init__(self, url, token, timeout):
        self.calls: list[str] = []
        FakeMcp.instances.append(self)

    def initialize(self):
        self.calls.append("initialize")

    def call_tool(self, name, arguments=None):
        self.calls.append(name)
        return {}

    def close(self):
        self.calls.append("close")


class JavaLaunchGuardTest(unittest.TestCase):
    """`start_java` must refuse the launch before connecting a mixed-language client."""

    def launch(self, log_text: str):
        artifact = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, artifact, True)

        def start_client(*, log_path, **_):
            Path(log_path).write_text(log_text, encoding="utf-8")
            return SimpleNamespace(args=["java", "-client"], poll=lambda: None), io.StringIO()

        FakeMcp.instances.clear()
        ctx = SimpleNamespace(commands=[], cwds=[], exits=[], logs=[], client_evidence={})
        with mock.patch.multiple(
            scene_shot_client,
            McpClient=FakeMcp,
            wait_java_bootstrap=mock.Mock(return_value={}),
        ), mock.patch.multiple(
            scene_shot_client.runtime,
            reserve_port=mock.Mock(return_value=1234),
            sibling_root=mock.Mock(return_value=Path("/nonexistent/loader")),
            start_client=mock.Mock(side_effect=start_client),
            stop_process=mock.Mock(),
        ):
            scene_shot_client.start_java(ctx, artifact, "127.0.0.1:25565", "SceneShot", 5.0, ":99", 8, 70,
                                         language="ru_ru")
        return FakeMcp.instances[-1]

    def test_a_broken_language_refuses_the_launch_before_connect(self):
        with self.assertRaises(RuntimeError) as raised:
            self.launch(RUN_HEAD + BROKEN_RU_RU)
        self.assertIn("asset store is incomplete", str(raised.exception))

    def test_a_loaded_language_connects(self):
        mcp = self.launch(RUN_HEAD + "[05:00:20] [Render thread/INFO]: Connecting\n")
        self.assertEqual(mcp.calls, ["initialize", "minecraft_connect"])


if __name__ == "__main__":
    unittest.main()
