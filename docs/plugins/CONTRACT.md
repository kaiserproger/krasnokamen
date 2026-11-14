# Plugin authoring contract

## Package manifest

A minimal server-only component package can declare no subscriptions,
capabilities, or player commands:

```toml
id = "hello"
name = "Hello"
version = "0.1.0"
api = "0.8.0"
```

The optional `entry` names a component file inside the package and defaults to
`plugin.wasm`. `events`, `capabilities`, `required_features`,
`player_commands`, `command_permissions`, `permission_provider`, `hooks`,
`[client]`, and `[worldgen]` are validated package declarations. Unknown fields
and invalid or duplicate values refuse the package before guest code is
compiled.

`required_features` describes authored data core must load. It does not grant
access. Feature capabilities must also appear in `required_features`. Packages
declare ordinary capabilities in their manifests; selecting the deployment
directory is the operator authorization decision.

Packages without `[client]` bundles are `server_only` and support ordinary
vanilla clients. Any package that declares client bundles makes the relevant
deployment `server_and_client`: connecting players need a compatible Krasnokamen
Loader and must approve the bundle's requested client permissions. Bundle paths,
hashes, sizes, content kinds, supported loaders, and permissions are validated
before a player is served content. There is no server-only substitute for a
package that requires client content. See [`KRASNOKAMEN_LOADER.md`](../KRASNOKAMEN_LOADER.md)
for player installation.

A bundle may set `required = false` (default `true`) in its `[[client.bundles]]`
table for content that enriches rather than enables the package, such as a custom
item texture. When every bundle of a deployment is optional, Configuration sends
a `krasnokamen:loader/probe` payload (empty) before known-pack negotiation. A Loader
client answers `krasnokamen:loader/hello` (empty) while handling it, so the hello
precedes its known-pack reply; the server then runs the normal manifest/artifact/
ACK negotiation. A client that does not answer (a vanilla client, or a Loader
build that predates the probe) joins without any Loader content, and its item
stacks omit non-`minecraft:` `item_model` components so plugin items show as
their carrier item with their custom name rather than the missing-model cube.
One required bundle anywhere in the deployment keeps the unchanged behavior:
every client negotiates and a client without the Loader is refused.

## Build, encode, and deploy a Rust guest

The SDK is an API dependency, not a home for product plugins. A guest's source,
manifest, configuration, and deployed artifact belong to that plugin's own
repository. Build its `wasm32-unknown-unknown` module, then encode that module
as a component before placing it in its package:

```sh
rustup target add wasm32-unknown-unknown
cargo build --manifest-path path/to/plugin/Cargo.toml \
  --target wasm32-unknown-unknown --release
cargo run --manifest-path crates/mc-plugin-host/Cargo.toml \
  --example plugin_component -- \
  path/to/plugin.wasm plugins/my-plugin/plugin.wasm
```

`plugin-component` validates while encoding; a module that lacks the component
types cannot become a deployable artifact. The plugin repository owns
`plugin.toml` and optional `config.toml`; the core repository neither ships nor
compiles first-party product plugins.

Authors implement `krasnokamen_plugin_sdk::Plugin` (or bindings for the same WIT
world) and use the WIT and SDK definitions as the API reference. Do not copy
internal host types into a guest or rely on a host implementation detail.

The contract is `krasnokamen:plugin@0.9.0`: WIT record layout changes require
regenerating host and guest bindings, and every guest must be rebuilt and
re-encoded against it (the Rust SDK workspace is 0.8.0; add `mob_blueprints:
None` to existing full `StartupContribution` literals and update the manifest
`api`). This applies to the first-party `krasnokamen-default-plugins` packages too;
no 0.7.0 component or old record-layout shim is accepted, and an older
component's record layout is not a compatible fallback.

## Lifecycle and configuration

A component has two startup phases in distinct stores:

1. `configure` receives the package configuration in a short-lived store with
   no runtime capability. It may return a normalized startup contribution that
   is validated and fingerprinted into the world contract.
