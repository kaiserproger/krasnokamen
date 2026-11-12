"""Validate statically known MCP arguments against the selected client's schema."""
from __future__ import annotations

import ast
import json
import math
import os
import re
import subprocess
import tempfile
from pathlib import Path
from typing import Any


class SchemaError(ValueError):
    pass


_UNKNOWN = object()


def _fully_known(value: Any) -> bool:
    if value is _UNKNOWN:
        return False
    if isinstance(value, dict):
        return all(_fully_known(key) and _fully_known(item) for key, item in value.items())
    if isinstance(value, list):
        return all(_fully_known(item) for item in value)
    return True


def native_schema(binary: Path, windowed: bool) -> dict[str, dict] | None:
    """The client's exported schemas, or None for a release that predates `mcp-schema`.

    Older pinned clients stay valid gate subjects (negative controls, released-client
    checks); they refuse the subcommand with `unknown_command: mcp-schema` and the
    client itself still validates every call at run time. Any other failure fails closed.

    Optional native census arguments (entity_type and cursor) come from this exact
    binary, not the Loader's catalog. Never merge the two clients' capabilities.
    """
    try:
        result = subprocess.run([str(binary), 'mcp-schema'], capture_output=True, text=True,
                                timeout=15)
        if result.returncode != 0 and 'unknown_command: mcp-schema' in result.stderr:
            return None
        if result.returncode != 0:
            raise ValueError(f'mcp-schema exited {result.returncode}: {result.stderr.strip()[-300:]}')
        payload = json.loads(result.stdout)
        tools = payload['tools' if windowed else 'headless_tools']
        if not isinstance(tools, list) or not tools:
            raise ValueError('empty tool list')
        schemas = {tool['name']: tool['inputSchema'] for tool in tools}
        if (len(schemas) != len(tools)
                or any(not isinstance(name, str) or not isinstance(schema, dict)
                       for name, schema in schemas.items())):
            raise ValueError('invalid tool schemas')
        return schemas
    except (OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError) as error:
        raise SchemaError(f'cannot read MCP schemas from client {binary}: {error}') from error


def loader_schema(loader: Path) -> dict[str, dict]:
    """Compile the checkout's actual Java catalog, without starting a client."""
    try:
        build = (loader / 'bridge-core' / 'build.gradle.kts').read_text()
        matches = re.findall(r'com\.google\.code\.gson:gson:([0-9.]+)', build)
        if len(matches) != 1:
            raise ValueError('cannot identify Loader Gson version')
        version = matches[0]
        from . import runtime
        gradle = runtime.ensure_loader_gradle_home(loader)
        jars = sorted((gradle / 'caches/modules-2/files-2.1/com.google.code.gson/gson' / version)
                      .glob(f'*/gson-{version}.jar'))
        if not jars:
            raise ValueError(f'Loader Gson {version} is not in the Gradle cache')
        source = loader / 'bridge-core/src/main/java/dev/solaris/agent/mcp'
        exporter = Path(__file__).with_name('LoaderSchema.java')
        with tempfile.TemporaryDirectory(prefix='loader-schema-') as directory:
            subprocess.run(['javac', '-J-Xmx128m', '-cp', str(jars[0]), '-d', directory,
                            str(source / 'McpToolDefinition.java'), str(source / 'ClientMcpTools.java'),
                            str(exporter)], capture_output=True, text=True, timeout=60, check=True)
            output = subprocess.run(['java', '-Xmx128m', '-cp', os.pathsep.join((directory, str(jars[0]))),
                                     'dev.solaris.agent.mcp.LoaderSchema'], capture_output=True,
                                    text=True, timeout=30, check=True)
            tools = json.loads(output.stdout)['tools']
        schemas = {tool['name']: tool['inputSchema'] for tool in tools}
        if not schemas or len(schemas) != len(tools):
            raise ValueError('invalid Loader tool catalog')
        return schemas
    except (OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError) as error:
        detail = getattr(error, 'stderr', None) or str(error)
        raise SchemaError(f'cannot load MCP schemas from Loader checkout {loader}: {detail}') from error


