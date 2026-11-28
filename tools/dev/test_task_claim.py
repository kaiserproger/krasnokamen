"""Task leases (task-claim, task-release) and fleet-status: python3 tools/dev/test_task_claim.py

The fleet dir is private (KRASNOKAMEN_FLEET_DIR) and the clock injected (KRASNOKAMEN_FLEET_NOW), so expiry needs
no sleep. Claims run inside this test's own process, the way Main's subagents run inside Main's one
omp process: one live PID claims for two logical agents, so only the lease can decide."""
from __future__ import annotations

import fcntl
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
        patcher = mock.patch.dict(os.environ, {"KRASNOKAMEN_FLEET_DIR": str(self.fleet)})
        patcher.start()
        self.addCleanup(patcher.stop)

    def claim(self, *argv: str, at: int) -> str:
        os.environ["KRASNOKAMEN_FLEET_NOW"] = str(at)
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
                                 env=os.environ | {"KRASNOKAMEN_FLEET_NOW": str(T0 + 2 * MIN)})
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
             "--slots", "1,2,3", "--env", "KRASNOKAMEN_BUILD_TOKEN", "--fd", "9", "--",
             sys.executable, "-c", "import sys; print('held', flush=True); sys.stdin.read()"],
            cwd=self.base, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.addCleanup(holder.wait, 30)
        self.addCleanup(holder.stdin.close)
        self.addCleanup(holder.stdout.close)
        self.assertTrue(select.select([holder.stdout], [], [], 30)[0], "token holder did not start")
        self.assertEqual(holder.stdout.readline().strip(), "held")
        status = subprocess.run(
            [str(HERE / "fleet-status"), "--mem-dir", str(mem)], capture_output=True, text=True, check=True,
            env=os.environ | {"KRASNOKAMEN_FLEET_NOW": str(T0 + 20 * MIN), "KRASNOKAMEN_SCENE_SLOT_DIR": str(self.base / "scenes")},
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


class TimingTests(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.base = Path(tmp.name)
        self.fleet = self.base / "fleet"
        self.log = self.fleet / "claims.log"

    def env(self, ticks: int, *, wall: int | None = None, boot: str = "boot-a") -> dict:
        return os.environ | {
            "KRASNOKAMEN_FLEET_DIR": str(self.fleet),
            "KRASNOKAMEN_FLEET_NOW": str(T0 + (ticks if wall is None else wall)),
            "KRASNOKAMEN_FLEET_MONO_NS": str(ticks * 1_000_000_000),
            "KRASNOKAMEN_FLEET_BOOT_ID": boot,
        }

    def cli(self, name: str, *args: str, at: int, ok: bool = True,
            **clock) -> subprocess.CompletedProcess:
        result = subprocess.run([str(HERE / name), *args], cwd=self.base, env=self.env(at, **clock),
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0 if ok else 1, result.stdout + result.stderr)
        return result

    def report(self, task: str = "T", *, at: int, receipt: Path | None = None, **clock) -> dict:
        extra = ("--harness-receipt", str(receipt)) if receipt else ()
        return json.loads(self.cli("fleet-status", "--task", task, *extra, at=at, **clock).stdout)

    def claim(self, task: str = "T", *, at: int = 0, minutes: int = 1) -> str:
        self.cli("task-claim", task, "--agent", "Agent", "--timebox", str(minutes), "--grace", "0", at=at)
        return self.report(task, at=at)["lease"]

    def edge_args(self, lease: str, seq: int, phase: str, *, task: str = "T",
                  session: str = "s1", previous: str | None = None, agent: str = "Agent") -> list[str]:
        return [task, "--agent", agent, "--lease", lease, "--session", session, "--seq", str(seq),
                "--check-in", phase, *(["--replace-session", previous] if previous is not None else [])]

    def edge(self, lease: str, seq: int, phase: str, *, at: int, task: str = "T", session: str = "s1",
             previous: str | None = None, agent: str = "Agent", **kw) -> subprocess.CompletedProcess:
        return self.cli("task-claim", *self.edge_args(lease, seq, phase, task=task, session=session,
                        previous=previous, agent=agent), at=at, **kw)

    def records(self) -> list[dict]:
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_task_phase_totals_close_on_edges_not_status_or_release(self):
        lease = self.claim()
        claim_file = self.fleet / "claims/T.json"
        original = claim_file.read_bytes()
        self.edge(lease, 0, "start", at=3)
        for seq, (phase, when) in enumerate((("build-wait", 8), ("build-run", 10), ("active", 16),
                                            ("peer-wait", 23), ("review-wait", 28), ("owner-wait", 31),
                                            ("unknown", 35)), 1):
            self.edge(lease, seq, phase, at=when)
        open_report = self.report(at=37)
        self.assertEqual((open_report["phase"], open_report["phase_age_s"], open_report["next_seq"]),
                         ("unknown", 2, 8))
        self.assertEqual(open_report["closed_seconds"]["unknown"], 3)  # prefix only
        self.edge(lease, 8, "done", at=40)
        self.assertEqual(claim_file.read_bytes(), original)
        self.cli("task-release", "T", at=41)
        report = self.report(at=100)
        self.assertEqual((report["lease_state"], report["phase"], report["phase_age_s"]), ("released", "done", None))
        self.assertFalse(report["unmeasured_gap"])
        self.assertEqual(report["closed_seconds"], {
            "active": 12, "build-wait": 2, "build-run": 6, "peer-wait": 5,
            "review-wait": 3, "owner-wait": 4, "unknown": 8,
        })

    def test_concurrent_processes_append_one_edge_and_refuse_conflicts(self):
        lease = self.claim()
        self.edge(lease, 0, "start", at=0)
        commands = [("active", 3), ("build-wait", 1), ("build-wait", 1), ("peer-wait", 1)]
        children = []
        try:
            with (self.fleet / "claims/.lock").open("rb") as lock:
                fcntl.flock(lock, fcntl.LOCK_EX)
                for phase, seq in commands:
                    children.append(subprocess.Popen(
                        [str(HERE / "task-claim"), *self.edge_args(lease, seq, phase)],
                        cwd=self.base, env=self.env(5), stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE, text=True))
            for child in children:
                child.communicate(timeout=30)
        finally:
            for child in children:
                if child.returncode is None:
                    child.kill()
                    child.communicate(timeout=30)
        edges = [e for e in self.records() if e["op"] == "check-in"]
        self.assertEqual([edge["seq"] for edge in edges], [0, 1])
        self.assertEqual([c.returncode for c in children],
                         [1, 0, 0, 1] if edges[1]["phase"] == "build-wait" else [1, 1, 1, 0])
        self.assertIn("already checked in", self.edge(lease, 1, edges[1]["phase"], at=8).stdout)
        self.assertIn("out-of-order", self.edge(lease, 0, "owner-wait", at=9, ok=False).stderr)
        self.edge(lease, 2, "blocked", at=10)
        self.assertEqual(self.report(at=14)["phase"], "blocked")
        self.assertIn("already ended", self.edge(lease, 3, "done", at=15, ok=False).stderr)

    def test_missing_legacy_expired_and_renewed_gap_are_not_work(self):
        self.assertIn("no readable claim", self.cli("fleet-status", "--task", "T", at=0, ok=False).stderr)
        first = self.claim()
        path = self.fleet / "claims/T.json"
        legacy = json.loads(path.read_bytes())
        for key in ("lease_id", "created_clock", "lease_clock"):
            del legacy[key]
        path.write_text(json.dumps(legacy))
        report = self.report(at=4)
        self.assertNotEqual(first, report["lease"])
        self.assertEqual((report["phase"], report["phase_evidence"]), ("unknown", "no-check-in"))
        lease = report["lease"]
        self.edge(lease, 0, "start", at=5)
        self.assertTrue(self.report(at=6)["unmeasured_gap"])
        self.edge(lease, 1, "build-wait", at=10)
        expired = self.report(at=60)
        self.assertEqual((expired["lease_state"], expired["phase_age_s"], expired["phase_evidence"]),
                         ("expired", None, "lease-expired"))
        self.assertEqual(expired["closed_seconds"]["build-wait"], 0)
        self.edge(lease, 2, "done", at=61, ok=False)
        self.cli("task-claim", "--renew", "T", "--agent", "Agent", at=62)
        after = self.report(at=63)
        self.assertEqual(after["phase_evidence"], "no-check-in")
        self.assertNotEqual(after["lease"], lease)
        self.assertIn("stale lease", self.edge(lease, 2, "done", at=63, ok=False).stderr)
        self.assertEqual(sum(e["op"] == "check-in" for e in self.records()), 2)

    def test_same_agent_takeover_and_release_fence_old_commands(self):
        lease = self.claim()
        self.edge(lease, 0, "start", at=1)
        self.cli("task-release", "T", at=2)
        self.assertEqual(self.report(at=3)["phase_evidence"], "lease-released")
        anew = self.claim(at=3)
        self.assertNotEqual(anew, lease)
        self.assertIn("stale lease", self.edge(lease, 1, "done", at=4, ok=False).stderr)
        self.assertIn("wrong logical agent", self.edge(anew, 0, "start", at=4, agent="SomeoneElse", ok=False).stderr)
        self.edge(anew, 0, "start", at=4, session="new-owner")
        self.assertEqual(self.report(at=5)["closed_seconds"]["unknown"], 1)

    def test_same_boot_wall_jumps_and_restart_use_only_monotonic_time(self):
        lease = self.claim(minutes=2)
        self.edge(lease, 0, "start", at=5)
        self.edge(lease, 1, "peer-wait", at=10, wall=100_000)
        report = self.report(at=12, wall=-100_000)
        self.assertEqual((report["phase_age_s"], report["closed_seconds"]["active"]), (2, 5))
        self.assertIn("monotonic clock regressed", self.edge(lease, 2, "done", at=9, ok=False).stderr)
        self.edge(lease, 0, "start", at=15, session="s2", previous="s1")
        self.assertEqual(self.report(at=16)["closed_seconds"]["unknown"], 10)
        self.edge(lease, 2, "owner-wait", at=16, ok=False)
        report = self.report(at=2, wall=20, boot="boot-b")
        self.assertEqual((report["phase"], report["phase_age_s"]), ("unknown", None))
        self.edge(lease, 1, "owner-wait", at=2, wall=20, boot="boot-b", session="s2", ok=False)
        self.edge(lease, 0, "start", at=3, wall=21, boot="boot-b", session="s3", previous="s2")
        self.edge(lease, 1, "done", at=6, wall=24, boot="boot-b", session="s3")
        report = self.report(at=7, wall=25, boot="boot-b")
        self.assertEqual(report["closed_seconds"]["active"], 8)
        self.assertTrue(report["unmeasured_gap"])

    def test_live_renewal_does_not_extend_the_observation(self):
        lease = self.claim()
        self.edge(lease, 0, "start", at=1)
        self.edge(lease, 1, "peer-wait", at=2)
        self.cli("task-claim", "--renew", "T", "--agent", "Agent", at=30)
        report = self.report(at=60)
        self.assertEqual((report["lease"], report["lease_state"], report["phase"], report["phase_age_s"]),
                         (lease, "live", "unknown", None))
        self.edge(lease, 2, "done", at=61, ok=False)
        self.edge(lease, 0, "start", at=62, session="s2", previous="s1")
        self.assertEqual(self.report(at=63)["closed_seconds"]["unknown"], 61)

    def test_wall_fallback_session_restart_cannot_extend_a_lease_epoch(self):
        for legacy in (True, False):
            with self.subTest(legacy=legacy):
                task = "Legacy" if legacy else "Rebooted"
                self.claim(task)
                boot = "boot-a" if legacy else "boot-b"
                if legacy:
                    path = self.fleet / "claims" / f"{task}.json"
                    record = json.loads(path.read_bytes())
                    for key in ("lease_id", "created_clock", "lease_clock"):
                        del record[key]
                    path.write_text(json.dumps(record))
                lease = self.report(task, at=10, wall=10, boot=boot)["lease"]
                self.edge(lease, 0, "start", at=10, wall=10, boot=boot, task=task)
                # A slow/stepped wall clock must not extend the cutoff even if the
                # worker replaces its session before that cutoff elapses.
                self.edge(lease, 0, "start", at=40, wall=20, boot=boot, task=task,
                          session="s2", previous="s1")
                expired = self.report(task, at=61, wall=21, boot=boot)
                self.assertEqual((expired["lease_state"], expired["phase"], expired["phase_age_s"]),
                                 ("live", "unknown", None))
                self.edge(lease, 1, "done", at=61, wall=21, boot=boot, task=task, session="s2", ok=False)
                refused = self.edge(lease, 0, "start", at=62, wall=22, boot=boot, task=task,
                                    session="s3", previous="s2", ok=False)
                self.assertIn("cannot extend the lease", refused.stderr)
                self.assertEqual(self.report(task, at=63, wall=23, boot=boot)["closed_seconds"]["active"], 0)

                # Only a real renewal changes the epoch; the previous open tail
                # remains unknown and cannot be made into work retroactively.
                self.cli("task-claim", "--renew", task, "--agent", "Agent", at=63, wall=23, boot=boot)
                self.assertEqual(self.report(task, at=64, wall=24, boot=boot)["lease"], lease)
                self.edge(lease, 0, "start", at=64, wall=24, boot=boot, task=task,
                          session="s3", previous="s2")
                self.edge(lease, 1, "done", at=71, wall=31, boot=boot, task=task, session="s3")
                done = self.report(task, at=72, wall=32, boot=boot)
                self.assertEqual((done["phase"], done["closed_seconds"]["active"]), ("done", 7))
                starts = [e["renewals"] for e in self.records()
                          if e["task"] == task and e.get("phase") == "start"]
                self.assertEqual(starts, [0, 0, 1])

    def test_replayed_exact_done_row_is_idempotent_but_conflicting_row_is_not(self):
        lease = self.claim()
        self.edge(lease, 0, "start", at=1)
        self.edge(lease, 1, "done", at=6)
        expected = self.report(at=7)
        done = self.log.read_bytes().splitlines(keepends=True)[-1]
        with self.log.open("ab") as out:
            out.write(done)
        self.assertEqual(self.report(at=8), expected)
        with self.log.open("a") as out:
            conflict = json.loads(done) | {"ticks": 7_000_000_000}
            out.write(json.dumps(conflict) + "\n")
        flagged = self.report(at=9)
        self.assertEqual((flagged["phase"], flagged["phase_evidence"]), ("unknown", "incomplete-log"))
        self.assertEqual(flagged["closed_seconds"]["active"], 5)

    def test_torn_final_row_and_mismatched_epoch_do_not_claim_completion(self):
        lease = self.claim()
        self.edge(lease, 0, "start", at=1)
        self.edge(lease, 1, "review-wait", at=2)
        torn = json.dumps(self.records()[-1] | {"phase": "done", "seq": 2, "ticks": 4_000_000_000})
        with self.log.open("a") as out:
            out.write(torn)  # Valid-looking JSON without its final newline is still torn.
        self.assertEqual(self.report(at=5)["phase_evidence"], "incomplete-log")
        self.edge(lease, 2, "done", at=5, ok=False)
        self.edge(lease, 0, "start", at=6, previous="s1", session="s2")
        self.assertIn(torn + " !incomplete\n", self.log.read_text())
        report = self.report(at=7)
        self.assertEqual((report["phase"], report["closed_seconds"]["review-wait"]), ("active", 0))
        self.assertTrue(report["unmeasured_gap"])
        edge = json.loads(self.log.read_text().splitlines()[-1])
        with self.log.open("a") as out:
            out.write(json.dumps(edge | {"lease": "wrong-generation", "phase": "done", "seq": 1}) + "\n")
        self.assertEqual(self.report(at=8)["phase_evidence"], "incomplete-log")

    def test_overlap_and_harness_evidence_are_not_aggregated(self):
        a = self.claim("A")
        b = self.claim("B")
        self.edge(a, 0, "start", at=1, task="A")
        self.edge(b, 0, "start", at=1, task="B")
        self.edge(a, 1, "done", at=10, task="A")
        self.edge(b, 1, "done", at=10, task="B")
        receipt = self.base / "result.json"
        receipt.write_text(json.dumps({"started_at": "first", "status": "passed", "commands": ["private"]}))
        self.assertEqual(self.report("A", at=11, receipt=receipt)["harness_evidence"]["status"], "unavailable")
        receipt.write_text(json.dumps({"started_at": "first", "ended_at": "last", "status": "passed",
                                       "duration_seconds": 7, "details": {"duration_seconds": 6},
                                       "commands": ["private"]}))
        result = self.report("A", at=11, receipt=receipt)
        self.assertEqual(result["closed_seconds"]["active"], 9)
        self.assertEqual(self.report("B", at=11)["closed_seconds"]["active"], 9)
        self.assertEqual(result["harness_evidence"], {"status": "passed", "source": str(receipt),
                         "duration_seconds": 7, "details_duration_seconds": 6})
        self.assertNotIn("private", json.dumps(result))
        overview = self.cli("fleet-status", "--mem-dir", str(self.base / "mem"), at=11).stdout
        self.assertIn("0m left", overview)
        self.assertEqual(overview.count("| done"), 2)


if __name__ == "__main__":
    unittest.main()
