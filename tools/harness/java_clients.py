#!/usr/bin/env python3
"""Parallel Java client group for the wave-1 MC26.1.2 matrix.

The matrix runner owns the server lifecycle and calls
:func:`client_group` to bring up ``count`` real Java clients against an
already-listening ``address``::

    from tools.harness import java_clients

    with java_clients.client_group(artifact_dir, 3, "127.0.0.1:25567") as clients:
        run_wave_one_scenario(clients)  # list[McpClient], all in Play

Contract the matrix runner relies on:

* 1-8 clients share one fresh ``runtime.start_xvfb`` display; each client
  gets a unique identity (``JMatrix00`` …), its own game dir
  (``<artifact>/client-NN/game`` with ``onboardAccessibility:false``),
  client log, MCP token and MCP port. No server is started here.
* ``client_profile`` selects the canonical runtime's true vanilla java-agent,
  Fabric Loader, SVC, or DH launcher. An explicit Loader platform is passed
  through; its permission screen is confirmed only for the exact connected
  server address. No fallback titles or standing grants.
* Client JVMs default to ``JDK_JAVA_OPTIONS=-Xmx512M`` during group startup.
  Explicit runtime JVM flags take precedence; no host-sized default heap is
  allowed for an eight-client group.
* Startup is bounded-parallel (``ThreadPoolExecutor`` with ``count``
  workers) and every wait is event-driven via ``runtime`` helpers
  (port/state inotify+pidfd, MCP push waits); nothing sleeps.
* Headless isolation applies while clients launch (``WAYLAND_DISPLAY``
  unset) and the previous environment is restored before the group yields.
  Audio isolation (OpenAL ``null``, no audio server) is forced per client by
  ``runtime.start_client`` and recorded in each ``game/java-audio-isolation.json``.
* ``authenticate=True`` registers every Play session through the live
  ``krasnokamen-auth`` plugin (``/register <pw> <pw>`` with a generated
  password, then fresh ``'registered'`` and ``'logged in'`` chat
  acknowledgements per the ``auth_live_check.py`` contract). Only for
  gameplay/capability fixtures whose server deploys ``krasnokamen-auth``;
  without it registration fails closed. Passwords never appear in
  ``group.json``, receipts or errors — use :func:`authenticate_client`
  directly when a custom password is required.
* On full readiness ``group.json`` is written into ``artifact_dir`` with
  usernames, MCP ports, profiles, per-client stages and the actual
  ``minecraft_wait_for_play`` responses. Any launch/readiness failure
  instead writes a failure ``group.json`` (``passed: false`` plus the
  error and per-client stages, still secret-free) so every lane attempt
  leaves exact prerequisite evidence. Either way every launched MCP
  session, client process, log handle and the shared Xvfb are cleaned up
  before the error propagates.
"""

from __future__ import annotations

import concurrent.futures
import contextlib
import json
import os
from collections import Counter
import secrets
import threading
import time
from pathlib import Path
from typing import Any, Iterator

from . import runtime
from .mcp import McpClient

MAX_CLIENTS = 8
_CLIENT_MAX_HEAP = "-Xmx512M"
_AUTH_WAIT_CAP_SECONDS = 90.0


def _client_options(platform: str | None) -> str:
    lines = [
        "version:4790",
        "onboardAccessibility:false",
        "guiScale:2",
        "renderDistance:4",
        "simulationDistance:5",
        "maxFps:30",
        "soundCategory_master:0.0",
        "soundCategory_music:0.0",
        "pauseOnLostFocus:false",
    ]
    return "\n".join(lines) + "\n"


def _confirm_exact_permission(client: Any, address: str, timeout_seconds: float) -> str | None:
    """Click Allow only for the exact connected server address.

    Returns the confirmed title, or None when the client reaches Play
    without showing the permission screen (e.g. cached grant). Never
    attempts any other title.
    """
    expected = f"Allow Krasnokamen content from {address}?"
    deadline = time.monotonic() + timeout_seconds
    last_error: Exception | None = None
    observed: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        try:
            client.call_tool(
                "minecraft_click_confirmation_button",
                {"expected_title": expected, "button_label": "Allow"},
            )
            return expected
        except Exception as error:
            last_error = error
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        if observed is None:
            observed = runtime.current_client_state(client, remaining)
        else:
            observed = runtime.next_client_state(client, observed, deadline)
        if observed.get("in_play"):
            return None
    raise RuntimeError(
        f"Loader permission prompt for {address!r} never confirmed "
        f"within {timeout_seconds:.1f}s: {last_error}"
    )


def _chat_lines(state: dict[str, Any]) -> list[str]:
    lines = state.get("recent_chat")
    if not isinstance(lines, list):
        return []
    return [line for line in lines if isinstance(line, str)]


