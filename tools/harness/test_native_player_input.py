"""The held-drop verdict reads the client's serverbound counters and is fail-closed."""
import unittest

from .backends.native_player_input import one_drop, sent_drop_actions


def observation(drop_item: int = 0, drop_stack: int = 0) -> dict:
    return {"packets": {"counts": {"1": 4},
                        "sent_actions": {"player_action": {"5": drop_item, "4": drop_stack},
                                         "drop_item": drop_item, "drop_stack": drop_stack}}}


class SentDropActionsTests(unittest.TestCase):
    def test_reads_the_counters(self):
        self.assertEqual(sent_drop_actions(observation(2, 1)),
                         {"drop_item": 2, "drop_stack": 1})

    def test_a_build_without_the_counters_is_unreadable_not_a_pass(self):
        # CLIENT-HELD-DROP-KEY-DOUBLE-DROP: an older client can only report clientbound packet
        # counts, so the verdict must refuse to guess.
        for state in ({}, {"packets": {"counts": {"1": 4}}}, {"packets": None}):
            with self.assertRaisesRegex(RuntimeError, "packets.sent_actions"):
                sent_drop_actions(state)

    def test_non_integer_counters_are_refused(self):
        state = observation()
        state["packets"]["sent_actions"]["drop_item"] = "1"
        with self.assertRaisesRegex(RuntimeError, "drop_item/drop_stack"):
            sent_drop_actions(state)

    def test_one_press_is_one_item_and_a_repeat_is_not(self):
        self.assertTrue(one_drop({"drop_item": 0, "drop_stack": 0},
                                 {"drop_item": 1, "drop_stack": 0}))
        # The reversed-fix client sends the press-edge drop and the held-key tick-edge drop.
        self.assertFalse(one_drop({"drop_item": 0, "drop_stack": 0},
                                  {"drop_item": 2, "drop_stack": 0}))
        self.assertFalse(one_drop({"drop_item": 3, "drop_stack": 0},
                                  {"drop_item": 3, "drop_stack": 0}))
        self.assertFalse(one_drop({"drop_item": 0, "drop_stack": 0},
                                  {"drop_item": 1, "drop_stack": 1}))


if __name__ == "__main__":
    unittest.main()
