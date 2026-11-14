"""Station-menu executable verbs as a mixin for ProtocolMatrixClient.

Parent imports this into ``tools/harness/protocol_matrix_client.py``; this
module never owns lifecycle/receipts (canonical ``tools.harness cmd_run`` does)
and never completes an action itself -- every verb only writes one real
serverbound packet plus the ``0x0D`` tick-end, exactly like the existing
``ProtocolMatrixClient`` verbs. Observation of outcomes stays with the caller
(``snapshot`` / ``wait_until`` on received state).

Packet layouts/ids are pinned against local ``mc-protocol`` source:

- ``ServerboundContainerButtonClick`` id ``0x11``: VarInt container +
  VarInt button (``crates/mc-protocol/src/packets/play/inventory.rs``).
- ``ServerboundSelectTrade`` id ``0x33``: VarInt offer, negatives rejected
  (``crates/mc-protocol/src/packets/play/merchant.rs``).
- ``ServerboundRenameItem`` id ``0x30``: one UTF string, max 32767 UTF-16
  code units (``.../play/rename_item.rs``).
- ``ServerboundSetBeacon`` id ``0x34``: two ``optional(holderRegistry)``
  effects, each bool + VarInt registry id when present
  (``.../play/set_beacon.rs``; effect ids are the zero-based
  ``mc-data/src/mob_effects_26_1_2.rs`` table, e.g. speed=0,
  regeneration=9).
- ``ServerboundContainerSlotStateChanged`` id ``0x14``: VarInt slot_id,
  VarInt container_id, bool new_state where True *enables*
  (``.../play/crafter.rs``).
- ``ServerboundSetCreativeModeSlot`` id ``0x38``: big-endian i16 slot then
  ``ItemStack`` optional payload: VarInt 0 when empty, else VarInt count +
  VarInt item id + VarInt add-count + VarInt remove-count
  (``.../play/inventory.rs`` + ``.../play/tests/serverbound_and_slots.rs``:
  slot 36, item 1 x64 encodes ``00 24 40 01 00 00``).
- ``ServerboundPlaceRecipe`` id ``0x27``: VarInt container, VarInt recipe
  display id, bool use_max (``.../play/inventory.rs``).
- ``ServerboundContainerClick`` id ``0x12``: VarInt container, VarInt
  state, i16 slot, i8 button, VarInt ContainerInput, empty changed-slot map
  and empty HashedStack prediction (``.../play/inventory.rs`` +
  ``tools/harness/wire.click_body``).
"""

from __future__ import annotations

import struct

from . import wire
from .matrix_api import MissingCapability

_CLICK_MODES = {
    'pickup': 0,
    'quick_move': 1,
    'swap': 2,
    'clone': 3,
    'throw': 4,
    'quick_craft': 5,
    'pickup_all': 6,
}

# Zero-based mob_effect registry order from
# crates/mc-data/src/mob_effects_26_1_2.rs name_for_id (40 entries).
_EFFECT_NAMES = (
    "minecraft:speed",
    "minecraft:slowness",
    "minecraft:haste",
    "minecraft:mining_fatigue",
    "minecraft:strength",
    "minecraft:instant_health",
    "minecraft:instant_damage",
    "minecraft:jump_boost",
    "minecraft:nausea",
    "minecraft:regeneration",
    "minecraft:resistance",
    "minecraft:fire_resistance",
    "minecraft:water_breathing",
    "minecraft:invisibility",
    "minecraft:blindness",
    "minecraft:night_vision",
    "minecraft:hunger",
    "minecraft:weakness",
    "minecraft:poison",
    "minecraft:wither",
    "minecraft:health_boost",
    "minecraft:absorption",
    "minecraft:saturation",
    "minecraft:glowing",
    "minecraft:levitation",
    "minecraft:luck",
    "minecraft:unluck",
    "minecraft:slow_falling",
    "minecraft:conduit_power",
    "minecraft:dolphins_grace",
    "minecraft:bad_omen",
    "minecraft:hero_of_the_village",
    "minecraft:darkness",
    "minecraft:trial_omen",
    "minecraft:raid_omen",
    "minecraft:wind_charged",
    "minecraft:weaving",
    "minecraft:oozing",
    "minecraft:infested",
    "minecraft:breath_of_the_nautilus",
)

_RENAME_MAX_UNITS = 32767
_CREATIVE_MAX_COUNT = 127  # wire.py plain-stack bound; avoids inventing 64/99.


def _effect_id(name: str) -> int:
    key = name if name.startswith("minecraft:") else f"minecraft:{name}"
    try:
        return _EFFECT_NAMES.index(key)
    except ValueError:
        raise MissingCapability(f'mob_effect:{name}') from None


def _utf16_units(text: str) -> int:
    return len(text.encode('utf-16-le')) // 2


