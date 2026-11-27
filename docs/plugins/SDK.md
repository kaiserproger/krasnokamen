# Rust plugin SDK reference

[`crates/mc-script/wit`](../../crates/mc-script/wit), world
`krasnokamen:plugin@0.9.0`, is authoritative; the
[SDK](../../sdk/rust/krasnokamen-plugin-sdk/src/lib.rs) generates its types.
This is an API-family map, not a second declaration catalog.
[Quickstart](QUICKSTART.md) owns the compilable callback/import example;
[authoring contract](CONTRACT.md) owns manifests, limits and lifecycle.

## Available API families

Callbacks return `Vec<Command>` for native admission, not direct world effects.
Use generated WIT records when no wrapper exists.

| Family | API and boundary |
|---|---|
| Lifecycle/config/log | `Plugin::init`, `on_events`, `Config::raw`/`toml`, `log`; `PlayerJoined`/`CommandInvoked` are subscribed observations |
| Permissions | `command_permissions` gates declared roots; sole provider calls `publish_command_permissions` with bounded grants |
| Chat | `message_session` for the exact connection, `message_player` for stable identity, `broadcast` |
| Storage | `storage_get`, `storage_cas`: asynchronous private KV (`storage`) |
| Atomic inventory/KV | `inventory_storage_transaction` (`inventory_storage_transactions`); request ID is correlation, **not** a durable operation ID |
| Menus | `open_inventory_menu`, `close_inventory_menu` (`inventory_menus`); bounded native slots, clicks as `InventoryMenuClicked`; opening/closing is fire-and-forget, silence proves no rendering |
| Timers | `schedule_timer`, `cancel_timer`, `TimerFired`; instance-local ticks, forgotten on restart |
| Players | `list_online_players` bounded snapshot (`player_queries`); `Command::TeleportPlayer` only within current dimension (`player_teleport`) |
| World/structures | `query_blocks`, `survey`, `prepare_structure`: bounded facts/native validated physical work |
| Startup content | `StartupContribution::{placement,trees,clay,spawning,items,work_recipes,crafting_recipes,block_tools,mob_blueprints}` ([contract](CONTRACT.md#startup-contributions)); recipes/mobs are JSON, not arbitrary callbacks; `Command::SpawnEntity` requires an allow-listed type |
| NPCs/routes | `query_npcs`, `issue_npc_order`, `query_npc_route`: owned native NPCs |
| Settlements | `query_settlements`, `rename_settlement`: native owner requests |
| Owned inventory | `query_owned_inventory`, `transfer_owned_items`: ownership/revision-fenced operations |
| Protection/hooks | `Command::UpsertProtectedZone` (`zones`); `Plugin::before_build` only for registered `before-build`, returns a decision/version guards, not commands |
| Loader | `open_client_view`, `play_client_sound`: owned content and authorized ready sessions; sound has no completion event |
| Auth | `password_hash`/`password_verify`: host Argon2id; `unlock_session` requires `auth` and names the exact locked connection |

Initialize every `StartupContribution` field explicitly; see
[sky-creeper](../../sdk/rust/examples/sky-creeper/src/lib.rs).
[First-party packages](https://github.com/kaiserproger/krasnokamen-default-plugins)
own production examples (`chisel`: recipe/block-tool rules).
Inspect each operation's typed completion/refusal where provided; returned
commands are not committed effects, and fire-and-forget calls invent no ACK.

## Not available as a general Bukkit/Paper API

No arbitrary live world handles/block writes, packet interception/injection,
sockets, scoreboard/team API, Java/reflection/NMS, guest threads or persistent
scheduler. Use bounded native operations, native slot menus/Loader views and
instance timers; the [offline converter](CONVERTER.md) supports only its stated
slice. Cross-dimension player teleport refuses mismatches.
Observations cannot be generically cancelled: only supported precommit hooks
decide; `before-damage` refuses deployment.

## Sandbox, authority and durability

The WIT world exports lifecycle, events and precommit and imports only host and
command-permissions functions: typed bounded values, no world, socket, session or
scheduler handles and no native ownership. Capabilities, batch validation and
native rechecks are in the [contract](CONTRACT.md#events-commands-and-capability-boundaries);
plugin state, append-not-fsync replies and operation-id receipts are in
[operations](OPERATIONS.md#native-server). A request id that is only
correlation is not a receipt.
