"""Bounded clientbound entity-state decoder for the protocol matrix.

Pure decoder/state accumulator owned by the entity-matrix slice. The parent
(`tools/harness/protocol_matrix_client.py`) instantiates exactly one
`ProtocolEntities` per observer, feeds it every received Play packet while
holding the observer condition, and merges `snapshot()` into the matrix
result. Entity *type* ids stay numeric here; the parent maps them to names
through the vanilla `reports/registries.json` (`minecraft:entity_type`
`protocol_id`), so projectiles (arrow, fireball, ...) are derived by the
parent from the type name, never guessed here.

Packet IDs and layouts are source-verified against `crates/mc-protocol`
(26.1.2, ADR 0002: local `.analysis/protocol-dump.txt` plus `javap` of the
vanilla server jar):

- 0x01 AddEntity: `packets/play/entity.rs` `impl Packet for AddEntity`
  (game-CB index 1). VarInt id, 16-byte UUID, VarInt entity-type registry
  id, f64 x/y/z, `Vec3.LP_STREAM_CODEC` movement, packed pitch/yaw/head-yaw
  bytes, VarInt data.
- 0x23 EntityPositionSync: `entity.rs` (game-CB index 35). VarInt id plus
  `PositionMoveRotation` (f64 xyz, f64 per-tick velocity xyz, f32 yaw, f32
  pitch) plus bool onGround. This is the absolute move/teleport the edge
  planner emits when a relative delta does not fit (`mc-edge` `plan.rs`
  `entity_movement` -> `MovePacket::Sync`); no dedicated TeleportEntity
  packet exists in 26.1.2 `mc-protocol` or in the edge planner output.
- 0x35 MoveEntityPos: `entity.rs` (game-CB index 53). VarInt id, three i16
  relative deltas in block units times 4096, bool onGround.
- 0x36 MoveEntityPosRot: `entity.rs` (game-CB index 54). As 0x35 plus packed
  yaw/pitch bytes.
- 0x38 MoveEntityRot: `entity.rs` (game-CB index 56, after the inserted
  move-along-track packet). VarInt id, packed yaw/pitch bytes, bool;
  position untouched.
- 0x4D RemoveEntities: `entity.rs` (game-CB index 77). VarInt count plus
  VarInt ids (`FriendlyByteBuf.writeIntIdList`).
- 0x52 ClientboundRespawn: `packets/play/login_flow.rs` (game-CB index 82).
  Dimension switch; the observer already clears chunk residency here
  (`wire.py` `PlayObserver`), so tracked-entity state clears too. The body
  is intentionally not decoded: only the dimension-switch signal is folded.
- 0x63 ClientboundSetEntityData: `entity.rs` (game-CB index 99). VarInt id,
  repeated (u8 index, VarInt serializer id, payload) entries, 0xFF
  terminator; index 0xFF is reserved. At most 64 values
  (`play.rs` `MAX_ENTITY_DATA_VALUES`).
- 0x65 SetEntityMotion: `entity.rs` (game-CB index 101). VarInt id plus
  `Vec3.LP_STREAM_CODEC` velocity; validated only, velocity is not part of
  the snapshot contract.
- 0x67 ClientboundSetExperience: `packets/play/env_state.rs` (game-CB index
  103). Wire order is f32 progress, VarInt level, VarInt total (struct
  field order differs; the encode impl is authoritative).
- 0x4E RemoveEntityEffect: `packets/play/entity_sync_26_1_2.rs` (game-CB
  index 78). VarInt entity id plus VarInt mob-effect id.
- 0x84 UpdateEntityEffect: `entity_sync_26_1_2.rs` (game-CB index 132).
  VarInt entity/effect/amplifier/duration plus u8 flags (bits 0..3:
  ambient, visible, show_icon, blend).
- 0x7C TakeItemEntity: `entity.rs` (game-CB index 124). Three VarInts;
  validated, no state change (the actual removal arrives via 0x4D).

Metadata serializers (`play.rs` constants, `entity.rs` `EntityDataValue`):
0 Byte, 1 Int, 2 Long, 3 Float, 4 String, 5 Component, 6 OptionalComponent,
7 ItemStack, 8 Boolean, 9 Rotations, 10 BlockPosition, 11
OptionalBlockPosition, 12 Direction, 13 OptionalLivingEntityReference, 14
BlockState, 15 OptionalBlockState, 16 Particle, 17 Particles, 18
VillagerData, 19 OptionalUnsignedInt, 20 Pose, 37/38 copper golem, 42
humanoid arm. Indices with folded meaning: item 8
(`ITEM_ENTITY_DATA_ITEM_INDEX`), living health Float 9
(`LIVING_ENTITY_DATA_HEALTH_INDEX_26_1_2`), experience-orb value Int 8
(`plan.rs` `push_appearance` experience_orb arm). Particle option
`minecraft:entity_effect` is protocol id 21 with one i32 ARGB; trial/raid
omen (113/114) carry no payload.

Semantics:

- `consume()` returns True when the packet id is handled and the body
  fully decodes with exact length consumption. Anything else returns False
  and records `unavailable`; no exception ever escapes `consume`, so an
  unknown payload can never crash the parent reader loop.
- An unknown metadata *serializer* fails that one packet closed: the whole
  update is dropped, the affected entity (if tracked) is marked partial,
  and `unavailable` names the serializer. A known serializer at an
  uninteresting index is skipped by length (meaning unobserved, wire
  intact) without poisoning the observation.
- ItemStack metadata (serializer 7) needs the injected `decode_stack`
  callback (`tools/harness/protocol_observations.py`
  `decode_stack(data, offset) -> (stack | None, offset)`, stack a dict
  with numeric `item_id`/`count`/`components`/`removed`). Without it the packet
  fails closed as `item_stack_decoder_missing`.
- Motion/sync/rotation/effect packets for never-added ids are validated
  and ignored: entities are never invented. `RemoveEntities` drops
  whatever is tracked and ignores the rest.
- No sleeps, no threads, no I/O. The parent must call `consume` while
  holding the observer condition and treat the instance as parent-owned.

Parent integration (in `PlayObserver._read`, inside the existing
`with self.condition:` block, next to the `frames` bookkeeping)::

    from .protocol_entities import HANDLED_IDS, ProtocolEntities
    self.entities = ProtocolEntities(decode_stack=None)  # or the
        # observations helper once it lands
    ...
    if packet_id in HANDLED_IDS:
        self.entities.consume(packet_id, body)

Merge into `ProtocolMatrixClient.snapshot()`::

    result["entities"] = self.observer.entities.snapshot()["entities"]

then map each entry's numeric `type` through the vanilla entity-type
registry (`reports/registries.json`) to names; derive projectile
behaviour parent-side from the type name.

Known limitations (kept minimal on purpose):

- Snapshot carries id, uuid, type, position plus observed-only item,
  health, xp_value and effects. Yaw/pitch/velocity are validated and
  dropped; rotation-only packets change nothing.
- Custom names, absorption, pose, equipment, passengers and leashes use
  known serializers and are skipped by length, never surfaced.
- Item stacks require the injected callback; without it those metadata
  packets fail closed instead of guessing.
- `unavailable` remains sticky until respawn: an unrelated successful packet
  cannot reconstruct previously lost entities or metadata. Per-entity `partial`
  remains until re-add or respawn for the same reason.
"""

