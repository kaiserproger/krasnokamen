"""Executable bug-ledger checks using the canonical harness and nextest."""
from __future__ import annotations

from collections import defaultdict
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import shlex
import subprocess
import sys
import tomllib

from .ledger_base_fixed import BASE_FIXED
from tools.dev.wtlib import PINS
from .runtime import REPO_ROOT

REQUIRED = {'id', 'title', 'area', 'repro', 'severity', 'status', 'source', 'regression'}
OPTIONAL = {'repro_check', 'failing_first', 'fix_commit', 'promotion', 'ingress_proof', 'proving_assertions'}
PROMOTION_DIR = 'docs/bugs/promotions'


def _promotion_errors(ident, bug):
    """Why this row's `fixed` status or `promotion` field is not acceptable, if it is not."""
    promotion, fix_commit = bug.get('promotion'), bug.get('fix_commit')
    if 'fix_commit' in bug and (not isinstance(fix_commit, str) or not re.fullmatch(r'(?:client:)?[0-9a-f]{7,40}|patch:[0-9a-f]{64}', fix_commit)):
        return [f'{ident}: fix_commit must be a lowercase hex commit id, client:<commit> or patch:<sha256>']
    if promotion is None:
        if bug.get('status') == 'fixed' and ident not in BASE_FIXED:
            return [f'{ident}: fixed without a promotion receipt; only `python3 -m tools.harness ledger promote` may set fixed']
        return []
    if promotion != f'{PROMOTION_DIR}/{ident}.json':
        return [f'{ident}: promotion must be {PROMOTION_DIR}/{ident}.json']
    if bug.get('status') != 'fixed':
        return [f'{ident}: promotion receipt on a row that is not fixed']
    try:
        receipt = json.loads((REPO_ROOT / promotion).read_text())
    except (OSError, ValueError):
        return [f'{ident}: promotion receipt {promotion} is missing or not JSON']
    if not isinstance(receipt, dict) or receipt.get('id') != ident or receipt.get('verdict') != 'promoted' or not fix_commit or receipt.get('fix_commit') != fix_commit:
        return [f'{ident}: promotion receipt does not match the row id and fix_commit']
    return []


def _nextest_listing(text):
    decoder = json.JSONDecoder()
    for match in re.finditer(r'\{', text):
        try:
            value, _ = decoder.raw_decode(text[match.start():])
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and 'rust-suites' in value:
            return value
    raise ValueError('nextest did not produce a test listing')


def _loader_live_command(scenario):
    """argv template for one loader-live scenario ref. `{server_bin}` and `{plugins_dir}`
    are substituted by ledger._run_harness once the tree's release server is built; the
    scenario must be a registered graphical loader-live entry (fail-closed otherwise)."""
    from . import scenarios

    entry = scenarios.SCENARIOS.get(scenario)
    if entry is None or entry.profile != 'loader-live' or entry.evidence != 'graphical':
        raise ValueError(f'unknown runnable scenario loader-live:{scenario}')
    return [sys.executable, '-m', 'tools.harness', 'run', 'scenario', scenario,
            '--platform', LOADER_LIVE_PLATFORM, '--server-bin', '{server_bin}',
            '--plugins-dir', '{plugins_dir}', '--timeout-seconds', '420']


def _client_command(profile, scenario):
    from . import scenarios

    entry = scenarios.SCENARIOS.get(scenario)
    if entry is None or entry.profile != profile or entry.evidence not in ('graphical', 'tcp', 'native'):
        raise ValueError(f'unknown runnable scenario {profile}:{scenario}')
    if profile == 'loader-live':
        return _loader_live_command(scenario)
    if profile == 'dh-gate':
        raise ValueError(f'unknown runnable scenario {profile}:{scenario}')
    argv = [sys.executable, '-m', 'tools.harness', 'run', 'scenario', scenario]
    if scenario.startswith('native-'):
        argv.extend(['--client', 'native', '--windowed'])
    else:
        argv.extend(['--client', 'java'])
        if profile == 'scene-shot':
            argv.append('--windowed')
    return argv

