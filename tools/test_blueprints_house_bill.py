import ast
from pathlib import Path
import subprocess
import sys
import unittest


class HouseBillTest(unittest.TestCase):
    def test_house_l1_base_bill_covers_whole_door_batch(self):
        root = Path(__file__).resolve().parents[1]
        output = subprocess.check_output(
            [sys.executable, "tools/blueprints.py", "materials", "house"],
            cwd=root,
            text=True,
        )
        line = next(line for line in output.splitlines() if "L1:" in line and "base" in line)
        bill = ast.literal_eval(line.split("base", 1)[1].split(";", 1)[0].strip())
        self.assertEqual(bill, {"coal": 1, "cobblestone": 51, "oak_log": 89, "white_wool": 3})


if __name__ == "__main__":
    unittest.main()
