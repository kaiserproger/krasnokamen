# Executable matrix adapters

This is an extension of the canonical harness, **not a second runner or DSL**.
`python3 -m tools.harness run protocol-matrix` uses the shared process lifecycle,
receipt writer and release-binary option. Java/native profiles use the same
`matrix_execution.run_subject(MatrixContext)` primitive with their own real
client factories. YAML selects a subject/context; it never contains Python,
shell fragments or arbitrary executable steps.

## Domain contract

`tools/harness/matrix_domains/<domain>.py` exports
`ADAPTERS = {"domain.subject": Adapter(setup, act, oracle)}`. Import `Adapter`,
`Check`, `MatrixContext`, `MissingCapability` from `..matrix_api`.

- `setup(ctx)` builds finite, observable fixtures. It must not override the
  selected mode, dimension, difficulty or operator level.
- `act(ctx)` sends ordinary client input. Call **`ctx.boundary()` exactly once**
  after the first acknowledged state-changing input and before completing the
  action. This lets the shared driver apply disconnect/reconnect, save/restart,
  direct SIGKILL, backup or pause at a real semantic boundary. The call is a no-op
  for normal/steady contexts. A nonsteady adapter that omits it is blocked.
- `oracle(ctx)` returns a nonempty `list[Check(name, passed: bool, detail)]`.
  Every required observation must be present. All checks must pass; a packet
  write, fixture console echo, elapsed timeout or missing observation is not a
  successful oracle. Generic login/lifecycle probes never cover subject rows.
- `ctx.state` is adapter-owned per-run state. `ctx.row`, `ctx.factors`,
  `ctx.clients`, `ctx.server`, `ctx.artifact_dir` are supplied by the driver.
- `ctx.wait_until(predicate, description=..., timeout_seconds=30)` takes a
  **zero-argument predicate** and waits on received packet/state notifications.
  Never sleep, retry-mint fixtures or busy-poll. Server-only waits use server
  console/log event fences instead.

See `block_place.py` for the concrete finite-stock placement example.

## Observations and verbs

The exact client/server method signatures are in `matrix_api.py`. Inventory
`slot` numbers use **window-0 protocol slots**: hotbar 36..44, main 9..35,
armor 5..8, offhand 45. A menu's sparse `slots` use that menu's protocol indexes;
`slot_count` gives full capacity. Menu contents already include the player
inventory suffix, so do not add both to a conservation total. Item IDs are
namespaced registry strings. Unknown components/fields remain unavailable,
not empty/default data. Drivers may expose richer received observations.

Optional domain verbs must use `getattr` with a callable check and raise
`MissingCapability("client:<verb>")` if absent. Missing rich snapshot fields
raise `MissingCapability("snapshot:<field>")`. Never swallow those errors or
manufacture a passing fallback. Packet writes do not claim action completion;
the domain waits for its received-state oracle.

`server.command(text)` / `query(text)` return exact fresh console feedback.
The server facade scopes world commands to the scenario dimension. Fixture
commands use console authority; **the tested action must use client authority**
except a scenario whose permission context explicitly is console.
`execute if block ... run say <unique marker>` can prove a block condition;
a bare condition or the harness's echo cannot. On the original frozen binary,
`data get entity`, general `data get block`, and `execute store` are unavailable;
these must block until Main supplies a binary containing those commands.

## Context and lifecycle ownership

Before server start, drivers call
`matrix_execution.prepare_access(server_dir, names, permission)` to write an
isolated `ops.json` with exact levels (vanilla `/op` only grants level 4).
After real clients join, `run_subject(ctx)` applies difficulty/mode/dimension,
loads the adapter and enforces oracle/boundary shape. The profile is responsible
for checking every returned check and persisting it in its canonical receipt.

All profiles may reuse `backends.protocol_lifecycle.ProtocolServer` with
`port_range=(min,max)` (default RunProto 26100..26199; Java262xx/native263xx).
It provides command/query/save_all/clean_restart/kill9_restart/hot_backup/
restore_proof/dashboard_stats/mark_boundary/wait_empty_pause. SIGKILL requires
exit -9 and reopens the same world; backups verify manifest hashes and a restored
native store. World directories, plugin copies, access files and logs are per
artifact. Restart logs append; they are never truncated.

## Canonical invocation and evidence

With the protocol worktree as cwd (all paths below are on the games disk):

```sh
SOLARIS_VALIDATION_ROOT=/run/media/kaiserroman/games/solaris-work/scenarios/runs/RunProto \
/home/kaiserroman/solaris/.analysis/tools/mem-run.sh -m 6G \
python3 -m tools.harness run protocol-matrix \
  --server-bin /run/media/kaiserroman/games/solaris-work/targets/Hotfix/release/solaris-v2 \
  --plugins-dir /run/media/kaiserroman/games/solaris-work/scenarios/live-plugins \
  --vanilla-data /run/media/kaiserroman/games/solaris-work/solaris/data/vanilla \
  --matrix /run/media/kaiserroman/games/solaris-work/scenarios/matrix.yaml \
  --matrix-id '<exact stable id from gen_matrix.py --show>'
```

