"""Pre-start validation uses the selected client's exported argument schemas."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from . import mcp_schema


class SchemaTests(unittest.TestCase):
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
