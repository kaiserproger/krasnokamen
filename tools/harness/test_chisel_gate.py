"""Offline unit tests for the chisel gate helpers (no server/client launched)."""
from __future__ import annotations

import unittest

from .backends import chisel


class ChiselHelperTests(unittest.TestCase):
    def test_pad_fills_fit_the_server_limit(self) -> None:
        limit = 32768  # server max_block_modifications: larger fills reject whole
        for command in chisel._pad_commands(100, 64, -100):
            parts = command.split()
            x0, y0, z0, x1, y1, z1 = (int(v) for v in parts[1:7])
            volume = (x1 - x0 + 1) * (y1 - y0 + 1) * (z1 - z0 + 1)
            self.assertLessEqual(volume, limit, command)


if __name__ == "__main__":
    unittest.main()
