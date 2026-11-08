"""Subjects part 4: social/admin, plugins, client surfaces, world/persistence."""
from subjects import domain

domain("chat", ["core:tick/input/social.rs", "edge:decode.rs"], ["chat_send", "read_chat"], [
    ("public_chat", "send chat message", "broadcast to all as <name> msg; ordering preserved", {"mp": True}),
    ("chat_validation", "send 257-char / illegal-character / empty message (raw)", "edge rejects/kicks per vanilla (illegal characters kick)", {"caps": ["malformed_packet_injection"]}),
    ("chat_spam", "send > vanilla spam threshold", "disconnect for spamming (vanilla 20/200 rule)", {}),
    ("secure_chat_session", "client sends chat session update / signed command", "server accepts signed messages or degrades to system; no kick for valid profile keys", {"src": ["edge:decode.rs"]}),
    ("private_message", "/msg /tell /w to player; non-op with selector", "only target sees; non-op selector refused", {"mp": True}),
    ("emote_say", "/me and /say", "/me broadcast emote; /say op-only broadcast", {}),
    ("team_message", "/teammsg in a team", "only team members receive", {"mp": True, "src": ["core:scoreboard.rs"]}),
    ("join_leave_messages", "player joins/leaves", "yellow join/leave messages to others", {"mp": True}),
    ("advancement_announce", "earn advancement (announceAdvancements)", "chat announcement to all when true", {"mp": True}),
    ("chat_visibility_setting", "client chat visibility = hidden/commands only", "server suppresses chat accordingly", {"src": ["edge:play.rs"]}),
])

domain("scoreboard_teams", ["core:scoreboard.rs", "core:scoreboard_persistence.rs"], ["command_send", "read_hud_ui"], [
    ("objectives_display", "/scoreboard objectives add/setdisplay sidebar|list|below_name", "displayed objective packets to all clients", {"op": 2}),
    ("scores_modify", "/scoreboard players set/add/remove/reset/get/operation", "scores updated; query returns", {"op": 2}),
    ("criteria_auto", "deathCount/playerKillCount/health/food objectives", "auto-updated by events", {"op": 2}),
    ("teams_membership", "/team add/join/leave/empty/remove", "membership and tab-list color", {"op": 2}),
    ("team_options", "friendlyFire, color, prefix/suffix, nametagVisibility, collisionRule, seeFriendlyInvisibles, deathMessageVisibility", "each option enforced/presented", {"op": 2, "mp": True}),
    ("trigger", "non-op /trigger enabled objective", "score incremented once then disabled", {}),
    ("scoreboard_persistence", "objectives/teams across restart", "state restored exactly", {"op": 2, "caps": ["server_clean_restart"]}),
])

domain("world_border", ["core:command_border.rs"], ["command_send", "read_world_state"], [
    ("set_and_lerp", "/worldborder set 100; /worldborder set 50 10s (explicit unit: in 26.1.2 the time is a TimeArgument, so a bare 10 means 10 ticks); /worldborder add -10 200t",
     "border packets; size lerps 100->50 over 200 ticks (10 s) and by -10 over 200 ticks; a unit-less number is ticks (WorldBorderCommand.javap: TimeArgument.time(0))", {"op": 2}),
    ("center", "/worldborder center x z", "border moves; players outside pushed/damaged", {"op": 2}),
    ("damage_buffer_warning", "/worldborder damage amount/buffer, warning distance/time (time with an explicit unit, e.g. 15s; a bare number is ticks)", "damage outside buffer; red warning", {"op": 2}),
    ("block_actions_outside", "place/break outside border", "refused", {"op": 2}),
    ("teleport_outside_clamped", "/tp a player beyond the border; throw an ender pearl at the border", "vanilla: /tp outside the border is allowed (player then takes border damage outside the buffer); ender pearls collide with the border (1.21.2+), no universal clamp", {"op": 2}),
    ("border_persistence", "border state across restart", "size/center/lerp restored", {"op": 2, "caps": ["server_clean_restart"]}),
])

