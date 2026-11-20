"""Pure verdicts of the stage-2 settlement walk: they must be red on the wrong text, not only green on the right one."""
from __future__ import annotations

import unittest

from tools.harness.backends import settlement_quickwins_stage2 as s2

PASS = "\n".join([
    "✔ площадь пола 25/9", "✔ высота 5/4", "✔ кровати 1/1", "✔ крыша 1/1", "✔ стены по периметру 1/1", "✔ дверь 1/1"])
BROKEN = "\n".join([
    "✔ площадь пола 25/9", "✔ высота 5/4", "✘ кровати 0/1 — добавьте недостающие кровати целиком",
    "✘ крыша 0/1 — закройте отверстия в крыше", "✔ стены по периметру 1/1", "✔ дверь 1/1"])
EXPECT = {"кроват": 1, "крыш": 1, "двер": 1}


class ChecklistTest(unittest.TestCase):
    def test_parse_marks_numbers_and_hint(self) -> None:
        rows = s2.checklist(BROKEN)
        self.assertEqual([row["ok"] for row in rows], [True, True, False, False, True, True])
        self.assertEqual((rows[2]["found"], rows[2]["required"]), (0, 1))
        self.assertEqual(rows[3]["hint"], "закройте отверстия в крыше")
        self.assertIsNone(rows[0]["hint"])

    def test_pass_needs_every_tick_the_counters_and_the_certify_button(self) -> None:
        self.assertTrue(s2.pass_verdict(PASS, ["Сертифицировать", "Проверить здесь", "Назад"], EXPECT)[0])
        self.assertFalse(s2.pass_verdict(BROKEN, ["Сертифицировать"], EXPECT)[0])  # a cross
        self.assertFalse(s2.pass_verdict(PASS, ["Проверить здесь", "Назад"], EXPECT)[0])  # no Certify
        self.assertFalse(s2.pass_verdict(PASS, ["Сертифицировать"], {**EXPECT, "кроват": 2})[0])  # wrong required number
        self.assertFalse(s2.pass_verdict("", ["Сертифицировать"], EXPECT)[0])  # no checklist at all
        self.assertFalse(s2.pass_verdict(PASS + "\n✔ roof 1/1", ["Сертифицировать"], EXPECT)[0])  # jargon

    def test_fail_needs_numbers_hint_and_no_certify(self) -> None:
        failing = [("кроват", 0, 1), ("крыш", 0, 1)]
        self.assertTrue(s2.fail_verdict(BROKEN, ["Проверить здесь", "Назад"], failing)[0])
        self.assertFalse(s2.fail_verdict(BROKEN, ["Сертифицировать", "Назад"], failing)[0])  # Certify offered
        self.assertFalse(s2.fail_verdict(BROKEN.replace("0/1 — добавьте недостающие кровати целиком", "0/1"), [], failing)[0])  # no hint
        self.assertFalse(s2.fail_verdict(BROKEN, [], [("кроват", 0, 2)])[0])  # wrong number
        self.assertFalse(s2.fail_verdict(PASS, [], failing)[0])  # nothing failed

    def test_refusal_is_a_russian_sentence_with_an_action_and_no_rows(self) -> None:
        text = "Замкнутая комната не найдена. Закройте проёмы в стенах и встаньте внутри одной комнаты."
        self.assertTrue(s2.refusal_verdict(text, ["Проверить здесь", "Назад"])[0])
        self.assertFalse(s2.refusal_verdict(text, ["Сертифицировать"])[0])
        self.assertFalse(s2.refusal_verdict(PASS, [])[0])
        self.assertFalse(s2.refusal_verdict("No enclosed room found. Close wall openings.", [])[0])

    def test_corner_and_height_controls_are_found_by_label(self) -> None:
        nodes = [{"id": "dialog/input/kind", "label": "Тип постройки: Жильё: Дом"},
                 {"id": "dialog/input/from", "label": "Угол пола 1 (x y z)"},
                 {"id": "dialog/input/extra", "label": "Высота над углами: 4"},
                 {"id": "dialog/input/from/label", "label": "Угол пола 1 (x y z)"},
                 {"id": "dialog/action/0", "label": "Проверить здесь"}]
        self.assertEqual(s2.corner_or_height_controls(nodes), ["Угол пола 1 (x y z)", "Высота над углами: 4"])
        self.assertEqual(s2.corner_or_height_controls([nodes[0], nodes[4]]), [])

    def test_building_list_row_and_containment(self) -> None:
        match = s2.LIST_ROW.search("#3 Workshop(Cartographer) level 1 [10, 200, 20]-[14, 204, 24]")
        self.assertIsNotNone(match)
        row = {"min": [10, 200, 20], "max": [14, 204, 24]}
        self.assertTrue(s2.contains(row, (13, 201, 23)))
        self.assertFalse(s2.contains(row, (15, 201, 23)))

    def test_criteria_of_the_suite(self) -> None:
        criteria, blocks = s2.suite("stage2")
        owed = {cid for _, ids, _ in blocks for cid in ids}
        self.assertEqual(owed, set(criteria))
        self.assertFalse(set(s2.OBSOLETE) & set(criteria))
        self.assertEqual(s2.suite("stage2op")[0], s2.OP)


if __name__ == "__main__":
    unittest.main()