from __future__ import annotations

import copy
import struct
import uuid
from typing import Any, Callable

ADD_ENTITY = 0x01
ENTITY_POSITION_SYNC = 0x23
MOVE_ENTITY_POS = 0x35
MOVE_ENTITY_POS_ROT = 0x36
MOVE_ENTITY_ROT = 0x38
REMOVE_ENTITIES = 0x4D
RESPAWN = 0x52
SET_ENTITY_DATA = 0x63
SET_ENTITY_MOTION = 0x65
SET_EXPERIENCE = 0x67
TAKE_ITEM_ENTITY = 0x7C
REMOVE_ENTITY_EFFECT = 0x4E
UPDATE_ENTITY_EFFECT = 0x84

HANDLED_IDS = frozenset((
    ADD_ENTITY,
    ENTITY_POSITION_SYNC,
    MOVE_ENTITY_POS,
    MOVE_ENTITY_POS_ROT,
    MOVE_ENTITY_ROT,
    REMOVE_ENTITIES,
    RESPAWN,
    SET_ENTITY_DATA,
    SET_ENTITY_MOTION,
    SET_EXPERIENCE,
    TAKE_ITEM_ENTITY,
    REMOVE_ENTITY_EFFECT,
    UPDATE_ENTITY_EFFECT,
))