domain("plugins", ["core:plugin.rs", "script:runtime.rs", "edge:plugin_loader.rs"], ["command_send", "plugin_deploy"], [
    ("perm_groups_user_set", "/perm groups; /perm user <uuid|me> set <group>", "group assignment persisted; command gates follow nodes", {"plugins": ["solaris-permissions"], "op": 3}),
    ("perm_check_node", "/perm check <node> [context]", "true/false per group; denies essentials sethome without solaris.essentials.use", {"plugins": ["solaris-permissions", "solaris-essentials"]}),
    ("essentials_homes", "/sethome /home /delhome", "home saved per player; teleport", {"plugins": ["solaris-essentials"], "caps": ["walk_move"]}),
    ("essentials_spawn_warps", "/setspawn /spawn /setwarp /warp /delwarp (manage node)", "warps/spawn teleports; manage gated", {"plugins": ["solaris-essentials"]}),
    ("essentials_back", "/back after death/teleport", "returns to previous location (player.died event)", {"plugins": ["solaris-essentials"]}),
    ("essentials_tpa", "/tpa target; target /tpaccept", "requester teleported after accept only", {"plugins": ["solaris-essentials"], "mp": True}),
    ("essentials_msg_reply", "/msg (plugin root) and /reply", "plugin owns msg root; reply to last sender", {"plugins": ["solaris-essentials"], "mp": True}),
    ("economy_money_pay", "/money; /pay <uuid> <amount> <token>; repeat same token", "balances move once; idempotent token", {"plugins": ["solaris-economy"], "mp": True, "custody": True}),
    ("economy_admin", "/econadmin set|add <uuid> <amount>", "admin-only balance change", {"plugins": ["solaris-economy"], "op": 3}),
    ("towns_create_info", "/town create <name>; /town info; stand in an unclaimed chunk and /town claim; /town info; /town create <other> again",
     "'Town <name> created.' with the creator as leader and 0 claims; info prints exactly '<name>: 1 members, 0 claims. 0 trusted guests, doors members-only.' (counts only, never an owner uuid); "
     "claim answers 'Chunk claimed for <name>.' and info then shows 1 claims; a second create by a town member answers 'Cannot create that town.' "
     "(solaris-towns lib.rs create/info/claim; live wasm == monitor-plugins build)", {"plugins": ["solaris-towns"]}),
    ("towns_protection", "guest builds/opens chest/ignites/hurts animals in claim; /town trust; /town flag use on|off", "guest refused w/o item loss; trusted allowed; flag toggles use", {"plugins": ["solaris-towns"], "mp": True}),
    ("audit_queries", "/audit [count]; here <r>; since <ticks>; actor <uuid>", "records of break/place/craft/pickup/kill/interact/death", {"plugins": ["solaris-audit"]}),
    ("basic_economy_shop", "/economy shop; buy/sell via menu; enter/leave market zone", "inventory-backed transaction exact; zone events", {"plugins": ["basic-economy"], "menu": True, "custody": True, "caps": ["open_container", "click_pickup_quickmove"]}),
    ("auth_register_login", "/register pw pw; relog; wrong pw; /login; /changepassword", "locked until login (no build/damage); wrong pw refused; secrets never echoed", {"plugins": ["solaris-auth"]}),
    ("auth_session_timeout", "stay unauthenticated past timeout", "kicked after timeout", {"plugins": ["solaris-auth"]}),
    ("chisel_conversion", "chisel stone bricks/stairs to slab/stairs", "block converted; tool wear; slab grants", {"plugins": ["chisel"]}),
    ("chisel_loader_bundle", "join the live set (chisel bundle required = false) with a Solaris client (native, Fabric/NeoForge/Forge Loader) and with vanilla", "Solaris client that consents gets the chisel item definition/model/texture; vanilla completes configuration without the bundle (edge configuration.rs: optional bundles are negotiated only after a Loader hello) and sees chisel:chisel as its minecraft:stick carrier; the chisel README sentence 'vanilla is refused' is stale vs plugin.toml required = false", {"plugins": ["chisel"], "caps": ["loader_bundle_handshake"]}),
    ("graves_custody", "die with items (graves deployed)", "barrel grave at death spot holds exact drops; no ground copy", {"plugins": ["graves"], "custody": True}),
    ("graves_owner_protection", "other player breaks/opens grave before public_after_seconds", "refused until public; owner can loot", {"plugins": ["graves"], "mp": True}),
    ("plugin_startup_fail_closed", "deploy two death_graves packages / api 0.6 legacy (land-claims, online-roster, geological-mines)", "startup refuses with clear error", {"status": "not_deployable", "caps": ["server_clean_restart"]}),
    ("plugin_trap_isolation", "plugin guest traps during event", "server keeps ticking; plugin disabled/isolated with log", {"src": ["core:plugin.rs"]}),
    ("plugin_storage_persistence", "plugin data across restart", "storage restored exactly", {"caps": ["server_clean_restart"]}),
    ("plugin_command_tree", "client command tree with plugins deployed at op 0/4", "plugin roots advertised per permission; core root shadowed when plugin owns it", {"src": ["edge:command_tree.rs"]}),
])

