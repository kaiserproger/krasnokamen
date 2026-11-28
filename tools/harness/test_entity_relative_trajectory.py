"""ENTITY-RELATIVE-MOVEMENT-DRIFT: the trajectory check's red and green endpoints.

The scene drives 192 closed +/-0.01 cycles; the edge's own cargo oracle
(`relative_movement_cycles_return_the_decoded_consumer_to_origin`) fixes what the client
must read at the end of one: the origin exactly. Flooring the raw displacement instead
loses one 1/4096 block per closed cycle, which is the row's 0.25 blocks after 1024 cycles
and 0.047 blocks after the scene's 192.
"""

from __future__ import annotations

import unittest

# `scene_shot_checks` reaches the scenario seams through `scene_shot`; importing that first
# keeps the module's own import order (a direct import is a circular one).
from tools.harness.backends import scene_shot  # noqa: F401
from tools.harness.backends import scene_shot_checks as checks

ORIGIN = (1000.5, 200.0, 1000.5)


def drifted(cycles: int) -> tuple[float, float, float]:
    """The client endpoint a flooring encoder leaves after `cycles` closed cycles."""
    return (ORIGIN[0], ORIGIN[1] - cycles / 4096.0, ORIGIN[2])


class RelativeTrajectoryTest(unittest.TestCase):
    def test_the_rows_1024_cycles_lose_a_quarter_block(self):
        self.assertAlmostEqual(checks.relative_trajectory_drift(ORIGIN, drifted(1024)), 0.25, places=9)

    def test_the_scene_reds_the_pre_fix_encoder(self):
        drift = checks.relative_trajectory_drift(ORIGIN, drifted(checks.RELATIVE_TRAJECTORY_CYCLES))
        self.assertGreater(drift, checks.RELATIVE_TRAJECTORY_TOLERANCE,
                           "192 lost steps must exceed the 0.02 read tolerance")

    def test_the_scene_greens_the_fixed_encoder(self):
        self.assertLessEqual(checks.relative_trajectory_drift(ORIGIN, ORIGIN),
                             checks.RELATIVE_TRAJECTORY_TOLERANCE)

    def test_a_plain_observation_noise_stays_inside_the_tolerance(self):
        # The client's entity list reports f32 positions; a fixed encoder differs only by
        # float rounding of the same endpoint.
        endpoint = tuple(value + 0.0005 for value in ORIGIN)
        self.assertLessEqual(checks.relative_trajectory_drift(ORIGIN, endpoint),
                             checks.RELATIVE_TRAJECTORY_TOLERANCE)


if __name__ == "__main__":
    unittest.main()
