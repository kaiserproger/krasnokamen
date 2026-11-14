"""Bounded public-wire observation decoders for component-bearing packets.

Layouts are re-implemented from local Rust sources only (no memory of live
traffic, no guessed shapes):

* ``ItemStack`` stream codec: ``crates/mc-protocol/src/packets/play.rs``
  (``ItemStackWireCodec``), ``copied_components.rs``, ``station_components.rs``.
* Menu image: ``inventory.rs`` ``ClientboundContainerSetContent`` (game id ``0x12``):
  VarInt window, VarInt state, VarInt slot count, slots, carried item.
* Merchant offers: ``merchant.rs`` ``ClientboundMerchantOffers`` (game id ``0x34``):
  VarInt container, VarInt offer count, offers, VarInt level, VarInt xp,
  two bools. ``MerchantItemCost`` supports the empty exact-component
  predicate only, mirroring the Rust decoder.
* Chat: ``chat_commands.rs`` ``ClientboundSystemChat`` (game id ``0x79``):
  trusted Component NBT bytes followed by one overlay bool.
* Network NBT value shape (tag byte, big-endian payloads, modified UTF-8
  strings): ``crates/mc-nbt/src/lib.rs`` ``read_network_value``.
* Component id table: ``crates/mc-data/src/data_component_types_26_1_2.rs``.

Supported component payloads (small exact subset; everything else fails
closed)::

    0   minecraft:custom_data        compound NBT root
    2   minecraft:max_damage         VarInt, 1..=i32::MAX
    3   minecraft:damage             VarInt, clamped at 0 like the Rust decoder
    6   minecraft:custom_name        text component (string, compound or list NBT)
    11  minecraft:lore               bounded list of text components
    13  minecraft:enchantments       bounded list of [registry id, level 1..=255]
    51  minecraft:potion_contents    has_potion, optional color, bounded effects,
                                    optional custom name

SUPPORTED_COMPONENT_IDS documents this subset. Any other *added* component id
raises :class:`UnsupportedObservation` naming the exact registry name (or the
numeric id when the id itself is outside the 110-entry table), so the caller
can never desync its cursor or mistake an unknown patch for an empty one.
Removed component ids are validated against the same table and captured
separately under the ``"removed"`` key (plain registry names; the Rust-side
``!`` marker is a durable-NBT internal and is not exposed here).

Normalized shapes:

* stack: ``{"item_id": int, "count": int, "components": {...}, "removed": [...]}``
  or ``None`` for the empty stack. ``item_id`` stays numeric.
* menu: ``{"window", "state_id", "slots", "cursor"}``.
* merchant: ``{"container_id", "offers", "level", "xp", "show_progress",
  "can_restock"}`` where each offer carries ``input1``, ``input2`` (or
  ``None``), ``output``, ``uses``, ``max_uses``, ``xp`` plus the remaining
  counters verbatim.
* chat: ``{"text", "translate", "with", "fallback", "overlay"}``. Literal
  components render to ``text``; ``translate`` components keep their key and
  structured args with ``text`` left empty -- localization is never
  fabricated.

Integration contract (for the observer owner; this module edits nothing):

* The parent maps numeric ``item_id`` / enchantment / potion ids to registry
  names; this module has no Python item registry and keeps them numeric.
* Call ``decode_menu`` on ``0x12`` bodies, ``decode_merchant_offers`` on
  merchant-offer bodies, ``decode_chat_component`` on ``0x79`` bodies, and
  ``decode_stack(data, offset)`` for single slots.
* ``UnsupportedObservation`` (shared identity with :mod:`wire`) means "valid
  framing, outside the supported subset": record it like the existing
  ``menu_errors`` path. ``TruncatedObservation`` means "bytes missing,
  overlong, malformed or trailing": fail the read like any other corrupt
  frame. After *any* error the cursor position is unspecified (mirroring the
  Rust all-or-something codec semantics); discard the packet body, never
  resume mid-packet.
"""

from __future__ import annotations

import math
import struct

from .wire import UnsupportedObservation

__all__ = [
    "COMPONENT_NAMES",
    "SUPPORTED_COMPONENT_IDS",
    "Cursor",
    "TruncatedObservation",
    "UnsupportedObservation",
    "decode_chat_component",
    "decode_menu",
    "decode_merchant_offers",
    "decode_registry",
    "decode_stack",
]


