"""Java matrix profile primitives; canonical cmd_run owns the run receipt.

Shares ProtocolServer and matrix_execution with the protocol/native drivers.
Only trusted Python domain adapters define actions and assertions. A missing
adapter, query, client verb or actual distribution is blocked, never smoke-passed.
"""
from __future__ import annotations

import argparse
from collections import Counter
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
from typing import Any

from ..java_clients import client_group
from ..java_matrix_client import JavaMatrixClient
from ..matrix_api import MatrixContext, MissingCapability, load_adapter
from ..matrix_execution import copy_plugins, plugin_set_fingerprint, prepare_access, run_subject, select_plugin_profile
from .protocol_lifecycle import Blocked, ProtocolServer, reserve_matrix_port

JAVA_KINDS = frozenset({"vanilla_java_26_1_2", "fabric_solaris_loader", "fabric_svc",
                        "fabric_distant_horizons", "neoforge_solaris_loader", "forge_solaris_loader"})
JAVA_CLIENT_LAUNCH = {"vanilla_java_26_1_2": (None, "vanilla"),
            "fabric_solaris_loader": ("fabric", "loader"),
            "fabric_svc": (None, "svc"), "fabric_distant_horizons": (None, "dh"),
            "neoforge_solaris_loader": ("neoforge", "loader"),
            "forge_solaris_loader": ("forge", "loader")}
PORT_RANGE = (26200, 26299)


def blockers(row: dict[str, Any]) -> list[str]:
    try:
        load_adapter(row["subject"])
    except MissingCapability as error:
        return [str(error)]
    return []