def _wait_fresh_chat(client: Any, baseline: list[str], needle: str, deadline: float) -> str:
    """Return the first post-baseline chat line containing needle (case-insensitive).

    Event-driven: blocks on the client's state-change push, never on sleep.
    Only occurrences beyond the pre-send baseline count, so an old identical
    login message does not hide a new acknowledgement after reconnect.
    """
    seen = Counter(baseline)
    observed: dict[str, Any] | None = None
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError(f"missing fresh chat containing {needle!r}")
        if observed is None:
            observed = runtime.current_client_state(client, remaining)
        else:
            observed = runtime.next_client_state(client, observed, deadline)
        lines = _chat_lines(observed)
        counts = Counter(lines)
        for line in lines:
            if counts[line] > seen[line] and needle.lower() in line.lower():
                return line


def authenticate_client(
    client: Any, password: str, *, timeout_seconds: float = 60.0, register: bool = True
) -> dict[str, Any]:
    """Register a fresh account, or log an existing account back in.

    Both paths require a fresh logged-in acknowledgement. Registration also
    requires the fresh registered acknowledgement. Passwords never enter receipts.
    """
    if not isinstance(password, str) or not password:
        raise ValueError("password must be a non-empty string")
    encoded = password.encode("utf-8")
    if not 4 <= len(encoded) <= 128 or any(char.isspace() for char in password):
        raise ValueError("password must be 4..128 UTF-8 bytes without whitespace")
    if (
        isinstance(timeout_seconds, bool)
        or not isinstance(timeout_seconds, (int, float))
        or not timeout_seconds > 0
    ):
        raise ValueError(f"timeout_seconds must be positive, got {timeout_seconds!r}")
    timeout_seconds = float(timeout_seconds)
    deadline = time.monotonic() + timeout_seconds
    baseline = _chat_lines(runtime.current_client_state(client, min(timeout_seconds, 120.0)))
    try:
        client.call_tool(
            "minecraft_send_chat",
            {"message": f"register {password} {password}" if register else f"login {password}", "command": True},
        )
        if register:
            _wait_fresh_chat(client, baseline, "registered", deadline)
        _wait_fresh_chat(client, baseline, "logged in", deadline)
    except Exception as error:
        raise RuntimeError(f"authentication acknowledgement missing: {str(error).replace(password, '[REDACTED]')}") from None
    return {"authenticated": True}


def _record_payload(record: dict[str, Any]) -> dict[str, Any]:
    return {
        "index": record["index"],
        "username": record["username"],
        "game_dir": str(record["game_dir"]),
        "log": str(record["log_path"]),
        "mcp_port": record["mcp_port"],
        "platform": record["platform"],
        "client_profile": record["client_profile"],
        "stage": record["stage"],
        "permission_title": record["permission_title"],
        "play": record["play"],
        "authenticated": record["authenticated"],
    }