_SER_ITEM_STACK = 7
_SER_PARTICLE = 16
_SER_PARTICLES = 17

_HEALTH_INDEX = 9
_ORB_INDEX = 8
_ITEM_INDEX = 8

_MAX_METADATA_VALUES = 64
_MAX_EFFECT_PARTICLES = 256
_NBT_MAX_DEPTH = 64

_PARTICLE_ENTITY_EFFECT = 21
_PARTICLE_TRIAL_OMEN = 113
_PARTICLE_RAID_OMEN = 114

# decode_stack(data, offset) -> (stack | None, new_offset), using the canonical
# protocol_observations numeric item_id/count/components/removed shape.
DecodeStack = Callable[[bytes, int], tuple[Any, int]]


class _DecodeError(Exception):
    """Internal: malformed, truncated, over-long, or unknown payload."""


class _Cursor:
    """Bounded big-endian reader over one packet body."""

    __slots__ = ("data", "off")

    def __init__(self, data: bytes) -> None:
        self.data = data
        self.off = 0

    def remaining(self) -> int:
        return len(self.data) - self.off

    def end(self) -> None:
        if self.remaining() != 0:
            raise _DecodeError("trailing_bytes")

    def take(self, count: int) -> bytes:
        if count < 0 or self.remaining() < count:
            raise _DecodeError("truncated")
        start = self.off
        self.off += count
        return self.data[start:self.off]

    def u8(self) -> int:
        return self.take(1)[0]

    def u16(self) -> int:
        return struct.unpack_from(">H", self.take(2))[0]

    def i16(self) -> int:
        return struct.unpack_from(">h", self.take(2))[0]

    def i32(self) -> int:
        return struct.unpack_from(">i", self.take(4))[0]

    def f32(self) -> float:
        return struct.unpack_from(">f", self.take(4))[0]

    def f64(self) -> float:
        return struct.unpack_from(">d", self.take(8))[0]

    def boolean(self) -> bool:
        return self.u8() != 0

    def varint(self) -> int:
        value = 0
        for shift in range(0, 35, 7):
            raw = self.u8()
            if shift == 28 and raw > 15:
                raise _DecodeError("overflow_varint")
            value |= (raw & 0x7F) << shift
            if raw & 0x80 == 0:
                if value & 0x80000000:
                    value -= 0x100000000
                return value
        raise _DecodeError("overlong_varint")


def _skip_lp_vec3(cur: _Cursor) -> None:
    """Consume one `Vec3.LP_STREAM_CODEC` value (`play.rs` write/read)."""
    first = cur.u8()
    if first == 0:
        return
    cur.take(1 + 4)
    if first & 0x04:
        cur.varint()


def _skip_nbt_payload(cur: _Cursor, tag: int, depth: int) -> None:
    if tag == 0:
        return
    if tag == 1:
        cur.take(1)
        return
    if tag == 2:
        cur.take(2)
        return
    if tag in (3, 5):
        cur.take(4)
        return
    if tag in (4, 6):
        cur.take(8)
        return
    if tag == 7:
        length = cur.i32()
        if length < 0 or length > cur.remaining():
            raise _DecodeError("bad_nbt_array")
        cur.take(length)
        return
    if tag == 8:
        cur.take(cur.u16())
        return
    if tag == 9:
        element = cur.u8()
        length = cur.i32()
        if length < 0 or (element == 0 and length != 0) or length > cur.remaining():
            raise _DecodeError("bad_nbt_list")
        for _ in range(length):
            if depth + 1 > _NBT_MAX_DEPTH:
                raise _DecodeError("nbt_too_deep")
            _skip_nbt_payload(cur, element, depth + 1)
        return
    if tag == 10:
        while True:
            nested = cur.u8()
            if nested == 0:
                return
            if depth + 1 > _NBT_MAX_DEPTH:
                raise _DecodeError("nbt_too_deep")
            cur.take(cur.u16())
            _skip_nbt_payload(cur, nested, depth + 1)
    if tag == 11:
        length = cur.i32()
        if length < 0 or length > cur.remaining() // 4:
            raise _DecodeError("bad_nbt_array")
        cur.take(4 * length)
        return
    if tag == 12:
        length = cur.i32()
        if length < 0 or length > cur.remaining() // 8:
            raise _DecodeError("bad_nbt_array")
        cur.take(8 * length)
        return
    raise _DecodeError("bad_nbt_tag")


