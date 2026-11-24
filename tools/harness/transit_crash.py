"""Migration transit custody under SIGKILL (ADR 0023 section 6): departure, in transit, arrival.

Each cut runs the pinned release server (`KRASNOKAMEN_SERVER_BINARY`) on a fresh
world with no players and no plugins. Fixture (console only, recorded): two
force-loaded platforms at y=200; settlement TransitSource around a roofed
three-bed room, settlement TransitDest 100 blocks east; two adult villagers
bound to the source, each fed 3 bread (read back with `data get`). A native
birth forms their household (parents + child), which then leaves with
`settlement transit depart`.

Cuts are event-bound, never elapsed time:
* departure: SIGKILL as soon as the WAL batch covering the departure tick
  (the server's own `settlement_transit_departure tick=N`) is published;
* in-transit: the record became `awaiting_entry` in a published WAL image
  while TransitDest is unsimulated, `save-all` committed a checkpoint covering
  it, SIGKILL;
* arrival: TransitDest is force-loaded, SIGKILL once the batch covering the
  arrival tick (`settlement_transit_arrival tick=N`) is published.

Evidence on copies of the crash image (the crash copy itself is kept):
* durable custody: `world custody-census` (checkpoint rows plus published WAL
  as folded by offline recovery, before any entity is restored) holds every
  household member UUID exactly once, at the place the cut implies;
* WAL: the cut's batch is published, validated by `crash_wal`, and carries the
  expected transit image;
* recovery acceptance: the same binary starts `--start-frozen` at exactly the
  durable tick, stops cleanly, and `world census` of the stored rows agrees.

`--control forged-duplicate` (negative control) publishes one more batch that
re-adds a departed member as a body beside its record: the custody census must
FAIL and recovery must refuse. A custody-breaking binary must turn the plain
departure cut red.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import uuid
from pathlib import Path
from typing import Any

from . import runtime, wal_frame
from .backends.protocol_lifecycle import ProtocolServer, reserve_matrix_port
from .crash_wal import read_published

ROOT = runtime.REPO_ROOT
CUTS = ("departure", "in-transit", "arrival")
SEED = 2311
WAIT = 900.0
ANSI = re.compile(r"\x1b\[[0-9;]*m")
CHECKPOINT = re.compile(r"checkpoint committed tick=(\d+)")
BIRTH = re.compile(r"settlement_birth child=(\S+) settlement=(\d+) household=(\S+) tick=(\d+)")
DEPARTED = re.compile(
    r"Household (\S+) left in transit ([0-9a-f-]{36}) with (\d+) members; "
    r"earliest arrival at game time (\d+)\.")
FILLED = re.compile(r"Successfully filled")
# One inventory stack in a `data get entity <uuid> Inventory` reply.
STACK = re.compile(r"\{[^{}]*\}")
BREAD = re.compile(r'id: "minecraft:bread"')
COUNT = re.compile(r"count: (\d+)")
FEED_ATTEMPTS = 10
PICKUP_TICKS = 40  # fenced commands, one server tick each
# Loaded and already air (an unloaded region replies "Position is not loaded").
CLEARED = re.compile(r"Successfully filled|No blocks were changed")
SOURCE, DEST = "TransitSource", "TransitDest"
# Force-loaded platform areas (x1 z1 x2 z2), three by three chunks each.
SOURCE_AREA, DEST_AREA = (-16, -16, 31, 31), (80, -16, 127, 31)
EXIT = (4.5, 202, 5.5)
BEDS = (2, 4, 6)  # foot at (x, 202, 3), head at (x, 202, 2)
FORGED_WIRE_ID = 2_000_000_000


def clean(text: str) -> str:
    return ANSI.sub("", text)


def save_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, default=str) + "\n")


def uuid_array(value: str) -> str:
    """Vanilla `UUID:[I;a,b,c,d]` int array of one UUID."""
    number = uuid.UUID(value).int
    words = [(number >> shift) & 0xFFFFFFFF for shift in (96, 64, 32, 0)]
    return "[I;" + ",".join(str(word - (1 << 32) if word >= 1 << 31 else word) for word in words) + "]"


def bread_count(reply: str) -> int:
    """Bread held, from one `data get entity <uuid> Inventory` reply."""
    return sum(int(count[1]) for stack in STACK.findall(reply)
               if BREAD.search(stack) and (count := COUNT.search(stack)))


def last_checkpoint(log: Path) -> int:
    ticks = CHECKPOINT.findall(clean(log.read_text(errors="replace")))
    if not ticks:
        raise RuntimeError(f"no committed checkpoint in {log}")
    return int(ticks[-1])


def published_through(journal: Path, checkpoint_tick: int) -> int:
    """Highest tick of the contiguous published batches after the checkpoint (names only)."""
    spans = sorted(span for name in os.listdir(journal) if (span := wal_frame.span(name)))
    following = checkpoint_tick + 1
    for lo, hi in spans:
        if hi <= checkpoint_tick:
            continue
        if lo != following:
            break
        following = hi + 1
    return following - 1


def _transits(entries: list[dict], record_id: str):
    """`Transit` images of one record in one frame's entries, dimension envelopes included."""
    for entry in entries:
        if "Dimension" in entry:
            yield from _transits(entry["Dimension"]["entries"], record_id)
        elif (image := entry.get("Transit")) and image["id"] == record_id:
            yield image


