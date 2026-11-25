"""Unit tests for the join-lag gate: its options, verdicts and row-facing registration."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest import mock

from .backends import join_lag
from .profiles import PROFILES, ProfileContext


def context(*extra: str, client: str = "native", timeout: float | None = None) -> ProfileContext:
    return ProfileContext(artifact_dir=Path("/tmp/unused"), extra_args=list(extra),
                          client=client, timeout_seconds=timeout)


class ArgsTest(unittest.TestCase):
    def test_defaults_are_the_gate_options(self) -> None:
        timeout, tick_rate, lag_ticks = join_lag.validate_args(context())
        self.assertEqual((timeout, tick_rate, lag_ticks), (join_lag.TIMEOUT, 3, 8))

    def test_options_select_the_load_and_the_rate(self) -> None:
        timeout, tick_rate, lag_ticks = join_lag.validate_args(
            context("--tick-rate", "10", "--pump-lag-ticks", "6", timeout=600.0))
        self.assertEqual((timeout, tick_rate, lag_ticks), (600.0, 10, 6))

    def test_a_java_client_is_refused(self) -> None:
        with self.assertRaisesRegex(ValueError, "requires --client native"):
            join_lag.validate_args(context(client="java"))

    def test_out_of_range_options_are_refused(self) -> None:
        for argv in (("--tick-rate", "0"), ("--tick-rate", "21"),
                     ("--pump-lag-ticks", "0"), ("--pump-lag-ticks", "65"),
                     ("--scenario", "border-reveal")):
            with self.subTest(argv=argv):
                with self.assertRaises((ValueError, SystemExit)):
                    join_lag.validate_args(context(*argv))

    def test_the_timeout_bounds_are_enforced(self) -> None:
        for timeout in (0.0, 2400.5):
            with self.subTest(timeout=timeout):
                with self.assertRaisesRegex(ValueError, "timeout-seconds"):
                    join_lag.validate_args(context(timeout=timeout))


def record(**fields) -> dict:
    # lag 8 at 3 TPS = 2.7 s: past the 1.6 s publish budget, which the verdicts require.
    base = {"admitted": True, "landed_seconds": None, "timed_out": False, "lag": 8, "knockout": None}
    base.update(fields)
    return base


class JoinVerdictTest(unittest.TestCase):
    def test_a_lagged_only_skip_is_not_the_budget_witness(self) -> None:
        # pump_skipped_ticks also counts broadcast Lagged runs; the dedicated budget counter is the
        # only skip witness, so Lagged-only with no over-budget lag is null.
        ok, why = join_lag.join_verdict(record(lag=4, landed_seconds=2.0, knockout="landed"),
                                        True, 3.0, 0, 9)
        self.assertIsNone(ok)
        self.assertIn("Lagged", why)

    def test_a_join_inside_the_deadline_behind_a_skipping_pump_passes(self) -> None:
        ok, why = join_lag.join_verdict(record(lag=4, landed_seconds=2.5, knockout="landed"),
                                        True, 3.0, 4)
        self.assertTrue(ok)
        self.assertIn("2.5 s", why)
        self.assertIn("publish budget", why)

    def test_a_join_inside_the_deadline_behind_a_measured_over_budget_lag_passes(self) -> None:
        ok, why = join_lag.join_verdict(record(lag=26, landed_seconds=2.5, knockout="landed"), True, 3.0, 0)
        self.assertTrue(ok)
        self.assertIn("behind", why)

    def test_a_join_past_the_deadline_fails(self) -> None:
        ok, why = join_lag.join_verdict(record(landed_seconds=10.5, knockout="landed"), True, 3.0, 0)
        self.assertFalse(ok)
        self.assertIn("deadline", why)

    def test_a_confirmed_timeout_behind_an_over_budget_delay_fails(self) -> None:
        ok, why = join_lag.join_verdict(record(timed_out=True, knockout="timeout"), True, 3.0, 0)
        self.assertFalse(ok)
        self.assertIn("missed", why)

    def test_no_admission_is_infrastructure_not_a_verdict(self) -> None:
        ok, why = join_lag.join_verdict(record(admitted=False), True, 3.0, 0)
        self.assertIsNone(ok)
        self.assertIn("admission", why)

    def test_an_mcp_failure_without_an_outcome_is_not_a_verdict(self) -> None:
        ok, why = join_lag.join_verdict(record(knockout=None), True, 3.0, 0)
        self.assertIsNone(ok)
        self.assertIn("neither", why)

    def test_a_lag_inside_the_publish_budget_without_skips_is_inconclusive(self) -> None:
        # lag 4 at 3 TPS = 1.33 s with no skip: the receipt 20261120T022817 case.
        ok, why = join_lag.join_verdict(record(lag=4, landed_seconds=2.0, knockout="landed"), True, 3.0, 0)
        self.assertIsNone(ok)
        self.assertIn("budget", why)

    def test_a_lag_inside_the_budget_at_a_faster_rate_is_inconclusive(self) -> None:
        # lag 4 at 14.65 TPS = 0.27 s, no skip: a rate that never settled to 3 must not pass.
        ok, why = join_lag.join_verdict(record(lag=4, landed_seconds=0.5, knockout="landed"), True, 14.65, 0)
        self.assertIsNone(ok)
        self.assertIn("budget", why)

    def test_an_unconfirmed_tick_rate_is_inconclusive(self) -> None:
        ok, why = join_lag.join_verdict(record(landed_seconds=2.0, knockout="landed"), True, None, 5)
        self.assertIsNone(ok)
        self.assertIn("tick rate", why)

    def test_neither_a_skip_nor_an_over_budget_lag_is_inconclusive(self) -> None:
        ok, why = join_lag.join_verdict(record(lag=2, landed_seconds=2.0, knockout="landed"), True, 3.0, 0)
        self.assertIsNone(ok)
        self.assertIn("neither", why)

    def test_an_unset_knob_proves_nothing(self) -> None:
        ok, why = join_lag.join_verdict(record(landed_seconds=30.0, knockout="landed"), False, 3.0, 5)
        self.assertIsNone(ok)
        self.assertIn("did not occur", why)

    def test_a_core_halt_is_row_two_not_row_one(self) -> None:
        # A landing inside the deadline is row 1's pass even when the core halts later; the halt is
        # judged by live_pump_does_not_halt.
        ok, why = join_lag.join_verdict(record(landed_seconds=2.5, knockout="landed"), True, 3.0, 4)
        self.assertTrue(ok)
        self.assertIn("2.5 s", why)

    def test_a_join_record_stores_its_landing_stamp_as_iso_text(self) -> None:
        # The receipt is JSON: a datetime in `details.join` would fail serialization (the
        # 20261124T181257 gate run did), so the record carries the ISO form and the verdict parses it.
        import json

        with tempfile.TemporaryDirectory() as temp:
            log = Path(temp) / "server.log"
            log.write_text(
                "2026-11-24T14:01:24.500000Z  INFO connection: login admitted name=JoinLagJoiner connection=4\n"
                "2026-11-24T14:01:26.400000Z DEBUG pump: join landed connection=4 "
                "core_tick_seen_at_wait=Some(400) core_tick_seen=Some(406) pump_tick=380\n")
            rec = join_lag.join_record(log, "JoinLagJoiner")
            json.dumps(rec)  # must not raise
            self.assertEqual(rec["landed_at"], "2026-11-24T14:01:26.400000+00:00")
            ok, _ = join_lag.join_verdict(rec, True, 3.0, 0)
            self.assertTrue(ok)

    def test_a_join_landed_after_the_halt_was_drained_by_it(self) -> None:
        # The four wave-2 controls: the halt line precedes `join landed`, so the landing is the
        # halt's own queue drain — row 1's deadline was never exercised (review wave2-8 M2).
        from datetime import datetime, timedelta, timezone

        admitted = datetime(2026, 11, 24, 14, 59, 0, tzinfo=timezone.utc)
        ok, why = join_lag.join_verdict(
            record(landed_seconds=9.2, landed_at=admitted + timedelta(seconds=9.264), knockout="landed"),
            True, 3.0, 0, halt_at=admitted + timedelta(seconds=9.240))
        self.assertIsNone(ok)
        self.assertIn("halt", why)
        self.assertIn("drained", why)

    def test_a_join_landed_before_the_halt_still_reads_its_own_deadline(self) -> None:
        from datetime import datetime, timedelta, timezone

        admitted = datetime(2026, 11, 24, 14, 59, 0, tzinfo=timezone.utc)
        ok, why = join_lag.join_verdict(
            record(landed_seconds=2.0, landed_at=admitted + timedelta(seconds=2.0), knockout="landed"),
            True, 3.0, 0, halt_at=admitted + timedelta(seconds=9.240))
        self.assertTrue(ok)
        self.assertIn("2.0 s", why)

    def test_a_join_whose_client_never_reached_play_is_not_a_pass(self) -> None:
        # 145907 recorded `joiner_entered_play: false`: a landing without a client in play is not
        # evidence that the pump served the join.
        ok, why = join_lag.join_verdict(record(landed_seconds=9.3, knockout="landed"), True, 3.0, 0,
                                        joiner_in_play=False)
        self.assertIsNone(ok)
        self.assertIn("never reached play", why)

    def test_a_witness_rate_from_another_phase_does_not_witness_the_delay(self) -> None:
        # The witness joined at 20 TPS before `/tick rate`; its own record carries that rate, and
        # the target applied rate must not lend the 3 TPS verdict (review wave2-8 M4).
        ok, why = join_lag.join_verdict(
            record(rate=20.0, lag=31, landed_seconds=2.0, knockout="landed",
                   landed_at=None), True, 3.0, 0)
        self.assertIsNone(ok)
        self.assertIn("neither", why)


class HaltVerdictTest(unittest.TestCase):
    def test_a_ticking_core_behind_a_witnessed_delay_passes(self) -> None:
        ok, _ = join_lag.halt_verdict(True, record(), 3.0, 4, 100, 160, None, window_seconds=13.0)
        self.assertTrue(ok)

    def test_a_halted_core_under_a_witnessed_delay_fails(self) -> None:
        ok, _ = join_lag.halt_verdict(True, record(), 3.0, 0, 100, 160, "ERROR edge pump not draining")
        self.assertFalse(ok)

    def test_a_stuck_simulation_tick_fails(self) -> None:
        ok, why = join_lag.halt_verdict(True, record(), 3.0, 4, 100, 100, None)
        self.assertFalse(ok)
        self.assertIn("did not advance", why)

    def test_a_halt_watch_shorter_than_the_stall_window_is_inconclusive(self) -> None:
        # The PASS 151525 window was ~3 s at ~3 TPS, shorter than the 10 s outbox stall watch: that
        # core was never given the chance to halt, so the pass proved nothing (review wave2-8 M3).
        ok, why = join_lag.halt_verdict(True, record(), 3.0, 4, 100, 110, None, window_seconds=3.3)
        self.assertIsNone(ok)
        self.assertIn("stall window", why)

    def test_a_full_stall_window_passes(self) -> None:
        # 13.3 s of wall clock: past the 10 s watch plus the margin.
        ok, _ = join_lag.halt_verdict(True, record(), 3.0, 4, 100, 140, None, window_seconds=13.3)
        self.assertTrue(ok)

    def test_an_unwitnessed_delay_proves_nothing(self) -> None:
        ok, why = join_lag.halt_verdict(True, record(lag=0), 3.0, 0, 100, 160, None)
        self.assertIsNone(ok)
        self.assertIn("neither", why)

    def test_a_ticking_core_without_a_witness_is_inconclusive(self) -> None:
        ok, why = join_lag.halt_verdict(True, record(lag=1), 20.0, 0, 100, 160, None)
        self.assertIsNone(ok)
        self.assertIn("neither", why)


class DispositionTest(unittest.TestCase):
    """A false verdict reads as the row's symptom in the ledger; an unreached condition must not."""

    def test_a_clean_run_passes(self) -> None:
        self.assertEqual(join_lag.disposition({"a": True, "b": True}), ("passed", []))

    def test_a_real_symptom_fails_and_is_named(self) -> None:
        self.assertEqual(join_lag.disposition({"a": True, "b": False}), ("failed", ["b"]))

    def test_an_unreached_condition_is_inconclusive_never_false(self) -> None:
        outcome, names = join_lag.disposition({"a": None, "b": True})
        self.assertEqual(outcome, "inconclusive")
        self.assertEqual(names, ["a"])

    def test_a_symptom_outranks_an_unreached_condition(self) -> None:
        self.assertEqual(join_lag.disposition({"a": None, "b": False}), ("failed", ["b"]))

    def test_neither_verdict_returns_false_without_the_delay(self) -> None:
        self.assertIsNone(join_lag.join_verdict(record(landed_seconds=30.0, knockout="landed"),
                                                False, 3.0, 5)[0])
        self.assertIsNone(join_lag.halt_verdict(False, record(), 3.0, 5, 100, 160, None)[0])


