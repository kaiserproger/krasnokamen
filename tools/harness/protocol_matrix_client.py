"""MatrixClient over the canonical wire offline client, not a second runner."""
from __future__ import annotations

import copy
import json
import math
import struct
import time
import uuid
from pathlib import Path

from . import wire
from .matrix_api import MissingCapability
from .protocol_station_verbs import ProtocolStationVerbs
from .protocol_input_verbs import ProtocolInputVerbs


class ProtocolMatrixClient(ProtocolStationVerbs, ProtocolInputVerbs):
    def __init__(self, name: str, address: tuple[str, int], artifact_dir: Path,
                 vanilla_data: Path, fixture_command, *, account_uuid: uuid.UUID,
                 timeout_seconds: float = 30, require_auth: bool = False):
        self.name = name
        self.address = address
        self.artifact_dir = artifact_dir
        self.fixture_command = fixture_command
        self.account_uuid = account_uuid
        self.timeout_seconds = timeout_seconds
        self.require_auth = require_auth
        self.auto_auth = True
        self.generation = 0
        self._event_history = []
        self.protocol = json.loads((vanilla_data / 'version.json').read_text())['protocol_version']
        registries = json.loads((vanilla_data / 'reports/registries.json').read_text())
        self.items = {value['protocol_id']: key for key, value in registries['minecraft:item']['entries'].items()}
        self.entity_types = {value['protocol_id']: key for key, value in
                             registries['minecraft:entity_type']['entries'].items()}
        self.static_registries = {
            registry: {str(entry['protocol_id']): name
                       for name, entry in registries[registry]['entries'].items()}
            for registry in ('minecraft:potion', 'minecraft:mob_effect')}
        blocks = json.loads((vanilla_data / 'reports/blocks.json').read_text())
        self.block_states = {state['id']: {'block_id': name, 'properties': state.get('properties', {})}
                             for name, block in blocks.items() for state in block['states']}
        self.observer = None
        self.chunk_min_y = None
        self.reconnect()

    def reconnect(self):
        if self.observer is not None and not self.observer.closed:
            raise RuntimeError('reconnect requires disconnect first')
        generation = self.generation + 1
        observer = wire.PlayObserver(self.address, self.name, account_uuid=self.account_uuid,
                          timeout=self.timeout_seconds,
                          trace_path=self.artifact_dir / f'{self.name}-{generation}.wire.jsonl',
                          protocol=self.protocol)
        history = getattr(self, '_event_history', [])
        if self.observer is not None:
            # Reconnect requires the old reader already closed. Archive only
            # after construction succeeds, so a failed join cannot duplicate it.
            history.extend(self.observer.events)
        self._event_history = history
        self.generation = generation
        self.observer = observer
        self.observer.chunk_min_y = getattr(self, 'chunk_min_y', None)
        self.observer.ready()
        if self.require_auth and self.auto_auth:
            self.wait_until(lambda: any('/register' in event or '/login' in event
                                       for event in self.observer.events),
                            description='authentication prompt')
            register = any('/register' in event for event in self.observer.events)
            self.command('register MatrixProbe42 MatrixProbe42' if register else 'login MatrixProbe42')
            self.wait_until(lambda: any('Successfully logged in' in event for event in self.observer.events),
                            description='authentication unlock')

    def set_auto_auth(self, enabled: bool) -> None:
        """Control client login automation, never the server's auth policy."""
        if not isinstance(enabled, bool):
            raise TypeError('auto auth control must be bool')
        self.auto_auth = enabled

    def disconnect(self):
        if self.observer is not None:
            self.observer.close()

    def _stack(self, stack, slot=None):
        if stack is None:
            return None
        item_id = self.items.get(stack['item_id'])
        if item_id is None:
            raise MissingCapability(f"item_registry_id:{stack['item_id']}")
        result = {'item_id': item_id, 'count': stack['count'],
                  'components': copy.deepcopy(stack['components']),
                  'removed': list(stack['removed'])}
        if slot is not None:
            result['slot'] = slot
        return result

    def _cost(self, cost):
        if cost is None:
            return None
        item_id = self.items.get(cost['item_id'])
        if item_id is None:
            raise MissingCapability(f"item_registry_id:{cost['item_id']}")
        return {'item_id': item_id, 'count': cost['count']}

    def snapshot(self):
        observer = self.observer
        with observer.condition:
            result = {'position': copy.deepcopy(observer.position),
                      'events': list(getattr(self, '_event_history', [])) + list(observer.events),
                      'generation': getattr(self, 'generation', 0),
                      'health': observer.health, 'food': observer.food,
                      'revision': observer.revision,
                      'block_ack_sequence': getattr(observer, 'block_ack_sequence', None),
                      'passengers': copy.deepcopy(getattr(observer, 'passengers', {})),
                      'loaded_chunks': [list(chunk) for chunk in sorted(observer.chunks)]}
            accepted_identity = getattr(observer, 'join', {})
            for field in ('uuid', 'entity_id'):
                if field in accepted_identity:
                    result[field] = accepted_identity[field]
            result['chat_components'] = copy.deepcopy(list(observer.chat_components))
            result['observation_errors'] = {
                'menus': dict(observer.menu_errors), 'offers': dict(observer.merchant_errors),
                'chat': list(observer.chat_errors)}
            registry_names = copy.deepcopy(getattr(self, 'static_registries', {}))
            registry_sources = {name: 'vanilla_report' for name in registry_names}
            received_registries = getattr(observer, 'join', {}).get('registries', {})
            registry_names.update(copy.deepcopy(received_registries))
            registry_sources.update({name: 'received_configuration'
                                     for name in received_registries})
            result['registries'] = registry_names
            result['registry_sources'] = registry_sources
            if hasattr(observer, 'entities'):
                observed = observer.entities.snapshot()
                if observed['unavailable'] is not None:
                    result['observation_errors']['entities'] = observed['unavailable']
                else:
                    entities = observed['entities']
                    for entity in entities:
                        type_id = entity['type']
                        entity['type'] = self.entity_types.get(type_id)
                        if entity['type'] is None:
                            result['observation_errors']['entities'] = (
                                f'unknown_entity_registry_id:{type_id}')
                            break
                        if 'item' in entity:
                            entity['item'] = self._stack(entity['item'])
                    else:
                        result['entities'] = entities
                    if observed['xp'] is not None:
                        result['xp'] = observed['xp']
                    if observed['local'] is not None and 'effects' in observed['local']:
                        result['effects'] = observed['local']['effects']
            player = observer.menus.get(0)
            window = observer.current_window
            menu = observer.menus.get(window)
            if (player is not None and 0 not in observer.menu_errors
                    and menu is not None and window not in observer.menu_errors):
                inventory = list(player['slots'])
                if window != 0:
                    # Every vanilla menu appends main inventory then hotbar.
                    # Those received slots supersede the stale window-0 cache.
                    if len(menu['slots']) < 36 or len(inventory) < 45:
                        raise MissingCapability('snapshot:player_inventory_suffix')
                    inventory[9:36] = menu['slots'][-36:-9]
                    inventory[36:45] = menu['slots'][-9:]
                result['inventory'] = [self._stack(stack, slot) for slot, stack in enumerate(inventory) if stack]
            if menu is not None and window not in observer.menu_errors:
                result['menu'] = {'id': window, 'state_id': menu['state_id'],
                                  'slot_count': len(menu['slots']),
                                  'type_id': observer.menu_types.get(window),
                                  'properties': copy.deepcopy(observer.menu_properties.get(window, {})),
                                  'slots': [self._stack(stack, slot) for slot, stack in enumerate(menu['slots']) if stack]}
                result['cursor'] = self._stack(menu['cursor'])
                merchant = observer.merchant_offers.get(window)
                if merchant is not None and window not in observer.merchant_errors:
                    result['menu'].update({key: merchant[key] for key in
                                           ('level','xp','show_progress','can_restock')})
                    offers = []
                    for offer in merchant['offers']:
                        normalized = {key: value for key, value in offer.items()
                                      if key not in {'input1','input2','output'}}
                        normalized.update(input1=self._cost(offer['input1']),
                                          input2=self._cost(offer['input2']),
                                          output=self._stack(offer['output']))
                        offers.append(normalized)
                    result['menu']['offers'] = offers
            return result

    def wait_until(self, predicate, *, description, timeout_seconds=30):
        return self.observer.wait(lambda _: predicate(), description=description, timeout=timeout_seconds)

    def command(self, text):
        self.observer.send(0x07, wire.mc_string(text.removeprefix('/')))
        self.observer.send(0x0D)

    def chat(self, text):
        self.observer.send(0x09, wire.mc_string(text) + struct.pack('>qq', int(time.time()*1000), 0)
                           + b'\x00\x00\x00\x00\x00\x00')

    def teleport_to(self, x, y, z):
        before = self.observer.teleports
        self.fixture_command(f'tp {self.name} {x} {y} {z}')
        self.wait_until(lambda: self.observer.teleports > before,
                        description='received fixture teleport', timeout_seconds=self.timeout_seconds)
        target_chunk = (math.floor(x) // 16, math.floor(z) // 16)
        self.wait_until(lambda: target_chunk in self.observer.chunks,
                        description='received target chunk after teleport', timeout_seconds=self.timeout_seconds)

    def look_at(self, x, y, z):
        position = self.snapshot()['position']
        if position is None:
            raise MissingCapability('snapshot:position')
        dx, dy, dz = x-position[0], y-(position[1]+1.62), z-position[2]
        yaw = math.degrees(math.atan2(-dx, dz))
        pitch = -math.degrees(math.atan2(dy, math.hypot(dx, dz)))
        self.rotation = (yaw, pitch)
        self.observer.send(0x20, struct.pack('>ffB', yaw, pitch, 1))
        self.observer.send(0x0D)

    def use_item_on(self, position, face='up', hand='main', cursor=(0.5, 0.5, 0.5)):
        directions = {'down':0, 'up':1, 'north':2, 'south':3, 'west':4, 'east':5}
        if face not in directions:
            raise ValueError('unknown block face ' + face)
        self.observer.sequence += 1
        if hand not in {'main', 'off'}:
            raise ValueError('unknown hand ' + hand)
        self.observer.send(0x42, wire.write_varint(0 if hand == 'main' else 1)
                           + wire.pack_block_position(tuple(position))
                           + wire.write_varint(directions[face]) + struct.pack('>fff', *cursor)
                           + b'\x00\x00' + wire.write_varint(self.observer.sequence))
        self.observer.send(0x0D)
        return self.observer.sequence

    def attack(self, entity_id):
        self.observer.send(0x01, wire.write_varint(entity_id))
        self.observer.send(0x0D)

    def click_slot(self, slot, kind='pickup', button=0):
        modes = {'pickup':0, 'quick_move':1, 'swap':2, 'clone':3, 'throw':4, 'quick_craft':5, 'pickup_all':6}
        if kind not in modes:
            raise ValueError('unknown click kind ' + kind)
        self.observer.click(self.observer.current_window, slot, button=button, mode=modes[kind])

    def open_container(self, position):
        old = self.observer.current_window
        self.use_item_on(position)
        self.wait_until(lambda: self.observer.current_window != old,
                        description='received OpenScreen', timeout_seconds=self.timeout_seconds)
        self.wait_until(lambda: self.observer.current_window in self.observer.menus,
                        description='received menu contents', timeout_seconds=self.timeout_seconds)

    def close_container(self):
        self.observer.close_menu(self.observer.current_window)

    def open_inventory(self):
        self.close_container()
        self.wait_until(lambda: 0 in self.observer.menus, description='received player inventory')

    def drop(self, all=False):
        self.observer.sequence += 1
        # interaction.rs ServerboundPlayerAction 0x29; direction=down.
        self.observer.send(0x29, wire.write_varint(4 if all else 5)
                           + wire.pack_block_position((0,0,0)) + b'\x00'
                           + wire.write_varint(self.observer.sequence))
        self.observer.send(0x0D)

    def select_hotbar(self, slot):
        if not 0 <= slot <= 8:
            raise ValueError('hotbar slot outside 0..8')
        self.observer.send(0x36, struct.pack('>h', slot))
        self.observer.send(0x0D)

    def read_block_state(self, position):
        position = tuple(position)
        observer = self.observer
        with self.observer.condition:
            state = self.observer.blocks.get(position)
            minimum = getattr(observer, 'chunk_min_y', None)
            sections = getattr(observer, 'chunk_sections', {}).get((position[0] // 16, position[2] // 16))
            if state is None and minimum is not None and sections is not None:
                from .protocol_chunks import block_state
                state = block_state(sections, position, minimum)
            if state is None:
                raise MissingCapability('snapshot:block_state_not_received:' + str(position))
            if state not in self.block_states:
                raise MissingCapability('block_registry_state:' + str(state))
            return copy.deepcopy(self.block_states[state])

    def wait_for_chunk(self, position):
        chunk = (math.floor(position[0]) // 16, math.floor(position[2]) // 16)
        return self.wait_until(lambda: chunk in self.observer.chunks,
                               description='received target chunk ' + str(chunk),
                               timeout_seconds=self.timeout_seconds)

    def ensure_loaded(self, area):
        """Fixture-only chunk readiness from actual received chunk packets."""
        low, high = area
        chunks = {(x, z) for x in range(low[0] // 16, high[0] // 16 + 1)
                  for z in range(low[2] // 16, high[2] // 16 + 1)}
        original = self.snapshot()['position']
        if original is None:
            raise MissingCapability('snapshot:position')
        moved = False
        for chunk in sorted(chunks):
            if chunk not in self.observer.chunks:
                self.teleport_to(chunk[0] * 16 + 8.5, original[1], chunk[1] * 16 + 8.5)
                moved = True
            self.wait_for_chunk((chunk[0] * 16, 0, chunk[1] * 16))
        if moved:
            self.teleport_to(*original)

