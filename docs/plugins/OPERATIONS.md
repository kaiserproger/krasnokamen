# Plugin operations

## Operator deployment

The configured plugin root contains one directory per package:

```text
plugins/
`-- my-plugin/
    |-- plugin.toml
    |-- plugin.wasm
    `-- config.toml       # optional
```

`plugin.toml` declares the package identity, API version, subscriptions,
command roots, requested capabilities, and optional client, world-generation,
or precommit declarations. `plugin.wasm` is a WebAssembly **component** for
`solaris:plugin@0.8.0`, not an unencoded core Wasm module. `config.toml` is
optional opaque package configuration: the host passes its text to the guest,
which defines and validates its own settings.
The host opens configuration relative to the package directory, refuses symlinks
and non-regular files (including FIFOs), and bounds the bytes actually read by
`PluginLimits::text_bytes` (8192 by default). Only a missing file means empty
configuration; unsafe, unreadable, oversized or non-UTF-8 files refuse loading.

A configured plugin directory that does not exist yet (the first start from
`example.toml`) is created empty and logged; the server then runs without
plugins.

### Native server

`solaris-v2` uses the production component host with a directory-only deployment:

```toml
[plugins]
directory = "plugins"
```

```sh
solaris-v2 --config server.toml
```

Every entry must be a valid package directory; malformed manifests or WASM,
duplicate identities/command roots, failed startup, and unsupported startup
declarations refuse startup rather than skip a package. Selecting this directory
authorizes its manifest-declared ordinary capabilities. The retired deployment
strict/expected/grants/reload and explicit hook-roster settings are not supported;
unknown configuration keys warn and are ignored. Every manifest-declared
`before-build` hook registers automatically in deterministic package-id order
with fail-closed (`deny`) policy. Unsupported hooks, including `before-damage`,
fail startup before component compilation or native world admission. There is
no server `--check`: startup performs real validation before serving.

The v2 edge obtains a native read-only build plan, asks the production component
host outside the tick, then submits a single-use frozen authorization. One
queue-inclusive 100 ms deadline covers preview and the host handler chain,
ending when the host answers. A timely answer may wait for the next native
merge without expiring; consumption still rechecks roster/source validity,
single-use, native state and plugin-KV versions. Click-time orientation and ray
origin stay frozen; current reach, occupancy and support still validate.
A native-input FIFO fence prevents previews from overtaking
earlier movement or hotbar changes. An idle native owner can retire metadata
fences and answer ready read-only previews on an explicit wake without creating
an extra tick or renewing the deadline.
Denial, timeout, unavailable/retired handlers and stale native or
plugin-KV guards repair client block predictions and inventory without creating
a world edit or block-WAL record; the tick never waits for a guest.

The covered native operations match the old build-hook call sites: player block
breaks and placements (including paired/construction cells), bone-meal growth,
successful composter fills, clicked/raycast bucket edits, and server-owned block
edits (`SetWorldBlock`, plugin/NPC physical structure portions, and NPC harvest,
tree-cut, mine and replant work). Ordinary interactions, tools such as hoes or
ignition, environmental simulation and damage are not build hooks; protected
zones (below) cover them natively instead.
Hook-free deployment keeps the direct native path.

Deploying an auth-capable package (manifest capability `auth`) enables the
native session gate by itself; `[auth].require_plugin_login = true` also makes
startup fail when no such package is deployed. The gate denies gameplay until
an authentication plugin unlocks that session. Until then only `/register`,
`/login` and `/changepassword` run; every other command and chat line is
refused with a short Russian/English hint and nothing is broadcast. `UnlockSession`
requires the explicit manifest capability `auth`; observing `player.joined`
alone grants no unlock authority. Of the first-party packages only the optional
`solaris-auth` (not part of the standard pack) declares `auth`. It does not
register a `before-build` hook: the native login gate is the single login-lock
enforcement path.

Registered commands are advertised in the actual client Commands tree and
dispatch to the loaded component. For player sources, a manifest-owned root takes
precedence over an otherwise available native root, including its manifest permission in the
tree, help and completions. Its immutable deployment reservation survives guest
retirement: unavailable admission returns an error, never native-command fallback.
Native aliases not claimed by a manifest retain their native behavior.
The console has no player context and uses native commands rather than these
player-only reservations; for example, Essentials does not shadow console `msg`.
Plugin KV and native effects share v2's native checkpoint and tick WAL; there are
no separate plugin state files. The journal worker maintains the committed plugin
image and serializes it for checkpoints outside the tick. Operation callbacks and
command replies follow successful journal append completion, not fsync, and an
unflushed crash tail can consistently roll back an acknowledged effect.

Operation-id receipts remain durable for the world's lifetime. A recorded id
cannot execute again: retries return its original outcome and conflicting reuse
is refused, including after native checkpoint/WAL recovery. There is no age/count
eviction or id-reuse horizon. Receipt memory and checkpoint size can therefore
grow with cumulative operation ids; serialization is off the tick, but retention
remains intentionally unbounded rather than weakening exactly-once effects.

Inventory reservations retain at most 4096 active or structure-bound references,
plus the latest 256 unbound terminal references. Committed structure-bound
references remain pinned while their structure needs them; capacity exhaustion
refuses new admission with `Busy`. Unbound terminal references leave a bounded
FIFO and cease to resolve after eviction. Custody checks use native source
indexes, and mutations journal individual reservation records rather than a
whole historical ledger.