class TruncatedObservation(RuntimeError):
    """Packet bytes are missing, overlong, malformed, or have trailing bytes."""


# ---------------------------------------------------------------------------
# Component id table (exact copy of the 26.1.2 DataComponents registration
# order in crates/mc-data/src/data_component_types_26_1_2.rs).
# ---------------------------------------------------------------------------

_COMPONENT_TABLE: tuple[tuple[int, str], ...] = (
    (0, "minecraft:custom_data"),
    (1, "minecraft:max_stack_size"),
    (2, "minecraft:max_damage"),
    (3, "minecraft:damage"),
    (4, "minecraft:unbreakable"),
    (5, "minecraft:use_effects"),
    (6, "minecraft:custom_name"),
    (7, "minecraft:minimum_attack_charge"),
    (8, "minecraft:damage_type"),
    (9, "minecraft:item_name"),
    (10, "minecraft:item_model"),
    (11, "minecraft:lore"),
    (12, "minecraft:rarity"),
    (13, "minecraft:enchantments"),
    (14, "minecraft:can_place_on"),
    (15, "minecraft:can_break"),
    (16, "minecraft:attribute_modifiers"),
    (17, "minecraft:custom_model_data"),
    (18, "minecraft:tooltip_display"),
    (19, "minecraft:repair_cost"),
    (20, "minecraft:creative_slot_lock"),
    (21, "minecraft:enchantment_glint_override"),
    (22, "minecraft:intangible_projectile"),
    (23, "minecraft:food"),
    (24, "minecraft:consumable"),
    (25, "minecraft:use_remainder"),
    (26, "minecraft:use_cooldown"),
    (27, "minecraft:damage_resistant"),
    (28, "minecraft:tool"),
    (29, "minecraft:weapon"),
    (30, "minecraft:attack_range"),
    (31, "minecraft:enchantable"),
    (32, "minecraft:equippable"),
    (33, "minecraft:repairable"),
    (34, "minecraft:glider"),
    (35, "minecraft:tooltip_style"),
    (36, "minecraft:death_protection"),
    (37, "minecraft:blocks_attacks"),
    (38, "minecraft:piercing_weapon"),
    (39, "minecraft:kinetic_weapon"),
    (40, "minecraft:swing_animation"),
    (41, "minecraft:additional_trade_cost"),
    (42, "minecraft:stored_enchantments"),
    (43, "minecraft:dye"),
    (44, "minecraft:dyed_color"),
    (45, "minecraft:map_color"),
    (46, "minecraft:map_id"),
    (47, "minecraft:map_decorations"),
    (48, "minecraft:map_post_processing"),
    (49, "minecraft:charged_projectiles"),
    (50, "minecraft:bundle_contents"),
    (51, "minecraft:potion_contents"),
    (52, "minecraft:potion_duration_scale"),
    (53, "minecraft:suspicious_stew_effects"),
    (54, "minecraft:writable_book_content"),
    (55, "minecraft:written_book_content"),
    (56, "minecraft:trim"),
    (57, "minecraft:debug_stick_state"),
    (58, "minecraft:entity_data"),
    (59, "minecraft:bucket_entity_data"),
    (60, "minecraft:block_entity_data"),
    (61, "minecraft:instrument"),
    (62, "minecraft:provides_trim_material"),
    (63, "minecraft:ominous_bottle_amplifier"),
    (64, "minecraft:jukebox_playable"),
    (65, "minecraft:provides_banner_patterns"),
    (66, "minecraft:recipes"),
    (67, "minecraft:lodestone_tracker"),
    (68, "minecraft:firework_explosion"),
    (69, "minecraft:fireworks"),
    (70, "minecraft:profile"),
    (71, "minecraft:note_block_sound"),
    (72, "minecraft:banner_patterns"),
    (73, "minecraft:base_color"),
    (74, "minecraft:pot_decorations"),
    (75, "minecraft:container"),
    (76, "minecraft:block_state"),
    (77, "minecraft:bees"),
    (78, "minecraft:lock"),
    (79, "minecraft:container_loot"),
    (80, "minecraft:break_sound"),
    (81, "minecraft:villager/variant"),
    (82, "minecraft:wolf/variant"),
    (83, "minecraft:wolf/sound_variant"),
    (84, "minecraft:wolf/collar"),
    (85, "minecraft:fox/variant"),
    (86, "minecraft:salmon/size"),
    (87, "minecraft:parrot/variant"),
    (88, "minecraft:tropical_fish/pattern"),
    (89, "minecraft:tropical_fish/base_color"),
    (90, "minecraft:tropical_fish/pattern_color"),
    (91, "minecraft:mooshroom/variant"),
    (92, "minecraft:rabbit/variant"),
    (93, "minecraft:pig/variant"),
    (94, "minecraft:pig/sound_variant"),
    (95, "minecraft:cow/variant"),
    (96, "minecraft:cow/sound_variant"),
    (97, "minecraft:chicken/variant"),
    (98, "minecraft:chicken/sound_variant"),
    (99, "minecraft:zombie_nautilus/variant"),
    (100, "minecraft:frog/variant"),
    (101, "minecraft:horse/variant"),
    (102, "minecraft:painting/variant"),
    (103, "minecraft:llama/variant"),
    (104, "minecraft:axolotl/variant"),
    (105, "minecraft:cat/variant"),
    (106, "minecraft:cat/sound_variant"),
    (107, "minecraft:cat/collar"),
    (108, "minecraft:sheep/color"),
    (109, "minecraft:shulker/color"),
)

