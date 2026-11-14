"""Ledger lines that ran on an older plugin build stop counting as matrix coverage."""
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from tools.harness.matrix_execution import plugin_set_fingerprint

REPO = Path(__file__).resolve().parents[2]
GEN = REPO / "tools" / "harness" / "matrix_specs" / "gen_matrix.py"
# gen_matrix requires every plugin a plugin-sensitive subject depends on.
PLUGINS = ("graves", "solaris-essentials", "solaris-auth")
# Frozen wave-1 ids (MATRIX_FREEZE.json); both use the full live set.
OUTDATED_ID = "bl:commands.msg/vanilla_java_26_1_2/survival/overworld/normal/2/normal/steady/non_op_0@live"
CURRENT_ID = "bl:death_respawn.keep_inventory_false/vanilla_java_26_1_2/survival/overworld/normal/1/normal/steady/non_op_0@live"


def line(sid, plugin_set):
    return {"id": sid, "status": "fail", "driver": "java_mcp", "server_sha": "0" * 64, "run_dir": "/tmp/run",
            "ts": "2026-11-11T00:00:00Z", "note": "fixture", "plugin_set": plugin_set}


class OutdatedPluginSetTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.data = Path(tmp.name)
        live = self.data / "live-plugins"
        for plugin in PLUGINS:
            (live / plugin).mkdir(parents=True)
            (live / plugin / "plugin.toml").write_text(f'id = "{plugin}"\nentry = "plugin.wasm"\n')
            (live / plugin / "plugin.wasm").write_bytes(plugin.encode())
        current = plugin_set_fingerprint(live)
        self.ledger = self.data / "ledger.jsonl"
        self.ledger.write_text(json.dumps(line(OUTDATED_ID, "f" * 64)) + "\n" + json.dumps(line(CURRENT_ID, current)) + "\n")

    def gen(self, *args):
        env = dict(os.environ, SCENARIO_LEDGER=str(self.ledger), SOLARIS_MATRIX_DATA=str(self.data))
        return subprocess.run([sys.executable, str(GEN), *args], cwd=REPO, env=env,
                              capture_output=True, text=True, timeout=300, check=True).stdout

    def test_status_counts_only_the_current_plugin_set(self):
        summary = next(row for row in self.gen("--status").splitlines() if row.startswith("ledger:"))
        self.assertIn("(1 ids, 0 invalid lines", summary)
        self.assertIn("1 outdated plugin-set lines", summary)

    def test_pending_reruns_the_outdated_cell_only(self):
        pending = {row.split("\t")[1] for row in self.gen("--full", "--wave", "1", "--pending").splitlines() if "\t" in row}
        self.assertIn(OUTDATED_ID, pending)
        self.assertNotIn(CURRENT_ID, pending)


if __name__ == "__main__":
    unittest.main()
