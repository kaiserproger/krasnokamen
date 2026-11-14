"""CPU-throttled release-server and software-rendered singleplayer probe."""
from __future__ import annotations

import os
from pathlib import Path
import select
import shlex
import subprocess
import time

from .. import runtime
from . import native_client


def stage_server(server: Path, stage: Path, artifact: Path, *, slow: bool = True) -> tuple[Path, str | None]:
    """Place a lifecycle-owned scope wrapper beside the copied native client."""
    unit = "solaris-sp-slow-" + os.urandom(8).hex()
    pidfile = artifact / "slow-server.pid"
    process_pidfile = artifact / "server-process.pid"
    quota = artifact / "slow-server-cpu.max"
    if not slow:
        wrapper = stage / "krasnokamen"
        wrapper.write_text(
            "#!/bin/sh\n"
            f"printf '%s\\n' \"$$\" > {shlex.quote(str(pidfile))}\n"
            f"printf '%s\\n' \"$$\" > {shlex.quote(str(process_pidfile))}\n"
            f"exec {shlex.quote(str(server))} \"$@\"\n"
        )
        wrapper.chmod(0o755)
        return pidfile, None
    child = ('set -e; printf "%s\\n" "$$" > "$1"; shift; '
             'IFS=: read -r _ _ group < /proc/self/cgroup; '
             'cat "/sys/fs/cgroup$group/cpu.max" > "$1"; shift; exec "$@"')
    wrapper = stage / "krasnokamen"
    wrapper.write_text(
        "#!/bin/sh\n"
        f"printf '%s\\n' \"$$\" > {shlex.quote(str(pidfile))}\n"
        f"exec systemd-run --user --scope --quiet --collect --expand-environment=no --unit={unit} "
        "--slice=solaris-heavy.slice --property=MemoryMax=3G --property=MemorySwapMax=1G "
        f"--property=CPUQuota=1% -- /bin/sh -c {shlex.quote(child)} slow-server "
        f"{shlex.quote(str(process_pidfile))} {shlex.quote(str(quota))} "
        f"{shlex.quote(str(server))} \"$@\"\n"
    )
    wrapper.chmod(0o755)
    return pidfile, unit


def release_startup_quota(unit: str) -> None:
    """End the startup-only slow fixture once the real server has listened."""
    subprocess.run(
        ["systemctl", "--user", "set-property", "--runtime", f"{unit}.scope", "CPUQuota="],
        check=True, capture_output=True, text=True,
    )


def wait_server(pidfile: Path, log: Path, timeout: float, *, on_progress=None) -> str:
    """Wait for real server readiness or its wrapper's exit, using inotify/pidfd."""
    deadline = time.monotonic() + timeout
    runtime.wait_for_path_condition(pidfile, timeout,
        lambda: (pidfile.is_file() and pidfile.read_text().strip().isdigit(), None))
    pid = int(pidfile.read_text())
    pidfd = os.pidfd_open(pid)
    notify = runtime._inotify_for(log.parent)
    if notify < 0:
        os.close(pidfd)
        raise RuntimeError("slow-machine gate requires inotify")
    try:
        while True:
            if select.select([pidfd], [], [], 0)[0]:
                return "exited"
            if log.is_file():
                text = log.read_text(errors="replace")
                if on_progress is not None and 'stage="registries"' in text:
                    on_progress(text)
                    on_progress = None
                if "is listening" in text:
                    return "listening"
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("slow singleplayer server neither listened nor exited")
            ready, _, _ = select.select([pidfd, notify], [], [], remaining)
            if not ready:
                raise TimeoutError("slow singleplayer server neither listened nor exited")
            if notify in ready:
                os.read(notify, 65536)
    finally:
        os.close(notify)
        os.close(pidfd)


def capture(probe, artifact: Path, name: str) -> dict:
    image = native_client.capture(probe.client, artifact, name)
    adapter = image.get("previous_render_metrics", {}).get("adapter", {})
    identity = " ".join(str(adapter.get(key, "")) for key in ("device_type", "name", "driver")).lower()
    probe.check("software_renderer", "cpu" in identity or "llvmpipe" in identity or "lavapipe" in identity)
    return image


def rss(client_pid: int, server_pid: int) -> dict:
    """Retain process high-water marks and the actual shared hardware envelope."""
    result = {}
    for name, pid in (("client", client_pid), ("server", server_pid)):
        fields = dict(line.split(":", 1) for line in Path(f"/proc/{pid}/status").read_text().splitlines())
        group = Path(f"/proc/{pid}/cgroup").read_text().strip().split("::", 1)[1]
        cgroup = Path("/sys/fs/cgroup") / group.lstrip("/")
        result[name] = {
            "pid": pid,
            "rss_kib": int(fields["VmRSS"].split()[0]),
            "peak_rss_kib": int(fields["VmHWM"].split()[0]),
            "cpus": fields["Cpus_allowed_list"].strip(),
            "cgroup": str(cgroup),
            "limits": {key: (cgroup / key).read_text().strip()
                       for key in ("memory.max", "memory.peak", "cpu.max")},
        }
    return result