class ProtocolStationVerbs:
    """Mixin: station-menu serverbound verbs. Requires ``self.observer``
    (``wire.PlayObserver`` with ``send``/``current_window``) and ``self.items``
    (numeric item id -> namespaced name, as built by
    ``ProtocolMatrixClient``)."""

    observer: object  # wire.PlayObserver at runtime
    items: dict[int, str]

    def _window(self) -> int:
        return self.observer.current_window  # type: ignore[attr-defined]

    def _send(self, packet_id: int, body: bytes = b"") -> None:
        self.observer.send(packet_id, body)  # type: ignore[attr-defined]
        self.observer.send(0x0D)  # type: ignore[attr-defined]

    def container_button(self, button: int) -> None:
        """Write ContainerButtonClick (0x11) for the open window. Write-only."""
        if button < 0:
            raise ValueError(f'button out of range: {button}')
        self._send(0x11, wire.write_varint(self._window()) + wire.write_varint(button))

    def select_trade(self, index: int) -> None:
        """Write SelectTrade (0x33) with a non-negative offer index. Write-only."""
        if index < 0:
            raise ValueError(f'offer index out of range: {index}')
        self._send(0x33, wire.write_varint(index))

    def rename_item(self, text: str) -> None:
        """Write RenameItem (0x30) with one UTF string. Write-only."""
        if _utf16_units(text) > _RENAME_MAX_UNITS:
            raise ValueError(f'rename text exceeds {_RENAME_MAX_UNITS} UTF-16 units')
        self._send(0x30, wire.mc_string(text))

    def set_beacon(self, primary: str | None, secondary: str | None = None) -> None:
        """Write SetBeacon (0x34); names are namespaced or bare effect ids.

        Unknown effect names raise ``MissingCapability('mob_effect:<name>')``.
        Write-only.
        """
        body = b""
        for effect in (primary, secondary):
            if effect is None:
                body += b"\x00"
            else:
                body += b"\x01" + wire.write_varint(_effect_id(effect))
        self._send(0x34, body)

    def toggle_crafter_slot(self, slot: int, disabled: bool) -> None:
        """Write ContainerSlotStateChanged (0x14) for the open crafter window.

        ``disabled=True`` sends ``new_state=False`` (vanilla disables the
        slot). Write-only.
        """
        if slot < 0:
            raise ValueError(f'crafter slot out of range: {slot}')
        body = (wire.write_varint(slot) + wire.write_varint(self._window())
                + bytes([0x00 if disabled else 0x01]))
        self._send(0x14, body)

    def set_creative_slot(self, slot: int, item_id: str, count: int,
                          components: dict | None = None) -> None:
        """Write SetCreativeModeSlot (0x38) with a plain stack. Write-only.

        ``count == 0`` encodes the vanilla empty payload (clears the slot).
        Any nonempty ``components`` mapping raises
        ``MissingCapability('unsupported components:[...]')`` naming the exact
        keys, since this adapter only encodes component-free stacks.
        """
        if not -32768 <= slot <= 32767:
            raise ValueError(f'creative slot out of i16 range: {slot}')
        if not 0 <= count <= _CREATIVE_MAX_COUNT:
            raise ValueError(f'creative count out of range: {count}')
        if components:
            raise MissingCapability(f'unsupported components:{sorted(components)}')
        if count == 0:
            body = struct.pack('>h', slot) + b"\x00"
        else:
            numeric = next((num for num, name in self.items.items() if name == item_id), None)
            if numeric is None:
                raise MissingCapability(f'item:{item_id}')
            body = (struct.pack('>h', slot) + wire.write_varint(count)
                    + wire.write_varint(numeric) + b"\x00\x00")
        self._send(0x38, body)

    def place_recipe(self, recipe_id: str, all: bool = False) -> None:
        """Write PlaceRecipe (0x27) using the received recipe-display mapping.

        The numeric display id MUST come from the actual received mapping at
        ``observer.recipe_displays`` (``recipe_id -> display id``); nothing is
        inferred from the string form. Missing mapping raises
        ``MissingCapability('recipe_display_mapping')``; an id absent from the
        mapping raises ``MissingCapability('recipe_display:<id>')``.
        Write-only.
        """
        mapping = getattr(self.observer, 'recipe_displays', None)
        if not isinstance(mapping, dict) or recipe_id not in mapping:
            if not isinstance(mapping, dict):
                raise MissingCapability('recipe_display_mapping')
            raise MissingCapability(f'recipe_display:{recipe_id}')
        display_id = mapping[recipe_id]
        if display_id < 0:
            raise MissingCapability(f'recipe_display:{recipe_id}')
        body = (wire.write_varint(self._window()) + wire.write_varint(display_id)
                + bytes([0x01 if all else 0x00]))
        self._send(0x27, body)

    def stale_click(self, slot, kind='pickup', button=0, state_id=...):
        """Write ContainerClick (0x12) with an explicitly stale ``state_id``.

        ``state_id`` has no usable default: the ``...`` sentinel raises
        ``ValueError`` unless the caller supplies the actual received state.
        ``kind`` uses the ContainerInput order pickup/quick_move/swap/clone/
        throw/quick_craft/pickup_all = 0..6. Write-only.
        """
        if type(state_id) is not int or not -(1 << 31) <= state_id < (1 << 31):
            raise ValueError('stale_click requires an explicit i32 state_id')
        if kind not in _CLICK_MODES:
            raise ValueError(f'unknown click kind: {kind}')
        if not -32768 <= slot <= 32767:
            raise ValueError(f'click slot out of i16 range: {slot}')
        if not -128 <= button <= 127:
            raise ValueError(f'click button out of i8 range: {button}')
        self._send(0x12, wire.click_body(self._window(), state_id, slot,
                                         button, _CLICK_MODES[kind]))
