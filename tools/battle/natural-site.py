#!/usr/bin/env python3
"""Prepare a stopped native battle world on unedited generated terrain.

`prepare` pregenerates the site and its surroundings with the pinned server
(`world pregenerate`, the serving generator), then starts that server once to
fix the clock and weather by game rules, remove every non-player entity from
the pregenerated square, and set the world spawn (the console battle origin)
to the site centre. No block is placed or removed. The world is stopped, its
entity census is recorded, and every file gets a SHA-256 line.

`map` renders the `battle_site_scan detail` columns (`columns.bin`,
`stats.json`) as a top-down shaded heightmap PNG with water, trees and the
field outline.
"""

import argparse
import hashlib
import json
import socket
import struct
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

TIME_OF_DAY = 3000
# `mob_drops` is what `battle setup` sets anyway; `entity_drops` is off only while
# the square is cleared, so a killed chest minecart leaves no item entities.
RULES = (
    ("spawn_mobs", "false"),
    ("advance_time", "false"),
    ("advance_weather", "false"),
    ("mob_drops", "false"),
    ("entity_drops", "false"),
)
KILL_PASSES = 3
BAD = ("unknown", "incorrect", "failed", "failure", "error", "invalid", "could not")


def plan(center_x, center_z, radius, spawn_y):
    """Every console command of the preparation, in order; a pure function."""
    commands = [f"gamerule {rule} {value}" for rule, value in RULES]
    commands += [f"time set {TIME_OF_DAY}", "weather clear", "difficulty normal"]
    lo_x, hi_x = center_x - radius, center_x + radius
    lo_z, hi_z = center_z - radius, center_z + radius
    # forceload takes at most 256 chunks per call; 240 blocks touch at most 16 chunks per axis.
    step = 240
    for x0 in range(lo_x, hi_x + 1, step):
        for z0 in range(lo_z, hi_z + 1, step):
            commands.append(f"forceload add {x0} {z0} {min(x0 + step - 1, hi_x)} {min(z0 + step - 1, hi_z)}")
    commands.append(f"setworldspawn {center_x} {spawn_y} {center_z}")
    return commands


def chunk_probes(center_x, center_z, radius):
    """One `execute if block` probe per chunk of the square: refused while the chunk is unloaded."""
    lo_cx, hi_cx = (center_x - radius) // 16, (center_x + radius) // 16
    lo_cz, hi_cz = (center_z - radius) // 16, (center_z + radius) // 16
    return [f"execute if block {cx * 16 + 8} 319 {cz * 16 + 8} minecraft:air"
            for cx in range(lo_cx, hi_cx + 1) for cz in range(lo_cz, hi_cz + 1)]


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


def run_command(rcon, command, log):
    reply = rcon.command(command)
    log.write(json.dumps({"command": command, "reply": reply[:300]}) + "\n")
    if any(word in reply.lower() for word in BAD):
        raise RuntimeError(f"{command}: {reply[:300]}")
    return reply


def probe(rcon, command, log, deadline=900):
    """True for 'Test passed'; a 'not loaded' reply is retried until the chunk loads."""
    end = time.monotonic() + deadline
    while True:
        reply = rcon.command(command).strip()
        if reply.startswith("Test passed"):
            return True
        if reply.startswith("Test failed"):
            return False
        if "not loaded" in reply and time.monotonic() < end:
            time.sleep(0.2)  # the chunk load has no RCON notification; bounded by `deadline`
            continue
        log.write(json.dumps({"command": command, "reply": reply[:300]}) + "\n")
        raise RuntimeError(f"{command}: {reply[:300]}")


def free_ports(count):
    socks = [socket.socket() for _ in range(count)]
    for s in socks:
        s.bind(("127.0.0.1", 0))
    ports = [s.getsockname()[1] for s in socks]
    for s in socks:
        s.close()
    return ports


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def checksum_list(world):
    lines = []
    for path in sorted(p for p in world.rglob("*") if p.is_file()):
        lines.append(f"{sha256_file(path)}  {path.relative_to(world)}")
    return "\n".join(lines) + "\n"


