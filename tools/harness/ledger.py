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


def _failing_verdict(receipt, profile, verdict):
    try:
        regressions._profile_verdict(receipt, profile, verdict)
    except regressions.VerdictNotMet as error:
        return error.value is not None
    return False


# Assertion names that report a lost or late connection, not the scenario's own check.
_CONNECTION_ASSERTION = re.compile(r'(^|_)(in_play|entered_play|join(ed)?)(_|$)')
_PRE_SCENARIO_PHASES = frozenset({'preflight', 'server_start', 'client_start', 'join'})


def _scenario_assertion_failed(receipt, check):
    """A named-scenario ref is red when its own receipt (`details.named_scenario.id` is the
    ref) failed on one of the scenario's assertions. Read from the receipt's `error` only,
    never from the command line that ran it. A connection assertion (`in_play`,
    `*_entered_play`, join) or a failure tagged before the scenario body is a flake."""
    if not isinstance(receipt, dict):
        return False
    details = receipt.get('details') if isinstance(receipt.get('details'), dict) else {}
    named = details.get('named_scenario')
    if not (isinstance(named, dict) and named.get('id') == check):
        return False
    failure = details.get('failure') if isinstance(details.get('failure'), dict) else {}
    if failure.get('phase') in _PRE_SCENARIO_PHASES:
        return False
    match = re.search(r'\bassertion failed: (\S+)', str(receipt.get('error') or ''))
    return bool(match) and not _CONNECTION_ASSERTION.search(match.group(1))


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
        return f'loader-live requires a Loader checkout at {loader_root} (set SOLARIS_LOADER_ROOT)'
    env['SOLARIS_LOADER_ROOT'] = str(loader_root)
    gradle_home = Path(env.get('GRADLE_USER_HOME') or (main_checkout().parent / 'loader-gradle-home'))
    if not gradle_home.is_dir():
        return f'loader-live requires a Gradle home at {gradle_home} (set GRADLE_USER_HOME)'
    env['GRADLE_USER_HOME'] = str(gradle_home)
    version = _loader_mc_version(loader_root)
    if not env.get('SOLARIS_CLIENT_JAR'):
        jar = gradle_home / 'caches' / 'fabric-loom' / (version or '') / 'minecraft-client.jar'
        if not jar.is_file():
            return f'loader-live requires the cached {version} client jar at {jar} (set SOLARIS_CLIENT_JAR)'
        env['SOLARIS_CLIENT_JAR'] = str(jar)
    wrapper = REPO_ROOT / 'tools' / 'harness' / 'screen_audio_graph.sh'
    if not wrapper.is_file():
        return f'loader-live requires the private audio wrapper {wrapper}'
    env['SOLARIS_AUDIO_RECEIPT_DIR'] = str(ctx.artifact_dir)
    plugins_dir = ctx.artifact_dir / 'loader-live-plugins'
    plugins_dir.mkdir(parents=True, exist_ok=True)
    data_source = Path(os.environ.get('SOLARIS_VANILLA_DATA_ROOT') or (main_checkout() / 'data'))
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
            env.setdefault('SOLARIS_AUDIO_RECEIPT_DIR', str(ctx.artifact_dir))
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
                results[ref] = (RED if red else UNRUNNABLE, str(error), None)
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
    out, cargo, harness = {}, set(), []
    for ref in refs:
        try:
            kind, first, second = regressions._parse_ref(ref)
        except ValueError as error:
            out[ref] = {'state': UNRUNNABLE, 'detail': str(error)}
            continue
        if kind == 'cargo':
            cargo.add((first, second))
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


def _hunk_changed_lines(hunk):
    """New-side line numbers of the changed lines of a hunk (a deletion counts at its position)."""
    line = int(_HUNK.match(hunk[0]).group(3))
    changed = []
    for text in hunk[1:]:
        if text.startswith('+'):
            changed.append(line)
            line += 1
        elif text.startswith('-'):
            changed.append(line)
        elif text.startswith(' '):
            line += 1
    return changed


def non_test_patch(root, commit):
    """The fix commit's non-test hunks: (patch text, reversed files, kept test files)."""
    _, diff = _git(root, 'diff', '--no-renames', '--no-color', f'{commit}^1', commit)
    return _split_non_test(diff, lambda path: _git(root, 'show', f'{commit}:{path}')[1])


