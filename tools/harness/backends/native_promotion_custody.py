"""Graphical mount-custody and optional resource-pack promotion probes.

The enclosing harness owns offline conversion processes and the configuration
wire observer. The mount fixture becomes imported Anvil data before the tested
withdrawal/deposit; a summon or configured URL alone is not acceptance.
"""
from __future__ import annotations

import json
import uuid
from pathlib import Path
from typing import Any
import tomllib

from .light_wire import Reader

from .native_p1 import _aim, _capture
from .native_v2 import _click, _echo, _give
from .probe_steps import ProbeSteps, container_slot, item_count


MOUNT_SCENARIO = "native-v2-custody-mount-export"
PACK_SCENARIO = "native-v2-p2-optional-pack"

class PackWireReceipt:
    """Observe the unmodified Configuration offer/status and Play login."""

    def __init__(self, packet_report: Path, artifact: Path) -> None:
        report = json.loads(packet_report.read_text())
        self.push_id = report["configuration"]["clientbound"]["minecraft:resource_pack_push"]["protocol_id"]
        self.status_id = report["configuration"]["serverbound"]["minecraft:resource_pack"]["protocol_id"]
        self.play_id = report["play"]["clientbound"]["minecraft:login"]["protocol_id"]
        self.path = artifact / "optional-pack-wire.json"
        self.sequence = 0
        self.observed: dict[str, Any] = {}
        self.connection: object | None = None

    def _same_connection(self, connection: object) -> None:
        if self.connection is None:
            self.connection = connection
        elif self.connection is not connection:
            raise ValueError("optional pack receipt spans multiple connections")

    def server_packet(self, connection: object, phase: str, packet_id: int, payload: bytes) -> None:
        self.sequence += 1
        if phase == "configuration" and packet_id == self.push_id:
            self._same_connection(connection)
            reader = Reader(payload)
            offer = str(uuid.UUID(bytes=reader.take(16)))
            for _ in range(2):  # URL and SHA1.
                reader.take(reader.varint())
            required = reader.take(1)[0]
            if required not in (0, 1):
                raise ValueError("resource-pack offer has invalid required flag")
            self.observed.update(offer_id=offer, offer_required=bool(required),
                                 offer_sequence=self.sequence)
            self._save()
        elif phase == "play" and packet_id == self.play_id:
            self._same_connection(connection)
            self.observed["play_sequence"] = self.sequence
            self._save()

    def client_packet(self, connection: object, phase: str, packet_id: int, payload: bytes) -> None:
        self.sequence += 1
        if phase == "configuration" and packet_id == self.status_id:
            self._same_connection(connection)
            reader = Reader(payload)
            identity = str(uuid.UUID(bytes=reader.take(16)))
            action = reader.varint()
            if reader.offset != len(payload):
                raise ValueError("trailing bytes in resource-pack status")
            # Declined is ordinal 1 in the verified ResourcePackAction codec.
            self.observed.update(decline_id=identity, action="declined" if action == 1 else str(action),
                                 decline_sequence=self.sequence)
            self._save()

    def _save(self) -> None:
        self.path.write_text(json.dumps(self.observed, indent=2) + "\n")

    def receipt(self) -> dict[str, Any]:
        return dict(self.observed)



def convert_mount_world(ctx: Any, binary: Path, config: Path, evidence: dict[str, Any]) -> dict[str, str]:
    """Offline conversion on a stopped private world, via the shipping CLI."""
    from ..profiles import _run_command

    index = len(evidence.setdefault("mount_conversions", []))
    source = Path(evidence["world_dir"])
    exported = ctx.artifact_dir / f"mount-export-{index}"
    imported = ctx.artifact_dir / f"mount-import-{index}"
    receipts = {}
    for operation, src, dst, label in (
            ("export-anvil", source, exported, "export"),
            ("import-anvil", exported, imported, "import")):
        receipt = ctx.artifact_dir / f"mount-{label}-{index}.json"
        _run_command([str(binary), "--config", str(config), "world", operation,
                      "--source", str(src), "--destination", str(dst), "--receipt", str(receipt)],
                     ctx, f"mount-{label}-{index}.log")
        json.loads(receipt.read_text())
        receipts[label + "_receipt"] = str(receipt)
    text = config.read_text()
    old_world = tomllib.loads(text)["data"]["world_dir"]
    token = "world_dir = " + json.dumps(old_world)
    if text.count(token) != 1:
        raise RuntimeError("mount roundtrip cannot identify its private world config field")
    config.write_text(text.replace(token, "world_dir = " + json.dumps(str(imported))))
    evidence["world_dir"] = str(imported)
    evidence["mount_conversions"].append(receipts)
    return receipts


