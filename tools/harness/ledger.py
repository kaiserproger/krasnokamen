"""Bug-ledger tools: ``ledger probe`` classifies rows on the current tree and
``ledger promote`` is the only way a row becomes ``fixed``.

Both reuse the check construction of ``regressions`` (one source of truth for what
a registered ref runs and what its receipt must show).
"""
from __future__ import annotations

import argparse
from collections import defaultdict
import contextlib
import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib

from . import regressions
from .runtime import REPO_ROOT, main_checkout, sibling_root

GREEN, RED, UNRUNNABLE, NEEDS_CLIENT = 'green', 'red', 'unrunnable', 'needs-real-client'
ARTIFACT_ROOT = REPO_ROOT / '.analysis' / 'validation'


class Refused(Exception):
    """A promotion precondition failed; the message is the stated reason."""


# --------------------------------------------------------------------- ledger


def _load_bugs(root):
    with (root / 'docs/bugs/ledger.toml').open('rb') as file:
        bugs = tomllib.load(file)['bug']
    return bugs


def _git(root, *args, stdin=None, check=True):
    done = subprocess.run(['git', '-C', str(root), *args], input=stdin, capture_output=True, check=False)
    if check and done.returncode:
        raise Refused(f"git {' '.join(args[:3])} failed: {done.stderr.decode(errors='replace').strip()}")
    return done.returncode, done.stdout.decode(errors='surrogateescape')


def _artifact_ctx(kind):
    from .profiles import ProfileContext

    ARTIFACT_ROOT.mkdir(parents=True, exist_ok=True)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S')
    return ProfileContext(Path(tempfile.mkdtemp(prefix=f'{stamp}-ledger-{kind}-', dir=ARTIFACT_ROOT)))


# ------------------------------------------------------------------ running refs


def _exec(ctx, argv, tag, tree, env):
    """Run one command in `tree`; return (exited_zero, log_text)."""
    from . import profiles

    name = f'ledger-{len(ctx.logs)}-{tag}.log'
    try:
        profiles._run_command(argv, ctx, name, cwd=tree, env=env)
        ok = True
    except RuntimeError:
        ok = False
    log = ctx.artifact_dir / name
    return ok, log.read_text(errors='replace') if log.exists() else ''


_RESULT_LINE = re.compile(r'^\s+([A-Z][A-Z ]*?)\s+\[\s*[\d.]+s\]\s+(?:\(\s*\d+/\d+\)\s+)?(\S+)\s+(\S+)\s*$', re.MULTILINE)


def _run_cargo(ctx, tests, tree):
    """Run exact cargo tests {(crate, name)} in one nextest invocation -> {(crate, name): (state, detail)}."""
    expression = ' | '.join(f'(package({crate}) & test(={name}))' for crate, name in sorted(tests))
    package_args = [arg for crate in sorted({crate for crate, _ in tests}) for arg in ('-p', crate)]
    argv, env = regressions.tree_build_command(tree, ['cargo', 'nextest', 'list', *package_args, '-E', expression, '--message-format', 'json'])
    ok, text = _exec(ctx, argv, 'list', tree, env)
    try:
        listing = regressions._nextest_listing(text)
    except ValueError:
        reason = 'nextest list produced no listing (build error?); see log' if not ok else 'nextest list produced no listing'
        return {test: (UNRUNNABLE, reason) for test in tests}
    found = {(suite['package-name'], name) for suite in listing['rust-suites'].values()
             for name, result in suite.get('testcases', {}).items()
             if result.get('filter-match', {}).get('status') == 'matches' and not result.get('ignored', False)}
    results = {test: (UNRUNNABLE, 'unknown or ignored test') for test in tests - found}
    runnable = tests & found
    if not runnable:
        return results
    expression = ' | '.join(f'(package({crate}) & test(={name}))' for crate, name in sorted(runnable))
    argv, env = regressions.tree_build_command(tree, ['cargo', 'nextest', 'run', *package_args, '-E', expression, '--ignore-default-filter',
                                                      '--no-fail-fast', '--no-tests=fail', '--status-level', 'pass'])
    ok, text = _exec(ctx, argv, 'run', tree, env)
    seen = {}
    for status, binary, name in _RESULT_LINE.findall(text):
        seen[(binary.split('::', 1)[0], name)] = status
    for test in runnable:
        status = seen.get(test)
        if status is None:
            results[test] = (UNRUNNABLE, 'no result line (build error or run aborted); see log' if not ok else 'no result line')
        elif status in ('PASS', 'LEAK PASS'):
            results[test] = (GREEN, '')
        else:
            results[test] = (RED, status)
    return results


def _client_test_target(packages, crate):
    """One unambiguous Cargo unit-test target for a registered client package."""
    matches = [package for package in packages if package.get('name') == crate]
    if len(matches) != 1:
        raise ValueError(f'client package {crate}: unknown or ambiguous package in cargo metadata')
    targets = [target for target in matches[0]['targets'] if target.get('test', True)]
    libraries = [target for target in targets if set(target['kind']) & {
        'lib', 'rlib', 'dylib', 'cdylib', 'staticlib', 'proc-macro'}]
    if len(libraries) == 1:
        return ['--lib']
    binaries = [target for target in targets if target['kind'] == ['bin']]
    if not libraries and len(binaries) == 1:
        return ['--bin', binaries[0]['name']]
    raise ValueError(f'client package {crate}: no unambiguous lib or bin unit-test target in cargo metadata')


def _run_client(ctx, tests, root):
    """Run exact client-crate tests {(crate, name)} in the client checkout -> {(crate, name): (state, detail)}.

    A `client:<crate>:<test>` ref is a fix in the client repository: it never builds here, so the
    checkout is `runtime.sibling_root('client')` (KRASNOKAMEN_CLIENT_ROOT or the sibling repo)."""
    # Cargo metadata is one JSON line; mem-run can prepend its scope diagnostic.
    repo = client_repo(root)
    if not repo.is_dir():
        return {test: (UNRUNNABLE, f'no client checkout at {repo} (set KRASNOKAMEN_CLIENT_ROOT)') for test in tests}
    argv, env = regressions.tree_build_command(
        repo, ['cargo', 'metadata', '--no-deps', '--format-version', '1'])
    ok, text = _exec(ctx, argv, 'client-metadata', repo, env)
    try:
        metadata = next(json.loads(line) for line in text.splitlines() if line.startswith('{'))
        if not ok or not isinstance(metadata.get('packages'), list):
            raise ValueError('cargo metadata failed or omitted packages')
        targets = {}
        errors = {}
        for crate in sorted({crate for crate, _ in tests}):
            try:
                targets[crate] = _client_test_target(metadata['packages'], crate)
            except ValueError as error:
                errors[crate] = str(error)
    except (ValueError, StopIteration, KeyError, TypeError, AttributeError):
        return {test: (UNRUNNABLE, f'client package {test[0]}: cargo metadata failed or was invalid; see log')
                for test in tests}
    results = {}
    for crate, name in sorted(tests):
        if crate in errors:
            results[(crate, name)] = (UNRUNNABLE, errors[crate])
            continue
        # A ref may name one test or a whole module (`ui::toasts::tests`): cargo's filter selects it
        # either way, and `--exact` would find nothing for the module form. Green needs at least one
        # selected test and no failure, so a ref can never pass by selecting nothing.
        argv, env = regressions.tree_build_command(
            repo, ['cargo', 'test', '-p', crate, *targets[crate], '--', name])
        ok, text = _exec(ctx, argv, f'client-{name.rsplit("::", 1)[-1][:24]}', repo, env)
        summary = re.search(r'test result: (\w+)\. (\d+) passed; (\d+) failed', text)
        if summary is None:
            results[(crate, name)] = (UNRUNNABLE, 'no test summary (build error?)')
            continue
        passed, failed = int(summary.group(2)), int(summary.group(3))
        if passed + failed == 0:
            results[(crate, name)] = (UNRUNNABLE, 'ran no matching test')
        elif failed or not ok:
            results[(crate, name)] = (RED, f'{failed} failed of {passed + failed}')
        else:
            results[(crate, name)] = (GREEN, '')
    return results


