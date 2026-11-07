"""Shared executable input verbs for matrix protocol clients.

Mixin for ``ProtocolMatrixClient`` (imported by the parent; this module never
imports the client, so there is no import cycle). Every verb writes a real
26.1.2 serverbound packet encoded from local ``mc-protocol``/``mc-domain``
source evidence, then a ``ClientTickEnd`` (``0x0D``) frame. Completion is never
claimed here: outcomes are observed separately via ``snapshot``/server query.

Packet evidence (all IDs/fields from the local repo, not memory):

- ``ServerboundPlayerAction`` ``0x29``: VarInt action, i64 packed pos, VarInt
  direction, VarInt sequence; actions ``StartDestroyBlock=0``,
  ``AbortDestroyBlock=1``, ``StopDestroyBlock=2``, ``DropAllItems=3``,
  ``DropItem=4``, ``ReleaseUseItem=5``, ``SwapItemWithOffhand=6``
  (``crates/mc-protocol/src/packets/play/interaction.rs`` ``PlayerActionKind``,
  ``impl Packet for ServerboundPlayerAction``).
- Direction ordinals ``down=0 up=1 north=2 south=3 west=4 east=5``
  (``crates/mc-domain/src/lib.rs`` ``Direction``); hand ordinals
  ``MainHand=0 OffHand=1`` (``InteractionHand``).
- ``ServerboundUseItem`` ``0x43``: VarInt hand, VarInt sequence, f32 yRot,
  f32 xRot (``interaction.rs``; vector
  ``[0x00, 0xAC, 0x02, 42, B4, 0, 0, C1, 70, 0, 0]`` for main/seq 300/yaw
  90/pitch -15 in ``play/tests/serverbound_and_slots.rs``).
- ``ServerboundInteract`` ``0x1A``: VarInt entity, VarInt hand, LP Vec3, bool
  secondary (``interaction.rs``; vector ``[0x7B, 0x00, 0x00, 0x00]`` for entity
  123, main hand, zero location, not sneaking; LP zero is one ``0x00`` byte
  from ``write_lp_vec3`` in ``packets/play.rs``).
- ``ServerboundMovePlayerPos`` ``0x1E``: f64 x/y/z, u8 flags bit0 on_ground
  (``interaction.rs`` ``MovePlayerFlags``; IDs ``0x1E/0x1F/0x20/0x21`` pinned in
  ``packets/play/tests.rs`` ``move_player_ids_match_javap``).
- ``ServerboundPlayerInput`` ``0x2B``: one u8 bitset forward ``0x01``,
  backward ``0x02``, left ``0x04``, right ``0x08``, jump ``0x10``,
  shift ``0x20``, sprint ``0x40`` (``interaction.rs`` ``PlayerInput::flags``;
  vector ``[0x59]`` for forward+right+jump+sprint). Edge decodes shift/sprint
  bits into ``Sneak``/``Sprint`` (``crates/mc-edge/src/decode.rs``).
- ``ServerboundClientCommand`` ``0x0C``: VarInt action, ``PerformRespawn=0``
  (``interaction.rs`` ``ClientCommandAction``).
- ``ServerboundClientInformation`` (Play) ``0x0E``: UTF(16) language, i8 view
  distance, VarInt ChatVisibility (Full 0/System 1/Hidden 2), bool colors,
  u8 model bits, VarInt HumanoidArm (Left 0/Right 1), bool filtering, bool
  listing, VarInt ParticleStatus (All 0/Decreased 1/Minimal 2)
  (``packets/mod.rs`` ``ClientInformation``; layout vector for ``ru_ru`` in
  ``play/tests/serverbound_and_slots.rs``).
- ``ServerboundSignUpdate`` ``0x3D``: i64 pos, bool is_front, four UTF(384)
  lines (``packets/play/chat_commands.rs``; ``SIGN_LINE_COUNT=4``,
  ``MAX_SIGN_LINE_LEN=384`` in ``packets/play.rs``).
- ``ServerboundEditBook`` ``24`` (``0x18``): VarInt slot, VarInt page count,
  pages UTF(1024) up to 100, bool has_title, optional UTF(32) title
  (``chat_commands.rs``; vector ``28 01 03 one 01 05 title`` for slot 40 in
  ``play/book_packet_tests.rs``).
- ``ServerboundClientTickEnd`` ``0x0D``: empty body (``chat_commands.rs``).
- Clientbound ``SetTime`` ``0x71`` carries i64 ``game_time``
  (``packets/play/env_state.rs``); the observer exposes it as
  ``observer.game_time`` and ``wait_ticks`` advances on it event-driven.

Requires on the host: ``self.observer`` (``wire.PlayObserver`` with
``send``/``sequence``/``game_time``), ``self.timeout_seconds``,
``self.wait_until`` (zero-arg predicate).
"""
from __future__ import annotations