def _mount_menu(steps: ProbeSteps, expected_uuid: str) -> dict[str, Any]:
    target = steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": "minecraft:donkey", "radius": 12,
        "timeout_seconds": steps.remaining(),
    })
    steps.check("imported_donkey_uuid", target["entity_uuid"] == expected_uuid)
    identity = {key: target[key] for key in ("entity_id", "entity_uuid", "entity_type")}
    steps.call("minecraft_approach_entity", {
        "entity_id": identity["entity_id"], "timeout_seconds": steps.remaining(),
    })
    steps.call("minecraft_set_hotbar_slot", {"slot": 8})
    rows = steps.call("minecraft_list_entities", {"radius": 12, "limit": 128})["entities"]
    matches = [row for row in rows if row["entity_uuid"] == expected_uuid]
    steps.check("imported_donkey_visible_for_aim", len(matches) == 1)
    _aim(steps, matches[0])
    steps.wait(lambda s: s.get("target", {}).get("entity_uuid") == expected_uuid,
               "imported_donkey_crosshair_target")
    steps.call("minecraft_press_inputs", {"keys": ["sneak", "use"], "ticks": 1})
    state = steps.wait(
        lambda s: s["container"]["container_id"] != 0
        and s["container"]["menu_class"] == "net.minecraft.world.inventory.HorseInventoryMenu",
        "donkey_menu_received",
    )
    steps.check("chested_donkey_menu_53_slots", state["container"]["slot_count"] == 53)
    return state


def _cargo_exact(state: dict[str, Any], item: str, count: int) -> bool:
    return all(
        (container_slot(state, slot)["item_id"], container_slot(state, slot)["count"])
        == ((item, count) if slot == 2 else ("minecraft:air", 0))
        for slot in range(2, 17)
    ) and state["container"]["carried"]["count"] == 0


def _roundtrip(steps: ProbeSteps) -> None:
    steps.fixture("save-all")
    _echo(steps, "mount_save_ordered")
    player_uuid = steps.observe()["player"]["entity_uuid"]
    steps.call("minecraft_disconnect")
    steps.check("mount_client_disconnected", steps.call("minecraft_observe")["in_play"] is False)
    receipt = steps.client.promotion_mount_export_reimport(steps.remaining())
    if not isinstance(receipt, dict) or not isinstance(receipt.get("address"), str) or not receipt["address"]:
        raise RuntimeError("mount export/reimport hook must return an address and conversion receipts")
    if not all(isinstance(receipt.get(key), str) and receipt[key] for key in ("export_receipt", "import_receipt")):
        raise RuntimeError("mount export/reimport hook omitted conversion receipt paths")
    steps.report.setdefault("mount_roundtrips", []).append(receipt)
    steps.call("minecraft_connect", {"server_addr": receipt["address"]})
    steps.check("mount_reimport_entered_play", steps.call("minecraft_wait_for_play", {
        "timeout_seconds": steps.remaining(),
    })["in_play"] is True)
    state = steps.wait(lambda s: s["player"]["on_ground"], "mount_reimport_grounded")
    steps.check("mount_reimport_same_player", state["player"]["entity_uuid"] == player_uuid)
    _echo(steps, "mount_reimport_ordered")


