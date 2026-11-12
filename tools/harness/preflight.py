"""Checks that can be made before a real client profile starts a process."""
from __future__ import annotations

import json
from contextlib import contextmanager
import hashlib
import os
import shutil
from importlib import import_module
from pathlib import Path

from . import runtime

_POOL = Path('/run/media/kaiserroman/games/solaris-work/targets')
# Read-only client content under XDG_DATA_HOME/solaris-client. Runs link these and
# never copy anything else (settings, caches such as far-terrain, saved runs).
CLIENT_CONTENT_DIRS = ('facts', 'assets', 'data', 'entities', 'resources', 'mojang')

_ARGUMENT_VALIDATORS = {
    'admin-commands': 'admin_commands',
    'anti-xray': 'anti_xray',
    'army-qa': 'army_qa',
    'autocraft-qa': 'autocraft_qa',
    'block-log': 'block_log',
    'buildings-qa': 'buildings_qa',
    'chisel': 'chisel',
    'composter-chance': 'composter_chance',
    'dialogs': 'dialogs',
    'dh-gate': 'dh_gate',
    'claim-only-qa': 'claim_only_qa',
    'graves': 'graves',
    'item-merge': 'item_merge',
    'logical-native': 'native_logical',
    'native-scale': 'native_scale',
    'pause-empty': 'pause_empty',
    'singleplayer-regressions': 'singleplayer_regressions',
    'survival': 'gameplay',
    'title-connect': 'title_connect',
    'village-qa': 'village_qa',
    'weak-first-view': 'weak_first_view',
}


_SCENARIO_MODULES = {
    'core-client': 'client_probes',
    'inventory': 'client_probes',
    'entities': 'client_probes',
    'native-lifecycle': 'client_probes',
    'village-autonomy-qa': 'village_autonomy_qa',
    'civilization-seed-qa': 'village_autonomy_qa',
    'village-owner-order-qa': 'village_autonomy_qa',
    'settlement-map-qa': 'settlement_map_qa',
    'seed-review': 'seed_review',
    'scene-shot': 'scene_shot',
}
_PROFILE_SCENARIO_SOURCES = {
    'survival': ('backends/survival.py',),
    'inventory': ('compatibility.py',),
    'entities': ('backends/entities_probe.py',),
    'native-lifecycle': ('backends/native_lifecycle.py',),
}
_JAVA_MCP_PROFILES = {
    'survival', 'scene-shot', 'settlement-map-qa', 'core-client',
    'inventory', 'entities', 'native-lifecycle', 'loader-live', 'dh-gate', 'seed-review',
}


def _binary(name: str, value: str, evidence: dict, prefix: str) -> None:
    path = Path(value).expanduser().resolve()
    evidence[prefix + '_path'] = str(path)
    if _POOL in path.parents and path.relative_to(_POOL).parts[0].startswith('slot-'):
        raise ValueError(f'{name} points inside a pooled target: {path}; pin a copy outside targets/slot-*')
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError(f'{name} is missing or not executable: {path}; provide a pinned executable')
    with path.open('rb') as executable:
        evidence[prefix + '_sha256'] = hashlib.file_digest(executable, 'sha256').hexdigest()