# Only profiles explicitly supported by the CLI's windowed native client path.
_WINDOWED_PROFILES = frozenset({
    'survival', 'inventory', 'entities', 'core-client', 'native-lifecycle',
    'logical-native', 'native-scale', 'native-matrix', 'title-connect', 'singleplayer-regressions',
    'dialogs', 'admin-commands', 'chisel', 'anti-xray', 'block-log',
    'composter-chance', 'chat-visibility', 'datapack-reload', 'village-qa', 'village-autonomy-qa',
    'civilization-seed-qa', 'village-owner-order-qa', 'scene-shot',
    'settlement-map-qa', 'claim-only-qa', 'buildings-qa', 'army-qa', 'autocraft-qa', 'screen-share',
    'screens-voice-qa',
})
_AUTONOMY_VERDICTS = frozenset({
    'walk_to_village', 'ownerless_settlement', 'founding_chest_empty',
    'generated_buildings_certified', 'natural_zones', 'workers_produce_unfunded',
    'mutations_refused_unowned', 'builder_completes_storage_l1',
    'builder_completes_further_building', 'builder_completes_autonomous_upgrade',
    'claim_via_dialog_button', 'second_claim_refused', 'owner_orders_building',
    'restart_resume',
})
_BOOLEAN_PROFILE_VERDICTS = {
    'civilization-seed-qa': frozenset({
        'new_house', 'births', 'second_building', 'iron_smelted', 'all',
    }),
    # backends/settlement_map_qa.py VERDICTS (a test keeps the two in step).
    'settlement-map-qa': frozenset({
        'bell_opens_hub', 'dashboard', 'map', 'building_order', 'shortage_icons',
        'storage_transfer', 'project_progress', 'residents', 'resources', 'production_order',
        'localized_text', 'all',
    }),
}
# Run arguments of a profile ref: (client, args after `--`) per verdict that needs another client
# than native, and extra harness options per profile. The native client has no language option
# (it sends ClientInformation `en_us` fixed), so the Russian text check runs on vanilla Java.
_PROFILE_VERDICT_RUN = {
    ('settlement-map-qa', 'localized_text'): ('java', ['--', '--language', 'ru_ru']),
    # The chisel resource-pack admission is a vanilla-Java client behaviour; the native client
    # never asks for the optional pack.
    ('chisel', 'vanilla_optional_admission'): ('java', []),
}
_PROFILE_RUN_OPTIONS = {
    'civilization-seed-qa': ['--timeout-seconds', '7200'],
    'settlement-map-qa': ['--timeout-seconds', '3600'],
}
_PROFILE_VERDICTS = {
    'chisel': ('chisel_admission', frozenset({'vanilla_optional_admission'})),
    'village-qa': ('settlement_acceptance', frozenset({
        'door_walk', 'sleep_return', 'night_door', 'farmer_harvest', 'miner_deposit', 'lumberjack',
    })),
    'village-autonomy-qa': ('autonomy_acceptance', _AUTONOMY_VERDICTS),
    # backends/screens_voice_qa.py verdicts: consent, audio, respawn and SVC proximity/group voice.
    'screens-voice-qa': ('screens_voice', frozenset({
        'native_loads_computer_monitor', 'owner_consent_names_app_audio', 'owner_deny_sends_no_audio',
        'owner_allow_streams_picture_and_audio', 'fabric_loader_plays_screen_audio',
        'viewer_keeps_screen_after_same_dimension_respawn', 'native_voice_resets_on_non_voice_server',
        'svc_proximity_fabric_to_fabric', 'svc_proximity_fabric_and_native', 'svc_group_fabric_to_fabric',
        'native_outsider_hears_no_group_audio', 'all',
    })),
    'claim-only-qa': ('claim_acceptance', frozenset({
        'ownerless_settlement', 'claim_via_dialog_button', 'second_claim_refused',
        'owner_orders_building', 'restart_resume',
    })),
    'army-qa': ('settlement_acceptance', frozenset({
        'follow_30_blocks', 'guard_holds_post', 'patrol_moves_inside',
        'return_to_barracks', 'soldiers_kill_zombies', 'villagers_survive',
        'guard_returns_after_combat', 'slot_refills_no_dupe',
    })),
    # Building verdicts use "<blueprint>.<behaviour>" below.
    'buildings-qa': ('buildings_acceptance', frozenset()),
}
# One blueprint's real-builder run: L1..LN built, block-matched and exercised (buildings-qa `--timeout-seconds`).
BLUEPRINT_RUN_SECONDS = 1800
_PROFILE_OVERRIDES = {
    'native-matrix': ('KRASNOKAMEN_MATRIX_PATH', 'KRASNOKAMEN_MATRIX_LEDGER', 'KRASNOKAMEN_MATRIX_RUN_ROOT'),
    'buildings-qa': ('BUILDINGS_QA_SELECT', 'BUILDINGS_QA_KEEP_WORLD',
                     'BUILDINGS_QA_STOP_AFTER_ORDER'),
    'army-qa': ('KRASNOKAMEN_ARMY_QA_PHASE', 'KRASNOKAMEN_ARMY_QA_PLUGINS'),
    'village-autonomy-qa': ('VILLAGE_AUTONOMY_RESUME_WORLD', 'VILLAGE_AUTONOMY_RESUME_PHASE'),
    'civilization-seed-qa': ('VILLAGE_AUTONOMY_RESUME_WORLD', 'VILLAGE_AUTONOMY_RESUME_PHASE',
                             'VILLAGE_AUTONOMY_SEED', 'VILLAGE_AUTONOMY_CENTER'),
}