2. `init` receives the same configuration and initialization context in the
   component's runtime store. It creates the instance's runtime-local state and
   may return its opening command batch.

State retained during `configure` cannot reach `init`; initialize runtime state
from the configuration again. The host performs discovery, manifest validation,
startup contribution validation, and deployment-surface validation before it
opens a world or binds the listener. A conflicting startup contribution,
world-generation declaration, client surface, or command ownership refuses
startup.

`shutdown` is best effort. The host reclaims its own resources whether or not
it runs, and a trap can skip it, so no durable correctness decision may depend
on cleanup. Plugin storage remains keyed by plugin identity in the native tick
WAL/checkpoint. Restart creates fresh component-local state; it is not a guest
instance migration. The current server does not support SIGHUP deployment reload.
Retain the same startup/world identity when restarting an existing world.

## Custom items, crafting recipes and block tools

`configure` may declare up to 16 plugin-owned `items` (a vanilla `carrier`
identity, stack size, optional durability), which need a Loader bundle with
`register_items` to present their own model/texture. Two further startup
fields extend them, both immutable, bounded and fingerprinted into the world
contract (omitted from it when unused):

- `crafting-recipes` (at most 32 strings, 64 KiB): vanilla-format
  `minecraft:crafting_shaped` / `crafting_shapeless` recipe JSON. The recipe id is
  `result.id`, which must be one of the plugin's declared items; every ingredient
  must be a registered item, an owned item or a known item tag.
- `block-tools` (at most 1024 strings, 1 MiB): rules for what a held plugin-owned
  item does to the block it is used on. A rule is strict JSON
  `{"item","block","when"?,"face","result","set"?,"wear"?,"give"?}`: the held
  `item` used on `block` (whose properties include `when`) at a clicked `face`
  (`up`, `down`, `side` = any horizontal face, or `any`) replaces it with
  `result`, whose default state takes the `set` properties. A `set` value is a
  literal, `"$face"` (the clicked face name) or `"@name"` (the source block's
  property `name`). `wear` (default 1) is durability charged and `give` one
  vanilla item granted to the player (dropped at their feet when the inventory is
  full). Creative players pay no wear and receive no grant; main hand only. The
  first rule of the item and block matching the face, `when` and a changing result
  wins. Startup resolves every rule against the block registry for every face it
  covers and refuses the deployment when a block, property or value does not
  exist.

The use is one native intent (`IntentKind::BlockTool`) planned beside hoe tilling in
`use_item_on`: the block replacement, the wear and the grant commit or are refused
together against the world and inventory they were planned on. Protection zones
and reach are checked by the same early use gate as block placement; plugin
before-build hooks do not cover it, like hoes. The `chisel` package in
`krasnokamen-default-plugins` is the reference user.

## Custom native mobs

API **0.8.0** adds `mob-blueprints: option<list<string>>` to the configure
contribution. Each string is a strict `MobBlueprintDef` JSON object (see
[ADR 0016](../decisions/0016-mob-capabilities.md)); a package may contribute up to
64 strings, each at most 16 KiB, and the complete registry allows 64 custom
types. Each id must use its defining plugin's namespace. Definitions are sorted
by id before registration and pinned into the world contract. Changing or
removing them refuses an existing world rather than reinterpreting stored mobs.

`sdk/rust/examples/sky-creeper` is a complete Rust guest: it contributes
`sky-creeper:sky_creeper`, presented as a vanilla creeper, with flying movement,
a 20-tick non-griefing explosion and water/rain damage. Its `/sky-creeper`
command spawns one near the calling player through the normal native owner:

```toml
player_commands = ["sky-creeper"]
spawn_entity_types = ["sky-creeper:sky_creeper"]
```