def prepare(a):
    root = a.run_dir.resolve()
    root.mkdir(parents=True, exist_ok=False)
    binary = a.pin / "solaris-v2" if a.pin.is_dir() else a.pin
    world = root / "world"
    network, dashboard, rcon_port = free_ports(3)
    password = "natural-site-prep-only"
    (root / "server.toml").write_text(
        f'[server]\nname = "natural-site-prep"\nview_distance = 8\nsimulation_distance = 8\npause_when_empty_seconds = 0\n'
        f'[network]\nbind_address = "127.0.0.1"\nport = {network}\n[auth]\nonline_mode = false\n'
        f'[dashboard]\nenabled = true\nbind_address = "127.0.0.1"\nport = {dashboard}\n'
        f'[rcon]\nenabled = true\nbind_address = "127.0.0.1"\nport = {rcon_port}\npassword = "{password}"\n'
        f'[data]\nworld_dir = "{world}"\nvanilla_data_dir = "{a.cache.resolve()}"\nseed = {a.world_seed}\n'
    )
    commands = plan(a.center_x, a.center_z, a.radius, a.spawn_y)
    receipt = {"status": "failed", "world": str(world), "world_seed": a.world_seed,
               "center": [a.center_x, a.center_z], "radius": a.radius, "spawn": [a.center_x, a.spawn_y, a.center_z],
               "time_of_day": TIME_OF_DAY, "rules": dict(RULES), "commands": commands,
               "server_binary": str(binary.resolve()), "server_sha256": sha256_file(binary)}
    started = time.monotonic()
    try:
        with (root / "pregenerate.log").open("w") as log:
            pregen = subprocess.run([str(binary), "--config", str(root / "server.toml"), "world", "pregenerate",
                                     "--radius", str(a.radius), f"--center={a.center_x},{a.center_z}"],
                                    stdout=log, stderr=subprocess.STDOUT, cwd=root)
        receipt["pregenerate_seconds"] = round(time.monotonic() - started, 1)
        if pregen.returncode != 0:
            raise RuntimeError(f"world pregenerate exited {pregen.returncode}; see pregenerate.log")
        server_log = (root / "server.log").open("w")
        process = subprocess.Popen([str(binary), "--config", str(root / "server.toml"), "--no-console"],
                                   stdout=server_log, stderr=subprocess.STDOUT, cwd=root)
        try:
            # The dashboard has no readiness push; its first answer is the start signal.
            while True:
                if process.poll() is not None:
                    raise RuntimeError("server exited before the dashboard came up")
                try:
                    with urllib.request.urlopen(f"http://127.0.0.1:{dashboard}/stats", timeout=2) as response:
                        json.load(response)
                    break
                except OSError:
                    if time.monotonic() - started > 900:
                        raise
                    time.sleep(0.5)
            rcon = Rcon(rcon_port, password)
            with (root / "commands.jsonl").open("w") as log:
                for command in commands[:-1]:
                    run_command(rcon, command, log)
                probes = chunk_probes(a.center_x, a.center_z, a.radius)
                for command in probes:
                    probe(rcon, command, log)
                receipt["chunks_loaded"] = len(probes)
                receipt["entities_before_kill"] = probe(rcon, "execute if entity @e[type=!minecraft:player]", log)
                receipt["kill_replies"] = []
                # A drop that slips through (container contents) is an entity of the next pass.
                while probe(rcon, "execute if entity @e[type=!minecraft:player]", log):
                    if len(receipt["kill_replies"]) == KILL_PASSES:
                        raise RuntimeError(f"entities remain after {KILL_PASSES} kill passes")
                    receipt["kill_replies"].append(run_command(rcon, "kill @e[type=!minecraft:player]", log))
                run_command(rcon, "gamerule entity_drops true", log)
                receipt["rules"]["entity_drops"] = "true"
                run_command(rcon, commands[-1], log)
                run_command(rcon, "forceload remove all", log)
                receipt["time_query"] = run_command(rcon, "time query daytime", log)
            rcon.sock.close()
            process.send_signal(2)  # --no-console: SIGINT is the graceful shutdown
            if process.wait(timeout=300) != 0:
                raise RuntimeError("server did not shut down cleanly")
        finally:
            if process.poll() is None:
                process.send_signal(2)
                try:
                    process.wait(timeout=120)
                except subprocess.TimeoutExpired:
                    process.terminate()
                    process.wait(timeout=30)
            server_log.close()
        census = subprocess.run([str(binary), "--config", str(root / "server.toml"), "world", "census",
                                 "--source", str(world), "--receipt", str(root / "census.json")],
                                capture_output=True, text=True, cwd=root)
        (root / "census.txt").write_text(census.stdout + census.stderr)
        receipt["census_exit"] = census.returncode
        (root / "world.sha256").write_text(checksum_list(world))
        receipt["world_sha256_list"] = str(root / "world.sha256")
        receipt["status"] = "complete"
        receipt["seconds"] = round(time.monotonic() - started, 1)
    except Exception as error:
        receipt["error"] = str(error)
        raise
    finally:
        (root / "result.json").write_text(json.dumps(receipt, indent=2) + "\n")
    return receipt