def _run_python(ctx, tests, tree):
    """Run exact unittest targets {(module, dotted test)} in `tree` -> {(module, test): (state, detail)}.

    A tools/** fix has no cargo check and no windowed profile; its own unittest is the guard. The
    module is imported from `tree` so the run is about that candidate, not the invoking checkout."""
    results = {}
    for module, test in sorted(tests):
        argv = [sys.executable, '-m', 'unittest', f'{module}.{test}']
        ok, text = _exec(ctx, argv, f'python-{test.rsplit(".", 1)[-1][:24]}', tree, None)
        summary = re.search(r'(?m)^Ran (\d+) tests? in ', text)
        skipped = re.search(r'\bskipped=(\d+)', text)
        count = int(summary.group(1)) if summary else 0
        skipped_count = int(skipped.group(1)) if skipped else 0
        if not summary or count <= skipped_count or re.search(r'(?m)^ERROR:', text):
            results[(module, test)] = (UNRUNNABLE, 'no executed test or unittest error; see log')
        elif ok and re.search(r'(?m)^OK(?: \(|$)', text):
            results[(module, test)] = (GREEN, '')
        elif not ok and re.search(r'(?m)^FAIL:', text):
            results[(module, test)] = (RED, 'test assertion failed')
        else:
            results[(module, test)] = (UNRUNNABLE, 'no conclusive unittest result; see log')
    return results


def _failing_verdict(receipt, profile, verdict):
    """A failed profile run is red for `verdict` when the receipt reports it unmet, or, for a
    verdict the profile cannot report on failure, when its own assertion failed:

    - `all` of a profile without a verdict map (autocraft-qa, village-autonomy-qa);
    - a buildings-qa `<blueprint>.L<n>_<behaviour>` never recorded because a
      same-kind, same-level assertion in the behavioural allowlist failed.
      Setup failures, observation races, unknown steps and other levels are unrunnable."""
    try:
        regressions._profile_verdict(receipt, profile, verdict)
    except regressions.VerdictNotMet as error:
        if error.value is not None:
            return True
        if profile != 'buildings-qa':
            return False
        from .backends.buildings_qa import NEGATIVE_CONTROL_STEPS

        blueprint, _, behaviour = verdict.partition('.')
        if profile == 'buildings-qa' and verdict == 'house.L1_broken_cell_repaired':
            return _assertion_failure(receipt) == 'buildings_house_L1_broken_cell_repaired'
        level = re.match(r'L\d+_', behaviour)
        if level is None:
            return False
        prefix = f'buildings_{blueprint}_{level.group(0)}'
        assertion = _assertion_failure(receipt) or ''
        return assertion.startswith(prefix) and assertion[len(prefix):] in NEGATIVE_CONTROL_STEPS
    return verdict == 'all' and _assertion_failure(receipt) is not None


# Assertion names that report a lost or late connection, not the scenario's own check.
_CONNECTION_ASSERTION = re.compile(r'(^|_)(in_play|entered_play|join(ed)?)(_|$)')
_PRE_SCENARIO_PHASES = frozenset({'preflight', 'server_start', 'client_start', 'join'})


def _assertion_failure(receipt):
    """The name of the scenario assertion a failed receipt tripped, or None. Read from the
    receipt's `error` only, never from the command line that ran it. A connection assertion
    (`in_play`, `*_entered_play`, join) or a failure tagged before the scenario body is a flake."""
    if not isinstance(receipt, dict):
        return None
    details = receipt.get('details') if isinstance(receipt.get('details'), dict) else {}
    failure = details.get('failure') if isinstance(details.get('failure'), dict) else {}
    if failure.get('phase') in _PRE_SCENARIO_PHASES:
        return None
    match = re.search(r'\bassertion failed: (\S+)', str(receipt.get('error') or ''))
    if match is None or _CONNECTION_ASSERTION.search(match.group(1)):
        return None
    return match.group(1)


def _scenario_assertion_failed(receipt, check):
    """A named-scenario ref is red when its own receipt (`details.named_scenario.id` is the
    ref) failed on one of the scenario's assertions."""
    if not isinstance(receipt, dict):
        return False
    details = receipt.get('details') if isinstance(receipt.get('details'), dict) else {}
    named = details.get('named_scenario')
    return isinstance(named, dict) and named.get('id') == check and _assertion_failure(receipt) is not None


def _loader_mc_version(loader_root: Path) -> str | None:
    """The Minecraft version the Loader checkout builds its client against."""
    properties = loader_root / 'gradle.properties'
    if not properties.is_file():
        return None
    match = re.search(r'^minecraftVersion=(\S+)$', properties.read_text(), re.M)
    return match.group(1) if match else None


def _loader_live_setup(ctx, env, tree) -> str | None:
    """Prepare env/placeholders for loader-live refs; return a fail-closed reason or None.

    loader-live launches the real Loader client through Gradle under a private audio
    graph, so it needs the loader checkout, the Gradle home with the cached client jar,
    an empty plugins dir and the vanilla translation data in `tree` (26.1.2 client)."""
    loader_root = sibling_root('loader')
    if not (loader_root / 'gradlew').is_file():
        return f'loader-live requires a Loader checkout at {loader_root} (set KRASNOKAMEN_LOADER_ROOT)'
    env['KRASNOKAMEN_LOADER_ROOT'] = str(loader_root)
    gradle_home = Path(env.get('GRADLE_USER_HOME') or (main_checkout().parent / 'loader-gradle-home'))
    if not gradle_home.is_dir():
        return f'loader-live requires a Gradle home at {gradle_home} (set GRADLE_USER_HOME)'
    env['GRADLE_USER_HOME'] = str(gradle_home)
    version = _loader_mc_version(loader_root)
    if not env.get('KRASNOKAMEN_CLIENT_JAR'):
        jar = gradle_home / 'caches' / 'fabric-loom' / (version or '') / 'minecraft-client.jar'
        if not jar.is_file():
            return f'loader-live requires the cached {version} client jar at {jar} (set KRASNOKAMEN_CLIENT_JAR)'
        env['KRASNOKAMEN_CLIENT_JAR'] = str(jar)
    wrapper = REPO_ROOT / 'tools' / 'harness' / 'screen_audio_graph.sh'
    if not wrapper.is_file():
        return f'loader-live requires the private audio wrapper {wrapper}'
    env['KRASNOKAMEN_AUDIO_RECEIPT_DIR'] = str(ctx.artifact_dir)
    plugins_dir = ctx.artifact_dir / 'loader-live-plugins'
    plugins_dir.mkdir(parents=True, exist_ok=True)
    data_source = Path(os.environ.get('KRASNOKAMEN_VANILLA_DATA_ROOT') or (main_checkout() / 'data'))
    for name in ('vanilla-steps', f'vanilla-{version or ""}'):
        source = data_source / name
        target = Path(tree) / 'data' / name
        if target.exists():
            continue
        if not source.is_dir():
            return f'loader-live requires {source} (vanilla translation data for the Loader client)'
        subprocess.run(['cp', '-a', '--', str(source), str(target)], check=True)
    return None


