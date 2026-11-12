#!/usr/bin/env python3
"""Apply the Solaris -> Krasnokamen name contract to one repository checkout.

    tools/dev/rebrand.py <repo-root>            rewrite tracked text files and rename paths
    tools/dev/rebrand.py <repo-root> --check    print the remaining occurrences, classified

The script is deterministic and idempotent: it reads only the tracked files of the
repository (git ls-files) and the rules below, never a previous run's output. It
does not commit, does not run rustfmt and does not touch ignored files.

Name contract (rule order matters, see `transform`):
    plugin WIT package     <old>:plugin@0.8.0  ->  krasnokamen:plugin@0.9.0
    binary                 <old>-v2            ->  krasnokamen
    Solaris / SOLARIS / solaris (any identifier, namespace, env var, path segment)
                           ->  Krasnokamen / KRASNOKAMEN / krasnokamen
    Minecraft usernames    <old>Primary|Secondary|Client|Mcp|Loader -> Krasno<...> (16-character limit)

Protected literals (kept as they are, see PROTECTED): the work root, checkout
directories used as host paths, /tmp and /var/tmp names, systemd unit names, test
VPS paths, brand assets. History (HISTORY_FILES, HISTORY_LINES, `rebrand:keep`
blocks) and the slice-A files (OWNER_FILES) are not rewritten.
"""
import os
import re
import subprocess
import sys
from collections import Counter

OLD = 'solar' + 'is'          # spelled in pieces so the script never rewrites itself
OLD_T, OLD_U = OLD.capitalize(), OLD.upper()
NEW, NEW_T, NEW_U = 'krasnokamen', 'Krasnokamen', 'KRASNOKAMEN'

# ---- files that are never rewritten -------------------------------------------------
SELF_FILES = {'tools/dev/rebrand.py'}
# Slice A (brand): root README and brand assets.
OWNER_FILES = (r'^README\.md$', r'^docs/brand/', r'^docs/assets/brand/')
# History: past release notes, recorded evidence, the archive, promoted ledger rows.
HISTORY_FILES = (
    r'^docs/releases/v0\.0\.8\.md$',
    r'^docs/releases/v1\.0\.0-rc\.[1-4]\.md$',
    r'^docs/evidence/',
    r'^docs/archive/',
    r'^docs/bugs/promotions/',
)
# History lines kept inside otherwise rewritten files: (path regex, line regex).
HISTORY_LINES = (
    (r'^docs/bugs/ledger\.toml$', r'^(title|source)\s*='),
    (r'^docs/decisions/\d{4}-.*\.md$', r'^\*\*Status:\*\*'),
)
KEEP_BEGIN, KEEP_END = 'rebrand:keep-begin', 'rebrand:keep-end'

