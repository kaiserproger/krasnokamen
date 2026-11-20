"""The Horizon quick-wins walk as a harness profile: lifecycle, receipt, and no wall-clock sleep.

The walk used to be a standalone `main()`: nothing stopped the server/Xvfb/client when it failed
before its own `try`, and `result.json` named neither tree nor pin. `run(ctx)` owns every process in
one `ExitStack`, records the pin, and the harness writes the tree fingerprint; these tests pin those
contracts without starting a real client. `Probe.wait` still polls (`time.sleep`): Horizon has no
state-change push yet, so that replacement is blocked, not silently approximated.
"""
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest import mock

from tools.harness import ledger, plan, preflight, profiles
import tools.harness.backends.settlement_quickwins_horizon as horizon


class FakeProcess:
    def __init__(self, args=("java",)):
        self.args = list(args)
        self.pid = 4242
        self.returncode = None

    def poll(self):
        return None


class FakeServer:
    instances: list["FakeServer"] = []

    def __init__(self, ctx, binary, artifact, label, config, port):
        self.ctx, self.port, self.label = ctx, port, label
        self.argv = [str(binary), "--config", str(artifact / f"{label}.toml")]
        self.log_path = artifact / f"{label}.log"
        self.config = config
        self.starts = 0
        self.stops = 0
        FakeServer.instances.append(self)

    def start(self):
        self.starts += 1
        self.ctx.commands.append(self.argv)
        self.ctx.cwds.append("repo")
        self.ctx.exits.append(None)
        self.ctx.logs.append(str(self.log_path))

    def wait_ready(self, timeout):
        self.timeout = timeout

    def stop(self):
        self.stops += 1


class ProbeDeadlineTest(unittest.TestCase):
    def test_the_probe_deadline_is_the_run_timeout(self):
        artifact = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, artifact, True)
        probe = horizon.Probe(lambda *_a, **_k: {}, artifact, deadline_seconds=30)
        self.assertLessEqual(probe.remaining(), 30)
        self.assertGreater(probe.remaining(), 0)


class PushWaitTest(unittest.TestCase):
    """`Probe.wait` blocks on `wait-for state-change`; it must never sleep on the wall clock."""

    def probe(self, tool_calls: list[tuple[str, dict]], ready: dict[str, bool]):
        artifact = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, artifact, True)

        def hz(tool, **arguments):
            tool_calls.append((tool, arguments))
            if tool == "state":
                return {"player": {"position": [0.0, 64.0, 0.0], "yaw": 0.0, "pitch": 0.0},
                        "screen": "playing", "revision": 41}
            if tool == "messages":
                return {"chat": []}
            if tool == "wait_for":
                if arguments.get("condition") == "state-change" and arguments.get("since") == 41:
                    ready["value"] = True
                    return {"ok": True, "waited_seconds": 0.1, "screen": "playing"}
                return {"error": "wrong condition", "kind": "timeout"}
            raise AssertionError(f"unexpected Horizon tool {tool}")

        return horizon.Probe(hz, artifact)

    def test_the_wait_pushes_on_the_state_revision_instead_of_sleeping(self):
        calls: list[tuple[str, dict]] = []
        probe = self.probe(calls, ready := {"value": False})

        def no_sleep(_seconds):
            raise AssertionError("Probe.wait slept on the wall clock")

        with mock.patch.object(horizon.time, "sleep", no_sleep):
            state = probe.wait(lambda _state: ready["value"], "QW test screen", timeout_seconds=5)
        self.assertFalse(state["screen"]["open"])
        self.assertTrue(probe.report["assertions"]["QW test screen"])
        self.assertIn(("wait_for", {"condition": "state-change", "since": 41, "timeout": mock.ANY}), calls)
        self.assertNotIn("wait", [tool for tool, _ in calls])

    def test_a_state_without_a_revision_fails_loudly(self):
        probe = horizon.Probe(lambda tool, **_k: {"player": {"position": [0, 0, 0], "yaw": 0, "pitch": 0},
                                                   "screen": "playing"}.get(tool, {}), Path(tempfile.mkdtemp()))
        with self.assertRaises(RuntimeError) as raised:
            probe.wait(lambda _state: True, "QW no revision", timeout_seconds=1)
        self.assertIn("no revision", str(raised.exception))


class LanguageGuardTest(unittest.TestCase):
    """A `ru_ru` run on resources without `lang/ru_ru.json` is not evidence (English item names)."""

    def ctx(self, resources: Path, extra: list[str] | None = None):
        fake = resources.parent / "horizon-client"
        fake.write_bytes(b"#!client\n")
        fake.chmod(0o755)
        ctl = resources.parent / "horizon-ctl"
        ctl.write_bytes(b"#!ctl\n")
        ctl.chmod(0o755)
        return SimpleNamespace(artifact_dir=resources.parent / "artifact",
                               extra_args=extra or ["--horizon-client", str(fake), "--horizon-ctl", str(ctl),
                                                    "--horizon-resources", str(resources)],
                               timeout_seconds=60.0, platform=None, plugins_dir=None, client="java")

    def test_the_ru_ru_run_refuses_when_the_resources_have_no_russian_language_file(self):
        root = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, root, True)
        resources = root / "resources"
        (resources / "assets" / "minecraft" / "lang").mkdir(parents=True)
        (resources / "assets" / "minecraft" / "lang" / "en_us.json").write_text("{}\n")
        with self.assertRaises(RuntimeError) as raised:
            horizon.validate_args(self.ctx(resources))
        self.assertIn("no assets/minecraft/lang/ru_ru.json", str(raised.exception))
        (resources / "assets" / "minecraft" / "lang" / "ru_ru.json").write_text("{}\n")
        args, _ = horizon.validate_args(self.ctx(resources))
        self.assertEqual(args.language, "ru_ru")