domain("block_log", ["core:block_log.rs"], ["command_send"], [
    ("co_inspect", "/co inspect then click blocks", "history shown", {"op": 3, "caps": ["use_item_on_block"]}),
    ("co_lookup", "/co lookup filters", "matching records listed", {"op": 3}),
    ("co_rollback", "grief then /co rollback", "blocks and chest contents restored exactly", {"op": 3, "custody": True}),
    ("co_restore", "/co restore after rollback", "changes re-applied", {"op": 3}),
    ("co_history_restart", "restart then lookup", "history persisted", {"op": 3, "caps": ["server_clean_restart"]}),
])

domain("advancements_stats", ["core:advancements.rs", "core:statistics.rs", "edge:advancements.rs"], ["read_world_state"], [
    ("earn_advancements", "earn story/nether/end/adventure/husbandry criteria", "advancement granted with toast; parent tree shown", {"caps": ["advancement_stats_ui"]}),
    ("advancement_tabs_seen", "open advancements screen, switch tabs", "SeenAdvancements handled; no toast flood", {"caps": ["advancement_stats_ui"]}),
    ("recipe_unlocks", "pick up items that unlock recipes", "recipe book entries added", {"caps": ["recipe_book_place"]}),
    ("statistics_request", "open statistics screen repeatedly", "authoritative counters (blocks mined, distance, kills) match actions", {"caps": ["advancement_stats_ui"]}),
    ("advancement_location", "enter structures/biomes for location criteria", "location triggers fire", {"src": ["core:advancements_location.rs"]}),
])

domain("ui_presentation", ["core:bossbar.rs", "core:dialog.rs", "edge:boss_bars.rs"], ["read_hud_ui"], [
    ("bossbar_custom", "/bossbar add/set players/value/color/style", "bossbar shown/updated/removed", {"op": 2}),
    ("bossbar_entities", "wither/dragon/raid bossbars", "auto bossbars track health/progress", {}),
    ("title_actionbar", "/title times/title/subtitle/actionbar/clear/reset", "client shows texts with timing", {"op": 2}),
    ("tellraw_components", "/tellraw with clickEvent/hoverEvent/translate/score/selector", "components resolved server-side", {"op": 2}),
    ("tab_list", "players join/leave/change gamemode/latency", "player info updates; spectators greyed", {"mp": True}),
    ("server_dialogs", "show_dialog with text/slider/checkbox; submit", "custom_click_action payload exact at server", {"caps": ["dialog_interaction"], "src": ["core:dialog.rs"]}),
    ("nameplates_labels", "named entities, CustomNameVisible, settlement labels", "labels shown per flags", {"visual": True, "src": ["edge:settlement_labels.rs"]}),
    ("entity_glow_invisibility", "glowing/invisibility effects on others", "metadata flags visible to observers", {"mp": True, "visual": True}),
])

