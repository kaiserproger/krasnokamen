#!/usr/bin/env python3
"""Shared process lifecycle and real-client readiness for the harness.

Every wait here is push-driven: port readiness is observed with
non-blocking connects plus selector waits, process exit with pidfd, and
log/file readiness with inotify or -displayfd. Nothing declares success
merely because a timeout elapsed or time passed.
"""

from __future__ import annotations

import contextlib
import ctypes
import errno
import functools
import hashlib
import json
import os
import re
import select
import selectors
import shutil
import signal
import socket
import subprocess
import time
from pathlib import Path
from typing import Any
from .matrix_api import MissingCapability

REPO_ROOT = Path(__file__).resolve().parents[2]
# The sibling repositories beside the core checkout, and the variable that names another checkout of each.
SIBLING_NAMES = {"client": "solaris-client", "loader": "solaris-loader", "plugins": "solaris-default-plugins"}
SIBLING_ENV = {"client": "KRASNOKAMEN_CLIENT_ROOT", "loader": "KRASNOKAMEN_LOADER_ROOT",
               "plugins": "KRASNOKAMEN_DEFAULT_PLUGINS_ROOT"}


@functools.lru_cache(maxsize=None)
def main_checkout(root: Path = REPO_ROOT) -> Path:
    """The main working tree of the repository holding ``root``.

    In the main checkout that is ``root`` itself. In a linked worktree (`git worktree add`, e.g.
    `<work>/wt/<Name>`) it is the checkout the worktree belongs to: the parent of the shared
    `--git-common-dir`. Without git or outside a repository it is ``root``."""
    try:
        completed = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "--path-format=absolute", "--git-common-dir"],
            capture_output=True, text=True, timeout=10, check=False)
    except (OSError, subprocess.SubprocessError):
        return root
    common = completed.stdout.strip()
    return Path(common).parent if completed.returncode == 0 and common else root


def sibling_root(kind: str, root: Path = REPO_ROOT) -> Path:
    """The checkout of sibling repository ``kind`` (a SIBLING_NAMES key).

    Its SIBLING_ENV variable wins; otherwise it is the sibling beside the main checkout, so a
    linked worktree finds the same `<work>/solaris-loader` as the main checkout instead of a
    nonexistent `<work>/wt/solaris-loader`."""
    override = os.environ.get(SIBLING_ENV[kind])
    if override:
        return Path(override).expanduser().resolve()
    return main_checkout(root).parent / SIBLING_NAMES[kind]


USERNAME_RE = re.compile(r"[A-Za-z0-9_]{1,16}\Z")
CLIENT_PROFILES = ("vanilla", "loader", "svc", "dh")
PLATFORMS = ("fabric", "neoforge", "forge")

_CLIENT_MAX_HEAP = "-Xmx512M"

_PROFILE_MOD_ARTIFACTS = {"svc": "svc-artifacts.json", "dh": "dh-artifacts.json"}


def _mod_artifact_path(profile: str) -> Path:
    return Path(__file__).resolve().parent / _PROFILE_MOD_ARTIFACTS[profile]


def _default_mod_jar_dir(profile: str) -> Path:
    stem = {"svc": "SVCCompat", "dh": "DHCompat"}[profile]
    return REPO_ROOT / ".analysis" / stem / "artifacts"


def _mod_env_override(profile: str) -> str | None:
    override = os.environ.get({"svc": "KRASNOKAMEN_SVC_JAR", "dh": "KRASNOKAMEN_DH_JAR"}[profile])
    return override or None


def _hash_file(path: Path, algorithm: str) -> str:
    digest = hashlib.new(algorithm)
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _select_mod_entry(profile: str, platform: str) -> dict[str, Any]:
    """Return the pinned publisher artifact for this MC 26.1.2 profile/platform cell."""
    payload = json.loads(_mod_artifact_path(profile).read_text())
    if profile == "dh":
        entries = [item for item in payload["artifacts"] if item["minecraft"] == "26.1.2"]
    else:
        entries = [
            item for item in payload["artifacts"]
            if "26.1.2" in item["game_versions"] and platform in item["loaders"]
        ]
    compatible = [item for item in entries if platform in item["loaders"]]
    if not compatible:
        raise MissingCapability(
            f"no published {profile} artifact covers MC 26.1.2 on {platform}; "
            "the profile is blocked until the publisher ships one"
        )
    if profile == "dh":
        baseline = payload.get("baseline", {})
        for item in compatible:
            if item.get("distant_horizons") == baseline.get("distant_horizons"):
                return dict(item)
    return dict(sorted(compatible, key=lambda item: item.get("date_published", ""))[-1])


def _profile_jar_path(profile: str, entry: dict[str, Any]) -> Path:
    override = _mod_env_override(profile)
    if override:
        return Path(override).expanduser()
    return _default_mod_jar_dir(profile) / entry["filename"]


