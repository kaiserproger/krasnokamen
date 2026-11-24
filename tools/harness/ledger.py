"""Bug-ledger tools: ``ledger probe`` classifies rows on the current tree and
``ledger promote`` is the only way a row becomes ``fixed``.

The ledger is one file per row, ``docs/bugs/rows/<ID>.toml`` (a row edit touches one
small file instead of a 7000-line table). ``ledger show``/``list``/``edit`` read and
write a row, ``ledger split`` converts a single-file ledger and verifies every row.

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


# A client-repository fix whose guard a client unit test observes: no scene assertion can flip on it,
# so its pair is the test red with the fix's production hunks reversed and green on the fix
# (OWNER-DECISIONS-NEEDED CL-7 option (a)).
CLIENT_TEST_PREFIX = 'client-test:'
LOADER_FIX_PREFIX = 'loader:'


# ----------------------------------------------------------------- ledger rows

ROW_DIR = 'docs/bugs/rows'
_BLOCK_HEAD = re.compile(r'(?m)^(?=\[\[bug\]\][ \t]*$)')
# Fields `ledger edit` refuses: they have exactly one writer each (`ledger promote`, the file name).
IMMUTABLE = {
    'id': 'the id is the row file name',
    'status': 'only `python3 -m tools.harness ledger promote` sets a status',
    'promotion': 'only `python3 -m tools.harness ledger promote` records a promotion',
}


def rows_dir(root=REPO_ROOT):
    """The per-row ledger directory: one `[[bug]]` table per file, named `<ID>.toml`."""
    return root / ROW_DIR


def row_path(root, ident):
    """The file holding row `ident`; rows never move, so ids and paths cannot drift apart."""
    return rows_dir(root) / f'{ident}.toml'


def _relative(root, path):
    try:
        return path.relative_to(root).as_posix()
    except ValueError:
        return str(path)


def _row_blocks(text):
    """(preamble, [(id, block text)]) of a ledger document; the preamble may only hold comments.

    A block keeps the exact text of its `[[bug]]` table, comments and multi-line strings
    included, so a file split round-trips byte for byte.
    """
    parts = _BLOCK_HEAD.split(text)
    preamble, blocks = parts[0], parts[1:]
    for line in preamble.splitlines():
        if line.strip() and not line.lstrip().startswith('#'):
            raise ValueError(f'ledger: text before the first [[bug]] row must be comments only, not {line.strip()[:60]!r}')
    rows = []
    for block in blocks:
        table = tomllib.loads(block)
        if set(table) != {'bug'} or not isinstance(table['bug'], list) or len(table['bug']) != 1:
            raise ValueError('ledger: every [[bug]] block must hold exactly one row and nothing else')
        ident = table['bug'][0].get('id')
        if not isinstance(ident, str) or not ident:
            raise ValueError('ledger: a row has no string id')
        rows.append((ident, block.rstrip('\n') + '\n'))
    return preamble, rows


def load_rows(root=REPO_ROOT):
    """Every ledger row in file-name order; ValueError names the offending file.

    One row per `<ID>.toml`, the file name is the row id, and nothing else may live in
    the file: ids stay unique across the directory without reading a 6000-line table.
    """
    directory = rows_dir(root)
    files = sorted(directory.glob('*.toml')) if directory.is_dir() else []
    if not files:
        raise ValueError(f'{ROW_DIR}: no ledger rows (expected one <ID>.toml per bug)')
    rows, seen = [], {}
    for path in files:
        name = _relative(root, path)
        try:
            parsed = tomllib.loads(path.read_text(encoding='utf-8'))
        except tomllib.TOMLDecodeError as error:
            raise ValueError(f'{name}: {error}') from None
        if set(parsed) != {'bug'} or not isinstance(parsed['bug'], list) or len(parsed['bug']) != 1:
            raise ValueError(f'{name}: must hold exactly one [[bug]] row and nothing else')
        row = parsed['bug'][0]
        ident = row.get('id')
        if not isinstance(ident, str) or not ident:
            raise ValueError(f'{name}: no string id')
        if path.stem != ident:
            raise ValueError(f'{name}: the file name must be {ident}.toml, not {path.name}')
        if ident in seen:
            raise ValueError(f'{name}: duplicate id {ident}, also in {seen[ident]}')
        seen[ident] = name
        rows.append(row)
    return rows


def document(root=REPO_ROOT):
    """Every row assembled into one TOML document (what `ledger show` with no id prints)."""
    return '\n'.join(row_path(root, row['id']).read_text(encoding='utf-8').rstrip('\n')
                     for row in load_rows(root)) + '\n'


def write_rows(root, text):
    """Write a ledger document as one row file per row; row files absent from `text` are removed."""
    _, rows = _row_blocks(text)
    idents = {ident for ident, _ in rows}
    if len(idents) != len(rows):
        raise ValueError('ledger: the document holds a duplicate id')
    directory = rows_dir(root)
    directory.mkdir(parents=True, exist_ok=True)
    for ident, block in rows:
        row_path(root, ident).write_text(block, encoding='utf-8')
    for stale in sorted(directory.glob('*.toml')):
        if stale.stem not in idents:
            stale.unlink()
    return idents


def split(root=REPO_ROOT, source=None):
    """Convert the single-file ledger into row files; verified row for row, then the source is removed.

    Re-runnable: the single file is authoritative, so running it again on a newer
    `docs/bugs/ledger.toml` rewrites the rows and reports a mismatch instead of a half-move.
    """
    source = Path(source) if source is not None else root / 'docs/bugs/ledger.toml'
    if not source.is_file():
        raise Refused(f'{source}: no single-file ledger to split')
    text = source.read_text(encoding='utf-8')
    parsed = tomllib.loads(text)
    if set(parsed) != {'bug'} or not isinstance(parsed['bug'], list) or not parsed['bug']:
        raise Refused(f'{source}: must hold a nonempty [[bug]] array and no other tables')
    _, blocks = _row_blocks(text)
    write_rows(root, text)
    before = {row['id']: row for row in parsed['bug']}
    after = {row['id']: row for row in load_rows(root)}
    if after != before:
        differ = sorted(set(before) ^ set(after)) or sorted(i for i in before if before[i] != after[i])
        raise Refused(f'{ROW_DIR}: rows differ from {source} after the split ({", ".join(differ[:5])}); '
                      'the single file is kept')
    for ident, block in blocks:
        if row_path(root, ident).read_text(encoding='utf-8') != block:
            raise Refused(f'{ROW_DIR}/{ident}.toml: text differs from its block in {source}; '
                          'the single file is kept')
    source.unlink()
    return len(blocks)


def show(root, ident, field=None):
    """The text of one row file, or one field of it (`--field`); Refused when the row is absent."""
    path = row_path(root, ident)
    if not path.is_file():
        raise Refused(f'{ident}: no row file {_relative(root, path)}; `ledger list` shows the ids')
    text = path.read_text(encoding='utf-8')
    if field is None:
        return text
    row = tomllib.loads(text)['bug'][0]
    if field not in row:
        raise Refused(f'{ident}: no field {field}')
    value = row[field]
    return value if isinstance(value, str) else json.dumps(value)


def _key_line(text, key):
    """Index of the row's own `key = ...` line, outside any multi-line string, or None."""
    inside = ''
    for index, line in enumerate(text.splitlines(keepends=True)):
        if inside:
            inside = '' if line.count(inside) % 2 else inside
            continue
        if re.match(rf'^{re.escape(key)}[ \t]*=', line):
            return index
        for quote in ('"""', "'''"):
            if line.count(quote) % 2:
                inside = quote
                break
    return None


def edit(root, ident, assignments, *, dry_run=False):
    """Set fields of one row (`--set key=value`), validated by re-parsing the row.

    `id`, `status` and `promotion` are refused: they have exactly one writer each
    (the file name, `ledger promote`), so a hand edit cannot fake a promotion.
    """
    path = row_path(root, ident)
    if not path.is_file():
        raise Refused(f'{ident}: no row file {_relative(root, path)}; `ledger list` shows the ids')
    text = path.read_text(encoding='utf-8')
    known = regressions.REQUIRED | regressions.OPTIONAL
    for assignment in assignments:
        key, separator, value = assignment.partition('=')
        key, value = key.strip(), value.strip()
        if not separator or not key or not value:
            raise Refused(f'{ident}: --set {assignment!r} is not `key = value`')
        if key in IMMUTABLE:
            raise Refused(f'{ident}: {key} is not editable here: {IMMUTABLE[key]}')
        if key not in known:
            raise Refused(f'{ident}: unknown field {key} ({", ".join(sorted(known))})')
        replacement = f'{key} = {value}\n'
        index = _key_line(text, key)
        if index is None:
            text = text.rstrip('\n') + '\n' + replacement
        else:
            lines = text.splitlines(keepends=True)
            lines[index] = replacement
            text = ''.join(lines)
        try:
            row = tomllib.loads(text)['bug'][0]
        except (tomllib.TOMLDecodeError, KeyError, IndexError) as error:
            raise Refused(f'{ident}: {key} = {value} is not a valid row value ({error})') from None
        if row.get('id') != ident:
            raise Refused(f'{ident}: the edit would change the row id')
    if not dry_run:
        path.write_text(text, encoding='utf-8')
    return text


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
_PANIC_LINE = re.compile(r"panicked at ([^\s:]+):(\d+)")
# A test body's `.unwrap()` panics through #[track_caller] at the test's own line, but it is a setup
# failure, not an assertion: no control kind may accept it as evidence.
_SETUP_PANIC = re.compile(r'^called `(?:Option|Result)::(unwrap|expect|unwrap_err|expect_err)')
# Confirmed fixture-setup failures the harness's own tests produced: a shared registry or world
# builder panics before the registered test body runs (ReviewD123Harness, Mechanics rows: the control
# was red, but at `Registry("invalid map color registry")` from a fixture, not at an assertion).
_FIXTURE_PANICS = (
    re.compile(r'registry loads', re.IGNORECASE),
    re.compile(r'invalid map color registry', re.IGNORECASE),
    re.compile(r'\btest_registry\('),
    re.compile(r'\bfixture_store\('),
)
_ASSERT_LINE = re.compile(r'\b(?:assert(?:_eq|_ne)?!|debug_assert(?:_eq|_ne)?!|panic!\s*\()')
_ASSERT_MESSAGE = re.compile(r'^(assertion|assert\b|explicit panic|panicked at)', re.IGNORECASE)
_UNWRAP_LINE = re.compile(r'\.(?:unwrap|expect)\s*\(')

FIXTURE_FAILURE = 'fixture-failure'
UNPROVEN_PANIC = 'unproven-panic'


def _panic_locations(text):
    """{(crate, test name): (file, line, message)} of the panics nextest captured.

    A negative control is red only at an assertion: `panicked at <path>:<line>` is where the panic
    happened, and the message plus that source line is what later decides whether it was an
    assertion or a fixture. The key pairs the panic with the result line it follows (its binary's
    crate and the test name), so two modules with a test of the same name do not overwrite each
    other."""
    locations = {}
    current = None
    for line in text.splitlines():
        result = _RESULT_LINE.match(line)
        if result:
            current = (result.group(2).split('::', 1)[0], result.group(3))
            continue
        match = _PANIC_LINE.search(line)
        if match is None:
            continue
        rest = text[text.find(line) + len(line):].splitlines()
        message = rest[1].strip() if len(rest) > 1 else ''
        thread = re.search(r"thread '([^']+)' panicked", line)
        if thread and '::' in thread.group(1):
            name = thread.group(1).rsplit('::', 1)[-1]
            crate = thread.group(1).split('::', 1)[0] if current is None else current[0]
        elif current is not None:
            name, crate = current[1], current[0]
            name = name.rsplit('::', 1)[-1]
        else:
            continue
        locations[(crate, name)] = (match.group(1), int(match.group(2)), message)
    return locations


