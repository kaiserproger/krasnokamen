# Plugin converter

## Converting Bukkit and Forge input (experimental)

### Converter support at a glance

| Input | `--inspect-mod` | Conversion |
|---|---|---|
| Bukkit/Spigot plugin JAR (`plugin.yml`) | n/a | A small API slice (below): join event, break event, manifest commands. |
| Forge mod JAR (`META-INF/mods.toml`) | Metadata and bytecode inventory | One server `BlockEvent.BreakEvent` handler (`--convert-mod`). |
| NeoForge mod JAR (`META-INF/neoforge.mods.toml`) | Metadata and bytecode inventory | **Not implemented.** |
| Fabric mod JAR (`fabric.mod.json`) | Metadata (entrypoints) and bytecode inventory | **Not implemented.** |

For every input, Mixins and access transformers are never applied, client-only
code is never converted, and nothing runs mod code at inspection time. A
converted package is unproven until you have exercised it; none of these paths
is CraftBukkit, Spigot, Paper, Forge or Fabric compatibility in general.

### How the converter works

`tools/bukkit/convert.py` is an **explicit offline converter**, not a JVM server
backend. It takes an ordinary Bukkit/Spigot plugin JAR, reads `plugin.yml` and
annotated handlers without executing plugin classes, compiles reachable Java
bytecode with TeaVM 0.10, and links it with a Rust SDK adapter into a normal WIT
component. Deploy only the resulting `plugin.toml` and `plugin.wasm` using the
[package directory layout](OPERATIONS.md#operator-deployment). The ordinary server build and runtime do not require
Java, Maven, TeaVM, Binaryen, or an additional Cargo feature. Nothing loads JARs
automatically; native plugins and core gameplay remain unchanged.

This is a small API compatibility slice, **not CraftBukkit, Spigot or Paper
compatibility in general**. There is no CraftServer/NMS implementation, embedded
JVM, Java agent, or second Wasmtime backend.

### Bukkit/Spigot plugin JARs

Supported surface:

- `JavaPlugin.onEnable/onDisable`, `getLogger().info`, `getServer()`,
  `Bukkit.getServer/getPluginManager`, and `PluginManager.registerEvents`.
- Manifest-declared lowercase commands, `getCommand(name).setExecutor`,
  `CommandExecutor.onCommand` (including the JavaPlugin override), command name,
  label and argument array. A false result emits the declared usage, if any.
- `PlayerJoinEvent.getPlayer`, `Player`/`CommandSender.getName`,
  `Player.getUniqueId`, and `sendMessage(String)`. Messages become ordinary
  session-addressed WIT commands; they do not write directly to a socket.
- Public instance `@EventHandler` methods for `PlayerJoinEvent` and
  `BlockBreakEvent`, ordered by `EventPriority`, with `ignoreCancelled`.
  Listener classes must be public and available in the input JAR. This is
  statically generated dispatch, not reflection-based event discovery.
- `BlockBreakEvent.getBlock/getPlayer/isCancelled/setCancelled`, immutable
  block coordinates and `getType`, and block-material enum constants generated
  from the deployment's vanilla `reports/blocks.json`.

The break mapping is deliberately narrower than Bukkit: a player-authored
**non-air to air** cell in a native `before-build` batch creates an event.
It also covers non-mining removals represented that way; it does not distinguish
the action cause. A cancellation rejects the **whole native batch**, including
paired cells. Placements and non-player actors do not create a break event.
There are no XP/drop controls or world mutation methods. The precommit snapshot
has UUID/session but no player name: asking for that name throws. Logging or
messaging inside the hook traps rather than leaking effects from a rejected
build. Native admission, frozen authorization and the native tick still decide
whether an allowed operation commits.

Use the exact same block report for conversion and deployment. The output
`conversion.json` records SHA-256 of the input JAR and report for the operator;
the runtime cannot authenticate that report against native state. Reconvert
after changing the vanilla cache. Unknown/custom state IDs fail closed, but a
different registry reusing an existing ID cannot be detected by this subset.

Unsupported: dependencies/soft dependencies, aliases, permission metadata,
configuration/resources, scheduler/threads, networking/filesystem, reflection,
dynamic class loading, other events, arbitrary Bukkit APIs and Paper/NMS calls.
Unsupported metadata/events are rejected explicitly; unavailable reachable
methods fail Java/TeaVM compilation or component linking. A successful
conversion does not certify unexercised plugin behavior. Fatal Java exceptions,
fuel/memory limits and invalid output use the existing host failure path.
The transport caps a string at 8192 bytes and a returned batch at 256 messages;
ordinary host/native validation may impose tighter bounds.

### Inspecting Forge and Fabric input

The same converter can inspect a mod JAR before conversion:

```sh
python3 tools/bukkit/convert.py --inspect-mod \
  --java-home /path/to/jdk17 --maven /path/to/mvn \
  --jar /path/to/mod.jar --output /path/to/inspection
```

This offline command writes `mod-inspection.json` and `mod-references.tsv`. The
report identifies loader metadata, declared dependencies, entrypoints, access
transformers and Mixin configurations; the TSV inventories direct method and
field instructions, dynamic call sites and their direct bootstrap method handles,
Forge/NeoForge `@Mod` entrypoints and annotated event handlers. A handler
annotation does not prove registration. The inventory includes client and
unreachable classes; it does not resolve nested dynamic constants, reflection,
dynamic class loading or embedded JARs. No mod classes are run.

For `call` and `field` TSV rows the columns are record type, target owner,
target name and descriptor, caller class, caller method and descriptor, JVM
opcode, method interface-target flag (false for fields), and count within that
caller. Dynamic and handler rows identify their caller directly; they are static
evidence, not a runtime trace.

Inspection **does not convert mods** or apply Mixins and does not establish
compatibility, source-version support or equivalent behavior. Declared Mixins
are reported, not silently treated as supported. Inspection requires Python 3.11+
and the existing Maven/ASM dependencies cached locally; it does not download them.

### Converting a Forge mod (`--convert-mod`)

`--convert-mod` uses the same offline TeaVM compiler, Rust adapter, WIT component
encoder and normal plugin host as the Bukkit path; it does not install Forge or
start a JVM in the server. Supply the same build inputs and target blocks report
as below, replacing the plugin JAR and adding `--convert-mod`. Conversion first
builds the same bytecode inventory as `--inspect-mod`, bound to the JAR's SHA-256.

The only supported Forge slice is **one** statically registered, public static,
`NORMAL` priority, `receiveCanceled = false` `BlockEvent.BreakEvent` handler,
annotated directly on a public `@Mod.EventBusSubscriber` for the Forge bus on
`Dist.DEDICATED_SERVER` (or both distributions). The callback may read block
position coordinates, inspect or set cancellation, and use primitive or string
mod fields and static helpers. Only player-authored non-air-to-air changes are
mapped, and a cancellation rejects the whole native build batch. The generated
package declares the `before-build` hook and no commands or events. This is not
a general Forge API, Forge event-bus ordering, or cross-version compatibility
claim.

Conversion is refused (not partially applied) for: a JAR that is not a Forge
mod with exactly one mod identity; a mod version that is missing or an unresolved
`${...}` placeholder; access transformers, embedded JARs or a manifest
`Class-Path`; a mandatory server dependency other than `minecraft` and `forge`;
a bundled `net.minecraft`, `net.minecraftforge` or `net.neoforged` class; more
than one server handler or any other server handler or registration;
dynamic registration, reflection, dynamic invocation or class constants,
synchronized or native reachable methods, and any reachable API outside the small
shim. Fabric and NeoForge conversion are **not implemented**: a NeoForge or
Fabric JAR can only be inspected.

Explicit omissions, written to `conversion.json` (`omissions`) rather than
executed: declared **Mixins** and handlers or subscriber classes that are
**client-only**. Inspect them before deployment; a successful conversion does
not imply the mod remains useful without them.


### Offline build and receipt

Required local tools: JDK **17**, Python 3 with PyYAML, Maven 3.9, Binaryen **123**
(`wasm-opt`, `wasm-merge`), Rust with `wasm32-unknown-unknown`, and the normal
Solaris dependencies. Maven resolves the pinned TeaVM dependencies in
`tools/bukkit/pom.xml`. The offline compiler toolchain is **not a sandbox**;
convert trusted input only (compiler extensions may execute build-time code).
The converter does not intentionally invoke the plugin lifecycle. Build the
adapter and existing component encoder from the **same Solaris checkout**:

```sh
cargo build --manifest-path tools/bukkit/adapter/Cargo.toml --target wasm32-unknown-unknown
cargo build -p mc-plugin-host --example plugin_component
python3 tools/bukkit/convert.py \
  --java-home /path/to/jdk17 --maven /path/to/mvn \
  --binaryen-bin /path/to/binaryen-version_123/bin \
  --jar /path/to/plugin.jar --blocks-report data/vanilla/reports/blocks.json \
  --adapter tools/bukkit/adapter/target/wasm32-unknown-unknown/debug/solaris_bukkit_adapter.wasm \
  --component-encoder target/debug/examples/plugin_component --output /tmp/converted-plugin
```

On shared agent machines, run Cargo through the repository's memory slot runner
and point `--adapter`/`--component-encoder` at that runner's target directory.
The converter strips debug sections from the adapter before linking; otherwise
a debug-built adapter can exceed the normal host component-size limit.
The converted manifest declares join subscriptions, command roots and the
build hook; no compiled-in plugin list is added.

`tools/bukkit/fixture` is an ordinary plugin source with logging, `/hello`, a
join greeting, and diamond-ore cancellation. `build_fixture.py --java-home ...
--spigot-api /path/to/upstream-spigot-api.jar --output /tmp/bukkit-receipt --
<converter arguments except --java-home, --jar, --output>` compiles it against
the **upstream API**, never the shim, then uses the same generic converter.
Run the opt-in in-process host test against its output:

```sh
SOLARIS_BUKKIT_PACKAGE=/tmp/bukkit-receipt/package \
SOLARIS_BUKKIT_BLOCKS=data/vanilla/reports/blocks.json \
  cargo test -p mc-plugin-host --test bukkit_component -- --ignored
python3 -m unittest discover -s tools/bukkit -p 'test_*.py'
```

The test exercises normal package validation, isolated configure, lifecycle,
UTF-8 join/command messages with exact session targets, native precommit
keep/cancel decisions and unknown-state retirement. It is not a real-client
or full third-party-plugin compatibility test. Upstream API JARs, Mojang
reports, generated Java/Wasm and downloaded tools remain local artifacts.
