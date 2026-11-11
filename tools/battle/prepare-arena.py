#!/usr/bin/env python3
"""Build the stopped grass battle arena world through public server commands.

The arena replaces natural terrain in a 512x512 square centred on the world
origin with a flat grass field at natural height (grass at y=66, spawn at y=67),
a ring of gentle terraces at the rim, a camera hill, trees, boulders and sparse
grass tufts and flowers. Nothing but blocks is placed: no entity survives the
build, so the arena costs the tick nothing beyond terrain. Time is fixed at
mid-morning and the weather at clear.

The layout is a pure function of `--seed`: `plan()` returns the exact command
list and `result.json` records its SHA-256. The output world is copied by
`run-battle.py --world <run-dir>/world --world-seed <world-seed>`; `battle setup`
then stages at the world spawn, which sits on the arena centre.

    prepare-arena.py --pin /abs/server-pin-dir-or-binary --cache /abs/vanilla \\
        --run-dir /abs/new-dir [--seed 42] [--world-seed 42] [--no-decor]
"""
import argparse
import functools
import hashlib
import json
import socket
import struct
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

GRASS_Y = 66  # top grass layer of the flat field; units stand at y=67
SPAWN = (0, GRASS_Y + 1, 0)
HALF = 256  # arena covers x,z in [-HALF, HALF-1]
FLAT = 200  # nothing but flat grass within |x|,|z| < FLAT (battle bounds are +-192)
LOAD_HALF = 288  # force-loaded square: arena plus a ring cleared of natural entities
BASE_Y = 30  # dirt fill starts here; seabed or caves below are invisible
CLEAR_TOP = 162  # air up to here; verification proves nothing natural above
STEPS = 5  # rim terraces, 1 block high each
STEP_W = 11
SEGMENT = 16
HILL = {"x": 228, "z": -60, "radius": 26, "height": 12}
FILL_LIMIT = 4_194_304
DECOR_TUFTS = 1200
DECOR_FLOWERS = 240
FLOWERS = ("dandelion", "poppy", "cornflower", "oxeye_daisy", "azure_bluet")
MASK = (1 << 64) - 1


def mix(*values):
    """Deterministic 64-bit hash (splitmix64 chain), independent of the Python version."""
    x = 0x9E3779B97F4A7C15
    for value in values:
        x = (x ^ (value & MASK)) & MASK
        x = (x + 0x9E3779B97F4A7C15) & MASK
        z = x
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        x = z ^ (z >> 31)
    return x


def rim_start(seed, side, segment, step):
    """First distance from the centre line (in blocks) of a terrace on one side."""
    wobble = mix(seed, 1, side, segment, step) % 7 - 3
    return FLAT + 3 + STEP_W * (step - 1) + wobble


def side_of(x, z):
    """Yield (side, distance, segment) for each side whose terraces may cover the cell."""
    yield 0, z, (x + HALF) // SEGMENT  # north (+z)
    yield 1, -z - 1, (x + HALF) // SEGMENT  # south
    yield 2, x, (z + HALF) // SEGMENT  # east (+x)
    yield 3, -x - 1, (z + HALF) // SEGMENT  # west


def rim_level(seed, x, z):
    level = 0
    for side, distance, segment in side_of(x, z):
        for step in range(STEPS, 0, -1):
            if step <= level:
                break
            if distance >= rim_start(seed, side, segment, step):
                level = step
                break
    return level