F_WATER, F_LAVA, F_TREE, F_OTHER = 1, 2, 4, 16


def load_columns(scan_dir):
    """`battle_site_scan detail` output: stats plus row-major ground heights and flags."""
    stats = json.loads((scan_dir / "stats.json").read_text())
    area = stats["area"]
    width, depth = area["width"], area["depth"]
    raw = (scan_dir / "columns.bin").read_bytes()
    if len(raw) != width * depth * 5:
        raise ValueError(f"columns.bin has {len(raw)} bytes, expected {width * depth * 5}")
    ground = [struct.unpack_from("<h", raw, i * 5)[0] for i in range(width * depth)]
    flags = raw[4::5]
    return stats, width, depth, ground, flags


def prefix_sums(values, width, depth):
    sums = [0] * ((width + 1) * (depth + 1))
    for z in range(depth):
        row = 0
        for x in range(width):
            row += values[z * width + x]
            sums[(z + 1) * (width + 1) + x + 1] = sums[z * (width + 1) + x + 1] + row
    return sums


def rect_sum(sums, width, x0, z0, x1, z1):
    """Sum over columns [x0, x1) x [z0, z1)."""
    w = width + 1
    return sums[z1 * w + x1] - sums[z0 * w + x1] - sums[z1 * w + x0] + sums[z0 * w + x0]


