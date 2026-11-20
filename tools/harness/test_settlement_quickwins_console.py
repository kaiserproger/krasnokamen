"""The stage-2 walk's console fixtures: `@s` substitution and exact reply attribution."""
from __future__ import annotations

import os
import tempfile
import threading
import unittest
from pathlib import Path

from tools.harness.backends.settlement_quickwins_console import ConsoleFixtures


class FakeServer:
    """Reads console.fifo like the server's stdin and logs replies like its stdout: `say X` -> `[Server] X`,
    anything else -> `<reply> <line>`; unrelated tick noise is interleaved."""

    def __init__(self, root: Path) -> None:
        self.root = root
        os.mkfifo(root / "console.fifo")
        self.fd = os.open(root / "console.fifo", os.O_RDWR)
        (root / "server.log").write_text("stale: old reply from an earlier command\n")
        self.seen: list[str] = []
        self.stop = False
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    def serve(self) -> None:
        pending = b""
        with (self.root / "server.log").open("a", buffering=1) as log:
            while not self.stop:
                pending += os.read(self.fd, 4096)
                while b"\n" in pending:
                    line, pending = pending.split(b"\n", 1)
                    text = line.decode()
                    self.seen.append(text)
                    log.write("tick noise 1\n")
                    log.write(f"[Server] {text[4:]}\n" if text.startswith("say ") else f"reply {text}\n")
                    log.write("tick noise 2\n")

    def close(self) -> None:
        self.stop = True
        os.write(self.fd, b"say bye\n")
        self.thread.join(timeout=5)
        os.close(self.fd)


class ConsoleFixturesTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.server = FakeServer(self.root)
        self.console = ConsoleFixtures(self.root, "SceneShot")

    def tearDown(self) -> None:
        self.server.close()
        self.tmp.cleanup()

    def test_at_s_is_the_player_and_only_a_selector_token(self) -> None:
        self.assertEqual(self.console.substitute("tp @s 1 2 3"), "tp SceneShot 1 2 3")
        self.assertEqual(self.console.substitute("/gamemode creative @s"), "gamemode creative SceneShot")
        self.assertEqual(self.console.substitute("give @s minecraft:stone 1"), "give SceneShot minecraft:stone 1")
        self.assertEqual(self.console.substitute("kill @sheep"), "kill @sheep")  # not the `@s` selector
        self.assertEqual(self.console.substitute("fill 0 0 0 1 1 1 minecraft:air"), "fill 0 0 0 1 1 1 minecraft:air")

    def test_reply_is_exactly_the_lines_between_this_commands_fences(self) -> None:
        first = self.console.run("data get block 1 2 3 Items")
        second = self.console.run("tp @s 4 5 6")
        self.assertEqual(first, ["tick noise 2", "tick noise 1", "reply data get block 1 2 3 Items",
                                 "tick noise 2", "tick noise 1"])
        self.assertIn("reply data get block 1 2 3 Items", first)
        self.assertNotIn("reply tp SceneShot 4 5 6", first)
        self.assertIn("reply tp SceneShot 4 5 6", second)
        self.assertNotIn("reply data get block 1 2 3 Items", second)
        self.assertFalse(any("stale" in line or "qwc-" in line for line in first + second))

    def test_each_run_uses_its_own_unique_fences(self) -> None:
        self.console.run("list")
        self.console.run("list")
        fences = [line for line in self.server.seen if line.startswith("say qwc-")]
        self.assertEqual(len(fences), 4)
        self.assertEqual(len(set(fences)), 4)


if __name__ == "__main__":
    unittest.main()