def _run_harness(ctx, refs, tree):
    """Run real-client refs in `tree` -> {ref: (state, detail, receipt path or None)}."""
    from . import profiles

    try:
        binary = regressions._release_server(ctx, profiles, tree)
    except Exception as error:
        return {ref: (UNRUNNABLE, f'release server: {error}', None) for ref in refs}
    _register_server(ctx.client_evidence['regressions_server']['sha256'], tree_fingerprint(tree))
    env = regressions._client_env(binary)
    if regressions.is_other_tree(tree):
        env.pop('CARGO_TARGET_DIR', None)  # nothing started for another tree may land in the caller's slot
    grouped = defaultdict(list)
    for ref in refs:
        _, profile, check = ref.split(':', 2)
        command, is_profile = regressions._check_command(profile, check)
        grouped[tuple(command)].append((ref, profile, check, is_profile))
    loader_refs = {ref for group in grouped.values() for ref, profile, _, _ in group if profile == 'loader-live'}
    loader_reason = None
    if loader_refs:
        loader_reason = _loader_live_setup(ctx, env, tree)
        if loader_reason is None:
            env.setdefault('KRASNOKAMEN_AUDIO_RECEIPT_DIR', str(ctx.artifact_dir))
    results = {}
    for command, group in grouped.items():
        _, profile, check, is_profile = group[0]
        command = list(command)
        if profile == 'loader-live':
            if loader_reason is not None:
                results.update({ref: (UNRUNNABLE, loader_reason, None) for ref, *_ in group})
                continue
            wrapper = REPO_ROOT / 'tools' / 'harness' / 'screen_audio_graph.sh'
            plugins_dir = ctx.artifact_dir / 'loader-live-plugins'
            command = [str(wrapper)] + [
                str(binary) if part == '{server_bin}' else str(plugins_dir) if part == '{plugins_dir}' else part
                for part in command
            ]
        try:
            receipt, path = regressions._client_receipt(ctx, profiles, command, env, profile, check, is_profile, cwd=tree)
        except regressions.ReceiptNotPassed as error:
            for ref, profile, check, is_profile in group:
                red = error.status == 'failed' and (
                    (is_profile and _failing_verdict(error.receipt, profile, check))
                    or (not is_profile and _scenario_assertion_failed(error.receipt, check))
                )
                results[ref] = (RED if red else UNRUNNABLE, str(error), str(error.path) if error.path else None)
            continue
        except Exception as error:
            results.update({ref: (UNRUNNABLE, str(error), None) for ref, *_ in group})
            continue
        for ref, profile, check, is_profile in group:
            try:
                if is_profile:
                    regressions._profile_verdict(receipt, profile, check)
                results[ref] = (GREEN, '', str(path))
            except RuntimeError as error:
                results[ref] = (RED, str(error), str(path))
    return results


def run_refs(ctx, refs, tree, client):
    """Run every ref of a row in `tree` -> {ref: {state, detail, receipt?}}.

    A failed client run is red only when its receipt carries an explicitly failing verdict for the
    referenced profile check. A timeout, a client that never reached play or any other failure
    without a negative verdict is unrunnable: it is not evidence about the check."""
    out, cargo, harness, python, client_tests = {}, set(), [], set(), set()
    for ref in refs:
        try:
            kind, first, second = regressions._parse_ref(ref)
        except ValueError as error:
            out[ref] = {'state': UNRUNNABLE, 'detail': str(error)}
            continue
        if kind == 'cargo':
            cargo.add((first, second))
        elif kind == 'python':
            python.add((first, second))
        elif kind == 'client':
            client_tests.add((first, second))
        elif client:
            harness.append(ref)
        else:
            out[ref] = {'state': NEEDS_CLIENT, 'detail': 'graphical ref; pass --client to run it'}
    with _one_off_build_env(tree):
        if cargo:
            for (crate, name), (state, detail) in _run_cargo(ctx, cargo, tree).items():
                out[f'cargo:{crate}:{name}'] = {'state': state, 'detail': detail}
        if harness:
            for ref, (state, detail, receipt) in _run_harness(ctx, harness, tree).items():
                out[ref] = {'state': state, 'detail': detail, **({'receipt': receipt} if receipt else {})}
        if python:
            for (module, test), (state, detail) in _run_python(ctx, python, tree).items():
                out[f'python:{module}:{test}'] = {'state': state, 'detail': detail}
        if client_tests:
            for (crate, name), (state, detail) in _run_client(ctx, client_tests, tree).items():
                out[f'client:{crate}:{name}'] = {'state': state, 'detail': detail}
    return out



def row_verdict(states):
    """One verdict for a row from its refs' states: red > green-only > needs-real-client > unrunnable."""
    if not states:
        return UNRUNNABLE, 'no registered check'
    if RED in states:
        return RED, ''
    if all(state == GREEN for state in states):
        return GREEN, ''
    if NEEDS_CLIENT in states:
        return NEEDS_CLIENT, ''
    return UNRUNNABLE, ''


# ------------------------------------------------------------------------ probe


def probe(bugs, ids, *, client, ctx, runner=run_refs, tree=REPO_ROOT):
    """Classify the rows `ids` (all given ids must exist) -> {id: {...}}; refs run once, shared by rows."""
    by_id = {bug['id']: bug for bug in bugs}
    unknown = [ident for ident in ids if ident not in by_id]
    if unknown:
        raise Refused(f"unknown ledger id: {', '.join(unknown)}")
    wanted = sorted({ref for ident in ids for ref in by_id[ident]['regression']})
    results = runner(ctx, wanted, tree, client) if wanted else {}
    rows = {}
    for ident in ids:
        bug = by_id[ident]
        refs = {ref: results[ref] for ref in bug['regression']}
        verdict, reason = row_verdict([entry['state'] for entry in refs.values()])
        if verdict == UNRUNNABLE and not reason:
            reason = '; '.join(f"{ref}: {entry['detail']}" for ref, entry in refs.items() if entry['state'] == UNRUNNABLE)
        if verdict == RED:
            reason = '; '.join(f"{ref}: {entry['detail'] or 'failed'}" for ref, entry in refs.items() if entry['state'] == RED)
        if verdict == NEEDS_CLIENT:
            reason = ', '.join(f"{ref} {entry['state']}" for ref, entry in refs.items() if entry['state'] != NEEDS_CLIENT)
        rows[ident] = {'verdict': verdict, 'reason': reason, 'status': bug['status'], 'refs': refs}
    return rows


def format_row(ident, row):
    suffix = f"({row['reason']})" if row['verdict'] == UNRUNNABLE else (f" [{row['reason']}]" if row['reason'] else '')
    return f"{ident}: {row['verdict']}{suffix}"


# ---------------------------------------------------------------------- promote

_TEST_PATH = re.compile(r'(^|/)(tests?|testdata|fixtures?)/|(^|/)[^/]*_tests?(\.rs|\.py|/)|(^|/)tests?\.rs$|(^|/)test_[^/]*\.py$')
_CFG_TEST_MOD = re.compile(r'^[ \t]*#\[cfg\(test\)\][ \t]*\n(?:[ \t]*#\[[^\n]*\][ \t]*\n)*[ \t]*(?:pub(?:\([^)]*\))? )?mod \w+\s*\{', re.MULTILINE)
_HUNK = re.compile(r'^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@')
_RUST_RAW_STRING = re.compile(r'(?:br|rb|r)(#*)"')
_RUST_CHAR = re.compile(r"(?:b)?'(?:\\.|[^'\\\n])'")


def _rust_test_ranges(source):
    """Line ranges of inline cfg(test) modules, ignoring braces in Rust literals and comments."""
    masked = list(source)
    pos = 0
    while pos < len(source):
        start = pos
        if source.startswith('//', pos):
            end = source.find('\n', pos)
            pos = len(source) if end < 0 else end
        elif source.startswith('/*', pos):
            pos += 2
            depth = 1
            while pos < len(source) and depth:
                if source.startswith('/*', pos):
                    depth += 1
                    pos += 2
                elif source.startswith('*/', pos):
                    depth -= 1
                    pos += 2
                else:
                    pos += 1
            if depth:
                raise Refused('unterminated Rust block comment while splitting test hunks')
        else:
            raw = _RUST_RAW_STRING.match(source, pos)
            char = _RUST_CHAR.match(source, pos)
            if raw:
                end = source.find('"' + raw.group(1), raw.end())
                if end < 0:
                    raise Refused('unterminated Rust raw string while splitting test hunks')
                pos = end + 1 + len(raw.group(1))
            elif char:
                pos = char.end()
            elif source[pos] == '"':
                pos += 1
                while pos < len(source) and source[pos] != '"':
                    pos += 2 if source[pos] == '\\' else 1
                if pos >= len(source):
                    raise Refused('unterminated Rust string while splitting test hunks')
                pos += 1
            else:
                pos += 1
                continue
        for index in range(start, pos):
            if source[index] != '\n':
                masked[index] = ' '
    code = ''.join(masked)
    ranges = []
    for match in _CFG_TEST_MOD.finditer(code):
        depth = 1
        pos = match.end()
        while pos < len(code) and depth:
            if code[pos] == '{':
                depth += 1
            elif code[pos] == '}':
                depth -= 1
            pos += 1
        if depth:
            raise Refused('unterminated Rust test module while splitting test hunks')
        ranges.append((source.count('\n', 0, match.start()) + 1,
                       source.count('\n', 0, pos - 1) + 1))
    return ranges


