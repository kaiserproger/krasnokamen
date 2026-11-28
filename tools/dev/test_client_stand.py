"""Regression for the sourced client stand (owner D153).

Sourcing `tools/dev/client-stand` must not change the caller's shell options and
must not kill the caller when a path is missing: it returns 1 so the lane can
handle an unavailable stand.
"""

import os
import pathlib
import subprocess
import unittest

STAND = pathlib.Path(__file__).with_name("client-stand")


def run(script: str, workspace: str | None = None) -> subprocess.CompletedProcess:
    env = dict(os.environ)
    if workspace is None:
        env.pop("SOLARIS_WORK", None)
    else:
        env["SOLARIS_WORK"] = workspace
    return subprocess.run(["bash", "-c", script], env=env, capture_output=True, text=True, timeout=120)


class ClientStandSourcedTests(unittest.TestCase):
    def test_sourcing_keeps_options_and_continues_on_working_stand(self):
        result = run(
            f'. "{STAND}" || exit $?; '
            'printf "after=%s bin=%s\\n" "$-" "$KRASNOKAMEN_CLIENT_BINARY"; '
            'echo caller-survived'
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("caller-survived", result.stdout)
        self.assertIn("bin=/run/media/kaiserroman/games/solaris-work/pins/", result.stdout)
        self.assertNotIn("ehuBc", result.stdout.split("after=")[1])

    def test_sourcing_returns_one_and_keeps_the_caller_on_a_missing_path(self):
        result = run(
            'before=$-; '
            f'. "{STAND}"; rc=$?; after=$-; '
            'printf "rc=%s before=%s after=%s\\n" "$rc" "$before" "$after"; '
            'echo caller-survived'
        , workspace="/nonexistent-client-stand")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("caller-survived", result.stdout)
        self.assertIn("rc=1", result.stdout)
        before = result.stdout.split("before=")[1].split()[0]
        after = result.stdout.split("after=")[1].split()[0]
        self.assertEqual(before, after, "sourcing changed the caller's shell options")
        self.assertIn("is missing at", result.stderr)

    def test_direct_execution_fails_loudly(self):
        result = subprocess.run(
            ["bash", str(STAND)],
            env={**os.environ, "SOLARIS_WORK": "/nonexistent-client-stand"},
            capture_output=True,
            text=True,
            timeout=120,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("is missing at", result.stderr)


if __name__ == "__main__":
    unittest.main()