@functools.lru_cache(maxsize=None)
def hill_layers(seed):
    """Per layer j=1..height: rectangles (half widths) approximating a rounded dome.

    The profile is a smoothstep fall-off, so the steepest slope stays near 45 degrees;
    a seeded jitter on each half width (never growing with height) keeps the dome from
    reading as concentric rings.
    """
    height, radius = HILL["height"], HILL["radius"]
    layers = []
    previous = None
    for j in range(1, height + 1):
        want = 1.0 - (j - 0.5) / height  # smoothstep value at the layer's outer edge
        lo, hi = 0.0, 1.0
        for _ in range(40):
            mid = (lo + hi) / 2
            if 3 * mid * mid - 2 * mid ** 3 > want:
                hi = mid
            else:
                lo = mid
        r = int(radius * lo + 0.5)
        if r < 2:
            break
        halves = [(r, max(1, int(r * 0.41))), (max(1, int(r * 0.41)), r), (int(r * 0.8), int(r * 0.8))]
        halves = [(a + mix(seed, 8, j, i, 0) % 3 - 1, b + mix(seed, 8, j, i, 1) % 3 - 1) for i, (a, b) in enumerate(halves)]
        halves = [(max(1, a), max(1, b)) for a, b in halves]
        if previous:
            halves = [(min(a, pa), min(b, pb)) for (a, b), (pa, pb) in zip(halves, previous)]
        previous = halves
        layers.append((j, halves))
    return layers


def hill_level(seed, x, z):
    dx, dz = abs(x - HILL["x"]), abs(z - HILL["z"])
    level = 0
    for j, rects in hill_layers(seed):
        if any(dx <= a and dz <= b for a, b in rects):
            level = j
    return level


def surface(seed, x, z):
    return GRASS_Y + max(rim_level(seed, x, z), hill_level(seed, x, z))


def place_features(seed):
    """Trees and boulders on the rim, away from the flat field, the hill and each other."""
    taken = []

    def free(x, z, radius):
        if abs(x) > HALF - 8 - radius or abs(z) > HALF - 8 - radius:
            return False
        if max(abs(x), abs(z)) - radius < FLAT:
            return False
        if (x - HILL["x"]) ** 2 + (z - HILL["z"]) ** 2 < (HILL["radius"] + radius + 2) ** 2:
            return False
        return all((x - tx) ** 2 + (z - tz) ** 2 >= (radius + tr + 1) ** 2 for tx, tz, tr in taken)

    trees, boulders, attempt = [], [], 0
    wanted = [("hero", 3, 5), ("tree", 2, 55), ("boulder", 1, 14)]
    for kind, radius, count in wanted:
        placed = 0
        while placed < count and attempt < 20000:
            attempt += 1
            x = mix(seed, 2, attempt) % (2 * HALF - 16) - (HALF - 8)
            z = mix(seed, 3, attempt) % (2 * HALF - 16) - (HALF - 8)
            if not free(x, z, radius):
                continue
            taken.append((x, z, radius))
            placed += 1
            roll = mix(seed, 4, attempt)
            if kind == "boulder":
                boulders.append((x, z, roll))
            else:
                trees.append((x, z, radius, 9 if kind == "hero" else 4 + roll % 3, "birch" if roll >> 8 & 3 == 0 else "oak"))
    return trees, boulders, taken


def fill(a, b, block):
    (x1, y1, z1), (x2, y2, z2) = a, b
    return f"fill {x1} {y1} {z1} {x2} {y2} {z2} {block}"