class EngagementTest(unittest.TestCase):
    def test_the_delay_lines_warn_says_it_ran(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text("INFO tick\nWARN " + join_lag.ENGAGED_LINE + ' lag_ticks=32\n')
            self.assertTrue(join_lag.engaged(log))

    def test_a_quiet_log_has_no_engagement(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text("INFO tick\n")
            self.assertFalse(join_lag.engaged(log))

    def test_a_missing_log_has_no_engagement(self) -> None:
        self.assertFalse(join_lag.engaged(Path("/tmp/definitely-missing-join-lag.log")))


class HaltLineTest(unittest.TestCase):
    def test_a_halt_line_is_found(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text("INFO tick\nERROR edge pump not draining; core halted\n")
            self.assertIn("edge pump not draining", join_lag.halt_line(log) or "")

    def test_a_healthy_log_has_none(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text("INFO tick\nINFO joined\n")
            self.assertIsNone(join_lag.halt_line(log))

    def test_a_missing_log_has_none(self) -> None:
        self.assertIsNone(join_lag.halt_line(Path("/tmp/definitely-missing-join-lag.log")))


class JoinRecordTest(unittest.TestCase):
    LOG = (
        "2026-11-20T01:18:15.209868Z  INFO mc_edge::server::connection: "
        "login admitted peer=127.0.0.1:53492 connection=2 name=JoinLagWitness protocol=777 release=\"26.3\"\n"
        "2026-11-20T01:18:17.827594Z  INFO mc_edge::server::connection: "
        "login admitted peer=127.0.0.1:53510 connection=4 name=JoinLagJoiner protocol=777 release=\"26.3\"\n"
        "2026-11-20T01:18:18.000000Z DEBUG mc_edge::server::pump: connection = 2 joined=1 \"join landed\" "
        "core_tick_seen=Some(120) pump_tick=118\n"
        "2026-11-20T01:18:30.827594Z DEBUG mc_edge::server::pump: connection = 4 joined=1 \"join landed\" "
        "core_tick_seen=Some(340) pump_tick=310\n"
    )
    TIMEOUT = (
        "2026-11-20T01:18:17.827594Z  INFO mc_edge::server::connection: "
        "login admitted peer=127.0.0.1:53510 connection=4 name=JoinLagJoiner protocol=777 release=\"26.3\"\n"
        "2026-11-20T01:18:27.697423Z  WARN mc_edge::server::connection: connection closed "
        "peer=127.0.0.1:44294 connection=4 stage=\"join-wait\" name=JoinLagJoiner "
        "error=timed out waiting for join (core tick 351 seen when the wait began, 358 now; "
        "pump published tick 349)\n"
    )

    def test_a_landing_carries_the_interval_and_the_pump_lag(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(self.LOG)
            joiner = join_lag.join_record(log, "JoinLagJoiner")
            self.assertTrue(joiner["admitted"])
            self.assertAlmostEqual(joiner["landed_seconds"], 13.0, places=3)
            self.assertEqual(joiner["lag"], 30)
            witness = join_lag.join_record(log, "JoinLagWitness")
            self.assertAlmostEqual(witness["landed_seconds"], 2.79, places=2)
            self.assertEqual(witness["lag"], 2)

    def test_a_timeout_is_the_servers_own_deadline_with_its_ticks(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(self.TIMEOUT)
            joiner = join_lag.join_record(log, "JoinLagJoiner")
            self.assertTrue(joiner["timed_out"])
            self.assertIsNone(joiner["landed_seconds"])
            self.assertEqual(joiner["lag"], 9)

    def test_each_record_carries_the_rate_of_its_own_wait(self) -> None:
        """The server prints the core ticks around the wait, so its rate survives a held reply."""
        timeout_line = (
            "2026-11-24T14:02:06.366330Z  WARN mc_edge::server::connection: connection closed "
            "peer=127.0.0.1:1 connection=4 stage=\"join-wait\" name=JoinLagJoiner "
            "error=timed out waiting for join (core tick 251 seen when the wait began, 281 now; "
            "pump published tick 249)\n"
        )
        landed_line = (
            "2026-11-24T12:38:23.719152Z DEBUG mc_edge::server::pump: join landed connection=4 "
            "path=\"woken\" core_tick_seen_at_wait=Some(241) core_tick_seen=Some(247) pump_tick=243\n"
        )
        admitted = (
            "2026-11-24T12:38:21.817891Z  INFO mc_edge::server::connection: login admitted "
            "peer=127.0.0.1:46632 connection=4 name=JoinLagJoiner protocol=777 release=\"26.3\"\n"
        )
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(admitted + landed_line)
            landed = join_lag.join_record(log, "JoinLagJoiner")
            # 6 core ticks advanced over the 1.901 s wait: the rate the join actually ran at.
            self.assertAlmostEqual(landed["rate"], 6 / 1.901261, places=3)
            log.write_text(admitted + timeout_line)
            timed_out = join_lag.join_record(log, "JoinLagJoiner")
            # 30 ticks over the server's own 10 s join deadline.
            self.assertAlmostEqual(timed_out["rate"], 3.0, places=6)

    def test_an_unadmitted_client_has_an_empty_record(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(self.LOG)
            self.assertFalse(join_lag.join_record(log, "Nobody")["admitted"])

    def test_a_missing_log_has_an_empty_record(self) -> None:
        self.assertFalse(join_lag.join_record(Path("/tmp/definitely-missing-join-lag.log"),
                                              "JoinLagJoiner")["admitted"])


class AttributionTest(unittest.TestCase):
    """Each row's verdict only from its own symptom."""

    def test_a_halt_without_a_join_attempt_leaves_row_one_without_an_outcome(self) -> None:
        # The early-halt path in the gate: the halt fires before the joiner launches. Row 1 has no
        # join outcome (null); row 2 is the halt (false) under the witness.
        rec = record(landed_seconds=None, knockout=None)
        lag_ok, _ = join_lag.witness(rec, True, 3.0, 0)
        self.assertTrue(lag_ok, "the witness client own landing carries the delay")
        self.assertIsNone(join_lag.join_verdict(rec, True, 3.0, 0)[0])
        self.assertFalse(join_lag.halt_verdict(True, rec, 3.0, 0, 100, 160,
                                               "ERROR edge pump not draining")[0])


class AppliedRateTest(unittest.TestCase):
    def test_the_tick_query_reply_names_the_rate(self) -> None:
        self.assertEqual(join_lag.applied_tick_rate(["Target tick rate: 3.0", "hello"]), 3.0)
        self.assertEqual(join_lag.applied_tick_rate(["Some chat", "Target tick rate: 14.65"]), 14.65)

    def test_a_quiet_chat_has_no_rate(self) -> None:
        self.assertIsNone(join_lag.applied_tick_rate([]))
        self.assertIsNone(join_lag.applied_tick_rate(["Set the target tick rate to 3.0 per second"]))


class WitnessRateFallbackTest(unittest.TestCase):
    """A join whose `/tick query` reply the pump held still proves the interval, from its own ticks."""

    def test_the_records_own_rate_witnesses_the_delay(self) -> None:
        # 32 ticks behind at the 3 TPS the wait printed = 10.7 s, past the 1.6 s budget.
        rec = record(lag=32, timed_out=True, knockout="timeout", rate=3.0)
        ok, why = join_lag.join_verdict(rec, True, None, 0)
        self.assertFalse(ok, "a join that missed the deadline behind the witnessed delay is row 1")
        self.assertIn("3 TPS", why)

    def test_the_records_own_rate_wins_over_the_target_rate(self) -> None:
        # The witness joined before `/tick rate`; its own record's measured rate is the only one
        # that belongs to its wait (review wave2-8 M4).
        rec = record(lag=32, timed_out=True, knockout="timeout", rate=20.0)
        ok, why = join_lag.join_verdict(rec, True, 3.0, 0)
        self.assertIsNone(ok)
        self.assertIn("20 TPS", why)

    def test_a_record_without_any_interval_stays_inconclusive(self) -> None:
        ok, why = join_lag.join_verdict(record(lag=32, timed_out=True, knockout="timeout"), True, None, 0)
        self.assertIsNone(ok)
        self.assertIn("tick rate", why)


class FakeSteps:
    """The witness's `ProbeSteps` surface `_gate` uses, with the call order recorded.

    `halt_after` names the wait after which the core's halt has disconnected the witness: every
    later observation fails the `in_play` check exactly as `ProbeSteps.observe` records it.
    """

    def __init__(self, states: dict, events: list[str], *, halt_after: str | None = None,
                 halt_log: Path | None = None, halt_text: str = "") -> None:
        self.states = states
        self.events = events
        self.halt_after = halt_after
        self.halt_log = halt_log
        self.halt_text = halt_text
        self.halted = False
        self.deadline = 900.0
        self.client = None
        self.game_time = 400
        self.report = {"id": "native-join-lag-witness", "passed": False, "assertions": {},
                       "fixture_commands": [], "details": {}}

    def check(self, name: str, condition: bool) -> None:
        self.report["assertions"][name] = bool(condition)
        if not condition:
            raise RuntimeError(f"{self.report['id']} assertion failed: {name}")

    def _observe(self) -> None:
        self.check("in_play", not self.halted)

    def observe(self, *, wait_for_play: bool = False, outside_play: bool = False):
        self.events.append("observe")
        self.game_time += 4
        if not outside_play:
            self._observe()
        return {"time": {"game_time": self.game_time}, "in_play": not self.halted}

    def _halt(self) -> None:
        if self.halted or self.halt_log is None:
            return
        self.halted = True
        # The real server writes its halt line before the disconnect that follows it.
        self.halt_log.write_text(self.halt_log.read_text() + self.halt_text)

    def wait(self, predicate, label: str, *, timeout_seconds: float | None = None,
             wait_for_play: bool = False, outside_play: bool = False, record: bool = True):
        self.events.append(f"wait:{label}")
        if not outside_play:
            self._observe()
        value = self.states.get(label, TimeoutError(f"timed out waiting for {label}"))
        if label == self.halt_after:
            self._halt()
        if isinstance(value, Exception):
            if "in_play" in str(value):
                self.check("in_play", False)
            raise value
        self.check(label, bool(predicate(value)))
        return value

    def fixture(self, command: str) -> None:
        self.events.append(f"fixture:{command}")


class FakeJoiner:
    """The joiner's `ProbeSteps`: its own tools answer from `results`."""

    def __init__(self, _client, _artifact_dir, _timeout, _scenario, results=None) -> None:
        self.calls: list[str] = []
        self.results = results or {"minecraft_wait_for_play": {"in_play": True},
                                   "minecraft_observe": {"in_play": True}}
        self.report = {"id": "native-join-lag-joiner", "assertions": {}}

    def __enter__(self) -> FakeJoiner:
        return self

    def call(self, name: str, arguments=None):
        self.calls.append(name)
        return self.results.get(name, {})

    def check(self, name: str, condition: bool) -> None:
        self.report["assertions"][name] = bool(condition)
        if not condition:
            raise RuntimeError(f"native-join-lag-joiner assertion failed: {name}")


class GateWalkTest(unittest.TestCase):
    """The walk order the row needs (review REVIEW-wave2-2 B1/N3), without a client or a server."""

    WITNESS = (
        "2026-11-24T14:01:21.400000Z  INFO mc_edge::server::connection: login admitted "
        "peer=127.0.0.1:2 connection=2 name=JoinLagWitness protocol=777 release=\"26.3\"\n"
        "2026-11-24T14:01:24.400000Z DEBUG mc_edge::server::pump: join landed connection=2 "
        "path=\"woken\" core_tick_seen_at_wait=Some(223) core_tick_seen=Some(232) pump_tick=201\n"
    )
    ADMITTED = (
        "2026-11-24T14:01:24.500000Z  INFO mc_edge::server::connection: login admitted "
        "peer=127.0.0.1:1 connection=4 name=JoinLagJoiner protocol=777 release=\"26.3\"\n"
    )
    TIMED_OUT = (
        "2026-11-24T14:01:34.600000Z  WARN mc_edge::server::connection: connection closed "
        "peer=127.0.0.1:1 connection=4 stage=\"join-wait\" name=JoinLagJoiner "
        "error=timed out waiting for join (core tick 251 seen when the wait began, 281 now; "
        "pump published tick 249)\n"
    )
    LANDED = (
        "2026-11-24T14:01:26.400000Z DEBUG mc_edge::server::pump: join landed connection=4 "
        "path=\"woken\" core_tick_seen_at_wait=Some(400) core_tick_seen=Some(406) pump_tick=380\n"
    )
    HALT = ("2026-11-24T14:01:34.700000Z ERROR mc_core::core::tick_loop: core tick owner halted "
            "(fail closed) tick=344 cause=\"edge pump not draining for 10 s, 2884 bytes queued\"\n")

    def _run_gate(self, *, join_lines: str, halt: bool, states: dict, halt_after: str | None,
                  events: list[str]):
        """`_gate` against a log file that plays the server's own lines on each event wait."""
        root = tempfile.TemporaryDirectory()
        self.addCleanup(root.cleanup)
        artifact = Path(root.name)
        log = artifact / "server.log"
        log.write_text("WARN " + join_lag.ENGAGED_LINE + " lag_ticks=8\n" + self.WITNESS)
        a = FakeSteps(states, events, halt_after=halt_after,
                      halt_log=log if halt else None, halt_text=self.HALT)
        joiner_box: list[FakeJoiner] = []

        def wait_for_path(path, timeout, condition, process=None):
            matched, value = condition()
            if matched:
                return value
            if timeout == join_lag.JOIN_RECORD_SECONDS:
                path.write_text(path.read_text() + join_lines + (self.HALT if halt else ""))
            elif halt:
                path.write_text(path.read_text() + self.HALT)
            matched, value = condition()
            if matched:
                return value
            raise TimeoutError(f"filesystem event timeout for {path}")

        def launch_joiner():
            events.append("launch:joiner")
            joiner_box.append(FakeJoiner(None, artifact, 60.0, "native-join-lag-joiner"))
            return object(), object()

        def fake_steps(*_args, **_kwargs):
            return joiner_box[0]

        ticks = [80, 0 if halt else 130]
        with mock.patch.object(join_lag.runtime, "wait_for_path_condition", wait_for_path), \
                mock.patch.object(join_lag, "ProbeSteps", fake_steps), \
                mock.patch.object(join_lag, "_safe_tick", lambda port: ticks.pop(0)), \
                mock.patch.object(join_lag, "_skip_counters", lambda port: (0, 0)), \
                mock.patch.object(join_lag, "_stat", lambda port: {"publish": {}, "tps": 3.0}):
            detail, verdicts = join_lag._gate(a, context(), artifact, 3.0, "127.0.0.1:1", log,
                                              1, None, launch_joiner)
        return a, detail, verdicts

    def test_the_joiner_is_launched_before_the_rate_reply_is_awaited(self) -> None:
        events: list[str] = []
        a, _, _ = self._run_gate(join_lines=self.ADMITTED + self.LANDED, halt=False,
                                 states={"player_present": {"player": {"x": 0.5}}},
                                 halt_after=None, events=events)
        self.assertEqual([name for name in events if name != "event:10.0"],
                         ["wait:player_present", "fixture:tick rate 3",
                          "launch:joiner", "fixture:tick query", "wait:tick_rate_applied"])

    def test_a_held_rate_reply_is_tolerated_and_its_verdict_comes_from_the_joiner(self) -> None:
        events: list[str] = []
        a, detail, verdicts = self._run_gate(
            join_lines=self.ADMITTED + self.LANDED, halt=False,
            states={"player_present": {"player": {"x": 0.5}}}, halt_after=None, events=events)
        self.assertIn("tick_rate_applied", str(detail.get("rate_wait_error")))
        self.assertIsNone(detail["applied_tick_rate"])
        self.assertEqual(verdicts["join_behind_slow_pump"], True)
        self.assertEqual(a.report["assertions"]["in_play"], True)
        self.assertNotIn("play_lost", detail)

    def test_a_halt_after_play_does_not_record_the_client_as_never_in_play(self) -> None:
        events: list[str] = []
        a, detail, verdicts = self._run_gate(
            join_lines=self.ADMITTED + self.TIMED_OUT, halt=True,
            states={"player_present": {"player": {"x": 0.5}}}, halt_after="player_present",
            events=events)
        # The witness's loss of play is the halt's own disconnect: recorded, not an in_play failure.
        self.assertNotIn("in_play", a.report["assertions"])
        self.assertTrue(a.report["assertions"]["client_reached_play"])
        self.assertTrue(detail.get("play_lost"))
        # Both rows read the joiner's record: the join missed the deadline behind the witnessed
        # delay, and the core halted under it.
        self.assertEqual(verdicts["join_behind_slow_pump"], False)
        self.assertEqual(verdicts["live_pump_does_not_halt"], False)
        self.assertIn("missed", detail["join_verdict"])
        self.assertIn("halted", detail["halt_verdict"])

    def test_a_join_that_never_reached_the_server_has_no_verdict(self) -> None:
        events: list[str] = []
        a, detail, verdicts = self._run_gate(
            join_lines="", halt=True,
            states={"player_present": {"player": {"x": 0.5}}}, halt_after="player_present",
            events=events)
        # Row 2 reads the joiner's own record only: the witness joined before `/tick rate`, so its
        # 20 TPS lag cannot witness a 3 TPS delay (review wave2-8 M4).
        self.assertIsNone(verdicts["join_behind_slow_pump"])
        self.assertIsNone(verdicts["live_pump_does_not_halt"])
        self.assertIn("never reached the server", detail["halt_verdict"])


class FailedReceiptVerdictTest(unittest.TestCase):
    """A promotion control reads the gate's own verdicts out of a failed receipt."""

    def test_a_failed_receipt_carries_the_red_verdict(self) -> None:
        from . import ledger

        receipt = {"status": "failed",
                   "error": "native-join-lag-witness assertion failed: join_behind_slow_pump",
                   "details": {"verdicts": {"join_behind_slow_pump": False,
                                            "live_pump_does_not_halt": True}}}
        self.assertTrue(ledger._failing_verdict(receipt, "join-lag-gate", "join_behind_slow_pump"))
        self.assertFalse(ledger._failing_verdict(receipt, "join-lag-gate", "live_pump_does_not_halt"))

    def test_a_receipt_without_verdicts_is_not_a_red(self) -> None:
        from . import ledger

        receipt = {"status": "failed",
                   "error": "native-join-lag-witness assertion failed: in_play",
                   "details": {"scenario": {"assertions": {"in_play": False}}}}
        self.assertFalse(ledger._failing_verdict(receipt, "join-lag-gate", "join_behind_slow_pump"))


class RegistrationTest(unittest.TestCase):
    def test_the_profile_is_a_windowed_real_client_gate(self) -> None:
        from . import regressions

        self.assertEqual(PROFILES["join-lag-gate"]["scope"], "real-client")
        self.assertIn("join-lag-gate", regressions._WINDOWED_PROFILES)

    def test_every_verdict_is_a_registered_row_ref(self) -> None:
        from . import regressions

        for verdict in join_lag.CRITERIA:
            with self.subTest(verdict=verdict):
                self.assertTrue(regressions.is_graphical_ref(f"harness:join-lag-gate:{verdict}"))
                self.assertIn(verdict, regressions._BOOLEAN_PROFILE_VERDICTS["join-lag-gate"])

    def test_a_foreign_verdict_is_not_a_ref(self) -> None:
        from . import regressions

        self.assertFalse(regressions.is_graphical_ref("harness:join-lag-gate:not_a_verdict"))
        self.assertFalse(regressions.is_graphical_ref("client:join-lag-gate:join_behind_slow_pump"))


class PlayLostTest(unittest.TestCase):
    """Only the core halt excuses a lost play check; any other disconnect stays the client's own."""

    def _steps(self) -> mock.Mock:
        steps = mock.Mock()
        steps.report = {"assertions": {"in_play": False}}
        return steps

    def test_a_lost_play_without_a_halt_keeps_its_failed_check(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            log = Path(temp) / "server.log"
            log.write_text("2026-11-24T14:59:00Z INFO kicked JoinLagJoiner keepalive\n")
            steps = self._steps()
            detail: dict = {}
            join_lag._note_play_lost(steps, detail, RuntimeError("gone"), log)
            self.assertIs(steps.report["assertions"]["in_play"], False)
            self.assertNotIn("play_lost", detail)

    def test_the_core_halt_excuses_the_lost_play(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            log = Path(temp) / "server.log"
            log.write_text("2026-11-24T14:59:00Z ERROR edge pump not draining: halted\n")
            steps = self._steps()
            detail: dict = {}
            join_lag._note_play_lost(steps, detail, RuntimeError("gone"), log)
            self.assertNotIn("in_play", steps.report["assertions"])
            self.assertTrue(steps.report["assertions"]["client_reached_play"])
            self.assertEqual(detail["play_lost"], ["gone"])


MIXED_RATE_LOG = """\
2026-11-24T18:19:29.600000Z DEBUG mc_edge::server::pump: pump published tick tick=85 viewers=1 elapsed_us=1200
2026-11-24T18:19:30.000000Z DEBUG mc_edge::server::pump: pump published tick tick=86 viewers=1 elapsed_us=1100
2026-11-24T18:19:31.393470Z DEBUG mc_edge::server::pump: pump published tick tick=241 viewers=1 elapsed_us=3764
2026-11-24T18:19:31.790858Z DEBUG mc_edge::server::pump: pump published tick tick=249 viewers=1 elapsed_us=3190
2026-11-24T18:19:32.589589Z  INFO mc_edge::server::connection: login admitted peer=127.0.0.1:42548 connection=5 name=JoinLagJoiner protocol=777 release="26.3"
2026-11-24T18:19:33.361663Z DEBUG mc_edge::server::pump: pump published tick tick=280 viewers=1 elapsed_us=2900
2026-11-24T18:19:35.992221Z DEBUG mc_edge::server::pump: pump published tick tick=292 viewers=1 elapsed_us=2019
2026-11-24T18:19:38.656790Z DEBUG mc_edge::server::pump: pump published tick tick=300 viewers=2 elapsed_us=2797
"""
EXTENDED_LINE = ("2026-11-24T18:19:50.900000Z DEBUG mc_edge::server::pump: "
                 "pump published tick tick=337 viewers=2 elapsed_us=2797\n")


class PublishWindowTest(unittest.TestCase):
    """The halt watch's window is wall clock from the server's stamps (review wave2-9 Major-1)."""

    def test_a_startup_interval_does_not_name_the_applied_rate(self) -> None:
        # server.log of receipt 201826 has tick 85 -> 86 in 0.40 s at startup: 2.5 TPS frame spacing
        # with 20 TPS of ticks, i.e. <= 1.25*3 and indistinguishable from the applied rate by the
        # interval alone. The search must start after the admission (review wave2-12 Major-2).
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(MIXED_RATE_LOG)
            window = join_lag.publish_window(log, join_lag.JOINER, 3.0)
            self.assertEqual(window["admitted_at"], "2026-11-24T18:19:32.589589+00:00")
            self.assertEqual(window["applied_at"], "2026-11-24T18:19:38.656790+00:00")
            self.assertEqual(window["start_at"], "2026-11-24T18:19:38.656790+00:00")
            self.assertEqual(window["end_tick"], 300)
            self.assertLess(window["seconds"], join_lag.HALT_WINDOW_SECONDS)
        # The startup interval alone would have named 18:19:30 as the applied rate before the fix.
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(MIXED_RATE_LOG.replace('name=JoinLagJoiner', 'name=SomeoneElse'))
            window = join_lag.publish_window(log, join_lag.JOINER, 3.0)
            self.assertIsNone(window["start_at"])  # no admission: no window, never the startup line

    def test_the_deadline_is_recomputed_from_the_current_start(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(MIXED_RATE_LOG)
            early = join_lag.publish_window(log, join_lag.JOINER, 3.0)
            self.assertFalse(join_lag.window_complete(early))
            # A publication 13 s after the admission but only 6.4 s after the applied rate: a
            # deadline computed once from the admission would call this complete.
            log.write_text(MIXED_RATE_LOG + "2026-11-24T18:19:45.900000Z DEBUG mc_edge::server::pump: pump published tick tick=310 viewers=2 elapsed_us=2797\n")
            moved = join_lag.publish_window(log, join_lag.JOINER, 3.0)
            self.assertEqual(moved["start_at"], "2026-11-24T18:19:38.656790+00:00")
            self.assertFalse(join_lag.window_complete(moved))
            log.write_text(MIXED_RATE_LOG + "2026-11-24T18:19:51.000000Z DEBUG mc_edge::server::pump: pump published tick tick=330 viewers=2 elapsed_us=2797\n")
            self.assertTrue(join_lag.window_complete(join_lag.publish_window(log, join_lag.JOINER, 3.0)))

    def test_the_gate_recompute_moves_its_deadline_with_the_windows_start(self) -> None:
        # The gate's own readiness (`gate_window`, called by the extended() closure) is what must
        # move, not only window_complete() (review wave2-13 m4).
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(MIXED_RATE_LOG)
            ready, window = join_lag.gate_window(log, 3.0)
            self.assertFalse(ready)
            self.assertEqual(window["start_at"], "2026-11-24T18:19:38.656790+00:00")
            log.write_text(MIXED_RATE_LOG + EXTENDED_LINE.replace("18:19:50", "18:19:45"))
            ready, window = join_lag.gate_window(log, 3.0)
            self.assertFalse(ready)
            self.assertEqual(window["start_at"], "2026-11-24T18:19:38.656790+00:00")
            log.write_text(MIXED_RATE_LOG + EXTENDED_LINE)
            ready, window = join_lag.gate_window(log, 3.0)
            self.assertTrue(ready)
            self.assertGreaterEqual(window["seconds"], join_lag.HALT_WINDOW_SECONDS)
            # A later publication that is itself at the requested rate must move the start again
            # (the recompute takes the newest applied interval, not the first one it saw).
            shifted = MIXED_RATE_LOG + "2026-11-24T18:19:42.000000Z DEBUG mc_edge::server::pump: pump published tick tick=310 viewers=2 elapsed_us=3900\n"
            log.write_text(shifted)
            ready, window = join_lag.gate_window(log, 3.0)
            self.assertEqual(window["start_at"], "2026-11-24T18:19:38.656790+00:00")

    def test_an_admission_with_no_applied_rate_is_inconclusive(self) -> None:
        # m3 itself: an admission followed only by 20 TPS publications must give seconds None,
        # never fall back to the admission (review wave2-14 Major-1 asks for this red).
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(MIXED_RATE_LOG.replace(
                """
2026-11-24T18:19:35.992221Z DEBUG mc_edge::server::pump: pump published tick tick=292 viewers=1 elapsed_us=2019
2026-11-24T18:19:38.656790Z DEBUG mc_edge::server::pump: pump published tick tick=300 viewers=2 elapsed_us=2797
""",
                """
2026-11-24T18:19:35.960000Z DEBUG mc_edge::server::pump: pump published tick tick=292 viewers=1 elapsed_us=2019
2026-11-24T18:19:35.990000Z DEBUG mc_edge::server::pump: pump published tick tick=300 viewers=2 elapsed_us=2797
"""))
            window = join_lag.publish_window(log, join_lag.JOINER, 3.0)
            self.assertIsNotNone(window["admitted_at"])
            self.assertIsNone(window["start_at"])
            self.assertIsNone(window["seconds"])
            ready, _ = join_lag.gate_window(log, 3.0)
            self.assertFalse(ready)

    def test_a_rate_where_the_guard_is_blind_is_refused(self) -> None:
        for rate in ("16", "20"):
            with self.subTest(rate=rate):
                with self.assertRaisesRegex(ValueError, "tick-rate must be in"):
                    join_lag.validate_args(context("--tick-rate", rate))

    def test_a_publication_past_the_window_extends_it(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(MIXED_RATE_LOG + EXTENDED_LINE)
            window = join_lag.publish_window(log, join_lag.JOINER, 3.0)
            self.assertGreaterEqual(window["seconds"], join_lag.HALT_WINDOW_SECONDS)
            self.assertEqual(window["end_tick"], 337)

    def test_a_late_admission_defers_the_start_to_the_next_applied_interval(self) -> None:
        # An admission inside an interval leaves that interval's end as the first publication that
        # is known to be at the applied rate; the window starts there, never at the admission.
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(MIXED_RATE_LOG.replace("18:19:32.589589", "18:19:39.500000")
                           + EXTENDED_LINE)
            window = join_lag.publish_window(log, join_lag.JOINER, 3.0)
            self.assertEqual(window["admitted_at"], "2026-11-24T18:19:39.500000+00:00")
            self.assertEqual(window["start_at"], "2026-11-24T18:19:50.900000+00:00")
            self.assertAlmostEqual(window["seconds"], 0.0, places=1)
            self.assertFalse(join_lag.window_complete(window))


if __name__ == "__main__":
    unittest.main()