`--subject block_place.full_block` is an explicit supplemental adapter probe
(default ordinary survival/overworld/non-op/one player), **not** a matrix row
receipt and must not be entered in the matrix ledger. `--capability <name>`
likewise exercises a named primitive, never a matrix subject by substitution.

`result.json` is the canonical receipt; `protocol-result.json`, server.log and
wire JSONL provide detailed evidence even when an adapter raises. Append ledger
lines only from finished receipts, with full server SHA, exact stable matrix ID,
absolute run directory and real pass/fail/blocked status. Use the shared
`scenarios/gen_matrix.py --record-file` validator/locked append, never rewrite
ledger lines. Missing capabilities are blocked, not skipped/pass.

## Readiness, JSON state and plugin lanes

Before world fixture writes use
`ctx.server.ensure_loaded(((min_x,min_y,min_z),(max_x,max_y,max_z)))`.
It marks the region forced, then uses the driver's **received chunk packets**
as a residency fence. Protocol binds `ensure_loaded_callback` to the real
client observer and, when needed, fixture-teleports through the region before
returning to the original position. Java/native bind an equivalent real-client
signal or raise `MissingCapability("server:chunk_loaded_signal")`. A
`forceload add` feedback line alone is not this fence. Current protocol
`snapshot.loaded_chunks` and `wait_for_chunk(position)` expose only actual
current-dimension received chunks; unload/respawn invalidates them.

`ctx.state` and every `Check.detail` must be JSON values (dict/list/string/
number/bool/null); no `Check` objects, callbacks, Paths, bytes, clients or sets.
The shared executor validates this contract and fails before receipt writing.
Store a path as `str(path)` and checks as plain dictionaries, not objects.