def _panic_class(tree, location, message):
    """'assertion' | 'fixture' for a panic a cargo test emitted, from its message and source line.

    (a) a confirmed fixture/setup panic (registry or world builder, a `#[track_caller]`
    unwrap/expect) is `fixture`: the registered test body never ran its assertion.
    (b) a panic whose source line is an assert macro (or an explicit `panic!`) is `assertion`; a
    shared helper inside a `*_tests.rs` file counts, so a helper assert is not rejected.
    (c) anything else is `unknown` and the promotion must name its file:line as the proving
    assertion (ReviewD123Harness, fixture-panic rule)."""
    file, line = location
    if any(pattern.search(message or '') for pattern in _FIXTURE_PANICS) or _SETUP_PANIC.match(message or ''):
        return 'fixture'
    if not _TEST_PATH.search(file):
        # An assert macro in production code (even in a test-support module outside _TEST_PATH) is not
        # evidence about the oracle: class (c) (ReviewD123Harness3 N3b).
        return 'unknown'
    try:
        lines = (Path(tree) / file).read_text(errors='surrogateescape').splitlines()
    except OSError:
        return 'unknown'
    if not 1 <= line <= len(lines):
        return 'unknown'
    source = lines[line - 1]
    # A line that is an unwrap/expect (with #[track_caller] the test's own line) is only an assertion
    # when the panic message says so (an `assert_eq!` inside `expect(...)`); otherwise it is class (c).
    if _UNWRAP_LINE.search(source) and not _ASSERT_MESSAGE.match(message or ''):
        return 'unknown'
    if _ASSERT_LINE.search(source) and (_ASSERT_MESSAGE.search(message or '') or _UNWRAP_LINE.search(source)):
        return 'assertion'
    return 'unknown'


def _python_assertion(text):
    """The innermost traceback frame of a failing unittest, when its own exception is AssertionError."""
    blocks = text.split('Traceback (most recent call last):')
    if len(blocks) < 2:
        return None
    block = blocks[-1]
    # The exception line closes the block: `AssertionError: ...` (or `module.Error: ...`). A text that
    # mentions AssertionError elsewhere (a captured log, another test's output) is not this failure.
    exception = re.search(r'^([A-Za-z_][\w.]*(?:Error|Exception|Failure))(: .*)?$', block, re.MULTILINE)
    if exception is None or exception.group(1).rsplit('.', 1)[-1] != 'AssertionError':
        return None
    frames = re.findall(r'^\s*File "([^"]+)", line (\d+)', block, re.MULTILINE)
    if not frames:
        return None
    file, line = frames[-1]
    return {'file': file, 'line': int(line), 'error': 'AssertionError'}


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
        return {test: (UNRUNNABLE, reason, None) for test in tests}
    found = {(suite['package-name'], name) for suite in listing['rust-suites'].values()
             for name, result in suite.get('testcases', {}).items()
             if result.get('filter-match', {}).get('status') == 'matches' and not result.get('ignored', False)}
    results = {test: (UNRUNNABLE, 'unknown or ignored test', None) for test in tests - found}
    runnable = tests & found
    if not runnable:
        return results
    expression = ' | '.join(f'(package({crate}) & test(={name}))' for crate, name in sorted(runnable))
    argv, env = regressions.tree_build_command(tree, ['cargo', 'nextest', 'run', *package_args, '-E', expression, '--ignore-default-filter',
                                                      '--no-fail-fast', '--no-tests=fail', '--status-level', 'pass'])
    ok, text = _exec(ctx, argv, 'run', tree, env)
    seen = {}
    for status, binary, name in _RESULT_LINE.findall(text):
        if status == 'SLOW':
            continue  # nextest's slow summary follows the terminal PASS/FAIL
        seen[(binary.split('::', 1)[0], name)] = status
    panics = _panic_locations(text)
    for test in runnable:
        status = seen.get(test)
        if status is None:
            results[test] = (UNRUNNABLE, 'no result line (build error or run aborted); see log' if not ok else 'no result line', None)
        elif status in ('PASS', 'LEAK PASS'):
            results[test] = (GREEN, '', None)
        elif status == 'FAIL':
            location = panics.get((test[0], test[1].rsplit('::', 1)[-1]))
            assertion = None
            if location:
                assertion = {'file': location[0], 'line': location[1], 'error': 'panic',
                             'class': _panic_class(tree, location[:2], location[2])}
            results[test] = (RED, status, assertion)
        else:
            results[test] = (UNRUNNABLE, f'{status}; see log', None)
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


def _client_failure(text):
    """(file, line, message) of the first panic cargo test printed, or None.

    Cargo prints `thread '<name>' panicked at <path>:<line>:` and the message on the next line; a test
    that asserts on a child process' output carries the child's own text in that message, so the first
    occurrence is the test's own assert (order of printing, the child's text is inside the message)."""
    match = _PANIC_LINE.search(text)
    if match is None:
        return None
    rest = text[match.end():].splitlines()
    message = rest[0].strip() if rest else ''
    return match.group(1), int(match.group(2)), message


def _run_client(ctx, tests, root, repo=None):
    """Run exact client-crate tests {(crate, name)} in the client checkout -> {(crate, name): (state, detail, assertion)}.

    A `client:<crate>:<test>` ref is a fix in the client repository: it never builds here, so the
    checkout is `runtime.sibling_root('client')` (KRASNOKAMEN_CLIENT_ROOT or the sibling repo).
    `repo` selects another tree of that repository (the reversed client-test control copy)."""
    # Cargo metadata is one JSON line; mem-run can prepend its scope diagnostic.
    repo = Path(repo) if repo is not None else client_repo(root)
    if not repo.is_dir():
        return {test: (UNRUNNABLE, f'no client checkout at {repo} (set KRASNOKAMEN_CLIENT_ROOT)', None) for test in tests}
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
        return {test: (UNRUNNABLE, f'client package {test[0]}: cargo metadata failed or was invalid; see log', None)
                for test in tests}
    results = {}
    for crate, name in sorted(tests):
        if crate in errors:
            results[(crate, name)] = (UNRUNNABLE, errors[crate], None)
            continue
        # A ref may name one test or a whole module (`ui::toasts::tests`): cargo's filter selects it
        # either way, and `--exact` would find nothing for the module form. Green needs at least one
        # selected test and no failure, so a ref can never pass by selecting nothing.
        argv, env = regressions.tree_build_command(
            repo, ['cargo', 'test', '-p', crate, *targets[crate], '--', name])
        ok, text = _exec(ctx, argv, f'client-{name.rsplit("::", 1)[-1][:24]}', repo, env)
        summary = re.search(r'test result: (\w+)\. (\d+) passed; (\d+) failed', text)
        if summary is None:
            results[(crate, name)] = (UNRUNNABLE, 'no test summary (build error?)', None)
            continue
        passed, failed = int(summary.group(2)), int(summary.group(3))
        if passed + failed == 0:
            results[(crate, name)] = (UNRUNNABLE, 'ran no matching test', None)
        elif failed or not ok:
            failure = _client_failure(text)
            assertion = None
            if failure is not None:
                assertion = {'file': failure[0], 'line': failure[1], 'error': 'panic'}
            results[(crate, name)] = (RED, f'{failed} failed of {passed + failed}', assertion)
        else:
            results[(crate, name)] = (GREEN, '', None)
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
            results[(module, test)] = (UNRUNNABLE, 'no executed test or unittest error; see log', None)
        elif ok and re.search(r'(?m)^OK(?: \(|$)', text):
            results[(module, test)] = (GREEN, '', None)
        elif not ok and re.search(r'(?m)^FAIL:', text):
            results[(module, test)] = (RED, 'test assertion failed', _python_assertion(text))
        else:
            results[(module, test)] = (UNRUNNABLE, 'no conclusive unittest result; see log', None)
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
    loader_python, loader_gradle = set(), set()
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
        elif kind == 'loader':
            (loader_python if first == 'python' else loader_gradle).add(ref)
        elif client:
            harness.append(ref)
        else:
            out[ref] = {'state': NEEDS_CLIENT, 'detail': 'graphical ref; pass --client to run it'}
    with _one_off_build_env(tree):
        if cargo:
            for (crate, name), (state, detail, assertion) in _run_cargo(ctx, cargo, tree).items():
                out[f'cargo:{crate}:{name}'] = {'state': state, 'detail': detail,
                                               **({'assertion': assertion} if assertion else {})}
        if harness:
            for ref, (state, detail, receipt) in _run_harness(ctx, harness, tree).items():
                out[ref] = {'state': state, 'detail': detail, **({'receipt': receipt} if receipt else {})}
        if python:
            for (module, test), (state, detail, assertion) in _run_python(ctx, python, tree).items():
                out[f'python:{module}:{test}'] = {'state': state, 'detail': detail,
                                                  **({'assertion': assertion} if assertion else {})}
        if loader_python:
            for (directory, test), entry in _run_loader(
                    ctx, {(r.split(':', 2)[2].partition(':')[0], r.split(':', 2)[2].partition(':')[2])
                          for r in loader_python}).items():
                assertion = entry[2] if len(entry) > 2 else None
                out[f'loader:python:{directory}:{test}'] = {'state': entry[0], 'detail': entry[1],
                                                            **({'assertion': assertion} if assertion else {})}
        if loader_gradle:
            for (module, selector), entry in _run_loader_gradle(
                    ctx, {(r.split(':', 2)[2].partition(':')[0], r.split(':', 2)[2].partition(':')[2])
                          for r in loader_gradle}).items():
                assertion = entry[2] if len(entry) > 2 else None
                out[f'loader:gradle:{module}:{selector}'] = {'state': entry[0], 'detail': entry[1],
                                                             **({'assertion': assertion} if assertion else {})}
        if client_tests:
            for (crate, name), entry in _run_client(ctx, client_tests, tree).items():
                assertion = entry[2] if len(entry) > 2 else None
                out[f'client:{crate}:{name}'] = {'state': entry[0], 'detail': entry[1],
                                                 **({'assertion': assertion} if assertion else {})}
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
# A control reverses code. Row metadata and prose are skipped; any other docs/ path is a harness
# input (docs/playable/real-client-playable-loop.json, docs/real-client-regression/manifests/*.json
# and their kin) and must never be silently left out of a patch or reversed inside a control.
def _skipped_doc(path):
    if path.startswith('docs/bugs/') or path.endswith('.md'):
        return True
    if path.startswith('docs/'):
        raise Refused(f'{path} is a harness input under docs/; a promotion control cannot skip or reverse it')
    return False


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