The guest `spawn-entity` command carries a request id, live player session,
namespaced entity type and position in that player's dimension. The manifest
allow-list authorizes type names, not bypasses of session or capacity checks.
`entity-spawn-answered` reports success (`failure = none`) or a typed refusal.
Each defining plugin has a 1024-live-custom-mob limit across dimensions; splitters
reserve descendant capacity as described in ADR 0016. Custom types have no natural
spawn weight. Native checkpoints retain their custom identity; Anvil export uses
the vanilla presentation id and a `krasnokamen:MobBlueprint` marker, not custom
gameplay on a vanilla server.

## Events, commands, and capability boundaries

A manifest subscribes only to the named events it needs and claims only its own
player-command roots. The host delivers events to one component in order. Each
callback receives immutable, bounded records and returns a bounded command
batch; host admission validates every command before native work is scheduled.
An oversized, malformed, unauthorized, trapped, budget-exceeding, or rejected
callback does not expose a partial command batch. A command's eventual result
is delivered as a typed event when the owning native service completes it.

Capabilities gate privileged operations such as storage, player queries,
teleportation, zones, inventory, or world changes. They are not client
permissions and they do not make a request authoritative. Every native owner
still checks the live player session, ownership, revisions, input bounds,
capacity, and its domain invariants before committing work.

`command-invoked` carries the player's authoritative dimension alongside the
position captured at admission. `teleport-player` requires an explicit
destination dimension and only moves a player already in that dimension;
otherwise its typed result is `dimension-mismatch` and no teleport occurs. This
check happens when the request is applied, including after asynchronous storage
callbacks. Cross-dimension plugin teleports are not supported.

### Command permission nodes

A manifest may require one node per command root it already declares:

```toml
player_commands = ["home", "sethome"]
command_permissions = { home = "krasnokamen.essentials.use", sethome = "krasnokamen.essentials.use" }
```

Nodes become enforced when the deployment installs one permission provider
package:

```toml
permission_provider = true
```

The provider is the only instance handed the `publish-command-permissions` host
import. It replaces the whole grant table with `"<identity> <node>"` rows:
`identity` is a dashed or compact lower-case UUID, or `*` for the default row
that applies to a player the table has no explicit row for; a node of `*` grants
every node. Rows are rebuilt from the provider's own durable state, so the table
is not persisted by the host. A publication is bounded (8192 entries, 192 bytes
per entry, 128-byte nodes), all-or-nothing, and refused during a pre-commit hook
like every other import.

Command admission checks the table on the tick thread before an owned root
receives an event. A denied root answers `You do not have permission to use this
plugin command.` and never reaches the guest; operator-only roots keep requiring
operator level 2 first. A second `permission_provider` package, or a node for a
root the package does not declare, refuses the deployment. In a deployment that
has never installed a provider, node declarations are inert. Once managed,
provider retirement clears grants but keeps gated roots denied until an accepted
provider publishes again; a guest trap never returns the deployment to unmanaged
admission. The host's component-reload API stages candidate grants separately,
rejects manifest-only contract changes before guest execution, and installs the
staged snapshot only on acceptance. Rejected initialization and old-generation
shutdown cannot publish to the live table.

`client-presentation.view-marker.position` is optional, display-only geometry:
a resource-identifier dimension plus finite coordinates within the native
`types.position` bounds. The host validates those bounds and counts dimension
text against the same command staging budget as other model strings. Geometry
does not query world state, replace selection fields or their context token,
grant an action, or authorize a physical route or world mutation.

Use stable request and operation identifiers where the WIT operation requires
them. A successful submission is not confirmation of client rendering or a
future mutation; inspect the typed completion or refusal event. For durable
operations, reuse an operation id only for the same intent and payload.

Precommit hooks are a separate bounded decision path. Directory-only deployment
automatically registers manifest `before-build` declarations and rejects
unsupported kinds, including `before-damage`. Hooks receive immutable contexts
and answer decisions, not command batches. They cannot perform I/O, submit
asynchronous work or mutate the world. Missing, expired or retired handlers
follow the [fail-closed policy](OPERATIONS.md#native-server).

