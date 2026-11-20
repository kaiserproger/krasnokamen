import asyncio
import tempfile
import unittest
from pathlib import Path

from tools.harness.backends.catalog_spain_disk import WorldDiskGuard


class SpainDiskGuardTests(unittest.IsolatedAsyncioTestCase):
    async def test_reserve_refuses_before_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(RuntimeError):
                WorldDiskGuard(Path(directory), 2**80, asyncio.current_task())