# ---- protected literals ------------------------------------------------------------------
CHECKOUTS = rf'{OLD}(?:-client|-loader|-default-plugins)?(?![\w-])'
PROTECTED = [
    # the work root and the checkout directory that follows it
    ('host path', rf'{OLD}-work(?:/{CHECKOUTS})?'),
    # checkout directories named as host paths
    ('host path',
     rf'(?:\.\./|<work>/|\bwt/|sibling:|/home/(?:kaiserroman|user|u)/|\$WORK/|\$\{{WORK\}}/|\bkaiserroman/|~/'
     rf'|(?<=path: )|(?<=working-directory: )|(?<=workspaces: )|(?<=github\.workspace \}}\}}/)){CHECKOUTS}'),
    ('host path', rf'{OLD}(?=/\.analysis)'),
    # checkout directories joined onto the work root in code
    ('host path', rf'(?:\b(?:WORK|work|WORK_ROOT|HOME)|\.parent|\.work|\{{self\.work\}})\s*/\s*["\']?{CHECKOUTS}'),
    ('host path', rf'(?<="other" / "){CHECKOUTS}'),
    ('host path', rf'SIBLING_NAMES = \{{[^}}]*\}}'),
    ('host path', rf'\("{OLD}", "{OLD}-default-plugins", "{OLD}-loader", "{OLD}-client"\)'),
    ('host path', rf'(?<=/games/){OLD}(?![\w-])'),
    ('host path', rf'(?<=dirname "\$MAIN_CHECKOUT"\)/){CHECKOUTS}'),
    # xtask architecture: (repository name, checkout directory) pairs, written after the rename
    ('host path', rf'(?<="{NEW}-default-plugins", "){OLD}-default-plugins'),
    ('host path', rf'(?<="{NEW}-loader", "){OLD}-loader'),
    # temporary names shared with the live runner
    ('tmp name (shared with the live runner)', rf'/(?:var/)?tmp/{OLD}-[\w.@%-]*'),
    ('tmp name (shared with the live runner)', rf'{OLD}-(?:bench|scene-slot|sp-slow|pw|mem-guard|mem-slots|port-locks|plugin-tutorial)[\w.%-]*'),
    # systemd units and slices
    ('systemd unit/slice name', rf'{OLD}(?:-(?:heavy|agents|scenario|sccache|metrics|sp-slow)[\w-]*)?\.(?:slice|service|timer|scope)(?:\.d)?'),
    ('systemd unit/slice name', rf'{OLD}-heavy-watchdog'),
    ('systemd unit/slice name', rf'{OLD}-heavy\.slice'),
    # test VPS
    ('test VPS path/unit', rf'/opt/{OLD}\b'),
    ('test VPS path/unit', rf'/etc/default/{OLD}\b'),
    ('test VPS path/unit', rf'(?<=-u ){OLD}(?![\w-])'),
    ('test VPS path/unit', rf'(?<=systemctl start ){OLD}\b'),
    ('test VPS path/unit', rf'(?<=systemctl stop ){OLD}\b'),
    ('test VPS path/unit', rf'(?<=systemctl restart ){OLD}\b'),
    ('test VPS path/unit', rf'(?<=SERVER_USER:-){OLD}(?=\}})'),
    # hash domain separators keep their bytes so derived ids and worlds do not change
    ('hash domain separator (value-preserving)', rf'(?<=update\(b")(?:{OLD}|{OLD_T})'),
    # recorded performance baselines compare by the recipe digest, which includes the cohort name
    ('recipe cohort id (value-preserving)', rf'(?<=json\.dumps\("){OLD}(?=-coreperf-fixture)'),
    # brand assets keep their file names (slice A)
    ('brand asset file name (slice A)', rf'docs/assets/brand/{OLD}-[\w.-]+'),
]
PROTECTED_RE = [(k, re.compile(p)) for k, p in PROTECTED]

SUBS = [
    (re.compile(rf'{OLD}:plugin((?:/[\w-]+)?)@0\.8\.0'), rf'{NEW}:plugin\1@0.9.0'),
    (re.compile(rf'b"{OLD_T}V2Native\\0"'), lambda m: 'b"KrasnoV2Native\\0\\0"'),
    (re.compile(rf'{OLD_T}V2Native'), 'KrasnoV2Native'),
    (re.compile(rf'{OLD_T}(Primary|Secondary|Client|Mcp[A-Z]?|Loader)(?![A-Za-z])'), r'Krasno\1'),
    (re.compile(rf'{OLD}-v2(?![A-Za-z0-9_])'), NEW),
    (re.compile(OLD_U), NEW_U),
    (re.compile(OLD_T), NEW_T),
    (re.compile(OLD), NEW),
]
ANY_OLD = re.compile(OLD, re.I)


def protected_spans(text):
    spans = []
    for klass, rx in PROTECTED_RE:
        spans.extend((*m.span(), klass) for m in rx.finditer(text))
    spans.sort()
    merged = []
    for lo, hi, klass in spans:
        if merged and lo <= merged[-1][1]:
            merged[-1][1] = max(merged[-1][1], hi)
        else:
            merged.append([lo, hi, klass])
    return merged


def sub_segment(segment):
    for rx, repl in SUBS:
        segment = rx.sub(repl, segment)
    return segment


def transform(text):
    out, pos = [], 0
    for lo, hi, _ in protected_spans(text):
        out.append(sub_segment(text[pos:lo]))
        out.append(text[lo:hi])
        pos = hi
    out.append(sub_segment(text[pos:]))
    return ''.join(out)


# ---- file selection --------------------------------------------------------------------------
def matches(path, patterns):
    return any(re.search(p, path) for p in patterns)


def tracked(root):
    raw = subprocess.check_output(['git', 'ls-files', '-z'], cwd=root)
    return [p for p in raw.decode().split('\0') if p and not p.startswith('data/')]


def read_text(full):
    if os.path.islink(full) or not os.path.isfile(full):
        return None
    data = open(full, 'rb').read()
    if b'\0' in data:
        return None
    try:
        return data.decode('utf-8')
    except UnicodeDecodeError:
        return None


