"""Task leases (task-claim, task-release) and fleet-status: python3 tools/dev/test_task_claim.py

The fleet dir is private (SOLARIS_FLEET_DIR) and the clock injected (SOLARIS_FLEET_NOW), so expiry needs
no sleep. Claims run inside this test's own process, the way Main's subagents run inside Main's one
omp process: one live PID claims for two logical agents, so only the lease can decide."""
from __future__ import annotations

import io
import json
import os
import select
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from test_slot_gc import load_tool  # noqa: E402

claim_tool = load_tool("task-claim")
T0 = 1_800_000_000
MIN = 60


class LeaseTests(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.base = Path(tmp.name)
        self.fleet = self.base / "fleet"
        patcher = mock.patch.dict(os.environ, {"SOLARIS_FLEET_DIR": str(self.fleet)})
        patcher.start()
        self.addCleanup(patcher.stop)

    def claim(self, *argv: str, at: int) -> str:
        os.environ["SOLARIS_FLEET_NOW"] = str(at)
        out = io.StringIO()
        with redirect_stdout(out):
            claim_tool.main(list(argv))
        return out.getvalue()

    def refused(self, *argv: str, at: int) -> str:
        with self.assertRaises(claim_tool.w.ToolError) as caught:
            self.claim(*argv, at=at)
        return str(caught.exception)

    def log(self) -> list[dict]:
        return [json.loads(line) for line in (self.fleet / "claims.log").read_text().splitlines()]

    def test_second_agent_is_refused_until_the_first_releases_then_takes_over(self):
        self.assertIn("claimed T1 for AgentA", self.claim("T1", "--agent", "AgentA", "--timebox", "30", at=T0))
        self.assertIn("held by AgentA", self.refused("T1", "--agent", "AgentB", "--timebox", "30", at=T0 + MIN))
        self.assertIn("held by AgentA, not AgentB", self.refused("--renew", "T1", "--agent", "AgentB", at=T0 + MIN))
        release = subprocess.run([str(HERE / "task-release"), "T1"], capture_output=True, text=True, check=True,
                                 env=os.environ | {"SOLARIS_FLEET_NOW": str(T0 + 2 * MIN)})
        self.assertIn("released T1 (held by AgentA", release.stdout)
        took = self.claim("T1", "--agent", "AgentB", "--timebox", "30", at=T0 + 3 * MIN)
        self.assertIn("took over T1 for AgentB from AgentA (released", took)
        self.assertEqual([entry["op"] for entry in self.log()], ["claim", "refuse", "release", "takeover"])
        takeover = self.log()[-1]
        self.assertEqual((takeover["agent"], takeover["previous"], takeover["previous_state"]),
                         ("AgentB", "AgentA", "released"))
        self.assertEqual(json.loads((self.fleet / "claims" / "T1.json").read_text())["agent"], "AgentB")

    def test_expired_lease_is_taken_over_and_only_the_holder_renews(self):
        self.claim("T2", "--agent", "AgentA", "--timebox", "10", "--grace", "5", at=T0)  # expires T0 + 15 min
        self.assertIn("held by AgentA", self.refused("T2", "--agent", "AgentB", "--timebox", "10", at=T0 + 15 * MIN - 1))
        self.assertIn("renewed T2 for AgentA", self.claim("--renew", "T2", "--agent", "AgentA", at=T0 + 10 * MIN))
        self.assertIn("held by AgentA", self.refused("T2", "--agent", "AgentB", "--timebox", "10", at=T0 + 24 * MIN))
        took = self.claim("T2", "--agent", "AgentB", "--timebox", "10", at=T0 + 25 * MIN)
        self.assertIn("took over T2 for AgentB from AgentA (expired", took)
        takeover = self.log()[-1]
        self.assertEqual((takeover["op"], takeover["previous"], takeover["previous_state"], takeover["previous_at"]),
                         ("takeover", "AgentA", "expired", T0 + 25 * MIN))
        self.assertIn("not AgentA", self.refused("--renew", "T2", "--agent", "AgentA", at=T0 + 26 * MIN))

    def test_fleet_status_shows_leases_and_token_holders(self):
        self.claim("Live", "--agent", "AgentA", "--timebox", "60", at=T0)
        self.claim("Late", "--agent", "AgentB", "--timebox", "10", "--grace", "30", at=T0)
        self.claim("Gone", "--agent", "AgentC", "--timebox", "5", "--grace", "0", at=T0)
        self.claim("Done", "--agent", "AgentD", "--timebox", "5", at=T0)
        self.claim("--release", "Done", at=T0 + MIN)
        mem = self.base / "mem"
        # One build token held the way mem-run holds it, by slot-lock.py, until the holder reads EOF.
        holder = subprocess.Popen(
            [sys.executable, str(HERE / "slot-lock.py"), "--dir", str(mem / "tokens"), "--name", "token-%s.lock",
             "--slots", "1,2,3", "--env", "SOLARIS_BUILD_TOKEN", "--fd", "9", "--",
             sys.executable, "-c", "import sys; print('held', flush=True); sys.stdin.read()"],
            cwd=self.base, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.addCleanup(holder.wait, 30)
        self.addCleanup(holder.stdin.close)
        self.addCleanup(holder.stdout.close)
        self.assertTrue(select.select([holder.stdout], [], [], 30)[0], "token holder did not start")
        self.assertEqual(holder.stdout.readline().strip(), "held")
        status = subprocess.run(
            [str(HERE / "fleet-status"), "--mem-dir", str(mem)], capture_output=True, text=True, check=True,
            env=os.environ | {"SOLARIS_FLEET_NOW": str(T0 + 20 * MIN), "SOLARIS_SCENE_SLOT_DIR": str(self.base / "scenes")},
        ).stdout
        rows = {line.split()[0]: line for line in status.splitlines() if line.startswith("  ") and line.split()}
        self.assertIn("CLAIMS (2 live, 1 expired)", status)
        self.assertIn("AgentA", rows["Live"])
        self.assertIn("40m left", rows["Live"])
        self.assertIn("OVERRUN 10m", rows["Late"])
        self.assertIn("until", rows["Late"])
        self.assertIn("EXPIRED", rows["Gone"])
        self.assertIn("can be taken over", rows["Gone"])
        self.assertNotIn("Done", rows)
        self.assertIn("BUILD TOKENS (1/3 in use, 0 waiting)", status)
        self.assertIn(f"pid {holder.pid}", rows["1"])
        self.assertIn("print('held'", rows["1"])
        self.assertIn(f"in {self.base}", status)


if __name__ == "__main__":
    unittest.main()