def _profile_command(profile, verdict):
    from . import profiles

    spec = profiles.PROFILES.get(profile)
    if profile not in _WINDOWED_PROFILES or not spec or spec.get('scope') != 'real-client':
        raise ValueError(f'unknown runnable profile {profile}')
    overrides = [name for name in _PROFILE_OVERRIDES.get(profile, ()) if os.environ.get(name)]
    if overrides:
        raise ValueError(f'{profile}: partial or resumed profile overrides forbidden: {", ".join(overrides)}')
    if verdict == 'all':
        valid = True
    elif profile == 'native-matrix':
        from .backends.native_matrix import frozen_native_rows, verdict_key
        valid = verdict in {verdict_key(row['id']) for row in frozen_native_rows()}
    elif profile in _BOOLEAN_PROFILE_VERDICTS:
        valid = verdict in _BOOLEAN_PROFILE_VERDICTS[profile]
    elif profile == 'buildings-qa':
        from .backends.buildings_qa import LEVELS, PERIMETER
        from .backends.perimeter_qa import VERDICTS as PERIMETER_VERDICTS
        blueprint, separator, behaviour = verdict.partition('.')
        if blueprint == PERIMETER:
            valid = bool(separator) and behaviour in PERIMETER_VERDICTS
        else:
            valid = bool(separator) and blueprint in LEVELS and (
                behaviour in {'settlement_founded', 'L1_built'}
                or (blueprint, behaviour) == ('house', 'broken_cell_replaced')
                or behaviour == 'L2_upgrade' and LEVELS[blueprint] >= 2
                or behaviour == 'L3_upgrade' and LEVELS[blueprint] >= 3
            )
    else:
        valid = verdict in _PROFILE_VERDICTS.get(profile, (None, {'all'}))[1]
    if not valid:
        raise ValueError(f'unknown verdict {profile}:{verdict}')
    client, tail = _PROFILE_VERDICT_RUN.get((profile, verdict), ('native', []))
    command = [sys.executable, '-m', 'tools.harness', 'run', profile,
               '--client', client, '--windowed', *_PROFILE_RUN_OPTIONS.get(profile, []), *tail]
    if profile == 'buildings-qa' and verdict != 'all':
        # A blueprint verdict is one blueprint's run: the default `ALL` run builds nine blueprints under one
        # 600 s deadline and can never pass. The select reaches the child through argv, never the caller's
        # environment (partial-profile overrides there stay forbidden).
        blueprint = verdict.partition('.')[0]
        return ['env', f'BUILDINGS_QA_SELECT={blueprint}', *command, '--timeout-seconds', str(BLUEPRINT_RUN_SECONDS)]
    return command


def _check_command(profile, check):
    from . import scenarios

    if check in scenarios.SCENARIOS:
        return _client_command(profile, check), False
    return _profile_command(profile, check), True


def _graphical_check(profile, check):
    from . import scenarios

    if check in scenarios.SCENARIOS:
        entry = scenarios.SCENARIOS[check]
        return entry.profile == profile and entry.evidence == 'graphical'
    try:
        _profile_command(profile, check)
        return True
    except ValueError:
        return False