COMPONENT_NAMES: dict[int, str] = {cid: name for cid, name in _COMPONENT_TABLE}

SUPPORTED_COMPONENT_IDS: tuple[int, ...] = (0, 2, 3, 6, 11, 13, 51)

# Bounds mirrored from the Rust decoders cited above.
_MAX_PATCH_COMPONENTS = 110
_MAX_MENU_SLOTS = 256
_MAX_MERCHANT_OFFERS = 256
_MAX_MERCHANT_COST_COMPONENTS = 64
_MAX_STRING = 32767
_MAX_LORE_LINES = 256
_MAX_ENCHANTMENTS = 256
_MAX_POTION_EFFECTS = 256
_MAX_POTION_HIDDEN_DEPTH = 32
_NBT_MAX_DEPTH = 512
_NBT_MAX_COMPOUND_ENTRIES = 65536
_NBT_MAX_NODES = 1048576
_NBT_MAX_LIST = 16 * 1024 * 1024

# Minimum encoded bytes per NBT list element (mirrors mc-nbt
# minimum_payload_bytes); used for the wire-feasibility fence.
_NBT_MIN_PAYLOAD = {
    0: 1,
    1: 1,
    2: 2,
    3: 4,
    4: 8,
    5: 4,
    6: 8,
    7: 4,
    8: 2,
    9: 5,
    10: 1,
    11: 4,
    12: 4,
}


def _decode_mutf8(raw: bytes) -> str:
    """Decode NBT modified UTF-8 (null is ``0xC0 0x80``; astral chars ride
    surrogate pairs) the way mc-nbt does; anything else fails closed."""
    try:
        text = raw.replace(b"\xc0\x80", b"\x00").decode("utf-8", "surrogatepass")
        combined = text.encode("utf-16-le", "surrogatepass").decode("utf-16-le")
    except (UnicodeDecodeError, UnicodeEncodeError, ValueError) as error:
        raise TruncatedObservation(f"invalid NBT string bytes: {error}") from error
    if any(0xD800 <= ord(char) <= 0xDFFF for char in combined):
        raise TruncatedObservation("unpaired surrogate in NBT string")
    return combined