def _verify_mod_jar(profile: str, entry: dict[str, Any], jar: Path) -> None:
    """Fail closed unless the staged jar matches every publisher-pinned field."""
    if not jar.is_file():
        raise MissingCapability(
            f"{profile} jar missing at {jar} (fetch {entry['url']} and verify "
            "size/sha512/sha1 before use; no download happens inside this call)"
        )
    actual_size = jar.stat().st_size
    if actual_size != entry["size"]:
        raise MissingCapability(
            f"{profile} jar size mismatch at {jar}: {actual_size} != published {entry['size']}; "
            "re-fetch, do not trust"
        )
    for algorithm in ("sha512", "sha1"):
        actual = _hash_file(jar, algorithm)
        if actual != entry[algorithm]:
            raise MissingCapability(
                f"{profile} jar {algorithm} mismatch at {jar}: {actual} != published {entry[algorithm]}; "
                "re-fetch, do not trust"
            )


def _stage_profile_mods(profile: str, platform: str, game_dir: Path) -> list[str]:
    """Verify the pinned jar and stage it into this run's isolated game_dir/mods.

    Returns the staged filenames. Presence of a jar is never treated as
    working voice/DH support; the mod still has to negotiate at runtime.
    """
    entry = _select_mod_entry(profile, platform)
    jar = _profile_jar_path(profile, entry)
    _verify_mod_jar(profile, entry, jar)
    mods_dir = game_dir / "mods"
    mods_dir.mkdir(parents=True, exist_ok=True)
    unexpected = [path.name for path in mods_dir.glob("*.jar") if path.name != entry["filename"]]
    if unexpected:
        raise MissingCapability(f"{profile}:isolated_mods_directory:unexpected {sorted(unexpected)}")
    staged = mods_dir / entry["filename"]
    if not staged.is_file() or _hash_file(staged, "sha512") != entry["sha512"]:
        shutil.copy2(jar, staged)
    _verify_mod_jar(profile, entry, staged)
    marker = game_dir / "krasnokamen-profile.json"
    marker.write_text(json.dumps({
        "client_profile": profile,
        "platform": platform,
        "composition": _resolve_profile_task(profile, platform)["composition"],
        "task": _resolve_profile_task(profile, platform)["task"],
        "mod": {key: entry[key] for key in ("filename", "url", "size", "sha512", "sha1") if key in entry},
    }, indent=2, sort_keys=True) + "\n")
    return [staged.name]


def profile_prerequisites(profile: str, platform: str | None) -> dict[str, Any]:
    """Describe the pinned mod jar for a profile without launching anything.

    Never downloads. ``present`` is False with a fetch hint when the jar is
    absent; hash fields report mismatch details when it is staged wrong.
    """
    if profile not in CLIENT_PROFILES:
        raise ValueError(f"client_profile must be one of {CLIENT_PROFILES}, got {profile!r}")
    resolved = _resolve_profile_task(profile, platform)
    info: dict[str, Any] = {"client_profile": profile, "platform": resolved["platform"],
                             "task": resolved["task"], "label": resolved["label"],
                             "composition": resolved["composition"]}
    if profile in ("svc", "dh"):
        try:
            entry = _select_mod_entry(profile, resolved["platform"])
        except RuntimeError as error:
            return {**info, "jar": {"present": False, "detail": str(error)}}
        jar = _profile_jar_path(profile, entry)
        record: dict[str, Any] = {
            "filename": entry["filename"], "url": entry["url"], "size": entry["size"],
            "sha512": entry["sha512"], "sha1": entry["sha1"], "local_path": str(jar),
            "present": jar.is_file(),
        }
        if jar.is_file():
            record["size_ok"] = jar.stat().st_size == entry["size"]
            record["sha512_ok"] = _hash_file(jar, "sha512") == entry["sha512"]
            record["sha1_ok"] = _hash_file(jar, "sha1") == entry["sha1"]
            if not all((record["size_ok"], record["sha512_ok"], record["sha1_ok"])):
                record["detail"] = (
                    f"size/hash mismatch at {jar}; re-fetch {entry['url']} and verify "
                    "size/sha512/sha1 before use"
                )
        else:
            record["detail"] = (
                f"not downloaded; fetch {entry['url']} and verify size/sha512/sha1 before use. "
                "No download happens inside the harness."
            )
        info["jar"] = record
    return info


