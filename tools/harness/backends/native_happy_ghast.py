"""Ordinary harness custody and four independent native riders, viewed graphically."""
from __future__ import annotations

import copy
import json
from pathlib import Path

from .light_wire import Reader
from .native_p1 import _aim, _capture
from .native_v2 import _echo, _give
from .probe_steps import ProbeSteps, item_count


class PassengerReceipt:
    def __init__(self, report: Path, artifact: Path):
        packets = json.loads(report.read_text())["play"]["clientbound"]
        self.packet = packets["minecraft:set_passengers"]["protocol_id"]
        self.events = []
        self.artifact = artifact / "happy-ghast-passengers.json"

    def server_packet(self, wire, phase, packet, payload):
        if phase != "play" or packet != self.packet:
            return
        reader = Reader(payload)
        vehicle, count = reader.varint(), reader.varint()
        if not 0 <= count <= 4:
            raise ValueError(f"unbounded passenger count {count}")
        passengers = [reader.varint() for _ in range(count)]
        if reader.offset != len(payload):
            raise ValueError("set_passengers trailing bytes")
        self.events.append({"vehicle": vehicle, "passengers": passengers})

    def snapshot(self, wire):
        result = copy.deepcopy(self.events)
        self.artifact.write_text(json.dumps(result, indent=2) + "\n")
        return result


def run(steps: ProbeSteps, origin: tuple[int, int, int]):
    x, y, z = origin
    steps.fixture(f"summon minecraft:happy_ghast {x + 1.5} {y} {z + 3.5} {{NoAI:1b}}")
    _echo(steps, "ghast_fixture_created")
    ghast = steps.call("minecraft_wait_for_visible_entity", {
        "entity_type": "minecraft:happy_ghast", "radius": 12,
        "timeout_seconds": steps.remaining()})
    identity = {key: ghast[key] for key in ("entity_id", "entity_uuid", "entity_type")}

    def passengers():
        events = steps.client.happy_ghast_passengers()
        rows = [event["passengers"] for event in events if event["vehicle"] == identity["entity_id"]]
        return rows[-1] if rows else []

    steps.call("minecraft_interact_entity", identity)
    _echo(steps, "bare_ghast_interaction_processed")
    steps.check("no_harness_no_riders", passengers() == [])
    _give(steps, "minecraft:red_harness")
    steps.call("minecraft_set_hotbar_slot", {"slot": 0})
    steps.call("minecraft_interact_entity", identity)
    steps.wait(lambda state: item_count(state, "minecraft:red_harness") == 0,
               "harness_attached_exactly_once")
    steps.call("minecraft_interact_entity", identity)
    steps.wait(lambda state: len(passengers()) == 1, "first_rider_received")
    peers = steps.client.start_ghast_riders()
    expected = list(passengers())
    for index, peer in enumerate(peers, 2):
        name = f"GhastRider{index}"
        steps.fixture(f"tp {name} {x + 1.5} {y} {z + 1.0}")
        _echo(steps, f"{name}_teleport_processed")
        peer.call_tool("minecraft_wait_for_play", {"timeout_seconds": steps.remaining()})
        peer.call_tool("minecraft_interact_entity", identity)
        marker = f"{name} mount interaction processed"
        peer.call_tool("minecraft_send_chat", {"message": marker, "command": False})
        steps.wait(lambda state: marker in "\n".join(state["recent_chat"]), f"{name}_mount_processed")
        if index <= 4:
            steps.wait(lambda state: len(passengers()) == index, f"rider_{index}_received")
            current = passengers()
            steps.check(f"rider_{index}_distinct_and_keeps_others",
                        len(set(current)) == index and set(expected).issubset(current))
            expected = current
    # The fifth attempt has no positive event to wait for, and the public NBT
    # view hides krasnokamen:Riders. The fourth rider's dismount below is a later
    # event in the same ordered passenger stream, so a fifth passenger id could
    # only appear before it; the check after it proves refusal.
    _capture(steps, "four-riders-mounted")
    peers[2].call_tool("minecraft_press_inputs", {"keys": ["sneak"], "ticks": 1})
    steps.wait(lambda state: len(passengers()) == 3, "fourth_rider_dismounted")
    rows = [event["passengers"] for event in steps.client.happy_ghast_passengers()
            if event["vehicle"] == identity["entity_id"]]
    steps.check("fifth_rider_refused_before_ordered_dismount",
                all(set(row) <= set(expected) for row in rows))
    steps.check("other_riders_retained", set(passengers()).issubset(expected))
    steps.call("minecraft_press_inputs", {"keys": ["sneak"], "ticks": 1})
    steps.wait(lambda state: len(passengers()) == 2, "primary_rider_dismounted")
    _aim(steps, ghast)
    _capture(steps, "harness-and-remaining-riders")
    steps.report["passenger_packets"] = steps.client.happy_ghast_passengers()
