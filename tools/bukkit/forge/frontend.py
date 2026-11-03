"""Fail-closed Forge BreakEvent frontend for the existing Bukkit TeaVM pipeline.

Call ``generate(jar, inventory, blocks_report, generated_dir, api=wit_version)``
AFTER running InspectMods on ``jar``. The inventory is bound to the JAR SHA-256.
The result contains ``main_class``, ``shim_sources`` and ``generated_sources``
(Paths for javac), ``manifest`` (plugin.toml text), and ``report`` (JSON-safe).
Compile these sources against the ORIGINAL JAR and TeaVM dependencies, then pass
those classes and that JAR to the existing Compile/prune/adapter/component path.
Do not include the Bukkit shim. Write the returned manifest/report only after
javac and TeaVM both succeed: generation is preflight, NOT conversion success.
No additional compiler, runtime, host import or Wasm adapter is introduced.

Supported registration is one public static, NORMAL, receiveCanceled=false
BreakEvent handler, directly annotated on a public Forge EventBusSubscriber
for the FORGE bus and DEDICATED_SERVER (or both distributions). One handler
avoids inventing same-priority discovery order. Separate subscriber classes
with static initializers are rejected because registration-time initialization
cannot be reproduced by a direct callback. Other server handlers are rejected,
not omitted. Explicit CLIENT-only handlers and declared Mixins are reported as
omissions. Only primitive/String mod fields and static helper calls are linked;
no dynamic dispatch, dynamic registration, reflection, or live-world APIs.

The native before-build hook owns player-authored non-air -> air changes only;
a cancellation vetoes the WHOLE native batch. It is not general Forge gameplay
compatibility, and does not reproduce other mods' Forge event-bus ordering.
"""

from collections import defaultdict
import hashlib
import json
from pathlib import Path
import re
from zipfile import ZipFile

from mod_metadata import inspect_mod

ROOT = Path(__file__).resolve().parent
IDENT = re.compile(r"[A-Za-z_$][A-Za-z0-9_$]*(?:\.[A-Za-z_$][A-Za-z0-9_$]*)*\Z")
BREAK = "net/minecraftforge/event/level/BlockEvent$BreakEvent"
BLOCK_EVENT = "net/minecraftforge/event/level/BlockEvent"
EVENT = "net/minecraftforge/eventbus/api/Event"
POS = "net/minecraft/core/BlockPos"
VEC = "net/minecraft/core/Vec3i"
HANDLER = f"(L{BREAK};)V"
MOD_ANNOTATION = "Lnet/minecraftforge/fml/common/Mod;"
SUBSCRIBE = "Lnet/minecraftforge/eventbus/api/SubscribeEvent;"
API_TYPES = {BREAK, BLOCK_EVENT, EVENT, POS, VEC, "java/lang/String"}