def _resolve_profile_task(profile: str | None, platform: str | None) -> dict[str, Any]:
    """Map (client_profile, platform) to the one real launch path.

    ``None`` (legacy default) preserves the historic no-Loader Fabric agent
    task ``:fabric-agent:runClientMcp`` — the ``fabric-agent`` module is a
    NeoForge mod despite its directory name, so ``None`` must never be
    described as vanilla Minecraft. Explicit ``vanilla`` runs the true
    plain-Mojang 26.1.2 client via ``java_launcher.vanilla_command``
    (unmodified Mojang main + ``-javaagent``, no modloader). ``loader``
    requires an explicit platform and uses the matching Loader task.
    ``svc``/``dh`` reuse the real ``:loader-fabric:runClientMcp`` launch with
    the pinned publisher jar staged into the run's mods dir — that lane also
    carries the Krasnokamen Loader, so receipts must record the full composition
    (``krasnokamen-loader`` + the mod) and never claim a Loader-free mod-only
    client. Exact voice/DH behavior additionally needs runtime negotiation,
    not just jar presence.
    """
    if profile is None:
        if platform is not None:
            return _resolve_profile_task("loader", platform)
        return {"platform": None, "task": ":fabric-agent:runClientMcp",
                "label": "legacy no-Loader Fabric agent task (NeoForge mod; not vanilla Minecraft)",
                "composition": ["krasnokamen-agent-neoforge-mod"]}
    if profile == "vanilla":
        if platform is not None:
            raise ValueError("vanilla client_profile takes no platform; pass platform=None")
        return {"platform": None, "task": "direct-java:net.minecraft.client.main.Main+javaagent",
                "label": "true plain-Mojang 26.1.2 client + java-agent (no modloader)",
                "composition": ["plain-mojang-26.1.2", "java-agent"]}
    if profile == "loader":
        if platform is None:
            raise ValueError("loader client_profile requires platform fabric|neoforge|forge")
        return {"platform": platform, "task": f":loader-{platform}:runClientMcp",
                "label": f"Loader {platform}",
                "composition": [f"krasnokamen-loader-{platform}"]}
    if profile in ("svc", "dh"):
        if platform is not None and platform != "fabric":
            raise ValueError(f"{profile} client_profile is Fabric-only; got platform={platform!r}")
        return {"platform": "fabric", "task": ":loader-fabric:runClientMcp",
                "label": f"Krasnokamen Loader (Fabric) + {profile}",
                "composition": ["krasnokamen-loader-fabric", profile]}
    raise ValueError(f"client_profile must be one of {CLIENT_PROFILES}, got {profile!r}")


_CLIENT_READY_TIMEOUT_SECONDS = 180.0
_XVFB_READY_TIMEOUT_SECONDS = 20.0
# Owner rule (real-client gates, isolation): agent clients never use :0 or :1.
_OWNER_DISPLAYS = frozenset({0, 1})
_LOG_TAIL_BYTES = 8192

_LIBC = ctypes.CDLL(None, use_errno=True)


def _pidfd_open(pid: int) -> int:
    """Return a pidfd for pid, or -1 when the platform lacks pidfd_open."""
    try:
        pidfd_open = _LIBC.pidfd_open
    except AttributeError:
        return -1
    pidfd_open.argtypes = [ctypes.c_int, ctypes.c_uint]
    pidfd_open.restype = ctypes.c_int
    fd = pidfd_open(pid, 0)
    if fd < 0:
        return -1
    return int(fd)


def _inotify_for(parent: Path) -> int:
    """Return an inotify fd watching parent, or -1 when unavailable."""
    try:
        init = _LIBC.inotify_init1
        add = _LIBC.inotify_add_watch
    except AttributeError:
        return -1
    init.argtypes = [ctypes.c_int]
    init.restype = ctypes.c_int
    add.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_uint32]
    add.restype = ctypes.c_int
    fd = init(os.O_CLOEXEC | os.O_NONBLOCK)
    if fd < 0:
        return -1
    # IN_MODIFY | IN_ATTRIB | IN_CLOSE_WRITE | IN_CREATE | IN_MOVED_TO | IN_DELETE_SELF.
    watch = add(int(fd), os.fsencode(parent), 0x2 | 0x4 | 0x8 | 0x100 | 0x80 | 0x400)
    if watch < 0:
        os.close(int(fd))
        return -1
    return int(fd)


def _drain(fd: int) -> None:
    try:
        while os.read(fd, 65536):
            pass
    except (BlockingIOError, OSError):
        pass


def _tail_log(log_path: Path, max_bytes: int = _LOG_TAIL_BYTES) -> str:
    try:
        with log_path.open("rb") as handle:
            handle.seek(0, os.SEEK_END)
            size = handle.tell()
            handle.seek(max(0, size - max_bytes))
            return handle.read().decode("utf-8", "replace")
    except OSError:
        return ""


def ensure_loader_gradle_home(loader_root: Path | None = None) -> Path:
    """The Gradle home Loader gates use. When GRADLE_USER_HOME is unset and the owner host's
    `<work>/loader-gradle-home` (beside the Loader checkout; the one `ledger probe` uses)
    exists, export it so the schema compiler and the Gradle client launch share one cache."""
    if not os.environ.get("GRADLE_USER_HOME"):
        shared = (loader_root or sibling_root("loader")).parent / "loader-gradle-home"
        if shared.is_dir():
            os.environ["GRADLE_USER_HOME"] = str(shared)
    return Path(os.environ.get("GRADLE_USER_HOME", Path.home() / ".gradle"))


def reserve_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def port_open(port: int) -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(2.0)
        try:
            sock.connect(("127.0.0.1", port))
        except OSError:
            return False
        return True