import math
import struct

from . import wire
from .matrix_api import MissingCapability

PLAYER_ACTION_ID = 0x29
USE_ITEM_ID = 0x43
INTERACT_ID = 0x1A
MOVE_PLAYER_POS_ID = 0x1E
PLAYER_INPUT_ID = 0x2B
CLIENT_COMMAND_ID = 0x0C
CLIENT_INFORMATION_ID = 0x0E
SIGN_UPDATE_ID = 0x3D
EDIT_BOOK_ID = 24
CLIENT_TICK_END_ID = 0x0D

_FACES = {'down': 0, 'up': 1, 'north': 2, 'south': 3, 'west': 4, 'east': 5}
_HANDS = {'main': 0, 'off': 1}
_CHAT_VISIBILITY = {'full': 0, 'system': 1, 'hidden': 2}

_LP_ZERO_EPSILON = 3.051944088384301E-5
_LP_CLAMP = 1.7179869183E10


_XZ_MIN, _XZ_MAX = -(1 << 25), (1 << 25) - 1
_Y_MIN, _Y_MAX = -(1 << 11), (1 << 11) - 1
_MAX_SIGN_LINES = 4
_MAX_SIGN_LINE_UNITS = 384
_MAX_BOOK_PAGES = 100
_MAX_BOOK_PAGE_UNITS = 1024
_MAX_BOOK_TITLE_UNITS = 32
_MAX_LOCALE_UNITS = 16
_MIN_VIEW_DISTANCE, _MAX_VIEW_DISTANCE = 2, 32

# Vanilla client defaults for the ClientInformation fields this adapter does
# not manage; kept per client so repeated updates preserve prior values.
_CLIENT_INFO_DEFAULTS = {
    'locale': 'en_us',
    'view_distance': 12,
    'chat_visibility': 0,
    'chat_colors': True,
    'model_bits': 0x7F,
    'main_hand': 1,
    'text_filtering': False,
    'allows_listing': True,
    'particle_status': 0,
}


def _utf16_units(value: str) -> int:
    return len(value.encode('utf-16-le')) // 2


def _require_text(value: str, name: str, max_units: int) -> str:
    if not isinstance(value, str):
        raise ValueError(name + ' must be a string')
    if _utf16_units(value) > max_units:
        raise ValueError(name + ' exceeds ' + str(max_units) + ' UTF-16 units')
    return value


def _parse_hand(hand: str) -> int:
    if hand in _HANDS:
        return _HANDS[hand]
    raise ValueError("unknown hand " + repr(hand) + '; expected "main"/"off"')


def _parse_face(face: str) -> int:
    if face not in _FACES:
        raise ValueError('unknown block face ' + repr(face))
    return _FACES[face]


def _clamp_lp(value: float) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError('interact location must be finite numbers')
    if not math.isfinite(value):
        raise ValueError('interact location must be finite numbers')
    return max(-_LP_CLAMP, min(_LP_CLAMP, float(value)))


def _pack_lp_component(value: float) -> int:
    return int(math.floor((value * 0.5 + 0.5) * 32766.0 + 0.5))


def _encode_lp_vec3(x: float, y: float, z: float) -> bytes:
    cx, cy, cz = _clamp_lp(x), _clamp_lp(y), _clamp_lp(z)
    if max(abs(cx), abs(cy), abs(cz)) < _LP_ZERO_EPSILON:
        return b'\x00'
    scale = int(math.ceil(max(abs(cx), abs(cy), abs(cz))))
    cont = (scale & 3) != scale
    bits = (scale & 3) | 4 if cont else scale
    packed = (bits | (_pack_lp_component(cx / scale) << 3)
              | (_pack_lp_component(cy / scale) << 18)
              | (_pack_lp_component(cz / scale) << 33))
    body = struct.pack('<Q', packed)[0:6]
    if cont:
        body += wire.write_varint(scale >> 2)
    return body