def transit_images(wal: dict, record_id: str) -> list[dict]:
    """Published `Transit` images of one record after the checkpoint, in order."""
    found = []
    for frame in wal["frames"]:
        if frame["hi"] <= wal["checkpoint_tick"]:
            continue
        for image in _transits(frame["entries"], record_id):
            record = json.loads(image["record"]) if image["record"] is not None else None
            found.append({"tick": image["tick"], "file": frame["file"],
                          "sha256": frame["sha256"], "record": record})
    return found


def places(receipt: dict, members: list[str]) -> dict[str, list[str]]:
    """Every place each member UUID occupies in a custody receipt."""
    found: dict[str, list[str]] = {member: [] for member in members}
    for member in members:
        if member in receipt.get("villagers", {}):
            found[member].append("entity")
        for record_id, record in receipt.get("transits", {}).items():
            found[member].extend(f"transit:{record_id}" for held in record["members"] if held == member)
    return found


def custody_verdict(cut: str, receipt: dict, members: list[str], record_id: str,
                    destination: int, household: str) -> list[str]:
    """Failures of the exactly-once invariant and of the cut's expected placement."""
    failures = []
    if receipt.get("status") != "PASS":
        failures.append(f"custody census {receipt.get('status')}: {receipt.get('error')}")
        return failures
    for member, held in places(receipt, members).items():
        if len(held) != 1:
            failures.append(f"member {member} held {len(held)} times: {held}")
    if cut == "arrival":
        if receipt["transits"]:
            failures.append(f"transit records remain after arrival: {sorted(receipt['transits'])}")
        for member in members:
            body = receipt["villagers"].get(member)
            if body and (body["settlement"], body["household"]) != (destination, household):
                failures.append(f"arrived member {member} is {body}, want settlement "
                                f"{destination} household {household}")
    else:
        record = receipt["transits"].get(record_id)
        if record is None:
            failures.append(f"transit record {record_id} missing at the durable boundary")
        else:
            if sorted(record["members"]) != sorted(members):
                failures.append(f"record members {record['members']} != household {members}")
            if record["household"] != household:
                failures.append(f"record household {record['household']} != {household}")
            want = "awaiting_entry" if cut == "in-transit" else "travelling"
            if record["status"] != want:
                failures.append(f"record status {record['status']} != {want}")
    return failures


def census_verdict(cut: str, census: dict, destination_name: str) -> list[str]:
    """Stored rows after recovery and a clean stop agree with the cut."""
    if census.get("status") != "PASS":
        return [f"census {census.get('status')}: {census.get('error')}"]
    counts, failures = census["counts"], []
    want = (0, 0) if cut == "arrival" else (1, 3)
    if (counts["transit_records"], counts["transit_members"]) != want:
        failures.append(f"stored transit records/members {counts['transit_records']}/"
                        f"{counts['transit_members']} != {want}")
    if counts["settlement_orphan_members"]:
        failures.append(f"{counts['settlement_orphan_members']} orphan settlement members")
    dest = next((s for s in census["settlement_details"] if s["name"] == destination_name), None)
    expected = 3 if cut == "arrival" else 0
    if dest is None or (dest["members"], dest["household_members"]) != (expected, expected):
        failures.append(f"stored destination membership {dest} != {expected}")
    return failures