def _skip_nbt_value(cur: _Cursor) -> None:
    _skip_nbt_payload(cur, cur.u8(), 0)


def _skip_effect_particle(cur: _Cursor) -> None:
    option = cur.varint()
    if option == _PARTICLE_ENTITY_EFFECT:
        cur.take(4)
    elif option in (_PARTICLE_TRIAL_OMEN, _PARTICLE_RAID_OMEN):
        return
    else:
        raise _DecodeError("unknown_effect_particle:%d" % option)


def _skip_metadata_value(cur: _Cursor, serializer: int) -> None:
    """Consume one metadata payload whose meaning is not snapshotted."""
    if serializer == 0:
        cur.take(1)
    elif serializer == 1:
        cur.varint()
    elif serializer == 2:
        cur.take(8)
    elif serializer == 3:
        cur.take(4)
    elif serializer == 4:
        length = cur.varint()
        if length < 0 or length > cur.remaining():
            raise _DecodeError("bad_string")
        cur.take(length)
    elif serializer == 5:
        _skip_nbt_value(cur)
    elif serializer == 6:
        if cur.boolean():
            _skip_nbt_value(cur)
    elif serializer == 8:
        cur.take(1)
    elif serializer == 9:
        cur.take(12)
    elif serializer == 10:
        cur.take(8)
    elif serializer == 11:
        if cur.boolean():
            cur.take(8)
    elif serializer == 12:
        cur.varint()
    elif serializer == 13:
        if cur.boolean():
            cur.take(16)
    elif serializer in (14, 15):
        cur.varint()
    elif serializer == _SER_PARTICLE:
        _skip_effect_particle(cur)
    elif serializer == _SER_PARTICLES:
        count = cur.varint()
        if count < 0 or count > _MAX_EFFECT_PARTICLES:
            raise _DecodeError("bad_particle_count")
        for _ in range(count):
            _skip_effect_particle(cur)
    elif serializer == 18:
        cur.varint()
        cur.varint()
        cur.varint()
    elif serializer in (19, 20, 37, 38, 42):
        cur.varint()
    else:
        raise _DecodeError("unknown_metadata_serializer:%d" % serializer)