def check(profile: str, ctx, spec: dict) -> frozenset[str]:
    from .profiles import prerequisites
    needs = prerequisites(spec, ctx)
    if profile == 'scene-shot':
        from .backends import scene_shot
        scenario = scene_shot.validate_scenario(ctx)[0]
    elif profile == 'settlement-map-qa':
        from .backends import scene_shot
        scene_shot.validate_scenario(ctx, {"steps": []})
    elif profile == 'loader-live':
        from .profiles import validate_loader_live_args
        validate_loader_live_args(ctx)
    elif profile in ('regression', 'playable', 'replay'):
        from .profiles import _regression_env
        _regression_env(ctx)
    elif profile == 'catalog-load':
        from .backends import catalog_load
        extra = ctx.extra_args[1:] if ctx.extra_args[:1] == ['--'] else ctx.extra_args
        try:
            catalog_load.parse_args(['--out', str(ctx.artifact_dir / 'catalog-load'), *extra])
        except SystemExit as error:
            raise ValueError('invalid catalog-load arguments') from error
    elif profile == 'quirk-parity':
        from .quirk_parity import __main__ as quirk_parity
        extra = ctx.extra_args[1:] if ctx.extra_args[:1] == ['--'] else ctx.extra_args
        try:
            quirk_parity.parse_args(['--out', str(ctx.artifact_dir / 'quirk-parity'), *extra])
        except SystemExit as error:
            raise ValueError('invalid quirk-parity arguments') from error
    elif profile in ('seed-review', 'seed-contact-sheet'):
        module = import_module(f'.backends.{profile.replace("-", "_")}', __package__)
        try:
            module.parse_args(ctx.extra_args)
        except SystemExit as error:
            raise ValueError(f'invalid {profile} arguments') from error
    elif profile in _ARGUMENT_VALIDATORS:
        module = import_module(f'.backends.{_ARGUMENT_VALIDATORS[profile]}', __package__)
        module.validate_args(ctx)
    server = str(ctx.server_bin) if ctx.server_bin else os.environ.get('SOLARIS_SERVER_BINARY')
    if 'server_binary' in needs and not server:
        raise ValueError('SOLARIS_SERVER_BINARY is required; provide a pinned executable outside the pooled targets')
    if server and needs.intersection({'server_binary', 'optional_server_binary'}):
        _binary('SOLARIS_SERVER_BINARY', server, ctx.client_evidence, 'server_binary')
    if 'client_binary' in needs:
        client_binary = os.environ.get('SOLARIS_CLIENT_BINARY')
        if not client_binary:
            raise ValueError('SOLARIS_CLIENT_BINARY is required; provide a pinned executable outside the pooled targets')
        client_path = Path(client_binary).expanduser()
        if not client_path.is_absolute():
            client_path = runtime.sibling_root('client') / client_path
        _binary('SOLARIS_CLIENT_BINARY', str(client_path), ctx.client_evidence, 'binary')
    if needs.intersection({'client_data', 'windowed_assets'}):
        root = runtime.sibling_root('client')
        names = ['SOLARIS_CLIENT_DATA_DIR'] if 'client_data' in needs else []
        if 'windowed_assets' in needs:
            names += ['SOLARIS_CLIENT_RESOURCE_ROOT', 'SOLARIS_CLIENT_ASSET_ROOT', 'SOLARIS_CLIENT_ASSET_INDEX']
        for name in names:
            value = os.environ.get(name)
            if not value:
                raise ValueError(f'{name} is required; set it to the prepared client data path (see docs/development/real-client-gates.md)')
            path = Path(value).expanduser()
            if not path.is_absolute():
                path = root / path
            valid = path.is_file() if name.endswith('ASSET_INDEX') else path.is_dir()
            if not valid:
                raise ValueError(f'{name} is missing: {path}; provide a complete launcher data root (see docs/development/real-client-gates.md)')
            required = {
                'SOLARIS_CLIENT_DATA_DIR': ('reports/blocks.json', 'reports/block_mining.json',
                                            'reports/block_light.json', 'reports/registries.json'),
                'SOLARIS_CLIENT_RESOURCE_ROOT': ('assets',),
                'SOLARIS_CLIENT_ASSET_ROOT': ('objects',),
            }.get(name, ())
            if any(not (path / entry).exists() for entry in required):
                raise ValueError(f'{name} is incomplete: {path}; bootstrap a complete client data root')
            if name == 'SOLARIS_CLIENT_ASSET_INDEX':
                try:
                    index = json.loads(path.read_text())
                    if not isinstance(index, dict) or not index.get('objects'):
                        raise ValueError('missing objects')
                except (OSError, ValueError) as error:
                    raise ValueError(f'{name} is incomplete: {path}; bootstrap a complete asset index') from error
    if 'loader' in needs:
        loader = runtime.sibling_root('loader')
        if not (loader / 'gradlew').is_file() or not os.access(loader / 'gradlew', os.X_OK):
            raise ValueError(f"Loader checkout missing executable {loader / 'gradlew'}; set SOLARIS_LOADER_ROOT")
    if 'xvfb' in needs and not shutil.which('Xvfb'):
        raise ValueError('Xvfb is missing; install Xvfb for the isolated client')
    if 'plugins_root' in needs:
        root = runtime.sibling_root('plugins')
        if not (root / 'tools' / 'run_integration.py').is_file():
            raise ValueError(f'plugins integration runner is missing: {root}')
    if 'plugins_dir' in needs and (ctx.plugins_dir is None or not ctx.plugins_dir.is_dir()):
        raise ValueError('an existing --plugins-dir is required')
    schemas = None
    if 'client_binary' in needs:
        from . import mcp_schema
        schemas = mcp_schema.native_schema(client_path, ctx.windowed)
        if schemas is None:
            ctx.client_evidence['mcp_schema'] = 'not validated: client predates mcp-schema'
    elif ctx.client == 'java' and 'loader' in needs and profile in _JAVA_MCP_PROFILES:
        from . import mcp_schema
        schemas = mcp_schema.loader_schema(loader)
    if schemas is not None:
        if profile == 'scene-shot':
            for tool, arguments in scene_shot.scene_mcp_calls(scenario['steps']):
                if tool not in schemas:
                    raise ValueError(f'client schema does not offer required tool {tool}')
                mcp_schema.validate(schemas[tool], arguments, tool)
        sources = set()
        module_name = _SCENARIO_MODULES.get(profile, _ARGUMENT_VALIDATORS.get(profile))
        if module_name and profile != 'scene-shot':
            sources.add(Path(__file__).parent / 'backends' / f'{module_name}.py')
        sources.update(Path(__file__).parent / source
                       for source in _PROFILE_SCENARIO_SOURCES.get(profile, ()))
        if ctx.scenario_id:
            from .scenarios import SCENARIOS
            for entry in SCENARIOS.values():
                if entry.profile == profile and entry.scenario_id == ctx.scenario_id:
                    sources.update(runtime.REPO_ROOT / source for source in entry.sources
                                   if source.startswith('tools/harness/') and source.endswith('.py'))
        for path in sorted(sources):
            mcp_schema.validate_source(path, schemas)
    return needs


