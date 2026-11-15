"""The correctness roundtrip uses the normal lane and the canonical runner where it exists."""
import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import vanilla_world_roundtrip as roundtrip


class RoundtripLaneTests(unittest.TestCase):
    def spawned_commands(self, mem_run: Path) -> tuple[list[str], list[str]]:
        """(first Java server command, conversion command) up to the stubbed conversion."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            jar = root / "server.jar"
            jar.write_bytes(b"unit jar")
            eula = root / "eula.txt"
            eula.write_text("eula=true\n")
            output = root / "receipt"
            java = mock.Mock(stdout=io.StringIO('Done (1.0s)! For help, type "help"\n'),
                             stdin=io.StringIO(), returncode=0)
            java.poll.return_value = 0
            with mock.patch("sys.argv", ["roundtrip", "--jar", str(jar), "--eula-file", str(eula),
                                         "--output", str(output)]), \
                    mock.patch.object(roundtrip, "MEM_RUN", mem_run), \
                    mock.patch.object(roundtrip.subprocess, "Popen", return_value=java) as popen, \
                    mock.patch.object(roundtrip.subprocess, "run", side_effect=RuntimeError("unit stop after admission")) as run, \
                    contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(roundtrip.main(), 1)
            receipt = json.loads((output / "result.json").read_text())
            self.assertEqual(receipt["error"], "unit stop after admission")
            self.assertFalse(receipt["passed"])
            return popen.call_args_list[0].args[0], run.call_args.args[0]

    def test_commands_use_normal_lane_of_the_canonical_runner(self):
        with tempfile.TemporaryDirectory() as directory:
            mem_run = Path(directory) / "mem-run.sh"
            mem_run.write_text("#!/bin/sh\n")
            java, command = self.spawned_commands(mem_run)
        self.assertEqual(java[:3], [str(mem_run), "-m", "4G"])
        self.assertEqual(command[:4], [str(mem_run), "-m", "8G", "env"])
        self.assertNotIn("-x", command)
        self.assertEqual(command[command.index("cargo"):],
                         ["cargo", "test", "-p", "mc-world-conversion", roundtrip.TEST,
                          "--", "--ignored", "--exact", "--nocapture"])

    def test_commands_run_directly_without_the_owner_host_runner(self):
        with tempfile.TemporaryDirectory() as directory:
            java, command = self.spawned_commands(Path(directory) / "absent-mem-run.sh")
        self.assertEqual(java[0], "java")
        self.assertEqual(command[0], "env")
        self.assertEqual(command[command.index("cargo"):],
                         ["cargo", "test", "-p", "mc-world-conversion", roundtrip.TEST,
                          "--", "--ignored", "--exact", "--nocapture"])


if __name__ == "__main__":
    unittest.main()
