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
    def conversion_command(self, mem_run: Path) -> list[str]:
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
                    mock.patch.object(roundtrip.subprocess, "Popen", return_value=java), \
                    mock.patch.object(roundtrip.subprocess, "run", side_effect=RuntimeError("unit stop after admission")) as run, \
                    contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(roundtrip.main(), 1)
            receipt = json.loads((output / "result.json").read_text())
            self.assertEqual(receipt["error"], "unit stop after admission")
            self.assertFalse(receipt["passed"])
            return run.call_args.args[0]

    def test_conversion_uses_normal_lane_of_the_canonical_runner(self):
        with tempfile.TemporaryDirectory() as directory:
            mem_run = Path(directory) / "mem-run.sh"
            mem_run.write_text("#!/bin/sh\n")
            command = self.conversion_command(mem_run)
        self.assertEqual(command[:4], [str(mem_run), "-m", "8G", "env"])
        self.assertNotIn("-x", command)
        self.assertEqual(command[command.index("cargo"):],
                         ["cargo", "test", "-p", "mc-world-conversion", roundtrip.TEST,
                          "--", "--ignored", "--exact", "--nocapture"])

    def test_conversion_runs_cargo_directly_without_the_owner_host_runner(self):
        with tempfile.TemporaryDirectory() as directory:
            command = self.conversion_command(Path(directory) / "absent-mem-run.sh")
        self.assertEqual(command[0], "env")
        self.assertEqual(command[command.index("cargo"):],
                         ["cargo", "test", "-p", "mc-world-conversion", roundtrip.TEST,
                          "--", "--ignored", "--exact", "--nocapture"])


if __name__ == "__main__":
    unittest.main()