def _probe_port(port: int, timeout_seconds: float) -> bool:
    """Non-blocking connect; True only when the port accepts.

    The wait for writability is the readiness event itself, not a sleep.
    """
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        sock.setblocking(False)
        error = sock.connect_ex(("127.0.0.1", port))
        if error not in (0, errno.EINPROGRESS, errno.EALREADY):
            return False
        _, writable, _ = select.select([], [sock], [], max(0.0, timeout_seconds))
        if not writable:
            return False
        return sock.getsockopt(socket.SOL_SOCKET, socket.SO_ERROR) == 0
    except OSError:
        return False
    finally:
        sock.close()


def _listener_inodes(port: int) -> set[int]:
    """Socket inodes of the LISTEN sockets on 127.0.0.1:<port> (procfs)."""
    inodes: set[int] = set()
    for table in ("/proc/net/tcp", "/proc/net/tcp6"):
        try:
            rows = Path(table).read_text().splitlines()[1:]
        except OSError:
            continue
        for row in rows:
            fields = row.split()
            if len(fields) < 10 or fields[3] != "0A":  # 0A = LISTEN
                continue
            if int(fields[1].rsplit(":", 1)[1], 16) == port:
                inodes.add(int(fields[9]))
    return inodes


def _descendant_pids(root_pid: int) -> set[int]:
    """`root_pid` plus all its descendants, via /proc/<pid>/task/<tid>/children.

    Loader clients are forked by Gradle's JavaExec, so the MCP listener can
    belong to a grandchild of the spawned process.
    """
    seen = {root_pid}
    pending = [root_pid]
    while pending:
        pid = pending.pop()
        task_dir = Path(f"/proc/{pid}/task")
        try:
            tids = os.listdir(task_dir)
        except OSError:
            continue
        for tid in tids:
            try:
                children = (task_dir / tid / "children").read_text().split()
            except OSError:
                continue
            for child in children:
                child_pid = int(child)
                if child_pid not in seen:
                    seen.add(child_pid)
                    pending.append(child_pid)
    return seen


def _owns_listener(process: subprocess.Popen[Any], port: int) -> bool:
    """True only when a LISTEN socket on `port` belongs to `process` or one of
    its descendants.

    A port that merely accepts may belong to a foreign process (a concurrent
    or stale server on the shared host); readiness must be proven, not assumed.
    """
    inodes = _listener_inodes(port)
    if not inodes:
        return False
    for pid in _descendant_pids(process.pid):
        fd_dir = Path(f"/proc/{pid}/fd")
        try:
            entries = os.listdir(fd_dir)
        except OSError:
            continue
        for entry in entries:
            try:
                target = os.readlink(fd_dir / entry)
            except OSError:
                continue
            if target.startswith("socket:[") and int(target[8:-1]) in inodes:
                return True
    return False


def _require_event_fds(
    process: subprocess.Popen[Any] | None, parent: Path
) -> tuple[selectors.BaseSelector, int, int]:
    """Selector over pidfd + inotify; Linux event sources are mandatory."""
    notify_fd = _inotify_for(parent)
    if notify_fd < 0:
        raise RuntimeError("harness readiness requires Linux inotify")
    pid_fd = -1
    if process is not None:
        if process.pid is None:
            os.close(notify_fd)
            raise RuntimeError("harness readiness requires a running process")
        pid_fd = _pidfd_open(process.pid)
        if pid_fd < 0:
            os.close(notify_fd)
            raise RuntimeError("harness readiness requires Linux pidfd")
    selector = selectors.DefaultSelector()
    selector.register(notify_fd, selectors.EVENT_READ)
    if pid_fd >= 0:
        selector.register(pid_fd, selectors.EVENT_READ)
    return selector, notify_fd, pid_fd


PHASES = ("preflight", "server_start", "client_start", "join",
          "scenario", "assertion", "teardown")


def tag_failure(error: BaseException, name: str) -> None:
    if name not in PHASES:
        raise ValueError(f"unknown lifecycle phase: {name}")
    if not hasattr(error, "failure_phase"):
        error.failure_phase = name


@contextlib.contextmanager
def phase(name: str):
    """Tag a failure escaping this block with the lifecycle phase it was observed in.

    The first (innermost) tag wins, so a helper's own phase is never
    overwritten by an enclosing scenario section.
    """
    if name not in PHASES:
        raise ValueError(f"unknown lifecycle phase: {name}")
    try:
        yield
    except BaseException as error:
        tag_failure(error, name)
        raise


def in_phase(name: str):
    """Decorator form of `phase` for the shared launch/join/teardown helpers."""
    def wrap(function):
        @functools.wraps(function)
        def tagged(*args, **kwargs):
            with phase(name):
                return function(*args, **kwargs)
        return tagged
    return wrap


def _causes(error: BaseException):
    """The error and everything it was raised from or during, outermost first."""
    seen: set[int] = set()
    pending: list[BaseException] = [error]
    while pending:
        current = pending.pop(0)
        if id(current) in seen:
            continue
        seen.add(id(current))
        yield current
        pending += [link for link in (current.__cause__, current.__context__) if link is not None]