domain("entities_misc", ["core:entities.rs", "core:entity_controls.rs"], ["interact_entity", "read_world_state"], [
    ("boats", "place/ride/row boat and chest boat on water and ice", "rowing physics; break drops boat (+contents)", {"src": ["core:vehicles/mod.rs", "core:vehicles/motion.rs"], "caps": ["vehicle_control"]}),
    ("minecarts", "ride minecart on rails; furnace minecart with coal", "rail motion; minecartMaxSpeed; furnace push", {"src": ["core:vehicles/mod.rs"], "caps": ["vehicle_control"]}),
    ("horses_mounts", "tame/saddle/armor horse; ride pig/strider with stick; camel dash", "steering per mount; saddle custody", {"src": ["core:mounts.rs", "core:taming.rs"], "caps": ["vehicle_control"], "custody": True}),
    ("leash", "leash mobs, tie to fence, break leash by distance", "leash entity/knot; snaps beyond 12 blocks (16 for happy ghast) per 26.1.2 / minecraft.wiki Lead", {"src": ["core:leash.rs"]}),
    ("armor_stand_entity", "armor stand pose/equipment/break", "two-punch break drops stand + equipment", {"src": ["core:armor_stand.rs"], "custody": True}),
    ("item_frame_entity", "glow/item frame drop, rotate, break support", "frame and item drop once", {"src": ["core:item_frame.rs"], "custody": True}),
    ("experience_orbs", "orb merge/pickup/mending", "merge by value; pickup cooldown", {"src": ["core:experience_orbs.rs"]}),
    ("display_interaction_markers", "summon block/item/text display, interaction, marker", "spawned with data; interaction entity reports clicks", {"op": 2, "src": ["core:command_summon.rs"]}),
    ("area_effect_cloud", "lingering potion / dragon breath cloud", "cloud applies effect; dragon breath bottle collection", {}),
    ("paintings", "place paintings of all sizes", "vanilla: random fitting variant; drops on support break", {"status": "no_code_hit"}),
    ("happy_ghast_harness", "ride happy ghast with harness", "vanilla 1.21.6+: harnessed flying mount for up to 4", {"status": "no_code_hit", "caps": ["vehicle_control"]}),
])

domain("sculk_trials", ["core:sculk.rs", "core:shrieker.rs", "core:warden.rs", "core:trial_spawner.rs", "core:vault.rs"], ["read_world_state"], [
    ("sculk_sensor", "make vibrations near sensor; wool occludes", "sensor active/cooldown; wool blocks", {"src": ["core:sculk.rs", "core:vibration.rs"]}),
    ("calibrated_sensor", "calibrated sensor with input frequency", "only selected frequency activates", {"src": ["core:sculk.rs"]}),
    ("sculk_catalyst_spread", "mob dies near catalyst", "sculk spreads by XP", {"src": ["core:sculk.rs"]}),
    ("shrieker_warden_summon", "trigger shrieker 4x (doWardenSpawning)", "darkness warning levels; warden emerges", {"src": ["core:shrieker.rs", "core:warden.rs"]}),
    ("warden_behaviour", "warden anger, sniff, sonic boom, dig after 60s idle", "anger by vibrations/projectiles; sonic boom ignores armor", {"src": ["core:warden.rs"]}),
    ("trial_spawner_waves", "enter trial spawner range with 1/2 players", "mob count by player count; reward loot; cooldown", {"src": ["core:trial_spawner.rs"]}),
    ("ominous_trials", "Trial Omen enters spawner", "ominous waves; ominous item spawners", {"src": ["core:trial_spawner.rs", "core:omens.rs"]}),
])