class Cursor:
    """Reusable bounded reader over one packet body.

    Every read is fenced against the remaining bytes; overlong
    VarInts, negative lengths and infeasible counts raise
    :class:`TruncatedObservation`. After any error the position is
    unspecified: discard the body, never resume it.
    """

    __slots__ = ("_data", "_pos")

    def __init__(self, data: bytes | bytearray | memoryview, offset: int = 0) -> None:
        if isinstance(data, (bytearray, memoryview)):
            data = bytes(data)
        if not isinstance(data, bytes):
            raise TypeError(f"packet body must be bytes, got {type(data).__name__}")
        if not 0 <= offset <= len(data):
            raise TruncatedObservation(
                f"offset {offset} outside {len(data)}-byte body"
            )
        self._data = data
        self._pos = offset

    @property
    def remaining(self) -> int:
        return len(self._data) - self._pos

    def tell(self) -> int:
        return self._pos

    def take(self, count: int, what: str = "bytes") -> bytes:
        if count < 0:
            raise TruncatedObservation(f"negative {what} length {count}")
        if count > self.remaining:
            raise TruncatedObservation(
                f"truncated {what}: need {count}, have {self.remaining}"
            )
        start = self._pos
        self._pos += count
        return self._data[start:self._pos]

    def u8(self, what: str = "byte") -> int:
        return self.take(1, what)[0]

    def bool(self, what: str = "bool") -> bool:
        return self.u8(what) != 0

    def i8(self) -> int:
        return struct.unpack_from(">b", self.take(1, "i8"))[0]

    def u16(self) -> int:
        return struct.unpack_from(">H", self.take(2, "u16"))[0]

    def i16(self) -> int:
        return struct.unpack_from(">h", self.take(2, "i16"))[0]

    def i32(self) -> int:
        return struct.unpack_from(">i", self.take(4, "i32"))[0]

    def i64(self) -> int:
        return struct.unpack_from(">q", self.take(8, "i64"))[0]

    def f32(self) -> float:
        return struct.unpack_from(">f", self.take(4, "f32"))[0]

    def f64(self) -> float:
        return struct.unpack_from(">d", self.take(8, "f64"))[0]

    def varint(self, what: str = "VarInt") -> int:
        value = 0
        for shift in range(0, 35, 7):
            byte = self.u8(what)
            if shift == 28 and byte > 15:
                raise TruncatedObservation(f"overflow {what}")
            value |= (byte & 0x7F) << shift
            if not byte & 0x80:
                return value - (1 << 32) if value >= 1 << 31 else value
        raise TruncatedObservation(f"overlong {what}")

    def mc_string(self, max_units: int = _MAX_STRING, what: str = "string") -> str:
        raw = self.varint(what + " length")
        if raw < 0:
            raise TruncatedObservation(f"negative {what} length {raw}")
        if raw > max_units * 3:
            raise TruncatedObservation(f"{what} too long: {raw} bytes")
        try:
            text = self.take(raw, what).decode("utf-8")
        except UnicodeDecodeError as error:
            raise TruncatedObservation(f"invalid utf-8 in {what}: {error}") from error
        if len(text.encode("utf-16-le")) // 2 > max_units:
            raise TruncatedObservation(f"{what} exceeds {max_units} UTF-16 units")
        return text

    def identifier(self, what: str = "identifier") -> str:
        text = self.mc_string(_MAX_STRING, what)
        namespace, sep, path = text.partition(":")
        if not sep:
            namespace, path = "minecraft", namespace
        valid = (
            namespace
            and path
            and all(c.isascii() and (c.islower() or c.isdigit() or c in "_-.") for c in namespace)
            and all(
                c.isascii() and (c.islower() or c.isdigit() or c in "_-./") for c in path
            )
        )
        if not valid:
            raise TruncatedObservation(f"invalid {what} {text!r}")
        return f"{namespace}:{path}"

    # -- generic network NBT (mc-nbt read_network_value shape) -------------

    def nbt_string(self) -> str:
        return _decode_mutf8(self.take(self.u16(), "nbt string"))

    def _nbt_scalar(self, tag: int, budget: list[int]):
        budget[0] += 1
        if budget[0] > _NBT_MAX_NODES:
            raise TruncatedObservation("nbt node budget exceeded")
        if tag == 1:
            return self.i8()
        if tag == 2:
            return self.i16()
        if tag == 3:
            return self.i32()
        if tag == 4:
            return self.i64()
        if tag == 5:
            return self.f32()
        if tag == 6:
            return self.f64()
        if tag == 7:
            length = self.i32()
            if length < 0 or length > _NBT_MAX_LIST:
                raise TruncatedObservation(f"invalid nbt byte array length {length}")
            return [value - 256 if value > 127 else value
                    for value in self.take(length, "nbt byte array")]
        if tag == 8:
            return self.nbt_string()
        if tag == 11:
            length = self.i32()
            if length < 0 or length > _NBT_MAX_LIST:
                raise TruncatedObservation(f"invalid nbt int array length {length}")
            if length > self.remaining // 4:
                raise TruncatedObservation("truncated nbt int array")
            return [self.i32() for _ in range(length)]
        if tag == 12:
            length = self.i32()
            if length < 0 or length > _NBT_MAX_LIST:
                raise TruncatedObservation(f"invalid nbt long array length {length}")
            if length > self.remaining // 8:
                raise TruncatedObservation("truncated nbt long array")
            return [self.i64() for _ in range(length)]
        raise TruncatedObservation(f"unknown NBT tag {tag}")

    def _open_nbt_container(self, tag: int, depth: int, budget: list[int]) -> list:
        if depth > _NBT_MAX_DEPTH:
            raise TruncatedObservation(
                f"nbt nesting exceeds {_NBT_MAX_DEPTH}-level limit"
            )
        budget[0] += 1
        if budget[0] > _NBT_MAX_NODES:
            raise TruncatedObservation("nbt node budget exceeded")
        if tag == 10:
            return [0, [], None]
        element = self.u8("nbt list element")
        count = self.i32()
        if count < 0 or count > _NBT_MAX_LIST:
            raise TruncatedObservation(f"invalid nbt list length {count}")
        if element > 12 or (element == 0 and count):
            raise TruncatedObservation(f"invalid NBT list element type {element}")
        if count and count > self.remaining // _NBT_MIN_PAYLOAD[element]:
            raise TruncatedObservation(
                f"truncated nbt list: {count} elements do not fit in {self.remaining} bytes"
            )
        return [1, element, count, []]

    @staticmethod
    def _attach(parent: list, key: str | None, value) -> None:
        if parent[0] == 0:
            assert key is not None
            parent[1].append((key, value))
            parent[2] = None
        else:
            parent[3].append(value)

    def read_nbt_value(self):
        """Parse one anonymous network NBT value (any tag root).

        Iterative over an explicit frame stack, so adversarial nesting can
        never exhaust the interpreter stack. Compounds become dicts
        (insertion order, last duplicate wins), lists become lists, arrays
        become lists, strings become str, numerics become int/float.
        """
        tag = self.u8("nbt tag")
        budget = [0]
        if tag not in (9, 10):
            return self._nbt_scalar(tag, budget)
        stack = [self._open_nbt_container(tag, 1, budget)]
        while stack:
            frame = stack[-1]
            if frame[0] == 0:
                entry = self.u8("nbt entry")
                if entry == 0:
                    completed: dict = dict(frame[1])
                    budget[0] += 1
                    stack.pop()
                    if not stack:
                        return completed
                    # The pending key lives on the parent's scratch slot;
                    # the child's own slot may hold its inner nesting state.
                    parent = stack[-1]
                    self._attach(parent, parent[2] if parent[0] == 0 else None, completed)
                else:
                    name = self.nbt_string()
                    if len(frame[1]) >= _NBT_MAX_COMPOUND_ENTRIES:
                        raise TruncatedObservation("nbt compound entry budget exceeded")
                    if entry in (9, 10):
                        frame[2] = name
                        stack.append(
                            self._open_nbt_container(entry, len(stack) + 1, budget)
                        )
                    else:
                        frame[1].append((name, self._nbt_scalar(entry, budget)))
            elif frame[2] == 0:
                completed_list = frame[3]
                stack.pop()
                if not stack:
                    return completed_list
                parent = stack[-1]
                self._attach(parent, parent[2] if parent[0] == 0 else None, completed_list)
            elif frame[1] in (9, 10):
                stack.append(self._open_nbt_container(frame[1], len(stack) + 1, budget))
                frame[2] -= 1
            else:
                frame[3].append(self._nbt_scalar(frame[1], budget))
                frame[2] -= 1

    def read_nbt_compound(self) -> dict:
        value = self.read_nbt_value()
        if not isinstance(value, dict):
            raise TruncatedObservation("nbt root must be a compound")
        return value


