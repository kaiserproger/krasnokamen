"""ProbeSteps.wait: an idle client's state-event expiry is not a scenario failure."""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from .backends.probe_steps import ProbeSteps

EXPIRED = 'minecraft_wait_for_state_change failed: {"code": "client state event did not arrive", "retryable": false, "stage": "automation"}'


class IdleClient:
    """Reports the same state until `flip_after` observations, then the awaited one.

    Every state-change wait expires, as it does for a dead player on a death screen.
    """

    def __init__(self, flip_after: int | None, wait_error: str = EXPIRED) -> None:
        self.flip_after = flip_after
        self.wait_error = wait_error
        self.observations = 0
        self.waits = 0

    def call_tool(self, name: str, arguments: dict) -> dict:
        if name == "minecraft_observe":
            self.observations += 1
            done = self.flip_after is not None and self.observations > self.flip_after
            return {"state_version": 7, "in_play": True, "ready": done}
        assert name == "minecraft_wait_for_state_change"
        self.waits += 1
        raise RuntimeError(self.wait_error)


class ProbeStepsWaitTests(unittest.TestCase):
    def steps(self, client: IdleClient, timeout: float = 5.0) -> ProbeSteps:
        return ProbeSteps(client, Path(tempfile.mkdtemp()), timeout, "idle-client")

    def test_expired_state_wait_resamples_and_sees_the_later_change(self) -> None:
        client = IdleClient(flip_after=2)
        state = self.steps(client).wait(lambda s: s["ready"], "ready")
        self.assertTrue(state["ready"])
        self.assertEqual(client.waits, 2)

    def test_never_true_predicate_times_out_naming_the_label_not_the_transport(self) -> None:
        client = IdleClient(flip_after=None)
        steps = self.steps(client)
        with self.assertRaisesRegex(TimeoutError, "timed out waiting for never_ready"):
            steps.wait(lambda s: s["ready"], "never_ready", timeout_seconds=0.3)
        self.assertFalse(steps.report["assertions"]["never_ready"])

    def test_scenario_deadline_ends_an_idle_wait_as_a_timeout(self) -> None:
        client = IdleClient(flip_after=None)
        with self.assertRaisesRegex(TimeoutError, "idle-client deadline expired"):
            self.steps(client, timeout=0.3).wait(lambda s: s["ready"], "ready")

    def test_other_state_wait_errors_stay_fatal(self) -> None:
        client = IdleClient(flip_after=None, wait_error="minecraft_wait_for_state_change failed: connection lost")
        with self.assertRaisesRegex(RuntimeError, "connection lost"):
            self.steps(client).wait(lambda s: s["ready"], "ready")


if __name__ == "__main__":
    unittest.main()