class VerdictNotMet(RuntimeError):
    """A profile verdict is not met; `value` is None when the receipt never reported it."""

    def __init__(self, message, value):
        super().__init__(message)
        self.value = value


def _profile_verdict(receipt, profile, verdict):
    details = receipt.get('details', {})
    if profile == 'native-matrix' and details.get('matrix_full_run') is not True:
        raise VerdictNotMet(f'{profile}:{verdict} requires the full frozen matrix', None)
    verdicts = details.get('verdicts')
    if profile == 'native-matrix' or profile in _BOOLEAN_PROFILE_VERDICTS or verdicts is not None:
        value = verdicts.get(verdict) if isinstance(verdicts, dict) else None
        if value is not True:
            raise VerdictNotMet(f'{profile}:{verdict} verdict is {value!r}, not true', value)
        return
    if verdict == 'all':
        return
    section = _PROFILE_VERDICTS[profile][0]
    detail = receipt.get('details', {}).get('scenario', {}).get(section, {})
    if profile == 'buildings-qa':
        blueprint, _, verdict = verdict.partition('.')
        detail = detail.get('blueprints', {}).get(blueprint, {})
    behaviour = detail.get('behaviour', {})
    if behaviour.get(verdict) != 'PASS':
        raise VerdictNotMet(f'{profile}:{verdict} verdict is {behaviour.get(verdict)!r}, not PASS', behaviour.get(verdict))

def _failed_ids(text, cargo):
    failed = set()
    for binary, name in re.findall(r'^\s*FAIL\s+\[[^\]]+\]\s+\(\s*\d+/\d+\)\s+(\S+)\s+(\S+)', text, re.MULTILINE):
        failed.update(cargo.get(binary.split('::', 1)[0], {}).get(name, ()))
    return failed



MEM_RUN = Path(os.environ.get('KRASNOKAMEN_MEM_RUN', '/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/mem-run.sh'))
LOADER_LIVE_PLATFORM = 'fabric'



def is_other_tree(tree):
    return tree is not None and Path(tree).resolve() != REPO_ROOT.resolve()


def tree_build_command(tree, argv):
    """(argv, env) for a build in `tree`. For a tree other than REPO_ROOT on the owner host it goes
    through mem-run with cwd=tree and without the caller's CARGO_TARGET_DIR, so the runner gives that
    tree its own pooled slot (marks it and touches the files on a tree switch): a build must never
    land in a slot marked for another tree. The cap is 12G: an mc-core test build peaks above 6G and
    was OOM-killed there, which left every mc-core negative control unrunnable (2026-11-12)."""
    env = os.environ.copy()
    if not is_other_tree(tree) or not MEM_RUN.exists():
        return list(argv), env
    env.pop('CARGO_TARGET_DIR', None)
    return [str(MEM_RUN), '-m', '12G', *argv], env


def _host_target():
    """The rustc host triple, the target pin-release builds by default."""
    version = subprocess.run(['rustc', '-vV'], check=True, capture_output=True, text=True).stdout
    for line in version.splitlines():
        if line.startswith('host: '):
            return line.removeprefix('host: ').strip()
    raise RuntimeError('rustc did not report its host target')