Default package selection is the complete supplied live set. A row may declare
`exclude_plugins: [graves]`; a domain may use
`Adapter(setup, act, oracle, exclude_plugins=("graves",))`; a supplemental probe
may use `--exclude-plugin graves`. Selection uses actual `plugin.toml` **IDs**
(computer's ID is `solaris`), not guessed directory names. Unknown exclusions
fail. Shared `matrix_execution.copy_plugins` makes a private copy and returns
effective IDs. Profiles record enabled/excluded IDs, expose
`ctx.state["enabled_plugins"]` and `ctx.server.enabled_plugins`, and ledger notes
must preserve that lane identity. Vanilla death and full-set graves custody are
distinct lanes; never describe one as the other.

For plugin-sensitive matrix rows, the ID's `@vanilla`/`@live` suffix and
`context.plugin_profile` instead select the exact package set from
`matrix.ledger_contract.plugin_profiles[subject]`. Shared
`select_plugin_profile(row, contract, adapter_exclusions=(), requested_exclusions=())`
returns `(excluded_ids, expected_fingerprint)`. Exact profiles override adapter
defaults, so `@live` cannot silently inherit an adapter's graves exclusion.
Contradictory explicit exclusions fail. Canonical protocol receipts include
`plugin_set`, computed over the private deployed manifest and entry-WASM bytes
by `plugin_set_fingerprint(path)` and checked against the authoritative profile.
The ledger requires that field for profile-suffixed IDs, including blocked
attempts; package selection/fingerprinting precedes an unsupported adapter error.

`server.reconfigure(overrides, plugins_dir=None)` applies explicit scalar core
TOML section fixtures and restarts the same world. Adapters disconnect clients
before it and reconnect afterwards; they must not change selected context
factors. Package-specific configuration remains in the private package copy.
`dashboard_full_stats()` exposes raw `/stats`; the smaller `dashboard_stats()`
continues to provide `simulation_tick`, integer `players`, and `uptime_secs`.

Input submission is not completion. There is no general click/trade/button ACK.
Wait on the exact expected inventory/cursor/menu/block/feedback state before
`ctx.boundary()`. `snapshot.revision` is a received-packet revision, not a success
counter; unrelated updates cannot stand in for the expected effect. While a
menu is open, protocol inventory uses its fresh 36-slot player suffix instead
of the stale window-0 cache. Supported received stack component patches are
preserved; unknown patches and unreceived block states remain unavailable.

`client.set_auto_auth(False)` disables only the protocol driver's automatic
register/login commands for subsequent reconnects. It does not disable the
server auth plugin or bypass its lock. Authentication-negative adapters must
set it before reconnecting, then submit and observe their own credentials.

Protocol `use_item_on` and `break_block` return the submitted prediction sequence.
Protocol snapshots expose `block_ack_sequence` from actual `BlockChangedAck`
(`0x04`), initially `None`. Waiting for that sequence fences processed input,
including rejected/no-change interactions, but never proves accepted placement
or mining success. Keep a separate exact world/inventory oracle. Mining
`phase="complete"` only submits start/stop; it does not simulate tool-dependent
hold timing. Block observations include both single `BlockUpdate` and batched
`SectionBlocksUpdate` deltas; neither invents unreceived initial chunk contents.
Drivers unable to expose the real request sequence return `None`. ACK-dependent
adapters must reject a missing/noninteger token or missing ACK field with
`MissingCapability("block_ack_sequence")`, never compare `None` or substitute
another driver's generic ACK-version counter.

## Concurrent actor coverage

N8 rows and explicit concurrent multiplayer triples exercise all N actors
on their own targets and on a shared target, with separate domain state oracles:

```python
ctx.concurrently(own_actions, target="own")
ctx.concurrently(shared_actions, target="shared")
```

Each action is `(client, verb_name, kwargs)`. The driver hook receives
`same_tick=False` by default: release is coordinated and each exact consumed
input must be identified, but ordinary concurrent actors may cross core ticks.
The two explicit same-tick dupe origins and `triple:multiplayer_x_same_block`
instead require `same_tick=True` on both batches. A singleton batch without
`target` may fence an ordinary exact input; it does not cover multiplayer.
Ordinary N2 rows instead have one actor and an independent observer. Their
domain oracle must check the observer's received result; they do not require
the observer to mutate its own or the shared target.

Only actual driver receipts for every distinct scenario actor register these
targets; setting a public state flag cannot satisfy `run_subject`. A passing
oracle on such a row without both batches is blocked as `concurrent_subject_actors`. Proven
product failures remain failures rather than being hidden by incomplete actor
coverage. Composite dupe/same-block adapters also need their origin-specific
oracle and `state["composite_origin_verified"] = row["origin"]`; concurrency
alone is not proof of item conservation, world success, or observer agreement.

## Received components, offers and chat

Protocol stacks carry `components` (actual added patch keyed by namespaced
component ID) and `removed` (actual removed component IDs). Supported additions
are custom data/name, maximum damage, damage, lore, enchantments and potion
contents. An empty patch is observed absence of overrides, not a claim that all
vanilla prototype defaults are absent. Unknown additions fail closed for that
menu rather than discard a component or poison unrelated packet observation.

Received merchant packets populate `menu.offers` with `input1`, optional
`input2`, full `output`, use/max-use/XP/price counters, plus menu-level `level`,
`xp`, `show_progress`, and `can_restock`. Item IDs are registry names; component
enchantment/potion IDs remain numeric wire IDs, not guessed names.

Resolve those numeric IDs through `snapshot()["registries"][registry][str(id)]`.
Each registry is an ID-string-to-namespaced-name mapping, never an assumed
registration order. Dynamic registries (including enchantments) come from
received configuration RegistryData entry names, even when KnownPacks omits
their NBT payloads. Potion and effect mappings come from the supplied vanilla
registry report. `registry_sources[registry]` distinguishes
`received_configuration` from `vanilla_report`; received mappings take
precedence. Missing registries or IDs are unavailable, not a different item.

`chat_components` preserves decoded literal/translation structures. `events`
contains clean literal text or structured translation JSON, never raw NBT
decoded as UTF-8 or fabricated localized wording. `observation_errors` reports
unsupported menu/offer/chat payloads explicitly. Malformed/truncated framing
still fails the reader. Every inventory/menu/cursor stack shares one decoder.

`snapshot.events` retains actual received texts for the entire client scenario,
including closed connections before reconnect/restart boundaries. Event indices
do not reset or roll over; `snapshot.generation` identifies the current socket
generation. Authentication still examines only that generation's received
prompts. Structured `chat_components` remains the current connection's bounded
diagnostic view; use cumulative `events` for cross-boundary assertions.

Protocol `snapshot.uuid` is the UUID accepted in LoginSuccess, not the requested
offline identity. `snapshot.entity_id` is the actual LoginPlay local entity ID.
Both describe the current connection and are omitted until received.

Protocol `entities` is an array of actual tracked spawn packets: numeric `id`,
wire `uuid`, registry-named `type`, current `position`, and immutable
`spawn_position`. Received metadata may add `item`, `health`, `xp_value` or
`effects`; absent metadata is unavailable, not a default. Relative/absolute
movement changes current position only; remove/respawn packets invalidate
tracking. `xp` contains received progress/level/total. Local `effects` uses the
actual LoginPlay entity ID, never an invented local spawn; effect keys are
numeric-ID strings resolved with the mob-effect registry above.

Malformed/unsupported entity packets leave sticky
`observation_errors.entities` and omit the entity/XP/effect observations until
respawn. An unrelated successful packet cannot reconstruct lost state. These
observations do not expose custom names, equipment, passengers, leashes, pose
or movement velocity.