def plan(seed, decor=True):
    """Return [(phase, command)] for the whole arena; a pure function of (seed, decor)."""
    out = []

    def add(phase, command):
        out.append((phase, command))

    for rule, value in (
        ("max_block_modifications", FILL_LIMIT),
        ("block_drops", "false"),
        ("entity_drops", "false"),
        ("spawn_mobs", "false"),
        ("advance_time", "false"),
        ("advance_weather", "false"),
    ):
        add("rules", f"gamerule {rule} {value}")
    add("rules", "time set 3000")
    add("rules", "weather clear")
    add("rules", "difficulty normal")
    edges = [(-LOAD_HALF, -33), (-32, 223), (224, LOAD_HALF - 1)]
    for x0, x1 in edges:
        for z0, z1 in edges:
            add("load", f"forceload add {x0} {z0} {x1} {z1}")
    lo, hi = -HALF, HALF - 1
    top = BASE_Y
    while top < GRASS_Y:
        add("bulk", fill((lo, top, lo), (hi, min(top + 7, GRASS_Y - 1), hi), "minecraft:dirt"))
        top += 8
    bottom = GRASS_Y + 1
    while bottom <= CLEAR_TOP:
        add("clear", fill((lo, bottom, lo), (hi, min(bottom + 15, CLEAR_TOP), hi), "minecraft:air"))
        bottom += 16
    add("grass", fill((lo, GRASS_Y, lo), (hi, GRASS_Y, hi), "minecraft:grass_block"))
    for step in range(1, STEPS + 1):
        y = GRASS_Y + step
        for segment in range(2 * HALF // SEGMENT):
            s0 = -HALF + segment * SEGMENT
            s1 = s0 + SEGMENT - 1
            n = rim_start(seed, 0, segment, step)
            add("rim", fill((s0, y, n), (s1, y, hi), "minecraft:grass_block"))
            south = rim_start(seed, 1, segment, step)
            add("rim", fill((s0, y, lo), (s1, y, -south - 1), "minecraft:grass_block"))
            east = rim_start(seed, 2, segment, step)
            add("rim", fill((east, y, s0), (hi, y, s1), "minecraft:grass_block"))
            west = rim_start(seed, 3, segment, step)
            add("rim", fill((lo, y, s0), (-west - 1, y, s1), "minecraft:grass_block"))
    for j, rects in hill_layers(seed):
        y = GRASS_Y + j
        for a, b in rects:
            add("hill", fill((HILL["x"] - a, y, HILL["z"] - b), (HILL["x"] + a, y, HILL["z"] + b), "minecraft:grass_block"))
    trees, boulders, taken = place_features(seed)
    for x, z, roll in boulders:
        y = surface(seed, x, z) + 1
        add("boulder", fill((x - 1, y, z - 1), (x + 1, y + 1, z + 1), "minecraft:cobblestone" if roll & 1 else "minecraft:mossy_cobblestone"))
        add("boulder", f"setblock {x} {y + 2} {z} minecraft:stone")
    for x, z, radius, trunk, wood in trees:
        ys = surface(seed, x, z)
        leaves = f"minecraft:{wood}_leaves[persistent=true]"
        layers = [(trunk - 1, trunk, 2), (trunk + 1, trunk + 1, 1)] if radius == 2 else [(trunk - 2, trunk, 3), (trunk + 1, trunk + 1, 2), (trunk + 2, trunk + 2, 1)]
        for y0, y1, r in layers:
            add("tree", fill((x - r, ys + y0, z - r), (x + r, ys + y1, z + r), leaves))
        add("tree", f"setblock {x} {ys + (trunk + 2 if radius == 2 else trunk + 3)} {z} {leaves}")
        add("tree", fill((x, ys + 1, z), (x, ys + trunk, z), f"minecraft:{wood}_log"))
    if decor:
        reach = HALF - 8
        tufts = placed = attempt = 0
        while placed < DECOR_TUFTS + DECOR_FLOWERS and attempt < 100_000:
            attempt += 1
            x = mix(seed, 5, attempt) % (2 * reach) - reach
            z = mix(seed, 6, attempt) % (2 * reach) - reach
            if any((x - tx) ** 2 + (z - tz) ** 2 <= (tr + 1) ** 2 for tx, tz, tr in taken):
                continue
            roll = mix(seed, 7, attempt)
            if placed < DECOR_TUFTS:
                block = "short_grass"
            else:
                block = FLOWERS[roll % len(FLOWERS)]
            add("decor", f"setblock {x} {surface(seed, x, z) + 1} {z} minecraft:{block}")
            placed += 1
    add("finish", f"setworldspawn {SPAWN[0]} {SPAWN[1]} {SPAWN[2]}")
    return out


def verify_columns(seed, trees_taken):
    """Columns whose top block is plain grass, for the post-build probe."""
    columns = []
    for x in range(-HALF + 8, HALF - 8, 24):
        for z in range(-HALF + 8, HALF - 8, 24):
            if any((x - tx) ** 2 + (z - tz) ** 2 <= (tr + 2) ** 2 for tx, tz, tr in trees_taken):
                continue
            columns.append((x, surface(seed, x, z), z))
    return columns


def command_digest(commands):
    digest = hashlib.sha256()
    for phase, command in commands:
        digest.update(f"{phase}\t{command}\n".encode())
    return digest.hexdigest()


class Rcon:
    def __init__(self, port, password):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=60)
        self.sock.settimeout(900)
        self._send(3, password)

    def _exact(self, n):
        data = b""
        while len(data) < n:
            chunk = self.sock.recv(n - len(data))
            if not chunk:
                raise EOFError("RCON closed")
            data += chunk
        return data

    def _send(self, kind, text):
        payload = struct.pack("<ii", 1, kind) + text.encode() + b"\0\0"
        self.sock.sendall(struct.pack("<i", len(payload)) + payload)
        size = struct.unpack("<i", self._exact(4))[0]
        reply = self._exact(size)
        if struct.unpack("<i", reply[:4])[0] != 1:
            raise RuntimeError("RCON rejected the password")
        return reply[8:-2].decode(errors="replace")

    def command(self, text):
        return self._send(2, text)

    def close(self):
        self.sock.close()