def _bounded_count(cursor: Cursor, maximum: int, minimum: int, what: str) -> int:
    count = cursor.varint(what + " count")
    if count < 0:
        raise TruncatedObservation(f"negative {what} count {count}")
    if count > maximum:
        raise TruncatedObservation(f"{what} count {count} exceeds {maximum}")
    if count > cursor.remaining // minimum:
        raise TruncatedObservation(
            f"truncated {what}: {count} entries do not fit in {cursor.remaining} bytes"
        )
    return count


def decode_registry(body: bytes | bytearray | memoryview) -> tuple[str, list[str]]:
    """Read configuration RegistryData names in their actual numeric-ID order.

    KnownPacks may omit NBT payloads, never entry names. Source:
    ``packets/configuration.rs::RegistryData`` (clientbound configuration 0x07).
    """
    cursor = Cursor(body)
    name = cursor.identifier("registry")
    count = _bounded_count(cursor, 65_536, 2, "registry entries")
    entries = []
    for _ in range(count):
        entries.append(cursor.identifier("registry entry"))
        if cursor.bool("registry entry has data"):
            cursor.read_nbt_compound()
    if cursor.remaining:
        raise TruncatedObservation(f"trailing registry bytes: {cursor.remaining}")
    return name, entries


def _decode_enchantments(cursor: Cursor) -> list[list[int]]:
    count = _bounded_count(cursor, _MAX_ENCHANTMENTS, 2, "enchantments")
    pairs = []
    seen = set()
    for _ in range(count):
        enchant_id = cursor.varint("enchantment id")
        if enchant_id < 0:
            raise UnsupportedObservation(
                f"negative enchantment registry id {enchant_id}"
            )
        level = cursor.varint("enchantment level")
        if not 1 <= level <= 255:
            raise UnsupportedObservation(f"invalid enchantment level {level}")
        if enchant_id in seen:
            raise UnsupportedObservation(
                f"duplicate enchantment id {enchant_id}"
            )
        seen.add(enchant_id)
        pairs.append([enchant_id, level])
    return pairs