def observed_phase(error: BaseException) -> str:
    """The phase tagged on `error` or on a failure that caused it; `unknown` if none."""
    return next((current.failure_phase for current in _causes(error)
                 if getattr(current, "failure_phase", None) in PHASES), "unknown")


def child_exit(error: BaseException) -> ChildExited | None:
    """The first ChildExited in the failure's cause chain."""
    return next((current for current in _causes(error) if isinstance(current, ChildExited)), None)


class ChildExited(RuntimeError):
    """A child process exited (returncode < 0: killed by that signal) while it was awaited."""

    def __init__(self, message: str, returncode: int, log_path: Path) -> None:
        super().__init__(message)
        self.returncode = returncode
        self.log_path = log_path


def wait_port(
    port: int,
    timeout_seconds: float,
    process: subprocess.Popen[Any],
    *,
    log_path: Path,
    phase_name: str = "server_start",
) -> None:
    """Return once `process` itself listens on the port; raise if the process
    exits or time runs out.

    A bare accept on the port is not readiness: on the shared host another
    run's server may hold it. `phase_name` is the lifecycle phase a failure
    belongs to: the server's readiness by default, `client_start` when the
    port is a client's MCP.
    """
    with phase(phase_name):
        _wait_port(port, timeout_seconds, process, log_path)


def _wait_port(port: int, timeout_seconds: float, process: subprocess.Popen[Any], log_path: Path) -> None:
    """Re-probes happen only after real events: socket writability, log growth
    (inotify), or process exit (pidfd). There is no polling quantum.

    A probe that succeeds against a foreign listener is not readiness: the
    port counts as ready only once `process` owns the LISTEN socket itself.
    """
    if timeout_seconds <= 0:
        raise ValueError("timeout_seconds must be positive")
    deadline = time.monotonic() + timeout_seconds
    parent = log_path.parent
    parent.mkdir(parents=True, exist_ok=True)
    selector, notify_fd, pid_fd = _require_event_fds(process, parent)
    try:
        while True:
            if process.poll() is not None:
                tail = _tail_log(log_path)
                raise ChildExited(
                    f"process exited with {process.returncode} before port "
                    f"{port} became ready; log {log_path}: {tail[-2000:]}",
                    process.returncode, log_path,
                )
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                tail = _tail_log(log_path)
                raise RuntimeError(
                    f"port {port} did not become ready within "
                    f"{timeout_seconds:.1f}s; log {log_path}: {tail[-2000:]}"
                )
            if _probe_port(port, remaining) and _owns_listener(process, port):
                return
            # Only log growth or process exit wakes us to probe again.
            for key, _ in selector.select(max(0.0, deadline - time.monotonic())):
                _drain(key.fd)
    finally:
        selector.close()
        os.close(notify_fd)
        if pid_fd >= 0:
            os.close(pid_fd)


def wait_for_path_condition(path: Path, timeout_seconds: float, condition: Any) -> Any:
    """Evaluate a filesystem condition again only when its parent changes."""
    path.parent.mkdir(parents=True, exist_ok=True)
    selector, notify_fd, _ = _require_event_fds(None, path.parent)
    try:
        deadline = time.monotonic() + timeout_seconds
        while True:
            matched, value = condition()
            if matched:
                return value
            remaining = deadline - time.monotonic()
            if remaining <= 0.0 or not selector.select(remaining):
                raise TimeoutError(f"filesystem event timeout for {path}")
            _drain(notify_fd)
    finally:
        selector.close()
        os.close(notify_fd)


@in_phase("server_start")
def start_server(*args: Any, **kwargs: Any) -> subprocess.Popen[Any]:
    """Launch a server process; readiness is observed separately by wait_port."""
    return subprocess.Popen(*args, **kwargs)


@in_phase("teardown")
def stop_process(
    process: subprocess.Popen[Any] | None,
    *,
    interrupt: bool = False,
    timeout_seconds: float = 8.0,
) -> None:
    if process is None or process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGINT if interrupt else signal.SIGTERM)
    except (ProcessLookupError, PermissionError):
        return
    try:
        process.wait(timeout=timeout_seconds)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass
        process.wait(timeout=5)


def _wait_for_file(path: Path, timeout_seconds: float, process: subprocess.Popen[Any]) -> None:
    """Block until path exists; raise if process exits first (event-driven)."""
    if timeout_seconds <= 0:
        raise ValueError("timeout_seconds must be positive")
    deadline = time.monotonic() + timeout_seconds
    path.parent.mkdir(parents=True, exist_ok=True)
    selector, notify_fd, pid_fd = _require_event_fds(process, path.parent)
    try:
        while not path.exists():
            if process.poll() is not None:
                raise RuntimeError(
                    f"Xvfb exited with {process.returncode} before {path} appeared"
                )
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise RuntimeError(f"Xvfb socket {path} did not appear in time")
            for key, _ in selector.select(remaining):
                _drain(key.fd)
    finally:
        selector.close()
        os.close(notify_fd)
        if pid_fd >= 0:
            os.close(pid_fd)


