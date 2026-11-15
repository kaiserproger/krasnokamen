"""The correctness roundtrip must not acquire an exclusive measurement window."""
import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import vanilla_world_roundtrip as roundtrip


class RoundtripLaneTests(unittest.TestCase):
    def test_conversion_uses_normal_lane(self):
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
                    mock.patch.object(roundtrip.subprocess, "Popen", return_value=java), \
                    mock.patch.object(roundtrip.subprocess, "run", side_effect=RuntimeError("unit stop after admission")) as run, \
                    contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(roundtrip.main(), 1)
            command = run.call_args.args[0]
            self.assertEqual(command[:4], [str(roundtrip.MEM_RUN), "-m", "8G", "env"])
            self.assertNotIn("-x", command)
            self.assertEqual(command[command.index("cargo"):],
                             ["cargo", "test", "-p", "mc-world-conversion", roundtrip.TEST,
                              "--", "--ignored", "--exact", "--nocapture"])
            receipt = json.loads((output / "result.json").read_text())
            self.assertEqual(receipt["error"], "unit stop after admission")
            self.assertFalse(receipt["passed"])


if __name__ == "__main__":
    unittest.main()
