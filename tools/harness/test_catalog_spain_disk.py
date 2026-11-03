import asyncio
import struct
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from tools.harness.backends.catalog_spain_disk import WorldDiskGuard


class SpainDiskGuardTests(unittest.IsolatedAsyncioTestCase):
    async def test_reserve_refuses_before_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(RuntimeError):
                WorldDiskGuard(Path(directory), 2**80, asyncio.current_task())

    async def test_inotify_overflow_cancels_work(self):
        cancelled = []
        with tempfile.TemporaryDirectory() as directory:
            guard = WorldDiskGuard(Path(directory), 0, SimpleNamespace(cancel=lambda: cancelled.append(True)))
            try:
                with patch("tools.harness.backends.catalog_spain_disk.os.read", return_value=struct.pack("iIII", -1, 0x4000, 0, 0)):
                    guard._ready()
                self.assertTrue(cancelled)
                self.assertIn("lost events", guard.reason)
            finally:
                guard.close()