def validate_screen_size(size: tuple[int, int] | list[int]) -> tuple[int, int]:
    """Bound a private display before creating files or starting a process."""
    if (not isinstance(size, (tuple, list)) or len(size) != 2
            or any(type(side) is not int for side in size)
            or not 320 <= size[0] <= 3840 or not 240 <= size[1] <= 2160):
        raise ValueError("screen_size must be [width, height] in 320..3840 × 240..2160")
    return size[0], size[1]


@in_phase("client_start")
def start_xvfb(artifact_dir: Path, screen_size: tuple[int, int] = (1280, 720)) -> tuple[subprocess.Popen[Any], str, Any]:
    """Start Xvfb and return (process, display, log_handle).

    Readiness comes from Xvfb's -displayfd announcement (or the X socket
    file event on fallback), never from assuming survival after a delay.
    """
    width, height = validate_screen_size(screen_size)
    screen_spec = f"{width}x{height}x24"
    artifact_dir.mkdir(parents=True, exist_ok=True)
    log_handle = (artifact_dir / "xvfb.log").open("wb")
    read_fd, write_fd = os.pipe()
    try:
        try:
            process = subprocess.Popen(
                [
                    "Xvfb",
                    "-displayfd",
                    str(write_fd),
                    "-screen",
                    "0",
                    screen_spec,
                    "-nolisten",
                    "tcp",
                ],
                cwd=REPO_ROOT,
                stdout=log_handle,
                stderr=subprocess.STDOUT,
                start_new_session=True,
                pass_fds=(write_fd,),
            )
        except (FileNotFoundError, OSError):
            return _start_xvfb_scan(artifact_dir, log_handle, screen_spec)
        finally:
            try:
                os.close(write_fd)
            except OSError:
                pass
        pid_fd = _pidfd_open(process.pid)
        deadline = time.monotonic() + _XVFB_READY_TIMEOUT_SECONDS
        display_number = b""
        selector = selectors.DefaultSelector()
        try:
            selector.register(read_fd, selectors.EVENT_READ)
            if pid_fd >= 0:
                selector.register(pid_fd, selectors.EVENT_READ)
            while True:
                if process.poll() is not None:
                    # -displayfd unsupported or Xvfb failed: fall back to scan.
                    stop_process(process)
                    return _start_xvfb_scan(artifact_dir, log_handle, screen_spec)
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    stop_process(process)
                    raise RuntimeError("Xvfb did not announce a display via -displayfd")
                for key, _ in selector.select(remaining):
                    if key.fd == read_fd:
                        chunk = os.read(read_fd, 64)
                        if not chunk:
                            stop_process(process)
                            return _start_xvfb_scan(artifact_dir, log_handle, screen_spec)
                        display_number += chunk
                        if display_number.endswith(b"\n"):
                            number = int(display_number.decode().strip())
                            if number in _OWNER_DISPLAYS:
                                stop_process(process)
                                return _start_xvfb_scan(artifact_dir, log_handle, screen_spec)
                            return process, f":{number}", log_handle
                    else:
                        _drain(key.fd)
        finally:
            selector.close()
            if pid_fd >= 0:
                os.close(pid_fd)
    except BaseException:
        log_handle.close()
        raise
    finally:
        os.close(read_fd)