def _decode_potion_effect(cursor: Cursor, depth: int) -> dict:
    if depth > _MAX_POTION_HIDDEN_DEPTH:
        raise UnsupportedObservation("potion hidden effect depth exceeded")
    effect_id = cursor.varint("potion effect id")
    if effect_id < 0:
        raise UnsupportedObservation(f"negative potion effect id {effect_id}")
    amplifier = cursor.varint("potion effect amplifier")
    if not 0 <= amplifier <= 255:
        raise UnsupportedObservation(f"invalid potion effect amplifier {amplifier}")
    return {
        "id": effect_id,
        "amplifier": amplifier,
        "duration": cursor.varint("potion effect duration"),
        "ambient": cursor.bool("potion effect ambient"),
        "show_particles": cursor.bool("potion effect particles"),
        "show_icon": cursor.bool("potion effect icon"),
        "hidden": _decode_potion_effect(cursor, depth + 1)
        if cursor.bool("potion effect hidden")
        else None,
    }


def _decode_potion_contents(cursor: Cursor) -> dict:
    potion = None
    if cursor.bool("potion present"):
        potion = cursor.varint("potion id")
        if potion < 0:
            raise UnsupportedObservation(f"negative potion id {potion}")
    color = cursor.i32() if cursor.bool("potion color present") else None
    count = _bounded_count(cursor, _MAX_POTION_EFFECTS, 7, "potion effects")
    effects = [_decode_potion_effect(cursor, 0) for _ in range(count)]
    custom_name = (
        cursor.mc_string(_MAX_STRING, "potion custom name")
        if cursor.bool("potion name present")
        else None
    )
    return {
        "potion": potion,
        "color": color,
        "effects": effects,
        "custom_name": custom_name,
    }


def _decode_text_value(cursor: Cursor, what: str):
    value = cursor.read_nbt_value()
    if not isinstance(value, (str, dict, list)):
        raise UnsupportedObservation(f"invalid {what} text component")
    return value


def _decode_component(cursor: Cursor, cid: int):
    if cid == 0:
        return cursor.read_nbt_compound()
    if cid == 2:
        value = cursor.varint("max_damage")
        if not 1 <= value <= 0x7FFFFFFF:
            raise UnsupportedObservation(f"invalid max_damage {value}")
        return value
    if cid == 3:
        return max(cursor.varint("damage"), 0)
    if cid == 6:
        return _decode_text_value(cursor, "custom_name")
    if cid == 11:
        count = _bounded_count(cursor, _MAX_LORE_LINES, 1, "lore lines")
        lines = [cursor.read_nbt_value() for _ in range(count)]
        for line in lines:
            if not isinstance(line, (str, dict, list)):
                raise UnsupportedObservation("invalid lore text component")
        return lines
    if cid == 13:
        return _decode_enchantments(cursor)
    if cid == 51:
        return _decode_potion_contents(cursor)
    name = COMPONENT_NAMES.get(cid, f"unknown-id-{cid}")
    raise UnsupportedObservation(f"unsupported item component {name} (id {cid})")