def _release_server(ctx, profiles, tree=None):
    """Select a registered pin of the exact tree; never build or relabel a server."""
    from . import ledger

    built = Path(tree or REPO_ROOT).resolve()
    fingerprint = ledger.tree_fingerprint(built)
    registry = Path(os.environ.get('KRASNOKAMEN_LEDGER_ROOT') or built) / '.analysis/ledger-builds'
    command = shlex.join([str(MEM_RUN.with_name('pin-release')), str(built)])
    explicit = os.environ.get('KRASNOKAMEN_SERVER_BINARY')
    host = _host_target()
    manifests = ([Path(explicit).expanduser().resolve().parent / 'manifest.json'] if explicit
                 else sorted(PINS.glob('*/manifest.json')))
    reason = 'no registered pin-release server for this tree'
    for manifest in manifests:
        try:
            info = json.loads(manifest.read_text())
            binary = Path(info['binary']).resolve()
            digest = info['binary_sha256']
            if (info.get('repo') != 'core' or info.get('kind') != 'server'
                    or binary.parent != manifest.parent.resolve()
                    or (explicit and binary != Path(explicit).expanduser().resolve())):
                raise ValueError(f'{manifest} does not pin the requested server')
            if info.get('target') != host:
                raise ValueError(f'{manifest} pins target {info.get("target")!r}, not this host {host!r}')
            # The same provenance fence pin-release applies before reusing a pin.
            missing = [key for key in ('head', 'diff_sha256', 'inputs_sha256', 'env_deps') if info.get(key) is None]
            if missing:
                raise ValueError(f'{manifest} records no build provenance ({", ".join(missing)})')
            # A pin copied from another pin directory is used only if that source was built from
            # the same working state (pin-release's relabelled-pin fence).
            source = info.get('reused_from')
            if source:
                origin = json.loads((manifest.parent.parent / str(source) / 'manifest.json').read_text())
                if (origin.get('head'), origin.get('diff_sha256')) != (info['head'], info['diff_sha256']):
                    raise ValueError(f'{manifest} is a relabelled pin of {source}')
            registration = json.loads((registry / f'{digest}.json').read_text())
            if (info.get('head') != fingerprint['head'] or registration.get('sha256') != digest
                    or registration.get('tree') != {key: fingerprint[key] for key in ('head', 'sha256')}):
                raise ValueError(f'{manifest} pins a different tree (not {fingerprint["head"][:12]}/{fingerprint["sha256"][:12]})')
            if not os.access(binary, os.X_OK):
                raise ValueError(f'pin is not executable: {binary}')
            with binary.open('rb') as file:
                if hashlib.file_digest(file, 'sha256').hexdigest() != digest:
                    raise ValueError(f'pin checksum mismatch: {binary}')
        except (OSError, ValueError, KeyError, TypeError) as error:
            if explicit:
                reason = f'cannot use KRASNOKAMEN_SERVER_BINARY={explicit}: {error}'
            continue
        ctx.client_evidence['regressions_server'] = {
            'path': str(binary), 'sha256': digest, 'manifest': str(manifest), 'tree': fingerprint,
        }
        return binary
    raise RuntimeError(f'{reason}. Run `{command}` for the tree under test; '
                       'a negative control needs its own reverted-tree pin. No build fallback.')


def _client_env(binary):
    env = os.environ.copy()
    # A mutation control must never replace the binary in a ledger gate.
    env.pop('KRASNOKAMEN_FIRST_START_RECOVERY_BINARY', None)
    env['KRASNOKAMEN_SERVER_BINARY'] = str(binary)
    return env


class ReceiptNotPassed(RuntimeError):
    def __init__(self, status, detail='', receipt=None, path=None):
        super().__init__(f"client receipt is {status!r}, not passed{detail}")
        self.status = status
        self.receipt = receipt
        self.path = path


# What a kept client artifact holds: reports, logs, receipts and screenshots. Worlds, caches and
# binaries stay behind (the disk rule in docs/development/heavy-commands-and-memory.md#disk).
EVIDENCE_SUFFIXES = frozenset((
    '.json', '.jsonl', '.log', '.md', '.txt', '.yaml', '.toml', '.tsv', '.csv', '.sha256', '.html',
    '.png', '.jpg', '.webp', '.svg',
))


def _keep_artifact(ctx, artifact):
    """Copy a client run's evidence into this run's artifact dir -> the copy.

    `ledger promote` runs its control and restore in a temporary worktree that it removes afterwards,
    so a receipt path into that tree would dangle in promote.json."""
    kept = ctx.artifact_dir / 'client-artifacts' / artifact.name
    for source in artifact.rglob('*'):
        relative = source.relative_to(artifact)
        if source.is_file() and source.suffix in EVIDENCE_SUFFIXES and 'world' not in relative.parts:
            target = kept / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
    return kept


def _attach_failed_scenario(receipt, artifact):
    """A failed profile run leaves `details.scenario` out of result.json, but its probe wrote the
    same report to `<artifact>/<probe>/scenario.json` on the way out. The verdicts recorded before
    the failing assertion (village-qa writes `miner_deposit` before its aggregate `all_behaviours_pass`)
    are what name a failed run red for one verdict. The run's own artifact only; nothing is guessed
    when the probe wrote no single report."""
    details = receipt.get('details')
    if not isinstance(details, dict) or 'scenario' in details:
        return
    reports = list(Path(artifact).glob('*/scenario.json'))
    if len(reports) != 1:
        return
    try:
        report = json.loads(reports[0].read_text())
    except ValueError:
        return
    if isinstance(report, dict):
        details['scenario'] = report


