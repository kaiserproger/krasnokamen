"""Read-only receipts for map pixels/markers and placed item block-entity data.

Layouts: native client src/ui/packets.rs::read_map and the local 26.1.2
packets/registries reports. The proxy forwards bytes unchanged; these readers
never inject packets or patch the client/server's observed state.
"""
from __future__ import annotations

import copy
import json
import struct
from pathlib import Path

from .light_wire import Reader


def _nbt_payload(reader: Reader, kind: int):
    formats = {1: ">b", 2: ">h", 3: ">i", 4: ">q", 5: ">f", 6: ">d"}
    if kind in formats:
        fmt = formats[kind]
        return struct.unpack(fmt, reader.take(struct.calcsize(fmt)))[0]
    if kind == 8:
        return reader.take(struct.unpack(">H", reader.take(2))[0]).decode("utf-8")
    if kind == 10:
        fields = {}
        while (child := reader.take(1)[0]) != 0:
            name = _nbt_payload(reader, 8)
            fields[name] = _nbt_payload(reader, child)
        return fields
    if kind == 9:
        child = reader.take(1)[0]
        count = reader.int32()
        if count < 0:
            raise ValueError("negative NBT list length")
        return [_nbt_payload(reader, child) for _ in range(count)]
    if kind in (7, 11, 12):
        count = reader.int32()
        if count < 0:
            raise ValueError("negative NBT array length")
        fmt = {7: ">b", 11: ">i", 12: ">q"}[kind]
        return [struct.unpack(fmt, reader.take(struct.calcsize(fmt)))[0] for _ in range(count)]
    raise ValueError(f"unknown NBT kind {kind}")


def read_nbt(reader: Reader):
    kind = reader.take(1)[0]
    return _nbt_payload(reader, kind) if kind else None


def decode_map(payload: bytes) -> dict:
    reader = Reader(payload)
    result = {"id": reader.varint(), "scale": reader.take(1)[0],
              "locked": bool(reader.take(1)[0])}
    if reader.take(1)[0]:
        count = reader.varint()
        if not 0 <= count <= 4096:
            raise ValueError("map decoration count")
        markers = []
        for _ in range(count):
            kind = reader.varint()
            x, y, rotation = struct.unpack(">bbB", reader.take(3))
            name = read_nbt(reader) if reader.take(1)[0] else None
            markers.append({"kind": kind, "x": x, "y": y,
                            "rotation": rotation & 15, "name": name})
        result["markers"] = markers
    width = reader.take(1)[0]
    if width:
        height, x, y = reader.take(3)
        colors = reader.byte_array()
        if not height or x + width > 128 or y + height > 128 or len(colors) != width * height:
            raise ValueError("map patch bounds")
        result["patch"] = {"width": width, "height": height, "x": x, "y": y,
                           "colors": list(colors)}
    if reader.offset != len(payload) or result["id"] < 0 or result["scale"] > 4:
        raise ValueError("invalid map packet tail/header")
    return result


def _signed(value: int, bits: int) -> int:
    return value - (1 << bits) if value & (1 << (bits - 1)) else value


def decode_placement(payload: bytes) -> dict:
    reader = Reader(payload)
    packed = int.from_bytes(reader.take(8), "big")
    result = {"position": [_signed(packed >> 38, 26), _signed(packed & 4095, 12),
                            _signed((packed >> 12) & 0x3FFFFFF, 26)],
              "type": reader.varint(), "nbt": read_nbt(reader)}
    if reader.offset != len(payload):
        raise ValueError("block entity packet tail")
    return result


class PromotionWireReceipt:
    def __init__(self, report_path: Path, artifact: Path) -> None:
        packets = json.loads(report_path.read_text())["play"]["clientbound"]
        self.map_id = packets["minecraft:map_item_data"]["protocol_id"]
        self.placement_id = packets["minecraft:block_entity_data"]["protocol_id"]
        registries = json.loads(report_path.with_name("registries.json").read_text())
        self.marker_ids = {name: value["protocol_id"] for name, value in
                           registries["minecraft:map_decoration_type"]["entries"].items()}
        self.artifact = artifact / "promotion-wire-receipts.json"
        self.viewer = None
        self.events: list[dict] = []
        self.maps: dict[int, dict] = {}

    def server_packet(self, wire, phase: str, packet_id: int, payload: bytes) -> None:
        if phase != "play" or packet_id not in (self.map_id, self.placement_id):
            return
        if self.viewer is None:
            self.viewer = wire
        elif self.viewer is not wire:
            raise RuntimeError("promotion wire receipt requires one graphical viewer")
        if packet_id == self.placement_id:
            try:
                event = {"packet": "block_entity_data", **decode_placement(payload)}
            except ValueError as error:
                raise ValueError(f"block entity decode {payload.hex()}: {error}") from error
        else:
            try:
                packet = decode_map(payload)
            except ValueError as error:
                raise ValueError(f"map decode {payload.hex()}: {error}") from error
            state = self.maps.setdefault(packet["id"], {
                "id": packet["id"], "colors": [0] * 16384, "markers": []})
            state.update(scale=packet["scale"], locked=packet["locked"])
            if "markers" in packet:
                state["markers"] = packet["markers"]
            if patch := packet.get("patch"):
                for row in range(patch["height"]):
                    target = (row + patch["y"]) * 128 + patch["x"]
                    source = row * patch["width"]
                    state["colors"][target:target + patch["width"]] = \
                        patch["colors"][source:source + patch["width"]]
            state["sequence"] = len(self.events)
            event = {"packet": "map_item_data", **packet}
        event["sequence"] = len(self.events)
        self.events.append(event)

    def snapshot(self, wire) -> dict:
        if self.viewer is not None and self.viewer is not wire:
            raise RuntimeError("receipt viewer differs from queried graphical connection")
        result = copy.deepcopy({"events": self.events, "maps": self.maps,
                                "marker_ids": self.marker_ids})
        self.artifact.parent.mkdir(parents=True, exist_ok=True)
        self.artifact.write_text(json.dumps(result, indent=2) + "\n")
        return result