Observations have a separate 256-event queue per registered plugin. A full queue
drops the newest observation and increments
`ScriptBoundary::observation_events_dropped(plugin_id)`; round-robin dispatch
prevents one observer from owning the whole observation backlog. Durable native
completion callbacks and command replies are not shed: exhaustion of their
retained capacity fails closed. Timers advance only from acknowledged native
ticks. Optional Loader bundles use the existing configuration
manifest/artifact/ACK protocol; sound requires that session's actual Loader
readiness and the bundle's sound permission.


## Protected zones

`upsert-protected-zone` (capability `zones`) gives a plugin-owned box a native
allow-list of up to 32 player UUIDs plus a `public-use` flag; `upsert-zone` is the
event-only variant. The native owners enforce the list inside the box on every
tick path, with no guest call and no deadline: block break/place, buckets,
bone meal and every block use (so opening chests, barrels, furnaces and other
containers, and ignition by flint and steel or fire charges, are refused),
interaction with entities (item frames, armor stands, animals, villagers, boats,
minecarts, horses), and damage from a player's melee, sweep, mace or projectile to
anything but a hostile mob (PvP included). Operators are always admitted.
Explosion blasts spare every protected block and fire (spread from fire blocks or
lava) never enters one, whoever caused them. A zone with `public-use` additionally
admits everyone to doors, trapdoors, fence gates, buttons and levers, and only
those. Refusals repair client prediction without a world edit. An unlisted
hopper or piston still moves items and blocks across a boundary; claims guard
players and environmental fire and blasts, not redstone machinery. A zone
is keyed by its owning plugin, replaced atomically by re-sending the command, and
persists in the native checkpoint and WAL.

## Death graves

The manifest capability `death_graves` is a deployment surface rather than a
command grant: it changes what the native death path does for every player, so an
operator authorizes it by deploying the package that declares it. With the
capability deployed and `keepInventory` off, a player death moves the exact
dropped stacks into a `minecraft:barrel` placed at the death spot in the same
tick, journaled with the rest of the world; the packs a barrel's 27 slots cannot
hold are dropped beside it, so nothing is lost when a full inventory does not fit.

The placement tries the death cell, the cell above it, then a ring of radius 1–2
at the death level, one above and one below. A cell inside another package's
protected zone is skipped, so a grave never appears in a claim. Every candidate
must also stand on full-block collision ground that is not already a hopper: a
slab or a rail is not enough, because the remaining space in that cell is what a
hopper minecart needs. The full cell is what the zone can keep full, so no hopper
block, rail or minecart can be placed or rolled under the grave later, and an
existing hopper directly beneath it already pulls. A death in mid-air or over a
fluid therefore moves the grave to the nearest grounded ring cell, and when no
free grounded cell exists the drops stay ordinary items. Vanishing-curse stacks are destroyed by the
death before custody, exactly as in vanilla, and experience stays with the
player, because Solaris spawns no death XP orbs. The `keepInventory` gamerule wins
over the declaration: a retained inventory never reaches a grave.

The server then publishes `player.grave_created` to the declaring package only
(the event is targeted, like `inventory.menu.clicked`), carrying the owner, the
dimension, the grave cell and how many stacks went in and spilled. Product policy
- protecting the grave for its owner, announcing the coordinates and making it
public after a timer - belongs to that package and uses the zone owner above. One
deployment may declare the capability once; a second declaring package refuses
startup.

The first-party `graves` package protects the grave for its owner through the
zone owner above and removes that protection after `public_after_seconds`
(default 300, range 1..=604800). Custody is verifiable on a saved world without
starting the server: stop it normally, then run
[`world probe`](../OPERATING.md#probing-a-stopped-world). The harness profile
`graves` runs the whole scenario (real protocol death, clean stop, probe,
fail-closed assertions that the `grave-<x>-<y>-<z>` barrel holds exactly the
given stacks, that the stored player inventory keeps no copy and that no item
entity remains): `python3 -m tools.harness run graves -- --plugins-dir DEPLOYED_PACK`.

## Diagnosing a deployment

Startup validates manifests, components, API/capability/event names, duplicate
identities and command roots, client artifacts and startup answers. There is
no separate deployment `--check`; `content verify` checks only the vanilla cache.
Do not confuse it with plugin or gameplay validation.

At runtime, act on traps, budget failures and admission refusals. A failed
component is not a successful degraded deployment: owned command roots remain
reserved and unavailable, never native-command fallbacks. Repair or explicitly
remove the package and restart with a valid deployment.

Ordinary callbacks have a 50,000,000-unit guest fuel allowance and a separate
1,200-epoch hang guard (approximately 30 seconds with the production 25 ms
ticker). Fuel bounds guest instructions, not host scheduling delay. The hang
guard includes time inside native imports and descheduling; it interrupts at
the next Wasm check, not inside blocking native code. In particular, native
Argon2id password work retains its fixed 19 MiB, two-pass, single-lane cost and
does not consume guest fuel. Precommit hooks still obey their short absolute
chain deadline rather than the ordinary callback allowance.

With required plugin login, the configured deployment must contain an
`auth`-capable package. Every such provider is required: retirement logs the
package identity, failure and `restart required` at ERROR once. Pending and new
unauthenticated sessions are disconnected with an authentication-unavailable
reason; already authenticated players keep playing and native ticks continue.
The login fence stays enabled and a trapped Store is never retried. Inspect
the preceding callback error to distinguish fuel exhaustion from the wall-clock
hang guard, repair the package or host capacity, then restart the server with
the same world and account data. Do not disable required login as a recovery
shortcut. Other retired components retain the isolated command-unavailable
behavior above.

