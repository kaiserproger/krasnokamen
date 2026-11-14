"""Split contract for the village/civilization gates.

civilization-seed-qa's stages are milestones of one village observed over time,
so the only independent axis is the observation world: `--parts N` needs N named
seeds with verified centers. The other village gates share one world and one
client session, so they refuse a split instead of silently ignoring it.
"""
import os
import unittest
from types import SimpleNamespace
from unittest import mock

from tools.harness import profiles
from tools.harness.backends import civilization_seed_qa, settlement_map_qa, village_qa


class CivilizationSeedSplitTests(unittest.TestCase):
    def test_seeds_parse_with_and_without_a_center(self):
        self.assertEqual(civilization_seed_qa.split_seeds("24"),
                         [(24, "")])
        self.assertEqual(civilization_seed_qa.split_seeds("24:136,70,-216; 87122:-461,70,350"),
                         [(24, "136,70,-216"), (87122, "-461,70,350")])

    def test_a_split_without_named_seeds_is_refused(self):
        for value in (None, "", " ; ", "abc", "24:1,2", "24:1,2,3,4", "24;24"):
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    civilization_seed_qa.split_seeds(value)

    def test_part_labels_name_their_observation_world(self):
        self.assertEqual(civilization_seed_qa.split_label(24, "136,70,-216"), "seed24-136_70_-216")
        self.assertEqual(civilization_seed_qa.split_label(87122, ""), "seed87122")

    def verdicts(self, **stages):
        base = {name: True for name in civilization_seed_qa.STAGES}
        base.update(stages)
        return {**base, "all": all(base[name] for name in civilization_seed_qa.STAGES)}

    def block(self, label, verdicts, status="passed"):
        return {"label": label, "status": status, "details": {"verdicts": verdicts}}

    def test_every_stage_of_every_seed_must_hold(self):
        seeds = [(24, ""), (87122, "")]
        blocks = [self.block("seed24", self.verdicts()),
                  self.block("seed87122", self.verdicts(second_building=False))]
        merged = civilization_seed_qa.merge_seed_parts(blocks, seeds)
        self.assertEqual(merged["mode"], "civilization-seed-split")
        self.assertTrue(merged["stages_all_parts"]["new_house"])
        self.assertFalse(merged["stages_all_parts"]["second_building"])
        self.assertFalse(merged["all"])
        self.assertEqual([part["seed"] for part in merged["parts"]], [24, 87122])

    def test_a_part_that_published_no_verdicts_fails_the_split(self):
        seeds = [(24, ""), (87122, "")]
        blocks = [self.block("seed24", self.verdicts()), self.block("seed87122", {})]
        merged = civilization_seed_qa.merge_seed_parts(blocks, seeds)
        self.assertFalse(merged["verdicts_complete"])
        self.assertFalse(merged["all"])

    def test_a_missing_part_report_is_a_split_failure(self):
        with self.assertRaisesRegex(RuntimeError, "1 part reports for 2 seeds"):
            civilization_seed_qa.merge_seed_parts([self.block("seed24", self.verdicts())],
                                                  [(24, ""), (87122, "")])

    def test_parts_without_named_seeds_fail_before_any_server_starts(self):
        ctx = SimpleNamespace(parts=2, client="native", extra_args=[], platform=None,
                             windowed=True, timeout_seconds=600.0, artifact_dir=None,
                             client_evidence={})
        with mock.patch.dict(os.environ, {civilization_seed_qa.SPLIT_SEEDS_ENV: ""}):
            with self.assertRaisesRegex(ValueError, civilization_seed_qa.SPLIT_SEEDS_ENV):
                from tools.harness.backends import village_autonomy_qa
                village_autonomy_qa.run(ctx, civilization=True)

    def test_the_parent_runs_one_child_per_named_seed(self):
        ctx = SimpleNamespace(parts=2, client="native", extra_args=[], platform=None,
                             windowed=True, timeout_seconds=600.0, artifact_dir=None,
                             client_evidence={})
        blocks = [self.block("seed24-136_70_-216", self.verdicts()),
                  self.block("seed87122", self.verdicts())]
        with mock.patch.dict(os.environ, {
                civilization_seed_qa.SPLIT_SEEDS_ENV: "24:136,70,-216;87122"}), \
                mock.patch.dict(os.environ, {profiles.SPLIT_PART_ENV: ""}), \
                mock.patch("tools.harness.backends.village_autonomy_qa.run_split",
                           return_value=blocks) as split:
            from tools.harness.backends import village_autonomy_qa
            report = village_autonomy_qa.run(ctx, civilization=True)
        parts = split.call_args.args[2]
        self.assertEqual(split.call_args.args[1], "civilization-seed-qa")
        self.assertEqual([(part.label, part.env["VILLAGE_AUTONOMY_SEED"],
                           part.env["VILLAGE_AUTONOMY_CENTER"]) for part in parts],
                         [("seed24-136_70_-216", "24", "136,70,-216"),
                          ("seed87122", "87122", "")])
        self.assertTrue(report["scenario"]["all"])

    def test_a_part_count_that_does_not_match_the_named_seeds_is_refused(self):
        ctx = SimpleNamespace(parts=3, client="native", extra_args=[], platform=None,
                             windowed=True, timeout_seconds=600.0, artifact_dir=None,
                             client_evidence={})
        with mock.patch.dict(os.environ, {civilization_seed_qa.SPLIT_SEEDS_ENV: "24;87122"}):
            from tools.harness.backends import village_autonomy_qa
            with self.assertRaisesRegex(ValueError, "needs exactly 3 entries"):
                village_autonomy_qa.run(ctx, civilization=True)


class NoIndependentPartsTests(unittest.TestCase):
    def ctx(self, profile):
        return SimpleNamespace(parts=2, client="native", extra_args=[], platform=None,
                               windowed=True, timeout_seconds=600.0, artifact_dir=None,
                               client_evidence={})

    def test_village_qa_refuses_a_split_instead_of_ignoring_it(self):
        with self.assertRaisesRegex(ValueError, "village-qa has no independent parts"):
            village_qa.validate_args(self.ctx("village-qa"))

    def test_settlement_map_qa_refuses_a_split_instead_of_ignoring_it(self):
        with self.assertRaisesRegex(ValueError, "settlement-map-qa has no independent parts"):
            settlement_map_qa.run(self.ctx("settlement-map-qa"))

    def test_a_single_part_still_passes_the_old_guard(self):
        self.assertEqual(village_qa.validate_args(
            SimpleNamespace(parts=1, client="native", extra_args=[], platform=None,
                            timeout_seconds=600.0)), 600.0)


if __name__ == "__main__":
    unittest.main()
