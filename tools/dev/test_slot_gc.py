"""Tests for tools/dev/slot-gc: python3 tools/dev/test_slot_gc.py

Synthetic slots in a temp work root: .d files pointing into existing and missing
trees, incremental dirs with embedded paths, and slots whose pool lock is held.
"""

from __future__ import annotations

import fcntl
import importlib.machinery
import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))


def load_tool(name: str):
    loader = importlib.machinery.SourceFileLoader(name.replace("-", "_"), str(HERE / name))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    module = importlib.util.module_from_spec(spec)
    sys.modules[loader.name] = module  # dataclasses resolve their module by name
    loader.exec_module(module)
    return module


sg = load_tool("slot-gc")


class SlotFixture(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        base = Path(self._tmp.name).resolve()
        self.work = base / "work"
        self.tmproot = base / "tmproot"  # stands in for /tmp
        self.locks = base / "locks"
        self.targets = self.work / "targets"
        for d in (self.work / "wt" / "live", self.work / "solaris" / "crates", self.tmproot / "livedir", self.targets):
            d.mkdir(parents=True)
        self.roots = sg.Roots(work=self.work, targets=self.targets, tmp=self.tmproot)
        self.gone = self.work / "wt" / "gone"  # never created: a removed worktree
        self.gone_tmp = self.tmproot / "gonedir"
        self.live = self.work / "wt" / "live"

    # -- builders -----------------------------------------------------------
    def slot(self, letter: str = "a") -> Path:
        path = self.targets / f"slot-{letter}"
        (path / "debug" / "deps").mkdir(parents=True, exist_ok=True)
        return path

    def unit(self, slot: Path, stem: str, sources: list[str], profile: str = "debug") -> list[Path]:
        """A unit as cargo leaves it: bin-style file, .o, lib*.rlib/.rmeta/.so, and the .d."""
        deps = slot / profile / "deps"
        deps.mkdir(parents=True, exist_ok=True)
        escaped = " ".join(s.replace(" ", "\\ ") for s in sources)
        rules = f"{deps}/{stem}.d: {escaped}\n\n{deps}/lib{stem}.rlib: {escaped}\n\n" + "".join(f"{s}:\n" for s in sources)
        files = [deps / f"{stem}.d", deps / stem, deps / f"{stem}.o", deps / f"lib{stem}.rlib",
                 deps / f"lib{stem}.rmeta", deps / f"lib{stem}.so"]
        for f in files:
            f.write_text(rules if f.suffix == ".d" else "x" * 4096)
        return files

    def incremental(self, slot: Path, name: str, payload: bytes, profile: str = "debug") -> Path:
        crate = slot / profile / "incremental" / name
        session = crate / "s-aaa-bbb-ccc"
        session.mkdir(parents=True)
        (crate / "s-aaa-bbb.lock").write_text("")
        (session / "query-cache.bin").write_bytes(b"\x00\x01junk" + payload + b"\x02\x00tail")
        (session / "dep-graph.bin").write_bytes(b"\x00" * 64)
        (session / "0abc.o").write_bytes(b"o" * 8192)
        return crate

    def run_gc(self, delete: bool, slots: list[str] | None = None) -> dict[str, "sg.SlotReport"]:
        names = slots or sorted(p.name for p in self.targets.glob("slot-*"))
        return {n: sg.gc_slot(self.targets / n, self.roots, self.locks, delete) for n in names}


class UnitTests(SlotFixture):
    def test_dead_units_removed_live_registry_relative_mixed_kept(self):
        slot = self.slot()
        dead = self.unit(slot, "foo-aaaa", [f"{self.gone}/src/lib.rs", "src/util.rs"])  # relative source does not save it
        dead_tmp = self.unit(slot, "tmpcrate-1111", [f"{self.gone_tmp}/src/lib.rs"])
        live = self.unit(slot, "bar-bbbb", [f"{self.live}/src/lib.rs"])
        main = self.unit(slot, "main-9999", [f"{self.work}/solaris/crates/x/src/lib.rs"])
        reg = self.unit(slot, "reg-cccc", ["/home/u/.cargo/registry/src/index/reg-1.0.0/src/lib.rs"])
        rel = self.unit(slot, "rel-dddd", ["crates/x/src/lib.rs"])
        mixed = self.unit(slot, "mix-eeee", [f"{self.gone}/a.rs", f"{self.live}/b.rs"])
        mixed_main = self.unit(slot, "mixm-ffff", [f"{self.gone}/a.rs", f"{self.work}/solaris/crates/b.rs"])
        sibling = self.unit(slot, "foo-aaaab", ["src/lib.rs"])  # stem prefix of the dead unit, must stay
        out = self.unit(slot, "gen-0000", [f"{self.gone}/build.rs", f"{self.targets}/slot-a/debug/build/gen/out/x.rs"])

        report = self.run_gc(delete=False)["slot-a"]
        self.assertEqual(report.units, 3)
        self.assertTrue(all(f.exists() for f in dead + dead_tmp + out), "dry run must not delete")
        report = self.run_gc(delete=True)["slot-a"]
        self.assertEqual(report.units, 3)
        self.assertGreater(report.unit_bytes, 0)
        for f in dead + dead_tmp + out:
            self.assertFalse(f.exists(), f)
        for f in live + main + reg + rel + mixed + mixed_main + sibling:
            self.assertTrue(f.exists(), f)
        self.assertEqual(set(report.trees), {str(self.gone), str(self.gone_tmp)})

    def test_bare_file_in_tmp_root_is_no_tree(self):
        slot = self.slot()
        kept = self.unit(slot, "odd-1234", [f"{self.tmproot}/loose.rs"])
        self.assertEqual(self.run_gc(delete=True)["slot-a"].units, 0)
        self.assertTrue(all(f.exists() for f in kept))

    def test_escaped_space_in_source(self):
        deps = self.slot() / "debug" / "deps"
        (deps / "sp-1.d").write_text(f"{deps}/sp-1.d: {self.gone}/my\\ dir/lib.rs src/b.rs\n")
        self.assertEqual(sg.parse_depinfo(deps / "sp-1.d"), [f"{self.gone}/my dir/lib.rs", "src/b.rs"])

    def test_hard_linked_file_frees_nothing_and_uplift_survives(self):
        slot = self.slot()
        files = self.unit(slot, "bin-aaaa", [f"{self.gone}/main.rs"])
        uplift = slot / "debug" / "bin"
        os.link(files[1], uplift)
        report = self.run_gc(delete=True)["slot-a"]
        self.assertFalse(files[1].exists())
        self.assertTrue(uplift.exists(), "profile-level binaries are never touched")
        self.assertEqual(report.units, 1)

    def test_nested_target_dirs_and_other_profiles_are_found(self):
        slot = self.slot()
        files = self.unit(slot, "deep-aaaa", [f"{self.gone}/lib.rs"], profile="wasm32-unknown-unknown/release")
        self.assertEqual(self.run_gc(delete=True)["slot-a"].units, 1)
        self.assertFalse(files[0].exists())


class IncrementalTests(SlotFixture):
    def setUp(self):
        super().setUp()
        self.slot_a = self.slot()
        self.root_file = f"{self.gone}/src/lib.rs"
        self.unit(self.slot_a, "foo-aaaa", [self.root_file])

    def dirs(self, **payloads: bytes) -> dict[str, Path]:
        for key in payloads:
            self.unit(self.slot_a, f"foo-{key}", [self.root_file])
        return {key: self.incremental(self.slot_a, f"foo-{key}", payload) for key, payload in payloads.items()}

    def test_dead_root_only_is_deleted(self):
        dirs = self.dirs(dead=f"{self.root_file}\0".encode(),
                         registry=f"{self.root_file} /home/u/.cargo/registry/src/x/y.rs /home/u/.rustup/toolchains/t/lib.rs ".encode())
        report = self.run_gc(delete=True)["slot-a"]
        self.assertEqual(report.inc_dirs, 2)
        self.assertGreater(report.inc_bytes, 0)
        for path in dirs.values():
            self.assertFalse(path.exists())

    def test_live_cache_that_also_mentions_a_removed_worktree_is_kept(self):
        """Negative control for the reference scanner: a live cache mentions old spans of a removed tree."""
        dirs = self.dirs(
            main=f"{self.root_file} {self.work}/solaris/crates/mc-core/src/lib.rs".encode(),
            livewt=f"{self.root_file} {self.live}/src/lib.rs".encode(),
            livetmp=f"{self.root_file} {self.tmproot}/livedir/x.rs".encode(),
            otherwork=f"{self.root_file} {self.work}/receipts/x/y".encode(),
            home=f"{self.root_file} /home/u/solaris-client/src/lib.rs".encode(),
        )
        report = self.run_gc(delete=True)["slot-a"]
        self.assertEqual(report.inc_dirs, 0)
        self.assertEqual(report.unattributed, len(dirs))
        for key, path in dirs.items():
            self.assertTrue(path.exists(), key)

    def test_reference_scanner_heuristic_is_not_enough(self):
        """Only a removed-tree mention, but not the root file of any unit: unattributed."""
        dirs = self.dirs(spans=f"{self.gone}/src/other.rs {self.gone}/src/more.rs".encode(), blank=b"relative/src/lib.rs only")
        report = self.run_gc(delete=True)["slot-a"]
        self.assertEqual((report.inc_dirs, report.unattributed), (0, 2))
        self.assertTrue(all(p.exists() for p in dirs.values()))

    def test_crate_without_matching_unit_is_kept(self):
        orphan = self.incremental(self.slot_a, "qux-1abc", f"{self.root_file}".encode())
        self.assertEqual(self.run_gc(delete=True)["slot-a"].inc_dirs, 0)
        self.assertTrue(orphan.exists())

    def test_live_root_unit_never_attributes_a_dir(self):
        self.unit(self.slot_a, "liv-bbbb", [f"{self.live}/src/lib.rs"])
        crate = self.incremental(self.slot_a, "liv-1abc", f"{self.live}/src/lib.rs {self.gone}/old.rs".encode())
        self.assertEqual(self.run_gc(delete=True)["slot-a"].inc_dirs, 0)
        self.assertTrue(crate.exists())

    def test_root_path_straddling_a_chunk_boundary_is_found(self):
        original = sg.CHUNK, sg.OVERLAP
        sg.CHUNK, sg.OVERLAP = 64, 256
        try:
            payload = b"p" * 61 + self.root_file.encode() + b"p" * 100
            crate = self.incremental(self.slot_a, "foo-aaaa", payload)
            self.assertEqual(self.run_gc(delete=True)["slot-a"].inc_dirs, 1)
            self.assertFalse(crate.exists())
        finally:
            sg.CHUNK, sg.OVERLAP = original

    def test_dry_run_keeps_everything(self):
        crate = self.incremental(self.slot_a, "foo-aaaa", self.root_file.encode())
        report = self.run_gc(delete=False)["slot-a"]
        self.assertEqual(report.inc_dirs, 1)
        self.assertTrue(crate.exists())


class LockTests(SlotFixture):
    def hold(self, letter: str) -> int:
        self.locks.mkdir(parents=True, exist_ok=True)
        fd = os.open(self.locks / letter, os.O_WRONLY | os.O_CREAT, 0o664)
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        self.addCleanup(os.close, fd)
        return fd

    def test_busy_slot_is_skipped_idle_slot_is_cleaned(self):
        busy, idle = self.slot("a"), self.slot("b")
        busy_files = self.unit(busy, "foo-aaaa", [f"{self.gone}/lib.rs"])
        idle_files = self.unit(idle, "foo-aaaa", [f"{self.gone}/lib.rs"])
        self.hold("a")
        reports = self.run_gc(delete=True)
        self.assertEqual(reports["slot-a"].status, "busy")
        self.assertEqual(reports["slot-b"].units, 1)
        self.assertTrue(all(f.exists() for f in busy_files))
        self.assertFalse(any(f.exists() for f in idle_files))
        # a dry run also reports the busy slot
        self.assertEqual(self.run_gc(delete=False)["slot-a"].status, "busy")

    def test_lock_is_held_during_delete_and_released_after(self):
        slot = self.slot("a")
        self.unit(slot, "foo-aaaa", [f"{self.gone}/lib.rs"])
        seen = []
        original = sg.process_profile

        def probe(profile, roots, delete, report):
            lock, _ = sg.slot_lock(self.locks, "a")
            seen.append(lock is None)  # held by slot-gc itself while it works
            if lock is not None:
                lock.close()
            return original(profile, roots, delete, report)

        sg.process_profile = probe
        try:
            self.run_gc(delete=True)
        finally:
            sg.process_profile = original
        self.assertEqual(seen, [True])
        lock, _ = sg.slot_lock(self.locks, "a")
        self.assertIsNotNone(lock, "the lock is released afterwards")
        lock.close()

    def test_dry_run_does_not_keep_the_lock(self):
        slot = self.slot("a")
        self.unit(slot, "foo-aaaa", [f"{self.gone}/lib.rs"])
        seen = []
        original = sg.process_profile

        def probe(profile, roots, delete, report):
            lock, _ = sg.slot_lock(self.locks, "a")
            seen.append(lock is not None)
            if lock is not None:
                lock.close()
            return original(profile, roots, delete, report)

        sg.process_profile = probe
        try:
            self.run_gc(delete=False)
        finally:
            sg.process_profile = original
        self.assertEqual(seen, [True])

    def test_slot_without_pool_lock_protocol_is_skipped(self):
        odd = self.targets / "slot-foo" / "debug" / "deps"
        odd.mkdir(parents=True)
        files = self.unit(self.targets / "slot-foo", "foo-aaaa", [f"{self.gone}/lib.rs"])
        self.assertEqual(self.run_gc(delete=True)["slot-foo"].status, "unmanaged")
        self.assertTrue(all(f.exists() for f in files))

    def test_cli_reports_busy_and_deletes_only_with_flag(self):
        busy, idle = self.slot("a"), self.slot("b")
        self.unit(busy, "foo-aaaa", [f"{self.gone}/lib.rs"])
        idle_files = self.unit(idle, "foo-aaaa", [f"{self.gone}/lib.rs"])
        self.hold("a")
        base = [sys.executable, str(HERE / "slot-gc"), "--work", str(self.work), "--targets", str(self.targets),
                "--lock-dir", str(self.locks), "--tmp", str(self.tmproot)]
        dry = subprocess.run(base, capture_output=True, text=True)
        self.assertEqual(dry.returncode, 0, dry.stderr)
        self.assertIn("slot-a: BUSY", dry.stdout)
        self.assertIn("would delete 1 dead units", dry.stdout)
        self.assertTrue(all(f.exists() for f in idle_files))
        real = subprocess.run(base + ["--delete", "-v"], capture_output=True, text=True)
        self.assertEqual(real.returncode, 0, real.stderr)
        self.assertIn("slot-a: BUSY", real.stdout)
        self.assertIn("slot-b: deleted 1 dead units", real.stdout)
        self.assertFalse(any(f.exists() for f in idle_files))
        self.assertIn(str(self.gone), real.stdout)


def hold_in_subprocess(test: unittest.TestCase, path: Path, mode: str) -> subprocess.Popen:
    """A separate process holding flock `mode` (-s or -x) on `path` until the test ends;
    returns once the lock is held (it prints 'held' only after flock succeeded)."""
    path.parent.mkdir(parents=True, exist_ok=True)
    proc = subprocess.Popen(["flock", mode, str(path), "-c", "echo held; exec cat"],
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    test.addCleanup(lambda: (proc.stdin.close(), proc.wait(), proc.stdout.close()))
    test.assertEqual(proc.stdout.readline().strip(), "held")
    return proc


class UseLockTests(SlotFixture):
    """A runner command with an explicit pooled CARGO_TARGET_DIR takes no pool lock but
    holds `<l>.use` shared; slot-gc must not delete under it, whatever its snapshot says."""

    def test_runner_started_after_the_snapshot_is_protected(self):
        slot = self.slot("a")
        files = self.unit(slot, "foo-aaaa", [f"{self.gone}/lib.rs"])
        stale: dict[str, list[int]] = {}  # the snapshot was taken before the runner started
        hold_in_subprocess(self, self.locks / "a.use", "-s")  # mem-run's hold for the explicit target
        report = sg.gc_slot(slot, self.roots, self.locks, True, stale)
        self.assertEqual(report.status, "busy")
        self.assertIn("use lock held", report.busy_reason)
        self.assertTrue(all(f.exists() for f in files))

    def test_runner_use_lock_blocks_rotation(self):
        slot = self.slot("a")
        (slot / "debug" / "incremental" / "x").mkdir(parents=True)
        hold_in_subprocess(self, self.locks / "a.use", "-s")
        evictions, notes, _ = sg.rotate(["slot-a"], self.roots, self.locks, True, 10**9, {})
        self.assertEqual(evictions, [])
        self.assertTrue(any("use lock held" in n for n in notes))
        self.assertTrue((slot / "debug" / "incremental" / "x").exists())

    def test_runner_starting_during_a_pass_waits_for_it(self):
        slot = self.slot("a")
        self.unit(slot, "foo-aaaa", [f"{self.gone}/lib.rs"])
        seen = []
        original = sg.process_profile

        def probe(profile, roots, delete, report):
            # what mem-run's non-blocking first try sees while slot-gc works on the slot
            seen.append(subprocess.run(["flock", "-s", "-n", str(self.locks / "a.use"), "true"]).returncode)
            return original(profile, roots, delete, report)

        sg.process_profile = probe
        try:
            sg.gc_slot(slot, self.roots, self.locks, True, {})
        finally:
            sg.process_profile = original
        self.assertEqual(seen, [1])
        self.assertEqual(subprocess.run(["flock", "-s", "-n", str(self.locks / "a.use"), "true"]).returncode, 0)


class SessionTests(SlotFixture):
    """Class 1: every finalized session but the newest of a crate dir, with its lock."""

    def crate(self, slot: Path, name: str = "foo-1abc") -> Path:
        crate = slot / "debug" / "incremental" / name
        crate.mkdir(parents=True)
        return crate

    def session(self, crate: Path, name: str, mtime: float, size: int = 4096) -> Path:
        path = crate / name
        path.mkdir()
        (path / "query-cache.bin").write_bytes(b"q" * size)
        os.utime(path, (mtime, mtime))
        lock = path.with_name(name.rsplit("-", 1)[0] + ".lock") if not name.endswith("-working") else crate / (name[: -len("-working")] + ".lock")
        lock.write_text("")
        return path

    def test_newest_kept_older_deleted_working_skipped(self):
        slot = self.slot()
        crate = self.crate(slot)
        old = self.session(crate, "s-aaa-111-xxx", 1000)
        mid = self.session(crate, "s-bbb-222-yyy", 2000)
        new = self.session(crate, "s-ccc-333-zzz", 3000)
        working = self.session(crate, "s-ddd-444-working", 500)  # oldest mtime, but never finalized
        dry = self.run_gc(delete=False)["slot-a"]
        self.assertEqual((dry.sess_dirs, dry.sess_bytes), (2, 2 * 4096))
        self.assertTrue(all(p.exists() for p in (old, mid, new, working)), "dry run deletes nothing")
        report = self.run_gc(delete=True)["slot-a"]
        self.assertEqual(report.sess_dirs, 2)
        self.assertFalse(old.exists() or mid.exists())
        self.assertFalse((crate / "s-aaa-111.lock").exists() or (crate / "s-bbb-222.lock").exists())
        self.assertTrue(new.exists() and (crate / "s-ccc-333.lock").exists())
        self.assertTrue(working.exists() and (crate / "s-ddd-444.lock").exists())

    def test_single_session_and_other_crates_untouched(self):
        slot = self.slot()
        one = self.session(self.crate(slot, "foo-1"), "s-a-1-x", 1000)
        crate2 = self.crate(slot, "bar-2")
        self.session(crate2, "s-a-1-x", 1000)
        newest2 = self.session(crate2, "s-b-2-y", 2000)
        self.run_gc(delete=True)
        self.assertTrue(one.exists() and newest2.exists())
        self.assertFalse((crate2 / "s-a-1-x").exists())

    def test_hard_linked_bytes_are_not_counted(self):
        slot = self.slot()
        crate = self.crate(slot)
        old = self.session(crate, "s-a-1-x", 1000, size=8192)
        self.session(crate, "s-b-2-y", 2000)
        os.link(old / "query-cache.bin", slot / "kept-link")  # another name keeps the blocks
        self.assertEqual(self.run_gc(delete=False)["slot-a"].sess_bytes, 0)

    def test_live_path_in_older_session_prevents_whole_crate_deletion(self):
        slot = self.slot()
        root = f"{self.gone}/src/lib.rs"
        self.unit(slot, "foo-aaaa", [root])
        crate = self.crate(slot, "foo-aaaa")
        stale = self.session(crate, "s-a-1-x", 1000)
        (stale / "query-cache.bin").write_bytes(f"{self.live}/src/lib.rs".encode())  # superseded, mentions a live tree
        newest = self.session(crate, "s-b-2-y", 2000)
        (newest / "query-cache.bin").write_bytes(root.encode())
        report = self.run_gc(delete=True)["slot-a"]
        self.assertEqual(report.inc_dirs, 0)
        self.assertTrue(crate.exists())
        self.assertTrue(newest.exists())

    def test_old_session_whose_lock_rustc_holds_is_kept(self):
        slot = self.slot()
        crate = self.crate(slot)
        oldest = self.session(crate, "s-a-1-x", 1000)
        held = self.session(crate, "s-b-2-y", 2000)  # e.g. a running rustc copying it as its source
        newest = self.session(crate, "s-c-3-z", 3000)
        hold_in_subprocess(self, crate / "s-b-2.lock", "-x")
        report = self.run_gc(delete=True)["slot-a"]
        self.assertEqual((report.sess_dirs, report.sess_locked), (1, 1))
        self.assertFalse(oldest.exists())
        self.assertTrue(held.exists() and (crate / "s-b-2.lock").exists() and newest.exists())

    def test_rustc_name_timestamp_decides_the_newest_not_mtime(self):
        slot = self.slot()
        crate = self.crate(slot)
        named_newest = self.session(crate, "s-zz-1-x", 1000)  # base-36 "zz" > "a", but the older mtime
        named_older = self.session(crate, "s-a-2-y", 2000)
        self.run_gc(delete=True)
        self.assertTrue(named_newest.exists())
        self.assertFalse(named_older.exists())


class RotationTests(SlotFixture):
    """Class 2. Free space is simulated: 5 GiB plus one GiB per 4096 bytes removed from the slots."""

    def setUp(self):
        super().setUp()
        for letter in "abc":
            slot = self.slot(letter)
            (slot / "debug" / ".fingerprint" / "x").mkdir(parents=True)
            (slot / "debug" / "deps" / "libx.rlib").write_bytes(b"d" * 4096)
            crate = slot / "debug" / "incremental" / "foo-1" / "s-a-1-x"
            crate.mkdir(parents=True)
            (crate / "query-cache.bin").write_bytes(b"i" * 8192)
            (slot / "tmp").mkdir()
            (slot / "tmp" / "keep").write_bytes(b"t" * 4096)
            (slot / ".krasnokamen-tree").write_bytes(b"s" * 4096)
        self.locks.mkdir(exist_ok=True)
        self.used = {}
        self.baseline = self.total()
        self.original_free = sg.free_gib
        sg.free_gib = lambda path: 5.0 + (self.baseline - self.total()) / sg.GIB
        sg.GIB = 4096  # one simulated GiB is 4096 bytes
        self.addCleanup(setattr, sg, "free_gib", self.original_free)
        self.addCleanup(setattr, sg, "GIB", 2**30)

    def total(self) -> int:
        return sum(f.stat().st_size for f in self.targets.rglob("*") if f.is_file())

    def stamp(self, letter: str, when: float, lock: float | None = None) -> None:
        fp = self.targets / f"slot-{letter}" / "debug" / ".fingerprint" / "x"
        (fp / "invoked.timestamp").write_text("")
        for path in (fp / "invoked.timestamp", fp, fp.parent):
            os.utime(path, (when, when))
        if lock is not None:
            (self.locks / letter).write_text("")
            os.utime(self.locks / letter, (lock, lock))

    def rotate(self, threshold: float, delete: bool = True, users=None):
        names = sorted(p.name for p in self.targets.glob("slot-*"))
        return sg.rotate(names, self.roots, self.locks, delete, threshold, users or {})

    def test_lru_order_and_incremental_first(self):
        self.stamp("a", 1000)
        self.stamp("b", 3000)
        self.stamp("c", 2000)
        evictions, notes, errors = self.rotate(threshold=7)  # incremental of the oldest slot is enough
        self.assertEqual(errors, [])
        self.assertEqual([(e.slot, e.stage) for e in evictions], [("slot-a", "incremental")])
        self.assertFalse((self.targets / "slot-a" / "debug" / "incremental").exists())
        self.assertTrue((self.targets / "slot-a" / "debug" / "deps" / "libx.rlib").exists(), "the rest of the slot stays")
        self.assertIn("last use", evictions[0].reason)
        evictions, _, _ = self.rotate(threshold=12)  # a (+2 more: whole slot), then the next least recently used: c
        self.assertEqual([(e.slot, e.stage) for e in evictions], [("slot-a", "slot"), ("slot-c", "incremental"), ("slot-c", "slot")])
        self.assertTrue((self.targets / "slot-b" / "debug" / "incremental").exists(), "the most recently used slot is untouched")

    def test_lock_mtime_newer_than_fingerprints_counts_as_last_use(self):
        self.stamp("a", 1000, lock=9000)  # slot a ran most recently
        self.stamp("b", 2000)
        self.stamp("c", 3000)
        evictions, _, _ = self.rotate(threshold=12)
        self.assertEqual([(e.slot, e.stage) for e in evictions],
                         [("slot-b", "incremental"), ("slot-b", "slot"), ("slot-c", "incremental"), ("slot-c", "slot")])

    def test_whole_slot_except_tmp_when_incremental_is_not_enough(self):
        self.stamp("a", 1000)
        self.stamp("b", 2000)
        self.stamp("c", 3000)
        evictions, _, errors = self.rotate(threshold=8)  # incremental of a gives 7; the rest of a must go too
        self.assertEqual(errors, [])
        self.assertEqual([(e.slot, e.stage) for e in evictions], [("slot-a", "incremental"), ("slot-a", "slot")])
        a = self.targets / "slot-a"
        self.assertEqual(sorted(p.name for p in a.iterdir()), ["tmp"])
        self.assertEqual((a / "tmp" / "keep").read_bytes(), b"t" * 4096)
        self.assertTrue((self.targets / "slot-b" / "debug" / "incremental").exists())
        self.assertIn("incremental was not enough", evictions[1].reason)

    def test_busy_slots_are_never_rotated(self):
        self.stamp("a", 1000)
        self.stamp("b", 2000)
        self.stamp("c", 3000)
        fd = os.open(self.locks / "a", os.O_WRONLY | os.O_CREAT)
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)  # slot a: pool lock held, and the oldest
        self.addCleanup(os.close, fd)
        users = {str(self.targets / "slot-b"): [4242]}  # slot b: a process runs with CARGO_TARGET_DIR=slot-b
        before = {n: self.total_of(n) for n in ("a", "b")}
        evictions, notes, _ = self.rotate(threshold=100, users=users)  # unreachable: everything idle goes
        self.assertEqual({e.slot for e in evictions}, {"slot-c"})
        self.assertEqual(before, {n: self.total_of(n) for n in ("a", "b")})
        self.assertTrue(any("slot-a: busy (pool lock held)" in n for n in notes))
        self.assertTrue(any("slot-b: busy (CARGO_TARGET_DIR of pid 4242)" in n for n in notes))
        self.assertTrue(any("no idle slot left" in n for n in notes))

    def total_of(self, letter: str) -> int:
        return sum(f.stat().st_size for f in (self.targets / f"slot-{letter}").rglob("*") if f.is_file())

    def test_nothing_happens_when_already_above_the_threshold(self):
        self.stamp("a", 1000)
        before = self.total()
        evictions, notes, errors = self.rotate(threshold=5)  # free is exactly the threshold
        self.assertEqual((evictions, notes, errors), ([], [], []))
        self.assertEqual(before, self.total())

    def test_dry_run_plans_the_same_evictions_and_removes_nothing(self):
        self.stamp("a", 1000)
        self.stamp("b", 2000)
        self.stamp("c", 3000)
        before = self.total()
        planned, _, _ = self.rotate(threshold=8, delete=False)
        self.assertEqual(self.total(), before)
        self.assertEqual([(e.slot, e.stage) for e in planned], [("slot-a", "incremental"), ("slot-a", "slot")])
        self.assertEqual(planned[0].bytes, 8192)
        self.assertEqual(planned[1].bytes, 8192)  # libx.rlib + the .krasnokamen-tree marker; tmp/ stays

    def test_safe_classes_count_towards_the_threshold(self):
        self.stamp("a", 1000)
        evictions, _, _ = sg.rotate(["slot-a"], self.roots, self.locks, False, 8, {}, freed_estimate=3 * 4096)
        self.assertEqual(evictions, [])  # 5 + 3 already clears 8

    def test_leftover_trash_is_finished_by_the_next_pass(self):
        slot = self.targets / "slot-a"
        (slot / ".gc-trash-99" / "0-incremental").mkdir(parents=True)
        (slot / ".gc-trash-99" / "0-incremental" / "f").write_text("x")
        self.run_gc(delete=False)
        self.assertTrue((slot / ".gc-trash-99").exists(), "a dry run leaves it")
        self.run_gc(delete=True)
        self.assertFalse((slot / ".gc-trash-99").exists())

    def test_cli_rotates_idle_slot_and_skips_locked_one(self):
        self.stamp("a", 1000)
        self.stamp("b", 2000)
        fd = os.open(self.locks / "b", os.O_WRONLY | os.O_CREAT)
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        self.addCleanup(os.close, fd)
        base = [sys.executable, str(HERE / "slot-gc"), "--work", str(self.work), "--targets", str(self.targets),
                "--lock-dir", str(self.locks), "--tmp", str(self.tmproot), "--min-free-gb", "1000000"]
        dry = subprocess.run(base, capture_output=True, text=True)
        self.assertIn("rotation: would evict slot-a incremental dirs", dry.stdout)
        self.assertTrue((self.targets / "slot-a" / "debug").exists())
        real = subprocess.run(base + ["--delete"], capture_output=True, text=True)
        self.assertEqual(real.returncode, 0, real.stderr)
        self.assertIn("rotation: evicted slot-a everything except tmp/", real.stdout)
        self.assertIn("slot-b: busy (pool lock held)", real.stdout)
        self.assertEqual(sorted(p.name for p in (self.targets / "slot-a").iterdir()), ["tmp"])
        self.assertTrue((self.targets / "slot-b" / "debug" / "incremental").exists())
        off = subprocess.run(base + ["--delete", "--no-rotate"], capture_output=True, text=True)
        self.assertNotIn("rotation:", off.stdout)


class TargetDirUserTests(SlotFixture):
    def test_scan_finds_processes_with_the_slot_as_target_dir(self):
        proc = Path(self._tmp.name) / "proc"
        slot = self.slot("a")
        for pid, env in (("100", f"PATH=/bin\0CARGO_TARGET_DIR={slot}\0"), ("101", f"CARGO_TARGET_DIR={slot}/debug\0"),
                         ("102", "CARGO_TARGET_DIR=/elsewhere\0"), ("103", "HOME=/x\0")):
            (proc / pid).mkdir(parents=True)
            (proc / pid / "environ").write_bytes(env.encode().replace(b"\\0", b"\0"))
        (proc / "self").mkdir()
        users = sg.target_dir_users(proc)
        self.assertEqual(sg.users_of(slot, users), [100, 101])
        self.assertEqual(sg.users_of(self.slot("b"), users), [])

    def test_gc_slot_skips_a_slot_used_through_cargo_target_dir(self):
        slot = self.slot("a")
        files = self.unit(slot, "foo-aaaa", [f"{self.gone}/lib.rs"])
        report = sg.gc_slot(slot, self.roots, self.locks, True, {os.path.realpath(slot): [7]})
        self.assertEqual(report.status, "busy")
        self.assertIn("pid 7", report.busy_reason)
        self.assertTrue(all(f.exists() for f in files))


if __name__ == "__main__":
    unittest.main()