def _decode_stack_at(cursor: Cursor):
    count = cursor.varint("stack count")
    if count < 0:
        raise TruncatedObservation(f"negative stack count {count}")
    if count == 0:
        return None
    item_id = cursor.varint("item id")
    if item_id < 0:
        raise TruncatedObservation(f"negative item id {item_id}")
    added = cursor.varint("added component count")
    removed_count = cursor.varint("removed component count")
    if added < 0 or removed_count < 0:
        raise TruncatedObservation(
            f"negative component patch shape ({added}, {removed_count})"
        )
    if (
        added > _MAX_PATCH_COMPONENTS
        or removed_count > _MAX_PATCH_COMPONENTS
        or added + removed_count > _MAX_PATCH_COMPONENTS
    ):
        raise UnsupportedObservation(
            "unsupported DataComponentPatch shape "
            f"(added {added}, removed {removed_count})"
        )
    components: dict[str, object] = {}
    removed: list[str] = []
    seen: set[int] = set()
    for _ in range(added):
        cid = cursor.varint("component id")
        if cid in seen:
            raise UnsupportedObservation(f"duplicate item component id {cid}")
        seen.add(cid)
        name = COMPONENT_NAMES.get(cid)
        if name is None:
            raise UnsupportedObservation(f"unknown item component id {cid}")
        if cid not in SUPPORTED_COMPONENT_IDS:
            raise UnsupportedObservation(
                f"unsupported item component {name} (id {cid})"
            )
        components[name] = _decode_component(cursor, cid)
    for _ in range(removed_count):
        cid = cursor.varint("removed component id")
        if cid in seen:
            raise UnsupportedObservation(f"duplicate item component id {cid}")
        seen.add(cid)
        name = COMPONENT_NAMES.get(cid)
        if name is None:
            raise UnsupportedObservation(f"unknown removed item component id {cid}")
        removed.append(name)
    return {
        "item_id": item_id,
        "count": count,
        "components": components,
        "removed": removed,
    }


def decode_stack(data: bytes | bytearray | memoryview, offset: int = 0):
    """Decode one ``ItemStack`` at ``offset``; return ``(stack|None, offset)``.

    ``None`` is the empty stack (leading zero count). Numeric ``item_id``
    values are kept as-is: the parent maps them through the item registry.
    """
    cursor = Cursor(data, offset)
    return _decode_stack_at(cursor), cursor.tell()


def decode_menu(body: bytes | bytearray | memoryview) -> dict:
    """Decode a ``ClientboundContainerSetContent`` body (window, state_id,
    slots, cursor). Slots and cursor are ``decode_stack`` values; trailing
    bytes are rejected."""
    cursor = Cursor(body)
    window = cursor.varint("window id")
    state_id = cursor.varint("state id")
    count = _bounded_count(cursor, _MAX_MENU_SLOTS, 1, "menu slots")
    slots = [_decode_stack_at(cursor) for _ in range(count)]
    carried = _decode_stack_at(cursor)
    if cursor.remaining:
        raise TruncatedObservation(
            f"trailing bytes in menu image: {cursor.remaining}"
        )
    return {"window": window, "state_id": state_id, "slots": slots, "cursor": carried}


def _decode_item_cost(cursor: Cursor) -> dict:
    item_id = cursor.varint("merchant item id")
    if item_id < 0:
        raise UnsupportedObservation(f"negative merchant item id {item_id}")
    count = cursor.varint("merchant item count")
    if count <= 0:
        raise UnsupportedObservation("merchant item cost count must be positive")
    predicates = cursor.varint("merchant predicate size")
    if predicates < 0:
        raise TruncatedObservation(f"negative merchant predicate size {predicates}")
    if predicates > _MAX_MERCHANT_COST_COMPONENTS:
        raise TruncatedObservation(
            f"merchant predicate size {predicates} exceeds {_MAX_MERCHANT_COST_COMPONENTS}"
        )
    if predicates != 0:
        raise UnsupportedObservation("merchant exact component predicates are unsupported")
    return {"item_id": item_id, "count": count}