def _split_non_test(diff, source_of):
    """Split a diff into (patch text of its non-test hunks, reversed files, kept test files).

    Test hunks are those in test files and, for Rust, those after the first
    `#[cfg(test)] mod`. A hunk straddling that boundary refuses: it cannot be split.
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
        threshold = None
        if path.endswith('.rs') and not any(text.startswith('+++ /dev/null') for text in header):
            source = source_of(path)
            found = _CFG_TEST_MOD.search(source)
            if found:
                threshold = source.count('\n', 0, found.start()) + 1
            if threshold is not None and threshold > 1 and any(
                    text.startswith('new file mode') for text in header):
                # A file the fix commit adds is one hunk: reversing it deletes the file and
                # its inline test module together, so there is no boundary to straddle.
                threshold = None
        kept = []
        for hunk in hunks:
            changed = _hunk_changed_lines(hunk)
            if threshold is not None and changed and all(line >= threshold for line in changed):
                continue
            if threshold is not None and any(line >= threshold for line in changed):
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
    """Give the worktree its own copy of the vanilla content cache (57 MB): a check that fails only
    because it is missing would be a false red control. A real copy, not hard links or a symlink: a
    check that writes in place must not change the source tree's data."""
    source, target = root / 'data/vanilla', tree / 'data/vanilla'
    if source.is_dir() and not target.exists():
        (tree / 'data').mkdir(exist_ok=True)
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
    if not os.environ.get('SOLARIS_BUILD_TOKEN'):
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

    The scenarios run whatever SOLARIS_SERVER_BINARY names (a pinned old build, a sibling's): the
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
                      '(a pinned or foreign SOLARIS_SERVER_BINARY proves nothing about this source): '
                      'run `ledger probe --client` on the candidate and use its receipt')
    for build in named:
        if (build['tree']['head'], build['tree']['sha256']) == (tree['head'], tree['sha256']):
            return build['sha256']
    raise Refused(f"client receipt's server binary {named[0]['sha256'][:12]} was built from a different tree "
                  f"({named[0]['tree']['head'][:12]}), not the candidate")


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
    recorded = receipt.get('tree')
    if not isinstance(recorded, dict) or not re.fullmatch(r'[0-9a-f]{40}', str(recorded.get('head', ''))) or not re.fullmatch(r'[0-9a-f]{64}', str(recorded.get('sha256', ''))):
        raise Refused(f'client receipt {receipt_path} records no tree fingerprint (the harness writes it at run start): rerun the gate on the candidate')
    if recorded['head'] != tree['head']:
        raise Refused(f"client receipt ran at {recorded['head'][:12]}, not at the tree being promoted {tree['head'][:12]}: rerun the gate there")
    if recorded['sha256'] != tree['sha256']:
        raise Refused('client receipt ran with different uncommitted changes than the tree being promoted: rerun the gate on the candidate')
    server = _server_built_from(root, json.dumps(receipt), tree)
    return {'path': str(path), 'tree_head': recorded['head'], 'tree_dirty': bool(recorded.get('dirty')), 'tree_sha256': recorded['sha256'],
            'server_binary_sha256': server, 'covers': covered, 'profile': receipt.get('profile')}


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


def promote(ident, *, root=REPO_ROOT, client_receipt=None, fix_patch=None, dry_run=False, ctx, runner=run_refs):
    """Run every promotion check for `ident`; on success (and not dry_run) flip it to fixed. Refused on any failure.

    The candidate is the exact tree: HEAD plus uncommitted diff and untracked files (docs/bugs aside)."""
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
        receipt = promote(args.id, client_receipt=args.client_receipt, fix_patch=args.fix_patch, dry_run=args.dry_run, ctx=ctx)
    except Refused as error:
        print(f'[ledger] refused: {error}')
        print(f'[ledger] artifact={ctx.artifact_dir}')
        return 1
    print(f"[ledger] {'dry-run ok (nothing written)' if args.dry_run else 'promoted ' + args.id} artifact={ctx.artifact_dir}")
    return 0