def _sha(path):
    with Path(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def _name(value):
    if not IDENT.fullmatch(value):
        raise ValueError(f"unsupported Java identifier: {value!r}")
    return value.replace("$", ".")


def _inventory(path, digest):
    rows = defaultdict(list)
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        kind, *values = line.split("\t")
        rows[kind].append(values)
    if rows["inventory"] != [["forge-static-v1"]] or rows["input"] != [[digest]]:
        raise ValueError("Forge inventory is stale or lacks forge-static-v1 evidence bound to this JAR")
    return rows


def _preflight(meta, rows, jar):
    if meta["loader"] != "forge" or len(meta["mod_ids"]) != 1:
        raise ValueError("static Forge conversion requires exactly one Forge mod identity")
    mod_id = meta["mod_ids"][0]
    if not re.fullmatch(r"[a-z][a-z0-9_]{1,63}", mod_id):
        raise ValueError(f"unsupported Forge mod id: {mod_id!r}")
    if meta["access_transformers"] or meta["embedded_jars"] or meta["class_path"]:
        raise ValueError("access transformers, embedded JARs and manifest Class-Path are unsupported")
    for dep in meta["dependencies"]:
        if dep["side"] not in (None, "BOTH", "CLIENT", "SERVER"):
            raise ValueError(f"unknown dependency side: {dep!r}")
        if dep["side"] != "CLIENT" and dep["mandatory"] is not False and dep["id"] not in {"minecraft", "forge", mod_id}:
            raise ValueError(f"missing supported server dependency: {dep['id']}")
    if len(rows["entry"]) != 1 or rows["entry"][0][0] != mod_id:
        raise ValueError("@Mod identity must exactly match the single mods.toml declaration")
    entry = rows["entry"][0][1]
    classes = {r[0]: (int(r[1]), r[2], r[3]) for r in rows["class"]}
    methods = {tuple(r[:3]): int(r[3]) for r in rows["method"]}
    annotations = defaultdict(set)
    for owner, annotation in rows["class-annotation"]:
        annotations[owner].add(annotation)
    if MOD_ANNOTATION not in annotations[entry]:
        raise ValueError("entrypoint is not annotated with Forge @Mod")
    with ZipFile(jar) as archive:
        actual_classes = {name[:-6].replace("/", ".") for name in archive.namelist() if name.endswith(".class")}
        if set(classes) != actual_classes:
            raise ValueError("inventory class evidence does not match JAR contents")
    for name in classes:
        _name(name)
        if name.startswith(("net.minecraft.", "net.minecraftforge.", "net.neoforged.", "org.solaris.", "java.", "org.teavm.")):
            raise ValueError(f"mod bundles a reserved runtime/API class: {name}")
    subscribers = {}
    for owner, identity, bus, sides in rows["subscriber"]:
        if owner in subscribers:
            raise ValueError(f"duplicate subscriber registration: {owner}")
        side_set = set(sides.split(",")) if sides else set()
        if not side_set or not side_set <= {"CLIENT", "DEDICATED_SERVER"}:
            raise ValueError(f"unknown/empty subscriber distributions: {owner}: {sides}")
        if identity not in ("", mod_id):
            raise ValueError(f"subscriber modid does not match @Mod: {owner}: {identity}")
        if bus not in ("FORGE", "MOD"):
            raise ValueError(f"unknown subscriber bus: {owner}: {bus}")
        subscribers[owner] = (bus, side_set)
    handlers, omitted = [], []
    for owner, (bus, sides) in subscribers.items():
        if sides == {"CLIENT"}:
            omitted.append({"kind": "client-only-subscriber", "class": owner})
            continue
        if bus != "FORGE":
            raise ValueError(f"unsupported server subscriber bus: {owner}: {bus}")
        if owner not in classes or classes[owner][1:] != ("java/lang/Object", ""):
            raise ValueError(f"unsupported server subscriber hierarchy: {owner}")
        if owner != entry and (owner, "<clinit>", "()V") in methods:
            raise ValueError(f"separate subscriber has a static initializer; registration-time initialization is not representable: {owner}")
        if any("distmarker/OnlyIn" in annotation for annotation in annotations[owner]):
            raise ValueError(f"side-stripped subscriber class is unsupported: {owner}")
    method_annotations = defaultdict(set)
    for owner, name, desc, annotation in rows["method-annotation"]:
        method_annotations[(owner, name, desc)].add(annotation)
    for owner, name, desc, access, priority, receive in rows["handler"]:
        key = (owner, name, desc)
        if owner not in subscribers:
            raise ValueError(f"handler has no proven static subscriber registration: {key}")
        bus, sides = subscribers[owner]
        if sides == {"CLIENT"}:
            omitted.append({"kind": "client-only-handler", "method": list(key)})
            continue
        if "DEDICATED_SERVER" not in sides or bus != "FORGE":
            raise ValueError(f"unsupported server registration: {key}: bus={bus}, sides={sorted(sides)}")
        if SUBSCRIBE not in method_annotations[key]:
            raise ValueError(f"not a Forge SubscribeEvent handler: {key}")
        access = int(access)
        if desc != HANDLER or access & 0x9 != 0x9 or access & 0x1500:
            raise ValueError(f"unsupported server handler descriptor/access: {key}")
        if priority != "NORMAL" or receive != "false":
            raise ValueError(f"unsupported priority/cancellation delivery: {key}: {priority}, receiveCanceled={receive}")
        handlers.append(key)
    if len(handlers) != 1:
        raise ValueError("exactly one supported server BreakEvent handler is required; multiple handlers have unproven registration order")
    handler = handlers[0]
    if any("distmarker/OnlyIn" in annotation for annotation in annotations[entry]):
        raise ValueError(f"side-stripped @Mod class is unsupported: {entry}")
    for owner in {entry, handler[0]}:
        if owner not in classes or classes[owner][0] & 1 == 0:
            raise ValueError(f"entry/subscriber class must be public: {owner}")
    constructor = (entry, "<init>", "()V")
    if constructor not in methods or methods[constructor] & 1 == 0:
        raise ValueError(f"@Mod must have a public no-arg constructor: {entry}")
    # Registration calls anywhere in the input are ambiguous, even if a local
    # reachability approximation would label them dead/client-only.
    for row in rows["call"]:
        owner, name = row[:2]
        if owner.startswith(("net/minecraftforge/", "net/neoforged/")) and name in {"register", "addListener", "addGenericListener", "unregister"}:
            raise ValueError(f"dynamic Forge registration is unsupported: {owner}.{name}")
    calls, fields, types = defaultdict(list), defaultdict(list), defaultdict(list)
    for row in rows["call"]:
        calls[tuple(row[3:6])].append(row)
    for row in rows["field"]:
        fields[tuple(row[3:6])].append(row)
    for row in rows["type"]:
        types[tuple(row[:3])].append(row)
    forbidden = {tuple(row[:3]) for kind in ("dynamic", "constant") for row in rows[kind]}
    declared_fields = {tuple(r[:3]): int(r[3]) for r in rows["declared-field"]}
    field_annotations = defaultdict(set)
    for owner, name, desc, annotation in rows["field-annotation"]:
        field_annotations[(owner, name, desc)].add(annotation)
    queue, visited = [constructor, handler], set()

    def initialize(owner):
        if owner not in classes:
            raise ValueError(f"missing mod class: {owner}")
        if any("distmarker/OnlyIn" in annotation for annotation in annotations[owner]):
            raise ValueError(f"side-stripped reachable class is unsupported: {owner}")
        access, parent, interfaces = classes[owner]
        if parent != "java/lang/Object" or interfaces or access & 0x6200:
            raise ValueError(f"unsupported mod class hierarchy: {owner}: {parent}, {interfaces}")
        key = (owner, "<clinit>", "()V")
        if key in methods and key not in visited:
            queue.append(key)

    while queue:
        key = queue.pop()
        if key in visited:
            continue
        visited.add(key)
        owner, name, desc = key
        initialize(owner)
        if key not in methods or methods[key] & 0x1500:
            raise ValueError(f"missing/native/abstract/synthetic reachable method: {key}")
        if name not in ("<init>", "<clinit>") and methods[key] & 8 == 0:
            raise ValueError(f"unsupported reachable instance dispatch: {key}")
        if key in forbidden:
            raise ValueError(f"unsupported dynamic invocation/class constant in reachable method: {key}")
        if any("distmarker/OnlyIn" in annotation for annotation in method_annotations[key]):
            raise ValueError(f"side-stripped reachable method is unsupported: {key}")
        if methods[key] & 0x20:
            raise ValueError(f"synchronized reachable method is unsupported: {key}")
        for ref in re.findall(r"L([^;]+);", desc):
            if ref not in API_TYPES:
                raise ValueError(f"unsupported reachable descriptor type: {key}: {ref}")
        for row in types[key]:
            opcode, target = int(row[3]), row[4]
            if opcode not in (192, 193) or target not in API_TYPES:
                raise ValueError(f"unsupported reachable type instruction: {key}: {opcode} {target}")
        for row in calls[key]:
            target, method, signature = row[:3]
            opcode = int(row[6])
            target_key = (target.replace("/", "."), method, signature)
            if target_key in methods:
                if opcode != 184:
                    raise ValueError(f"unsupported mod instance call: {key} -> {target_key}")
                queue.append(target_key)
                continue
            supported = (
                (target == "java/lang/Object" and method == "<init>" and signature == "()V" and opcode == 183)
                or (target in {BREAK, BLOCK_EVENT} and method == "getPos" and signature == f"()L{POS};" and opcode == 182)
                or (target in {BREAK, BLOCK_EVENT, EVENT} and ((method == "isCanceled" and signature == "()Z") or (method == "setCanceled" and signature == "(Z)V") or (method == "isCancelable" and signature == "()Z")) and opcode == 182)
                or (target in {POS, VEC} and method in {"getX", "getY", "getZ"} and signature == "()I" and opcode == 182)
            )
            if not supported:
                raise ValueError(f"unsupported/missing reachable API: {key} -> {target}.{method}{signature}")
        for row in fields[key]:
            target, field, signature = row[:3]
            field_key = (target.replace("/", "."), field, signature)
            if field_key not in declared_fields or signature not in {"Z", "B", "C", "S", "I", "J", "F", "D", "Ljava/lang/String;"}:
                raise ValueError(f"unsupported/missing reachable field: {key} -> {field_key}")
            if any("distmarker/OnlyIn" in annotation for annotation in field_annotations[field_key]):
                raise ValueError(f"side-stripped reachable field is unsupported: {field_key}")
            initialize(field_key[0])
    omitted.extend({"kind": "mixin", "declaration": mixin} for mixin in meta["mixins"])
    return mod_id, entry, handler, omitted, sorted(visited)


def _states(report):
    blocks = json.loads(Path(report).read_text(encoding="utf-8"))
    ids, air = set(), set()
    for block, value in blocks.items():
        for state in value["states"]:
            ident = state["id"]
            if type(ident) is not int or not 0 <= ident <= 2147483647 or ident in ids:
                raise ValueError("invalid or duplicate deployment block state id")
            ids.add(ident)
            if block in {"minecraft:air", "minecraft:cave_air", "minecraft:void_air"}:
                air.add(ident)
    if not all(name in blocks for name in ("minecraft:air", "minecraft:cave_air", "minecraft:void_air")) or not air:
        raise ValueError("deployment report must contain all native air blocks")
    ranges = []
    for ident in sorted(ids):
        if ranges and ranges[-1][1] + 1 == ident:
            ranges[-1][1] = ident
        else:
            ranges.append([ident, ident])
    checks = "\n".join(f"if (id >= {lo} && id <= {hi}) return;" for lo, hi in ranges)
    return """package org.solaris.forge;
final class States {
    static void validate(int id) {
        CHECKS
        throw new IllegalArgumentException("unknown deployment block state");
    }
    static boolean air(int id) { return AIR; }
}
""".replace("CHECKS", checks).replace("AIR", " || ".join(f"id == {ident}" for ident in sorted(air)))


def generate(jar, inventory, blocks_report, generated_dir, *, api):
    """Generate Java inputs and native manifest; see module integration contract.

    Raises ValueError on unsupported metadata/registration/reachable bytecode.
    Does not run commands or compile, and never copies mod Java source. Paths in
    the result are absolute. Caller must compile original mod bytecode, preserve
    the report (including omissions), and refuse any javac/TeaVM linkage errors.
    """
    jar, inventory, blocks_report = map(Path, (jar, inventory, blocks_report))
    digest = _sha(jar)
    meta = inspect_mod(jar)
    rows = _inventory(inventory, digest)
    mod_id, entry, handler, omissions, reachable = _preflight(meta, rows, jar)
    version = meta["versions"][mod_id]
    if not version or "${" in version:
        raise ValueError("Forge version must be resolved in mods.toml before conversion")
    if not isinstance(api, str) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", api):
        raise ValueError("api must be the deployment's numeric WIT package version")
    source = """package org.solaris.forge;
import org.teavm.interop.Export;
import net.minecraft.core.BlockPos;
import net.minecraftforge.event.level.BlockEvent.BreakEvent;
public final class EntryPoint {
    private static MAIN mod;
    private static boolean constructed;
    private static boolean enabled;
    public static void main(String[] args) {}
    @Export(name = "enable") public static void enable() {
        if (enabled) throw new IllegalStateException("Forge mod already enabled");
        if (!constructed) {
            mod = new MAIN();
            constructed = true;
        }
        enabled = true;
    }
    @Export(name = "disable") public static void disable() {
        if (!enabled) throw new IllegalStateException("Forge mod is not enabled");
        enabled = false;
    }
    @Export(name = "deliver") public static void deliver(int kind, long session, int argc) {
        throw new IllegalArgumentException("Forge mod owns no delivery events or commands");
    }
    @Export(name = "break_block") public static int breakBlock(long session, int oldState, int newState, int x, int y, int z) {
        if (!enabled) throw new IllegalStateException("Forge mod is not enabled");
        if (session == 0) throw new IllegalArgumentException("BreakEvent requires a player-authored change");
        States.validate(oldState);
        States.validate(newState);
        if (States.air(oldState) || !States.air(newState)) return 0;
        BreakEvent event = new BreakEvent(new BlockPos(x, y, z));
        OWNER.METHOD(event);
        return event.isCanceled() ? 1 : 0;
    }
}
"""
    bindings = {"MAIN": _name(entry), "OWNER": _name(handler[0]), "METHOD": _name(handler[1])}
    source = re.sub(r"\b(?:MAIN|OWNER|METHOD)\b", lambda match: bindings[match[0]], source)
    states = _states(blocks_report)
    manifest = f'id = {json.dumps(mod_id.replace("_", "-"))}\nname = {json.dumps(mod_id)}\nversion = {json.dumps(version)}\napi = {json.dumps(api)}\nplayer_commands = []\nhooks = ["before-build"]\n'
    generated = Path(generated_dir).resolve()
    generated.mkdir(parents=True, exist_ok=True)
    sources = []
    for name, text in (("EntryPoint.java", source), ("States.java", states)):
        target = generated / name
        target.write_text(text, encoding="utf-8")
        sources.append(target)
    return {
        "main_class": "org.solaris.forge.EntryPoint",
        "shim_sources": sorted((ROOT / "shim").rglob("*.java")),
        "generated_sources": sources,
        "manifest": manifest,
        "report": {
            "frontend": "forge-static-break-event", "jar_sha256": digest,
            "blocks_sha256": _sha(blocks_report), "api": api, "mod_id": mod_id,
            "entrypoint": entry, "handler": list(handler), "reachable_methods": [list(key) for key in reachable],
            "omissions": omissions, "minecraft_constraints": meta["minecraft_constraints"],
            "semantics": "player-authored non-air->air only; cancellation vetoes the entire before-build batch; no live world/player access or synchronous effects",
            "compatibility": "supported callback translation only, not Forge/Minecraft version compatibility",
        },
    }