def _hunk_changed_lines(hunk):
    """New-side positions and whether each changed line is a deletion."""
    line = int(_HUNK.match(hunk[0]).group(3))
    changed = []
    for text in hunk[1:]:
        if text.startswith('+'):
            changed.append((line, False))
            line += 1
        elif text.startswith('-'):
            changed.append((line, True))
        elif text.startswith(' '):
            line += 1
    return changed


def non_test_patch(root, commit):
    """The fix commit's non-test hunks: (patch text, reversed files, kept test files)."""
    _, diff = _git(root, 'diff', '--no-renames', '--no-color', f'{commit}^1', commit)
    return _split_non_test(diff, lambda path: _git(root, 'show', f'{commit}:{path}')[1])


def _split_non_test(diff, source_of):
    """Split a diff into (patch text of its non-test hunks, reversed files, kept test files).

    Test hunks are those in test files or inside Rust inline `#[cfg(test)]` modules.
    A hunk straddling a module boundary refuses: it cannot be split.
    `source_of(path)` is the post-change text of a file.
    """
    patch, reversed_files, test_files = [], [], []
    for chunk in re.split(r'(?m)^(?=diff --git )', diff):
        if not chunk:
            continue
        lines = chunk.splitlines(keepends=True)
        match = re.match(r'diff --git a/(.+) b/(.+)$', lines[0].rstrip('\n'))
        if match is None:
            raise Refused(f'fix patch has an unparseable diff header (expected `diff --git a/<path> b/<path>`): {lines[0].rstrip()!r}')
        path = match.group(2)
        starts = [i for i, text in enumerate(lines) if text.startswith('@@ ')]
        if not starts:
            raise Refused(f'fix commit changes {path} without text hunks (binary or mode-only); cannot split test hunks')
        header = lines[:starts[0]]
        hunks = [lines[a:b] for a, b in zip(starts, starts[1:] + [len(lines)])]
        if path.startswith('docs/') or path.endswith('.md'):
            continue
        if _TEST_PATH.search(path):
            test_files.append(path)
            continue
        ranges = []
        if (path.endswith('.rs') and not any(text.startswith(('+++ /dev/null', 'new file mode')) for text in header)):
            ranges = _rust_test_ranges(source_of(path))
        kept = []
        for hunk in hunks:
            changed = _hunk_changed_lines(hunk)
            if any(deleted and any(line in (start, end + 1) for start, end in ranges)
                   for line, deleted in changed):
                raise Refused(f'a deletion in {path} touches a #[cfg(test)] module boundary; split the fix commit')
            in_test = [any(start <= line <= end for start, end in ranges) for line, _ in changed]
            if in_test and all(in_test):
                continue
            if any(in_test):
                raise Refused(f'a hunk of {path} straddles its #[cfg(test)] module; split the fix commit')
            kept.append(hunk)
        if kept:
            patch.extend(header)
            for hunk in kept:
                patch.extend(hunk)
            reversed_files.append(path)
        elif path.endswith('.rs'):
            test_files.append(path)
    return ''.join(patch), reversed_files, test_files


def _provision_data(root, tree):
    """Give the worktree its own copy of every vanilla cache of `root` (data/vanilla, 461 MB, and the
    older-release caches a Java 26.1.2 client joins through: data/vanilla-26.1.2 and the protocol steps
    data/vanilla-steps, a relative link into data/vanilla): a check that fails only because one is
    missing would be a false red control, or, for a Java client the server then refuses at the
    handshake, a run that waits out its deadline (ISSUE-1-25 probe, 2026-11-12: unrunnable after 1 h).
    A real copy, not hard links or a symlink to `root`: a check that writes in place must not change
    the source tree's data."""
    for source in sorted((root / 'data').glob('vanilla*')):
        target = tree / 'data' / source.name
        if target.exists() or target.is_symlink():
            continue
        (tree / 'data').mkdir(exist_ok=True)
        link = os.readlink(source) if source.is_symlink() else None
        if link is not None and not os.path.isabs(link):
            os.symlink(link, target)
        elif source.is_dir():
            subprocess.run(['cp', '-a', '--', str(source), str(target)], check=True)


def _worktree_parent(root):
    """Control worktrees live beside the other worktrees (`<work>/wt/`) on the work disk, not in /tmp."""
    return root.parent if root.parent.name == 'wt' else None


