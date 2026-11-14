"""heavy-watchdog room, freeze and jobserver rules on a fake slice (never the live one)."""
import importlib.util
import os
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
GIB = 1024 ** 3
KIB_PER_GIB = 1024 * 1024


def load(state: Path, slice_dir: Path):
    argv = sys.argv
    sys.argv = ["heavy-watchdog.py", str(state), str(slice_dir)]
    try:
        spec = importlib.util.spec_from_file_location("heavy_watchdog_under_test", HERE / "heavy-watchdog.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
    finally:
        sys.argv = argv
    return module


def write_slice(slice_dir: Path, *, cap_gib, current_gib, file_gib, shmem_gib=0.0, dirty_gib=0.0, writeback_gib=0.0):
    slice_dir.mkdir(parents=True, exist_ok=True)
    (slice_dir / "memory.max").write_text(f"{int(cap_gib * GIB)}\n")
    (slice_dir / "memory.current").write_text(f"{int(current_gib * GIB)}\n")
    (slice_dir / "memory.stat").write_text(
        f"anon {int((current_gib - file_gib) * GIB)}\nfile {int(file_gib * GIB)}\nshmem {int(shmem_gib * GIB)}\n"
        f"file_dirty {int(dirty_gib * GIB)}\nfile_writeback {int(writeback_gib * GIB)}\n")


def row(pid, started, frozen=False):
    return {"pid": pid, "cg": Path("/nonexistent") / pid, "est": 6 * KIB_PER_GIB, "preemptible": True,
            "target": f"/targets/{pid}", "cur": KIB_PER_GIB, "frozen": frozen, "started": started}


class WatchdogTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.slice = self.root / "slice"
        self.state = self.root / "state"
        self.state.mkdir()
        write_slice(self.slice, cap_gib=64, current_gib=62.5, file_gib=48, shmem_gib=0.1)
        self.wd = load(self.state, self.slice)
        self.avail_gib = 80
        self.wd.avail_kib = lambda: int(self.avail_gib * KIB_PER_GIB)
        self.frozen = []
        self.wd.set_frozen = lambda r, frozen, why: self.frozen.append((r["pid"], frozen))

    def test_clean_page_cache_counts_as_room(self):
        # 2026-11-11: 48 of 62.5 GiB were clean cache; the old formula left 1.5 GiB.
        room_gib = self.wd.headroom_kib() / KIB_PER_GIB
        self.assertAlmostEqual(room_gib, 64 - (62.5 - 47.9) - 4, places=2)

    def test_tmpfs_dirty_and_writeback_pages_are_not_room(self):
        write_slice(self.slice, cap_gib=64, current_gib=62.5, file_gib=48, shmem_gib=10, dirty_gib=20, writeback_gib=10)
        self.assertAlmostEqual(self.wd.unreclaimable_kib(self.slice) / KIB_PER_GIB, 62.5 - 8, places=2)
        self.assertAlmostEqual(self.wd.headroom_kib() / KIB_PER_GIB, 64 - 54.5 - 4, places=2)

    def test_pressure_with_room_freezes_nothing(self):
        self.wd.commands = lambda: [row("1", 1), row("2", 2)]
        self.wd.balance(True)
        self.assertEqual(self.frozen, [])

    def test_pressure_freezes_youngest_once_host_room_is_gone(self):
        self.wd.commands = lambda: [row("1", 1), row("2", 2)]
        self.avail_gib = 4 + 1 / 1024
        self.wd.balance(True)
        self.assertEqual(self.frozen, [])
        self.avail_gib = 4 - 1 / 1024
        self.wd.balance(True)
        self.assertEqual(self.frozen, [("2", True)])

    def test_pressure_freezes_youngest_once_slice_room_is_gone(self):
        self.wd.commands = lambda: [row("1", 1), row("2", 2)]
        write_slice(self.slice, cap_gib=64, current_gib=60.5 - 1 / 1024, file_gib=0.5)
        self.wd.balance(True)
        self.assertEqual(self.frozen, [])
        write_slice(self.slice, cap_gib=64, current_gib=60.5 + 1 / 1024, file_gib=0.5)
        self.wd.balance(True)
        self.assertEqual(self.frozen, [("2", True)])

    def drain(self):
        count = 0
        while True:
            try:
                chunk = os.read(self.wd.JOBSERVER, 4096)
            except BlockingIOError:
                return count
            if not chunk:
                return count
            count += len(chunk)

    def test_fresh_jobserver_gets_full_budget(self):
        self.wd.commands = lambda: [row("1", 1)]
        self.wd.open_jobserver(self.state / "jobserver")
        self.assertEqual(self.drain(), self.wd.JOBS)

    def test_restart_with_live_builds_keeps_their_budget(self):
        path = self.state / "jobserver"
        os.mkfifo(path)
        holder = os.open(path, os.O_RDWR | os.O_NONBLOCK)
        self.addCleanup(os.close, holder)
        os.write(holder, b"++")  # the rest of the tokens are held by running cargos
        self.wd.commands = lambda: [row("1", 1)]
        self.wd.open_jobserver(path)
        self.assertEqual(self.drain(), 2)

    def test_restart_with_nothing_running_resets_budget(self):
        path = self.state / "jobserver"
        os.mkfifo(path)
        holder = os.open(path, os.O_RDWR | os.O_NONBLOCK)
        self.addCleanup(os.close, holder)
        os.write(holder, b"+" * (self.wd.JOBS + 5))
        self.wd.commands = lambda: []
        self.wd.open_jobserver(path)
        self.assertEqual(self.drain(), self.wd.JOBS)


if __name__ == "__main__":
    unittest.main()