BAD = ("unknown", "incorrect", "failed", "failure", "too many", "error", "invalid", "could not")


def run_command(rcon, command, log, deadline=900):
    """Run one command; wait while its chunks are unloaded; raise on any refusal."""
    end = time.monotonic() + deadline
    while True:
        reply = rcon.command(command)
        log.write(json.dumps({"command": command, "reply": reply[:200]}) + "\n")
        if "not loaded" in reply:
            if time.monotonic() > end:
                raise TimeoutError(command)
            time.sleep(0.5)
            continue
        if any(word in reply.lower() for word in BAD):
            raise RuntimeError(f"{command}: {reply[:200]}")
        return reply


def probe(rcon, command, deadline=900):
    """`execute if` probe: True for 'Test passed'; waits while the position is unloaded."""
    end = time.monotonic() + deadline
    while True:
        reply = rcon.command(command).strip()
        if reply.startswith("Test passed"):
            return True
        if reply.startswith("Test failed"):
            return False
        if "not loaded" in reply and time.monotonic() < end:
            time.sleep(0.5)
            continue
        raise RuntimeError(f"{command}: {reply[:200]}")


def verify(rcon, seed, taken):
    """Prove flat grass where expected and nothing natural above the cleared volume."""
    for x0 in range(-HALF, HALF, 128):
        for z0 in range(-HALF, HALF, 128):
            if not probe(rcon, f"execute if blocks {x0} {CLEAR_TOP + 1} {z0} {x0 + 127} {CLEAR_TOP + 64} {z0 + 127} {x0} {CLEAR_TOP + 68} {z0} all"):
                raise RuntimeError(f"natural terrain remains above y={CLEAR_TOP} near {x0},{z0}")
    for x, y, z in verify_columns(seed, taken):
        if not probe(rcon, f"execute if block {x} {y} {z} minecraft:grass_block"):
            raise RuntimeError(f"expected grass at {x} {y} {z}")
        if not probe(rcon, f"execute if block {x} {y + 3} {z} minecraft:air"):
            raise RuntimeError(f"expected air above grass at {x} {y + 3} {z}")


def free_ports(count):
    socks = [socket.socket() for _ in range(count)]
    for s in socks:
        s.bind(("127.0.0.1", 0))
    ports = [s.getsockname()[1] for s in socks]
    for s in socks:
        s.close()
    return ports


