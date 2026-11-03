"""Fail-closed inotify guard for the Spain diagnostic's private world writes."""
from __future__ import annotations

import asyncio
import ctypes
import os
from pathlib import Path
import shutil
import struct
from typing import Any

_EVENT = struct.Struct("iIII")
_MODIFY = 0x2
_ATTRIB = 0x4
_CLOSE_WRITE = 0x8
_MOVED_TO = 0x80
_CREATE = 0x100
_DELETE = 0x200
_DELETE_SELF = 0x400
_MOVE_SELF = 0x800
_OVERFLOW = 0x4000
_IGNORED = 0x8000
_ISDIR = 0x40000000
_MASK = _MODIFY | _ATTRIB | _CLOSE_WRITE | _MOVED_TO | _CREATE | _DELETE | _DELETE_SELF | _MOVE_SELF


class WorldDiskGuard:
    """Check free space on private-world file events; unrelated writers are not watched."""

    def __init__(self, world: Path, minimum_bytes: int, task: asyncio.Task[Any]):
        self.world, self.minimum_bytes, self.task = world, minimum_bytes, task
        self.loop = asyncio.get_running_loop()
        self.libc = ctypes.CDLL(None, use_errno=True)
        self.libc.inotify_init1.argtypes = [ctypes.c_int]
        self.libc.inotify_init1.restype = ctypes.c_int
        self.libc.inotify_add_watch.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_uint32]
        self.libc.inotify_add_watch.restype = ctypes.c_int
        self.fd = self.libc.inotify_init1(os.O_NONBLOCK | os.O_CLOEXEC)
        if self.fd < 0:
            error = ctypes.get_errno()
            raise OSError(error, os.strerror(error))
        self.watches: dict[int, Path] = {}
        self.sizes: dict[Path, int] = {}
        self.events = 0
        self.reason: str | None = None
        self.minimum_free = shutil.disk_usage(world).free
        try:
            self._watch_tree(world)
            self.initial_world_bytes = sum(self.sizes.values())
            if self.minimum_free < minimum_bytes:
                raise RuntimeError("Spain world disk reserve already below minimum")
            self.loop.add_reader(self.fd, self._ready)
        except BaseException:
            os.close(self.fd)
            raise

    def _watch_tree(self, root: Path) -> None:
        def failed(error: OSError) -> None:
            raise error
        for directory, _, files in os.walk(root, onerror=failed, followlinks=False):
            path = Path(directory)
            watch = self.libc.inotify_add_watch(self.fd, os.fsencode(path), _MASK)
            if watch < 0:
                error = ctypes.get_errno()
                raise OSError(error, f"inotify watch {path}: {os.strerror(error)}")
            self.watches[watch] = path
            for name in files:
                child = path / name
                try:
                    if not child.is_symlink():
                        self.sizes[child] = child.stat().st_size
                except FileNotFoundError:
                    pass

    def _fail(self, reason: str) -> None:
        if self.reason is None:
            self.reason = reason
            self.task.cancel()

    def _ready(self) -> None:
        try:
            while True:
                try:
                    data = os.read(self.fd, 65536)
                except BlockingIOError:
                    break
                if not data:
                    raise RuntimeError("Spain world inotify stream closed")
                offset = 0
                while offset < len(data):
                    if len(data) - offset < _EVENT.size:
                        raise RuntimeError("Spain world inotify event truncated")
                    watch, mask, _, length = _EVENT.unpack_from(data, offset)
                    offset += _EVENT.size
                    if len(data) - offset < length:
                        raise RuntimeError("Spain world inotify name truncated")
                    name = os.fsdecode(data[offset:offset + length].split(b"\0", 1)[0])
                    offset += length
                    if mask & (_OVERFLOW | _IGNORED | _DELETE_SELF | _MOVE_SELF):
                        raise RuntimeError("Spain world inotify lost events or a watched directory")
                    parent = self.watches.get(watch)
                    if parent is None:
                        raise RuntimeError("Spain world inotify unknown watch")
                    path = parent / name if name else parent
                    self.events += 1
                    if mask & _ISDIR and mask & (_CREATE | _MOVED_TO) and path.is_dir():
                        self._watch_tree(path)
                    elif not mask & _ISDIR:
                        try:
                            if path.is_symlink():
                                raise RuntimeError("Spain world acquired a symlink")
                            self.sizes[path] = path.stat().st_size
                        except FileNotFoundError:
                            self.sizes.pop(path, None)
                self.minimum_free = min(self.minimum_free, shutil.disk_usage(self.world).free)
                if self.minimum_free < self.minimum_bytes:
                    raise RuntimeError("Spain world disk reserve fell below 60 GiB")
        except (OSError, RuntimeError) as error:
            self._fail(f"{type(error).__name__}: {error}")

    def close(self) -> dict[str, Any]:
        self.loop.remove_reader(self.fd)
        os.close(self.fd)
        return {"watch": "recursive private-world inotify", "events": self.events,
                "minimum_free_bytes": self.minimum_free, "minimum_required_bytes": self.minimum_bytes,
                "initial_world_logical_bytes": self.initial_world_bytes,
                "observed_world_logical_bytes": sum(self.sizes.values()), "reason": self.reason,
                "limitation": "unrelated writers can consume space between world events; logical bytes are not allocated bytes"}