def _mount(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    if not callable(getattr(steps.client, "promotion_mount_export_reimport", None)):
        raise RuntimeError("mount custody requires the offline clean export/reimport lifecycle hook")
    x, y, z = origin
    steps.fixture(f"summon minecraft:donkey {x + 1.5} {y} {z + 2.5} {{Tame:1b,NoAI:1b}}")
    _echo(steps, "fixture_tamed_donkey_summoned")
    donkey = steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": "minecraft:donkey", "radius": 12, "timeout_seconds": steps.remaining()})
    identity = {key: donkey[key] for key in ("entity_id", "entity_uuid", "entity_type")}
    mount_uuid = donkey["entity_uuid"]
    steps.report["mount_fixture"] = identity
    _give(steps, "minecraft:chest")
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    steps.call("minecraft_interact_entity", identity)
    steps.wait(lambda s: item_count(s, "minecraft:chest") == 0, "donkey_chest_attached_and_debited")
    steps.fixture("debug give minecraft:diamond 7 0")
    steps.call("minecraft_wait_for_inventory", {
        "item_id": "minecraft:diamond", "count": 7, "timeout_seconds": steps.remaining()})
    supplied = _mount_menu(steps, mount_uuid)
    source = next(stack["slot"] for stack in supplied["container"]["slots"]
                  if stack["item_id"] == "minecraft:diamond" and stack["count"] == 7)
    _click(steps, source, quick=True)
    steps.wait(lambda s: _cargo_exact(s, "minecraft:diamond", 7)
               and item_count(s, "minecraft:diamond") == 0, "fixture_seven_diamonds_in_mount")
    steps.call("minecraft_close_screen")
    # The first conversion makes the initial cargo genuinely imported Anvil NBT.
    _roundtrip(steps)
    initial = _mount_menu(steps, mount_uuid)
    steps.check("imported_exact_seven_diamonds", _cargo_exact(initial, "minecraft:diamond", 7)
                and item_count(initial, "minecraft:diamond") == 0
                and item_count(initial, "minecraft:emerald") == 0)
    _capture(steps, "custody-mount-imported")
    _click(steps, 2, quick=True)
    withdrawn = steps.wait(lambda s: _cargo_exact(s, "minecraft:air", 0)
                           and item_count(s, "minecraft:diamond") == 7,
                           "withdraw_seven_diamonds_exactly_once")
    steps.check("withdrawal_cursor_empty", withdrawn["container"]["carried"]["count"] == 0)
    steps.call("minecraft_close_screen")
    steps.fixture("debug give minecraft:emerald 5 0")
    steps.call("minecraft_wait_for_inventory", {"item_id": "minecraft:emerald", "count": 5,
                                                  "timeout_seconds": steps.remaining()})
    state = _mount_menu(steps, mount_uuid)
    sources = [s for s in state["container"]["slots"] if s["slot"] >= 17
               and s["item_id"] == "minecraft:emerald" and s["count"] == 5]
    steps.check("one_emerald_stack_for_deposit", len(sources) == 1)
    _click(steps, sources[0]["slot"], quick=True)
    steps.wait(lambda s: _cargo_exact(s, "minecraft:emerald", 5)
               and item_count(s, "minecraft:emerald") == 0
               and item_count(s, "minecraft:diamond") == 7,
               "deposited_five_emeralds_exact_custody")
    _capture(steps, "custody-mount-before-export")
    steps.call("minecraft_close_screen")
    _roundtrip(steps)
    state = _mount_menu(steps, mount_uuid)
    steps.check("reimport_exact_five_emeralds_no_resurrected_diamonds",
                _cargo_exact(state, "minecraft:emerald", 5)
                and item_count(state, "minecraft:diamond") == 7
                and item_count(state, "minecraft:emerald") == 0)
    _capture(steps, "custody-mount-after-reimport")
    steps.call("minecraft_close_screen")


def _pack(steps: ProbeSteps) -> None:
    read_receipt = getattr(steps.client, "promotion_pack_receipt", None)
    if not callable(read_receipt):
        raise RuntimeError("optional pack requires the pre-join configuration wire receipt hook")
    receipt = read_receipt()
    if not isinstance(receipt, dict):
        raise RuntimeError("optional pack wire receipt must be a mapping")
    steps.report["optional_pack_wire"] = receipt
    steps.check("optional_pack_offer_and_matching_decline_before_play",
                receipt.get("offer_required") is False
                and bool(receipt.get("offer_id"))
                and receipt.get("decline_id") == receipt.get("offer_id")
                and receipt.get("action") == "declined"
                and type(receipt.get("offer_sequence")) is int
                and type(receipt.get("decline_sequence")) is int
                and type(receipt.get("play_sequence")) is int
                and 0 < receipt["offer_sequence"] < receipt["decline_sequence"] < receipt["play_sequence"])
    state = steps.observe()
    steps.check("optional_pack_client_in_play", state["in_play"] is True)
    _echo(steps, "optional_pack_ordered_play_chat")
    _capture(steps, "optional-pack-declined-play")


def run(steps: ProbeSteps, origin: tuple[int, int, int], kind: str) -> None:
    if kind not in (MOUNT_SCENARIO, PACK_SCENARIO):
        raise ValueError(f"unknown custody promotion scenario: {kind}")
    if not getattr(steps.client, "p4_window", None):
        raise RuntimeError("custody promotion requires the harness-owned windowed Xvfb client")
    if kind == MOUNT_SCENARIO:
        _mount(steps, origin)
    else:
        _pack(steps)
