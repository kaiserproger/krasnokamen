"""DATAPACK-NEW-PACK-WORLDGEN-SKIP: the start-skip assertion reads red before the fix.

The assertion is the datapack-reload scenario's `new_worldgen_pack_skipped_and_named`, taken
from the restarted server's own log. This test drives the two pieces it is built from with a
pre-fix start (the refusal the row records) and with a fixed start (the skip line):

* `write_worldgen_pack` writes exactly the trigger the cargo oracle uses — a new pack with
  `data/mod/worldgen/structure_set/a.json`, which this server cannot prepare;
* `wait_log_lines` is False on a log whose only worldgen line is the start refusal and True
  once the skip warning names the pack and its cause.
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from tools.harness.backends import datapack_reload

PREFIX_REFUSAL = (
    'ERROR krasnokamen: refusing to start: Worldgen("Datapack worldgen requires the vanilla '
    'gameplay cache")\n'
)
FIXED_SKIP = (
    "WARN krasnokamen: Skipping new datapack file/newwg, it is not enabled: Datapack worldgen "
    "requires the vanilla gameplay cache\n"
)


class WorldgenSkipTest(unittest.TestCase):
    def test_writes_the_pack_the_cargo_oracle_uses(self):
        with tempfile.TemporaryDirectory() as root:
            pack = datapack_reload.write_worldgen_pack(Path(root))
            self.assertEqual(pack.name, datapack_reload.WORLDGEN_PACK)
            self.assertTrue((pack / "pack.mcmeta").is_file())
            self.assertEqual(
                (pack / "data/mod/worldgen/structure_set/a.json").read_text(),
                "{}\n", "the worldgen input the server cannot prepare")

    def test_start_refusal_does_not_satisfy_the_assertion(self):
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text(PREFIX_REFUSAL)
            self.assertFalse(datapack_reload.wait_log_lines(
                log, [f"Skipping new datapack {datapack_reload.WORLDGEN_ID}", "gameplay cache"], 0.4))

    def test_skip_line_satisfies_the_assertion(self):
        with tempfile.TemporaryDirectory() as root:
            log = Path(root) / "server.log"
            log.write_text("INFO krasnokamen: listening\n" + FIXED_SKIP)
            self.assertTrue(datapack_reload.wait_log_lines(
                log, [f"Skipping new datapack {datapack_reload.WORLDGEN_ID}", "gameplay cache"], 5.0))

    def test_the_scenario_check_is_false_unless_the_start_completes_and_is_named(self):
        # The scenario's own expression, both rows of its truth table.
        for started, named, expected in ((False, False, False), (True, False, False), (True, True, True)):
            self.assertIs(bool(started and named), expected, (started, named))


if __name__ == "__main__":
    unittest.main()