def _split_test_hunks(diff, only=None):
    """The fix commit's test-file hunks: (patch text, test files). Refused otherwise.

    `only` narrows *what control A reverses* to the listed test files (a row field `control_a_files`):
    one commit often corrects several oracles, and a row's control must reverse its own file, not
    another row's. It never narrows the refusal: every non-doc path of the commit is still judged.

    `test_only_fix` accepts a fix whose guard IS a corrected test oracle, so the control reverses the
    test hunks instead of production hunks. It refuses any non-doc path that is not a test file: a fix
    that also edits production code keeps the ordinary `non_test_patch` control."""
    patch, files = [], []
    for chunk in re.split(r'(?m)^(?=diff --git )', diff):
        if not chunk:
            continue
        lines = chunk.splitlines(keepends=True)
        match = re.match(r'diff --git a/(.+) b/(.+)$', lines[0].rstrip('\n'))
        if match is None:
            raise Refused(f'fix patch has an unparseable diff header: {lines[0].rstrip()!r}')
        path = match.group(2)
        if _skipped_doc(path):
            continue
        # Every non-doc path is judged, whether or not `only` reverses it: `control_a_files` chooses
        # what control A undoes, it never licenses a production file in the fix commit.
        if not _TEST_PATH.search(path):
            raise Refused(f'{path} is production code: a test_only_fix fix commit may change only test files '
                          '(use the ordinary non-test control for a real code fix)')
        if only is not None and path not in only:
            continue
        if any(text.startswith('+++ /dev/null') or text.startswith('--- /dev/null') for text in lines):
            raise Refused(f'{path} is deleted or renamed by the fix commit: a test_only_fix control restores the '
                          'corrected test in place, so a removed test file has no control A')
        if not any(text.startswith('@@ ') for text in lines):
            raise Refused(f'{path} changes without text hunks (binary or mode-only); cannot be a control')
        patch.extend(lines)
        files.append(path)
    if only is not None:
        missing = sorted(set(only) - set(files))
        if missing:
            raise Refused(f"control_a_files names {', '.join(missing)}, which the fix commit does not change "
                          'as a test file')
        if not files:
            raise Refused(f"control_a_files {sorted(only)} names no file of the fix commit's test hunks")
    return ''.join(patch), files


_MOD_DECL = re.compile(r'^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z0-9_]+)\s*;')
_PATH_ATTR = re.compile(r'^\s*#\[path\s*=\s*"([^"]+)"\]\s*$')


def _declared_module_file(source: Path, name: str):
    """The file `mod <name>;` in `source` names, honouring a preceding `#[path = "..."]`."""
    try:
        lines = source.read_text(errors='surrogateescape').splitlines()
    except OSError:
        return None
    for index, line in enumerate(lines):
        match = _MOD_DECL.match(line)
        if match is None or match.group(1) != name:
            continue
        for back in range(index - 1, max(index - 5, -1), -1):
            path_attr = _PATH_ATTR.match(lines[back])
            if path_attr:
                return source.parent / path_attr.group(1)
            # Walk only over blank lines, attributes and comments: a blank line above the mod
            # declaration is common and must not stop the walk, a statement must.
            if lines[back].strip() and not lines[back].lstrip().startswith(('#[', '///', '//')):
                break
        # A module declared in a non-mod.rs file lives under <stem>/<name>.rs, never beside it
        # (ReviewD123Harness1 9d: the sibling lookup could bind to an unrelated x.rs).
        if source.name not in ('mod.rs', 'lib.rs', 'main.rs'):
            candidate = source.parent / source.stem / f'{name}.rs'
            if candidate.is_file():
                return candidate
        candidate = source.parent / f'{name}.rs'
        if candidate.is_file():
            return candidate
        candidate = source.parent / name / 'mod.rs'
        if candidate.is_file():
            return candidate
    return None


def _cargo_module_file(root: Path, crate: str, module: list[str]):
    """The crate source file a module path (`a::b`) names, or None.

    The test file of a `cargo:<crate>:<module::path::test>` ref is not its name spelled with
    slashes: `falling::tests` is `falling_tests.rs` through `#[path = "..."] mod tests;`, and
    `fluids_tests::parity` is `fluid_parity_tests.rs`. Following the mod declarations is the only
    exact mapping, and it must pin the file, never a bare test-function name."""
    base = Path('crates') / crate / 'src'
    source = None
    for entry in ('lib.rs', 'main.rs'):
        if (root / base / entry).is_file():
            source = base / entry
            break
    if source is None:
        return None
    for name in module:
        source = _declared_module_file(root / source, name)
        if source is None:
            return None
        source = source.relative_to(root)
    return source.as_posix()


def _ref_test_file(root: Path, ref: str, test_files):
    """The changed test file a registered ref's test body is defined in, or None."""
    try:
        kind, first, second = regressions._parse_ref(ref)
    except ValueError:
        return None
    if kind == 'python':
        expected = first.replace('.', '/') + '.py'
        return expected if expected in test_files else None
    if kind == 'cargo':
        resolved = _cargo_module_file(Path(root), first, second.split('::')[:-1])
        return resolved if resolved in test_files else None
    return None


def _uncovered_test_files(candidate, test_files, refs):
    """Changed test files that hold no test one of the row's refs runs.

    Exact, not by bare name: the flag must not attach unrelated refs, and a ref whose module path
    names another file (or no file) leaves its own changed test uncovered."""
    covered = {path for ref in refs if (path := _ref_test_file(candidate, ref, test_files))}
    missing = []
    for path in test_files:
        if path in covered:
            continue
        if not (candidate / path).is_file():
            raise Refused(f'{path} is not a file on the tree being promoted; the fix commit deletes or renames it')
        missing.append(path)
    return missing


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
        if _skipped_doc(path):
            continue
        starts = [i for i, text in enumerate(lines) if text.startswith('@@ ')]
        if not starts:
            raise Refused(f'fix commit changes {path} without text hunks (binary or mode-only); cannot split test hunks')
        header = lines[:starts[0]]
        hunks = [lines[a:b] for a, b in zip(starts, starts[1:] + [len(lines)])]
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


_HUNK_HEADER = re.compile(r'^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@')


def _changed_old_lines(diff):
    """{path: {line}} of the pre-image lines the diff actually adds or removes (context excluded)."""
    changed, path, line = {}, None, 0
    for text in diff.splitlines():
        if text.startswith('diff --git '):
            match = re.match(r'diff --git a/(.+) b/(.+)$', text)
            path = match.group(2) if match else None
            continue
        header = _HUNK_HEADER.match(text)
        if header is not None:
            line = int(header.group(1))
            continue
        if path is None or text.startswith('+++') or text.startswith('---'):
            continue
        if text.startswith('-'):
            changed.setdefault(path, set()).add(line)
            line += 1
        elif text.startswith('+'):
            continue
        elif text.startswith(' ') or text == '':
            line += 1
    return changed


def _operator_span(tree, file, line):
    """(first, last) lines covered by the call/operator starting at `line`, from the source text."""
    try:
        lines = (Path(tree) / file).read_text(errors='surrogateescape').splitlines()
    except OSError:
        return None
    if not 1 <= line <= len(lines):
        return None
    depth = 0
    started = False
    for index in range(line - 1, min(len(lines), line + 200)):
        for char in lines[index]:
            if char in '([{':
                depth += 1
                started = True
            elif char in ')]}':
                depth -= 1
        if started and depth <= 0:
            return (line, index + 1)
    # Unstarted (no bracket) or unclosed within the window: no span, so no acceptance (review N-c).
    return None


def _assertion_in_ranges(entry, ranges, control_dir):
    """True when the red assertion's line lies inside a hunk the control reversed."""
    assertion = entry.get('assertion') or {}
    path, line = assertion.get('file'), assertion.get('line')
    if not isinstance(path, str) or not isinstance(line, int):
        return False
    if os.path.isabs(path):
        try:
            path = Path(path).resolve().relative_to(Path(control_dir).resolve()).as_posix()
        except ValueError:
            return False
    return any(first <= line <= last for first, last in ranges.get(path, []))


def _classify_control(ident, control, row, tree):
    """Rewrite a control's entries so a fixture panic or an unnamed panic cannot pass as red.

    (a) `fixture-failure`: a confirmed fixture/setup panic — the registered test's body never ran.
    (c) `unproven-panic`: a panic at a line that is not an assert macro; the row must name that exact
    `file:line` in `proving_assertions` for it to count. Python AssertionErrors are unaffected."""
    named = row.get('proving_assertions') if isinstance(row.get('proving_assertions'), dict) else {}
    out = {}
    for ref, entry in control.items():
        if entry.get('state') != RED:
            out[ref] = entry
            continue
        assertion = entry.get('assertion') or {}
        if assertion.get('error') == 'AssertionError' or assertion.get('class') == 'assertion':
            out[ref] = entry
            continue
        location = f"{assertion.get('file')}:{assertion.get('line')}"
        if not assertion:
            # A cargo FAIL with no panic line is how a `-> Result<()>` test shows a setup error through
            # `?`; it is not an assertion unless the row names it (ReviewD123Harness3 N3a).
            if ref.startswith('cargo:') and not named.get(ref):
                out[ref] = {**entry, 'state': UNPROVEN_PANIC,
                            'detail': entry.get('detail', '') + ' | cargo FAIL without a panic location; name the ref in proving_assertions'}
            else:
                out[ref] = entry
        elif assertion.get('class') == 'fixture':
            out[ref] = {**entry, 'state': FIXTURE_FAILURE,
                        'detail': entry.get('detail', '') + f' | fixture panic at {location}'}
        elif named.get(ref) == location:
            out[ref] = entry
        else:
            out[ref] = {**entry, 'state': UNPROVEN_PANIC,
                        'detail': entry.get('detail', '') + f' | panic at {location} is not an assert macro; name it in proving_assertions'}
    return out


def _assertion_in_files(entry, control_dir, test_files):
    """A red control entry only counts when it failed at an assertion inside a changed test file.

    Cargo panics carry the workspace-relative `panicked at <path>:<line>`; a python traceback frame
    is absolute inside the control worktree. A panic in production code (or a plain build failure)
    is not evidence that the corrected oracle guards anything."""
    assertion = entry.get('assertion')
    if not isinstance(assertion, dict):
        return False
    path, error = assertion.get('file'), assertion.get('error')
    if not isinstance(path, str) or error not in ('panic', 'AssertionError'):
        return False
    if not os.path.isabs(path):
        return path in test_files
    try:
        relative = Path(path).resolve().relative_to(Path(control_dir).resolve()).as_posix()
    except ValueError:
        return False
    return relative in test_files and error == 'AssertionError'


def _check_defect_patch(ident, defect, candidate, patch_path):
    """B1: a defect injection mutates production behaviour only.

    The patch is split exactly like a fix patch: a hunk in a test file, a hunks set that leaves part
    of the patch out (docs/ or a .md), a build-configuration path, or a hunk inside an inline
    `#[cfg(test)]` module refuses. Otherwise replaying it could rewrite the very test it must turn red."""
    text = defect.decode(errors='surrogateescape')
    parsed, files, test_files = _split_non_test(
        text, lambda path: (candidate / path).read_text(errors='surrogateescape'))
    if test_files:
        raise Refused(f'{ident}: control_b_patch {patch_path} changes test files ({", ".join(sorted(test_files))}); '
                      'a defect injection mutates production behaviour, never the test it proves')
    if parsed != text:
        raise Refused(f'{ident}: control_b_patch {patch_path} leaves part of itself out (a skipped docs/ or .md '
                      'hunk); only production hunks are allowed')
    for path in files:
        if (path in ('Cargo.toml', 'build.rs') or path.endswith(('/Cargo.toml', '/build.rs'))
                or path.startswith(('.cargo/', '.config/'))
                or '/.cargo/' in path or '/.config/' in path):
            raise Refused(f'{ident}: control_b_patch {patch_path} changes build configuration ({path}); refused: '
                          'a defect injection must not change how the code is built')
    if not files:
        raise Refused(f'{ident}: control_b_patch {patch_path} changes no production file')
    return files


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
    """Record which tree (fingerprint) a release server binary was built from by this tooling.

    One file per (binary, tree): a harness-only fix leaves the release server byte-identical on the
    candidate and on its reverted control tree, so both fingerprints must be citable at once
    (FINDINGS-20261124 F1). The legacy `<sha256>.json` name is still read, never written."""
    registry = REPO_ROOT / '.analysis/ledger-builds'
    registry.mkdir(parents=True, exist_ok=True)
    name = f'{sha256}-{tree["sha256"]}.json'
    (registry / name).write_text(json.dumps({'sha256': sha256, 'tree': {'head': tree['head'], 'sha256': tree['sha256']}}) + '\n')


