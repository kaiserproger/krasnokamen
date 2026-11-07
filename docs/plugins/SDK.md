# Rust plugin SDK reference

The authoritative contract is [`crates/mc-script/wit`](../../crates/mc-script/wit),
world `solaris:plugin@0.8.0`; the [SDK](../../sdk/rust/solaris-plugin-sdk/src/lib.rs)
generates its types from those files. The table is a map of API families, not an
exhaustive list of every variant. Snippets are fragments in a plugin callback;
`session`, `event`, etc. are values supplied by its caller. Return commands in a
`Vec<Command>`, not as direct effects. The [complete quickstart](QUICKSTART.md)
shows imports and a compilable callback. For declarations, limits and examples
see the [contract](CONTRACT.md) and WIT records, not an assumed Bukkit equivalent.

## Available API families

| API | Purpose | Minimal fragment |
|---|---|---|
| `Plugin::init`, `log` | Runtime initialization and diagnostic log. | `log(LogLevel::Info, "ready");` |
| `Plugin::on_events`, `Event::PlayerJoined` | Receive subscribed observations. | `if let Event::PlayerJoined(p) = event { let _ = p.session; }` |
| `Event::CommandInvoked` | Handle a manifest-owned player command. | `if let Event::CommandInvoked(c) = event { let _ = &c.name; }` |
| `command_permissions`, `publish_command_permissions` | Gate roots on nodes; the one provider publishes bounded grants. | `publish_command_permissions(&["* example.use".into()]);` |
| `message_session`, `message_player`, `broadcast` | Address chat to a session, identity, or all players. | `message_session(session, "Hello!")` |
| `Config::raw`, `Config::toml` | Read package-provided configuration in configure/init. | `let settings = config.toml();` |
| `storage_get`, `storage_cas` | Read or compare-and-swap private durable keys asynchronously (`storage`). | `storage_get("read-1", "coins")` |
| `inventory_storage_transaction` | Atomically request inventory and plugin storage mutations (`inventory_storage_transactions`). | `inventory_storage_transaction("trade-1", session, inventory, storage)` |
| `open_inventory_menu`, `close_inventory_menu` | Open bounded native slot menus; clicks arrive as `InventoryMenuClicked` (`inventory_menus`). | `open_inventory_menu(session, "menu-1", "Shop", slots)` |
| `schedule_timer`, `cancel_timer` | Instance-local tick timers; receive `TimerFired`, not durable across restarts. | `schedule_timer("reminder", 20)` |
| `list_online_players` | Request a bounded player snapshot (`player_queries`). | `list_online_players("list-1", 32)` |
| `Command::TeleportPlayer` | Request a live-session teleport within its current dimension (`player_teleport`). | `Command::TeleportPlayer(teleport)` |
| `query_blocks`, `survey`, `prepare_structure` | Query bounded world facts or request validated physical structures. | `query_blocks("blocks-1", "minecraft:overworld", positions)` |
| `StartupContribution::items`, `crafting_recipes`, `block_tools` | Declare carrier-based items, recipe JSON and validated block-tool rules at startup. | `contribution.crafting_recipes = Some(vec![recipe_json]);` |
| `StartupContribution::mob_blueprints`, `Command::SpawnEntity` | Register bounded native mob blueprints and spawn an allow-listed type. | `contribution.mob_blueprints = Some(vec![blueprint_json]);` |
| `query_npcs`, `issue_npc_order`, `query_npc_route` | Query and direct owned native NPCs and their routes. | `query_npcs("npc-1", &handles, None)` |
| `query_settlements`, `rename_settlement` | Query or request native settlement changes. | `query_settlements("settlements-1", &names, None)` |
| `query_owned_inventory`, `transfer_owned_items` | Query and transfer through native inventory ownership and revision checks. | `query_owned_inventory("inv-1", endpoint, None)` |
| `Command::UpsertProtectedZone` | Request native enforcement of a plugin-owned protected box (`zones`). | `Command::UpsertProtectedZone(zone)` |
| `Plugin::before_build` | Decide an admitted build preview when the manifest registers `before-build`. | `Ok(BuildResult { decision: BuildDecision::Keep, expected_plugin_storage_versions: Vec::new() })` |
| `open_client_view`, `play_client_sound` | Request Loader views and sound for ready, authorized sessions. | `play_client_sound(session, "example:ping", 1.0, 1.0, None)` |
| `password_hash`, `password_verify`, `unlock_session` | Host Argon2id and auth-capable login gate (`auth` for unlock). | `let hash = password_hash(password);` |
| `capabilities`, `required_features` | Manifest declares privilege and, for feature capabilities, required feature contracts. | `capabilities = ["storage"]` |

The startup contribution has multiple optional fields; use an explicitly
initialized `StartupContribution` as in the [sky-creeper example](../../sdk/rust/examples/sky-creeper/src/lib.rs).
Recipes and mob definitions are JSON strings, not arbitrary Rust callbacks.
Queries and mutations return results in later typed events: do not mistake a
returned command for a committed effect. The SDK exposes generated WIT records
when no convenience wrapper exists. The
[first-party packages](https://github.com/kaiserproger/solaris-default-plugins)
show production manifests and patterns; `chisel` is a recipe/block-tool example.

## Not available as a general Bukkit/Paper API

| Missing general surface | Actual boundary |
|---|---|
| Arbitrary synchronous world editing and live world handles | Bounded queries and native validated operations exist; plugins cannot directly mutate blocks or bypass owners. |
| Arbitrary Bukkit inventory GUIs | Bounded native slot menus **do** exist (`inventory_menus`), plus Loader views; they are not a general Bukkit inventory API. |
| Packet interception/injection and arbitrary sockets | The WIT world exports lifecycle/events/precommit and imports only host and command-permissions, not packets or sockets. |
| Bukkit scoreboard/team API | No such API is in the WIT world. |
| General Java plugin loading, reflection and NMS | Components run in Wasmtime; the [offline converter](CONVERTER.md) implements only its documented slice. |
| Arbitrary thread/task scheduler | Tick timers exist, scoped to an instance, but not guest threads or persistent scheduled tasks. |
| Cross-dimension player teleport | The supported teleport is within the player's current dimension and refuses mismatches. |
| Generic event cancellation | Observations have already happened. Only declared, supported precommit hooks decide; `before-damage` is rejected at startup. |

## Sandbox, authority and durability

The component has typed bounded values, no world, socket, session or scheduler
handles, and no direct ownership of native state. The operator authorizes
ordinary manifest-declared capabilities by selecting the deployment directory.
`required_features` is a contract consistency declaration, not a grant; feature
capabilities must appear in both lists. The host validates an entire returned
batch before admission; native owners recheck sessions, revisions, ownership,
capacity and invariants. Traps, budget exhaustion and invalid batches fail closed.
Client Loader bundle permissions are separate from server capabilities. Configure
and init use distinct stores; shutdown is best effort. See the
[full lifecycle and capability contract](CONTRACT.md) and
[operator limits](OPERATIONS.md).

Plugin KV and native effects share the world's native checkpoint and tick WAL,
not separate plugin files. Operation callbacks and command replies follow a
successful journal append, **not fsync**; an unflushed crash tail can consistently
roll back an acknowledged effect. Durable operation IDs replay the recorded
outcome and conflicting reuse is refused. See
[deployment and persistence](OPERATIONS.md#native-server).