def _fresh_client_data(ctx, env, log_name):
    """A private XDG_DATA_HOME for one client run -> its path.

    Windowed runs inherit the caller's XDG root, so a ledger probe or promote running many client
    gates in a row would share `client-settings.json` between them
    (docs/development/real-client-gates.md). The content directories are linked, never copied:
    from the caller's root when it has them, otherwise from the KRASNOKAMEN_CLIENT_DATA_DIR
    template that headless runs use."""
    from .preflight import CLIENT_CONTENT_DIRS, client_template

    source = Path(env.get('XDG_DATA_HOME') or Path.home() / '.local/share') / 'krasnokamen-client'
    if not (source / 'facts').is_dir() and env.get('KRASNOKAMEN_CLIENT_DATA_DIR'):
        source = client_template()
    root = ctx.artifact_dir / f'xdg-{Path(log_name).stem}'
    (root / 'krasnokamen-client').mkdir(parents=True)
    for name in CLIENT_CONTENT_DIRS:
        if (source / name).exists():
            (root / 'krasnokamen-client' / name).symlink_to((source / name).resolve(), target_is_directory=(source / name).is_dir())
    return root


def _client_receipt(ctx, profiles, command, env, profile, check, is_profile, cwd=None):
    """Run one real-client command; return its passed receipt and path, or raise."""
    from . import scenarios

    log_name = f'regressions-client-{len(ctx.logs)}.log'
    failure = None
    xdg = _fresh_client_data(ctx, env, log_name)
    try:
        profiles._run_command(list(command), ctx, log_name, env={**env, 'XDG_DATA_HOME': str(xdg)}, cwd=cwd)
    except RuntimeError as error:
        failure = error
    finally:
        shutil.rmtree(xdg, ignore_errors=True)
    log = ctx.artifact_dir / log_name
    paths = re.findall(r'artifact=(\S+)', log.read_text()) if log.exists() else []
    if not paths:
        raise failure or RuntimeError('client run produced no receipt')
    path = Path(paths[-1]) / 'result.json'
    if is_other_tree(cwd):
        path = _keep_artifact(ctx, path.parent) / 'result.json'
    receipt = json.loads(path.read_text())
    if receipt.get('status') != 'passed':
        _attach_failed_scenario(receipt, path.parent)
        raise ReceiptNotPassed(receipt.get('status'), f'; {failure}' if failure else '', receipt, path)
    if failure:
        raise failure
    server_hash = ctx.client_evidence['regressions_server']['sha256']
    if is_profile and receipt.get('details', {}).get('server_binary_sha256') != server_hash:
        raise RuntimeError('client receipt does not identify the built release server')
    if not is_profile and scenarios.SCENARIOS[check].evidence != 'native' and server_hash not in json.dumps(receipt.get('details', {})):
        raise RuntimeError('client receipt does not identify the built release server')
    return receipt, path


def _parse_ref(ref):
    """Split a registered check ref into (kind, a, b); ValueError when it cannot run."""
    parts = ref.split(':', 2)
    if len(parts) == 3 and parts[0] == 'cargo' and re.fullmatch(r'[A-Za-z0-9_-]+', parts[1]) and re.fullmatch(r'[A-Za-z0-9_:]+', parts[2]):
        return tuple(parts)
    if len(parts) == 3 and parts[0] == 'harness':
        _check_command(parts[1], parts[2])
        return tuple(parts)
    # A fix in the client repository: the test runs in the client checkout, not in this one.
    # `client:<crate>:<test>` (both resolved by cargo in runtime.sibling_root('client')).
    if (len(parts) == 3 and parts[0] == 'client'
            and re.fullmatch(r'[A-Za-z0-9_-]+', parts[1]) and re.fullmatch(r'[A-Za-z0-9_:]+', parts[2])):
        return tuple(parts)
    # A python fix in tools/** has no cargo check and no windowed profile: its own unittest is the
    # regression guard. `python:<dotted module>:<dotted TestClass.test_method>`.
    if (len(parts) == 3 and parts[0] == 'python'
            and re.fullmatch(r'[A-Za-z0-9_.]+', parts[1])
            and re.fullmatch(r'[A-Za-z0-9_]+(?:\.[A-Za-z0-9_]+)*', parts[2])):
        return tuple(parts)
    raise ValueError(f'invalid check {ref!r}')


