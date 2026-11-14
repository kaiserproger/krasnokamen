#!/usr/bin/env python3
"""Kill a runner's scope if either its launcher or wrapper disappears."""
import array
import os
import select
import selectors
import shutil
import signal
import socket
import sys

if sys.argv[1] == '--send-locks':
    fds = array.array('i')
    for fd in (3, 6, 7, 8):
        try:
            os.fstat(fd)
        except OSError:
            continue
        fds.append(fd)
    with socket.socket(socket.AF_UNIX) as conn:
        conn.connect(sys.argv[2] + '/locks')
        conn.sendmsg([b'1'], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, fds)] if fds else [])
        if conn.recv(1) != b'1':
            raise RuntimeError('guardian did not accept locks')
    sys.exit(0)

control, cgroup, wrapper, caller = sys.argv[1:]
selector = selectors.DefaultSelector()
fifo = os.open(control + '/events', os.O_RDWR | os.O_NONBLOCK)
listener = socket.socket(socket.AF_UNIX)
listener.bind(control + '/locks')
listener.listen(1)
selector.register(listener, selectors.EVENT_READ)
held_locks = []
selector.register(fifo, selectors.EVENT_READ)
try:
    wrapper_fd = os.pidfd_open(int(wrapper))
    selector.register(wrapper_fd, selectors.EVENT_READ)
    caller_fd = os.pidfd_open(int(caller)) if caller != wrapper else wrapper_fd
    if caller_fd != wrapper_fd:
        selector.register(caller_fd, selectors.EVENT_READ)
    ready_fd = os.open(control + '/ready', os.O_RDWR | os.O_NONBLOCK)
    os.write(ready_fd, b'ready\n')
    while True:
        events = selector.select()
        # Completion wins over wrapper exit, but not over the caller dying.
        caller_dead = caller_fd != wrapper_fd and any(key.fd == caller_fd for key, _ in events)
        if any(key.fileobj == listener for key, _ in events):
            conn, _ = listener.accept()
            with conn:
                _, ancillary, _, _ = conn.recvmsg(1, socket.CMSG_SPACE(4 * array.array('i').itemsize))
                for level, kind, data in ancillary:
                    if level == socket.SOL_SOCKET and kind == socket.SCM_RIGHTS:
                        fds = array.array('i')
                        fds.frombytes(data[:len(data) - len(data) % fds.itemsize])
                        held_locks.extend(fds)
                conn.sendall(b'1')
            selector.unregister(listener)
            listener.close()
        done = False
        if any(key.fd == fifo for key, _ in events):
            done = b'done\n' in os.read(fifo, 4096) and not caller_dead
        if done:
            # Hold the slot locks until the wrapper exits and the scope empties.
            select.select([wrapper_fd, caller_fd], [], [])
        if done or any(key.fd in (wrapper_fd, caller_fd) for key, _ in events):
            # Prevent a surviving launcher from creating the scope after our check.
            try:
                signal.pidfd_send_signal(wrapper_fd, signal.SIGKILL)
            except ProcessLookupError:
                pass
            try:
                events_fd = os.open(cgroup + '/cgroup.events', os.O_RDONLY)
            except FileNotFoundError:
                events_fd = None
            if events_fd is not None:
                with open(cgroup + '/cgroup.kill', 'w') as kill:
                    kill.write('1\n')
            selectors_for_exit = selectors.DefaultSelector()
            selectors_for_exit.register(wrapper_fd, selectors.EVENT_READ)
            selectors_for_exit.select()
            selectors_for_exit.close()
            try:
                if events_fd is None:
                    events_fd = os.open(cgroup + '/cgroup.events', os.O_RDONLY)
                    with open(cgroup + '/cgroup.kill', 'w') as kill:
                        kill.write('1\n')
                poller = select.poll()
                poller.register(events_fd, select.POLLPRI | select.POLLERR)
                while True:
                    os.lseek(events_fd, 0, os.SEEK_SET)
                    if b'populated 0\n' in os.read(events_fd, 4096):
                        break
                    poller.poll()
                os.close(events_fd)
            except FileNotFoundError:
                pass  # The launcher died before creating the scope.
            break
finally:
    shutil.rmtree(control, ignore_errors=True)