class LaunchTest(unittest.TestCase):
    """`run(ctx)` on mocked processes; the walk itself is not started."""

    def setUp(self):
        self.artifact = Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, self.artifact, True)
        self.binary = self.artifact / "krasnokamen"
        self.binary.write_bytes(b"#!server pin\n")
        fake_client = Path(self.artifact / "horizon-client")
        fake_client.write_bytes(b"#!client\n")
        fake_client.chmod(0o755)
        self.client = fake_client
        fake_ctl = Path(self.artifact / "horizon-ctl")
        fake_ctl.write_bytes(b"#!ctl\n")
        fake_ctl.chmod(0o755)
        resources = self.artifact / "resources"
        (resources / "assets" / "minecraft" / "lang").mkdir(parents=True)
        (resources / "assets" / "minecraft" / "lang" / "en_us.json").write_text("{}\n")
        (resources / "assets" / "minecraft" / "lang" / "ru_ru.json").write_text("{}\n")
        self.resources = resources
        extra_args = ["--horizon-client", str(fake_client), "--horizon-ctl", str(fake_ctl),
                      "--horizon-resources", str(resources)]
        self.ctx = SimpleNamespace(artifact_dir=self.artifact / "artifact", extra_args=extra_args, timeout_seconds=60.0,
                                   platform=None, plugins_dir=None, client="java", commands=[], cwds=[],
                                   exits=[], logs=[], client_evidence={})
        self.processes: dict[str, FakeProcess] = {}
        self.stop_calls: list[FakeProcess] = []
        self.control_sockets: list[Path] = []
        FakeServer.instances.clear()

    def start(self, control_error: str | None = None):
        def start_xvfb(artifact, *_):
            self.processes["xvfb"] = FakeProcess(["Xvfb"])
            return self.processes["xvfb"], ":99", io.BytesIO()

        def popen(command, **_):
            self.processes["client"] = FakeProcess(command)
            self.processes["client"].command = list(command)
            return self.processes["client"]

        def control(ctl, socket, home, env):
            self.control_sockets.append(Path(socket))
            if control_error is not None:
                raise RuntimeError(control_error)
            raise AssertionError("the walk must not start in these tests")

        def stop_process(process, **_):
            self.stop_calls.append(process)
            if process.returncode is None:
                process.returncode = 0

        with mock.patch.object(horizon, "Server", FakeServer), \
                mock.patch.object(horizon, "control", control), \
                mock.patch.object(horizon.subprocess, "Popen", popen), \
                mock.patch.object(horizon.runtime, "start_xvfb", start_xvfb), \
                mock.patch.object(horizon.runtime, "stop_process", stop_process), \
                mock.patch.object(horizon.runtime, "_wait_for_file", lambda *a, **k: None), \
                mock.patch.object(horizon.runtime, "reserve_port", lambda: 25599), \
                mock.patch.object(profiles, "_server_binary", lambda ctx, log: self.binary):
            with self.assertRaises(RuntimeError) as raised:
                horizon.run(self.ctx)
        return str(raised.exception)

    def test_a_failure_after_the_server_starts_leaves_no_live_process(self):
        message = self.start(control_error="the control socket closed during connect")
        self.assertIn("the control socket closed during connect", message)
        self.assertEqual(len(FakeServer.instances), 1)
        self.assertEqual(FakeServer.instances[0].starts, 1)
        self.assertEqual(FakeServer.instances[0].stops, 1, "the started server was not stopped")
        self.assertIn(self.processes["xvfb"], self.stop_calls, "the started Xvfb was not stopped")
        self.assertIn(self.processes["client"], self.stop_calls, "the started client was not stopped")
        self.assertEqual(self.processes["client"].command[0], str(self.client))
        self.assertEqual(self.processes["client"].returncode, 0)
        index = self.ctx.commands.index(self.processes["client"].command)
        self.assertEqual(self.ctx.exits[index], 0)
        self.assertEqual(len(self.control_sockets), 1)
        self.assertFalse(self.control_sockets[0].exists(), "the control socket directory outlived the failure")
        self.assertFalse((self.artifact / "artifact" / "scenario.json").exists())

    def test_the_receipt_names_the_tree_and_the_server_pin(self):
        self.start(control_error="stopped for the receipt test")
        evidence = self.ctx.client_evidence
        digest = hashlib.sha256(self.binary.read_bytes()).hexdigest()
        self.assertEqual(evidence["server_binary_path"], str(self.binary))
        self.assertEqual(evidence["server_binary_sha256"], digest)
        self.assertEqual(evidence["settlement_quickwins_horizon"]["verdict"]["status"], "error")
        # What the harness writes: its tree fingerprint beside this evidence (ledger._receipt_identity).
        receipt = {"profile": "settlement-quickwins-horizon", "details": evidence,
                   "tree": ledger.tree_fingerprint(profiles.REPO_ROOT)}
        text = json.dumps(receipt)
        self.assertRegex(receipt["tree"]["head"], r"^[0-9a-f]{40}$")
        self.assertRegex(receipt["tree"]["sha256"], r"^[0-9a-f]{64}$")
        self.assertIn(digest, text, "the receipt text must name the server pin the walk ran against")

    def test_the_profile_is_registered_for_the_harness(self):
        self.assertIs(profiles.PROFILES["settlement-quickwins-horizon"]["run"],
                      profiles.run_settlement_quickwins_horizon)
        self.assertEqual(plan.BACKEND_PROFILES["settlement_quickwins_horizon"], ("settlement-quickwins-horizon",))
        self.assertEqual(preflight._ARGUMENT_VALIDATORS["settlement-quickwins-horizon"],
                         "settlement_quickwins_horizon")


if __name__ == "__main__":
    unittest.main()
