"""A failed screens-voice-qa leg keeps its verdicts in the receipt.

The ledger reads a client row's negative control from a receipt's verdicts: a leg that failed at one
verdict must report that verdict as false, not as unreported (wave 2, HC-1). The profile runs its legs
inside a private audio graph; these tests drive the child entry and the parent's adoption of the
child's result without starting any graph, client or server.
"""
from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.harness.backends import screens_voice_qa as svq
from tools.harness.profiles import ProfileContext


class FailedLegReceiptTests(unittest.TestCase):
    def artifact(self) -> Path:
        directory = Path(tempfile.mkdtemp(prefix='svq-receipt-'))
        self.addCleanup(lambda: __import__('shutil').rmtree(directory, ignore_errors=True))
        return directory

    def run_main(self, artifact: Path) -> int:
        return svq._main([str(artifact), '60', str(artifact / 'plugins')])

    def test_a_failed_leg_keeps_the_verdicts_its_run_wrote(self):
        artifact = self.artifact()
        (artifact / 'verdicts.json').write_text(json.dumps({'native_voice_resets_on_non_voice_server': False}))

        def failed_run(_ctx):
            # `run` writes verdicts.json before refusing a failed leg.
            raise RuntimeError("screens-voice-qa verdicts failed: ['native_voice_resets_on_non_voice_server']")

        with patch.object(svq, 'run', failed_run):
            code = self.run_main(artifact)
        self.assertEqual(code, 1)
        result = json.loads((artifact / 'result.json').read_text())
        self.assertIn('native_voice_resets_on_non_voice_server', result['error'])
        self.assertEqual(result['details']['verdicts'], {'native_voice_resets_on_non_voice_server': False})

    def test_a_passed_leg_keeps_its_verdicts_unchanged(self):
        artifact = self.artifact()
        with patch.object(svq, 'run', lambda _ctx: {'verdicts': {'native_voice_resets_on_non_voice_server': True}}):
            code = self.run_main(artifact)
        self.assertEqual(code, 0)
        result = json.loads((artifact / 'result.json').read_text())
        self.assertIsNone(result['error'])
        self.assertEqual(result['details'], {'verdicts': {'native_voice_resets_on_non_voice_server': True}})

    def test_the_parent_adopts_a_failed_childs_details(self):
        artifact = self.artifact()
        inner = artifact / 'private-graph'

        class Finished:
            returncode = 1

            def wait(self, timeout=None):
                return 1

        def popen(_command, **_kwargs):
            inner.mkdir(exist_ok=True)
            (inner / 'result.json').write_text(json.dumps({
                'details': {'verdicts': {'native_voice_resets_on_non_voice_server': False}},
                'error': "RuntimeError: screens-voice-qa verdicts failed: "
                         "['native_voice_resets_on_non_voice_server']",
                'commands': [], 'cwds': [], 'exits': [1], 'logs': [], 'client_evidence': {},
            }))
            return Finished()

        ctx = ProfileContext(artifact_dir=artifact, client='native', windowed=True, timeout_seconds=60,
                             plugins_dir=artifact / 'plugins')
        with patch.object(svq.subprocess, 'Popen', popen):
            with self.assertRaises(RuntimeError):
                svq._run_in_private_graph(ctx, 60)
        self.assertEqual(ctx.client_evidence['verdicts'],
                         {'native_voice_resets_on_non_voice_server': False})


class ConsentDialogDetectorTests(unittest.TestCase):
    @staticmethod
    def state(title: str, body: str, choice: str) -> dict:
        return {'ui_nodes': [
            {'id': 'desktop/title', 'label': title},
            {'id': 'desktop/body/0', 'label': 'SvcSpeaker wants to view your screens.'},
            {'id': 'desktop/body/1', 'label': body},
            {'id': 'desktop/choice/0', 'label': 'Deny'},
            {'id': 'desktop/choice/1', 'label': choice},
        ]}

    def test_both_wordings_read_as_the_same_dialog(self):
        fixed = self.state('Allow screen and audio sharing?', 'Includes app audio, only while looking.',
                           'Allow all my open screens and their app audio')
        prefix = self.state('Allow desktop viewing?', 'Pictures only, while looking.',
                            'Allow viewing all my open screens')
        self.assertTrue(svq.consent_dialog_open(fixed))
        self.assertTrue(svq.consent_dialog_open(prefix))

    def test_no_dialog_is_not_open(self):
        self.assertFalse(svq.consent_dialog_open({'ui_nodes': []}))
        self.assertFalse(svq.consent_dialog_open({'ui_nodes': [{'id': 'desktop/title', 'label': 'x'}]}))


if __name__ == '__main__':
    unittest.main()