def validate(schema: dict, value: Any, path: str = 'arguments') -> None:
    if value is _UNKNOWN:
        return
    def fail(reason: str) -> None:
        raise SchemaError(f'{path}: {reason}')
    if 'anyOf' in schema:
        for alternative in schema['anyOf']:
            try:
                validate(alternative, value, path)
                break
            except SchemaError:
                continue
        else:
            fail('does not match a declared alternative')
    if 'enum' in schema and value not in schema['enum']:
        fail('outside declared enum')
    kind = schema.get('type')
    if kind == 'object':
        if not isinstance(value, dict):
            fail('must be an object')
        for key in schema.get('required', []):
            if key not in value and _UNKNOWN not in value:
                fail(f'missing {key}')
        if len(value) > schema.get('maxProperties', float('inf')):
            fail('too many properties')
        for key, dependencies in schema.get('dependentRequired', {}).items():
            if key in value and any(dep not in value for dep in dependencies):
                fail(f'{key} requires {dependencies}')
        for key, item in value.items():
            if key is _UNKNOWN:
                continue
            prop = schema.get('properties', {}).get(key)
            if prop is not None:
                validate(prop, item, f'{path}.{key}')
            elif schema.get('additionalProperties') is False:
                fail(f'undeclared {key}')
            elif isinstance(schema.get('additionalProperties'), dict):
                validate(schema['additionalProperties'], item, f'{path}.{key}')
    elif kind == 'array':
        if not isinstance(value, list):
            fail('must be an array')
        if not schema.get('minItems', 0) <= len(value) <= schema.get('maxItems', float('inf')):
            fail('violates item count')
        if schema.get('uniqueItems') and _fully_known(value):
            if len({json.dumps(v, sort_keys=True) for v in value}) != len(value):
                fail('items must be unique')
        if 'items' in schema:
            for index, item in enumerate(value):
                validate(schema['items'], item, f'{path}[{index}]')
    elif kind == 'string':
        if not isinstance(value, str):
            fail('must be a string')
        length = len(value.encode('utf-16-le')) // 2
        if not schema.get('minLength', 0) <= length <= schema.get('maxLength', float('inf')):
            fail('violates string length')
        if 'pattern' in schema and not re.search(schema['pattern'], value):
            fail('violates pattern')
    elif kind in ('integer', 'number'):
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
            fail('must be a finite number')
        if kind == 'integer' and value != int(value):
            fail('must be an integer')
        if not schema.get('minimum', -float('inf')) <= value <= schema.get('maximum', float('inf')):
            fail('outside declared numeric range')
    elif kind == 'boolean' and not isinstance(value, bool):
        fail('must be a boolean')
    elif kind not in ('boolean', None):
        fail(f'unsupported schema type {kind!r}')


def _static(node: ast.AST, constants: dict[str, Any] | None = None) -> Any:
    constants = constants or {}
    if isinstance(node, ast.Name):
        return constants.get(node.id, _UNKNOWN)
    if isinstance(node, ast.Dict):
        result = {}
        for key, value in zip(node.keys, node.values):
            if key is None:
                result[_UNKNOWN] = _UNKNOWN
                continue
            try:
                known_key = ast.literal_eval(key)
            except (ValueError, TypeError):
                result[_UNKNOWN] = _UNKNOWN
                continue
            result[known_key] = _static(value, constants)
        return result
    if isinstance(node, (ast.List, ast.Tuple)):
        return [_static(item, constants) for item in node.elts]
    try:
        return ast.literal_eval(node)
    except (ValueError, TypeError):
        return _UNKNOWN


def validate_source(path: Path, schemas: dict[str, dict]) -> None:
    """Check literal call names and statically known argument fragments in a module."""
    tree = ast.parse(path.read_text(encoding='utf-8'), filename=str(path))
    assignments: dict[str, int] = {}
    for item in ast.walk(tree):
        if isinstance(item, ast.Name) and isinstance(item.ctx, ast.Store):
            assignments[item.id] = assignments.get(item.id, 0) + 1
    constants = {}
    for item in tree.body:
        if isinstance(item, ast.Assign) and len(item.targets) == 1 and isinstance(item.targets[0], ast.Name):
            name = item.targets[0].id
            if name.isupper() and assignments.get(name) == 1:
                value = _static(item.value)
                if value is None or type(value) in (str, int, float, bool):
                    constants[name] = value
    parents = {child: parent for parent in ast.walk(tree) for child in ast.iter_child_nodes(parent)}
    local_cache = {}
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call) or not isinstance(node.func, ast.Attribute):
            continue
        if node.func.attr not in ('call', 'call_tool', 'retry_tool'):
            continue
        index = 1 if node.func.attr == 'retry_tool' else 0
        name_node = node.args[index] if len(node.args) > index else next(
            (kw.value for kw in node.keywords if kw.arg == 'name'), None)
        if name_node is None:
            continue
        scope = parents.get(node)
        while scope is not None and not isinstance(scope, (ast.FunctionDef, ast.AsyncFunctionDef)):
            scope = parents.get(scope)
        known = dict(constants)
        if scope is not None:
            if scope not in local_cache:
                stores: dict[str, int] = {}
                for item in ast.walk(scope):
                    if isinstance(item, ast.Name) and isinstance(item.ctx, ast.Store):
                        stores[item.id] = stores.get(item.id, 0) + 1
                parameters = {arg.arg for arg in (*scope.args.posonlyargs, *scope.args.args,
                                                  *scope.args.kwonlyargs)}
                parameters.update(arg.arg for arg in (scope.args.vararg, scope.args.kwarg) if arg)
                local_cache[scope] = (stores, parameters)
            stores, parameters = local_cache[scope]
            for name in stores.keys() | parameters:
                known.pop(name, None)
            for statement in scope.body:
                if statement.lineno >= node.lineno:
                    break
                if isinstance(statement, ast.Assign) and len(statement.targets) == 1 and isinstance(statement.targets[0], ast.Name):
                    name = statement.targets[0].id
                    if stores.get(name) == 1:
                        value = _static(statement.value, known)
                        if value is None or type(value) in (str, int, float, bool):
                            known[name] = value
        tool = _static(name_node, known)
        if not isinstance(tool, str) or not tool.startswith(('minecraft_', 'client_')):
            continue
        if tool not in schemas:
            continue  # Other presentation modes or clients may own this call site.
        argument = node.args[index + 1] if len(node.args) > index + 1 else None
        argument = next((kw.value for kw in node.keywords if kw.arg == 'arguments'), argument)
        values = {} if argument is None else _static(argument, known)
        try:
            validate(schemas[tool], values)
        except SchemaError as error:
            raise SchemaError(f'{path}:{node.lineno}: {tool}: {error}') from error


