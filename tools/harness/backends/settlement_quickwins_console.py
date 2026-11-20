"""Privileged fixtures for the stage-2 settlement walk through the server's own console.

The stage-2 walk needs a NON-operator player (the owner standing in a room), so no fixture can be a chat
command of that player. The server reads operator commands from stdin (`serve_with_console`); the scene
lifecycle (`scene_shot_scenario`, scenario key `server_console`) wires stdin to `RUN_DIR/console.fifo`, the
mechanism the battle runner already uses (`battle_recovery.FifoConsole`: commands in through the fifo, replies
read back from `server.log` with the harness's push wait). This class only adds what the walk needs on top of it:

* `@s` in a command means the player (the console has no sender), so the stage-1 fixtures read unchanged;
* every command is fenced by two `say` lines with a unique tag, so its reply is exactly the log lines between
  the fences (no other command, chat line or tick log can be attributed to it).
"""
from __future__ import annotations

import re
import uuid
from pathlib import Path

from .battle_recovery import FifoConsole


class ConsoleFixtures(FifoConsole):
    def __init__(self, root: Path, player: str) -> None:
        super().__init__(root)
        self.player = player

    def substitute(self, command: str) -> str:
        """The console has no sender: `@s` becomes the player's name."""
        return re.sub(r"@s\b", self.player, command.lstrip("/"))

    def run(self, command: str, timeout: float = 120.0) -> list[str]:
        """Run one console command; return its reply lines (oldest first), attributed by unique fences."""
        tag = uuid.uuid4().hex[:12]
        begin, end = f"qwc-{tag}-begin", f"qwc-{tag}-end"
        mark = self.mark()
        self.send(f"say {begin}")
        self.send(self.substitute(command))
        self.send(f"say {end}")
        self.wait(re.escape(end), mark, timeout)
        lines = self.text_since(mark).splitlines()
        first = next(index for index, line in enumerate(lines) if begin in line)
        last = next(index for index, line in enumerate(lines) if end in line and index > first)
        return [line.strip() for line in lines[first + 1:last] if line.strip()]

    def fixture(self, command: str) -> list[str]:
        return self.run(command)
