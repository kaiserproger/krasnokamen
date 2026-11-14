"""Autocraft QA readback parsing (the server's exact `job`/`carry`/`building list` text)."""
from __future__ import annotations

import json
import unittest

from .backends import autocraft_qa


class AutocraftReadbackTests(unittest.TestCase):
    def test_job_readback_reports_gate_and_receipts(self) -> None:
        receipt = {"job": "ui-1-1", "sequence": 1, "recipe": "krasnokamen-settlements:open_hay",
                   "recipe_revision": "1", "inputs": [], "outputs": [], "tool_wear": []}
        line = json.dumps({"after": None, "cancelled": False, "count": 3, "done": 1,
                           "id": "ui-1-1", "next_due": 90, "order": "ui-1", "paused": None,
                           "receipts": [receipt], "recipe": "krasnokamen-settlements:open_hay",
                           "recipe_revision": "1", "worker": "w"}, separators=(",", ":"))
        state = autocraft_qa.job_state(line)
        self.assertEqual((state["done"], state["receipts"], state["order"], state["after"]),
                         (1, 1, "ui-1", None))
        self.assertEqual(state["worker"], "w")
        # The readback needle for the product never matches its dependency.
        self.assertNotIn('"id":"ui-1"', line)

    def test_carry_readback_sums_slots_and_skips_empty(self) -> None:
        bread = {"item": "minecraft:bread", "stack": {"count": 5, "item_id": 1}}
        more = {"item": "minecraft:bread", "stack": {"count": 3, "item_id": 1}}
        wheat = {"item": "minecraft:wheat", "stack": {"count": 3, "item_id": 2}}
        line = json.dumps({"fence": {}, "slots": [bread, None, more, wheat, None, None, None, None],
                           "tick": 9, "worker": "w"})
        self.assertEqual(autocraft_qa.carry_counts(line),
                         {"minecraft:bread": 8, "minecraft:wheat": 3})

    def test_building_list_yields_full_ids(self) -> None:
        line = ("Buildings: #4294967297 storage level 1 [1, 2, 3]-[5, 6, 7], "
                "#4294967298 cartographer level 1 [0, 2, 3]-[4, 6, 7].")
        self.assertEqual(autocraft_qa.building_id([line], "cartographer"), 4294967298)
        self.assertEqual(autocraft_qa.building_id([line], "storage"), 4294967297)
        self.assertIsNone(autocraft_qa.building_id([line], "mason"))

    def test_readback_answer_is_the_oldest_match_newer_than_the_marker(self) -> None:
        # Exact newest-first order seen live on the first command after a
        # reconnect: the answer landed after the marker echo, and an older
        # stale answer sits below the marker.
        lines = ['{"id":"ui-1","done":1}',
                 '{"id":"ui-1","done":2}',
                 "<AutoCraftQA> autocraft-qa f8602a55",
                 "Settlements / Поселения: /settlement guide",
                 '{"id":"ui-1","done":0}']
        self.assertEqual(autocraft_qa.answer_after_marker(lines, "f8602a55", '"id":"ui-1"'), lines[1])
        self.assertIsNone(autocraft_qa.answer_after_marker(lines[2:], "f8602a55", '"id":"ui-1"'))
        self.assertIsNone(autocraft_qa.answer_after_marker(lines[:2], "f8602a55", '"id":"ui-1"'))

    def test_flat_undoes_dialog_wrapping(self) -> None:
        self.assertEqual(autocraft_qa._flat("Refused: missing 24\nminecraft:leather:  not"),
                         "Refused: missing 24 minecraft:leather: not")

    def test_admitted_worker_matches_its_form_not_an_earlier_refused_form(self) -> None:
        saddle = {"ui_nodes": [{"id": "dialog/input/worker", "label": "Resident: Ольга"}]}
        bread = {"ui_nodes": [{"id": "dialog/input/worker", "label": "Resident: Павел"}]}
        # Street work finished between forms; neither refusal reserved a worker.
        tags = {"former-builder": "Павел · безработный · работа",
                "cartographer": "Ольга · картограф · работа"}
        self.assertTrue(autocraft_qa.worker_matches_form(bread, "former-builder", tags))
        self.assertFalse(autocraft_qa.worker_matches_form(saddle, "former-builder", tags))
        self.assertFalse(autocraft_qa.worker_matches_form(bread, "cartographer", tags))
        self.assertFalse(autocraft_qa.worker_matches_form(bread, "not-a-resident", tags))

    def test_same_named_residents_do_not_make_the_admitted_uuid_ambiguous(self) -> None:
        form = {"ui_nodes": [{"id": "dialog/input/worker", "label": "Resident: Павел"}]}
        tags = {"one": "Павел · безработный · работа", "two": "Павел · картограф · работа"}
        self.assertTrue(autocraft_qa.worker_matches_form(form, "two", tags))


if __name__ == "__main__":
    unittest.main()