class ProtocolEntities:
    """Pure clientbound entity-state accumulator; parent-owned, no I/O."""

    def __init__(self, decode_stack: DecodeStack | None = None, *,
                 local_entity_id: int | None = None) -> None:
        self._decode_stack = decode_stack
        self._entities: dict[int, dict[str, Any]] = {}
        self._xp: dict[str, Any] | None = None
        self._local = {'id': local_entity_id} if local_entity_id is not None else None
        self.unavailable: str | None = None

    def consume(self, packet_id: int, body: bytes) -> bool:
        """Fold one Play packet body; True only when fully decoded.

        Never raises: malformed, truncated, over-long, or unknown payloads
        return False and record `unavailable`. Unknown packet ids return
        False without touching `unavailable`.
        """
        try:
            raw = bytes(body)
            if packet_id == ADD_ENTITY:
                self._add(raw)
            elif packet_id == ENTITY_POSITION_SYNC:
                self._absolute(raw)
            elif packet_id == MOVE_ENTITY_POS:
                self._relative(raw, with_rotation=False)
            elif packet_id == MOVE_ENTITY_POS_ROT:
                self._relative(raw, with_rotation=True)
            elif packet_id == MOVE_ENTITY_ROT:
                self._rotation(raw)
            elif packet_id == REMOVE_ENTITIES:
                self._remove(raw)
            elif packet_id == RESPAWN:
                self._entities.clear()
                self.unavailable = None
                if self._local is not None:
                    self._local = {'id': self._local['id']}
            elif packet_id == SET_ENTITY_DATA:
                self._metadata(raw)
            elif packet_id == SET_ENTITY_MOTION:
                self._motion(raw)
            elif packet_id == SET_EXPERIENCE:
                self._experience(raw)
            elif packet_id == TAKE_ITEM_ENTITY:
                self._take_item(raw)
            elif packet_id == REMOVE_ENTITY_EFFECT:
                self._remove_effect(raw)
            elif packet_id == UPDATE_ENTITY_EFFECT:
                self._update_effect(raw)
            else:
                return False
        except _DecodeError as error:
            self.unavailable = str(error) or "decode_failed"
            return False
        except Exception as error:  # observer must never die on wire input
            self.unavailable = "internal_%s" % type(error).__name__
            return False
        return True

    def snapshot(self) -> dict[str, Any]:
        """JSON-ready entity array plus observation health."""
        entities = []
        for entity_id in sorted(self._entities):
            entry = self._entities[entity_id]
            item: dict[str, Any] = {
                "id": entry["id"],
                "uuid": entry["uuid"],
                "type": entry["type"],
                "position": [float(value) for value in entry["position"]],
                "spawn_position": list(entry["spawn_position"]),
            }
            for key in ("item", "health", "xp_value", "effects"):
                if key in entry:
                    item[key] = copy.deepcopy(entry[key])
            if entry.get("partial"):
                item["partial"] = True
            entities.append(item)
        return {
            "entities": entities,
            "partial": any(entry.get("partial") for entry in self._entities.values()),
            "unavailable": self.unavailable,
            "xp": copy.deepcopy(self._xp),
            "local": copy.deepcopy(self._local),
        }

    def _add(self, raw: bytes) -> None:
        cur = _Cursor(raw)
        entity_id = cur.varint()
        entity_uuid = str(uuid.UUID(bytes=cur.take(16)))
        entity_type = cur.varint()
        position = [cur.f64(), cur.f64(), cur.f64()]
        _skip_lp_vec3(cur)
        cur.take(3)  # packed pitch, yaw, head yaw; orientation not snapshotted
        cur.varint()  # spawn data; not snapshotted
        cur.end()
        self._entities[entity_id] = {
            "id": entity_id,
            "uuid": entity_uuid,
            "type": entity_type,
            "position": position,
            "spawn_position": list(position),
        }

    def _relative(self, raw: bytes, *, with_rotation: bool) -> None:
        cur = _Cursor(raw)
        entity_id = cur.varint()
        delta = [cur.i16() / 4096.0, cur.i16() / 4096.0, cur.i16() / 4096.0]
        if with_rotation:
            cur.take(2)  # packed yaw/pitch; orientation not snapshotted
        cur.boolean()
        cur.end()
        entry = self._entities.get(entity_id)
        if entry is None:
            return
        entry["position"] = [
            old + step for old, step in zip(entry["position"], delta)
        ]

    def _rotation(self, raw: bytes) -> None:
        cur = _Cursor(raw)
        cur.varint()
        cur.take(2)
        cur.boolean()
        cur.end()

    def _absolute(self, raw: bytes) -> None:
        cur = _Cursor(raw)
        entity_id = cur.varint()
        position = [cur.f64(), cur.f64(), cur.f64()]
        cur.take(24)  # per-tick velocity; not snapshotted
        cur.take(8)  # yaw/pitch; not snapshotted
        cur.boolean()
        cur.end()
        entry = self._entities.get(entity_id)
        if entry is None:
            return
        entry["position"] = position

    def _remove(self, raw: bytes) -> None:
        cur = _Cursor(raw)
        count = cur.varint()
        if count < 0 or count > cur.remaining():
            raise _DecodeError("bad_remove_count")
        ids = [cur.varint() for _ in range(count)]
        cur.end()
        for entity_id in ids:
            self._entities.pop(entity_id, None)

    def _motion(self, raw: bytes) -> None:
        cur = _Cursor(raw)
        cur.varint()
        _skip_lp_vec3(cur)
        cur.end()

    def _experience(self, raw: bytes) -> None:
        cur = _Cursor(raw)
        progress = cur.f32()
        level = cur.varint()
        total = cur.varint()
        cur.end()
        self._xp = {"progress": progress, "level": level, "total": total}

    def _take_item(self, raw: bytes) -> None:
        cur = _Cursor(raw)
        cur.varint()
        cur.varint()
        cur.varint()
        cur.end()

    def _remove_effect(self, raw: bytes) -> None:
        cur = _Cursor(raw)
        entity_id = cur.varint()
        effect_id = cur.varint()
        cur.end()
        if effect_id < 0:
            raise _DecodeError("negative_mob_effect_id")
        entry = self._entities.get(entity_id)
        if self._local is not None and entity_id == self._local['id']:
            entry = self._local
        if entry is None:
            return
        effects = entry.setdefault("effects", {})
        effects.pop(str(effect_id), None)

    def _update_effect(self, raw: bytes) -> None:
        cur = _Cursor(raw)
        entity_id = cur.varint()
        effect_id = cur.varint()
        amplifier = cur.varint()
        duration = cur.varint()
        flags = cur.u8()
        cur.end()
        if effect_id < 0:
            raise _DecodeError("negative_mob_effect_id")
        if flags & ~0x0F:
            raise _DecodeError("unknown_entity_effect_flag_bits")
        entry = self._entities.get(entity_id)
        if self._local is not None and entity_id == self._local['id']:
            entry = self._local
        if entry is None:
            return
        entry.setdefault("effects", {})[str(effect_id)] = {
            "amplifier": amplifier,
            "duration_ticks": duration,
            "flags": {
                "ambient": bool(flags & 0x01),
                "visible": bool(flags & 0x02),
                "show_icon": bool(flags & 0x04),
                "blend": bool(flags & 0x08),
            },
        }

    def _metadata(self, raw: bytes) -> None:
        cur = _Cursor(raw)
        entity_id: int | None = None
        try:
            entity_id = cur.varint()
            updates = []
            seen = set()
            while True:
                index = cur.u8()
                if index == 0xFF:
                    break
                if len(updates) >= _MAX_METADATA_VALUES:
                    raise _DecodeError("too_many_metadata_values")
                if index in seen:
                    raise _DecodeError("duplicate_metadata_index")
                seen.add(index)
                serializer = cur.varint()
                updates.append(self._parse_metadata_value(cur, index, serializer))
            cur.end()
        except _DecodeError:
            if entity_id is not None:
                entry = self._entities.get(entity_id)
                if entry is not None:
                    entry["partial"] = True
            raise
        entry = self._entities.get(entity_id)
        if entry is None:
            return
        for kind, value in updates:
            if kind == "health":
                entry["health"] = value
            elif kind == "xp":
                entry["xp_value"] = value
            elif kind == "item":
                if value is None:
                    entry.pop("item", None)
                else:
                    entry["item"] = value

    def _parse_metadata_value(
        self, cur: _Cursor, index: int, serializer: int
    ) -> tuple[str, Any]:
        if serializer == _SER_ITEM_STACK:
            stack = self._decode_item_stack(cur)
            if index == _ITEM_INDEX:
                return ("item", stack)
            return ("skip", None)
        if serializer == 3 and index == _HEALTH_INDEX:
            return ("health", cur.f32())
        if serializer == 1 and index == _ORB_INDEX:
            return ("xp", cur.varint())
        _skip_metadata_value(cur, serializer)
        return ("skip", None)

    def _decode_item_stack(self, cur: _Cursor) -> dict[str, Any] | None:
        if self._decode_stack is None:
            raise _DecodeError("item_stack_decoder_missing")
        try:
            stack, new_off = self._decode_stack(cur.data, cur.off)
        except _DecodeError:
            raise
        except Exception as error:
            raise _DecodeError("item_stack_decoder_failed") from error
        if not isinstance(new_off, int) or new_off < cur.off:
            raise _DecodeError("item_stack_decoder_failed")
        if new_off > len(cur.data):
            raise _DecodeError("item_stack_decoder_failed")
        cur.off = new_off
        if stack is None:
            return None
        try:
            item_id = stack["item_id"]
            count = stack["count"]
        except (KeyError, TypeError) as error:
            raise _DecodeError("bad_item_stack_shape") from error
        if (
            isinstance(item_id, bool)
            or not isinstance(item_id, int)
            or isinstance(count, bool)
            or not isinstance(count, int)
        ):
            raise _DecodeError("bad_item_stack_shape")
        components = stack.get("components")
        removed = stack.get("removed")
        if not isinstance(components, dict) or not isinstance(removed, list):
            raise _DecodeError("bad_item_stack_shape")
        return {
            "item_id": item_id,
            "count": count,
            "components": copy.deepcopy(components),
            "removed": list(removed),
        }