@contextlib.contextmanager
def client_group(
    artifact_dir: Path,
    count: int,
    address: str,
    platform: str | None = None,
    client_profile: str = "vanilla",
    timeout_seconds: float = 180.0,
    authenticate: bool = False,
    prepare_launch: bool = False,
) -> Iterator[list[McpClient]]:
    """Bring up ``count`` ready Java clients and yield their MCP sessions."""
    if isinstance(count, bool) or not isinstance(count, int) or not 1 <= count <= MAX_CLIENTS:
        raise ValueError(f"count must be an int in 1..{MAX_CLIENTS}, got {count!r}")
    if not isinstance(address, str) or not address:
        raise ValueError(f"address must be a non-empty server address, got {address!r}")
    if platform is not None and platform not in runtime.PLATFORMS:
        raise ValueError(f"platform must be one of {runtime.PLATFORMS}, got {platform!r}")
    if not isinstance(client_profile, str) or not client_profile:
        raise ValueError(f"client_profile must be a non-empty string, got {client_profile!r}")
    if (
        isinstance(timeout_seconds, bool)
        or not isinstance(timeout_seconds, (int, float))
        or not timeout_seconds > 0
    ):
        raise ValueError(f"timeout_seconds must be positive, got {timeout_seconds!r}")
    if not isinstance(authenticate, bool):
        raise ValueError(f"authenticate must be a bool, got {authenticate!r}")
    timeout_seconds = float(timeout_seconds)

    artifact_dir.mkdir(parents=True, exist_ok=True)
    launch_manifest = None
    if prepare_launch and (platform == "fabric" or client_profile in {"svc", "dh"}):
        from .java_launcher import prepare_fabric_launch
        launch_manifest = prepare_fabric_launch(artifact_dir, runtime.sibling_root("loader"), timeout_seconds)
    xvfb_process, display, xvfb_log = runtime.start_xvfb(artifact_dir)
    launched: list[dict[str, Any]] = []
    launched_lock = threading.Lock()
    request_timeout = max(30.0, timeout_seconds + 10.0)
    passwords = [secrets.token_urlsafe(12) for _ in range(count)] if authenticate else [None] * count

    def close_record(record: dict[str, Any]) -> None:
        client = record.get("client")
        if client is not None:
            try:
                client.close()
            except Exception:
                pass
        try:
            runtime.stop_process(record.get("process"))
        except Exception:
            pass
        handle = record.get("log_handle")
        if handle is not None:
            try:
                handle.close()
            except Exception:
                pass

    def close_all() -> None:
        with launched_lock:
            pending = list(launched)
        for record in pending:
            close_record(record)
        runtime.stop_process(xvfb_process)
        try:
            xvfb_log.close()
        except Exception:
            pass

    def write_receipt(records: list[dict[str, Any]], error: BaseException | None) -> None:
        payload: dict[str, Any] = {
            "address": address,
            "platform": platform,
            "client_profile": client_profile,
            "display": display,
            "count": count,
            "authenticate": authenticate,
            "passed": error is None,
            "clients": [
                _record_payload(record)
                for record in sorted(records, key=lambda item: item["index"])
            ],
        }
        if error is not None:
            message = str(error)
            for record in records:
                message = message.replace(record["token"], "[REDACTED]")
            for password in passwords:
                if password:
                    message = message.replace(password, "[REDACTED]")
            payload["error"] = message
        (artifact_dir / "group.json").write_text(
            json.dumps(payload, indent=2, sort_keys=True) + "\n"
        )

    def launch_one(index: int) -> dict[str, Any]:
        username = f"JMatrix{index:02d}"
        client_dir = artifact_dir / f"client-{index:02d}"
        game_dir = client_dir / "game"
        game_dir.mkdir(parents=True, exist_ok=True)
        options = game_dir / "options.txt"
        if not options.exists():
            options.write_text(_client_options(platform))
        token = secrets.token_urlsafe(32)
        mcp_port = runtime.reserve_port()
        log_path = client_dir / "client.log"
        start_kwargs: dict[str, Any] = {
            "game_dir": game_dir,
            "log_path": log_path,
            "token": token,
            "mcp_port": mcp_port,
            "username": username,
            "display": display,
            "client_profile": client_profile,
        }
        if launch_manifest is not None:
            start_kwargs["launch_manifest"] = launch_manifest
        if platform is not None:
            start_kwargs["platform"] = platform
        record: dict[str, Any] = {
            "index": index,
            "username": username,
            "game_dir": game_dir,
            "log_path": log_path,
            "mcp_port": mcp_port,
            "token": token,
            "platform": platform,
            "client_profile": client_profile,
            "process": None,
            "log_handle": None,
            "client": None,
            "stage": "starting",
            "permission_title": None,
            "play": None,
            "authenticated": False,
        }
        process, log_handle = runtime.start_client(**start_kwargs)
        record["process"] = process
        record["log_handle"] = log_handle
        record["stage"] = "launched"
        with launched_lock:
            launched.append(record)
        # Register the session before initialize so an init failure still
        # closes it during cleanup.
        record["client"] = McpClient(f"http://127.0.0.1:{mcp_port}/mcp", token, request_timeout)
        client = record["client"]
        client.initialize()
        record["stage"] = "initialized"
        runtime.wait_client_ready_for_connect(client, timeout_seconds)
        record["stage"] = "ready"
        client.call_tool("minecraft_connect", {"server_addr": address})
        record["stage"] = "connected"
        if platform is not None:
            record["permission_title"] = _confirm_exact_permission(
                client, address, min(timeout_seconds, 90.0)
            )
            record["stage"] = "permission-confirmed"
        play = client.call_tool("minecraft_wait_for_play", {"timeout_seconds": timeout_seconds})
        if not play.get("in_play"):
            raise RuntimeError(f"{username} did not enter Play: {play}")
        record["play"] = play
        record["stage"] = "in-play"
        if authenticate:
            try:
                authenticate_client(
                    client,
                    passwords[index] or "",
                    timeout_seconds=min(timeout_seconds, _AUTH_WAIT_CAP_SECONDS),
                )
            except Exception as error:
                raise RuntimeError(f"{username} authentication failed: {error}") from error
            record["authenticated"] = True
            record["stage"] = "authenticated"
            client._matrix_auth_password = passwords[index]
        client._matrix_platform = platform
        return record

    _sentinel = object()
    saved_env: dict[str, Any] = {
        name: os.environ.get(name, _sentinel) for name in ("WAYLAND_DISPLAY", "JDK_JAVA_OPTIONS")
    }
    os.environ.pop("WAYLAND_DISPLAY", None)
    # N8 heap safety: cap each vanilla/Loader client JVM; an explicit
    # operator value is respected instead.
    heap_overrode = "JDK_JAVA_OPTIONS" not in os.environ
    if heap_overrode:
        os.environ["JDK_JAVA_OPTIONS"] = _CLIENT_MAX_HEAP
    wrote_receipt = False
    try:
        try:
            with concurrent.futures.ThreadPoolExecutor(max_workers=count) as executor:
                futures = [executor.submit(launch_one, index) for index in range(count)]
                try:
                    records = [future.result() for future in futures]
                except BaseException:
                    for future in futures:
                        future.cancel()
                    raise
        finally:
            for name, saved in saved_env.items():
                if saved is _sentinel:
                    os.environ.pop(name, None)
                else:
                    os.environ[name] = saved  # type: ignore[assignment]
        records.sort(key=lambda record: record["index"])
        write_receipt(records, None)
        wrote_receipt = True
        yield [record["client"] for record in records]
    except BaseException as error:
        if not wrote_receipt:
            with launched_lock:
                pending = list(launched)
            try:
                write_receipt(pending, error)
            except Exception:
                pass
        raise
    finally:
        close_all()