def publish_batch(journal: Path, tick: int, entries: list[dict], identity: wal_frame.Identity) -> Path:
    """Publish one complete one-tick WAL batch bound to `identity`, the world's own (`wal_frame`)."""
    payload = "".join(json.dumps(entry, separators=(",", ":")) + "\n" for entry in entries).encode()
    path = journal / wal_frame.batch_name(tick, tick)
    path.write_bytes(wal_frame.encode(tick, tick, payload, identity))
    return path


def body_entry(member: dict, wire_id: int, tick: int) -> dict:
    """A full entity post-image re-adding one stored transit member image as a body."""
    image = {key: value for key, value in member.items() if key != "ground"}
    image.update(id=wire_id, tick=tick)
    return {"Entity": image}


class Run:
    """One cut on one fresh server: fixture, departure, cut, crash copy."""

    def __init__(self, run: Path, binary: Path) -> None:
        self.dir, self.binary = run, binary
        self.server = ProtocolServer(binary=binary, run_dir=run, port=reserve_matrix_port(),
                                     vanilla_data=(ROOT / "data/vanilla").resolve(strict=True),
                                     seed=SEED, server_name="transit-crash")
        self.server.config_overrides = {"data": {"checkpoint_every_ticks": 1_000_000}}
        self.fixture: list[dict[str, str]] = []
        self.facts: dict[str, Any] = {}

    # -- console --------------------------------------------------------
    def cmd(self, text: str, expect: str | None = None) -> str:
        """One fenced fixture command; `expect` must appear in its reply."""
        reply = self.server.command(text)
        self.fixture.append({"command": text, "reply": reply.strip()})
        if expect is not None and expect not in reply:
            raise RuntimeError(f"{text!r} replied {reply.strip()!r}, want {expect!r}")
        return reply

    def until(self, text: str, accepted: re.Pattern[str], attempts: int = 600) -> str:
        """Repeat one fenced command until its reply matches (each try waits a server tick)."""
        for _ in range(attempts):
            reply = self.server.command(text)
            if accepted.search(reply):
                self.fixture.append({"command": text, "reply": reply.strip()})
                return reply
        raise RuntimeError(f"{text!r} never succeeded; last reply {reply!r}")

    def log_event(self, pattern: re.Pattern[str], offset: int = 0) -> re.Match[str]:
        process = self.server._process

        def seen():
            if process is None or process.poll() is not None:
                raise RuntimeError(f"server exited while waiting for {pattern.pattern!r}")
            return (match := pattern.search(clean(self.server.log_path.read_text(errors="replace"))[offset:])) is not None, match
        return runtime.wait_for_path_condition(self.server.log_path, WAIT, seen, process)

    def log_offset(self) -> int:
        return len(clean(self.server.log_path.read_text(errors="replace")))

    # -- fixture --------------------------------------------------------
    def platform(self, area: tuple[int, int, int, int]) -> None:
        x1, z1, x2, z2 = area
        self.cmd(f"forceload add {x1} {z1} {x2} {z2}")
        self.until(f"fill {x1} 200 {z1} {x2} 200 {z2} minecraft:stone", FILLED)
        for low, high in ((201, 208), (209, 215)):
            self.until(f"fill {x1} {low} {z1} {x2} {high} {z2} minecraft:air", CLEARED)

    def build(self) -> None:
        for rule in ("gamerule advance_time false", "gamerule spawn_mobs false",
                     "gamerule advance_weather false", "time set 1000"):
            self.cmd(rule)
        self.platform(DEST_AREA)
        self.cmd(f"execute positioned 104 201 8 run settlement create {DEST} 24", "Founded")
        self.cmd(f"forceload remove {DEST_AREA[0]} {DEST_AREA[1]} {DEST_AREA[2]} {DEST_AREA[3]}")
        self.platform(SOURCE_AREA)
        self.until("fill 0 201 0 8 204 8 minecraft:glass hollow", FILLED)
        for x in BEDS:
            self.cmd(f"setblock {x} 202 3 minecraft:red_bed[facing=north,part=foot]", "Changed")
            self.cmd(f"setblock {x} 202 2 minecraft:red_bed[facing=north,part=head]", "Changed")
        self.cmd(f"execute positioned 4 202 4 run settlement create {SOURCE} 24", "Founded")
        parents = [str(uuid.uuid4()) for _ in range(2)]
        for x, parent in zip((1.5, 6.5), parents):
            self.cmd(f"summon minecraft:villager {x} 202 5.5 {{UUID:{uuid_array(parent)}}}",
                     "Summoned")
            self.cmd(f"settlement bind {SOURCE} {parent}", "Bound")
        self.facts["parents"] = parents
        for parent in parents:
            self.feed(parent)

    def bread(self, parent: str) -> int:
        return bread_count(self.server.command(f"data get entity {parent} Inventory"))

    def feed(self, parent: str) -> None:
        """Drop 3 bread (12 food, courtship needs 12) at the parent's feet until it holds them.

        Summon NBT has no villager inventory, and any villager in reach picks
        bread up, so the other parent may take a drop; that one is repeated."""
        for _ in range(FEED_ATTEMPTS):
            self.cmd(f"execute at {parent} run summon minecraft:item ~ ~ ~ "
                     "{Item:{id:\"minecraft:bread\",count:3},PickupDelay:0}", "Summoned")
            for _ in range(PICKUP_TICKS):
                if (held := self.bread(parent)) >= 3:
                    self.fixture.append({"command": f"data get entity {parent} Inventory",
                                         "reply": f"{held} bread"})
                    return
        raise RuntimeError(f"parent {parent} never held 3 bread")

    def household(self) -> None:
        """Wait for the native birth that forms the parents' household."""
        birth = self.log_event(BIRTH)
        self.facts.update(child=birth[1], source=int(birth[2]), household=birth[3],
                          birth_tick=int(birth[4]))
        self.facts["members"] = sorted(self.facts["parents"] + [birth[1]])

    def depart(self) -> None:
        # A fresh world logs no checkpoint: commit one so every later WAL batch
        # has a known base (the departure must follow it).
        offset = self.log_offset()
        self.server.save_all()
        self.facts["base_checkpoint"] = int(self.log_event(CHECKPOINT, offset)[1])
        offset = self.log_offset()
        for _ in range(20):
            for member in self.facts["members"]:
                self.cmd(f"tp {member} {EXIT[0]} {EXIT[1]} {EXIT[2]}")
            reply = self.cmd(f"settlement transit depart {SOURCE} {self.facts['household']} {DEST}")
            if match := DEPARTED.search(reply):
                break
            if "not at the exit point" not in reply:
                raise RuntimeError(f"departure refused: {reply!r}")
        else:
            raise RuntimeError("members never stood together at the exit")
        if match[1] != self.facts["household"] or int(match[3]) != 3:
            raise RuntimeError(f"unexpected departure reply {reply!r}")
        record = match[2]
        line = self.log_event(re.compile(
            rf"settlement_transit_departure transit={record} source=(\d+) destination=(\d+).*?"
            rf"\btick=(\d+)"), offset)
        self.facts.update(record=record, arrives=int(match[4]), destination=int(line[2]),
                          departure_tick=int(line[3]))
        if int(line[1]) != self.facts["source"]:
            raise RuntimeError("departure source differs from the birth settlement")

    # -- cuts -----------------------------------------------------------
    def wait_published(self, tick: int) -> int:
        journal = self.server.world_dir / "journal"
        checkpoint = last_checkpoint(self.server.log_path)
        process = self.server._process

        def published():
            if process is None or process.poll() is not None:
                raise RuntimeError("server exited before the cut")
            through = published_through(journal, checkpoint)
            return through >= tick, through
        return runtime.wait_for_path_condition(journal / ".event", WAIT, published, process)

    def wait_image(self, wanted, refusal: re.Pattern[str] | None = None) -> int:
        """Tick of the first published `Transit` image of the record that `wanted` accepts
        (its decoded record, `None` for the arrival tombstone)."""
        journal = self.server.world_dir / "journal"
        checkpoint = last_checkpoint(self.server.log_path)
        process = self.server._process
        offset = self.log_offset()
        seen: set[str] = set()
        found: list[int] = []

        def published():
            if process is None or process.poll() is not None:
                raise RuntimeError("server exited before the transit image was published")
            if refusal and (line := refusal.search(
                    clean(self.server.log_path.read_text(errors="replace"))[offset:])):
                raise RuntimeError(f"transit refused: {line[0]}")
            # Published batch files are immutable: decode each new one once.
            for name in sorted(os.listdir(journal)):
                span = wal_frame.span(name)
                if span is None or name in seen or span[1] <= checkpoint:
                    continue
                seen.add(name)
                data = (journal / name).read_bytes()
                if self.facts["record"].encode() not in data:
                    continue
                entries = wal_frame.decode(data, span).entries()
                found.extend(image["tick"] for image in _transits(entries, self.facts["record"])
                             if wanted(json.loads(image["record"]) if image["record"] else None))
            return bool(found), found[:1]
        return runtime.wait_for_path_condition(journal / ".event", WAIT, published, process)[0]

    def cut(self, cut: str) -> None:
        if cut == "departure":
            # The server's departure tick, not the image: a custody-breaking
            # binary may publish that tick without any record.
            self.facts["cut_tick"] = self.facts["departure_tick"]
            self.facts["published_before_kill"] = self.wait_published(self.facts["departure_tick"])
        elif cut == "in-transit":
            self.facts["cut_tick"] = self.wait_image(
                lambda record: record is not None and record["status"] == "awaiting_entry")
            offset = self.log_offset()
            self.server.save_all()
            committed = int(self.log_event(CHECKPOINT, offset)[1])
            self.facts["checkpoint_before_kill"] = committed
            if committed < self.facts["cut_tick"]:
                raise RuntimeError("save-all checkpoint precedes the awaiting_entry image")
        else:
            self.cmd(f"forceload add {DEST_AREA[0]} {DEST_AREA[1]} {DEST_AREA[2]} {DEST_AREA[3]}")
            self.facts["cut_tick"] = self.wait_image(
                lambda record: record is None,
                re.compile(rf"settlement_transit_arrival_refused transit={self.facts['record']}.*"))
        self.kill()

    def kill(self) -> None:
        process = self.server._process
        os.kill(process.pid, signal.SIGKILL)
        code = process.wait(timeout=120)
        self.server._close_handles()
        if code != -signal.SIGKILL:
            raise RuntimeError(f"SIGKILL proof failed: exit {code}")
        crash = self.dir / "crash"
        crash.mkdir()
        shutil.copytree(self.server.world_dir, crash / "world")
        shutil.copy2(self.server.log_path, crash / "server.log")
        self.facts.update(exit=code, crash_copy=str(crash))

    def close(self) -> None:
        self.server.close()