def rewrite(path, text):
    """Return the rewritten text of one file; history lines and keep blocks stay."""
    line_rules = [re.compile(lr) for pr, lr in HISTORY_LINES if re.search(pr, path)]
    out, kept = [], False
    for line in text.splitlines(keepends=True):
        if KEEP_BEGIN in line:
            kept = True
        if kept or any(r.search(line) for r in line_rules):
            out.append(line)
        else:
            out.append(transform(line))
        if KEEP_END in line:
            kept = False
    return ''.join(out)


def apply(root):
    changed, renamed = 0, []
    files = tracked(root)
    for path in files:
        if path in SELF_FILES or matches(path, OWNER_FILES + HISTORY_FILES):
            continue
        full = os.path.join(root, path)
        text = read_text(full)
        if text is None:
            continue
        new = rewrite(path, text)
        if new != text:
            with open(full, 'w', encoding='utf-8', newline='') as handle:
                handle.write(new)
            changed += 1
    # renames, deepest first so directory moves do not invalidate later paths
    for path in sorted(files, key=lambda p: -p.count('/')):
        if path in SELF_FILES or matches(path, OWNER_FILES + HISTORY_FILES):
            continue
        new = transform(path)
        if new != path:
            os.makedirs(os.path.dirname(os.path.join(root, new)) or root, exist_ok=True)
            subprocess.check_call(['git', 'mv', path, new], cwd=root)
            renamed.append((path, new))
    print(f'rebrand: {changed} files rewritten, {len(renamed)} paths renamed')


# ---- --check ----------------------------------------------------------------------------------
def classify(root):
    """Yield (class, path, line number, excerpt) for every remaining occurrence."""
    for path in tracked(root):
        full = os.path.join(root, path)
        if os.path.islink(full) or not os.path.isfile(full):
            continue
        if ANY_OLD.search(path):
            spans = protected_spans(path)
            if not any(lo <= m.start() < hi for m in ANY_OLD.finditer(path) for lo, hi, _ in spans):
                yield ('UNEXPECTED path name', path, 0, path)
        text = read_text(full)
        if text is None:
            if ANY_OLD.search(open(full, 'rb').read().decode('latin-1')):
                yield ('binary file containing the name', path, 0, '')
            continue
        if path in SELF_FILES:
            continue
        if matches(path, OWNER_FILES):
            klass = 'slice A (brand, README)'
        elif matches(path, HISTORY_FILES):
            klass = 'history document'
        else:
            klass = None
        line_rules = [re.compile(lr) for pr, lr in HISTORY_LINES if re.search(pr, path)]
        kept = False
        for number, line in enumerate(text.splitlines(), 1):
            if KEEP_BEGIN in line:
                kept = True
            if ANY_OLD.search(line):
                if klass:
                    yield (klass, path, number, line)
                elif kept or any(r.search(line) for r in line_rules):
                    yield ('history line (ledger title/source, ADR status, kept block)', path, number, line)
                else:
                    spans = protected_spans(line)
                    for m in ANY_OLD.finditer(line):
                        hit = next((k for lo, hi, k in spans if lo <= m.start() < hi), None)
                        yield (hit or 'UNEXPECTED', path, number, line)
            if KEEP_END in line:
                kept = False


def check(root):
    counts, files, unexpected = Counter(), {}, []
    for klass, path, number, line in classify(root):
        counts[klass] += 1
        files.setdefault(klass, set()).add(path)
        if klass.startswith('UNEXPECTED'):
            unexpected.append(f'{path}:{number}: {line.strip()[:160]}')
    for klass, count in sorted(counts.items()):
        print(f'{count:6d} occurrences in {len(files[klass]):4d} files: {klass}')
    for klass in sorted(files):
        if not klass.startswith('UNEXPECTED'):
            print(f'  {klass}: ' + ', '.join(sorted(files[klass])[:12]) + (' ...' if len(files[klass]) > 12 else ''))
    for line in unexpected:
        print(line)
    return 1 if unexpected else 0


def main(argv):
    args = [a for a in argv if not a.startswith('--')]
    if len(args) != 1 or any(a not in ('--check',) for a in argv if a.startswith('--')):
        print(__doc__)
        return 2
    root = os.path.abspath(args[0])
    if '--check' in argv:
        return check(root)
    apply(root)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