def build(run_dir, server_binary, vanilla_cache, seed=42, world_seed=42, decor=True):
    """Build `<run_dir>/world`; return the receipt dict also written to `result.json`."""
    root = Path(run_dir).resolve()
    root.mkdir(parents=True, exist_ok=False)
    binary = Path(server_binary)
    if binary.is_dir():
        binary = binary / "solaris-v2"
    network, dashboard, rcon_port = free_ports(3)
    password = "arena-fixture-only"
    (root / "server.toml").write_text(
        f'[server]\nname = "grass-arena-prep"\nview_distance = 12\nsimulation_distance = 12\npause_when_empty_seconds = 0\n'
        f'[network]\nbind_address = "127.0.0.1"\nport = {network}\n[auth]\nonline_mode = false\n'
        f'[dashboard]\nenabled = true\nbind_address = "127.0.0.1"\nport = {dashboard}\n'
        f'[rcon]\nenabled = true\nbind_address = "127.0.0.1"\nport = {rcon_port}\npassword = "{password}"\n'
        f'[data]\nworld_dir = "{root / "world"}"\nvanilla_data_dir = "{Path(vanilla_cache).resolve()}"\nseed = {world_seed}\n'
    )
    commands = plan(seed, decor)
    receipt = {"status": "failed", "seed": seed, "world_seed": world_seed, "decor": decor, "grass_y": GRASS_Y,
               "spawn": list(SPAWN), "half": HALF, "flat_half": FLAT, "hill": HILL, "commands": len(commands),
               "command_sha256": command_digest(commands), "world": str(root / "world")}
    server_log = (root / "server.log").open("w")
    process = subprocess.Popen([str(binary), "--config", str(root / "server.toml"), "--no-console"],
                               stdout=server_log, stderr=subprocess.STDOUT, cwd=root)
    started = time.monotonic()
    try:
        while True:
            if process.poll() is not None:
                raise RuntimeError("server exited before the dashboard came up")
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{dashboard}/stats", timeout=2) as response:
                    json.load(response)
                break
            except OSError:
                if time.monotonic() - started > 600:
                    raise
                time.sleep(0.5)
        rcon = Rcon(rcon_port, password)
        with (root / "commands.jsonl").open("w") as log:
            for _, command in commands:
                run_command(rcon, command, log)
            _, _, taken = place_features(seed)
            verify(rcon, seed, taken)
            if probe(rcon, "execute if entity @e[type=!minecraft:player]"):
                run_command(rcon, "kill @e[type=!minecraft:player]", log)
            if probe(rcon, "execute if entity @e[type=!minecraft:player]"):
                raise RuntimeError("entities remain after the arena build")
            run_command(rcon, "forceload remove all", log)
            run_command(rcon, "gamerule block_drops true", log)
            run_command(rcon, "gamerule entity_drops true", log)
            run_command(rcon, "gamerule max_block_modifications 32768", log)
        rcon.close()
        process.send_signal(2)  # console is disabled: SIGINT is the graceful shutdown
        if process.wait(timeout=300) != 0:
            raise RuntimeError("server did not shut down cleanly")
        receipt["status"] = "complete"
        receipt["seconds"] = round(time.monotonic() - started, 1)
    except Exception as error:
        receipt["error"] = str(error)
        raise
    finally:
        if process.poll() is None:
            process.send_signal(2)
            try:
                process.wait(timeout=120)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=30)
        server_log.close()
        (root / "result.json").write_text(json.dumps(receipt, indent=2) + "\n")
    return receipt


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--pin", type=Path, required=True, help="server pin directory or solaris-v2 binary")
    parser.add_argument("--cache", type=Path, required=True, help="vanilla data cache directory")
    parser.add_argument("--run-dir", type=Path, required=True, help="new directory; the world is written to <run-dir>/world")
    parser.add_argument("--seed", type=int, default=42, help="arena layout seed (default 42)")
    parser.add_argument("--world-seed", type=int, default=42, help="world generation seed for the surroundings (default 42)")
    parser.add_argument("--no-decor", action="store_true", help="omit grass tufts and flowers")
    parser.add_argument("--dry-run", action="store_true", help="print the command count and digest only")
    args = parser.parse_args(argv)
    if args.dry_run:
        commands = plan(args.seed, not args.no_decor)
        print(json.dumps({"commands": len(commands), "command_sha256": command_digest(commands)}))
        return 0
    receipt = build(args.run_dir, args.pin, args.cache, args.seed, args.world_seed, not args.no_decor)
    print(json.dumps(receipt, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