def run_tool(run: Path, binary: Path, config: Path, name: str, args: list[str]) -> tuple[int, dict]:
    receipt = run / f"{name}.json"
    argv = [str(binary), "--config", str(config), "world", *args, "--receipt", str(receipt)]
    with (run / f"{name}.log").open("wb") as log:
        code = subprocess.run(argv, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT,
                              timeout=600, check=False).returncode
    return code, (json.loads(receipt.read_text()) if receipt.exists() and receipt.stat().st_size else {})


def copy_world(crash: Path, destination: Path) -> Path:
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(crash / "world", destination)
    return destination


def recover_frozen(run: Path, binary: Path, crash: Path) -> dict[str, Any]:
    """Start the binary frozen on a copy of the crash image, then stop it cleanly."""
    server = ProtocolServer(binary=binary, run_dir=run, port=reserve_matrix_port(),
                            vanilla_data=(ROOT / "data/vanilla").resolve(strict=True), seed=SEED,
                            server_name="transit-recovery")
    try:
        server.config_overrides = {"data": {"checkpoint_every_ticks": 1_000_000}}
        copy_world(crash, server.world_dir)
        config = server.write_config()
        log_path = run / "recovery.log"
        frozen = re.compile(r"recovery simulation frozen before first tick tick=(\d+)")
        with log_path.open("wb") as log:
            process = subprocess.Popen([str(binary), "--config", str(config), "--start-frozen"],
                                       cwd=ROOT, stdin=subprocess.PIPE, stdout=log,
                                       stderr=subprocess.STDOUT, start_new_session=True)
            try:
                # Readiness or exit (pidfd), never a log-only wait: a refusing
                # recovery exits after its last log line.
                try:
                    runtime.wait_port(server.port, WAIT, process, log_path=log_path)
                except runtime.ChildExited:
                    process.wait()
                    return {"refused": True, "exit": process.returncode, "config": str(config),
                            "log": str(log_path), "world": str(server.world_dir)}
                match = frozen.search(clean(log_path.read_text(errors="replace")))
                if match is None:
                    raise RuntimeError(f"recovery listens without its freeze barrier: {log_path}")
                tick = int(match[1])
                process.stdin.write(b"stop\n")
                process.stdin.flush()
                code = process.wait(timeout=WAIT)
            finally:
                runtime.stop_process(process)
        return {"refused": False, "frozen_tick": tick, "stop_exit": code, "config": str(config),
                "log": str(log_path), "world": str(server.world_dir)}
    finally:
        server.close()