def needs_real_client(bug):
    """Client-surface rows and owner/real-client repros are closed only by a real-client check."""
    return bug.get('area') == 'client' or bug.get('repro') in ('manual', 'client')


def is_graphical_ref(ref):
    parts = ref.split(':', 2)
    return len(parts) == 3 and parts[0] == 'harness' and _graphical_check(parts[1], parts[2])


def run(ctx):
    from . import profiles

    with (REPO_ROOT / 'docs/bugs/ledger.toml').open('rb') as file:
        ledger = tomllib.load(file)
    if set(ledger) != {'bug'} or not isinstance(ledger['bug'], list) or not ledger['bug']:
        raise ValueError('ledger must contain a nonempty [[bug]] array and no other tables')
    bugs = ledger['bug']
    ids = set()
    cargo = defaultdict(lambda: defaultdict(set))
    clients = defaultdict(set)
    python_refs = defaultdict(set)
    client_tests = defaultdict(set)
    errors = []
    for bug in bugs:
        if not isinstance(bug, dict):
            errors.append('bug entry must be a table')
            continue
        ident = bug.get('id')
        if not isinstance(ident, str) or not ident or ident in ids:
            errors.append(f'invalid or duplicate bug id: {ident!r}')
            continue
        ids.add(ident)
        errors.extend(_promotion_errors(ident, bug))
        if REQUIRED - bug.keys() or bug.keys() - REQUIRED - OPTIONAL:
            errors.append(f'{ident}: missing or unknown fields')
        if any(not isinstance(bug.get(k), str) or not bug[k] for k in ('title', 'source')):
            errors.append(f'{ident}: missing title or source')
        if bug.get('area') not in ('gameplay', 'client', 'harness'):
            errors.append(f'{ident}: invalid area')
        if bug.get('severity') not in ('blocker', 'major', 'minor') or bug.get('status') not in ('open', 'fixed'):
            errors.append(f'{ident}: invalid severity or status')
        if bug.get('repro') not in ('manual', 'client', 'cargo', 'source'):
            errors.append(f'{ident}: invalid repro')
        if 'repro_check' in bug:
            repro_checks = bug['repro_check'] if isinstance(bug['repro_check'], list) else [bug['repro_check']]
            if not repro_checks or any(not isinstance(ref, str) or not ref for ref in repro_checks):
                errors.append(f'{ident}: invalid repro_check')
        if 'ingress_proof' in bug:
            proofs = bug['ingress_proof'] if isinstance(bug['ingress_proof'], list) else [bug['ingress_proof']]
            if not proofs or any(not isinstance(ref, str) or not ref for ref in proofs):
                errors.append(f'{ident}: invalid ingress_proof')
        if 'failing_first' in bug and bug['failing_first'] not in ('not recorded', 'reported'):
            errors.append(f'{ident}: invalid failing_first')
        refs = bug.get('regression')
        if not isinstance(refs, list) or any(not isinstance(ref, str) for ref in refs):
            errors.append(f'{ident}: regression must be a list of check refs')
            continue
        if 'proving_assertions' in bug and (not isinstance(bug['proving_assertions'], dict) or any(
                ref not in refs or not isinstance(assertion, str) or not assertion
                for ref, assertion in bug['proving_assertions'].items())):
            errors.append(f'{ident}: proving_assertions must map registered refs to assertion names')
        if bug.get('status') == 'fixed' and not refs:
            errors.append(f'{ident}: fixed without regression')
        for ref in refs:
            try:
                kind, first, second = _parse_ref(ref)
            except ValueError as error:
                errors.append(f'{ident}: {error}')
                continue
            if kind == 'cargo':
                cargo[first][second].add(ident)
            elif kind == 'python':
                python_refs[(first, second)].add(ident)
            elif kind == 'client':
                client_tests[(first, second)].add(ident)
            else:
                clients[ref].add(ident)
        if bug.get('status') == 'fixed' and needs_real_client(bug) and not any(is_graphical_ref(ref) for ref in refs):
            errors.append(f'{ident}: original or client-surface repro needs a real-client check')
    statuses = {b['id']: b.get('status') for b in bugs if isinstance(b, dict) and isinstance(b.get('id'), str)}
    errors.extend(f'{ident}: reopened row is still in BASE_FIXED; remove it from ledger_base_fixed.py' for ident in sorted(BASE_FIXED) if statuses.get(ident) == 'open')
    fixed = sum(isinstance(b, dict) and b.get('status') == 'fixed' for b in bugs)
    print(f'[regressions] open={len(bugs)-fixed} fixed={fixed}', flush=True)
    if errors:
        raise ValueError('; '.join(errors))
    # Refuse before any unit/Cargo gates: a missing pin cannot be repaired by this run.
    binary = _release_server(ctx, profiles) if ctx.include_regression_client and clients else None
    failures = []
    from . import ledger as ledger_tool
    for refs, execute in ((python_refs, ledger_tool._run_python),
                          (client_tests if ctx.include_regression_client else {}, ledger_tool._run_client)):
        if refs:
            results = execute(ctx, set(refs), REPO_ROOT)
            for test, ids in refs.items():
                state, detail = results.get(test, (ledger_tool.UNRUNNABLE, 'no result'))
                if state != ledger_tool.GREEN:
                    failures.append(f"{', '.join(sorted(ids))}: {test}: {state}: {detail}")
    if cargo:
        expression = ' | '.join(
            f'(package({crate}) & test(={name}))'
            for crate, tests in sorted(cargo.items()) for name in sorted(tests)
        )
        all_ids = set().union(*(ids for tests in cargo.values() for ids in tests.values()))
        package_args = [arg for crate in sorted(cargo) for arg in ('-p', crate)]
        try:
            profiles._run_command(['cargo', 'nextest', 'list', *package_args, '-E', expression, '--message-format', 'json'], ctx, 'regressions-list.log')
            listing = _nextest_listing((ctx.artifact_dir / 'regressions-list.log').read_text())
            found = {(suite['package-name'], name) for suite in listing['rust-suites'].values() for name, result in suite.get('testcases', {}).items() if result.get('filter-match', {}).get('status') == 'matches' and not result.get('ignored', False)}
            missing = {(crate, name) for crate, tests in cargo.items() for name in tests} - found
            for crate, name in sorted(missing):
                failures.extend(f'{ident}: unknown or ignored test {crate}:{name}' for ident in sorted(cargo[crate][name]))
            if not missing:
                profiles._run_command(['cargo', 'nextest', 'run', *package_args, '-E', expression, '--ignore-default-filter', '--no-fail-fast', '--no-tests=fail'], ctx, 'regressions-run.log')
        except Exception as error:
            log = ctx.artifact_dir / 'regressions-run.log'
            failed = _failed_ids(log.read_text(), cargo) if log.exists() else set()
            failures.append(f"{', '.join(sorted(failed or all_ids))}: {error}")
    if ctx.include_regression_client and clients:
        env = _client_env(binary)
        grouped = defaultdict(list)
        for ref, bug_ids in sorted(clients.items()):
            _, profile, check = ref.split(':', 2)
            command, is_profile = _check_command(profile, check)
            grouped[tuple(command)].append((profile, check, bug_ids, is_profile))
        for command, checks in grouped.items():
            try:
                profile, check, _, is_profile = checks[0]
                receipt, _ = _client_receipt(ctx, profiles, command, env, profile, check, is_profile)
            except Exception as error:
                bug_ids = set().union(*(ids for _, _, ids, _ in checks))
                failures.append(f"{', '.join(sorted(bug_ids))}: {error}")
                continue
            for profile, check, bug_ids, is_profile in checks:
                if is_profile:
                    try:
                        _profile_verdict(receipt, profile, check)
                    except Exception as error:
                        failures.append(f"{', '.join(sorted(bug_ids))}: {error}")
    if failures:
        raise RuntimeError('; '.join(failures))
    return {'open': len(bugs)-fixed, 'fixed': fixed, 'cargo_checks': sum(map(len, cargo.values())), 'client_checks': len(clients), 'client_executed': ctx.include_regression_client}