domain("special_blocks", ["core:tick/input/interact.rs"], ["use_item_on_block", "read_world_state"], [
    ("beacon_pyramid_effects", "beacon on 1-4 tier pyramid, pay and select", "range/effects by tier; secondary at tier 4", {"src": ["core:beacon.rs"], "caps": ["set_beacon"], "menu": True, "custody": True}),
    ("conduit", "activate conduit with prismarine frame", "conduit power to players in water; attacks hostiles at full frame", {"src": ["core:conduit.rs"]}),
    ("jukebox_allay", "jukebox playing; allay dances and duplicates with amethyst", "allay duplicates once with cooldown; delivers items to note block", {"src": ["core:jukebox.rs", "core:allay.rs"]}),
    ("bell_ring", "ring bell near villagers/raiders", "villagers flee indoors; raiders glow", {"src": ["core:villager/brain.rs"]}),
    ("copper_aging", "oxidation over random ticks; waxing", "four stages; waxed stops", {"src": ["core:copper.rs"]}),
    ("composter_hopper", "hopper into/out of composter", "compost accepted; bone meal extracted", {"src": ["core:composter.rs"]}),
    ("monster_spawner_entity", "spawner with custom entity via egg", "spawns that type", {"src": ["core:monster_spawner.rs"]}),
    ("decorated_pot_chiseled_bookshelf", "insert items into decorated pot / books into chiseled bookshelf", "vanilla: single-stack storage / 6 book slots with comparator", {"status": "no_code_hit"}),
])

domain("world_chunks", ["edge:chunks.rs", "core:chunk_loader.rs", "core:chunk_tickets.rs", "core:light.rs"], ["walk_move", "read_world_state"], [
    ("fresh_generation", "explore ungenerated terrain in each dimension", "chunks generated deterministically by seed; no holes/stripes", {"caps": ["natural_terrain_navigation"]}),
    ("chunk_streaming_radius", "stand still with view 8 / sim 4", "full view square readable; simulation radius bounded", {"src": ["edge:chunks.rs"]}),
    ("light_updates", "place/remove torches, roofs, cross-chunk seams", "client light equals server light; no dark chunks", {"src": ["core:light.rs", "edge:light.rs"]}),
    ("anti_xray", "enclosed ores; mine next to them", "hidden ores absent from client data until exposed", {"src": ["edge:anti_xray.rs"]}),
    ("distant_horizons_lod", "DH client requests distant LOD", "LOD data for never-visited chunks; vanilla packet bounds", {"src": ["edge:dh.rs", "edge:lod.rs"], "clients": ["fabric_distant_horizons"]}),
    ("forceload_spawn_chunks", "/forceload add/remove/query; then /gamerule spawnChunkRadius (removed in 26.1.2)", "forced chunks tick without players; removed spawn-chunk rule is rejected as unknown (vanilla 26.1.2 has no spawn chunks, core:command_rules.rs)", {"op": 2, "src": ["core:forceload.rs", "core:command_rules.rs"]}),
    ("structures_loot", "generated ruin/structure loot chest first open", "loot table rolled once on first open", {"src": ["core:maps/loot.rs"], "menu": True, "custody": True}),
    ("pregenerate", "operator pregenerate radius", "chunks generated off-tick; server responsive", {"src": ["crates/mc-world-tools/src/pregenerate.rs"], "op": 4}),
    ("world_import_export_anvil", "import vanilla Anvil world; export back", "blocks/entities/players round-trip; vanilla opens export", {"src": ["crates/mc-world-conversion/src/lib.rs"], "caps": ["server_clean_restart"]}),
    ("worldgen_upgrade", "open world generated by older Solaris worldgen", "upgrade path consistent; no seams", {"src": ["crates/mc-world-tools/src/worldgen_upgrade.rs"]}),
])