def evidence(run: Path, binary: Path, config: Path, cut: str, facts: dict) -> dict[str, Any]:
    crash = Path(facts["crash_copy"])
    checkpoint = last_checkpoint(crash / "server.log")
    wal = read_published(crash / "world/journal", checkpoint)
    images = transit_images(wal, facts["record"])
    through = wal["published_through"]
    if cut == "in-transit":
        valid = checkpoint >= facts["cut_tick"] and not images
    else:
        valid = checkpoint < facts["cut_tick"] <= through
    image_ok = {
        "departure": any(i["tick"] == facts["cut_tick"] and i["record"] is not None for i in images),
        "in-transit": True,
        "arrival": any(i["tick"] == facts["cut_tick"] and i["record"] is None for i in images),
    }[cut]
    copy = copy_world(crash, run / "custody" / "world")
    code, custody = run_tool(run, binary, config, "custody", ["custody-census", "--copy", str(copy)])
    failures = custody_verdict(cut, custody, facts["members"], facts["record"],
                               facts["destination"], facts["household"])
    if custody and custody.get("durable_tick") != through:
        failures.append(f"custody durable tick {custody.get('durable_tick')} != WAL {through}")
    if custody and custody.get("checkpoint_tick") != checkpoint:
        failures.append(f"custody checkpoint tick {custody.get('checkpoint_tick')} != {checkpoint}")
    if not valid:
        failures.append(f"cut not at its boundary: checkpoint {checkpoint}, cut tick "
                        f"{facts['cut_tick']}, published through {through}, images {len(images)}")
    if not image_ok:
        failures.append(f"the cut batch lacks its transit image: {images}")
    recovery = recover_frozen(run / "recovery", binary, crash)
    census: dict = {}
    if recovery["refused"]:
        failures.append(f"recovery refused the crash image (exit {recovery['exit']})")
    else:
        if recovery["frozen_tick"] != through:
            failures.append(f"recovery froze at {recovery['frozen_tick']}, durable tick {through}")
        if recovery["stop_exit"] != 0:
            failures.append(f"recovered server stop exit {recovery['stop_exit']}")
        _, census = run_tool(run, binary, config, "census",
                             ["census", "--source", recovery["world"]])
        failures += census_verdict(cut, census, DEST)
    return {"checkpoint_at_cut": checkpoint, "published_through": through,
            "unpublished_temporary_files": wal["unpublished"], "transit_images": images,
            "moment_observed": valid, "custody_exit": code, "custody": custody,
            "places": places(custody, facts["members"]), "recovery": recovery,
            "census_counts": census.get("counts"), "failures": failures,
            "status": "passed" if not failures else "failed"}


