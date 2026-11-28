"""Archive atomically published native WAL batches before normal checkpoint pruning."""
import ctypes
import os
from pathlib import Path
import re
import select
import shutil
import struct
import threading


class JournalWitness:
    def __init__(self, source, destination):
        self.source, self.destination = Path(source), Path(destination)
        self.destination.mkdir(exist_ok=False)
        self.error = None
        self.condition = threading.Condition()
        self.frontier = 0
        self.libc = ctypes.CDLL(None, use_errno=True)
        self.fd = self.libc.inotify_init1(os.O_CLOEXEC)
        if self.fd < 0:
            raise OSError(ctypes.get_errno(), 'inotify_init1')
        if self.libc.inotify_add_watch(self.fd, os.fsencode(self.source), 0x80) < 0:
            os.close(self.fd)
            raise OSError(ctypes.get_errno(), 'inotify_add_watch')
        self.read_stop, self.write_stop = os.pipe()
        for path in self.source.iterdir():
            self.copy(path.name)
        self.thread = threading.Thread(target=self.run, name='d54-journal-witness', daemon=True)
        self.thread.start()

    def copy(self, name):
        if not re.fullmatch(r'batch-\d+-\d+\.bin', name):
            return
        destination = self.destination / name
        if destination.exists():
            raise ValueError(f'published WAL batch name reused: {name}')
        # An open descriptor survives the core's normal subsequent unlink.
        with (self.source / name).open('rb') as source, destination.open('xb') as target:
            shutil.copyfileobj(source, target, 1024 * 1024)
        with self.condition:
            self.frontier = max(self.frontier, int(name.removesuffix('.bin').rsplit('-', 1)[1]))
            self.condition.notify_all()

    def run(self):
        try:
            while True:
                readable, _, _ = select.select([self.fd, self.read_stop], [], [])
                if self.fd in readable:
                    events = os.read(self.fd, 65536)
                    offset = 0
                    while offset < len(events):
                        _, mask, _, size = struct.unpack_from('iIII', events, offset)
                        offset += 16
                        name = os.fsdecode(events[offset:offset + size].split(b'\0', 1)[0])
                        offset += size
                        if mask & 0x4000:
                            raise RuntimeError('inotify journal witness queue overflow')
                        if mask & 0x80:
                            self.copy(name)
                if self.read_stop in readable:
                    return
        except BaseException as error:
            with self.condition:
                self.error = error
                self.condition.notify_all()

    def wait_published(self, tick, timeout):
        with self.condition:
            if not self.condition.wait_for(lambda: self.frontier >= tick or self.error is not None, timeout):
                raise TimeoutError(f'published journal frontier did not reach {tick}')
            if self.error is not None:
                raise self.error

    def close(self):
        os.write(self.write_stop, b'x')
        self.thread.join()
        for fd in (self.fd, self.read_stop, self.write_stop):
            os.close(fd)
        if self.error is not None:
            raise self.error