@contextmanager
def client_data_root(ctx):
    """Give headless native runs private writable state with linked content."""
    if ctx.client != 'native' or ctx.windowed:
        yield
        return
    import tempfile

    template = client_template()
    previous = os.environ.get('XDG_DATA_HOME')
    with tempfile.TemporaryDirectory(prefix='xdg-', dir=ctx.artifact_dir) as temporary:
        target = Path(temporary) / 'solaris-client'
        target.mkdir()
        for name in CLIENT_CONTENT_DIRS:
            source = template / name
            if source.exists():
                (target / name).symlink_to(source.resolve(), target_is_directory=source.is_dir())
        os.environ['XDG_DATA_HOME'] = temporary
        try:
            yield
        finally:
            if previous is None:
                os.environ.pop('XDG_DATA_HOME', None)
            else:
                os.environ['XDG_DATA_HOME'] = previous


def client_template() -> Path:
    """The client data root named by SOLARIS_CLIENT_DATA_DIR (<root>/data/vanilla)."""
    client_root = runtime.sibling_root('client')
    data = Path(os.environ['SOLARIS_CLIENT_DATA_DIR']).expanduser()
    template = (data if data.is_absolute() else client_root / data).resolve().parent.parent
    if not (template / 'facts').is_dir() or not (template / 'resources' / 'assets').is_dir():
        raise ValueError(f'client template {template} lacks facts or resources/assets; bootstrap a complete client data root')
    return template
