"""Driver for one real vanilla server used as the parity oracle.

The world is a void superflat world; every scenario force-loads its box, runs
setup/phase commands through the server console, advances a frozen world by
exactly N ticks (`VanillaOracle.sprint`, checked against the game time) and
reads blocks/entities back from the saved Anvil files. No sleeps: every wait is
on a specific server log line.
"""
from __future__ import annotations

import json
import os
import queue
import re
import shutil
import socket
import subprocess
import threading
import time
import zipfile
from pathlib import Path

from . import anvil
from .. import runtime

REPO_ROOT = Path(__file__).resolve().parents[3]
MEM_RUN = Path("/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/mem-run.sh")
# `<work>` on the owner host; KRASNOKAMEN_WORK names it elsewhere.
WORK_ROOT = Path(os.environ.get("KRASNOKAMEN_WORK") or MEM_RUN.parents[3])
# Owner-authorized tool path (D128): Temurin JDK and the vanilla server bundlers, never in Git.
TOOLS_CACHE = WORK_ROOT / "tools-cache"
DEFAULT_JAVA = TOOLS_CACHE / "jdk" / "temurin-25" / "bin" / "java"


def resolve_java(explicit: str | None = None) -> str:
    """`--java`, else KRASNOKAMEN_ORACLE_JAVA, else the tools-cache Temurin 25; never PATH."""
    java = explicit or os.environ.get("KRASNOKAMEN_ORACLE_JAVA") or str(DEFAULT_JAVA)
    if not Path(java).is_file():
        raise VanillaError(f"no Java 25 at {java}: install Temurin 25 at {DEFAULT_JAVA.parent.parent} "
                           "(docs/QUIRK_PARITY.md) or set KRASNOKAMEN_ORACLE_JAVA")
    return java


def resolve_jar(mc_version: str, explicit: str | None = None) -> Path:
    """`--jar`, else `<work>/tools-cache/minecraft/<version>/server.jar` (the Mojang bundler).

    The bundler's own `version.json` must name `mc_version`: a recording is stored as that
    release, so a jar of another release is refused before any server starts.
    """
    jar = Path(explicit) if explicit else TOOLS_CACHE / "minecraft" / mc_version / "server.jar"
    if not jar.is_file():
        raise VanillaError(f"no vanilla {mc_version} server bundler at {jar} (docs/QUIRK_PARITY.md)")
    try:
        with zipfile.ZipFile(jar) as archive:
            names = set(archive.namelist())
            version = json.loads(archive.read("version.json"))
    except (OSError, KeyError, ValueError, zipfile.BadZipFile) as error:
        raise VanillaError(f"{jar} is not a vanilla server bundler with a version.json: {error}") from error
    if version.get("id") != mc_version:
        raise VanillaError(f"{jar} is vanilla {version.get('id')!r}, not {mc_version}")
    if "META-INF/versions.list" not in names:
        raise VanillaError(f"{jar} is the inner game jar, not the server bundler (no META-INF/versions.list)")
    return jar


def java_version(java: str) -> str:
    """First line of `java -version` (the runtime that produced a recording)."""
    out = subprocess.run([java, "-version"], capture_output=True, text=True, check=True)
    return (out.stderr or out.stdout).splitlines()[0].strip()


_ERROR_MARKERS = (
    "Unknown or incomplete command",
    "Could not ",
    "That position is not loaded",
    "Expected ",
    "Incorrect argument",
    "No entity was found",
    "Unknown ",
)


class VanillaError(RuntimeError):
    pass