domain("persistence", ["core:native_checkpoint.rs", "core:journal.rs", "core:hot_backup.rs"], ["server_clean_restart", "read_world_state"], [
    ("player_data_roundtrip", "inventory/ender chest/xp/effects/spawn point across restart", "identical after restart", {"custody": True}),
    ("block_entities_roundtrip", "chests/furnaces mid-smelt/brewing/signs/banners/spawners", "contents and progress identical", {"custody": True, "inflight": True}),
    ("entities_roundtrip", "named/leashed/tamed mobs, item frames, armor stands, vehicles", "same UUIDs and state", {"src": ["core:entity_state_persistence_tests.rs"]}),
    ("world_metadata_roundtrip", "time, weather, gamerules, difficulty, border, scoreboard, raids, dragon fight", "restored", {"src": ["core:level_metadata.rs"]}),
    ("corrupt_store_fail_closed", "corrupt a store/WAL file then start", "startup fails closed with clear error; no silent reset", {"src": ["core:persistence_crash_tests.rs"]}),
    ("backup_restore", "operator backup then restore", "restored world equals backup point", {"src": ["edge:backup.rs", "core:hot_backup.rs"], "caps": ["server_hot_backup"], "op": 4}),
    ("save_all_flush", "/save-all [flush] under load", "checkpoint written; tick not blocked", {"op": 4, "caps": ["server_save_all"]}),
])

domain("loader_content", ["edge:plugin_loader.rs", "script:client_view.rs", "crates/mc-loader/src/lib.rs"], ["loader_bundle_handshake"], [
    ("bundle_handshake_cache", "Loader client joins with plugin bundle; rejoin", "manifest -> download -> sha256 verify -> cache hit on rejoin", {}),
    ("optional_bundle_vanilla", "vanilla client joins the live set (every bundle is required = false: chisel, computer)", "configuration completes, no solaris:loader/manifest is sent, and the client plays without Loader content (edge configuration_tests vanilla_client_completes_configuration_with_only_optional_bundles)", {"plugins": ["chisel", "computer"]}),
    ("required_bundle_vanilla_refused", "fixture: a copy of the chisel package with required = true; vanilla client joins", "refused during Configuration with a reason naming the Loader requirement; no play (edge server.rs: any required bundle forces Loader negotiation on every client)", {"plugins": ["chisel"]}),
    ("permission_prompt_cache", "Loader client's first join to the live set: prompt asks register_items/load_assets for chisel-content; deny; rejoin; reset the stored decision; allow; rejoin", "deny -> the Loader disconnects itself with 'permissions were denied for <server>' (loader-fabric handleManifest -> disconnect; applies to optional bundles too once the client said hello) and stores the denial, so the rejoin disconnects without a new prompt; allow -> bundle downloaded/verified and the decision cached, rejoin without prompt (loader-core LoaderPermissionController)", {"plugins": ["chisel"]}),
    ("loader_view_request", "press C (settlement overview) / plugin view", "server-declared view opens with plugin data", {"caps": ["loader_view_request"]}),
    ("custom_items_render", "chisel:chisel in inventory, hand and dropped in the world on a Solaris client", "custom model/texture renders; the minecraft:stick carrier is never visible to a consenting Solaris client (the vanilla stick fallback is checked in plugins.chisel_loader_bundle)", {"plugins": ["chisel"], "visual": True}),
    ("stale_bundle_update", "plugin bundle version changes between joins", "new bundle fetched; stale not used", {}),
])