def append_ledger(path: Path, row: dict[str, Any], status: str, server_sha: str,
                  run_dir: Path, note: str, plugin_set: str | None = None) -> None:
    if status not in {"pass", "fail", "blocked", "skipped"}:
        raise ValueError(f"invalid status {status}")
    entry = dict(id=row["id"], status=status, driver=row["driver"]["kind"],
                 server_sha=server_sha, run_dir=str(run_dir.resolve()),
                 ts=datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"), note=note)
    if plugin_set is not None:
        entry["plugin_set"] = plugin_set
    data = (json.dumps(entry, sort_keys=True) + "\n").encode()
    path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        if os.write(fd, data) != len(data):
            raise OSError("short ledger append")
        os.fsync(fd)
    finally:
        fcntl.flock(fd, fcntl.LOCK_UN)
        os.close(fd)


def fingerprint(path: Path) -> str:
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def receipt_dir(root: Path, scenario_id: str) -> Path:
    directory = root / (scenario_id.split(":", 1)[0] + "-" + hashlib.sha256(scenario_id.encode()).hexdigest()[:20])
    directory.mkdir(parents=True, exist_ok=True)
    return directory


@contextmanager
def server_instance(args: argparse.Namespace, directory: Path, count: int,
                    permission: str = "non_op_0", lifecycle: str = "steady",
                    excluded_plugins: set[str] | None = None):
    """Reuse the shared release-server primitive on this driver's own 262xx ports.

    Ports are bind-probed per instance, never hardcoded: concurrent harness
    runs on the shared host would otherwise collide and the loser's clients
    would join the winner's server.
    """
    directory.mkdir(parents=True, exist_ok=True)
    plugin_dir = directory / "plugins"
    enabled_plugins = copy_plugins(args.plugins_dir, plugin_dir, excluded_plugins or set())
    prepare_access(directory, [f"JMatrix{index:02d}" for index in range(count)], permission)
    port = reserve_matrix_port(port_range=PORT_RANGE)
    server = ProtocolServer(binary=args.server_bin, run_dir=directory, port=port,
                            dashboard_port=reserve_matrix_port(exclude=port, port_range=PORT_RANGE),
                            port_range=PORT_RANGE,
                            vanilla_data=args.data_dir, seed=81, plugins_dir=plugin_dir,
                            server_name="RunJava", pause_when_empty_seconds=1 if lifecycle == "pause_when_empty_resume" else 0,
                            startup_timeout_seconds=args.timeout_seconds)
    server.enabled_plugins = enabled_plugins
    index = len(args.context.commands)
    args.context.commands.append([str(args.server_bin), "--config", str(server.config_path)])
    args.context.cwds.append(str(Path(__file__).resolve().parents[3]))
    args.context.exits.append(None)
    args.context.logs.append(str(server.log_path))
    try:
        server.start()
        yield server
    finally:
        process = server._process
        if server.running:
            server.stop()
        args.context.exits[index] = process.returncode if process is not None else None


def group(args: argparse.Namespace, directory: Path, count: int, server: ProtocolServer, kind: str):
    platform, profile = JAVA_CLIENT_LAUNCH[kind]
    return client_group(directory / "clients", count, f"127.0.0.1:{server.port}",
                        platform=platform, client_profile=profile,
                        timeout_seconds=args.timeout_seconds,
                        authenticate="solaris-auth" in server.enabled_plugins, prepare_launch=True)


def run_capabilities(args: argparse.Namespace, root: Path) -> dict[str, Any]:
    results = {}
    for kind in sorted(args.client_kind or JAVA_KINDS):
        directory = root / ("capability-" + kind)
        directory.mkdir(parents=True, exist_ok=True)
        result = dict(client_kind=kind, requested_clients=args.clients, passed=False,
                      server_sha=fingerprint(args.server_bin), status="blocked")
        try:
            with server_instance(args, directory, args.clients) as server:
                with group(args, directory, args.clients, server, kind) as clients:
                    states = [client.call_tool("minecraft_observe") for client in clients]
                    if not all(state.get("in_play") is True for state in states):
                        raise AssertionError("not all N clients remained in Play")
                    result.update(passed=True, status="pass", simultaneous_in_play=len(states), observations=states)
        except (MissingCapability, Blocked, FileNotFoundError) as error:
            result["error"] = f"{type(error).__name__}: {error}"
        except Exception as error:
            result.update(status="fail", error=f"{type(error).__name__}: {error}")
        (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        results[kind] = result
    return results


def run_cell(args: argparse.Namespace, row: dict[str, Any], directory: Path) -> dict[str, Any]:
    count = int(row["context"]["player_count"].removeprefix("N"))
    factors = row["context"]
    result: dict[str, Any] = {"id": row["id"], "status": "blocked", "checks": []}
    try:
        adapter = load_adapter(row["subject"])
        excluded, expected = select_plugin_profile(
            row, args.plugin_profiles, adapter_exclusions=adapter.exclude_plugins,
            requested_exclusions=getattr(args, "exclude_plugin", []))
        with server_instance(args, directory / "server", count, factors["permission"],
                             factors["server_lifecycle"], excluded_plugins=excluded) as server:
            actual = plugin_set_fingerprint(server.plugins_dir)
            if expected is not None and actual != expected:
                raise MissingCapability(f"plugin_set_mismatch:expected {expected}; actual {actual}")
            result["plugin_set"] = actual
            with group(args, directory, count, server, factors["client_kind"]) as sessions:
                clients = [JavaMatrixClient(f"JMatrix{index:02d}", session,
                                            f"127.0.0.1:{server.port}", directory,
                                            fixture_command=server.command,
                                            timeout_seconds=args.timeout_seconds)
                           for index, session in enumerate(sessions)]
                server.ensure_loaded_callback = clients[0].ensure_loaded
                result["enabled_plugins"] = server.enabled_plugins
                context = MatrixContext(row, server, clients, directory,
                                        state={"enabled_plugins": server.enabled_plugins})
                result["checks"] = run_subject(context)
                result["status"] = "pass" if all(check["passed"] for check in result["checks"]) else "fail"
                result["note"] = "all exact domain checks passed" if result["status"] == "pass" else "domain oracle assertion failed"
    except (MissingCapability, Blocked, FileNotFoundError) as error:
        result["note"] = f"{type(error).__name__}: {error}"
    except Exception as error:
        result.update(status="fail", note=f"{type(error).__name__}: {error}")
    return result


def run(ctx: Any) -> dict[str, Any]:
    import yaml
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--matrix", type=Path, required=True)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--data-dir", type=Path, required=True)
    parser.add_argument("--clients", type=int, default=8)
    parser.add_argument("--capabilities", action="store_true")
    parser.add_argument("--id", action="append", help="select exact stable id; default all Java wave-1 cells")
    parser.add_argument("--exclude-plugin", action="append", default=[])
    parser.add_argument("--client-kind", action="append", choices=sorted(JAVA_KINDS))
    args = parser.parse_args(ctx.extra_args)
    if ctx.server_bin is None or ctx.plugins_dir is None:
        raise ValueError("java-matrix requires --server-bin and --plugins-dir")
    if ctx.client != "java" or ctx.platform is not None:
        raise ValueError("java-matrix selects Java platforms from the matrix")
    args.server_bin, args.plugins_dir, args.run_dir = ctx.server_bin, ctx.plugins_dir, ctx.artifact_dir
    args.timeout_seconds = ctx.timeout_seconds or 180
    args.context = ctx
    for key in ("matrix", "ledger", "run_dir", "server_bin", "plugins_dir", "data_dir"):
        setattr(args, key, getattr(args, key).resolve())
    if not 1 <= args.clients <= 8 or args.timeout_seconds <= 0:
        raise ValueError("java-matrix needs 1..8 clients and positive timeout")
    if not os.access(args.server_bin, os.X_OK):
        raise ValueError("--server-bin must be executable")
    args.run_dir.mkdir(parents=True, exist_ok=True)
    matrix = yaml.safe_load(args.matrix.read_text())
    args.plugin_profiles = matrix.get("ledger_contract", {}).get("plugin_profiles", {})
    rows = [row for row in matrix["scenarios"] if row["context"]["client_kind"] in (args.client_kind or JAVA_KINDS)
            and (not args.id or row["id"] in args.id)]
    if args.id and set(args.id) != {row["id"] for row in rows}:
        raise ValueError("unknown or non-Java --id")
    sha = fingerprint(args.server_bin)
    id_set_sha = hashlib.sha256("\n".join(sorted(row["id"] for row in matrix["scenarios"])).encode()).hexdigest()
    (args.run_dir / "invocation.json").write_text(json.dumps({
        "argv": os.sys.argv, "server_sha": sha, "matrix_sha": fingerprint(args.matrix),
        "selected": len(rows), "wave1_id_set_sha256": id_set_sha}, indent=2) + "\n")
    capabilities = run_capabilities(args, args.run_dir) if args.capabilities else {}
    counts: Counter[str] = Counter()
    findings = []
    for row in rows:
        directory = receipt_dir(args.run_dir, row["id"])
        (directory / "matrix-cell.json").write_text(json.dumps(row, indent=2) + "\n")
        reasons = blockers(row)
        excluded, expected = select_plugin_profile(
            row, args.plugin_profiles, requested_exclusions=args.exclude_plugin)
        profile = capabilities.get(row["context"]["client_kind"])
        if profile and not profile["passed"]:
            reasons.insert(0, f"client_profile:{row['context']['client_kind']}: {profile['error']}")
        if expected is not None and reasons:
            copy_plugins(args.plugins_dir, directory / "plugins", excluded)
            actual = plugin_set_fingerprint(directory / "plugins")
            if actual != expected:
                raise MissingCapability(f"plugin_set_mismatch:expected {expected}; actual {actual}")
        result = {"id": row["id"], "status": "blocked", "checks": [], "note": "; ".join(reasons)} if reasons else run_cell(args, row, directory)
        result.update(server_sha=sha, harness_receipt=str(args.run_dir / "result.json"))
        if expected is not None:
            result.setdefault("plugin_set", expected)
        # Ledger is derived from this retained cell receipt, never a smoke result.
        (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        append_ledger(args.ledger, row, result["status"], sha, directory,
                      result["note"] + f"; evidence {directory / 'result.json'}",
                      plugin_set=result.get("plugin_set"))
        counts[result["status"]] += 1
        if result["status"] == "fail":
            findings.append(dict(id=row["id"], note=result["note"], evidence=str(directory)))
    summary = dict(verdict="blocked" if counts["blocked"] else "changes" if counts["fail"] else "pass",
                   selected=len(rows), executed=counts["pass"] + counts["fail"],
                   wave1_id_set_sha256=id_set_sha,
                   **{key: counts[key] for key in ("pass", "fail", "blocked")},
                   new_capabilities=["multi_client_N lifecycle", "verified client profiles", "shared MatrixClient verbs"],
                   capability_results=capabilities, top_bugs=findings)
    (args.run_dir / "summary.yaml").write_text(yaml.safe_dump(summary, sort_keys=False))
    bugs = ["# Java matrix findings", "", "Blocked cells are not passes.", ""]
    bugs += [f"## P2 {item['id']}\n\n{item['note']}\n\nEvidence: {item['evidence']}\n\nRepro: repeat invocation.json with --id {json.dumps(item['id'])}\n" for item in findings]
    if not findings:
        bugs.append("No executed product assertion failed. See capability receipts and cell results for blockers.")
    (args.run_dir / "bugs.md").write_text("\n".join(bugs) + "\n")
    ctx.client_evidence["java_matrix"] = summary
    if counts["blocked"] or counts["fail"] or any(not result["passed"] for result in capabilities.values()):
        raise RuntimeError(f"Java matrix: {counts['fail']} failed, {counts['blocked']} blocked; see {args.run_dir / 'summary.yaml'}")
    return summary