class VanillaOracle:
    def __init__(self, workdir: Path, *, seed: int, jar: Path,
                 java: str = "java", timeout: float = 180.0, use_mem_run: bool = True,
                 properties: dict[str, str] | None = None):
        self.workdir = workdir
        self.seed = seed
        self.jar = jar
        self.java = java
        self.timeout = timeout
        self.use_mem_run = use_mem_run and MEM_RUN.is_file()
        self.extra_properties = properties or {}
        self.process: subprocess.Popen | None = None
        self.lines: queue.Queue[str | None] = queue.Queue()
        self.log_path = workdir / "vanilla.log"
        self._sync = 0
        self.game_ticks = 0

    # -- lifecycle ---------------------------------------------------------
    def start(self, fresh: bool = True) -> None:
        """Start the server; `fresh=False` reopens the existing world (a restart)."""
        if not self.jar.is_file():
            raise VanillaError(f"missing vanilla bundler jar: {self.jar}")
        if fresh:
            if self.workdir.exists():
                shutil.rmtree(self.workdir)
            self.workdir.mkdir(parents=True)
            (self.workdir / "eula.txt").write_text("eula=true\n")
        self.lines = queue.Queue()
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        self.port = port
        properties = {
            "online-mode": "false", "server-ip": "127.0.0.1", "server-port": str(port),
            "level-name": "world", "level-seed": str(self.seed),
            "level-type": "minecraft:flat",
            "generator-settings": '{"biome":"minecraft:plains","layers":[],"features":false,"lakes":false,"structure_overrides":[]}',
            "generate-structures": "false", "view-distance": "2", "simulation-distance": "2",
            "spawn-protection": "0", "pause-when-empty-seconds": "-1", "max-tick-time": "-1",
            "enable-rcon": "false", "enable-query": "false", "difficulty": "normal",
            "sync-chunk-writes": "false", "max-players": "1", "spawn-monsters": "false",
            "spawn-animals": "false", "spawn-npcs": "false", "enable-command-block": "true",
            "allow-nether": "true",
        }
        properties.update(self.extra_properties)
        (self.workdir / "server.properties").write_text(
            "".join(f"{key}={value}\n" for key, value in properties.items()))
        argv = [self.java, "-Xms256M", "-Xmx1280M", "-jar", str(self.jar), "nogui"]
        if self.use_mem_run:
            argv = [str(MEM_RUN), "-m", "2G"] + argv
        self.process = runtime.start_server(
            argv, cwd=self.workdir, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, text=True, bufsize=1)

        def collect(lines=self.lines, mode="w" if fresh else "a"):
            assert self.process and self.process.stdout
            with self.log_path.open(mode) as handle:
                for line in self.process.stdout:
                    handle.write(line)
                    handle.flush()
                    lines.put(line.rstrip())
            lines.put(None)

        threading.Thread(target=collect, daemon=True).start()
        # mem-run may queue the JVM behind other heavy jobs for a long time: wait on the log line.
        self.expect("Done (", timeout=3600)

    def stop(self) -> None:
        proc = self.process
        if proc is None or proc.poll() is not None:
            return
        try:
            assert proc.stdin
            proc.stdin.write("stop\n")
            proc.stdin.flush()
            proc.wait(timeout=60)
        except Exception:
            proc.kill()
            proc.wait(timeout=10)

    def restart(self) -> None:
        """Stop cleanly (vanilla saves every loaded chunk on `stop`) and reopen the same world."""
        proc = self.process
        self.stop()
        if proc is None or proc.returncode != 0:
            raise VanillaError(f"vanilla did not stop cleanly before the restart; see {self.log_path}")
        self.start(fresh=False)

    def __enter__(self):
        self.start()
        return self

    def __exit__(self, *_exc):
        self.stop()

    # -- log waits ---------------------------------------------------------
    def expect(self, fragment: str, *, timeout: float | None = None,
               collect: list[str] | None = None) -> str:
        deadline = time.monotonic() + (timeout or self.timeout)
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise VanillaError(f"timed out waiting for {fragment!r}; see {self.log_path}")
            try:
                line = self.lines.get(timeout=remaining)
            except queue.Empty:
                continue
            if line is None:
                raise VanillaError(f"vanilla exited waiting for {fragment!r}; see {self.log_path}")
            if collect is not None:
                collect.append(line)
            if fragment in line:
                return line

    def send(self, command: str) -> None:
        assert self.process and self.process.stdin
        self.process.stdin.write(command + "\n")
        self.process.stdin.flush()

    # -- commands ----------------------------------------------------------
    def run(self, command: str) -> list[str]:
        """Execute one console command and return its feedback/error lines."""
        self._sync += 1
        marker = f"QP_SYNC_{self._sync}"
        collected: list[str] = []
        self.send(command)
        self.send(f"say {marker}")
        self.expect(f"[Server] {marker}", collect=collected)
        collected.pop()
        return [line.split("]: ", 1)[-1] for line in collected]

    def run_checked(self, commands: list[str], chunk: int = 200) -> list[str]:
        """Run commands pipelined (one sync per chunk); return rejection feedback lines.

        Feedback lines cannot be tied to the command that caused them, so a
        rejection row carries the chunk's first command for orientation.
        """
        problems = []
        for start in range(0, len(commands), chunk):
            batch = commands[start:start + chunk]
            self._sync += 1
            marker = f"QP_SYNC_{self._sync}"
            collected: list[str] = []
            for command in batch:
                self.send(command)
            self.send(f"say {marker}")
            self.expect(f"[Server] {marker}", timeout=max(self.timeout, 60 + len(batch)), collect=collected)
            for line in collected[:-1]:
                text = line.split("]: ", 1)[-1]
                if "Could not set the block" in text or "No entity was found" in text:
                    continue  # target already held that block: not a rejection
                if any(marker_ in text for marker_ in _ERROR_MARKERS):
                    problems.append(f"[chunk@{start}: {batch[0]} ..] {text}")
        return problems

    def game_time(self) -> int:
        for line in self.run("time query gametime"):
            found = re.search(r"The game time is (\d+) tick", line)
            if found:
                return int(found.group(1))
        raise VanillaError(f"`time query gametime` printed no game time; see {self.log_path}")

    def sprint(self, ticks: int) -> None:
        """Advance the frozen world by exactly `ticks` game ticks.

        From a frozen world `tick sprint N` runs N + 1 ticks: the sprint check reads the frozen
        tick's stale `runGameElements`, so the first unfrozen tick is not counted
        (ServerTickRateManager.checkShouldSprintThisTick, 26.3). One tick is `tick step 1`,
        whose tick runs before the next console command. The game time proves the count.
        """
        if ticks <= 0:
            return
        before = self.game_time()
        if ticks == 1:
            self.send("tick step 1")
            self.expect("Stepping 1 tick")
        else:
            self.send(f"tick sprint {ticks - 1}")
            self.expect("Sprint completed")
        ran = self.game_time() - before
        if ran != ticks:
            raise VanillaError(f"the world advanced {ran} game ticks instead of {ticks}; see {self.log_path}")
        self.game_ticks += ticks

    def freeze(self) -> None:
        self.run("tick freeze")

    # -- observation -------------------------------------------------------
    def snapshot(self, boxes, dimension: str = "overworld") -> dict:
        """Snapshot of the saved world: union plus per-box `cells` when several boxes."""
        self.send("save-all flush")
        self.expect("Saved the game", timeout=120)
        base = self.workdir / "world" / "dimensions" / "minecraft" / dimension
        cells = []
        for cell in anvil.read_world(base, boxes):
            cells.append({"blocks": cell["blocks"], "containers": cell["containers"],
                          "entities": [normalize_entity(nbt) for nbt in cell["entities"]]})
        out = {"blocks": {}, "entities": [], "containers": {}}
        for cell in cells:
            out["blocks"].update(cell["blocks"])
            out["containers"].update(cell["containers"])
            out["entities"].extend(cell["entities"])
        if len(boxes) > 1:
            out["cells"] = cells
        return out


def normalize_entity(nbt: dict) -> dict:
    entry = {
        "type": nbt.get("id"),
        "pos": nbt.get("Pos"),
        "motion": nbt.get("Motion"),
    }
    if "Health" in nbt:
        entry["health"] = nbt["Health"]
    item = nbt.get("Item")
    if isinstance(item, dict):
        entry["item"] = item.get("id")
        entry["count"] = item.get("count", item.get("Count", 1))
    if "fuse" in nbt:
        entry["fuse"] = nbt["fuse"]
    if "Age" in nbt:
        entry["age"] = nbt["Age"]
    return entry