def _registration(registry, sha256, tree):
    """The registry entry for this binary and tree fingerprint, or None (legacy names included)."""
    for entry in sorted(registry.glob('*.json')) if registry.is_dir() else []:
        try:
            build = json.loads(entry.read_text())
        except (OSError, ValueError):
            continue
        if build.get('sha256') == sha256 and build.get('tree') == {'head': tree['head'], 'sha256': tree['sha256']}:
            return build
    return None


def _server_built_from(root, receipt_text, tree):
    """sha256 of the server binary a receipt names, when the tooling built it from exactly `tree`.

    The scenarios run whatever KRASNOKAMEN_SERVER_BINARY names (a pinned old build, a sibling's): the
    source fingerprint of the run says nothing about that binary, so its hash must be one that
    `_run_harness` registered for this candidate."""
    registry = root / '.analysis/ledger-builds'
    named = []
    for entry in sorted(registry.glob('*.json')) if registry.is_dir() else []:
        try:
            build = json.loads(entry.read_text())
        except (OSError, ValueError):
            continue
        if build.get('sha256') in receipt_text:
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
    """The client checkout a `client:<sha>`/`client-test:<sha>` fix_commit is resolved in (sibling repo or env)."""
    from . import runtime

    return runtime.sibling_root('client', root)


def client_fix_kind(fix):
    """'client' | 'client-test' | None for a fix_commit text; Refused on a malformed client prefix."""
    for kind in (CLIENT_TEST_PREFIX.rstrip(':'), 'client'):
        if fix.startswith(kind + ':'):
            return kind
    return None


def client_fix_commit(fix, root=REPO_ROOT):
    """Full sha of a `client:<sha>`/`client-test:<sha>` fix_commit in the client repository.

    Refused when the sha is not a commit there."""
    kind = client_fix_kind(fix)
    if kind is None:
        raise Refused(f'fix_commit {fix}: expected client:<sha> or {CLIENT_TEST_PREFIX}<sha>')
    sha = fix[len(kind) + 1:]
    if not re.fullmatch(r'[0-9a-f]{7,40}', sha):
        raise Refused(f'fix_commit {fix}: expected {kind}:<sha>')
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
    """One row file's text with its status flipped and the promotion receipt recorded."""
    if not re.search(rf'(?m)^id = "{re.escape(ident)}"[ \t]*$', text):
        raise Refused(f'{ident}: the row file does not hold that id')
    index = _key_line(text, 'status')
    lines = text.splitlines(keepends=True)
    if index is None or not re.fullmatch(r'status = "open"[ \t]*\n?', lines[index]):
        raise Refused(f'{ident}: no `status = "open"` line to flip')
    lines[index] = 'status = "fixed"\n'
    return f'{"".join(lines).rstrip(chr(10))}\npromotion = "{promotion}"\n'


def _record_promotion(root, ident, receipt):
    """Write the promotion receipt and flip the row's own file; restore the row if it does not reload."""
    promotion = f'{regressions.PROMOTION_DIR}/{ident}.json'
    path = row_path(root, ident)
    if not path.is_file():
        raise Refused(f'{ident}: no row file {_relative(root, path)}')
    original = path.read_text(encoding='utf-8')
    (root / promotion).parent.mkdir(parents=True, exist_ok=True)
    (root / promotion).write_text(json.dumps(receipt, indent=2) + '\n')
    path.write_text(_mark_fixed(original, ident, promotion), encoding='utf-8')
    try:
        written = next(bug for bug in load_rows(root) if bug['id'] == ident)
        assert written['status'] == 'fixed' and written['promotion'] == promotion
    except Exception:
        path.write_text(original, encoding='utf-8')
        raise


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
        'client_fix': True, 'control_tree': 'client', 'control': 'client_receipt_pair',
        'negative_control': control, 'restored': restored_info, 'client_receipt': restored_info,
        'client_control_delta': delta,
        'candidate_refs': checked,
        'artifact_dir': str(ctx.artifact_dir),
    }
    (ctx.artifact_dir / 'promote.json').write_text(json.dumps(receipt, indent=2) + '\n')
    if dry_run:
        return receipt
    _record_promotion(root, ident, receipt)
    return receipt


_CLIENT_COPY_IGNORE = shutil.ignore_patterns('.git', 'target', 'dist', 'logs', '.analysis', 'model-audit',
                                              'node_modules', '*.log')


def _client_source_state(root):
    """(repo, head, untracked source files) of the client checkout, Refused when its source is dirty.

    The pair has to run on one client revision: a tracked edit under `src/` or a manifest would mean the
    control copy and the checkout are not the same source, so the reversal could prove the local edit."""
    repo = client_repo(root)
    if not repo.is_dir():
        raise Refused(f'no client checkout at {repo} (set KRASNOKAMEN_CLIENT_ROOT)')
    _, head = _git(repo, 'rev-parse', 'HEAD')
    _, status = _git(repo, 'status', '--porcelain')
    dirty, untracked = [], []
    for line in status.splitlines():
        state, path = line[:2], line[3:].strip('"')
        if not (path.endswith(('.rs', '.toml', '.lock')) or path == 'build.rs'):
            continue
        (untracked if state == '??' else dirty).append(path)
    if dirty:
        raise Refused(f'the client checkout {repo} has modified source files ({", ".join(dirty[:5])}): '
                      'commit or revert them first; a client-test pair must run on one client revision')
    return repo, head.strip(), sorted(untracked)


# The Loader's `.gradle/loom-cache` holds the remapped Minecraft dependency: without it a copy cannot
# compile offline (a reversed Java control would be unrunnable, not red). `build/` is excluded.
_LOADER_COPY_IGNORE = shutil.ignore_patterns('.git', 'build', 'node_modules', '*.log', 'run',
                                             'logs', '.idea', '.kotlin')


def _client_control_copy(repo, root, slot='ledger-client-control', ignore=None):
    """A disposable copy of a sibling checkout the fix's production hunks are reversed in.

    The checkout itself is never edited: a promotion reverses the real pre-image in a copy that
    carries the same source (every file except build output, `.git` and logs)."""
    ignore = ignore or _CLIENT_COPY_IGNORE
    parent = _worktree_parent(root) or Path(tempfile.mkdtemp(prefix='ledger-client-control-'))
    tree = parent / slot
    if tree.exists():
        shutil.rmtree(tree, ignore_errors=True)
    try:
        shutil.copytree(repo, tree, ignore=ignore, symlinks=True)
    except OSError as error:
        raise Refused(f'cannot copy the checkout {repo} for the control: {error}')
    return tree


def _client_test_assertion(tree, entry, changed_files):
    """Classify one client control red: an assertion inside test code of a file the fix changed.

    A client unit test lives either in a test file or in an inline `#[cfg(test)]` module of a production
    file, and `_panic_class` treats the latter as class (c) because the path is not a test path. Here the
    inline range is read from the control tree's own source, and the assert macro has to be on the line
    the panic names. A fixture/setup panic or an assertion elsewhere in the same file never counts."""
    assertion = entry.get('assertion') or {}
    file, line = assertion.get('file'), assertion.get('line')
    if not isinstance(file, str) or not isinstance(line, int):
        return 'unproven'
    if not os.path.isabs(file):
        file = str((Path(tree) / file).resolve())
    try:
        relative = Path(file).resolve().relative_to(Path(tree).resolve()).as_posix()
    except ValueError:
        return 'unproven'
    if relative not in changed_files:
        return 'unproven'
    try:
        source = Path(file).read_text(errors='surrogateescape')
    except OSError:
        return 'unproven'
    lines = source.splitlines()
    if not 1 <= line <= len(lines):
        return 'unproven'
    in_test = bool(_TEST_PATH.search(relative))
    if not in_test and relative.endswith('.rs'):
        in_test = any(first <= line <= last for first, last in _rust_test_ranges(source))
    if not in_test or not _ASSERT_LINE.search(lines[line - 1]):
        return 'unproven'
    return 'assertion'


def _promote_client_test_fix(ident, *, row, fix, root, tree_now, dry_run, ctx, client_runner=None):
    """A `client-test:<sha>` fix: the client unit ref is red with the fix's real production hunks
    reversed, and green on the client that contains the fix.

    The reversal is `git apply -R` of the fix commit's own non-test hunks in a copy of the client
    checkout: the a/- side is the commit's pre-image, never a hand-written re-break, and a hunk that no
    longer reverses refuses instead of falling back to another tree."""
    fix_full = client_fix_commit(fix, root)
    refs = row['regression']
    if not refs:
        raise Refused(f'{ident}: no registered check (regression is empty)')
    for ref in refs:
        try:
            regressions._parse_ref(ref)
        except ValueError as error:
            raise Refused(f'{ident}: {error}')
    client_refs = [ref for ref in refs if ref.startswith('client:')]
    if not client_refs:
        raise Refused(f'{ident}: a {CLIENT_TEST_PREFIX}<sha> fix needs a client:<crate>:<test> ref to carry '
                      'the pair (a harness ref would need a real-client control)')
    repo, head, untracked = _client_source_state(root)
    if not client_contains(root, head, fix_full):
        raise Refused(f'{ident}: the client checkout {repo} at {head[:12]} does not contain the fix '
                      f'{fix_full[:12]}: check out the client revision the row was fixed on')
    patch, reversed_files, kept_test_files = non_test_patch(repo, fix_full)
    if not patch:
        raise Refused(f'{ident}: fix commit {fix} changes no production hunk, so there is no pre-image to '
                      f'reverse: a {CLIENT_TEST_PREFIX} row proves a guard a client test observes')
    repair = f'client-test control of {ident}: the fix does not reverse on {repo} at {head[:12]}; commit the ' \
             'client revision that carries its pre-image (a hand-ported re-break is not a control)'
    control = restored = None
    with contextlib.ExitStack() as stack:
        stack.enter_context(_control_lock(root))
        tree = _client_control_copy(repo, root)
        stack.callback(shutil.rmtree, tree, True)
        check = _git(tree, 'apply', '-R', '--check', '--whitespace=nowarn', '-',
                     stdin=patch.encode(errors='surrogateescape'), check=False)
        if check[0]:
            raise Refused(repair)
        _git(tree, 'apply', '-R', '--whitespace=nowarn', '-', stdin=patch.encode(errors='surrogateescape'))
        runner = client_runner or (lambda refs, tree: _client_ref_states_of(_run_client(
            ctx, {_client_test(ref): ref for ref in refs}, root, tree)))
        control = runner(client_refs, tree)
        changed = set(reversed_files) | set(kept_test_files)
        for ref, entry in control.items():
            if entry.get('state') != RED:
                continue
            if _client_test_assertion(tree, entry, changed) != 'assertion':
                entry['detail'] = (entry.get('detail', '') + ' | the red is not an assertion inside test code '
                                   'of a file the fix changed').strip(' |')
                entry['state'] = UNPROVEN_PANIC
        red = {ref for ref, entry in control.items() if entry['state'] == RED}
        if not red:
            raise Refused(f'{ident}: the client unit refs stayed green with the fix reversed: '
                          + '; '.join(f"{ref}={entry['state']} ({entry['detail']})" for ref, entry in control.items())
                          + ' (a control that stays green does not test the fix)')
        restored = runner(refs, repo) if client_runner else _client_ref_states_of(
            _run_client(ctx, {_client_test(ref): ref for ref in refs}, root, repo))
        if not client_runner:
            others = [ref for ref in refs if not ref.startswith('client:')]
            if others:
                restored.update(run_refs(ctx, others, root, True))
        bad = {ref: entry for ref, entry in restored.items() if entry.get('state') != GREEN}
        if bad:
            raise Refused(f'{ident}: refs not green on the client that contains the fix: '
                          + '; '.join(f"{ref}={entry['state']} ({entry['detail']})" for ref, entry in bad.items()))
    receipt = {
        'id': ident, 'verdict': 'promoted', 'fix_commit': fix, 'fix_commit_full': fix_full,
        'promoted_at': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
        'tree_head': tree_now['head'], 'tree_dirty': tree_now['dirty'], 'tree_sha256': tree_now['sha256'],
        'client_fix': True, 'control': 'client_test_reversal', 'control_tree': 'client_head',
        'fix_patch_sha256': hashlib.sha256(patch.encode(errors='surrogateescape')).hexdigest(),
        'reversed_files': reversed_files, 'kept_test_files': kept_test_files,
        'client_checkout': {'path': str(repo), 'head': head, 'untracked': untracked},
        'negative_control': control, 'restored': restored, 'client_receipt': None,
        'artifact_dir': str(ctx.artifact_dir),
    }
    (ctx.artifact_dir / 'promote.json').write_text(json.dumps(receipt, indent=2) + '\n')
    if dry_run:
        return receipt
    _record_promotion(root, ident, receipt)
    return receipt