domain("computer", ["edge:computer.rs", "edge:screen.rs", "edge:screen_service.rs (deploy-final ad53e139, 97d69e1e audio)", "docs/decisions/0018-in-game-computer.md 'Screen sharing'", "ldr:loader-fabric ComputerClient/ComputerScreen"], [], [
    ("terminal_open_op4", "enabled computer; op4 right-clicks iron block", "terminal opens on solaris:computer; echo backend replies", {"op": 4, "clients": ["fabric_solaris_loader"], "menu": True, "caps": ["computer_channel"]}),
    ("terminal_refusals", "non-op, locked, far, other dimension opens", "refused with closed reply; no process spawned", {"clients": ["fabric_solaris_loader"], "caps": ["computer_channel"]}),
    ("terminal_session_close", "break block / leave reach / deop / disconnect during session", "session closed; child killed", {"op": 4, "clients": ["fabric_solaris_loader"], "caps": ["computer_channel"]}),
    ("terminal_disabled_vanilla_iron", "computer disabled; use iron block", "plain iron block behaviour", {}),
    ("desktop_capture_local", "local desktop capture to in-game texture (X11 spike)", "client-local only; no bytes to server; explicit consent", {"clients": ["fabric_solaris_loader"], "visual": True, "caps": ["computer_channel"]}),
    ("screen_share_to_others", "native owner OPENs a WxH solaris:monitor rectangle within 8 blocks; a second native viewer looks at it, REQUESTs, owner ANSWERs allow; viewer looks away/back; owner REVOKEs; viewer LEAVEs",
     "solaris:screen protocol 1 (ADR 0018): everyone in the dimension gets ACTIVE(on) only, no frames, until granted; DEMAND turns on for the owner only while a granted viewer looks (32 blocks, published pose, no occlusion); FRAME relayed to granted looking viewers only; looking away -> STOP 0, revoke -> STOP 2, owner close -> STOP 1, owner leave -> STOP 3; viewers never send input",
     {"plugins": ["computer"], "clients": ["native_linux", "native_windows"], "mp": True, "visual": True, "caps": ["native_screen_share", "multi_client_2"]}),
    ("screen_share_refusals", "OPEN out of reach / on non-monitor blocks / 5th screen / taken rectangle; frames without DEMAND, oversized (>640 px, >131072 B, >5 parts), repeated unsolicited; break or unload a monitor block mid-show",
     "REFUSED with the matching reason (2 out of reach, 1 not a monitor, 3 too many, 4 taken, 5 bad frame, 7 repeat/rate limited); budget overflow frames dropped; malformed or repeated unsolicited frames close the screen; broken/unloaded monitor ends the show; connection never kicked",
     {"plugins": ["computer"], "clients": ["native_linux", "native_windows"], "mp": True, "caps": ["native_screen_share", "multi_client_2"]}),
    ("screen_share_audio", "owner's captured app plays sound while a granted viewer looks; viewer looks away; a non-granted player stands next to it",
     "AUDIO relayed with the same id/seq only to granted looking viewers; nothing within one tick of looking away/revoke; non-granted hears nothing; packet/byte token buckets bound it (ADR 0018 'Screen audio')",
     {"plugins": ["computer"], "clients": ["native_linux", "native_windows"], "mp": True, "caps": ["native_screen_share", "multi_client_2"]}),
    ("screen_share_non_native_clients", "vanilla / Fabric / NeoForge / Forge Loader client next to a native owner sharing a monitor",
     "these clients never see solaris:screen (relay serves native-Loader clients only); they see the plain monitor/terracotta; no disconnect. Fabric not showing the share is the client lacking the channel, not a missing server feature",
     {"plugins": ["computer"], "clients": ["vanilla_java_26_1_2", "fabric_solaris_loader", "neoforge_solaris_loader", "forge_solaris_loader"], "mp": True, "caps": ["multi_client_2"]}),
])