def forged_duplicate(run: Path, binary: Path, config: Path, facts: dict) -> dict[str, Any]:
    """Negative control: one more published batch re-adds a member body beside its record."""
    crash = Path(facts["crash_copy"])
    checkpoint = last_checkpoint(crash / "server.log")
    wal = read_published(crash / "world/journal", checkpoint)
    images = [image for image in transit_images(wal, facts["record"]) if image["record"]]
    member = images[-1]["record"]["members"][0]
    forged = run / "forged"
    forged.mkdir()
    tick = wal["published_through"] + 1
    world = copy_world(crash, forged / "census" / "world")
    # The forged batch belongs to this copy under this build: take the store id,
    # branch and palette from a batch the server itself published there.
    identity = wal_frame.capture_identity(world)
    if identity is None:
        raise RuntimeError(f"no server-written WAL batch in {world} to take the forged batch's identity from")
    publish_batch(world / "journal", tick, [body_entry(member, FORGED_WIRE_ID, tick)], identity)
    code, custody = run_tool(forged, binary, config, "custody", ["custody-census", "--copy", str(world)])
    census_red = (code != 0 and custody.get("status") == "FAIL"
                  and f"member {member['uuid']} of transit record" in (custody.get("error") or ""))
    recovery_world = forged / "recovery-source"
    shutil.copytree(world.parent, recovery_world)
    recovery = recover_frozen(forged / "recovery", binary, recovery_world)
    refusal = "is both a world entity and in transit record"
    recovery_red = recovery["refused"] and recovery["exit"] != 0 and refusal in clean(
        Path(recovery["log"]).read_text(errors="replace"))
    return {"forged_tick": tick, "member": member["uuid"], "custody_exit": code,
            "custody": custody, "census_red": census_red, "recovery": recovery,
            "recovery_red": recovery_red, "detected": census_red and recovery_red}


