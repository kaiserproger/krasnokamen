"""Full-nextest evidence must replace, not bypass, registered Cargo checks."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

from tools.harness import ledger, profiles, regressions


class L2CargoResultsTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.ctx = profiles.ProfileContext(self.root)
        self.report = self.root / 'test-junit.xml'
        ledger.write_rows(self.root, '''[[bug]]
id = "RESULT-A"
title = "observed failure A"
area = "harness"
repro = "cargo"
severity = "major"
status = "open"
source = "observed failure"
regression = ["cargo:mc-core:checks::observed"]

[[bug]]
id = "RESULT-B"
title = "observed failure B"
area = "harness"
repro = "cargo"
severity = "major"
status = "open"
source = "observed failure"
regression = ["cargo:mc-edge:checks::other"]
''')
        self.enterContext(patch.object(regressions, 'REPO_ROOT', self.root))
        command = self.enterContext(patch.object(
            profiles, '_run_command', side_effect=AssertionError('must not rerun Cargo')))
        self.addCleanup(command.assert_not_called)

    def write_report(self, first=None, *, omitted=False, package='mc-core', name='checks::observed'):
        # Nextest's documented Jenkins XML: one suite per package/binary and
        # full Rust test names, with failure/error/skipped children as applicable.
        root = ET.Element('testsuites')
        if not omitted:
            suite = ET.SubElement(root, 'testsuite', name=f'{package}::mc_core')
            case = ET.SubElement(suite, 'testcase', name=name)
            if first:
                ET.SubElement(case, first)
        suite = ET.SubElement(root, 'testsuite', name='mc-edge::mc_edge')
        ET.SubElement(suite, 'testcase', name='checks::other')
        ET.ElementTree(root).write(self.report, encoding='utf-8', xml_declaration=True)

    def test_all_registered_refs_pass_without_second_cargo_run(self):
        self.write_report()
        details = regressions.run(self.ctx, cargo_junit=self.report)
        self.assertEqual(details['cargo_checks_reused'], 2)

    def test_failed_ref_names_its_row(self):
        for outcome in ('failure', 'error'):
            with self.subTest(outcome=outcome):
                self.write_report(outcome)
                with self.assertRaisesRegex(RuntimeError, 'RESULT-A.*failed'):
                    regressions.run(self.ctx, cargo_junit=self.report)

    def test_missing_ref_names_its_row(self):
        self.write_report(omitted=True)
        with self.assertRaisesRegex(RuntimeError, 'RESULT-A.*not executed'):
            regressions.run(self.ctx, cargo_junit=self.report)

    def test_ignored_ref_names_its_row(self):
        self.write_report('skipped')
        with self.assertRaisesRegex(RuntimeError, 'RESULT-A.*skipped'):
            regressions.run(self.ctx, cargo_junit=self.report)

    def test_package_and_full_test_name_must_match(self):
        for changes in ({'package': 'other-crate'}, {'name': 'checks::observed_suffix'}):
            with self.subTest(changes=changes):
                self.write_report(**changes)
                with self.assertRaisesRegex(RuntimeError, 'RESULT-A.*not executed'):
                    regressions.run(self.ctx, cargo_junit=self.report)

    def test_missing_or_malformed_report_fails_closed(self):
        for content in (None, '<testsuites>', '<not-nextest/>'):
            with self.subTest(content=content):
                if content is not None:
                    self.report.write_text(content)
                with self.assertRaisesRegex(RuntimeError, 'RESULT-A.*RESULT-B.*JUnit'):
                    regressions.run(self.ctx, cargo_junit=self.report)

    def test_success_in_another_binary_does_not_hide_failure(self):
        self.write_report('failure')
        root = ET.parse(self.report).getroot()
        suite = ET.SubElement(root, 'testsuite', name='mc-core::other_binary')
        ET.SubElement(suite, 'testcase', name='checks::observed')
        ET.ElementTree(root).write(self.report)
        with self.assertRaisesRegex(RuntimeError, 'RESULT-A.*failed'):
            regressions.run(self.ctx, cargo_junit=self.report)

    def test_python_refs_still_execute(self):
        self.write_report()
        row = ledger.load_rows(self.root)[0]
        row['regression'].append('python:tools.harness.test_example:Example.test_case')
        with patch.object(ledger, 'load_rows', return_value=[row]), patch.object(
                ledger, '_run_python', return_value={
                    ('tools.harness.test_example', 'Example.test_case'): (ledger.GREEN, '')
                }) as execute:
            regressions.run(self.ctx, cargo_junit=self.report)
        execute.assert_called_once_with(
            self.ctx, {('tools.harness.test_example', 'Example.test_case')}, self.root)


class L2ReportCaptureTests(unittest.TestCase):
    def test_full_run_discards_stale_reports_and_captures_failed_run(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifacts = root / 'artifacts'
            artifacts.mkdir()
            source = root / 'relative-target/nextest/default/junit.xml'
            source.parent.mkdir(parents=True)
            source.write_text('stale source')
            report = artifacts / 'test-junit.xml'
            report.write_text('stale artifact')

            def failed_run(*args, **kwargs):
                self.assertFalse(source.exists())
                self.assertFalse(report.exists())
                source.write_text('<testsuites/>')
                raise RuntimeError('nextest failed')

            with patch.object(profiles, 'REPO_ROOT', root), patch.dict(
                    'os.environ', {'CARGO_TARGET_DIR': 'relative-target'}), patch.object(
                    profiles, '_run_command', side_effect=failed_run):
                with self.assertRaisesRegex(RuntimeError, 'nextest failed'):
                    profiles.run_test(profiles.ProfileContext(artifacts), full=True)
            self.assertEqual(report.read_text(), '<testsuites/>')

    def test_full_run_cannot_opt_into_ignored_tests(self):
        with tempfile.TemporaryDirectory() as directory:
            for args in (['--run-ignored', 'all'], ['--run-ignored=only']):
                with self.subTest(args=args), patch.object(profiles, '_run_command') as command:
                    with self.assertRaisesRegex(ValueError, 'non-ignored'):
                        profiles.run_test(profiles.ProfileContext(Path(directory), extra_args=args), full=True)
                    command.assert_not_called()

    def test_correctness_consumes_full_run_and_preserves_failure_row(self):
        with tempfile.TemporaryDirectory() as directory:
            ctx = profiles.ProfileContext(Path(directory))
            calls = []

            def full_run(context, *, full):
                self.assertTrue(full)
                calls.append('test')
                raise RuntimeError('nextest failed')

            def consume(context, *, cargo_junit):
                self.assertEqual(cargo_junit, ctx.artifact_dir / 'test-junit.xml')
                calls.append('regressions')
                raise RuntimeError('RESULT-A: failed')

            with patch.object(profiles, 'run_fmt', return_value={}), patch.object(
                    profiles, 'run_clippy', return_value={}), patch.object(
                    profiles, 'run_code_health', return_value={}), patch.object(
                    profiles, 'run_test', side_effect=full_run), patch.object(
                    profiles, 'run_regressions', side_effect=consume):
                with self.assertRaisesRegex(RuntimeError, 'nextest failed.*RESULT-A'):
                    profiles.run_correctness(ctx)
            self.assertEqual(calls, ['test', 'regressions'])