domain("monitor", ["scenarios/live-plugins/computer (plugin id solaris; README)", "edge:computer.rs"], ["command_send", "place_block", "read_world_state"], [
    ("monitor_give_place_break", "/give @s solaris:monitor; place it facing the player; break it", "item carrier black_glazed_terracotta (stack 64); block stored as black_glazed_terracotta[facing=toward placer]; breaking drops solaris:monitor (not terracotta), exactly once",
     {"op": 2, "custody": True, "plugins": ["computer"], "caps": ["break_block"]}),
    ("monitor_native_bundle_render", "native client with the computer-content bundle sees the placed monitor", "bundle bound (loaders=[native]); monitor model with screen on the facing side renders; item named Monitor", {"plugins": ["computer"], "clients": ["native_linux", "native_windows"], "visual": True, "caps": ["loader_bundle_handshake", "screenshot_visual"]}),
    ("monitor_other_clients_fallback", "vanilla / Fabric clients join the server with the computer bundle and look at a monitor", "not bound to the optional bundle; see and break black glazed terracotta normally; no disconnect", {"plugins": ["computer"], "clients": ["vanilla_java_26_1_2", "fabric_solaris_loader", "neoforge_solaris_loader", "forge_solaris_loader", "fabric_svc", "fabric_distant_horizons", "protocol_bot"]}),
    ("monitor_plain_terracotta_not_placeable", "try to place a plain minecraft:black_glazed_terracotta item", "refused (the block states are reserved for solaris:monitor per the plugin README); client resynced; item kept", {"plugins": ["computer"], "op": 2, "custody": True}),
])

domain("voice", ["edge:voice.rs", "edge:voice_service.rs (deploy-final 617b3091)", "docs/decisions/0019 proximity voice"], [], [
    ("native_proximity_voice", "two native clients with LISTEN(1); one pushes to talk at 10 and at 60 blocks, and from another dimension",
     "solaris:voice protocol 1 (ADR 0019): AUDIO relayed with speaker uuid, eye position and seq to listening native clients in the same dimension within range_blocks (default 48), never to the speaker; nothing beyond range or across dimensions",
     {"clients": ["native_linux", "native_windows"], "mp": True, "caps": ["native_voice", "multi_client_2"]}),
    ("native_voice_listen_and_state", "native client joins with [voice] enabled and disabled; LISTEN(0) vs LISTEN(1); malformed/oversized (>1000 B) AUDIO",
     "VOICE_STATE(enabled, range_blocks) sent once on join; no AUDIO before LISTEN(1) or after LISTEN(0); malformed or unknown payloads ignored, never a kick; disabled -> no relay",
     {"clients": ["native_linux", "native_windows"], "mp": True, "caps": ["native_voice", "multi_client_2"]}),
    ("voice_non_native_invisible", "vanilla / Loader Java clients near a talking native player", "they never receive solaris:voice (no VOICE_STATE, no relay); play unaffected", {"clients": ["vanilla_java_26_1_2", "fabric_solaris_loader", "protocol_bot"], "mp": True, "caps": ["native_voice", "multi_client_2"]}),
    ("svc_client_join_without_server", "Fabric+SVC client joins Solaris", "joins and plays; SVC reports voice unavailable; no disconnect", {"clients": ["fabric_svc"], "caps": ["svc_voice"]}),
    ("proximity_voice", "two Simple Voice Chat clients talk at distance", "SVC proximity audio. Not served: Solaris has no SVC protocol bridge (its voice is native solaris:voice only)", {"clients": ["fabric_svc"], "mp": True, "status": "server_missing", "caps": ["svc_voice"]}),
    ("group_voice", "SVC groups", "SVC group audio. Not served: no SVC bridge", {"clients": ["fabric_svc"], "mp": True, "status": "server_missing", "caps": ["svc_voice"]}),
])

domain("resource_packs", ["edge:resource_pack.rs"], ["resource_pack_prompt"], [
    ("pack_accept", "server pack configured; client accepts", "pack downloaded/loaded; play continues", {}),
    ("pack_decline_optional", "decline optional pack", "plays without pack", {}),
    ("pack_decline_required", "decline required pack", "disconnected: required resource pack declined", {}),
    ("pack_download_fail", "bad URL/hash", "failed status; required -> disconnect", {}),
    ("pack_prompt_text", "custom prompt component", "prompt displayed", {"visual": True}),
])