def _parse_block_position(position) -> tuple:
    coords = tuple(position) if isinstance(position, (tuple, list)) else None
    if coords is None or len(coords) != 3 or any(isinstance(v, bool) or not isinstance(v, int) for v in coords):
        raise ValueError('position must be three ints (x, y, z)')
    x, y, z = coords
    if not (_XZ_MIN <= x <= _XZ_MAX and _XZ_MIN <= z <= _XZ_MAX):
        raise ValueError('x/z outside signed 26-bit block range')
    if not (_Y_MIN <= y <= _Y_MAX):
        raise ValueError('y outside signed 12-bit block range')
    return (x, y, z)


def _pack_pos_i64(position: tuple) -> bytes:
    raw = int.from_bytes(wire.pack_block_position(position), 'big', signed=False)
    return struct.pack('>q', raw - (1 << 64) if raw >= (1 << 63) else raw)

def _player_action_body(action: int, position: tuple, face: str, sequence: int) -> bytes:
    return (wire.write_varint(action) + wire.pack_block_position(position)
            + wire.write_varint(_parse_face(face)) + wire.write_varint(sequence))


class ProtocolInputVerbs:
    """Player-input verbs; mixes into ``ProtocolMatrixClient``."""

    def _client_info(self) -> dict:
        info = getattr(self, '_protocol_input_client_info', None)
        if info is None:
            info = dict(_CLIENT_INFO_DEFAULTS)
            self._protocol_input_client_info = info
        return info

    def break_block(self, position, face='up', phase='complete',
                    expected_drop_item_id=None, expected_drop_count=None) -> int:
        """Send mining phases for a block; drops are observed separately.

        ``phase`` ``start`` sends only ``StartDestroyBlock``, ``abort`` only
        ``AbortDestroyBlock``, ``stop`` only ``StopDestroyBlock``, and
        ``complete`` start then stop. The plain-stack observer cannot
        prove item drops, so any ``expected_drop_*`` assertion is rejected
        instead of faked.
        Returns the last submitted sequence; BlockChangedAck can fence admission
        of that sequence, but cannot prove mining completion or accepted damage.
        """
        if phase not in ('start', 'abort', 'stop', 'complete'):
            raise ValueError('unknown break phase ' + repr(phase))
        if expected_drop_item_id is not None or expected_drop_count is not None:
            raise MissingCapability('oracle:block_drop')
        block = _parse_block_position(position)
        _parse_face(face)
        actions = {'start': (0,), 'abort': (2,), 'stop': (3,), 'complete': (0, 3)}[phase]
        for action in actions:
            self.observer.sequence += 1
            self.observer.send(PLAYER_ACTION_ID,
                               _player_action_body(action, block, face, self.observer.sequence))
            self.observer.send(CLIENT_TICK_END_ID)
        return self.observer.sequence

    def use_item(self, hand='main', ticks=1) -> None:
        """Hold ``ServerboundUseItem`` until the observed tick boundary.

        Sends the packet once, then waits ``ticks`` observed ``game_time``
        advances (``SetTime``-driven, no sleeps) before returning so the
        hold spans real server ticks.
        """
        hand_id = _parse_hand(hand)
        if isinstance(ticks, bool) or not isinstance(ticks, int) or ticks < 1:
            raise ValueError('ticks must be an int >= 1')
        self.observer.sequence += 1
        self.observer.send(USE_ITEM_ID, wire.write_varint(hand_id)
                           + wire.write_varint(self.observer.sequence)
                           + struct.pack('>ff', *getattr(self, 'rotation', (0.0, 0.0))))
        self.observer.send(CLIENT_TICK_END_ID)
        self.wait_ticks(ticks)

    def release_use_item(self) -> None:
        """Send ``ReleaseUseItem`` (bow release / interrupted eating)."""
        self.observer.sequence += 1
        self.observer.send(PLAYER_ACTION_ID,
                           _player_action_body(6, (0, 0, 0), 'down', self.observer.sequence))
        self.observer.send(CLIENT_TICK_END_ID)

    def swap_hands(self) -> None:
        """Send ``SwapItemWithOffhand``."""
        self.observer.sequence += 1
        self.observer.send(PLAYER_ACTION_ID,
                           _player_action_body(7, (0, 0, 0), 'down', self.observer.sequence))
        self.observer.send(CLIENT_TICK_END_ID)

    def respawn(self) -> None:
        """Send ``PerformRespawn``; revival itself is observed separately."""
        self.observer.send(CLIENT_COMMAND_ID, wire.write_varint(0))
        self.observer.send(CLIENT_TICK_END_ID)

    def interact_entity(self, entity_id: int, hand='main', secondary=False,
                        location: tuple = None) -> None:
        """Send ``ServerboundInteract`` (right-click an entity)."""
        if isinstance(entity_id, bool) or not isinstance(entity_id, int) or entity_id < 0:
            raise ValueError('entity_id must be a non-negative int')
        hand_id = _parse_hand(hand)
        if location is None:
            encoded = b'\x00'
        else:
            coords = tuple(location) if isinstance(location, (tuple, list)) else None
            if coords is None or len(coords) != 3 or any(
                    isinstance(v, bool) or not isinstance(v, (int, float)) for v in coords):
                raise ValueError('location must be three numbers (x, y, z)')
            encoded = _encode_lp_vec3(float(coords[0]), float(coords[1]), float(coords[2]))
        self.observer.send(INTERACT_ID, wire.write_varint(entity_id)
                           + wire.write_varint(hand_id) + encoded
                           + (b'\x01' if secondary else b'\x00'))
        self.observer.send(CLIENT_TICK_END_ID)

    def move_to(self, x, y, z, on_ground=True) -> None:
        """Send an absolute ``MovePlayerPos``; arrival is observed separately."""
        for value in (x, y, z):
            if not isinstance(value, (int, float)) or not math.isfinite(value):
                raise ValueError('move_to coordinates must be finite numbers')
        self.observer.send(MOVE_PLAYER_POS_ID,
                           struct.pack('>dddB', float(x), float(y), float(z),
                                       0x01 if on_ground else 0x00))
        self.observer.send(CLIENT_TICK_END_ID)

    def press_inputs(self, *, forward=False, backward=False, left=False, right=False,
                     jump=False, sneak=False, sprint=False, ticks: int = 1) -> None:
        """Hold real PlayerInput flags across observed server ticks.

        These flags are not client physics and cannot by themselves prove
        movement; adapters must inspect the requested resulting state.
        """
        if isinstance(ticks, bool) or not isinstance(ticks, int) or ticks < 1:
            raise ValueError('ticks must be an int >= 1')
        flags = ((0x01 if forward else 0) | (0x02 if backward else 0)
                 | (0x04 if left else 0) | (0x08 if right else 0)
                 | (0x10 if jump else 0) | (0x20 if sneak else 0)
                 | (0x40 if sprint else 0))
        body = bytes([flags])
        self.observer.send(PLAYER_INPUT_ID, body)
        self.observer.send(CLIENT_TICK_END_ID)
        self.wait_ticks(ticks)

    def jump(self, ticks=1) -> None:
        """Hold the jump input flag across observed ticks; not fake motion."""
        self.press_inputs(jump=True, ticks=ticks)

    def sneak(self, enabled) -> None:
        """Pulse the shift input flag (single-flag packet; see press_inputs)."""
        self.press_inputs(sneak=bool(enabled), ticks=1)

    def sprint(self, enabled) -> None:
        """Pulse the sprint input flag via ``0x2B``.

        Uses the per-tick input-flags packet, not ``PlayerCommand``
        start/stop sprinting, which needs the entity id the observer does
        not track.
        """
        self.press_inputs(sprint=bool(enabled), ticks=1)

    def wait_ticks(self, ticks: int):
        """Event-driven wait for ``ticks`` observed ``game_time`` advances.

        Uses ``SetTime``-driven ``observer.game_time`` via ``wait_until``;
        never sleeps. Fails (timeout) rather than completing unfounded when
        time does not advance.
        """
        if isinstance(ticks, bool) or not isinstance(ticks, int) or ticks < 0:
            raise ValueError('ticks must be an int >= 0')
        if ticks == 0:
            return self.observer.game_time
        self.wait_until(lambda: self.observer.game_time is not None,
                        description='received SetTime',
                        timeout_seconds=self.timeout_seconds)
        target = self.observer.game_time + ticks
        self.wait_until(lambda: self.observer.game_time is not None
                        and self.observer.game_time >= target,
                        description='observed game_time advanced ' + str(ticks) + ' ticks',
                        timeout_seconds=self.timeout_seconds)
        return self.observer.game_time

    def move(self, forward: float = 0, strafe: float = 0, ticks: int = 20, sprint=False) -> None:
        """Local-physics movement is unsupported; only real input flags exist.

        Position stepping here would be teleport fabrication, so this is an
        explicit capability failure, never a no-op. Use ``press_inputs`` for
        real key flags plus observed displacement, or ``move_to`` for an
        explicit absolute position packet.
        """
        raise MissingCapability('client:move_physics')

    def update_sign(self, position, lines: list, front=True) -> None:
        """Send ``ServerboundSignUpdate`` with exactly four lines."""
        block = _parse_block_position(position)
        if not isinstance(lines, (tuple, list)) or len(lines) != _MAX_SIGN_LINES:
            raise ValueError('sign update needs exactly four lines')
        checked = [_require_text(line, 'sign line', _MAX_SIGN_LINE_UNITS) for line in lines]
        self.observer.send(SIGN_UPDATE_ID, _pack_pos_i64(block)
            + (b'\x01' if front else b'\x00')
            + b''.join(wire.mc_string(line) for line in checked))
        self.observer.send(CLIENT_TICK_END_ID)

    def edit_book(self, slot, pages, title=None) -> None:
        """Send ``ServerboundEditBook`` for the book in ``slot``."""
        if isinstance(slot, bool) or not isinstance(slot, int) or slot < 0:
            raise ValueError('book slot must be a non-negative int')
        page_list = list(pages) if isinstance(pages, (tuple, list)) else None
        if page_list is None or len(page_list) > _MAX_BOOK_PAGES:
            raise ValueError('book needs at most 100 pages')
        checked = [_require_text(page, 'book page', _MAX_BOOK_PAGE_UNITS) for page in page_list]
        if title is not None:
            _require_text(title, 'book title', _MAX_BOOK_TITLE_UNITS)
        body = wire.write_varint(slot) + wire.write_varint(len(checked))
        for page in checked:
            body += wire.mc_string(page)
        if title is not None:
            body += b'\x01' + wire.mc_string(title)
        else:
            body += b'\x00'
        self.observer.send(EDIT_BOOK_ID, body)
        self.observer.send(CLIENT_TICK_END_ID)

    def set_client_information(self, view_distance=None, locale=None) -> None:
        """Send full Play ``ClientInformation`` with stored client settings."""
        if view_distance is None and locale is None:
            raise ValueError('nothing to update: pass view_distance and/or locale')
        info = self._client_info()
        if view_distance is not None:
            if (isinstance(view_distance, bool) or not isinstance(view_distance, int)
                    or not (_MIN_VIEW_DISTANCE <= view_distance <= _MAX_VIEW_DISTANCE)):
                raise ValueError('view_distance must be an int in 2..32')
            info['view_distance'] = view_distance
        if locale is not None:
            info['locale'] = _require_text(locale, 'locale', _MAX_LOCALE_UNITS)
        self.observer.send(CLIENT_INFORMATION_ID, _encode_client_information(info))
        self.observer.send(CLIENT_TICK_END_ID)

    def configure_chat(self, visibility) -> None:
        """Send Play ``ClientInformation`` with a new chat visibility."""
        if not isinstance(visibility, str) or visibility.lower() not in _CHAT_VISIBILITY:
            raise ValueError('unknown chat visibility ' + repr(visibility)
                             + '; expected full/system/hidden')
        info = self._client_info()
        info['chat_visibility'] = _CHAT_VISIBILITY[visibility.lower()]
        self.observer.send(CLIENT_INFORMATION_ID, _encode_client_information(info))
        self.observer.send(CLIENT_TICK_END_ID)


def _encode_client_information(info: dict) -> bytes:
    return (wire.mc_string(info['locale']) + struct.pack('b', info['view_distance'])
            + wire.write_varint(info['chat_visibility'])
            + (b'\x01' if info['chat_colors'] else b'\x00')
            + bytes([info['model_bits']]) + wire.write_varint(info['main_hand'])
            + (b'\x01' if info['text_filtering'] else b'\x00')
            + (b'\x01' if info['allows_listing'] else b'\x00')
            + wire.write_varint(info['particle_status']))
