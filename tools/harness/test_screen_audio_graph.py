"""The private PipeWire graph wrapper owns its scratch directory.

Regression: `tools/harness/screen_audio_graph.sh` created `/tmp/solaris-pw.*`
and then `exec`'d `dbus-run-session`. Replacing the shell process discards the
EXIT trap, so every harness run left a full private graph directory behind
(650 of them on the shared host, ~19k files each).
"""
from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import runtime

SCRIPT = runtime.REPO_ROOT / 'tools' / 'harness' / 'screen_audio_graph.sh'
FAKES = {
    'dbus-run-session': '#!/bin/sh\n[ "$1" = "--" ] && shift\nexec "$@"\n',
    'pipewire': f'''#!{sys.executable}
import os, signal, socket
s = socket.socket(socket.AF_UNIX)
s.bind(os.environ["PIPEWIRE_RUNTIME_DIR"] + "/pipewire-0")
s.listen()
signal.pause()
''',
    'wireplumber': '''#!/bin/sh
if [ -f "$KRASNOKAMEN_PW_ROOT/registry-ready" ]; then
  echo sm-settings >"$KRASNOKAMEN_PW_ROOT/wp-result"
else
  echo "Failed to connect to PipeWire" >"$KRASNOKAMEN_PW_ROOT/wp-result"
fi
''',
    'pw-mon': f'''#!{sys.executable}
import os
from pathlib import Path
from tools.harness.runtime import wait_for_path_condition
p = Path(os.environ["KRASNOKAMEN_PW_ROOT"]) / "wp-result"
print(wait_for_path_condition(p, 5, lambda: (p.exists(), p.read_text() if p.exists() else "")))
''',
    'pw-dump': '#!/bin/sh\ntouch "$KRASNOKAMEN_PW_ROOT/registry-ready"\necho "node.name: test-null"\n',
}


class PrivateGraphCleanupTests(unittest.TestCase):
    def private_graph_run(self, command, overrides=None, *, scratch_name='scratch'):
        directory = tempfile.TemporaryDirectory(dir='/tmp')
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        scratch = root / scratch_name
        scratch.mkdir()
        bin_dir = root / 'bin'
        bin_dir.mkdir()
        for name, body in (FAKES | (overrides or {})).items():
            path = bin_dir / name
            path.write_text(body)
            path.chmod(0o755)
        host_runtime = root / 'runtime'  # no systemd/private: nothing to link
        host_runtime.mkdir()
        env = dict(os.environ, PATH=f'{bin_dir}:{os.environ["PATH"]}',
                   SA_SCRATCH=str(scratch), XDG_RUNTIME_DIR=str(host_runtime),
                   PYTHONPATH=str(runtime.REPO_ROOT))
        env.pop('KRASNOKAMEN_AUDIO_RECEIPT_DIR', None)
        result = subprocess.run(['sh', str(SCRIPT), *command], env=env,
                                capture_output=True, text=True, timeout=120)
        return result, scratch

    def test_long_explicit_scratch_is_rejected_before_daemons(self):
        result, scratch = self.private_graph_run(
            ['sh', '-c', 'echo CHILD-RAN'],
            {'dbus-run-session': '#!/bin/sh\necho DAEMON-STARTED\n'},
            scratch_name='x' * 100)
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertIn('private audio runtime path is too long', result.stderr)
        self.assertNotIn('DAEMON-STARTED', result.stdout)
        self.assertEqual(list(scratch.iterdir()), [])

    def test_long_tmpdir_keeps_socket_fixture_short(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as directory:
            long_tmp = Path(directory) / ('x' * 100)
            long_tmp.mkdir()
            with mock.patch.dict(os.environ, {'TMPDIR': str(long_tmp)}), \
                 mock.patch.object(tempfile, 'tempdir', str(long_tmp)):
                result, _ = self.private_graph_run(['true'])
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_private_graph_directory_is_gone_when_the_run_ends(self):
        probe = ('test -d "$KRASNOKAMEN_PW_ROOT" && '
                 'case "$KRASNOKAMEN_PW_ROOT" in */solaris-pw.*) exit 0;; *) exit 9;; esac')
        result, scratch = self.private_graph_run(['sh', '-c', probe])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('preflight: no alsa_*/bluez_* objects in the private graph',
                      result.stderr)
        self.assertEqual(list(scratch.iterdir()), [],
                         'the private graph directory survived the run')

    def test_wrapped_command_status_survives_the_cleanup(self):
        result, scratch = self.private_graph_run(['sh', '-c', 'exit 7'])
        self.assertEqual(result.returncode, 7, result.stderr)
        self.assertEqual(list(scratch.iterdir()), [])

    def test_wireplumber_starts_only_after_pipewire_registry(self):
        result, _ = self.private_graph_run(
            ['sh', '-c', 'grep -qx sm-settings "$KRASNOKAMEN_PW_ROOT/wp-result"'])
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_missing_wireplumber_settings_fails_before_child(self):
        result, _ = self.private_graph_run(
            ['echo', 'CHILD-RAN'], {'pw-mon': '#!/bin/sh\nexit 1\n'})
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertIn('WirePlumber did not publish sm-settings', result.stderr)
        self.assertNotIn('CHILD-RAN', result.stdout)

    def test_failed_socket_wait_fails_before_wireplumber_and_child(self):
        result, _ = self.private_graph_run(
            ['echo', 'CHILD-RAN'], {'python3': '#!/bin/sh\nexit 1\n'})
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertIn('PipeWire socket did not become ready', result.stderr)
        self.assertNotIn('CHILD-RAN', result.stdout)

    def test_failed_registry_wait_fails_before_child(self):
        result, _ = self.private_graph_run(
            ['echo', 'CHILD-RAN'], {'pw-dump': '#!/bin/sh\nexit 1\n'})
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertIn('PipeWire registry is unavailable', result.stderr)
        self.assertNotIn('CHILD-RAN', result.stdout)


if __name__ == '__main__':
    unittest.main()
