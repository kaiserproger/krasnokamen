"""Ledger AUDIT-SCULK: the downward latch of a sculk sensor chain.

The row is the audit finding of the DomWorld downward latch: a note-block vibration
reaches a sculk sensor, the comparator's output runs through dust, and a copper bulb
one block *below* the dust latches lit.  `native-v2-automation-storage` covers the
sensor's active/inactive phases only; this scenario asserts the two downward bulbs.

Both verdicts are assertions after bounded client-tick waits: the chain must first
carry the vibration (dust power above zero, proving the note-block ingress really
fired), and only then the bulbs must latch.  A tree without the copper bulb's latch
tick keeps them dark while the first assertion stays green.
"""
from __future__ import annotations

from typing import Any

from .native_v2 import _block
from .probe_steps import ProbeSteps

SENSOR = "minecraft:sculk_sensor"
COMPARATOR = "minecraft:comparator"
WIRE = "minecraft:redstone_wire"
BULB = "minecraft:copper_bulb"
NOTE = "minecraft:note_block"
REDSTONE_BLOCK = "minecraft:redstone_block"
TICKS_PER_STEP = 5
WAIT_STEPS = 16


def _read(steps: ProbeSteps, point: dict[str, int]) -> dict[str, Any]:
    return steps.call("minecraft_read_block", dict(point))


def _lit(steps: ProbeSteps, point: dict[str, int]) -> str:
    return str(_read(steps, point)["properties"].get("lit", ""))


def _power(steps: ProbeSteps, point: dict[str, int]) -> int:
    try:
        return int(_read(steps, point)["properties"].get("power", "0"))
    except ValueError:
        return 0


def run(steps: ProbeSteps, origin: tuple[int, int, int]) -> None:
    x, y, z = origin
    # The circuit sits two blocks south-east of the player's own cell, as the other
    # ledger gates place their fixtures, so no fixture block replaces the player.
    sensor = {"x": x + 2, "y": y, "z": z + 2}
    comparator = {"x": sensor["x"] - 1, "y": y, "z": sensor["z"]}
    dust = {"x": sensor["x"] - 2, "y": y, "z": sensor["z"]}
    lower_bulb = {"x": dust["x"], "y": y - 1, "z": dust["z"]}
    side_dust = {"x": sensor["x"] + 1, "y": y, "z": sensor["z"]}
    side_bulb = {"x": side_dust["x"], "y": y - 1, "z": side_dust["z"]}
    note = {"x": sensor["x"], "y": y, "z": z}
    trigger = {"x": note["x"] - 1, "y": y, "z": note["z"]}

    # The world regenerates per run: build the platform and clear the circuit layer
    # so a hill, tree or water cell cannot refuse a setblock.
    steps.fixture(f"fill {x - 1} {y - 1} {z - 4} {x + 4} {y - 1} {z + 3} stone")
    steps.fixture(f"fill {x - 1} {y} {z - 4} {x + 4} {y + 1} {z + 3} air")
    steps.fixture(
        f"setblock {sensor['x']} {sensor['y']} {sensor['z']} {SENSOR}")
    steps.fixture(
        f"setblock {comparator['x']} {comparator['y']} {comparator['z']} "
        f"{COMPARATOR}[facing=east]")
    steps.fixture(f"setblock {dust['x']} {dust['y']} {dust['z']} {WIRE}")
    steps.fixture(f"setblock {lower_bulb['x']} {lower_bulb['y']} {lower_bulb['z']} {BULB}")
    steps.fixture(f"setblock {side_dust['x']} {side_dust['y']} {side_dust['z']} {WIRE}")
    steps.fixture(f"setblock {side_bulb['x']} {side_bulb['y']} {side_bulb['z']} {BULB}")
    steps.fixture(f"setblock {note['x']} {note['y']} {note['z']} {NOTE}")
    _block(steps, sensor, SENSOR)
    _block(steps, lower_bulb, BULB)
    _block(steps, side_bulb, BULB)
    steps.check("lower_bulb_starts_dark", _lit(steps, lower_bulb) == "false")
    steps.check("side_bulb_starts_dark", _lit(steps, side_bulb) == "false")

    # A redstone block beside the note block plays it; the note-block vibration is
    # the sensor's input, exactly as the unit ref drives it.
    steps.fixture(f"setblock {trigger['x']} {trigger['y']} {trigger['z']} {REDSTONE_BLOCK}")
    powered = 0
    for _ in range(WAIT_STEPS):
        powered = _power(steps, dust)
        if powered > 0:
            break
        steps.call("minecraft_wait_ticks", {"ticks": TICKS_PER_STEP})
    steps.report["sculk_chain"] = {
        "dust_power": powered,
        "sensor_phase": _read(steps, sensor)["properties"].get("sculk_sensor_phase"),
        "lower_bulb_lit": _lit(steps, lower_bulb),
        "side_bulb_lit": _lit(steps, side_bulb),
    }
    steps.check("sculk_chain_carries_the_note_vibration", powered > 0)

    def latched() -> bool:
        return (_lit(steps, lower_bulb) == "true" and _lit(steps, side_bulb) == "true")

    lit = False
    for _ in range(WAIT_STEPS):
        if latched():
            lit = True
            break
        steps.call("minecraft_wait_ticks", {"ticks": TICKS_PER_STEP})
    steps.report["sculk_chain"]["lower_bulb_lit"] = _lit(steps, lower_bulb)
    steps.report["sculk_chain"]["side_bulb_lit"] = _lit(steps, side_bulb)
    steps.check("downward_bulbs_latch_lit", lit)

    # The bulb *latches*: the note block is unpowered again, the dust has no signal,
    # and both bulbs keep their lit state.
    steps.fixture(f"setblock {trigger['x']} {trigger['y']} {trigger['z']} air")
    for _ in range(WAIT_STEPS):
        steps.call("minecraft_wait_ticks", {"ticks": TICKS_PER_STEP})
        if _power(steps, dust) == 0:
            break
    steps.check("bulbs_keep_their_latch_after_the_vibration",
                _lit(steps, lower_bulb) == "true" and _lit(steps, side_bulb) == "true")
    steps.report["limits"] = [
        "One sculk sensor, one comparator, two dust cells and the two copper bulbs one "
        "block below them; the note-block vibration is driven by a fixture redstone "
        "block, as the unit ref drives it. No calibrated sensor, amethyst retransmission, "
        "cooldown ordering or player-caused vibration.",
    ]