def _client_test(ref):
    """(crate, test) of a `client:` ref."""
    _, crate, test = ref.split(':', 2)
    return crate, test


def _client_ref_states_of(results):
    """{ref: {state, detail, assertion?}} from `_run_client` results keyed by (crate, test)."""
    out = {}
    for (crate, test), entry in results.items():
        assertion = entry[2] if len(entry) > 2 else None
        out[f'client:{crate}:{test}'] = {'state': entry[0], 'detail': entry[1],
                                         **({'assertion': assertion} if assertion else {})}
    return out


# ---------------------------------------------------------------- Loader fixes
# The Loader keeps its source in `../solaris-loader`; a fix there has no core ref kind. A loader fix is
# proved by reversing its own production hunks in a copy of that checkout for a loader python test ref
# (docs/bugs/README.md). The row names the files the control reverses (`control_files`) because one
# Loader commit often carries a Gradle/Java change this row's guard does not depend on.


def loader_repo(root=REPO_ROOT):
    """The Loader checkout a `loader:<sha>` fix_commit is resolved in (sibling repo or env override)."""
    from . import runtime

    return runtime.sibling_root('loader', root)


def loader_fix_commit(fix, root=REPO_ROOT):
    """Full sha of a `loader:<sha>` fix_commit in the Loader repository; Refused when it is unknown."""
    sha = fix[len(LOADER_FIX_PREFIX):]
    if not re.fullmatch(r'[0-9a-f]{7,40}', sha):
        raise Refused(f'fix_commit {fix}: expected {LOADER_FIX_PREFIX}<sha>')
    repo = loader_repo(root)
    if not repo.is_dir():
        raise Refused(f'fix_commit {fix}: no Loader checkout at {repo} (set KRASNOKAMEN_LOADER_ROOT)')
    code, full = _git(repo, 'rev-parse', '--verify', '--quiet', f'{sha}^{{commit}}', check=False)
    if code:
        raise Refused(f'fix_commit {fix}: {sha} is not a commit of the Loader repository {repo}')
    return full.strip()


def _run_loader(ctx, tests, root, repo=None):
    """Run loader python refs {(directory, 'module.test')} with cwd `<loader>/<directory>` -> results.

    The Loader's python tests import their siblings from the directory they live in
    (`from battle_scenario import ...`), so the ref names that directory; the module path is
    resolved there, never from this checkout."""
    repo = Path(repo) if repo is not None else loader_repo(root)
    if not repo.is_dir():
        return {test: (UNRUNNABLE, f'no Loader checkout at {repo} (set KRASNOKAMEN_LOADER_ROOT)', None)
                for test in tests}
    results = {}
    for directory, test in sorted(tests):
        cwd = repo / directory
        if not cwd.is_dir():
            results[(directory, test)] = (UNRUNNABLE, f'{directory} is not in the Loader checkout', None)
            continue
        argv = [sys.executable, '-m', 'unittest', test]
        ok, text = _exec(ctx, argv, f'loader-{test.rsplit(".", 1)[-1][:24]}', cwd, None)
        summary = re.search(r'(?m)^Ran (\d+) tests? in ', text)
        skipped = re.search(r'\bskipped=(\d+)', text)
        count = int(summary.group(1)) if summary else 0
        skipped_count = int(skipped.group(1)) if skipped else 0
        if not summary or count <= skipped_count or re.search(r'(?m)^ERROR:', text):
            results[(directory, test)] = (UNRUNNABLE, 'no executed test or unittest error; see log', None)
        elif ok and re.search(r'(?m)^OK(?: \(|$)', text):
            results[(directory, test)] = (GREEN, '', None)
        elif not ok and re.search(r'(?m)^FAIL:', text):
            results[(directory, test)] = (RED, 'test assertion failed', _python_assertion(text))
        else:
            results[(directory, test)] = (UNRUNNABLE, 'no conclusive unittest result; see log', None)
    return results


def _gradle_junit(module_dir, selector):
    """(state, detail, assertion) of one Gradle test selector from its JUnit XML.

    Gradle writes `<module>/build/test-results/test/TEST-*.xml`; a failure counts as red only when the
    report says `AssertionError` and names the source line, so a compile error, a missing report or a
    timeout cannot stand as a control."""
    import xml.etree.ElementTree as ET

    results = module_dir / 'build' / 'test-results' / 'test'
    if not results.is_dir():
        return UNRUNNABLE, 'no Gradle test report (build or launch error)', None
    class_name = selector.rsplit('.', 1)[-1]
    suites = [path for path in sorted(results.glob('TEST-*.xml')) if class_name in path.name]
    if not suites:
        return UNRUNNABLE, f'no JUnit report for {class_name}', None
    passed = failed = 0
    for path in suites:
        try:
            root = ET.parse(path).getroot()
        except (OSError, ET.ParseError) as error:
            return UNRUNNABLE, f'JUnit report {path.name} unreadable: {error}', None
        for case in root.iter('testcase'):
            failure = case.find('failure')
            if failure is None:
                if case.find('skipped') is None:
                    passed += 1
                continue
            failed += 1
            kind = failure.get('type') or ''
            text = failure.text or ''
            position = re.search(r'\(([\w.$]+)\.java:(\d+)\)', text) or re.search(r'([\w.$]+)\.java:(\d+)', text)
            if 'AssertionError' not in kind and 'AssertionError' not in text:
                return UNRUNNABLE, f'{case.get("name")} failed with {kind or "an error"}, not an assertion', None
            if position is None:
                continue
            simple = position.group(1).rsplit('.', 1)[-1]
            sources = sorted(module_dir.glob(f'**/{simple}.java'))
            if not sources:
                continue
            return RED, f'{case.get("name")} AssertionError', {
                'file': str(sources[0]), 'line': int(position.group(2)), 'error': 'AssertionError'}
    if failed:
        return UNRUNNABLE, 'a failure carried no assertion location', None
    if passed:
        return GREEN, '', None
    return UNRUNNABLE, 'no executed test in the report', None


def _run_loader_gradle(ctx, tests, root, repo=None):
    """Run loader Gradle refs {(module, selector)} -> {(module, selector): (state, detail, assertion)}.

    The guard of a Loader Java fix is a JUnit test, so the control has to run Gradle in the tree it
    reverses. One invocation per selector keeps the failure attribution exact."""
    repo = Path(repo) if repo is not None else loader_repo(root)
    if not repo.is_dir():
        return {test: (UNRUNNABLE, f'no Loader checkout at {repo} (set KRASNOKAMEN_LOADER_ROOT)', None)
                for test in tests}
    gradlew = repo / 'gradlew'
    if not gradlew.is_file():
        return {test: (UNRUNNABLE, f'no Gradle wrapper at {gradlew}', None) for test in tests}
    results = {}
    client_jar = _loader_client_jar(repo)
    for module, selector in sorted(tests):
        module_dir = repo / module
        if not module_dir.is_dir():
            results[(module, selector)] = (UNRUNNABLE, f'{module} is not in the Loader checkout', None)
            continue
        argv = [str(gradlew), f':{module}:test', '--tests', selector, '--console=plain', '--no-daemon']
        if client_jar is not None:
            # `java-agent` defaults the Minecraft jar to `<loader>/../solaris/...`, which is wrong in a
            # reversal copy; the property is the build's own override.
            argv.append(f'-Pkrasnokamen.clientJar={client_jar}')
        _exec(ctx, argv, f'loader-gradle-{selector.rsplit(".", 1)[-1][:20]}', repo, None)
        results[(module, selector)] = _gradle_junit(module_dir, selector)
    return results


def _loader_client_jar(repo):
    """The Minecraft client jar `java-agent` compiles against, resolved from the Loader checkout.

    The build defaults it to `<loader>/../solaris/.analysis/client-automation/versions/<v>/client.jar`
    (a sibling checkout); a copy of the Loader elsewhere loses it, and a Java control would be
    unrunnable rather than red. Resolved here and passed as the build's own `-P` override."""
    properties = Path(repo) / 'gradle.properties'
    try:
        text = properties.read_text()
    except OSError:
        return None
    match = re.search(r'^minecraftVersion=(\S+)$', text, re.M)
    if not match:
        return None
    jar = Path(repo).resolve().parent / 'solaris/.analysis/client-automation/versions' / match.group(1) / 'client.jar'
    return str(jar) if jar.is_file() else None


def _loader_source_state(root):
    """(repo, head, untracked source files) of the Loader checkout; Refused when its source is dirty."""
    repo = loader_repo(root)
    if not repo.is_dir():
        raise Refused(f'no Loader checkout at {repo} (set KRASNOKAMEN_LOADER_ROOT)')
    _, head = _git(repo, 'rev-parse', 'HEAD')
    _, status = _git(repo, 'status', '--porcelain')
    dirty, untracked = [], []
    for line in status.splitlines():
        state, path = line[:2], line[3:].strip('"')
        if not path.endswith(('.py', '.java', '.kts', '.gradle', '.properties', '.toml')):
            continue
        (untracked if state == '??' else dirty).append(path)
    if dirty:
        raise Refused(f'the Loader checkout {repo} has modified source files ({", ".join(dirty[:5])}): '
                      'commit or revert them first; a loader control must run on one Loader revision')
    return repo, head.strip(), sorted(untracked)


