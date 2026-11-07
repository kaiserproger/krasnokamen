"""Reusable isolated release-server lifecycle for the RunProto matrix.

Primitive only (no CLI/runner): invoked via the existing cmd_run path.
One small class over the canonical helpers: config + ``runtime`` process
lifecycle + ``wire`` protocol join. No sleeps, no alternate framework.

Repo evidence used (never memory):
* ``tools/harness/backends/pause_empty.py`` — TOML shape, ``Popen`` argv,
  ``runtime.wait_port``, stdin ``save-all`` + ``"Saved the game"`` fence,
  ``wire.probe_status`` / ``join_offline_socket`` / ``wait_play_loop``.
* ``crates/mc-edge/src/server.rs`` — exact fences ``"Stopping the server"``,
  ``"Saving the game (this may take a moment!)"`` / ``"Saved the game"``.
* ``crates/mc-edge/src/console.rs`` — console-only ``backup`` (detached,
  outcome prints on completion), ``"Backing up the world (the server keeps
  running)"`` then ``"Backup <path> complete: checkpoint tick N,
  recoverable to tick M, K files, T ms"`` or ``"Backup failed: ..."`` /
  ``"Backups are disabled: set backup_dir under [data]"``.
* ``crates/mc-edge/src/operator_cli.rs`` + ``backup.rs`` — offline
  ``backup --world-dir --out`` / ``restore --from --world-dir`` CLI; restore
  verifies every manifest hash before writing and prints
  ``"restore of ... into ... complete: N files verified"``.
"""

from __future__ import annotations

import json
import re
import socket
import subprocess
import urllib.request
from pathlib import Path
from typing import Any

from .. import runtime, wire

PORT_MIN = 26100
PORT_MAX = 26199

SAVE_DONE = "Saved the game"
STOP_LINE = "Stopping the server"
PAUSE_LOG = "server empty, pausing world"
BACKUP_DONE_RE = re.compile(
    r"^Backup (.+) complete: checkpoint tick (\d+), recoverable to tick (\d+),"
    r" (\d+) files, (\d+) ms$"
)
BACKUP_FAILED = "Backup failed:"
BACKUP_DISABLED = "Backups are disabled"
RESTORE_DONE_RE = re.compile(r"(\d+) files verified")


class Blocked(RuntimeError):
    """Missing real prerequisite (binary, vanilla data, plugins, ports, config)."""


def _check_port(port: int, name: str, port_range=(PORT_MIN, PORT_MAX)) -> int:
    if not isinstance(port, int) or not (port_range[0] <= port <= port_range[1]):
        raise ValueError(f"{name} must be an int in {port_range}, got {port!r}")
    return port


def reserve_matrix_port(exclude: int = -1, port_range=(PORT_MIN, PORT_MAX)) -> int:
    """Bind-probe a free port inside this driver's assigned range."""
    for port in range(port_range[0], port_range[1] + 1):
        if port == exclude:
            continue
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
            try:
                sock.bind(("127.0.0.1", port))
            except OSError:
                continue
            return port
    raise Blocked(f"no free matrix port in {port_range}")


