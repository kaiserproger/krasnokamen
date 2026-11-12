import unittest

from tools.harness.backends import buildings_qa


class OrientationTest(unittest.TestCase):
    def observed(self, bp, level, origin, turns):
        return {pos: buildings_qa._state_base(want)
                for pos, want in buildings_qa.oriented_cells(bp, level, origin, turns).items()}

    def test_best_turns_reads_the_orientation_the_world_was_built_in(self):
        origin = (118, 69, -11)
        for bp, level in (("house", 1), ("wall", 1), ("wall", 2)):
            for turns in range(4):
                with self.subTest(bp=bp, level=level, turns=turns):
                    world = self.observed(bp, level, origin, turns)
                    cells = {t: buildings_qa.oriented_cells(bp, level, origin, t) for t in range(4)}
                    best = buildings_qa.best_turns(cells, world)
                    # A rotation-symmetric blueprint level reads as its lowest equivalent turn count.
                    self.assertEqual(buildings_qa.oriented_cells(bp, level, origin, best),
                                     buildings_qa.oriented_cells(bp, level, origin, turns))

    def test_a_house_with_the_door_on_the_east_side_is_not_rot_zero(self):
        # buildings-qa 2026-11-12: 13 of 90 house cells "mismatched" because the build was rotated.
        origin = (118, 69, -11)
        world = self.observed("house", 1, origin, 3)
        cells = {t: buildings_qa.oriented_cells("house", 1, origin, t) for t in range(4)}
        self.assertEqual(buildings_qa.best_turns(cells, world), 3)

    def test_rotation_keeps_the_bounds_minimum_at_the_origin(self):
        origin = (10, 5, 20)
        for turns in range(4):
            cells = buildings_qa.oriented_cells("wall", 2, origin, turns)
            self.assertEqual(min(x for x, _, _ in cells), 10)
            self.assertEqual(min(z for _, _, z in cells), 20)


if __name__ == "__main__":
    unittest.main()