def refine(scan_dir, field_width, field_depth, stride, top):
    """Rank every field placement inside the generated area by its real columns.

    Placements free of water, lava and non-natural blocks come first. Within
    each group the score is steep neighbour steps (>= 2 blocks) plus 400 per
    block of p1..p99 ground range plus 200 per block of the middle half's full
    range (both every 4th column) plus a quarter of the tree columns.
    """
    stats, width, depth, ground, flags = load_columns(scan_dir)
    area = stats["area"]
    wet = [1 if f & (F_WATER | F_LAVA) else 0 for f in flags]
    other = [1 if f & F_OTHER else 0 for f in flags]
    tree = [1 if f & F_TREE else 0 for f in flags]
    steep = [0] * (width * depth)
    for z in range(depth):
        for x in range(width):
            i = z * width + x
            if x + 1 < width and abs(ground[i + 1] - ground[i]) >= 2:
                steep[i] = 1
            if z + 1 < depth and abs(ground[i + width] - ground[i]) >= 2:
                steep[i] = 1
    sums = {name: prefix_sums(values, width, depth) for name, values in
            (("wet", wet), ("other", other), ("tree", tree), ("steep", steep))}
    results = []
    for z0 in range(0, depth - field_depth + 1, stride):
        for x0 in range(0, width - field_width + 1, stride):
            x1, z1 = x0 + field_width, z0 + field_depth
            count = {name: rect_sum(s, width, x0, z0, x1, z1) for name, s in sums.items()}
            sample = sorted(ground[z * width + x] for z in range(z0, z1, 4) for x in range(x0, x1, 4))
            p1, p99 = sample[len(sample) // 100], sample[len(sample) * 99 // 100]
            # Where the lines meet: the middle half of the field on both axes.
            middle = sorted(ground[z * width + x]
                            for z in range(z0 + field_depth // 4, z1 - field_depth // 4, 4)
                            for x in range(x0 + field_width // 4, x1 - field_width // 4, 4))
            cx, cz = area["x0"] + x0 + field_width // 2, area["z0"] + z0 + field_depth // 2
            results.append({"cx": cx, "cz": cz, "steep_columns": count["steep"], "p1_p99_range": p99 - p1,
                            "range": sample[-1] - sample[0], "middle_range": middle[-1] - middle[0],
                            "tree_columns": count["tree"],
                            "water_or_lava_columns": count["wet"], "non_natural_columns": count["other"],
                            "centre_ground_y": ground[(z0 + field_depth // 2) * width + x0 + field_width // 2]})
    results.sort(key=lambda r: (r["water_or_lava_columns"] + r["non_natural_columns"] > 0,
                                r["steep_columns"] + 400 * r["p1_p99_range"] + 200 * r["middle_range"]
                                + r["tree_columns"] // 4))
    return results[:top]


def render_map(scan_dir, output, scale=1):
    from PIL import Image, ImageDraw

    stats, width, depth, ground, flags = load_columns(scan_dir)
    area = stats["area"]
    margin = area["margin"]
    lo, hi = stats["ground_p1"] - 2, max(stats["ground_p99"] + 2, stats["surroundings"]["top_max"] // 2)
    lo, hi = min(lo, stats["ground_min"]), max(hi, stats["ground_max"])
    image = Image.new("RGB", (width, depth))
    pixels = image.load()
    for z in range(depth):
        for x in range(width):
            i = z * width + x
            g = ground[i]
            t = max(0.0, min(1.0, (g - lo) / max(1, hi - lo)))
            # Light from the north-west: brighter where the ground rises towards it.
            west = ground[i - 1] if x > 0 else g
            north = ground[i - width] if z > 0 else g
            shade = max(0.55, min(1.35, 1.0 + 0.18 * ((g - west) + (g - north))))
            base = (int(70 + 150 * t), int(150 - 40 * t), int(60 + 60 * t))
            colour = tuple(max(0, min(255, int(c * shade))) for c in base)
            f = flags[i]
            if f & 1:
                colour = (40, 90, 220)
            elif f & 2:
                colour = (240, 90, 20)
            elif f & 16:
                colour = (255, 0, 255)
            elif f & 4:
                colour = (20, 70, 25)
            pixels[x, z] = colour
    draw = ImageDraw.Draw(image)
    fx, fz = margin, margin
    draw.rectangle([fx, fz, fx + stats["field"]["width"] - 1, fz + stats["field"]["depth"] - 1], outline=(255, 255, 255))
    cx, cz = stats["centre"][0] - area["x0"], stats["centre"][2] - area["z0"]
    draw.ellipse([cx - 3, cz - 3, cx + 3, cz + 3], outline=(255, 0, 0), width=2)
    if scale != 1:
        image = image.resize((width * scale, depth * scale), Image.NEAREST)
    image.save(output)
    return {"png": str(output), "size": list(image.size), "height_colour_range": [lo, hi]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="mode", required=True)
    p = sub.add_parser("prepare", help="pregenerate and prepare the stopped world")
    p.add_argument("--pin", type=Path, required=True, help="server pin directory or solaris-v2 binary")
    p.add_argument("--cache", type=Path, required=True, help="vanilla content cache")
    p.add_argument("--run-dir", type=Path, required=True, help="new directory; the world is <run-dir>/world")
    p.add_argument("--world-seed", type=int, required=True)
    p.add_argument("--center-x", type=int, required=True)
    p.add_argument("--center-z", type=int, required=True)
    p.add_argument("--spawn-y", type=int, required=True, help="first air block above the centre's ground")
    p.add_argument("--radius", type=int, default=368, help="pregenerated block radius on both axes")
    p.add_argument("--dry-run", action="store_true")
    m = sub.add_parser("map", help="render battle_site_scan detail columns as a PNG")
    m.add_argument("--scan-dir", type=Path, required=True)
    m.add_argument("--output", type=Path, required=True)
    m.add_argument("--scale", type=int, default=1)
    r = sub.add_parser("refine", help="rank field placements inside a detail area by its real columns")
    r.add_argument("--scan-dir", type=Path, required=True)
    r.add_argument("--width", type=int, default=420)
    r.add_argument("--depth", type=int, default=340)
    r.add_argument("--stride", type=int, default=8)
    r.add_argument("--top", type=int, default=10)
    a = parser.parse_args(argv)
    if a.mode == "map":
        print(json.dumps(render_map(a.scan_dir, a.output, a.scale)))
        return 0
    if a.mode == "refine":
        for row in refine(a.scan_dir, a.width, a.depth, a.stride, a.top):
            print(json.dumps(row))
        return 0
    if a.dry_run:
        print("\n".join(plan(a.center_x, a.center_z, a.radius, a.spawn_y)))
        return 0
    print(json.dumps(prepare(a), indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