def decode_merchant_offers(body: bytes | bytearray | memoryview) -> dict:
    """Decode a ``ClientboundMerchantOffers`` body.

    Returns ``container_id``, ``offers`` (each with ``input1``, ``input2``,
    ``output``, ``uses``, ``max_uses``, ``xp`` plus the remaining counters
    verbatim), ``level``, ``xp``, ``show_progress`` and ``can_restock`` --
    enough for the parent to publish normalized ``menu.offers``.
    """
    cursor = Cursor(body)
    container_id = cursor.varint("merchant container id")
    if container_id < 0:
        raise UnsupportedObservation("invalid merchant container id")
    count = _bounded_count(cursor, _MAX_MERCHANT_OFFERS, 1, "merchant offers")
    offers = []
    for _ in range(count):
        cost_a = _decode_item_cost(cursor)
        result = _decode_stack_at(cursor)
        if result is None:
            raise UnsupportedObservation("merchant result must be non-empty")
        cost_b = _decode_item_cost(cursor) if cursor.bool("merchant second cost") else None
        offer = {
            "input1": cost_a,
            "input2": cost_b,
            "output": result,
            "out_of_stock": cursor.bool("merchant stock flag"),
            "uses": cursor.i32(),
            "max_uses": cursor.i32(),
            "xp": cursor.i32(),
            "special_price": cursor.i32(),
            "price_multiplier": cursor.f32(),
            "demand": cursor.i32(),
        }
        if (
            offer["uses"] < 0
            or offer["max_uses"] <= 0
            or offer["uses"] > offer["max_uses"]
            or offer["xp"] < 0
        ):
            raise UnsupportedObservation("invalid merchant offer counters")
        if not math.isfinite(offer["price_multiplier"]) or offer["price_multiplier"] < 0:
            raise UnsupportedObservation("invalid merchant price multiplier")
        offers.append(offer)
    level = cursor.varint("villager level")
    villager_xp = cursor.varint("villager xp")
    if level <= 0 or villager_xp < 0:
        raise UnsupportedObservation("invalid merchant level or xp")
    merchant = {
        "container_id": container_id,
        "offers": offers,
        "level": level,
        "xp": villager_xp,
        "show_progress": cursor.bool("merchant progress flag"),
        "can_restock": cursor.bool("merchant restock flag"),
    }
    if cursor.remaining:
        raise TruncatedObservation(
            f"trailing bytes in merchant offers: {cursor.remaining}"
        )
    return merchant


def _plain_extra_text(node) -> str:
    text, translate, _, _ = _normalize_text(node)
    if translate is not None:
        raise UnsupportedObservation("unsupported nested translate in chat component")
    return text


def _normalize_text(node):
    """Return ``(text, translate, with, fallback)`` for one text component.

    Literal compounds render ``text`` plus ``extra``; ``translate``
    compounds keep their key and structured args with ``text`` empty so no
    localization is fabricated. Anything else fails closed.
    """
    if isinstance(node, str):
        return node, None, None, None
    if isinstance(node, list):
        return "".join(_plain_extra_text(item) for item in node), None, None, None
    if isinstance(node, dict):
        if isinstance(node.get("text"), str):
            extra = node.get("extra", [])
            if not isinstance(extra, list):
                raise UnsupportedObservation("unsupported chat extra shape")
            return node["text"] + "".join(_plain_extra_text(item) for item in extra), None, None, None
        if isinstance(node.get("translate"), str):
            args = node.get("with", [])
            if not isinstance(args, list):
                raise UnsupportedObservation("unsupported chat translate args")
            normalized_args = [
                {
                    "text": text,
                    "translate": translate,
                    "with": with_args,
                    "fallback": fallback,
                }
                for text, translate, with_args, fallback in (
                    _normalize_text(item) for item in args
                )
            ]
            fallback = node.get("fallback")
            normalized_fallback = None
            if fallback is not None:
                text, translate, with_args, nested = _normalize_text(fallback)
                normalized_fallback = {
                    "text": text,
                    "translate": translate,
                    "with": with_args,
                    "fallback": nested,
                }
            return "", node["translate"], normalized_args, normalized_fallback
    raise UnsupportedObservation("unsupported chat component shape")


def decode_chat_component(body: bytes | bytearray | memoryview) -> dict:
    """Decode a ``ClientboundSystemChat`` body (component NBT + overlay bool).

    Parses the NBT properly instead of decoding raw bytes, so literal text
    never carries wire garbage; translation keys and args are preserved
    structurally rather than rendered.
    """
    cursor = Cursor(body)
    if not cursor.remaining:
        raise TruncatedObservation("empty chat component")
    payload = cursor.take(cursor.remaining - 1, "chat nbt")
    overlay = cursor.bool("chat overlay")
    nested = Cursor(payload)
    node = nested.read_nbt_value()
    if nested.remaining:
        raise TruncatedObservation(
            f"trailing bytes in chat component: {nested.remaining}"
        )
    text, translate, with_args, fallback = _normalize_text(node)
    return {
        "text": text,
        "translate": translate,
        "with": with_args,
        "fallback": fallback,
        "overlay": overlay,
    }
