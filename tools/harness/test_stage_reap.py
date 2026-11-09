"""Real stage process-tree timeout and interruption regressions."""

import json
import os
import select
import signal
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


class StageReapTests(unittest.TestCase):
    def run_case(self, stop_signal, detached=False):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fifo = root / 'ready'
            os.mkfifo(fifo)
            ready_fd = os.open(fifo, os.O_RDWR | os.O_NONBLOCK)
            grandchild = (
                "import os,sys; "
                "f=open(sys.argv[1],'w'); f.write(str(os.getppid())+' '+str(os.getpid())+'\\n'); f.flush(); "
                "r,w=os.pipe(); os.read(r,1)"
            )
            child = (
                "import os,subprocess,sys; "
                f"subprocess.Popen([sys.executable,'-c',sys.argv[1],sys.argv[2]], start_new_session={detached}); "
                "r,w=os.pipe(); os.read(r,1)"
            )
            launcher = (
                "import sys; from pathlib import Path; "
                "from tools.harness import __main__ as cli, profiles; "
                "cli.ARTIFACT_ROOT=Path(sys.argv[1]); "
                "profiles.PROFILES['test']={'scope':'harness','prerequisites':profiles._NONE, "
                "'run':lambda ctx: profiles._run_command([sys.executable,'-c',sys.argv[2],sys.argv[3],sys.argv[4]],ctx,'stage.log')}; "
                "sys.exit(cli.cmd_run(['test','--timeout-seconds',sys.argv[5]]))"
            )
            proc = subprocess.Popen(
                [sys.executable, '-c', launcher, tmp, child, grandchild, str(fifo),
                 '1' if stop_signal is None else '60'],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True,
            )
            pid = None
            stage_pid = None
            pidfd = None
            try:
                readable, _, _ = select.select([ready_fd], [], [], 10)
                self.assertTrue(readable, 'grandchild did not start')
                stage_pid, pid = map(int, os.read(ready_fd, 64).split())
                pidfd = os.pidfd_open(pid)
                if stop_signal is not None:
                    proc.send_signal(stop_signal)
                self.assertNotEqual(proc.wait(timeout=5), 0)
                exited, _, _ = select.select([pidfd], [], [], 5)
                self.assertTrue(exited, 'grandchild survived harness exit')
                receipt = json.loads(next(root.glob('*/result.json')).read_text())
                self.assertEqual(receipt['status'], 'timeout' if stop_signal is None else 'interrupted')
                if stop_signal is not None:
                    self.assertIn(signal.Signals(stop_signal).name, receipt['error'])
                self.assertEqual(len(receipt['commands']), 1)
                self.assertIsNotNone(receipt['exits'][0])
            finally:
                if proc.poll() is None:
                    os.killpg(proc.pid, signal.SIGKILL)
                    proc.wait()
                if stage_pid is not None:
                    try:
                        os.killpg(stage_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                if pid is not None:
                    try:
                        os.kill(pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                if pidfd is not None:
                    os.close(pidfd)
                os.close(ready_fd)

    def test_stage_timeout_reaps_tree_and_writes_receipt(self):
        self.run_case(None)

    def test_timeout_reaps_detached_grandchild(self):
        self.run_case(None, detached=True)

    def test_sigterm_reaps_tree_and_writes_receipt(self):
        self.run_case(signal.SIGTERM)

    def test_sighup_reaps_tree_and_writes_receipt(self):
        self.run_case(signal.SIGHUP)

    def test_sigint_reaps_tree_and_writes_receipt(self):
        self.run_case(signal.SIGINT)