def _start_xvfb_scan(
    artifact_dir: Path, log_handle: Any, screen_spec: str
) -> tuple[subprocess.Popen[Any], str, Any]:
    for display_number in range(99, 200):
        socket_path = Path(f"/tmp/.X11-unix/X{display_number}")
        if socket_path.exists():
            continue
        process = subprocess.Popen(
            ["Xvfb", f":{display_number}", "-screen", "0", screen_spec, "-nolisten", "tcp"],
            cwd=REPO_ROOT,
            stdout=log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            _wait_for_file(socket_path, _XVFB_READY_TIMEOUT_SECONDS, process)
        except RuntimeError:
            stop_process(process)
            continue
        if _x_lock_pid(display_number) != process.pid:
            # A concurrent Xvfb won this number; its socket satisfied the wait.
            stop_process(process)
            continue
        return process, f":{display_number}", log_handle
    raise RuntimeError("could not start an isolated Xvfb display")


def _x_lock_pid(display_number: int) -> int | None:
    try:
        return int(Path(f"/tmp/.X{display_number}-lock").read_text().strip())
    except (OSError, ValueError):
        return None


def current_client_state(client: Any, timeout_seconds: float) -> dict[str, Any]:
    if timeout_seconds <= 0:
        raise ValueError("timeout_seconds must be positive")
    observed = client.call_tool("minecraft_observe")
    version = observed.get("state_version")
    if not isinstance(version, int) or version < 0:
        raise RuntimeError(f"client observation omitted state_version: {observed}")
    return observed


def next_client_state(client: Any, observed: dict[str, Any], deadline: float) -> dict[str, Any]:
    version = observed.get("state_version")
    if not isinstance(version, int) or version < 0:
        raise RuntimeError(f"client state snapshot omitted state_version: {observed}")
    remaining = deadline - time.monotonic()
    if remaining < 0.1:
        raise TimeoutError(f"client state event deadline elapsed: {observed.get('screen')}")
    # Push-driven: the client pushes a state-change notification; the call
    # returns when it fires, not when a sleep interval ends.
    client.call_tool(
        "minecraft_wait_for_state_change",
        {"observed_version": version, "timeout_seconds": min(remaining, 120.0)},
    )
    return current_client_state(client, remaining)


def retry_tool(
    client: Any, name: str, arguments: dict[str, Any], timeout_seconds: float
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_seconds
    last_error: Exception | None = None
    observed: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        try:
            return client.call_tool(name, arguments)
        except Exception as error:  # MCP reports transient screen/state mismatches.
            last_error = error
            if observed is None:
                observed = current_client_state(client, min(timeout_seconds, 120.0))
            else:
                observed = next_client_state(client, observed, deadline)
    raise RuntimeError(f"{name} did not succeed within {timeout_seconds:.1f}s: {last_error}")


@in_phase("client_start")
def wait_client_ready_for_connect(client: Any, timeout_seconds: float) -> dict[str, Any]:
    import json as _json

    deadline = time.monotonic() + timeout_seconds
    last = current_client_state(client, min(timeout_seconds, 120.0))
    while True:
        screen_payload = last.get("screen")
        if isinstance(screen_payload, dict):
            screen = str(screen_payload.get("class", ""))
        else:
            screen = str(last.get("current_screen", ""))
        if not last.get("in_play") and screen.endswith("TitleScreen"):
            return last
        if screen.endswith("LoadingErrorScreen"):
            raise RuntimeError(
                "Forge loading warning/error screen blocked bootstrap: "
                + _json.dumps(last, ensure_ascii=False, sort_keys=True)
            )
        if time.monotonic() >= deadline:
            break
        last = next_client_state(client, last, deadline)
    raise RuntimeError(
        "client bootstrap did not reach TitleScreen before connect: "
        + _json.dumps(last, ensure_ascii=False, sort_keys=True)
    )


def _agent_jar_path() -> Path:
    """Locate the built java-agent jar (built once via mem-run, never here)."""
    override = os.environ.get("KRASNOKAMEN_JAVA_AGENT_JAR")
    if override:
        return Path(override).expanduser()
    candidates = sorted(sibling_root("loader").glob("java-agent/build/libs/*.jar"))
    if candidates:
        return candidates[0]
    raise MissingCapability(
        "java-agent jar missing (expected java-agent/build/libs/*.jar); "
        "build :java-agent:jar once through mem-run, never inside this call"
    )


@in_phase("client_start")
def start_client(
    *,
    game_dir: Path,
    log_path: Path,
    token: str,
    mcp_port: int,
    username: str,
    display: str,
    platform: str | None = None,
    launch_manifest: Path | None = None,
    client_profile: str | None = None,
    window_size: tuple[int, int] | None = None,
) -> tuple[subprocess.Popen[Any], Any]:
    """Launch a real client and return (process, log_handle) once MCP is ready.

    ``client_profile`` selects the launch. ``None`` (legacy default) preserves
    the historic no-Loader Fabric agent task ``:fabric-agent:runClientMcp`` —
    that module is a NeoForge mod despite its directory name, so ``None`` must
    never be described as vanilla Minecraft. Explicit ``vanilla`` runs the
    true plain-Mojang 26.1.2 client: the unmodified ``net.minecraft.client.main.Main``
    from locally cached Mojang metadata plus ``-javaagent`` instrumentation
    (``tools/harness/java_launcher.py::vanilla_command``), no modloader, one
    direct Java process per client (512M heap, isolated natives/game dir,
    read-only Gradle/Prism caches). ``loader`` requires an explicit platform and
    uses the matching Loader task; ``svc``/``dh`` reuse the real
    ``:loader-fabric:runClientMcp`` Fabric launch with the pinned publisher
    jar staged into the run's isolated ``game_dir/mods`` (verified
    size/sha512/sha1, fail-closed before launch starts). Prewritten game
    options are preserved. Readiness is the MCP port accepting (via
    log/process events); the call never returns merely because time passed.
    Profile artifacts stay under the caller's game dir (runs/RunJava or a
    worktree .analysis dir on the games disk), never the root disk.
    """
    if not token:
        raise ValueError("token must not be empty")
    if not isinstance(mcp_port, int) or not 1 <= mcp_port <= 65535:
        raise ValueError(f"mcp_port must be 1..65535, got {mcp_port!r}")
    if not USERNAME_RE.match(username or ""):
        raise ValueError(
            f"username must be 1..16 ASCII letters, digits, or underscores, got {username!r}"
        )
    if not display:
        raise ValueError("display must not be empty")
    if platform is not None and platform not in PLATFORMS:
        raise ValueError(f"platform must be one of {PLATFORMS}, got {platform!r}")
    if client_profile is not None and client_profile not in CLIENT_PROFILES:
        raise ValueError(f"client_profile must be one of {CLIENT_PROFILES}, got {client_profile!r}")
    resolved = _resolve_profile_task(client_profile, platform)
    task = resolved["task"]
    game_dir = REPO_ROOT / game_dir if not game_dir.is_absolute() else game_dir
    game_dir.mkdir(parents=True, exist_ok=True)
    options = game_dir / "options.txt"
    audio_options = {"soundCategory_master": "0.0", "soundCategory_music": "0.0"}
    lines = options.read_text().splitlines() if options.exists() else ["version:4790", "onboardAccessibility:false"]
    lines = [line for line in lines if line.partition(":")[0] not in (*audio_options, "soundDevice")]
    options.write_text("\n".join(lines + [f"{key}:{value}" for key, value in audio_options.items()]) + "\n")
    if client_profile in ("svc", "dh"):
        _stage_profile_mods(client_profile, resolved["platform"], game_dir)
    if port_open(mcp_port):
        raise RuntimeError(
            f"MCP port {mcp_port} is already in use; stop the existing client first"
        )
    env = os.environ.copy()
    # HOME alone does not change Java's user.home on Unix. Pass the property
    # to Gradle and its forked client so Forge's assets root is private too.
    home = game_dir / ".home"
    home.mkdir(parents=True, exist_ok=True)
    env["HOME"] = str(home)
    env["JAVA_TOOL_OPTIONS"] = (
        f'{env.get("JAVA_TOOL_OPTIONS", "")} -Duser.home="{home}"'
    ).strip()
    # Only the harness-owned Xvfb may reach this client; audio is the OpenAL
    # null backend with no audio server reachable (owner rule, never the
    # owner's speakers or microphone).
    env.pop("WAYLAND_DISPLAY", None)
    env.update(
        {
            "DISPLAY": display,
            "SDL_VIDEODRIVER": "x11",
            "SDL_VIDEO_DRIVER": "x11",
            "GDK_BACKEND": "x11",
            "XDG_SESSION_TYPE": "x11",
            "KRASNOKAMEN_CLIENT_MCP_TOKEN": token,
            "KRASNOKAMEN_CLIENT_MCP_PORT": str(mcp_port),
            "KRASNOKAMEN_CLIENT_MCP_GAME_DIR": str(game_dir),
            "KRASNOKAMEN_CLIENT_MCP_USERNAME": username,
        }
    )
    from .audio import force_java_audio_isolation, require_java_audio_isolation
    force_java_audio_isolation(env, game_dir)
    require_java_audio_isolation(env, game_dir)
    env.setdefault("JDK_JAVA_OPTIONS", _CLIENT_MAX_HEAP)
    log_path.parent.mkdir(parents=True, exist_ok=True)
    log_handle = log_path.open("wb")
    try:
        if launch_manifest is not None or client_profile == "vanilla":
            from .java_launcher import prepared_fabric_command, vanilla_command

            command = (prepared_fabric_command(launch_manifest, game_dir, username)
                       if launch_manifest is not None else vanilla_command(
                           game_dir=game_dir, username=username, agent_jar=_agent_jar_path()))
            if window_size is not None:
                if min(window_size) < 1:
                    raise ValueError("client window dimensions must be positive")
                command.extend(["--width", str(window_size[0]), "--height", str(window_size[1])])
            process = subprocess.Popen(
                command,
                cwd=REPO_ROOT,
                env=env,
                stdout=log_handle,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
        else:
            loader_root = sibling_root("loader")
            gradlew = loader_root / "gradlew"
            if not (gradlew.is_file() and os.access(gradlew, os.X_OK)):
                raise RuntimeError(f"missing executable Gradle wrapper: {gradlew}")
            process = subprocess.Popen(
                [
                    str(gradlew),
                    "--no-daemon",
                    "--no-configuration-cache",
                    "-p",
                    str(loader_root),
                    f"-Pkrasnokamen.clientMcp.gameDir={game_dir}",
                    f"-Pkrasnokamen.clientMcp.username={username}",
                    task,
                ],
                cwd=REPO_ROOT,
                env=env,
                stdout=log_handle,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
    except Exception:
        log_handle.close()
        raise
    try:
        wait_port(mcp_port, _CLIENT_READY_TIMEOUT_SECONDS, process, log_path=log_path,
                  phase_name="client_start")
        if client_profile == "vanilla":
            # premain exposes MCP before Minecraft.getInstance() is assigned.
            # This constructor log precedes the render loop's event snapshots.
            def initialized():
                code = process.poll()
                if code is not None:
                    raise ChildExited("Java exited before Minecraft initialized", code, log_path)
                text = log_path.read_text(errors="replace")
                return "Setting user:" in text, text
            wait_for_path_condition(log_path, _CLIENT_READY_TIMEOUT_SECONDS, initialized)
    except Exception:
        stop_process(process)
        log_handle.close()
        raise
    return process, log_handle
