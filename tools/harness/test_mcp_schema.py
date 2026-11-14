"""Pre-start validation uses the selected client's exported argument schemas."""
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import mcp_schema, preflight, profiles
from .backends import autocraft_qa


class SchemaTests(unittest.TestCase):
    def test_binary_without_export_fails_closed(self):
        with self.assertRaisesRegex(mcp_schema.SchemaError, 'cannot read MCP schemas'):
            mcp_schema.native_schema(Path('/bin/true'), True)

    def test_release_predating_export_is_recorded_not_validated(self):
        with tempfile.TemporaryDirectory() as tmp:
            old = Path(tmp) / 'krasnokamen-client'
            old.write_text('#!/bin/sh\necho "krasnokamen-client: unknown_command: $1" >&2\nexit 1\n')
            old.chmod(0o755)
            self.assertIsNone(mcp_schema.native_schema(old, True))

    def test_native_census_extensions_come_from_selected_binary(self):
        properties = {
            'radius': {'type': 'number', 'minimum': 1, 'maximum': 128},
            'limit': {'type': 'integer', 'minimum': 1, 'maximum': 512},
            'entity_type': {'type': 'string', 'minLength': 1, 'maxLength': 128},
            'cursor': {'type': 'string', 'minLength': 0, 'maxLength': 20},
        }
        schema = {'type': 'object', 'properties': properties, 'additionalProperties': False}
        tool = {'name': 'minecraft_list_entities', 'inputSchema': schema}
        exported = json.dumps({'tools': [tool], 'headless_tools': [tool]})
        result = mock.Mock(returncode=0, stdout=exported, stderr='')
        with mock.patch.object(mcp_schema.subprocess, 'run', return_value=result):
            for windowed in (False, True):
                selected = mcp_schema.native_schema(Path('/selected/native-client'), windowed)
                mcp_schema.validate(selected['minecraft_list_entities'], {
                    'radius': 128, 'limit': 16, 'entity_type': 'minecraft:villager', 'cursor': '',
                })
                with self.assertRaises(mcp_schema.SchemaError):
                    mcp_schema.validate(selected['minecraft_list_entities'], {'entity_type': ''})
        older = {'type': 'object', 'properties': {
            key: value for key, value in properties.items() if key not in ('entity_type', 'cursor')
        }, 'additionalProperties': False}
        with self.assertRaises(mcp_schema.SchemaError):
            mcp_schema.validate(older, {'radius': 128, 'limit': 16, 'cursor': ''})

    def test_autocraft_literal_is_checked_before_game_launch(self):
        schema = {'type': 'object', 'properties': {
            'ticks': {'type': 'integer', 'minimum': 1, 'maximum': 255}},
            'required': ['ticks'], 'additionalProperties': False}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            client = root / 'client'
            catalog = {'tools': [{'name': 'minecraft_wait_ticks', 'inputSchema': schema}],
                       'headless_tools': [{'name': 'minecraft_wait_ticks', 'inputSchema': schema}]}
            client.write_text('#!/bin/sh\nprintf %s ' + repr(json.dumps(catalog)) + '\n')
            client.chmod(0o755)
            data = root / 'data'
            (data / 'reports').mkdir(parents=True)
            for name in ('blocks.json', 'block_mining.json', 'block_light.json', 'registries.json'):
                (data / 'reports' / name).touch()
            sources = root / 'backends'
            sources.mkdir()
            original = Path(autocraft_qa.__file__).read_text()
            self.assertIn('\"ticks\": 240', original)
            (sources / 'autocraft_qa.py').write_text(original.replace('\"ticks\": 240', '\"ticks\": 1200'))
            ctx = profiles.ProfileContext(root, client='native')
            env = {'KRASNOKAMEN_CLIENT_BINARY': str(client), 'KRASNOKAMEN_SERVER_BINARY': '/bin/true',
                   'KRASNOKAMEN_CLIENT_DATA_DIR': str(data)}
            with mock.patch.dict(os.environ, env), mock.patch.object(preflight, '__file__', str(root / 'preflight.py')):
                with self.assertRaises(mcp_schema.SchemaError):
                    preflight.check('autocraft-qa', ctx, profiles.PROFILES['autocraft-qa'])

    def test_known_literal_argument_checked_against_export(self):
        schemas = {'minecraft_wait_ticks': {'type': 'object', 'properties': {
            'ticks': {'type': 'integer', 'minimum': 1, 'maximum': 255}},
            'required': ['ticks'], 'additionalProperties': False}}
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'scenario.py'
            source.write_text('probe.call("minecraft_wait_ticks", {"ticks": 256})\n')
            with self.assertRaises(mcp_schema.SchemaError):
                mcp_schema.validate_source(source, schemas)
            source.write_text('probe.call("minecraft_wait_ticks", {"ticks": 255})\n')
            mcp_schema.validate_source(source, schemas)
            source.write_text('GAP_TICKS = 1200\nprobe.call("minecraft_wait_ticks", {"ticks": GAP_TICKS})\n')
            with self.assertRaises(mcp_schema.SchemaError):
                mcp_schema.validate_source(source, schemas)
            source.write_text('def run(probe):\n    ticks = 1200\n    probe.call("minecraft_wait_ticks", {"ticks": ticks})\n')
            with self.assertRaises(mcp_schema.SchemaError):
                mcp_schema.validate_source(source, schemas)
            source.write_text('GAP_TICKS = 1200\ndef run(probe, GAP_TICKS):\n    probe.call("minecraft_wait_ticks", {"ticks": GAP_TICKS})\n')
            mcp_schema.validate_source(source, schemas)

    def test_unknown_dynamic_value_does_not_hide_static_invalid_argument(self):
        schemas = {'minecraft_move': {'type': 'object', 'properties': {
            'x': {'type': 'integer', 'minimum': 0}, 'y': {'type': 'integer'}},
            'required': ['x', 'y'], 'additionalProperties': False}}
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'scenario.py'
            source.write_text('probe.call("minecraft_move", {"x": -1, "y": runtime_y})\n')
            with self.assertRaises(mcp_schema.SchemaError):
                mcp_schema.validate_source(source, schemas)