def _loader_control_patch(repo, fix_full, control_files):
    """The fix commit's hunks for `control_files` only; Refused on anything else.

    `control_files` exists because one Loader commit often bundles a Gradle dependency or a Java
    production change this row's python guard does not depend on (reversing those breaks the run or
    proves another row). Each listed path must be a non-test file the commit changes; the patch is
    exactly those chunks, so the a/- side is still the commit's own pre-image."""
    _, diff = _git(repo, 'diff', '--no-renames', '--no-color', f'{fix_full}^1', fix_full)
    chunks, changed = {}, set()
    for chunk in re.split(r'(?m)^(?=diff --git )', diff):
        if not chunk:
            continue
        match = re.match(r'diff --git a/(.+) b/(.+)$', chunk.splitlines()[0])
        if match is None:
            raise Refused(f'fix commit {fix_full[:12]} has an unparseable diff header')
        changed.add(match.group(2))
        chunks[match.group(2)] = chunk
    for path in control_files:
        if _skipped_doc(path) or _TEST_PATH.search(path):
            raise Refused(f'control_files names {path}, a test or docs path: a control reverses production hunks')
    missing = sorted(set(control_files) - changed)
    if missing:
        raise Refused(f'control_files names {", ".join(missing)}, which the fix commit does not change')
        if any(line.startswith(('+++ /dev/null', '--- /dev/null', 'new file mode', 'deleted file mode'))
               for line in chunks[path].splitlines()[:4]):
            raise Refused(f'control_files names {path}, which the fix commit creates or deletes: a reversal '
                          'of it is not a pre-image')
    return ''.join(chunks[path] for path in control_files), list(control_files)


def _loader_assertion(entry, tree):
    """A loader control red only when the failing unittest frame is inside a test file of the reversed tree.

    The test may predate the fix (a guard older than the finding), so it is not required to be one of the
    fix commit's own files: what proves the control is an AssertionError raised in test code, never a
    production import error or a unittest setup error."""
    assertion = entry.get('assertion')
    if not isinstance(assertion, dict) or assertion.get('error') != 'AssertionError':
        return False
    path = assertion.get('file')
    if not isinstance(path, str) or not os.path.isabs(path):
        return False
    try:
        relative = Path(path).resolve().relative_to(Path(tree).resolve()).as_posix()
    except ValueError:
        return False
    return bool(_TEST_PATH.search(relative))


def _promote_loader_fix(ident, *, row, fix, root, tree_now, dry_run, ctx, loader_runner=None):
    """A `loader:<sha>` fix: the loader python ref is red with the row's production hunks reversed."""
    fix_full = loader_fix_commit(fix, root)
    refs = row['regression']
    if not refs:
        raise Refused(f'{ident}: no registered check (regression is empty)')
    for ref in refs:
        try:
            regressions._parse_ref(ref)
        except ValueError as error:
            raise Refused(f'{ident}: {error}')
    loader_refs = [ref for ref in refs if ref.startswith('loader:')]
    loader_python = [ref for ref in loader_refs if ref.split(':', 2)[1] == 'python']
    loader_gradle = [ref for ref in loader_refs if ref.split(':', 2)[1] == 'gradle']
    if not loader_refs:
        raise Refused(f'{ident}: a {LOADER_FIX_PREFIX}<sha> fix needs a loader:python:<dir>:<module>.<test> ref '
                      'to carry the pair')
    repo, head, untracked = _loader_source_state(root)
    if _git(repo, 'merge-base', '--is-ancestor', fix_full, head, check=False)[0]:
        raise Refused(f'{ident}: the Loader checkout {repo} at {head[:12]} does not contain the fix '
                      f'{fix_full[:12]}')
    control_files = row.get('control_files')
    if not isinstance(control_files, list) or not control_files:
        raise Refused(f'{ident}: a {LOADER_FIX_PREFIX} row records control_files (the non-test files whose '
                      'hunks the control reverses); one Loader commit usually bundles changes this guard '
                      'does not depend on')
    patch, reversed_files = _loader_control_patch(repo, fix_full, control_files)
    _, _, kept_tests = non_test_patch(repo, fix_full)
    control = restored = None
    with contextlib.ExitStack() as stack:
        stack.enter_context(_control_lock(root))
        tree = _client_control_copy(repo, root, 'ledger-loader-control', _LOADER_COPY_IGNORE)
        stack.callback(shutil.rmtree, tree, True)
        if _git(tree, 'apply', '-R', '--check', '--whitespace=nowarn', '-',
                stdin=patch.encode(errors='surrogateescape'), check=False)[0]:
            raise Refused(f'loader control of {ident}: the fix hunks do not reverse on {repo} at {head[:12]}; '
                          'commit the Loader revision that carries their pre-image (a hand-ported re-break is '
                          'not a control)')
        _git(tree, 'apply', '-R', '--whitespace=nowarn', '-', stdin=patch.encode(errors='surrogateescape'))
        def run_refs_here(refs, tree):
            states = {}
            if loader_python:
                states.update(_loader_ref_states(_run_loader(
                    ctx, {_loader_test(ref) for ref in refs if ref in loader_python}, root, tree), 'python'))
            if loader_gradle:
                states.update(_loader_ref_states(_run_loader_gradle(
                    ctx, {_loader_test(ref) for ref in refs if ref in loader_gradle}, root, tree), 'gradle'))
            return states

        runner = loader_runner or run_refs_here
        control = runner(loader_refs, tree)
        for ref, entry in control.items():
            if entry.get('state') == RED and not _loader_assertion(entry, tree):
                entry['state'] = UNPROVEN_PANIC
                entry['detail'] = (entry.get('detail', '') + ' | the red is not an AssertionError inside a '
                                   'test file the fix changed').strip(' |')
        red = {ref for ref, entry in control.items() if entry['state'] == RED}
        if not red:
            raise Refused(f'{ident}: the loader refs stayed green with the fix reversed: '
                          + '; '.join(f"{ref}={entry['state']} ({entry['detail']})" for ref, entry in control.items()))
        restored = runner(refs, repo)
        if not loader_runner:
            others = [ref for ref in refs if not ref.startswith('loader:')]
            if others:
                restored.update(run_refs(ctx, others, root, True))
        bad = {ref: entry for ref, entry in restored.items() if entry.get('state') != GREEN}
        if bad:
            raise Refused(f'{ident}: refs not green on the Loader that contains the fix: '
                          + '; '.join(f"{ref}={entry['state']} ({entry['detail']})" for ref, entry in bad.items()))
    receipt = {
        'id': ident, 'verdict': 'promoted', 'fix_commit': fix, 'fix_commit_full': fix_full,
        'promoted_at': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
        'tree_head': tree_now['head'], 'tree_dirty': tree_now['dirty'], 'tree_sha256': tree_now['sha256'],
        'loader_fix': True, 'control': 'loader_reversal', 'control_tree': 'loader_head',
        'fix_patch_sha256': hashlib.sha256(patch.encode(errors='surrogateescape')).hexdigest(),
        'reversed_files': reversed_files, 'kept_test_files': kept_tests,
        'loader_checkout': {'path': str(repo), 'head': head, 'untracked': untracked},
        'negative_control': control, 'restored': restored, 'client_receipt': None,
        'artifact_dir': str(ctx.artifact_dir),
    }
    (ctx.artifact_dir / 'promote.json').write_text(json.dumps(receipt, indent=2) + '\n')
    if dry_run:
        return receipt
    _record_promotion(root, ident, receipt)
    return receipt


def _loader_test(ref):
    """(directory, 'module.test') of a `loader:python:<dir>:<module>.<test>` ref."""
    _, _, rest = ref.split(':', 2)
    directory, _, test = rest.partition(':')
    return directory, test


def _loader_ref_states(results, runner='python'):
    """{ref: {state, detail, assertion?}} from `_run_loader`/`_run_loader_gradle` results."""
    out = {}
    for (location, selector), entry in results.items():
        assertion = entry[2] if len(entry) > 2 else None
        out[f'loader:{runner}:{location}:{selector}'] = {'state': entry[0], 'detail': entry[1],
                                                         **({'assertion': assertion} if assertion else {})}
    return out


