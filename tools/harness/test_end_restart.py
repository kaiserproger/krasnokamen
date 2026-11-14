"""Bounded End restart capability; adapter-contract tests, not matrix credit.

Regression TEST-END-RESTART-OBSERVER: the earlier causal replay bound the
pre-restart dragon UUID and then accepted a late census of a dragon that had
flown beyond the original observers' tracking range as proof of failed
persistence. The fixture must move the observers beside the live dragon after
the restart and must never move, re-summon or rewrite the persisted dragon.
"""
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from .backends import protocol_end_restart as end_restart

DRAGON = '0d0c0b0a-0908-0706-0504-030201000000'
OTHER = 'ffffffff-eeee-dddd-cccc-bbbb-aaaaaaaaaaaa'
OBSERVER_MOVE = ('execute in minecraft:the_end at '
                 '@e[type=minecraft:ender_dragon,limit=1] run tp ')

LOG = ('[ INFO ] native checkpoint writer stopped after shutdown requested offset=2609\n'
       '[ INFO ] orderly checkpoint complete\n')


class Server:
    def __init__(self, log_path: Path):
        self.commands: list[str] = []
        self.dimension = 'overworld'
        self.log_path = log_path
        self.lifecycle: list[str] = []

    def command(self, text):
        self.commands.append(text)
        return ''

    query = command

    def save_all(self):
        self.lifecycle.append('save_all')

    def clean_restart(self):
        self.lifecycle.append('clean_restart')
        return {'same_world': True}


class Client:
    def __init__(self, name, uuid=DRAGON, after_uuid=None, dragon_after=True):
        self.name = name
        self.uuid = uuid
        self.after_uuid = after_uuid
        self.dragon_after = dragon_after
        self.restarted = False
        self.lifecycle: list[str] = []
        self.waits: list[str] = []

    def _visible(self):
        if self.restarted:
            return self.dragon_after
        return True

    def snapshot(self):
        if not self._visible():
            return {'entities': []}
        uuid = self.after_uuid if self.restarted and self.after_uuid else self.uuid
        return {'entities': [{'type': 'minecraft:ender_dragon', 'uuid': uuid,
                              'position': [0.0, 128.0, 0.0]}]}

    def wait_until(self, predicate, *, description, timeout_seconds=30.0):
        self.waits.append(description)
        value = predicate()
        if not value:
            raise AssertionError(description)
        return value

    def disconnect(self):
        self.lifecycle.append('disconnect')

    def reconnect(self):
        self.lifecycle.append('reconnect')
        self.restarted = True


class EndRestartObserverTests(unittest.TestCase):
    def run_restart(self, clients=None, log=LOG):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        log_path = Path(directory.name) / 'server.log'
        log_path.write_text(log)
        server = Server(log_path)
        if clients is None:
            clients = [Client(f'Proto{index}') for index in range(8)]
        report: dict = {}
        end_restart.run(server, clients, report)
        return server, clients, report

    def test_restart_moves_observers_beside_the_dragon_and_never_the_dragon(self):
        server, clients, report = self.run_restart()
        moved = [text for text in server.commands if text.startswith(OBSERVER_MOVE)]
        self.assertEqual([text.removeprefix(OBSERVER_MOVE).removesuffix(' ~ ~ ~') for text in moved],
                         [client.name for client in clients])
        for text in moved:
            self.assertTrue(text.endswith(' ~ ~ ~'), text)
        # The fixture owns the observers only: the persisted dragon keeps its
        # own position, identity and state across the restart.
        self.assertFalse([text for text in server.commands if 'summon' in text])
        self.assertFalse([text for text in server.commands if 'data merge' in text])
        self.assertFalse([text for text in server.commands
                          if 'run tp @e[type=minecraft:ender_dragon' in text])
        self.assertEqual([client.lifecycle for client in clients],
                         [['disconnect', 'reconnect']] * 8)
        self.assertEqual(server.lifecycle, ['save_all', 'clean_restart'])
        self.assertEqual(report['restart_observer_fixture'],
                         'teleport each observer beside the live End dragon')
        self.assertEqual([check['name'] for check in report['checks']],
                         ['eight_clients_observe_same_dragon_before_restart',
                          'end_boots_with_same_retained_dragon_after_restart',
                          'orderly_save_restart_has_no_error_records'])
        self.assertEqual(report['after'][0]['uuid'], DRAGON)

    def test_a_dragon_beyond_tracking_range_after_restart_is_a_failure(self):
        clients = [Client(f'Proto{index}') for index in range(8)]
        clients[5].dragon_after = False
        with self.assertRaisesRegex(AssertionError, 'Proto5 receives the recovered End dragon'):
            self.run_restart(clients)

    def test_a_different_dragon_identity_after_recovery_is_a_failure(self):
        clients = [Client(f'Proto{index}') for index in range(8)]
        clients[2].after_uuid = OTHER
        with self.assertRaisesRegex(AssertionError, 'identity changed across native recovery'):
            self.run_restart(clients)

    def test_restart_capability_requires_eight_real_joined_clients(self):
        with self.assertRaisesRegex(AssertionError, 'eight real joined clients'):
            self.run_restart([Client(f'Proto{index}') for index in range(7)])

    def test_a_new_error_record_fails_the_capability(self):
        with self.assertRaisesRegex(AssertionError, 'save/restart emitted errors'):
            self.run_restart(log=LOG + '[ERROR] wal replay failed\n')

    def test_missing_info_level_writer_shutdown_receipt_fails_the_capability(self):
        with self.assertRaisesRegex(AssertionError, 'native writer shutdown receipt'):
            self.run_restart(log='native checkpoint writer stopped after shutdown requested\n')


if __name__ == '__main__':
    unittest.main()