def one(out: Path, binary: Path, cut: str, control: str | None = None) -> dict[str, Any]:
    run = out / cut
    run.mkdir(parents=True)
    session = Run(run, binary)
    record: dict[str, Any] = {"cut": cut, "binary": str(binary), "status": "failed"}
    try:
        session.server.start()
        session.build()
        session.household()
        session.depart()
        session.cut(cut)
        record["facts"] = session.facts
        record.update(evidence(run, binary, session.server.config_path, cut, session.facts))
        if control == "forged-duplicate":
            record["control"] = forged_duplicate(run, binary, session.server.config_path, session.facts)
    except Exception as error:  # the receipt names the failing step
        record["error"] = f"{type(error).__name__}: {error}"
        record["facts"] = session.facts
    finally:
        session.close()
        record["fixture_commands"] = session.fixture
        save_json(run / "cut.json", record)
    return record


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--cut", action="append", choices=CUTS,
                        help="cuts to run (default: all three, in order)")
    parser.add_argument("--control", choices=("forged-duplicate",),
                        help="negative control: forge a body beside the departure record; must be detected")
    args = parser.parse_args()
    binary = os.environ.get("KRASNOKAMEN_SERVER_BINARY")
    if not binary:
        parser.error("KRASNOKAMEN_SERVER_BINARY must name a tools/dev/pin-release binary")
    binary = Path(binary).resolve(strict=True)
    os.environ["KRASNOKAMEN_HARNESS_LOG_STDOUT"] = "1"
    args.out.mkdir(parents=True, exist_ok=False)
    if args.control:
        row = one(args.out, binary, "departure", args.control)
        detected = bool(row.get("control", {}).get("detected")) and not row.get("error")
        save_json(args.out / "negative-control.json", {"detected": detected, "row": row})
        raise SystemExit(0 if detected else 1)
    rows = [one(args.out, binary, cut) for cut in (args.cut or CUTS)]
    status = "passed" if all(row["status"] == "passed" and "error" not in row for row in rows) else "failed"
    save_json(args.out / "transit-crash.json", {"status": status, "binary": str(binary),
                                                "cuts": {row["cut"]: row for row in rows}})
    raise SystemExit(0 if status == "passed" else 1)


if __name__ == "__main__":
    main()