def promote(ident, *, root=REPO_ROOT, client_receipt=None, client_control_receipt=None, fix_patch=None,
            dry_run=False, ctx, runner=run_refs, client_runner=None, loader_runner=None):
    """Run every promotion check for `ident`; on success (and not dry_run) flip it to fixed. Refused on any failure.

    The candidate is the exact tree: HEAD plus uncommitted diff and untracked files (docs/bugs aside).
    A `client:<sha>` fix_commit is a client-repository fix: its control is a red real-client receipt of
    the pre-fix client and its restore a green one of the client that contains the fix (docs/bugs/README.md)."""
    row = next((bug for bug in load_rows(root) if bug['id'] == ident), None)
    if row is None:
        raise Refused(f'unknown ledger id {ident}')
    if row['status'] == 'fixed' and not dry_run:
        raise Refused(f'{ident} is already fixed (only --dry-run may re-audit a fixed row)')
    tree_now = tree_fingerprint(root)
    head = tree_now['head']
    fix = row.get('fix_commit')
    if not fix:
        raise Refused(f'{ident}: no fix_commit recorded')
    if fix.startswith('client:') and row.get('test_only_fix') is True:
        raise Refused(f'{ident}: test_only_fix is for a committed core test fix; a client:<sha> fix is proved '
                      'by real-client receipts, never by a reversed test oracle')
    if fix.startswith(LOADER_FIX_PREFIX):
        if row.get('test_only_fix') is True:
            raise Refused(f'{ident}: test_only_fix is for a committed core test oracle, not a loader fix')
        if client_receipt or client_control_receipt:
            raise Refused(f'{ident}: a {LOADER_FIX_PREFIX}<sha> fix is proved by reversing its own hunks for a '
                          'loader ref, not by real-client receipts')
        return _promote_loader_fix(ident, row=row, fix=fix, root=root, tree_now=tree_now, dry_run=dry_run,
                                   ctx=ctx, loader_runner=loader_runner)
    if fix.startswith(CLIENT_TEST_PREFIX):
        if row.get('test_only_fix') is True:
            raise Refused(f'{ident}: test_only_fix (a core test oracle plus its defect patch) is not the '
                           f'{CLIENT_TEST_PREFIX} kind; drop the flag and let the reversal prove the client fix')
        if client_receipt or client_control_receipt:
            raise Refused(f'{ident}: a {CLIENT_TEST_PREFIX}<sha> fix is proved by reversing its own production '
                          'hunks for the registered client unit ref, not by real-client receipts')
        return _promote_client_test_fix(ident, row=row, fix=fix, root=root, tree_now=tree_now, dry_run=dry_run,
                                        ctx=ctx, client_runner=client_runner)
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
    test_only = row.get('test_only_fix') is True
    if test_only and patch_mode:
        raise Refused(f'{ident}: test_only_fix is only for a committed fix_commit, not {fix}')
    patch, fix_patch_text = None, None
    if patch_mode:
        if not fix_patch:
            raise Refused(f'{ident}: fix_commit is {fix}: supply the patch with --fix-patch <file>')
        fix_patch_text = Path(fix_patch).read_text(errors='surrogateescape')
    elif fix_patch:
        raise Refused(f'{ident}: --fix-patch is only for a fix_commit of the form patch:<sha256>')
    if not patch_mode:
        if test_only:
            _, diff = _git(root, 'diff', '--no-renames', '--no-color', f'{full}^1', full)
            patch, reversed_files = _split_test_hunks(diff, row.get('control_a_files'))
            test_files = []
            if not patch:
                raise Refused(f'{ident}: fix commit {fix} changes no test file: test_only_fix needs a fix in test files')
        else:
            patch, reversed_files, test_files = non_test_patch(root, full)
            if not patch:
                raise Refused(f'{ident}: fix commit {fix} has no non-test hunks to reverse')
    reverse = ['apply', '-R', '--whitespace=nowarn', '-']
    defect_control, defect, defect_sha, defect_rel, proving = None, None, None, None, []
    if test_only:
        # M2: the defect patch that proves the corrected test is the committed one. The worktree is
        # not the source (it can change between the audit and the promotion), so the bytes applied
        # are `git show HEAD:<path>`, hashed and cross-checked against the row's control_b_sha256.
        defect_rel = row.get('control_b_patch')
        if not isinstance(defect_rel, str) or not defect_rel:
            raise Refused(f'{ident}: test_only_fix needs control_b_patch (a committed defect-injection patch)')
        if defect_rel.startswith('/') or '..' in defect_rel.split('/'):
            raise Refused(f'{ident}: control_b_patch must be a repo-relative path, got {defect_rel!r}')
        code, committed = _git(root, 'show', f'HEAD:{defect_rel}', check=False)
        if code:
            raise Refused(f'{ident}: control_b_patch {defect_rel} is not committed at HEAD '
                          '(an untracked defect patch can change between audit and promotion)')
        defect = committed.encode(errors='surrogateescape')
        defect_sha = hashlib.sha256(defect).hexdigest()
        if row.get('control_b_sha256') != defect_sha:
            raise Refused(f'{ident}: control_b_sha256 is {row.get("control_b_sha256")!r}, but the committed '
                          f'{defect_rel} hashes to {defect_sha}; record the committed patch hash on the row')
        try:
            worktree_bytes = (root / defect_rel).read_bytes()
        except OSError:
            raise Refused(f'{ident}: control_b_patch {defect_rel} is not in the worktree') from None
        if worktree_bytes != defect:
            raise Refused(f'{ident}: the worktree copy of {defect_rel} differs from HEAD; commit it first '
                          '(the applied bytes are the committed ones)')
    with contextlib.ExitStack() as stack:
        stack.enter_context(_control_lock(root))
        # Restore always runs on the exact tree being promoted: its HEAD plus any uncommitted diff and
        # untracked files (verified against the fingerprint), never on fix_commit.
        candidate = stack.enter_context(_worktree(root, head, 'candidate'))
        _materialize(root, candidate, tree_now)
        if test_only:
            missing = _uncovered_test_files(candidate, reversed_files, refs)
            if missing:
                raise Refused(f"{ident}: test_only_fix has no ref whose test is defined in {', '.join(missing)} "
                              '(every changed test file must hold the exact test the row registers)')
            _check_defect_patch(ident, defect, candidate, defect_rel)
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
        elif test_only:
            # M3: control A restores the old oracle on current code, so a `fix_commit` worktree is
            # never a control for a test-only row (its tree predates the fix's own other commits).
            raise Refused(f"{ident}: the fix's test hunks do not reverse onto the tree being promoted, so "
                          'control A cannot run against current code (control_tree must be head)')
        else:
            control_tree, control_dir = 'fix_commit', stack.enter_context(_worktree(root, full, 'fix'))
        # Stage the candidate content first: a dirty tree's unstaged diff would otherwise mix with
        # the reversal, and control B's changed-line ranges are read from that diff (review N-b).
        _git(control_dir, 'add', '-A')
        _git(control_dir, *reverse, stdin=patch.encode(errors='surrogateescape'))
        control = _classify_control(ident, runner(ctx, refs, control_dir, True), row, control_dir)
        valid_red = {ref for ref, entry in control.items()
                     if entry['state'] == RED and _assertion_in_files(entry, control_dir, reversed_files)} \
            if test_only else {ref for ref, entry in control.items() if entry['state'] == RED}
        if not valid_red:
            raise Refused(f"{ident}: negative control ({control_tree}) did not go red at an assertion with the fix reversed: "
                          + '; '.join(f"{ref}={entry['state']}" + (f" ({entry['detail']})" if entry['detail'] else '') for ref, entry in control.items())
                          + (f" (a red outside {', '.join(reversed_files)} is not evidence: a production panic or a build "
                             'failure is not control A)' if test_only else
                             ' (a check that stays green does not test the fix; one that cannot build or run without it cannot show an assertion failing)'))
        if test_only:
            # Control B: restoring the old oracle only shows the old assertion was red, which a wrongly
            # deleted assertion also gives. Re-introduce the defect the test claims to catch and require
            # the corrected test to go red, so the guard's own direction is proven too.
            # The reversed hunks' line numbers in the candidate come from the reversal just applied,
            # read before the tree is reset for control B (git applies at an offset once a file grows).
            # Context lines do not count: the red must sit on a changed line, or on an operator that
            # starts at the red line and spans one (a multi-line assert), or be named by the row.
            _, applied = _git(control_dir, 'diff', '-U0', '--no-renames', '--no-color')
            changed = _changed_old_lines(applied)
            _materialize(root, candidate, tree_now)
            if _git(candidate, 'apply', '--check', '--whitespace=nowarn', '-', stdin=defect, check=False)[0]:
                raise Refused(f"{ident}: control_b_patch {row['control_b_patch']} does not apply on the tree being promoted")
            _git(candidate, 'apply', '--whitespace=nowarn', '-', stdin=defect)
            defect_control = _classify_control(ident, runner(ctx, refs, candidate, True), row, candidate)
            def in_changed(entry, ref):
                assertion = entry.get('assertion') or {}
                file, line = assertion.get('file'), assertion.get('line')
                if not isinstance(file, str) or not isinstance(line, int):
                    return False
                if os.path.isabs(file):
                    try:
                        file = Path(file).resolve().relative_to(Path(candidate).resolve()).as_posix()
                    except ValueError:
                        return False
                lines = changed.get(file, set())
                if line in lines:
                    return True
                span = _operator_span(candidate, file, line)
                if span and any(first <= changed_line <= span[1] for changed_line in lines
                                for first in [span[0]]):
                    return True
                return row_named.get(ref) == f'{file}:{line}'

            row_named = row.get('proving_assertions') if isinstance(row.get('proving_assertions'), dict) else {}
            valid_b = {ref for ref, entry in defect_control.items()
                       if entry['state'] == RED and _assertion_in_files(entry, candidate, reversed_files)
                       and in_changed(entry, ref)}
            if not valid_b:
                raise Refused(f"{ident}: defect-injection control ({defect_rel}) did not go red at an assertion "
                              f"inside a hunk the fix reversed in {', '.join(reversed_files)}: "
                              + '; '.join(f"{ref}={entry['state']}" + (f" ({entry['detail']})" if entry['detail'] else '') for ref, entry in defect_control.items())
                              + ' (the corrected test does not catch the behaviour it claims to guard)')
            proving = sorted(valid_red & valid_b)
            if not proving:
                raise Refused(f'{ident}: control A red {sorted(valid_red)} and control B red {sorted(valid_b)} do '
                              'not overlap: the same test has to flip both ways, so the corrected oracle is '
                              'proven in its own direction and not merely the old assertion')
            for ref in proving:
                if _ref_test_file(candidate, ref, reversed_files) is None:
                    raise Refused(f'{ident}: proving ref {ref} is not defined in '
                                  f"{', '.join(reversed_files)}: a bare name from another file proves nothing")
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
        'control': 'test_hunks' if test_only else 'reversed_fix_hunks',
        'control_b': ({'patch': defect_rel, 'sha256': defect_sha, 'proving': proving,
                       'negative_control': defect_control} if test_only else None),
        'fix_patch_sha256': fix[len('patch:'):] if patch_mode else None,
        'reversed_files': reversed_files, 'kept_test_files': test_files,
        'negative_control': control, 'restored': restored, 'client_receipt': receipt_info,
        'artifact_dir': str(ctx.artifact_dir),
    }
    (ctx.artifact_dir / 'promote.json').write_text(json.dumps(receipt, indent=2) + '\n')
    if dry_run:
        return receipt
    _record_promotion(root, ident, receipt)
    return receipt


# ------------------------------------------------- graphical control (backfill)


def _reversed_patch(root, row, promotion):
    """The patch a promoted row's control reversed: its committed patch, or the fix commit's production hunks."""
    fix = row['fix_commit']
    if fix.startswith('patch:'):
        path = regressions._patch_file(row['id'], fix[len('patch:'):])
        if path is None:
            raise Refused(f"{row['id']}: no committed patch for {fix}; nothing to reverse for the control")
        return path.read_text(errors='surrogateescape')
    fix_full = client_fix_commit(fix, root) if client_fix_kind(fix) else None
    if fix_full is not None:
        raise Refused(f"{row['id']}: a client fix carries its own receipt pair; nothing to backfill")
    code, full = _git(root, 'rev-parse', '--verify', '--quiet', f'{fix}^{{commit}}', check=False)
    if code:
        raise Refused(f"{row['id']}: fix_commit {fix} is not a commit of this repository")
    patch, _, _ = non_test_patch(root, full.strip())
    if not patch:
        raise Refused(f"{row['id']}: fix commit {fix} has no production hunk to reverse")
    return patch


def _graphical_backfill(ident, promotion, entries, recorded):
    """The promotion receipt's backfill record with `recorded` appended (old dict shape normalised)."""
    base = promotion.get('graphical_control_backfill')
    history = list(base) if isinstance(base, list) else ([base] if isinstance(base, dict) else [])
    history.append(recorded)
    new = dict(promotion)
    new['negative_control'] = entries
    new['graphical_control_backfill'] = history
    return new