class ProtocolServer:
    """Isolated release server: config, start, console fence, stop/restart,
    hot backup + restore proof, pause-empty probe.

    Parent supplies per slice: release binary, fresh run_dir, opaque 261xx
    port (+ optional explicit 261xx dashboard port; else a free 26100..26199
    port is bind-probed), vanilla_data dir, per-run plugin copy, seed.
    Server logs always append so restarts preserve prior evidence.
    Every console call waits for its exact inotify log fence and fail-closes.
    """

    def __init__(
        self,
        *,
        binary: Path,
        run_dir: Path,
        port: int,
        vanilla_data: Path,
        seed: int,
        plugins_dir: Path | None = None,
        dashboard_port: int | None = None,
        server_name: str = "runproto",
        pause_when_empty_seconds: int = 0,
        view_distance: int = 2,
        startup_timeout_seconds: float = 120.0,
        command_timeout_seconds: float = 120.0,
        port_range: tuple[int, int] = (PORT_MIN, PORT_MAX),
    ) -> None:
        self.binary = Path(binary)
        self.run_dir = Path(run_dir)
        self.port_range = port_range
        self.port = _check_port(port, "port", port_range)
        if dashboard_port is not None:
            _check_port(dashboard_port, "dashboard_port", port_range)
            if dashboard_port == self.port:
                raise ValueError("dashboard_port must differ from port")
        if not isinstance(seed, int):
            raise ValueError(f"seed must be an int, got {seed!r}")
        self.vanilla_data = Path(vanilla_data)
        self.seed = seed
        self.plugins_dir = Path(plugins_dir) if plugins_dir is not None else None
        self.dashboard_port: int | None = dashboard_port
        self.server_name = server_name
        self.pause_when_empty_seconds = pause_when_empty_seconds
        self.view_distance = view_distance
        self.startup_timeout_seconds = startup_timeout_seconds
        self.command_timeout_seconds = command_timeout_seconds
        self.config_path = self.run_dir / "runproto.toml"
        self.log_path = self.run_dir / "server.log"
        self.world_dir = self.run_dir / "world"
        self.backup_dir = self.run_dir / "backups"
        self._process: subprocess.Popen[Any] | None = None
        self._log_handle: Any = None
        self._argv: list[str] = []
        self._command_number = 0
        self._boundary_log_offset = 0
        self.dimension = "overworld"
        self.ensure_loaded_callback = None
        self.config_overrides: dict[str, dict[str, Any]] = {}
        self.enabled_plugins: list[str] = []

    # -- config ---------------------------------------------------------
    def write_config(self) -> Path:
        if self.dashboard_port is None:
            self.dashboard_port = reserve_matrix_port(exclude=self.port, port_range=self.port_range)
        assert self.dashboard_port is not None
        self.run_dir.mkdir(parents=True, exist_ok=True)
        self.world_dir.mkdir(parents=True, exist_ok=True)
        self.backup_dir.mkdir(parents=True, exist_ok=True)
        plugins = (
            f"\n[plugins]\ndirectory = {json.dumps(str(self.plugins_dir))}\n"
            if self.plugins_dir is not None
            else ""
        )
        self.config_path.write_text(
            f"""[server]
name = {json.dumps(self.server_name)}
view_distance = {self.view_distance}
simulation_distance = {self.view_distance}
pause_when_empty_seconds = {self.pause_when_empty_seconds}

[network]
bind_address = "127.0.0.1"
port = {self.port}

[auth]
online_mode = false
require_plugin_login = {str(self.plugins_dir is not None and (self.plugins_dir / "solaris-auth").is_dir()).lower()}

[dashboard]
enabled = true
bind_address = "127.0.0.1"
port = {self.dashboard_port}
{plugins}
[data]
world_dir = {json.dumps(str(self.world_dir))}
vanilla_data_dir = {json.dumps(str(self.vanilla_data))}
backup_dir = {json.dumps(str(self.backup_dir))}
seed = {self.seed}
"""
        )
        if self.config_overrides:
            import tomllib
            config = tomllib.loads(self.config_path.read_text())
            for section, values in self.config_overrides.items():
                if not section.isidentifier() or not isinstance(values, dict):
                    raise ValueError("config overrides require named TOML sections")
                config.setdefault(section, {}).update(values)
            lines = []
            for section, values in config.items():
                lines.append(f"[{section}]")
                for key, value in values.items():
                    if not key.isidentifier() or value is None or isinstance(value, dict):
                        raise ValueError(f"unsupported config override {section}.{key}")
                    lines.append(f"{key} = {json.dumps(value)}")
                lines.append("")
            rendered = "\n".join(lines)
            tomllib.loads(rendered)
            self.config_path.write_text(rendered)
        return self.config_path

    def ensure_loaded(self, area) -> None:
        """Fixture chunk fence from the driver's actual received chunk signal.

        Forceload acknowledgement alone is never treated as chunk residency.
        Drivers bind ensure_loaded_callback to a real client's chunk observer.
        """
        from ..matrix_api import MissingCapability
        if not callable(self.ensure_loaded_callback):
            raise MissingCapability("server:chunk_loaded_signal")
        if len(area) == 3 and all(type(value) is int for value in area):
            low = high = tuple(area)
        else:
            low, high = area
        if len(low) != 3 or len(high) != 3 or any(low[i] > high[i] for i in range(3)):
            raise ValueError("ensure_loaded area must be ordered (min_xyz,max_xyz)")
        self.command(f"forceload add {low[0]} {low[2]} {high[0]} {high[2]}")
        self.ensure_loaded_callback((tuple(low), tuple(high)))

    def reconfigure(self, overrides: dict[str, dict], plugins_dir: Path | None = None) -> None:
        """Restart this same isolated server with explicit core config fixtures."""
        was_running = self.running
        if was_running:
            self.stop()
        for section, values in overrides.items():
            self.config_overrides.setdefault(section, {}).update(values)
        if plugins_dir is not None:
            self.plugins_dir = Path(plugins_dir)
        if was_running:
            self.start()
        else:
            self.write_config()

    # -- lifecycle ------------------------------------------------------
    def _require_prerequisites(self) -> None:
        if not (self.binary.is_file()):
            raise Blocked(f"missing server binary: {self.binary}")
        if not self.vanilla_data.is_dir():
            raise Blocked(f"missing vanilla_data dir: {self.vanilla_data}")
        if self.plugins_dir is not None and not self.plugins_dir.is_dir():
            raise Blocked(f"missing plugins_dir: {self.plugins_dir}")

    @property
    def running(self) -> bool:
        return self._process is not None and self._process.poll() is None

    def start(self, timeout: float | None = None) -> None:
        if self.running:
            raise RuntimeError("server already running")
        self._require_prerequisites()
        self.write_config()
        limit = timeout if timeout is not None else self.startup_timeout_seconds
        self._argv = [str(self.binary), "--config", str(self.config_path)]
        self.log_path.parent.mkdir(parents=True, exist_ok=True)
        # Append: fresh run dirs start empty; restarts keep prior evidence.
        self._log_handle = self.log_path.open("ab")
        try:
            self._process = runtime.start_server(
                self._argv,
                cwd=runtime.REPO_ROOT,
                stdin=subprocess.PIPE,
                stdout=self._log_handle,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            runtime.wait_port(self.port, limit, self._process, log_path=self.log_path)
        except BaseException:
            runtime.stop_process(self._process)
            self._process = None
            try:
                if self._log_handle is not None:
                    self._log_handle.close()
            finally:
                self._log_handle = None
            raise

    def console(self, command: str, expect: str, timeout: float | None = None) -> str:
        proc = self._process
        if proc is None or proc.poll() is not None:
            raise RuntimeError("server not running")
        if proc.stdin is None:
            raise RuntimeError("server console is unavailable")
        limit = timeout if timeout is not None else self.command_timeout_seconds
        before = self.log_path.read_text(errors="replace").count(expect)

        def reached() -> tuple[bool, str]:
            if proc.poll() is not None:
                raise RuntimeError(
                    f"server exited {proc.returncode} while waiting for {expect!r}"
                )
            text = self.log_path.read_text(errors="replace")
            return text.count(expect) >= before + 1, text

        try:
            proc.stdin.write((command + "\n").encode())
            proc.stdin.flush()
        except (BrokenPipeError, OSError) as error:
            raise RuntimeError(f"console write failed for {command!r}: {error}") from error
        text = runtime.wait_for_path_condition(self.log_path, limit, reached)
        return [line for line in text.splitlines() if expect in line][-1]

    def command(self, text: str) -> str:
        """Return exact fresh console feedback, fenced by a unique own echo.

        The echo proves input ordering, NOT command success. Domain oracles
        must inspect actual query feedback or received client state.
        """
        from ..matrix_api import MissingCapability
        if "\n" in text or "\r" in text:
            raise ValueError("one console command per call")
        if self.dimension != "overworld" and text.split(" ", 1)[0] in {
            "fill", "setblock", "summon", "execute", "data", "forceload", "worldborder"
        }:
            text = f"execute in minecraft:{self.dimension} run {text}"
        proc = self._process
        if proc is None or proc.poll() is not None or proc.stdin is None:
            raise RuntimeError("server console unavailable")
        self._command_number += 1
        marker = f"MatrixConsoleFence{self._command_number:08}"
        before = len(self.log_path.read_text(errors="replace"))
        proc.stdin.write((text + "\nsay " + marker + "\n").encode())
        proc.stdin.flush()
        def reached():
            if proc.poll() is not None:
                raise RuntimeError(f"server exited {proc.returncode} during command")
            fresh = self.log_path.read_text(errors="replace")[before:]
            return marker in fresh, fresh
        fresh = runtime.wait_for_path_condition(self.log_path, self.command_timeout_seconds, reached)
        if any(token in fresh for token in ("not implemented", "Unknown command", "Unsupported execute")):
            raise MissingCapability("server_command:" + text.split(" ", 1)[0] + ":" + fresh)
        return fresh

    def query(self, text: str) -> str:
        return self.command(text)

    def clean_restart(self) -> dict[str, Any]:
        self.stop()
        self.start()
        return {"same_world_dir": str(self.world_dir), "same_binary": str(self.binary)}

    def mark_boundary(self) -> None:
        self._boundary_log_offset = len(self.log_path.read_text(errors="replace"))

    def wait_empty_pause(self) -> dict[str, Any]:
        def reached():
            fresh = self.log_path.read_text(errors="replace")[self._boundary_log_offset:]
            return PAUSE_LOG in fresh, fresh
        text = runtime.wait_for_path_condition(self.log_path, self.command_timeout_seconds, reached)
        before = self.dashboard_stats()
        self.save_all()
        after = self.dashboard_stats()
        if before["players"] or before["simulation_tick"] != after["simulation_tick"]:
            raise AssertionError(f"empty pause did not freeze world: {before}, {after}")
        return {"log": text, "tick": before["simulation_tick"], "after_save": after}

    def save_all(self, timeout: float | None = None) -> str:
        return self.console("save-all", SAVE_DONE, timeout)

    def stop(self, timeout: float | None = None) -> int:
        proc = self._process
        if proc is None or proc.poll() is not None:
            raise RuntimeError("server not running")
        limit = timeout if timeout is not None else self.command_timeout_seconds
        self.console("stop", STOP_LINE, limit)
        try:
            returncode = proc.wait(timeout=limit)
        except subprocess.TimeoutExpired as error:
            raise RuntimeError(f"server did not exit after console stop within {limit}s") from error
        finally:
            self._close_handles()
        if returncode != 0:
            raise RuntimeError(f"server did not stop cleanly: {returncode}")
        return returncode

    def _close_handles(self) -> None:
        proc, self._process = self._process, None
        stdin = getattr(proc, "stdin", None)
        closer = getattr(stdin, "close", None) if stdin is not None else None
        if callable(closer):
            try:
                closer()
            except (BrokenPipeError, OSError):
                pass
        if self._log_handle is not None:
            try:
                self._log_handle.close()
            finally:
                self._log_handle = None

    def close(self) -> None:
        try:
            runtime.stop_process(self._process)
        finally:
            self._close_handles()

    def __enter__(self) -> ProtocolServer:
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()

    def kill9_restart(self, timeout: float | None = None) -> dict[str, Any]:
        """Direct SIGKILL restart on the same world/config/binary (WAL proof).

        ``proc.kill()`` + bounded ``proc.wait`` must report -9: a graceful
        SIGTERM checkpoint would not prove crash recovery. The new server
        must reach the port fence. Returns pid + sameness evidence.
        """
        proc = self._process
        if proc is None or proc.poll() is not None:
            raise RuntimeError("server not running")
        limit = timeout if timeout is not None else self.startup_timeout_seconds
        old_pid = proc.pid
        proc.kill()
        try:
            killed = proc.wait(timeout=limit)
        except subprocess.TimeoutExpired as error:
            raise RuntimeError(f"killed server did not exit within {limit}s") from error
        if killed != -9:
            raise RuntimeError(f"SIGKILL proof failed: exit was {killed}, want -9")
        published_tick = max(
            (int(path.stem.split("-")[-1]) for path in
             (self.world_dir / "journal").glob("batch-*-*.bin")), default=0)
        checkpoints = re.findall(r"checkpoint committed tick=(\d+)",
                                 self.log_path.read_text(errors="replace"))
        published_tick = max([published_tick, *map(int, checkpoints)])
        self._close_handles()
        self.log_path.parent.mkdir(parents=True, exist_ok=True)
        self._log_handle = self.log_path.open("ab")
        try:
            self._process = runtime.start_server(
                self._argv,
                cwd=runtime.REPO_ROOT,
                stdin=subprocess.PIPE,
                stdout=self._log_handle,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            runtime.wait_port(self.port, limit, self._process, log_path=self.log_path)
        except BaseException:
            runtime.stop_process(self._process)
            self._process = None
            raise
        assert self._process is not None
        return {
            "old_pid": old_pid,
            "new_pid": self._process.pid,
            "old_exit": killed,
            "published_prefix_before_kill": published_tick,
            "same_world_dir": str(self.world_dir),
            "same_config": str(self.config_path),
            "same_binary": str(self.binary),
        }

    # -- backup / restore -----------------------------------------------
    def hot_backup(self, timeout: float | None = None) -> Path:
        """Console ``backup`` on the running world; fail-closed state proof.

        Waits for the exact ``Backup <path> complete: ...`` line (the command
        runs detached), then proves the published directory holds
        ``manifest.json`` — an interrupted backup never looks valid.
        """
        proc = self._process
        if proc is None or proc.poll() is not None:
            raise RuntimeError("server not running")
        limit = timeout if timeout is not None else self.command_timeout_seconds
        before_lines = len(self.log_path.read_text(errors="replace").splitlines())
        if proc.stdin is None:
            raise RuntimeError("server console is unavailable")
        try:
            proc.stdin.write(b"backup\n")
            proc.stdin.flush()
        except (BrokenPipeError, OSError) as error:
            raise RuntimeError(f"console write failed for 'backup': {error}") from error

        def reached() -> tuple[bool, str]:
            if proc.poll() is not None:
                raise RuntimeError(f"server exited {proc.returncode} during backup")
            return True, self.log_path.read_text(errors="replace")

        # Wait for any fresh backup outcome line (inotify-driven, no sleep).
        def settled() -> tuple[bool, str]:
            ok, text = reached()
            fresh = text.splitlines()[before_lines:]
            if any(BACKUP_DISABLED in line or BACKUP_FAILED in line for line in fresh):
                return True, text
            if any(BACKUP_DONE_RE.match(line.strip()) for line in fresh):
                return True, text
            return False, text

        text = runtime.wait_for_path_condition(self.log_path, limit, settled)
        fresh = text.splitlines()[before_lines:]
        negative = next(
            (line for line in fresh if BACKUP_DISABLED in line or BACKUP_FAILED in line),
            None,
        )
        if negative is not None:
            raise RuntimeError(f"hot backup refused: {negative}")
        done = next(
            (line.strip() for line in fresh if BACKUP_DONE_RE.match(line.strip())),
            None,
        )
        if done is None:  # pragma: no cover — fence guarantees one outcome line
            raise RuntimeError("hot backup produced no completion line")
        match = BACKUP_DONE_RE.match(done)
        assert match is not None
        backup = Path(match.group(1))
        manifest = backup / "manifest.json"
        if not backup.is_dir() or not manifest.is_file():
            raise RuntimeError(f"hot backup has no state proof at {manifest}")
        return backup

    def restore_proof(
        self, backup: Path, target: Path | None = None, timeout_seconds: float = 120.0
    ) -> dict[str, Any]:
        """Offline ``restore`` CLI proof: checksum-verified copy into an empty dir.

        Runs while the server may stay up (restore leases only backup+target).
        Proves exit 0, the ``N files verified`` fence, and
        ``target/native/state.redb`` on disk.
        """
        backup = Path(backup)
        if not backup.is_dir() or not (backup / "manifest.json").is_file():
            raise Blocked(f"not a valid backup dir (no manifest.json): {backup}")
        dest = Path(target) if target is not None else (self.run_dir / "restore-proof")
        if dest.exists() and any(dest.iterdir()):
            raise RuntimeError(f"restore target must be empty: {dest}")
        if not self.config_path.is_file():
            raise Blocked(f"missing server config: {self.config_path}")
        argv = [
            str(self.binary),
            "--config",
            str(self.config_path),
            "restore",
            "--from",
            str(backup),
            "--world-dir",
            str(dest),
        ]
        try:
            completed = subprocess.run(
                argv,
                cwd=runtime.REPO_ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                timeout=timeout_seconds,
                check=False,
            )
        except FileNotFoundError as error:
            raise Blocked(f"missing server binary: {self.binary}") from error
        output = completed.stdout.decode(errors="replace")
        if completed.returncode != 0:
            raise RuntimeError(f"restore CLI failed ({completed.returncode}): {output[-2000:]}")
        match = RESTORE_DONE_RE.search(output)
        if match is None:
            raise RuntimeError(f"restore produced no files-verified fence: {output[-2000:]}")
        state = dest / "native" / "state.redb"
        if not state.is_file():
            raise RuntimeError(f"restore wrote no state proof at {state}")
        return {
            "backup": str(backup),
            "target": str(dest),
            "files_verified": int(match.group(1)),
            "world_state": str(state),
            "argv": argv,
        }

    # -- stats / pause probe ---------------------------------------------
    def dashboard_full_stats(self) -> dict[str, Any]:
        if self.dashboard_port is None:
            raise Blocked("dashboard port unknown: start the server first")
        with urllib.request.urlopen(
            f"http://127.0.0.1:{self.dashboard_port}/stats", timeout=10
        ) as response:
            return json.loads(response.read())

    def dashboard_stats(self) -> dict[str, Any]:
        if self.dashboard_port is None:
            raise Blocked("dashboard port unknown: start the server first")
        body = urllib.request.urlopen(
            f"http://127.0.0.1:{self.dashboard_port}/stats", timeout=10
        ).read()
        data = json.loads(body)
        players = data.get("players") or {"count": 0}
        return {
            "simulation_tick": data["simulation_tick"],
            "players": players["count"],
            "uptime_secs": data["uptime_secs"],
        }

    def _wait_log_line(self, needle: str, occurrences: int, timeout: float) -> str:
        def reached() -> tuple[bool, str]:
            text = self.log_path.read_text(errors="replace")
            return text.count(needle) >= occurrences, text

        text = runtime.wait_for_path_condition(self.log_path, timeout, reached)
        return [line for line in text.splitlines() if needle in line][occurrences - 1]

    def pause_empty_probe(
        self, join_name: str = "RunProtoProbe", timeout: float | None = None
    ) -> dict[str, Any]:
        """Empty-pause probe: pause log, frozen tick across save, status,
        protocol join resume (LoginPlay + KeepAlive), repause. Fail-closed."""
        if self.pause_when_empty_seconds <= 0:
            raise Blocked("pause_empty_probe needs pause_when_empty_seconds > 0")
        limit = timeout if timeout is not None else self.command_timeout_seconds
        pause_log = self._wait_log_line(PAUSE_LOG, 1, limit)
        paused = self.dashboard_stats()
        save_line = self.save_all(limit)
        after_save = self.dashboard_stats()
        if paused["simulation_tick"] != after_save["simulation_tick"]:
            raise RuntimeError(f"tick not frozen while paused: {paused!r} vs {after_save!r}")
        if after_save["players"] != 0:
            raise RuntimeError(f"players online while paused: {after_save!r}")
        status = wire.probe_status(("127.0.0.1", self.port))
        if status["version"] != {"name": wire.RELEASE, "protocol": wire.PROTOCOL}:
            raise RuntimeError(f"wrong status version: {status['version']!r}")
        sock, joined = wire.join_offline_socket(("127.0.0.1", self.port), join_name, timeout=limit)
        try:
            wire.wait_play_loop(sock, timeout=limit)
            after_join = self.dashboard_stats()
        finally:
            sock.close()
        if after_join["simulation_tick"] <= paused["simulation_tick"]:
            raise RuntimeError(f"tick did not advance after join: {after_join!r} vs {paused!r}")
        if after_join["players"] != 1:
            raise RuntimeError(f"player not online after join: {after_join!r}")
        repause_log = self._wait_log_line(PAUSE_LOG, 2, limit)
        return {
            "pause_log": pause_log,
            "tick_while_paused": paused["simulation_tick"],
            "save_line": save_line,
            "status": status,
            "join": joined,
            "tick_after_join": after_join["simulation_tick"],
            "repause_log": repause_log,
        }