@contextlib.contextmanager
def _control_lock(root):
    """One promotion at a time: the control worktrees have fixed paths (so cargo reuses their build
    artifacts instead of growing the target dir by a new path per run) and the builds are heavy.
    Lock order is a build token, then this lock: the promote runs under mem-run (the owner host's
    memory cap and token), and every build stage's own mem-run reuses that ancestor token. A promote
    that took this lock without a token deadlocked with promotes holding tokens while they waited
    here, because its next stage needed one of their tokens (2026-11-12, 1 h host-wide stall)."""
    parent = _worktree_parent(root)
    if parent is None:
        yield
        return
    if not os.environ.get('KRASNOKAMEN_BUILD_TOKEN'):
        raise Refused('ledger promote must run under mem-run (no build token held): it takes a build '
                      'token first, then wt/ledger-control.lock, and its build stages reuse that token; '
                      'run <work>/solaris/tools/dev/mem-run.sh -m 12G python3 -m tools.harness ledger promote ...')
    with (parent / 'ledger-control.lock').open('w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield


@contextlib.contextmanager
def _worktree(root, commit, slot):
    wt = _worktree_parent(root)
    parent = wt if wt else Path(tempfile.mkdtemp(prefix='ledger-control-'))
    tree = parent / f'ledger-control-{slot}' if wt else parent / 'tree'
    if tree.exists():  # left by an interrupted run (the lock is held)
        _git(root, 'worktree', 'remove', '--force', str(tree), check=False)
        shutil.rmtree(tree, ignore_errors=True)
        _git(root, 'worktree', 'prune', check=False)
    try:
        _git(root, 'worktree', 'add', '--detach', str(tree), commit)
        _provision_data(root, tree)
        yield tree
    finally:
        _git(root, 'worktree', 'remove', '--force', str(tree), check=False)
        shutil.rmtree(tree, ignore_errors=True)
        if not wt:
            shutil.rmtree(parent, ignore_errors=True)


@contextlib.contextmanager
def _one_off_build_env(tree):
    """Builds in a control worktree are disposable: no incremental cache (it was 27 GB for ~70 paths)."""
    if Path(tree) == Path(REPO_ROOT):
        yield
        return
    previous = os.environ.get('CARGO_INCREMENTAL')
    os.environ['CARGO_INCREMENTAL'] = '0'
    try:
        yield
    finally:
        if previous is None:
            os.environ.pop('CARGO_INCREMENTAL', None)
        else:
            os.environ['CARGO_INCREMENTAL'] = previous


def _dirty_parts(root):
    """(binary diff of tracked changes, untracked file names) of the tree; docs/bugs and the ignored
    .analysis are not part of the candidate."""
    _, diff = _git(root, 'diff', 'HEAD', '--binary', '--', '.', ':!docs/bugs', ':!.analysis')
    _, others = _git(root, 'ls-files', '--others', '--exclude-standard', '--', '.', ':!docs/bugs', ':!.analysis')
    return diff, sorted(others.splitlines())


def tree_fingerprint(root):
    """Exact identity of the candidate: HEAD plus a hash of the uncommitted diff and untracked files."""
    _, head = _git(root, 'rev-parse', 'HEAD')
    diff, others = _dirty_parts(root)
    digest = hashlib.sha256(diff.encode(errors='surrogateescape'))
    for name in others:
        digest.update(name.encode(errors='surrogateescape'))
        digest.update((root / name).read_bytes() if (root / name).is_file() else b'')
    return {'head': head.strip(), 'dirty': bool(diff or others), 'sha256': digest.hexdigest()}


def _materialize(root, tree, expected):
    """Make `tree` (a worktree) exactly the candidate `expected`: its HEAD plus the uncommitted diff and
    untracked files of `root`; Refused when the result does not reproduce the fingerprint."""
    _git(tree, 'reset', '--hard', '--quiet', expected['head'])
    _git(tree, 'clean', '-fdq')
    _provision_data(root, tree)
    diff, others = _dirty_parts(root)
    if diff:
        _git(tree, 'apply', '--binary', '--whitespace=nowarn', '-', stdin=diff.encode(errors='surrogateescape'))
    for name in others:
        if (root / name).is_file():
            (tree / name).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(root / name, tree / name)
    got = tree_fingerprint(tree)
    if (got['head'], got['sha256']) != (expected['head'], expected['sha256']):
        raise Refused('the candidate worktree does not reproduce the tree being promoted (HEAD plus uncommitted state)')


def _register_server(sha256, tree):
    """Record which tree (fingerprint) a release server binary was built from by this tooling."""
    registry = REPO_ROOT / '.analysis/ledger-builds'
    registry.mkdir(parents=True, exist_ok=True)
    (registry / f'{sha256}.json').write_text(json.dumps({'sha256': sha256, 'tree': {'head': tree['head'], 'sha256': tree['sha256']}}) + '\n')


def _server_built_from(root, receipt_text, tree):
    """sha256 of the server binary a receipt names, when the tooling built it from exactly `tree`.

    The scenarios run whatever KRASNOKAMEN_SERVER_BINARY names (a pinned old build, a sibling's): the
    source fingerprint of the run says nothing about that binary, so its hash must be one that
    `_run_harness` registered for this candidate."""
    registry = root / '.analysis/ledger-builds'
    named = []
    for entry in sorted(registry.glob('*.json')) if registry.is_dir() else []:
        build = json.loads(entry.read_text())
        if build['sha256'] in receipt_text:
            named.append(build)
    if not named:
        raise Refused('client receipt names no server binary built from the candidate by the ledger tooling '
                      '(a pinned or foreign KRASNOKAMEN_SERVER_BINARY proves nothing about this source): '
                      'run `ledger probe --client` on the candidate and use its receipt')
    for build in named:
        if (build['tree']['head'], build['tree']['sha256']) == (tree['head'], tree['sha256']):
            return build['sha256']
    raise Refused(f"client receipt's server binary {named[0]['sha256'][:12]} was built from a different tree "
                  f"({named[0]['tree']['head'][:12]}), not the candidate")


def client_repo(root=REPO_ROOT):
    """The client checkout a `client:<sha>` fix_commit is resolved in (the sibling repo, or its env override)."""
    from . import runtime

    return runtime.sibling_root('client', root)


def client_fix_commit(fix, root=REPO_ROOT):
    """Full sha of a `client:<sha>` fix_commit in the client repository; Refused when it is unknown."""
    sha = fix[len('client:'):]
    if not re.fullmatch(r'[0-9a-f]{7,40}', sha):
        raise Refused(f'fix_commit {fix}: expected client:<sha>')
    repo = client_repo(root)
    if not repo.is_dir():
        raise Refused(f'fix_commit {fix}: no client checkout at {repo} (set KRASNOKAMEN_CLIENT_ROOT)')
    code, full = _git(repo, 'rev-parse', '--verify', '--quiet', f'{sha}^{{commit}}', check=False)
    if code:
        raise Refused(f'fix_commit {fix}: {sha} is not a commit of the client repository {repo}')
    return full.strip()


def client_pin(binary_path):
    """`{path, sha256, manifest_path, head, dirty}` of the client binary a run launches.

    The binary is hashed here, now, and bound to its source only through the pin-release manifest
    beside it: a checkout HEAD beside the binary says nothing about what that binary was built from.
    Missing pieces stay missing (`head`/`dirty` None, `error` says why) so promotion can refuse."""
    from . import runtime

    info = {'path': str(binary_path) if binary_path else None, 'sha256': None,
            'manifest_path': None, 'head': None, 'dirty': None}
    if not binary_path:
        info['error'] = 'KRASNOKAMEN_CLIENT_BINARY is not set: no client ran under this receipt'
        return info
    path = Path(binary_path).expanduser()
    if not path.is_absolute():
        path = runtime.sibling_root('client') / path
    info['path'] = str(path)
    try:
        with path.open('rb') as binary:
            info['sha256'] = hashlib.file_digest(binary, 'sha256').hexdigest()
    except OSError as error:
        info['error'] = f'client binary {path} unreadable: {error}'
        return info
    manifest = path.parent / 'manifest.json'
    info['manifest_path'] = str(manifest)
    try:
        pin = json.loads(manifest.read_text())
    except (OSError, ValueError) as error:
        info['error'] = f'no client pin manifest beside {path}: {error}'
        return info
    if pin.get('kind') != 'client' or pin.get('repo') != 'client':
        info['error'] = f'{manifest} is not a client pin (kind {pin.get("kind")!r}, repo {pin.get("repo")!r})'
        return info
    info['head'] = pin.get('head')
    info['dirty'] = pin.get('dirty')
    if pin.get('binary_sha256') != info['sha256']:
        info['error'] = (f'{manifest} pins a different binary ({str(pin.get("binary_sha256"))[:12]}) than the '
                         f'one that ran ({str(info["sha256"])[:12]})')
    return info


def _client_pin_of_receipt(root, receipt, path):
    """The pin head a receipt's client ran, Refused unless the receipt's pin is complete and clean."""
    recorded = receipt.get('client_binary')
    if not isinstance(recorded, dict):
        raise Refused(f'client receipt {path} records no client binary (the harness writes client_binary at '
                      'run start): rerun the gate on a pinned client')
    if recorded.get('error'):
        raise Refused(f'client receipt {path}: {recorded["error"]}')
    if not re.fullmatch(r'[0-9a-f]{64}', str(recorded.get('sha256', ''))):
        raise Refused(f'client receipt {path} records no sha256 of the client binary that ran')
    manifest_path = recorded.get('manifest_path')
    if not manifest_path:
        raise Refused(f'client receipt {path} records no client pin manifest: a binary without its source '
                      'pin proves nothing about the fix')
    try:
        pin = json.loads(Path(manifest_path).read_text())
    except (OSError, ValueError) as error:
        raise Refused(f'client receipt {path}: client pin manifest {manifest_path} unreadable: {error}')
    if pin.get('kind') != 'client' or pin.get('repo') != 'client':
        raise Refused(f'client receipt {path}: {manifest_path} is not a client pin')
    if pin.get('binary_sha256') != recorded['sha256']:
        raise Refused(f"client receipt {path}: the manifest pins binary {str(pin.get('binary_sha256'))[:12]}, "
                      f"the receipt ran {recorded['sha256'][:12]}: the pin does not describe the client that ran")
    if pin.get('dirty') is not False:
        raise Refused(f"client receipt {path}: client pin {manifest_path} is dirty ({pin.get('dirty')!r}); a "
                      'dirty client tree has no reliable source revision')
    head = pin.get('head')
    if not re.fullmatch(r'[0-9a-f]{40}', str(head or '')):
        raise Refused(f'client receipt {path}: client pin {manifest_path} records no head')
    if recorded.get('head') is not None and recorded['head'] != head:
        raise Refused(f'client receipt {path}: records client head {str(recorded["head"])[:12]} but its pin '
                      f'says {head[:12]}')
    code, _ = _git(client_repo(root), 'cat-file', '-e', f'{head}^{{commit}}', check=False)
    if code:
        raise Refused(f'client receipt {path} ran a client revision {head[:12]} that the client repository '
                      f'{client_repo(root)} does not have')
    return head


def client_contains(root, client_head, fix_full):
    """True when the client revision contains the fix commit."""
    code, _ = _git(client_repo(root), 'merge-base', '--is-ancestor', fix_full, client_head, check=False)
    return code == 0


def _failed_assertion(details, proving):
    """Name of the first assertion a scenario report records as false, or None when it recorded none."""
    def assertion_maps(node, depth=0):
        if depth > 3 or not isinstance(node, dict):
            return
        assertions = node.get('assertions')
        if isinstance(assertions, dict):
            yield assertions
        for key in ('scenario', 'named_scenario'):
            if key in node:
                yield from assertion_maps(node[key], depth + 1)

    for assertions in assertion_maps(details):
        for name, passed in assertions.items():
            if name == proving and passed is False:
                return name
    return None


def _client_ref_states(receipt, graphical_refs, proving_assertions=None):
    """{ref: GREEN|RED|None} for the row's real-client refs in one receipt; None means it never reported it."""
    from . import regressions, scenarios

    details = receipt.get('details', {})
    states = {}
    for ref in graphical_refs:
        _, profile, check = ref.split(':', 2)
        if check in scenarios.SCENARIOS:
            named = details.get('named_scenario', {})
            if named.get('id') != check:
                states[ref] = None
            elif receipt.get('status') == 'passed':
                states[ref] = GREEN
            else:
                # RED is "the scenario ran and an assertion of it failed". A timeout, a launch error or
                # a crashed client never observed the bug, so it cannot stand as the negative control.
                states[ref] = RED if (receipt.get('status') == 'failed'
                                      and _failed_assertion(details, (proving_assertions or {}).get(ref)) is not None) else None
            continue
        try:
            regressions._profile_verdict(receipt, profile, check)
            states[ref] = GREEN
        except regressions.VerdictNotMet as missing:
            # A verdict that was never reported is not a red observation of the bug.
            states[ref] = RED if missing.value is False else None
    return states


def _receipt_identity(root, receipt_path, tree):
    """(receipt, path, server binary sha256) of a real-client result.json run on exactly `tree`."""
    path = Path(receipt_path).resolve()
    try:
        receipt = json.loads(path.read_text())
    except (OSError, ValueError) as error:
        raise Refused(f'client receipt {receipt_path} unreadable: {error}')
    if path.name != 'result.json':
        raise Refused(f'client receipt {receipt_path} is not a result.json')
    recorded = receipt.get('tree')
    if not isinstance(recorded, dict) or not re.fullmatch(r'[0-9a-f]{40}', str(recorded.get('head', ''))) or not re.fullmatch(r'[0-9a-f]{64}', str(recorded.get('sha256', ''))):
        raise Refused(f'client receipt {receipt_path} records no tree fingerprint (the harness writes it at run start): rerun the gate on the candidate')
    if recorded['head'] != tree['head']:
        raise Refused(f"client receipt ran at {recorded['head'][:12]}, not at the tree being promoted {tree['head'][:12]}: rerun the gate there")
    if recorded['sha256'] != tree['sha256']:
        raise Refused('client receipt ran with different uncommitted changes than the tree being promoted: rerun the gate on the candidate')
    server = _server_built_from(root, json.dumps(receipt), tree)
    return receipt, path, server, recorded


def check_client_control_receipt(root, receipt_path, graphical_refs, fix_full, tree, proving_assertions):
    """Validate the negative control of a `client:<sha>` row -> info dict.

    The control is a pinned client on the same server tree with the row's proving ref red.
    Its exact difference from the restored client is checked after both receipts are read."""
    receipt, path, server, recorded = _receipt_identity(root, receipt_path, tree)
    if not graphical_refs:
        raise Refused('a client:<sha> fix_commit needs a real-client ref to carry the control')
    states = _client_ref_states(receipt, graphical_refs, proving_assertions)
    red = [ref for ref, state in states.items() if state == RED]
    if not red:
        raise Refused(f"client receipt {path} is not red on any of the row's real-client refs ("
                      + ', '.join(f'{ref}={states[ref] or "unreported"}' for ref in graphical_refs)
                      + '): a control that stays green does not test the fix')
    client_head = _client_pin_of_receipt(root, receipt, path)
    return {'path': str(path), 'client_head': client_head, 'server_binary_sha256': server,
            'red': red, 'states': states}


def check_client_receipt(root, receipt_path, graphical_refs, tree):
    """Validate the supplied real-client receipt -> info dict.

    Refused unless the harness recorded, at run start, a clean tree revision equal to `tree` (the
    candidate being promoted, which contains the fix_commit)."""
    from . import scenarios

    path = Path(receipt_path).resolve()
    try:
        receipt = json.loads(path.read_text())
    except (OSError, ValueError) as error:
        raise Refused(f'client receipt {receipt_path} unreadable: {error}')
    if path.name != 'result.json' or receipt.get('status') != 'passed':
        raise Refused(f"client receipt {receipt_path} is not a passed result.json (status {receipt.get('status')!r})")
    details = receipt.get('details', {})
    covered = []
    for ref in graphical_refs:
        _, profile, check = ref.split(':', 2)
        if check in scenarios.SCENARIOS:
            if details.get('named_scenario', {}).get('id') == check:
                covered.append(ref)
        elif receipt.get('profile') == profile:
            try:
                regressions._profile_verdict(receipt, profile, check)
                covered.append(ref)
            except RuntimeError:
                pass
    if not covered:
        raise Refused(f"client receipt {receipt_path} does not pass any of the row's real-client refs: {', '.join(graphical_refs)}")
    _, _, server, recorded = _receipt_identity(root, path, tree)
    return {'path': str(path), 'tree_head': recorded['head'], 'tree_dirty': bool(recorded.get('dirty')), 'tree_sha256': recorded['sha256'],
            'server_binary_sha256': server, 'covers': covered, 'profile': receipt.get('profile')}


def check_client_restore_receipt(root, receipt_path, graphical_refs, fix_full, tree):
    """Validate the restore of a `client:<sha>` row -> info dict.

    A passed receipt of the candidate whose client contains the fix and whose ref is green: the pair
    with its control proves the ref follows the client fix, on this server tree."""
    receipt, path, server, recorded = _receipt_identity(root, receipt_path, tree)
    if receipt.get('status') != 'passed':
        raise Refused(f"client receipt {path} is not a passed result.json (status {receipt.get('status')!r})")
    states = _client_ref_states(receipt, graphical_refs)
    green = [ref for ref, state in states.items() if state == GREEN]
    if not green:
        raise Refused(f"client receipt {path} does not pass any of the row's real-client refs ("
                      + ', '.join(f'{ref}={states[ref] or "unreported"}' for ref in graphical_refs) + ')')
    client_head = _client_pin_of_receipt(root, receipt, path)
    if not client_contains(root, client_head, fix_full):
        raise Refused(f'client receipt {path} ran the client at {client_head[:12]}, which does not contain the fix '
                      f'{fix_full[:12]}: the restore has to run the fixed client')
    return {'path': str(path), 'client_head': client_head, 'server_binary_sha256': server,
            'green': green, 'states': states, 'tree_head': recorded['head'],
            'tree_dirty': bool(recorded.get('dirty')), 'tree_sha256': recorded['sha256']}


def _check_client_control_delta(root, control_head, restore_head, fix_full):
    """Require the control-to-restore production patch to be exactly the fix's production patch."""
    repo = client_repo(root)
    fix_patch, fix_paths, _ = non_test_patch(repo, fix_full)
    _, delta = _git(repo, 'diff', '--no-renames', '--no-color', control_head, restore_head)
    actual, actual_paths, _ = _split_non_test(delta, lambda path: _git(repo, 'show', f'{restore_head}:{path}')[1])
    def patch_id(patch):
        if not patch:
            return None
        code, output = _git(repo, 'patch-id', '--verbatim', stdin=patch.encode(errors='surrogateescape'), check=False)
        return output.split()[0] if code == 0 and output.split() else None
    expected_id, actual_id = patch_id(fix_patch), patch_id(actual)
    if expected_id is None or expected_id != actual_id:
        extra = sorted(set(actual_paths) - set(fix_paths))
        changed = sorted(set(actual_paths) | set(fix_paths))
        raise Refused('client control differs from the restore by more than the fix non-test hunks; '
                      f'extra paths: {", ".join(extra) or "(none; changed hunks in " + ", ".join(changed) + ")"}')
    return {'fix_patch_id': expected_id, 'paths': fix_paths}


def _mark_fixed(text, ident, promotion):
    blocks = re.split(r'(?m)^(?=\[\[bug\]\][ \t]*$)', text)
    for index, block in enumerate(blocks):
        if re.search(rf'(?m)^id = "{re.escape(ident)}"[ \t]*$', block):
            block, count = re.subn(r'(?m)^status = "open"[ \t]*$', 'status = "fixed"', block, count=1)
            if count != 1:
                raise Refused(f'{ident}: no `status = "open"` line to flip')
            body = block.rstrip('\n')
            blocks[index] = f'{body}\npromotion = "{promotion}"{block[len(body):]}'
            return ''.join(blocks)
    raise Refused(f'{ident}: row not found in ledger.toml')


def _promote_client_fix(ident, *, row, fix, root, tree_now, dry_run, ctx, client_receipt, client_control_receipt, runner):
    """A client fix needs a real-client red/green pair and all candidate refs green."""
    fix_full = client_fix_commit(fix, root)
    refs = row['regression']
    if not refs:
        raise Refused(f'{ident}: no registered check (regression is empty)')
    for ref in refs:
        try:
            regressions._parse_ref(ref)
        except ValueError as error:
            raise Refused(f'{ident}: {error}')
    graphical = [ref for ref in refs if regressions.is_graphical_ref(ref)]
    if not graphical:
        raise Refused(f'{ident}: a client:<sha> fix_commit needs a real-client harness ref (a graphical check) '
                      'to carry the red control')
    if not client_control_receipt:
        raise Refused(f'{ident}: fix_commit {fix} needs the negative control (--client-control-receipt '
                      '<result.json>): a real-client receipt of the pre-fix client with the ref red')
    if not client_receipt:
        raise Refused(f'{ident}: fix_commit {fix} needs the restore (--client-receipt <result.json>): a passed '
                      'receipt of the client that contains the fix with the same ref green')
    control = check_client_control_receipt(root, client_control_receipt, graphical, fix_full, tree_now,
                                           row.get('proving_assertions', {}))
    restored_info = check_client_restore_receipt(root, client_receipt, graphical, fix_full, tree_now)
    delta = _check_client_control_delta(root, control['client_head'], restored_info['client_head'], fix_full)
    if not set(control['red']) & set(restored_info['green']):
        raise Refused(f"{ident}: the control's red refs {control['red']} and the restore's green refs "
                      f"{restored_info['green']} do not overlap: the same check has to flip")
    remaining = [ref for ref in refs if ref not in restored_info['green']]
    checked = runner(ctx, remaining, root, True) if remaining else {}
    bad = {ref: checked.get(ref, {'state': UNRUNNABLE, 'detail': 'no result'})
           for ref in remaining if checked.get(ref, {}).get('state') != GREEN}
    if bad:
        raise Refused(f'{ident}: candidate refs not green: ' + '; '.join(
            f"{ref}={entry['state']} ({entry['detail']})" for ref, entry in bad.items()))
    receipt = {
        'id': ident, 'verdict': 'promoted', 'fix_commit': fix, 'fix_commit_full': fix_full,
        'promoted_at': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
        'tree_head': tree_now['head'], 'tree_dirty': tree_now['dirty'], 'tree_sha256': tree_now['sha256'],
        'client_fix': True, 'control_tree': 'client',
        'negative_control': control, 'restored': restored_info, 'client_receipt': restored_info,
        'client_control_delta': delta,
        'candidate_refs': checked,
        'artifact_dir': str(ctx.artifact_dir),
    }
    (ctx.artifact_dir / 'promote.json').write_text(json.dumps(receipt, indent=2) + '\n')
    if dry_run:
        return receipt
    promotion = f'{regressions.PROMOTION_DIR}/{ident}.json'
    ledger = root / 'docs/bugs/ledger.toml'
    original = ledger.read_text()
    (root / promotion).parent.mkdir(parents=True, exist_ok=True)
    (root / promotion).write_text(json.dumps(receipt, indent=2) + '\n')
    ledger.write_text(_mark_fixed(original, ident, promotion))
    try:
        written = next(bug for bug in _load_bugs(root) if bug['id'] == ident)
        assert written['status'] == 'fixed' and written['promotion'] == promotion
    except Exception:
        ledger.write_text(original)
        raise
    return receipt


def promote(ident, *, root=REPO_ROOT, client_receipt=None, client_control_receipt=None, fix_patch=None,
            dry_run=False, ctx, runner=run_refs):
    """Run every promotion check for `ident`; on success (and not dry_run) flip it to fixed. Refused on any failure.

    The candidate is the exact tree: HEAD plus uncommitted diff and untracked files (docs/bugs aside).
    A `client:<sha>` fix_commit is a client-repository fix: its control is a red real-client receipt of
    the pre-fix client and its restore a green one of the client that contains the fix (docs/bugs/README.md)."""
    bugs = _load_bugs(root)
    row = next((bug for bug in bugs if bug['id'] == ident), None)
    if row is None:
        raise Refused(f'unknown ledger id {ident}')
    if row['status'] == 'fixed' and not dry_run:
        raise Refused(f'{ident} is already fixed (only --dry-run may re-audit a fixed row)')
    tree_now = tree_fingerprint(root)
    head = tree_now['head']
    fix = row.get('fix_commit')
    if not fix:
        raise Refused(f'{ident}: no fix_commit recorded')
    if fix.startswith('client:'):
        return _promote_client_fix(ident, row=row, fix=fix, root=root, tree_now=tree_now, dry_run=dry_run,
                                   ctx=ctx, client_receipt=client_receipt,
                                   client_control_receipt=client_control_receipt, runner=runner)
    if client_control_receipt:
        raise Refused(f'{ident}: --client-control-receipt is only for a fix_commit of the form client:<sha>')
    patch_mode = fix.startswith('patch:')
    if patch_mode:
        full = fix
    else:
        code, full = _git(root, 'rev-parse', '--verify', '--quiet', f'{fix}^{{commit}}', check=False)
        if code:
            raise Refused(f'{ident}: fix_commit {fix} is not a commit of this repository')
        full = full.strip()
        code, _ = _git(root, 'merge-base', '--is-ancestor', full, 'HEAD', check=False)
        if code:
            raise Refused(f'{ident}: fix_commit {fix} is not contained in this tree (HEAD)')
    refs = row['regression']
    if not refs:
        raise Refused(f'{ident}: no registered check (regression is empty)')
    for ref in refs:
        try:
            regressions._parse_ref(ref)
        except ValueError as error:
            raise Refused(f'{ident}: {error}')
    graphical = [ref for ref in refs if regressions.is_graphical_ref(ref)]
    receipt_info = None
    if regressions.needs_real_client(row) and not graphical:
        raise Refused(f'{ident}: client-surface row has no real-client harness ref')
    if regressions.needs_real_client(row) and not client_receipt:
        raise Refused(f'{ident}: client-surface row needs a passed real-client receipt (--client-receipt <result.json>)')
    if client_receipt:
        receipt_info = check_client_receipt(root, client_receipt, graphical, tree_now)
    patch, fix_patch_text = None, None
    if patch_mode:
        if not fix_patch:
            raise Refused(f'{ident}: fix_commit is {fix}: supply the patch with --fix-patch <file>')
        fix_patch_text = Path(fix_patch).read_text(errors='surrogateescape')
    elif fix_patch:
        raise Refused(f'{ident}: --fix-patch is only for a fix_commit of the form patch:<sha256>')
    if not patch_mode:
        patch, reversed_files, test_files = non_test_patch(root, full)
        if not patch:
            raise Refused(f'{ident}: fix commit {fix} has no non-test hunks to reverse')
    reverse = ['apply', '-R', '--whitespace=nowarn', '-']
    with contextlib.ExitStack() as stack:
        stack.enter_context(_control_lock(root))
        # Restore always runs on the exact tree being promoted: its HEAD plus any uncommitted diff and
        # untracked files (verified against the fingerprint), never on fix_commit.
        candidate = stack.enter_context(_worktree(root, head, 'candidate'))
        _materialize(root, candidate, tree_now)
        if patch_mode:
            # An uncommitted fix: the supplied patch, split like a commit; its identity is the hash of
            # its non-test hunks, and it must still apply in reverse on the exact candidate.
            patch, reversed_files, test_files = _split_non_test(
                fix_patch_text, lambda path: (candidate / path).read_text(errors='surrogateescape'))
            digest = 'patch:' + hashlib.sha256(patch.encode(errors='surrogateescape')).hexdigest()
            if not patch:
                raise Refused(f'{ident}: the fix patch has no non-test hunks to reverse')
            if digest != fix:
                raise Refused(f"{ident}: the fix patch's non-test hunks are {digest}, not the recorded {fix}")
            if _git(candidate, 'apply', '-R', '--check', '--whitespace=nowarn', '-', stdin=patch.encode(errors='surrogateescape'), check=False)[0]:
                raise Refused(f'{ident}: the fix patch does not apply in reverse on the candidate (edited or re-broken since it was recorded)')
            control_tree, control_dir = 'head', candidate
        # Control: reverse the fix's non-test hunks onto the candidate; only if a committed fix's
        # hunks no longer apply there, reverse them at the historical fix_commit instead.
        elif _git(candidate, 'apply', '-R', '--check', '--whitespace=nowarn', '-', stdin=patch.encode(errors='surrogateescape'), check=False)[0] == 0:
            control_tree, control_dir = 'head', candidate
        else:
            control_tree, control_dir = 'fix_commit', stack.enter_context(_worktree(root, full, 'fix'))
        _git(control_dir, *reverse, stdin=patch.encode(errors='surrogateescape'))
        control = runner(ctx, refs, control_dir, True)
        if not any(entry['state'] == RED for entry in control.values()):
            raise Refused(f"{ident}: negative control ({control_tree}) did not go red with the fix reversed: "
                          + '; '.join(f"{ref}={entry['state']}" + (f" ({entry['detail']})" if entry['detail'] else '') for ref, entry in control.items())
                          + ' (a check that stays green does not test the fix; one that cannot build or run without it cannot show an assertion failing)')
        _materialize(root, candidate, tree_now)
        # A ref the supplied receipt already passed on this HEAD is not rerun, unless it carried the red
        # control: its restore proves the red was the reversal, not the environment.
        covered = set(receipt_info['covers']) - {ref for ref, entry in control.items() if entry['state'] == RED} if receipt_info else set()
        restored = runner(ctx, [ref for ref in refs if ref not in covered], candidate, True) if len(covered) < len(refs) else {}
        bad = {ref: entry for ref, entry in restored.items() if entry['state'] != GREEN}
        if bad:
            raise Refused(f"{ident}: check not green on the tree being promoted ({head[:12]}): "
                          + '; '.join(f"{ref}={entry['state']} ({entry['detail']})" for ref, entry in bad.items()))
    receipt = {
        'id': ident, 'verdict': 'promoted', 'fix_commit': fix, 'fix_commit_full': full,
        'promoted_at': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
        'tree_head': head, 'tree_dirty': tree_now['dirty'], 'tree_sha256': tree_now['sha256'], 'control_tree': control_tree,
        'fix_patch_sha256': fix[len('patch:'):] if patch_mode else None,
        'reversed_files': reversed_files, 'kept_test_files': test_files,
        'negative_control': control, 'restored': restored, 'client_receipt': receipt_info,
        'artifact_dir': str(ctx.artifact_dir),
    }
    (ctx.artifact_dir / 'promote.json').write_text(json.dumps(receipt, indent=2) + '\n')
    if dry_run:
        return receipt
    promotion = f'{regressions.PROMOTION_DIR}/{ident}.json'
    ledger = root / 'docs/bugs/ledger.toml'
    original = ledger.read_text()
    (root / promotion).parent.mkdir(parents=True, exist_ok=True)
    (root / promotion).write_text(json.dumps(receipt, indent=2) + '\n')
    ledger.write_text(_mark_fixed(original, ident, promotion))
    try:
        written = next(bug for bug in _load_bugs(root) if bug['id'] == ident)
        assert written['status'] == 'fixed' and written['promotion'] == promotion
    except Exception:
        ledger.write_text(original)
        raise
    return receipt


# -------------------------------------------------------------------------- CLI


def main(argv):
    parser = argparse.ArgumentParser(prog='python3 -m tools.harness ledger')
    sub = parser.add_subparsers(dest='action', required=True)
    probe_parser = sub.add_parser('probe', help='run rows\' registered checks on this tree; print one line per row')
    probe_parser.add_argument('ids', nargs='*')
    probe_parser.add_argument('--open', action='store_true', help='every open row')
    probe_parser.add_argument('--client', action='store_true', help='also run real-client refs (default: needs-real-client)')
    probe_parser.add_argument('--out', type=Path, default=None, help='write the full result as JSON (valid YAML) here')
    promote_parser = sub.add_parser('promote', help='the only way a row becomes fixed')
    promote_parser.add_argument('id')
    promote_parser.add_argument('--client-receipt', type=Path, default=None, help='passed real-client result.json from a tree containing fix_commit')
    promote_parser.add_argument('--client-control-receipt', type=Path, default=None,
                                help='negative control for fix_commit client:<sha>: a real-client result.json of the pre-fix client with the row\'s ref red')
    promote_parser.add_argument('--fix-patch', type=Path, default=None,
                                help="git diff of an uncommitted fix, for a fix_commit of the form patch:<sha256> of its non-test hunks")
    promote_parser.add_argument('--dry-run', action='store_true', help='run every check, write nothing to the ledger')
    args = parser.parse_args(argv)
    # Worktrees must share the invoking tree's build cache, not build their own target.
    os.environ.setdefault('CARGO_TARGET_DIR', str(REPO_ROOT / 'target'))
    if args.action == 'probe':
        if bool(args.ids) == args.open:
            parser.error('give row ids or --open')
        bugs = _load_bugs(REPO_ROOT)
        ids = args.ids or [bug['id'] for bug in bugs if bug['status'] == 'open']
        ctx = _artifact_ctx('probe')
        try:
            rows = probe(bugs, ids, client=args.client, ctx=ctx)
        except Refused as error:
            print(f'ledger probe: {error}', file=sys.stderr)
            return 2
        for ident, row in rows.items():
            print(format_row(ident, row))
        counts = defaultdict(int)
        for row in rows.values():
            counts[row['verdict']] += 1
        result = {'tree_head': _git(REPO_ROOT, 'rev-parse', 'HEAD')[1].strip(), 'client_run': args.client,
                  'artifact_dir': str(ctx.artifact_dir), 'counts': dict(counts), 'rows': rows}
        (ctx.artifact_dir / 'probe.json').write_text(json.dumps(result, indent=2) + '\n')
        if args.out:
            args.out.parent.mkdir(parents=True, exist_ok=True)
            args.out.write_text(json.dumps(result, indent=2) + '\n')
        print(f"[ledger] {dict(counts)} receipt={ctx.artifact_dir / 'probe.json'}")
        return 0
    ctx = _artifact_ctx('promote')
    try:
        receipt = promote(args.id, client_receipt=args.client_receipt,
                          client_control_receipt=args.client_control_receipt,
                          fix_patch=args.fix_patch, dry_run=args.dry_run, ctx=ctx)
    except Refused as error:
        print(f'[ledger] refused: {error}')
        print(f'[ledger] artifact={ctx.artifact_dir}')
        return 1
    print(f"[ledger] {'dry-run ok (nothing written)' if args.dry_run else 'promoted ' + args.id} artifact={ctx.artifact_dir}")
    return 0
