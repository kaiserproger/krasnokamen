"""Bounded received-entity census used by the real Spain trial."""
import unittest
from unittest.mock import Mock

from tools.showcase.spain_trial import resident_census


class ResidentCensusTests(unittest.TestCase):
    def test_pages_filtered_villagers_without_large_results(self):
        probe = Mock()
        rows = [{"entity_id": n} for n in range(9)]
        probe.call.side_effect = [
            {"entities": rows[:8], "visible_count": 9, "truncated": True, "next_cursor": "7"},
            {"entities": rows[8:], "visible_count": 9, "truncated": False, "next_cursor": None},
        ]
        self.assertEqual(resident_census(probe), {
            "resident_census": rows, "resident_visible_count": 9,
        })
        for call in probe.call.call_args_list:
            name, args = call.args
            self.assertEqual(name, "minecraft_list_entities")
            self.assertEqual(args["limit"], 8)
            self.assertEqual(args["entity_type"], "minecraft:villager")
        self.assertEqual(probe.call.call_args_list[0].args[1]["cursor"], "")
        self.assertEqual(probe.call.call_args_list[1].args[1]["cursor"], "7")

    def test_empty_received_census_is_not_invented_population(self):
        probe = Mock()
        probe.call.return_value = {
            "entities": [], "visible_count": 0, "truncated": False, "next_cursor": None,
        }
        self.assertEqual(resident_census(probe)["resident_census"], [])

    def test_truncated_without_cursor_refuses_incomplete_receipt(self):
        probe = Mock()
        probe.call.return_value = {
            "entities": [], "visible_count": 9, "truncated": True, "next_cursor": None,
        }
        with self.assertRaisesRegex(RuntimeError, "truncated without"):
            resident_census(probe)

    def test_repeated_cursor_cannot_loop(self):
        probe = Mock()
        probe.call.return_value = {
            "entities": [], "visible_count": 9, "truncated": True, "next_cursor": "7",
        }
        with self.assertRaisesRegex(RuntimeError, "did not advance"):
            resident_census(probe)
        self.assertEqual(probe.call.call_count, 2)


if __name__ == "__main__":
    unittest.main()