def graphical_control(ident, *, root=REPO_ROOT, receipt=None, control_tree=None, dry_run=False, ctx, runner=run_refs):
    """Record the graphical negative control of an already promoted row (backfill).

    Promotion of a client-visible row records its graphical control `unrunnable` when no build token
    allowed a reverted-pin run (`docs/bugs/README.md`). This re-runs only those graphical refs against
    the reversed tree, or validates a receipt of such a run, and writes the red into the row's
    promotion receipt. It never changes a row's status and never accepts a green or a non-assertion."""
    row = next((bug for bug in load_rows(root) if bug['id'] == ident), None)
    if row is None:
        raise Refused(f'unknown ledger id {ident}')
    if row['status'] != 'fixed':
        raise Refused(f'{ident} is {row["status"]}, not fixed: backfill records the control of a promotion, '
                      'it never promotes')
    promotion_rel = row.get('promotion')
    promotion_path = root / str(promotion_rel)
    try:
        promotion = json.loads(promotion_path.read_text())
    except (OSError, ValueError) as error:
        raise Refused(f'{ident}: promotion receipt {promotion_rel} unreadable: {error}')
    if promotion.get('id') != ident or promotion.get('fix_commit') != row.get('fix_commit'):
        raise Refused(f'{ident}: {promotion_rel} does not describe this row and fix_commit')
    if promotion.get('client_fix'):
        raise Refused(f'{ident}: a client fix already records its receipt pair; there is no unrunnable '
                      'graphical control to backfill')
    if bool(receipt) == bool(control_tree):
        raise Refused(f'{ident}: give exactly one of --receipt <result.json> (record a run) or '
                      '--control-tree <dir> (run the graphical refs there)')
    graphical = [ref for ref in row['regression'] if regressions.is_graphical_ref(ref)]
    if not graphical:
        raise Refused(f'{ident}: the row registers no graphical ref, so it has no graphical control')
    entries = dict(promotion.get('negative_control') or {})
    pending = [ref for ref in graphical if (entries.get(ref) or {}).get('state') != RED]
    if not pending:
        raise Refused(f'{ident}: every graphical ref already records a red control; nothing to backfill')
    proving = row.get('proving_assertions') if isinstance(row.get('proving_assertions'), dict) else {}
    record = {'at': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
              'refs': pending, 'tree_head': promotion.get('tree_head')}
    if receipt:
        path = Path(receipt).resolve()
        try:
            data = json.loads(path.read_text())
        except (OSError, ValueError) as error:
            raise Refused(f'{ident}: receipt {receipt} unreadable: {error}')
        if data.get('status') != 'failed':
            raise Refused(f"{ident}: receipt {receipt} is {data.get('status')!r}, not a failed control run")
        recorded_tree = data.get('tree')
        if not isinstance(recorded_tree, dict) or recorded_tree.get('head') != promotion.get('tree_head'):
            raise Refused(f"{ident}: receipt {receipt} ran at "
                          f"{(recorded_tree or {}).get('head')!r}, not the promoted tree {promotion.get('tree_head')!r}")
        if recorded_tree.get('sha256') == promotion.get('tree_sha256'):
            raise Refused(f'{ident}: receipt {receipt} ran the candidate tree itself, not the reversed control '
                          '(the reversal changes the tree fingerprint)')
        client_head = _client_pin_of_receipt(root, data, path)
        states = _client_ref_states(data, pending, proving)
        red = [ref for ref in pending if states.get(ref) == RED]
        if not red:
            raise Refused(f'{ident}: receipt {receipt} is not red on any pending graphical ref ('
                          + ', '.join(f'{ref}={states.get(ref) or "unreported"}' for ref in pending)
                          + '); a missing mapping, timeout or launch error cannot carry the control')
        for ref in red:
            assertion = _failed_assertion(data.get('details', {}), proving.get(ref))
            entries[ref] = {'state': 'red', 'detail': f'backfilled from {path}', 'receipt': str(path),
                            **({'assertion': assertion} if assertion else {})}
        record.update({'receipt': str(path), 'refs': red,
                       'control_tree': {'head': recorded_tree.get('head'), 'sha256': recorded_tree.get('sha256')},
                       'client_head': client_head})
    else:
        tree = Path(control_tree).resolve()
        if not (tree / '.git').exists():
            raise Refused(f'{ident}: --control-tree {tree} is not a git worktree of this repository')
        fingerprint = tree_fingerprint(tree)
        if fingerprint['head'] != promotion.get('tree_head'):
            raise Refused(f"{ident}: control tree {tree} is at {fingerprint['head'][:12]}, not the promoted "
                          f"{str(promotion.get('tree_head'))[:12]}")
        if fingerprint['sha256'] == promotion.get('tree_sha256'):
            raise Refused(f'{ident}: control tree {tree} is the candidate itself; the fix has to be reversed in it')
        patch = _reversed_patch(root, row, promotion)
        if _git(tree, 'apply', '--check', '--whitespace=nowarn', '-', stdin=patch.encode(errors='surrogateescape'),
                check=False)[0]:
            raise Refused(f'{ident}: the fix does not apply to {tree}, so that tree is not the reversed control '
                          '(reverse the fix hunks in it, not a hand-written re-break)')
        ran = _classify_control(ident, runner(ctx, pending, tree, True), row, tree)
        red = [ref for ref in pending if ran.get(ref, {}).get('state') == RED]
        if not red:
            raise Refused(f'{ident}: the graphical refs are not red on the reversed tree '
                          + '; '.join(f'{ref}={ran.get(ref, {}).get("state")} '
                                      f'({ran.get(ref, {}).get("detail")})' for ref in pending))
        for ref in red:
            detail = str(ran[ref].get('detail', ''))
            assertion = proving.get(ref) or re.search(r'assertion failed: (\S+)', detail)
            if not isinstance(assertion, str) and assertion is not None:
                assertion = assertion.group(1)
            entries[ref] = {'state': 'red', 'detail': detail, 'receipt': ran[ref].get('receipt'),
                            **({'assertion': assertion} if isinstance(assertion, str) else {})}
        record.update({'control_tree': {'head': fingerprint['head'], 'sha256': fingerprint['sha256']},
                       'refs': red, 'artifact_dir': str(ctx.artifact_dir)})
    updated = _graphical_backfill(ident, promotion, entries, record)
    (ctx.artifact_dir / 'graphical-control.json').write_text(json.dumps(updated, indent=2) + '\n')
    if dry_run:
        return updated
    promotion_path.write_text(json.dumps(updated, indent=2) + '\n')
    return updated


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
    control_parser = sub.add_parser('graphical-control',
                                    help='backfill the graphical control of an already promoted row (never promotes)')
    control_parser.add_argument('id')
    control_parser.add_argument('--receipt', type=Path, default=None,
                                help='failed real-client result.json of the reversed-tree control to record')
    control_parser.add_argument('--control-tree', type=Path, default=None,
                                help='a worktree at the promoted head with the fix reversed: run only its graphical refs')
    control_parser.add_argument('--dry-run', action='store_true', help='validate, write nothing')
    show_parser = sub.add_parser('show', help='print one row (no id: every row as one document)')
    show_parser.add_argument('id', nargs='?', default=None)
    show_parser.add_argument('--field', default=None, help='print only this field of the row')
    show_parser.add_argument('--out', type=Path, default=None, help='write the text here instead of stdout')
    list_parser = sub.add_parser('list', help='one line per row: id area status severity')
    list_parser.add_argument('--area', choices=('gameplay', 'client', 'harness'), default=None)
    list_parser.add_argument('--status', choices=('open', 'fixed'), default=None)
    edit_parser = sub.add_parser('edit', help='set fields of one row file (`status`/`promotion` stay with promote)')
    edit_parser.add_argument('id')
    edit_parser.add_argument('--set', action='append', default=[], metavar='KEY=VALUE', help='TOML value text, e.g. --set \'title = "new title"\'')
    edit_parser.add_argument('--dry-run', action='store_true', help='print the result, write nothing')
    split_parser = sub.add_parser('split', help='convert the single-file ledger into docs/bugs/rows/<ID>.toml')
    split_parser.add_argument('--source', type=Path, default=None, help='the single-file ledger (default docs/bugs/ledger.toml)')
    args = parser.parse_args(argv)
    # Worktrees must share the invoking tree's build cache, not build their own target.
    os.environ.setdefault('CARGO_TARGET_DIR', str(REPO_ROOT / 'target'))
    if args.action in ('show', 'list', 'edit', 'split'):
        try:
            if args.action == 'show':
                if args.field and not args.id:
                    parser.error('--field needs a row id')
                text = show(REPO_ROOT, args.id, field=args.field) if args.id else document(REPO_ROOT)
                if args.out:
                    args.out.parent.mkdir(parents=True, exist_ok=True)
                    args.out.write_text(text, encoding='utf-8')
                else:
                    print(text, end='' if text.endswith('\n') else '\n')
            elif args.action == 'list':
                for row in load_rows(REPO_ROOT):
                    if args.area and row.get('area') != args.area:
                        continue
                    if args.status and row.get('status') != args.status:
                        continue
                    print(f"{row['id']}\t{row.get('area', '')}\t{row.get('status', '')}\t{row.get('severity', '')}")
            elif args.action == 'edit':
                if not args.set:
                    parser.error('edit needs at least one --set key=value')
                text = edit(REPO_ROOT, args.id, args.set, dry_run=args.dry_run)
                print(f"[ledger] {'unchanged (dry run): ' + args.id if args.dry_run else 'edited ' + args.id} "
                      f"{_relative(REPO_ROOT, row_path(REPO_ROOT, args.id))}")
                if args.dry_run:
                    print(text, end='' if text.endswith('\n') else '\n')
            else:
                count = split(REPO_ROOT, args.source)
                print(f'[ledger] split {count} rows into {ROW_DIR}')
        except (Refused, ValueError) as error:
            print(f'ledger {args.action}: {error}', file=sys.stderr)
            return 2
        return 0
    if args.action == 'probe':
        if bool(args.ids) == args.open:
            parser.error('give row ids or --open')
        bugs = load_rows(REPO_ROOT)
        ids = args.ids or [bug['id'] for bug in bugs if bug['status'] == 'open']
        tree_before = tree_fingerprint(REPO_ROOT)
        ctx = _artifact_ctx('probe')
        try:
            rows = probe(bugs, ids, client=args.client, ctx=ctx)
        except Refused as error:
            print(f'ledger probe: {error}', file=sys.stderr)
            return 2
        tree_after = tree_fingerprint(REPO_ROOT)
        stable = tree_before == tree_after
        if not stable:
            for row in rows.values():
                row['verdict'] = UNRUNNABLE
                row['reason'] = 'tree changed during probe'
        for ident, row in rows.items():
            print(format_row(ident, row))
        counts = defaultdict(int)
        for row in rows.values():
            counts[row['verdict']] += 1
        result = {'tree_head': tree_before['head'], 'tree_start': tree_before, 'tree_end': tree_after,
                  'tree_stable': stable, 'client_run': args.client, 'artifact_dir': str(ctx.artifact_dir),
                  'counts': dict(counts), 'rows': rows}
        (ctx.artifact_dir / 'probe.json').write_text(json.dumps(result, indent=2) + '\n')
        if args.out:
            args.out.parent.mkdir(parents=True, exist_ok=True)
            args.out.write_text(json.dumps(result, indent=2) + '\n')
        print(f"[ledger] {dict(counts)} receipt={ctx.artifact_dir / 'probe.json'}")
        if not stable:
            print('[ledger] probe tree changed during execution; results are not attributable to one tree',
                  file=sys.stderr)
            return 2
        return 0
    if args.action == 'graphical-control':
        ctx = _artifact_ctx('graphical-control')
        try:
            updated = graphical_control(args.id, receipt=args.receipt, control_tree=args.control_tree,
                                        dry_run=args.dry_run, ctx=ctx)
        except (Refused, ValueError) as error:
            print(f'[ledger] refused: {error}')
            print(f'[ledger] artifact={ctx.artifact_dir}')
            return 1
        red = [ref for ref, entry in updated['negative_control'].items() if entry.get('state') == 'red']
        print(f"[ledger] {'dry-run: ' if args.dry_run else ''}graphical control of {args.id} recorded for "
              f"{', '.join(red)} artifact={ctx.artifact_dir}")
        return 0
    ctx = _artifact_ctx('promote')
    try:
        receipt = promote(args.id, client_receipt=args.client_receipt,
                          client_control_receipt=args.client_control_receipt,
                          fix_patch=args.fix_patch, dry_run=args.dry_run, ctx=ctx)
    except (Refused, ValueError) as error:
        print(f'[ledger] refused: {error}')
        print(f'[ledger] artifact={ctx.artifact_dir}')
        return 1
    print(f"[ledger] {'dry-run ok (nothing written)' if args.dry_run else 'promoted ' + args.id} artifact={ctx.artifact_dir}")
    return 0
