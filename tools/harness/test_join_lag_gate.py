"""Unit tests for the join-lag gate: its options, verdicts and row-facing registration."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

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


class HaltVerdictTest(unittest.TestCase):
    def test_a_ticking_core_behind_a_witnessed_delay_passes(self) -> None:
        ok, _ = join_lag.halt_verdict(True, record(), 3.0, 4, 100, 160, None)
        self.assertTrue(ok)

    def test_a_halted_core_under_a_witnessed_delay_fails(self) -> None:
        ok, _ = join_lag.halt_verdict(True, record(), 3.0, 0, 100, 160, "ERROR edge pump not draining")
        self.assertFalse(ok)

    def test_a_stuck_simulation_tick_fails(self) -> None:
        ok, why = join_lag.halt_verdict(True, record(), 3.0, 4, 100, 100, None)
        self.assertFalse(ok)
        self.assertIn("did not advance", why)

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


class KnobLagTest(unittest.TestCase):
    """The knob's own debug event is the join trigger; no polling."""

    def test_the_last_knob_line_carries_the_backlog(self) -> None:
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False) as handle:
            handle.write("x pump knob wait completed lag_ticks=3\n")
            handle.write("x pump knob wait completed lag_ticks=9\n")
            path = handle.name
        self.assertEqual(join_lag.knob_lag(join_lag.Path(path)), 9)
        self.assertIsNone(join_lag.knob_lag(join_lag.Path(path + ".missing")))


class JoinDueTest(unittest.TestCase):
    """Row 1's control timing: hold the join until the witnessed lag passes the threshold."""

    def test_the_default_timing_asks_no_question(self) -> None:
        self.assertFalse(join_lag.join_due(None, 0))
        self.assertFalse(join_lag.join_due(4, 0))

    def test_the_threshold_holds_the_join_until_the_pump_is_that_far_behind(self) -> None:
        self.assertFalse(join_lag.join_due(None, 6))
        self.assertFalse(join_lag.join_due(5, 6))
        self.assertTrue(join_lag.join_due(6, 6))
        self.assertTrue(join_lag.join_due(26, 6))


class AppliedRateTest(unittest.TestCase):
    def test_the_tick_query_reply_names_the_rate(self) -> None:
        self.assertEqual(join_lag.applied_tick_rate(["Target tick rate: 3.0", "hello"]), 3.0)
        self.assertEqual(join_lag.applied_tick_rate(["Some chat", "Target tick rate: 14.65"]), 14.65)

    def test_a_quiet_chat_has_no_rate(self) -> None:
        self.assertIsNone(join_lag.applied_tick_rate([]))
        self.assertIsNone(join_lag.applied_tick_rate(["Set the target tick rate to 3.0 per second"]))


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


if __name__ == "__main__":
    unittest.main()
