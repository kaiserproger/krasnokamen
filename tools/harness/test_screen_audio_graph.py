"""The private PipeWire graph wrapper owns its scratch directory.

Regression: `tools/harness/screen_audio_graph.sh` created `/tmp/solaris-pw.*`
and then `exec`'d `dbus-run-session`. Replacing the shell process discards the
EXIT trap, so every harness run left a full private graph directory behind
(650 of them on the shared host, ~19k files each).
"""
from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from . import runtime

SCRIPT = runtime.REPO_ROOT / 'tools' / 'harness' / 'screen_audio_graph.sh'
FAKES = {
    'dbus-run-session': '#!/bin/sh\n[ "$1" = "--" ] && shift\nexec "$@"\n',
    'pipewire': '#!/bin/sh\nexit 0\n',
    'wireplumber': '#!/bin/sh\nexit 0\n',
    'pipewire-pulse': '#!/bin/sh\nexit 0\n',
    'pw-mon': '#!/bin/sh\necho "sm-settings"\n',
    'pw-dump': '#!/bin/sh\necho "node.name: test-null"\n',
}


class PrivateGraphCleanupTests(unittest.TestCase):
    def private_graph_run(self, command):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        scratch = root / 'scratch'
        scratch.mkdir()
        bin_dir = root / 'bin'
        bin_dir.mkdir()
        for name, body in FAKES.items():
            path = bin_dir / name
            path.write_text(body)
            path.chmod(0o755)
        host_runtime = root / 'runtime'  # no systemd/private: nothing to link
        host_runtime.mkdir()
        env = dict(os.environ, PATH=f'{bin_dir}:{os.environ["PATH"]}',
                   SA_SCRATCH=str(scratch), XDG_RUNTIME_DIR=str(host_runtime))
        env.pop('KRASNOKAMEN_AUDIO_RECEIPT_DIR', None)
        result = subprocess.run(['sh', str(SCRIPT), *command], env=env,
                                capture_output=True, text=True, timeout=120)
        return result, scratch

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


if __name__ == '__main__':
    unittest.main()
