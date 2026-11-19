# Krasnokamen gameplay/server scenario matrix — summary

Generated 2026-10-24 by `tools/harness/matrix_specs/gen_matrix.py` (deterministic, seed 20261024) from code at solaris@7cff6cb35, solaris-default-plugins@9b407a4, solaris-loader@6b79ace, solaris-client@55d8b2c6. Machine-readable source: `matrix.yaml`.

## Totals

- Dimensions: 8 context factors + 46 gameplay domains with 890 subjects; 599 code-enumerated values (player_input, menu_kind, click_mode, command_root, damage_kind, mob_effect, gamerule, living_entity, nonliving_entity, mob_category, ai_goal, villager_profession, settlement_subcommand, plugin_package, plugin_command, weather, selector) each mapped to a subject.
- Scenarios: **1432** = 413 pairwise (covers all 2517 valid value pairs of the 9 factors) + 536 baseline (every remaining subject once) + 483 explicit high-risk triples.
- Every value of every dimension (context values, domains, subjects, and every code-enumerated value) appears in at least one scenario (asserted by the generator).
- Existing real-server coverage of scenarios: covered 51 (3.6%), partial 387 (27.0%), unit-only 871 (60.8%), none 123 (8.6%).
- Existing coverage of subjects (any context): real-server e2e 185 (20.8%), e2e admission-only 11 (1.2%), unit-only 597 (67.1%), none 97 (10.9%).
- Driveable today: ready 772 (53.9%), plausible (tools exist, unexercised) 100 (7.0%), missing capability 560 (39.1%).

## Per domain

| domain | subjects | scenarios | covered | partial | unit-only | none | subj e2e | subj e2e-partial | subj unit-only | subj none | driver ready | server gaps |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| session | 21 | 23 | 1 | 3 | 19 | 0 | 3 | 0 | 18 | 0 | 5 | 0 |
| movement | 15 | 19 | 2 | 4 | 13 | 0 | 4 | 0 | 11 | 0 | 7 | 0 |
| block_break | 24 | 38 | 3 | 6 | 29 | 0 | 4 | 0 | 20 | 0 | 24 | 0 |
| block_place | 24 | 28 | 6 | 7 | 15 | 0 | 9 | 0 | 15 | 0 | 19 | 1 |
| block_interact | 29 | 78 | 4 | 31 | 43 | 0 | 9 | 0 | 20 | 0 | 58 | 0 |
| containers | 33 | 268 | 9 | 122 | 137 | 0 | 13 | 3 | 17 | 0 | 116 | 2 |
| inventory | 18 | 46 | 6 | 22 | 18 | 0 | 9 | 0 | 9 | 0 | 21 | 0 |
| crafting | 17 | 17 | 4 | 7 | 6 | 0 | 11 | 0 | 6 | 0 | 9 | 0 |
| items | 18 | 26 | 1 | 5 | 20 | 0 | 4 | 0 | 14 | 0 | 15 | 3 |
| combat | 14 | 18 | 0 | 10 | 8 | 0 | 6 | 0 | 8 | 0 | 7 | 0 |
| death_respawn | 13 | 30 | 5 | 7 | 18 | 0 | 3 | 0 | 10 | 0 | 18 | 0 |
| spawning | 19 | 21 | 0 | 1 | 20 | 0 | 1 | 0 | 18 | 0 | 11 | 0 |
| villagers | 33 | 39 | 0 | 5 | 30 | 4 | 5 | 0 | 24 | 4 | 10 | 0 |
| farming | 18 | 27 | 1 | 8 | 18 | 0 | 3 | 0 | 15 | 0 | 16 | 0 |
| redstone | 17 | 23 | 0 | 10 | 13 | 0 | 6 | 1 | 10 | 0 | 15 | 0 |
| fluids | 12 | 19 | 1 | 5 | 13 | 0 | 1 | 0 | 11 | 0 | 10 | 1 |
| falling_blocks | 6 | 13 | 0 | 0 | 13 | 0 | 0 | 0 | 6 | 0 | 2 | 0 |
| explosions | 11 | 15 | 0 | 5 | 10 | 0 | 1 | 0 | 10 | 0 | 4 | 0 |
| fire | 5 | 11 | 0 | 0 | 11 | 0 | 0 | 0 | 5 | 0 | 2 | 0 |
| weather_time | 8 | 11 | 0 | 3 | 8 | 0 | 1 | 0 | 7 | 0 | 3 | 0 |
| portals_dimensions | 11 | 17 | 0 | 1 | 16 | 0 | 1 | 0 | 10 | 0 | 0 | 0 |
| chat | 10 | 11 | 0 | 2 | 9 | 0 | 2 | 0 | 8 | 0 | 2 | 0 |
| scoreboard_teams | 7 | 9 | 0 | 0 | 9 | 0 | 0 | 0 | 7 | 0 | 0 | 0 |
| world_border | 6 | 9 | 0 | 0 | 9 | 0 | 0 | 0 | 6 | 0 | 0 | 0 |
| plugins | 23 | 38 | 2 | 23 | 13 | 0 | 8 | 4 | 11 | 0 | 24 | 1 |
| block_log | 5 | 12 | 1 | 11 | 0 | 0 | 5 | 0 | 0 | 0 | 3 | 0 |
| advancements_stats | 5 | 9 | 0 | 5 | 4 | 0 | 3 | 0 | 2 | 0 | 0 | 0 |
| ui_presentation | 8 | 9 | 0 | 7 | 2 | 0 | 6 | 0 | 2 | 0 | 1 | 0 |
| entities_misc | 12 | 28 | 0 | 4 | 24 | 0 | 1 | 0 | 11 | 0 | 8 | 2 |
| sculk_trials | 7 | 9 | 0 | 1 | 8 | 0 | 1 | 0 | 6 | 0 | 0 | 0 |
| special_blocks | 8 | 9 | 0 | 0 | 8 | 1 | 0 | 0 | 7 | 1 | 0 | 1 |
| world_chunks | 10 | 12 | 0 | 9 | 3 | 0 | 7 | 0 | 3 | 0 | 2 | 0 |
| persistence | 7 | 14 | 1 | 9 | 4 | 0 | 4 | 0 | 3 | 0 | 2 | 0 |
| loader_content | 7 | 8 | 0 | 3 | 5 | 0 | 2 | 0 | 5 | 0 | 0 | 0 |
| computer | 9 | 14 | 0 | 0 | 14 | 0 | 0 | 0 | 9 | 0 | 0 | 0 |
| monitor | 4 | 9 | 0 | 0 | 9 | 0 | 0 | 0 | 4 | 0 | 0 | 0 |
| voice | 6 | 8 | 0 | 0 | 8 | 0 | 0 | 0 | 6 | 0 | 0 | 2 |
| resource_packs | 5 | 9 | 0 | 0 | 9 | 0 | 0 | 0 | 5 | 0 | 0 | 0 |
| commands | 77 | 78 | 0 | 10 | 66 | 2 | 10 | 0 | 65 | 2 | 68 | 1 |
| selectors | 18 | 19 | 0 | 0 | 13 | 6 | 0 | 0 | 13 | 5 | 9 | 0 |
| damage | 39 | 39 | 0 | 2 | 31 | 6 | 2 | 0 | 31 | 6 | 31 | 6 |
| effects | 40 | 74 | 0 | 0 | 28 | 46 | 0 | 0 | 15 | 25 | 65 | 0 |
| gamerules | 58 | 62 | 0 | 1 | 38 | 23 | 1 | 0 | 35 | 22 | 54 | 0 |
| mobs | 92 | 92 | 4 | 8 | 71 | 9 | 9 | 3 | 71 | 9 | 84 | 0 |
| settlements | 39 | 42 | 0 | 9 | 8 | 25 | 9 | 0 | 8 | 22 | 29 | 0 |
| load | 22 | 22 | 0 | 21 | 0 | 1 | 21 | 0 | 0 | 1 | 18 | 0 |

Scenario coverage % counts only the exact context match; `subj e2e` counts subjects with at least one real-server test in any context.

## Top missing bot capabilities (ordered by scenarios unlocked)

Greedy order: each step adds the capability that makes the most additional scenarios fully driveable (scenarios with <= 3 missing capabilities considered).

| # | capability | cumulative scenarios unlocked | scenarios blocked by it | sole blocker | drivers lacking it (bot/java/loader/native) | evidence |
|---|---|---|---|---|---|---|
| 1 | `server_kill9_wal_replay` | 82 | 134 | 82 | MMMM | no harness kills+replays and then re-verifies; only core unit persistence_crash_tests |
| 2 | `duplicate_login_same_account` | 151 | 110 | 59 | PMMM | two bot processes with one name plausible; no harness scenario |
| 3 | `server_hot_backup` | 201 | 98 | 46 | MMMM | no harness drives edge backup/hot_backup and restores |
| 4 | `multi_client_N` | 252 | 105 | 30 | AMMP | player_scale_bots N; native-scale canaries beside bots |
| 5 | `slow_reader_backpressure` | 301 | 72 | 24 | AMMM | player_scale_bots MIX-08 reader_rate_limit |
| 6 | `malformed_packet_injection` | 335 | 56 | 14 | PAMM | m94-03f malformed container rejection (java agent); wire_probe raw writer plausible |
| 7 | `server_pause_empty` | 375 | 56 | 15 | AMMM | pause-empty functional-native gate (protocol join) |
| 8 | `svc_client_profile` | 420 | 48 | 4 | MMMM | no Fabric+SVC client profile in loader-live/dh-gate |
| 9 | `windows_mcp_lane` | 465 | 46 | 12 | MMMM | no harness lane runs native MCP on Windows (TCP transport is portable) |
| 10 | `reconnect_mid_action` | 500 | 48 | 31 | MAPA | disconnect/connect at a chosen step (playable-36, native-lifecycle); bots only auto-reconnect after transport drop |
| 11 | `timed_mining_survival` | 506 | 6 | 5 | MAPA | break_block/mine_until_crack; bots send StartDestroyBlock only |
| 12 | `container_button` | 512 | 6 | 3 | MAPP | click_container_button (m94-08 enchanting); bots lack ContainerButtonClick |
| 13 | `click_advanced_modes` | 517 | 5 | 1 | MMMM | no driver issues Swap/Clone/Throw/QuickCraft/PickupAll container clicks |
| 14 | `admin_protocol_client` | 522 | 5 | 4 | PMMM | status/query/rcon/msmp/dashboard clients: raw protocol tooling plausible (wire_probe status), no gate |
| 15 | `select_trade` | 527 | 5 | 2 | MPPP | merchant offer click unexercised (native-v2-p1-trader admission only); bots lack SelectTrade |

Other missing capabilities by scenarios blocked: `portal_travel` 4, `crafter_slot_toggle` 4, `svc_voice` 3, `native_voice` 3, `vehicle_control` 3, `read_hud_ui` 2, `loader_bundle_handshake` 2, `text_input_screen` 2, `set_beacon` 2, `swap_hands_offhand` 2, `book_edit` 1, `advancement_stats_ui` 1, `resource_pack_prompt` 1, `screenshot_visual` 1, `velocity_proxy` 1, `elytra_glide` 1, `release_use_item` 1

### Unproven capabilities (tool/packet exists for the scenario's driver, but no existing scenario exercises it)

| capability | scenarios needing proof | drivers (bot/java/loader/native) | evidence |
|---|---|---|---|
| `loader_platform_neoforge_forge` | 89 | MMPM | loader-live accepts --platform neoforge|forge; only fabric scenario recorded |
| `multi_client_2` | 79 | AAPA | two-client regression driver; native p1 observer pair; bots |
| `server_clean_restart` | 52 | AAPA | catalog-load checkpoint-restart; regression before/after phases; native_v2 owned restart |
| `reconnect_mid_action` | 47 | MAPA | disconnect/connect at a chosen step (playable-36, native-lifecycle); bots only auto-reconnect after transport drop |
| `use_item_on_block` | 36 | AAPA | minecraft_use_item_on; bot use_block |
| `multi_client_N` | 32 | AMMP | player_scale_bots N; native-scale canaries beside bots |
| `server_save_all` | 31 | AAPA | console/op save-all in pause-empty, light-seam, catalog-load |
| `select_trade` | 31 | MPPP | merchant offer click unexercised (native-v2-p1-trader admission only); bots lack SelectTrade |
| `open_container` | 16 | AAPA | use_item_on container; bot open_container |
| `walk_move` | 16 | AAPA | navigate_to_block/press_inputs; bot movement windows |
| `place_block` | 15 | AAPA | use_item_on with block; bot use_block |
| `click_pickup_quickmove` | 13 | AAPA | click/quick_move_container_slot; bot click_slot Pickup|QuickMove |
| `portal_travel` | 13 | MPPP | stand-in-portal travel unexercised (native-v2-dimensions uses commands) |
| `interact_entity` | 13 | AAPA | interact_entity; bot entity_interact |
| `read_hud_ui` | 13 | MPPA | observe HUD nodes (admin-commands native); bots see packets only |
| `malformed_packet_injection` | 12 | PAMM | m94-03f malformed container rejection (java agent); wire_probe raw writer plausible |
| `duplicate_login_same_account` | 11 | PMMM | two bot processes with one name plausible; no harness scenario |
| `vehicle_control` | 11 | MPPP | interact + press_inputs riding unexercised; bots lack MoveVehicle/PaddleBoat |
| `console_command` | 10 | APPA | harness stdin console (pause-empty, admin fixtures) |
| `attack_entity` | 10 | AAPA | attack_entity_once; bot attack_entity |
| `resource_pack_prompt` | 8 | MPPP | client accepts/declines via screen buttons unexercised; bots lack ResourcePack status reply |
| `respawn` | 7 | AAPA | minecraft_respawn; bot respawn |
| `set_beacon` | 7 | MPPP | beacon confirm via click_screen_button unexercised; bots lack SetBeacon |
| `container_button` | 7 | MAPP | click_container_button (m94-08 enchanting); bots lack ContainerButtonClick |
| `crafter_slot_toggle` | 7 | MPPP | slot toggle click unexercised; bots lack ContainerSlotStateChanged |
| `timed_mining_survival` | 5 | MAPA | break_block/mine_until_crack; bots send StartDestroyBlock only |
| `use_item_air` | 5 | AAPA | press_inputs use; bot use_item |
| `swap_hands_offhand` | 5 | MPPP | press_inputs key F (unexercised); bots lack SwapItemWithOffhand |
| `computer_channel` | 4 | MMPM | loader-fabric ComputerClient/ComputerScreen exist; no harness scenario; native client lacks krasnokamen:computer |
| `release_use_item` | 3 | MAPA | press_inputs hold/release (native-v2-ranged, playable-26); bots lack ReleaseUseItem |
| `creative_instant_break` | 3 | AAPA | break_block in creative; bot break_block |
| `advancement_stats_ui` | 3 | MPPA | native-v2-p4-advancements/statistics |
| `recipe_book_place` | 3 | MPPP | read_recipe_book exists, placement unexercised; bots lack PlaceRecipe |
| `native_screen_share` | 3 | MMMP | krasnokamen:screen relay in deploy-final edge (screen.rs/screen_service.rs); native client branches implement capture/share (screen-codec, screen-audio); no MCP tools for share consent/request/answer yet; Java/Loader clients do not speak krasnokamen:screen |
| `native_voice` | 2 | MMMP | krasnokamen:voice relay in deploy-final edge (voice.rs/voice_service.rs); native client 'voice' branch; no MCP push-to-talk/listen tools yet; only native clients are served |
| `elytra_glide` | 2 | MPPP | press_inputs jump mid-air unexercised; bots lack StartFallFlying |
| `dialog_interaction` | 2 | MPAA | click_screen_button/set_screen_input (dialogs, loader settlements smoke) |
| `creative_slot_set` | 2 | APPP | bots SetCreativeModeSlot (setup); MCP creative tab clicks unexercised |
| `text_input_screen` | 1 | MAPA | set_screen_input (native-v2-p1-signs; java playable-17 sign edit); bots lack SignUpdate/RenameItem |
| `online_mode_auth` | 1 | MPMP | java 'client-token' prerequisite exists; no online-mode gate |
| `transfer_follow` | 1 | MPPP | client follows Transfer unexercised |
| `jump_sneak_sprint_inputs` | 1 | PAPA | press_inputs keys; bots send PlayerCommand sprint/PlayerInput |

## Server gaps surfaced while enumerating (vanilla behaviour with no server code hit)

- `block_place.spawn_protection_radius` (no_code_hit): vanilla: refused for non-ops within radius
- `containers.mount_inventory` (server_missing): vanilla: horse screen with saddle/armor/chest slots
- `containers.lectern` (server_missing): vanilla: lectern menu page turning + take button
- `items.totem_of_undying` (no_code_hit): vanilla: death prevented, health 1, regen/absorption/fire res, totem consumed
- `items.spyglass_goat_horn` (no_code_hit): vanilla: zoom (client) / horn sound + cooldown
- `items.recovery_compass_lodestone` (no_code_hit): vanilla: points to last death / lodestone
- `fluids.sponge` (no_code_hit): vanilla: absorbs water within 7, becomes wet sponge
- `plugins.plugin_startup_fail_closed` (not_deployable): startup refuses with clear error
- `entities_misc.paintings` (no_code_hit): vanilla: random fitting variant; drops on support break
- `entities_misc.happy_ghast_harness` (no_code_hit): vanilla 1.21.6+: harnessed flying mount for up to 4
- `special_blocks.decorated_pot_chiseled_bookshelf` (no_code_hit): vanilla: single-stack storage / 6 book slots with comparator
- `voice.proximity_voice` (server_missing): SVC proximity audio. Not served: Krasnokamen has no SVC protocol bridge (its voice is native krasnokamen:voice only)
- `voice.group_voice` (server_missing): SVC group audio. Not served: no SVC bridge
- `commands.data` (server_missing): vanilla 26.1.2 (pinned by DataCommands from the jar): root/unscaled -> '<name> has the following entity data: <SNBT>' / 'x, y, z has the following block data: <SNBT>'; scaled numeric -> '<path> on <name> after scale factor of <%.2f> is <floor(v*scale)>' / '<path> on block x, y, z after scale factor of <%.2f> is <int>'; result numeric=floor, list/compound=size, string=UTF-16 length, root=1; zero or multiple path matches fail; 26.1 paths (active_effects, equipment, respawn). Krasnokamen advertises the root but has no handler yet (server gap; DataCommands implementing); below op 2: root not advertised and 'Unknown or incomplete command'
- `damage.vanilla_freeze` (no_code_hit): vanilla: freeze damage (freezeDamage gamerule)
- `damage.vanilla_fly_into_wall` (no_code_hit): vanilla: fly_into_wall kinetic damage
- `damage.vanilla_cramming` (no_code_hit): vanilla: cramming damage
- `damage.vanilla_sonic_boom` (no_code_hit): vanilla: sonic_boom ignores armor/shield
- `damage.vanilla_dragon_breath` (no_code_hit): vanilla: dragon_breath magic damage
- `damage.vanilla_stalagmite` (no_code_hit): vanilla: stalagmite damage (fall x2)

## Divergences/notes found in code (not memory)

- Duplicate login: `crates/mc-core/src/tick/input/session.rs` join refuses the NEW connection ('UUID is already joined'); vanilla kicks the EXISTING session. Decide policy; triples `disconnect_x_*:duplicate_login` encode both.
- `DamageKind` (crates/mc-core/src/survival.rs) has no freeze, fly_into_wall, cramming, sonic_boom, dragon_breath or stalagmite kinds; listed as `damage.vanilla_*` gap subjects.
- Illusioner falls to the generic `Melee` combat policy (crates/mc-data/src/mob_behavior_26_1_2.rs combat_policy); vanilla illusioner is a ranged spellcaster.
- No server-side Simple Voice Chat bridge in any crate; `fabric_svc` can only test join/compat today.
- Computer terminal exists only in Fabric Loader (loader-core ComputerProtocol, loader-fabric ComputerClient); native client has no `krasnokamen:computer`; screen sharing to other players is excluded by ADR 0018.
- No horse/donkey/llama inventory screen and no lectern menu in `MenuKind` / container mapping (crates/mc-core/src/tick.rs container_kind_for_state).
- No spawn-protection key in server.properties support (crates/mc-edge/src/server_properties.rs).
- A legacy API 0.6.0 manifest is refused by manifest validation (`UnsupportedScriptApiVersion`, host contract 0.9.0) before its `entry` is resolved; the synthetic fixture `tools/fixtures/legacy-api-0.6` is meant for scenario `plugins.plugin_startup_fail_closed`, whose driver step (`plugin_invalid_startup`) is not implemented yet. The v1 packages land-claims/online-roster/geological-mines were deleted by the D66 duplicate-plugin audit (2026-11-19; receipts/PluginDupAudit/AUDIT.md).

## Full combinatorial set (all combinations)

- Unconstrained product: 890 subjects x 279,936 contexts (9 clients x 4 modes x 3 dimensions x 4 difficulties x 3 player counts x 6 network x 6 lifecycle x 6 permissions) = **251,662,464**.
- Removed by the invalid constraints (each cell counted once, at the first violated rule): C01 3,732,480, C02 4,572,288, C03 36,593,856, C04 8,370,432, C05 27,026,208.
- **Valid combinations: 171,367,200** (negative-oracle, server-gap and procedure cells included). Wave 1 = 1432 scenarios on 1411 distinct cells.

| wave | definition | cells |
|---|---|---|
| 1 | wave 1: the matrix.yaml scenarios (pairwise + baseline + explicit high-risk triples) | 1,411 |
| 2 | dupes: custody subject (items change owner) with 2/8 players, a non-normal network condition or a non-steady server lifecycle | 29,347,073 |
| 3 | saves: any non-steady server lifecycle (save-all, clean restart, kill -9 + WAL replay, hot backup, pause/resume) | 118,106,086 |
| 4 | disconnects: any non-normal network condition | 18,898,400 |
| 5 | multiplayer: 2 or 8 players | 3,198,730 |
| 6 | rest: 1 player, normal network, steady server; varies client, game mode, dimension, difficulty, permission | 1,815,500 |

| domain | valid cells | wave 1 | wave 2 | wave 3 | wave 4 | wave 5 | wave 6 |
|---|---|---|---|---|---|---|---|
| session | 4,298,400 | 23 | 192,240 | 3,419,991 | 547,198 | 91,200 | 47,748 |
| movement | 2,613,600 | 19 | 0 | 2,177,993 | 348,475 | 58,080 | 29,033 |
| block_break | 4,665,600 | 34 | 768,950 | 3,239,993 | 518,399 | 86,400 | 51,824 |
| block_place | 4,665,600 | 28 | 192,238 | 3,725,994 | 596,158 | 99,357 | 51,825 |
| block_interact | 5,546,880 | 70 | 1,769,006 | 3,131,995 | 501,119 | 83,512 | 61,178 |
| containers | 7,698,240 | 266 | 7,626,718 | 0 | 0 | 0 | 71,256 |
| inventory | 3,888,000 | 43 | 3,272,369 | 486,000 | 77,760 | 12,959 | 38,869 |
| crafting | 3,926,880 | 17 | 3,466,793 | 356,398 | 58,320 | 8,640 | 36,712 |
| items | 3,538,080 | 26 | 423,355 | 2,591,994 | 414,716 | 69,120 | 38,869 |
| combat | 2,656,800 | 18 | 0 | 2,213,993 | 354,234 | 60,479 | 28,076 |
| death_respawn | 3,188,160 | 27 | 1,038,950 | 1,781,992 | 285,118 | 47,520 | 34,553 |
| spawning | 3,304,800 | 21 | 0 | 2,753,991 | 440,638 | 73,440 | 36,710 |
| villagers | 6,985,440 | 39 | 4,544,635 | 1,997,992 | 319,679 | 53,279 | 69,816 |
| farming | 3,369,600 | 27 | 192,237 | 2,645,989 | 423,360 | 70,557 | 37,430 |
| redstone | 3,304,800 | 23 | 384,474 | 2,429,994 | 388,799 | 64,800 | 36,710 |
| fluids | 2,203,200 | 19 | 192,235 | 1,673,991 | 267,839 | 44,640 | 24,476 |
| falling_blocks | 1,101,600 | 13 | 0 | 917,988 | 146,880 | 24,480 | 12,239 |
| explosions | 2,138,400 | 15 | 0 | 1,781,990 | 285,119 | 47,518 | 23,758 |
| fire | 972,000 | 11 | 0 | 809,991 | 129,599 | 21,599 | 10,800 |
| weather_time | 712,800 | 11 | 0 | 593,992 | 95,037 | 15,840 | 7,920 |
| portals_dimensions | 1,101,600 | 17 | 0 | 917,988 | 146,877 | 24,480 | 12,238 |
| chat | 1,749,600 | 11 | 0 | 1,457,993 | 233,278 | 47,519 | 10,799 |
| scoreboard_teams | 1,296,000 | 9 | 0 | 1,079,993 | 172,799 | 30,239 | 12,960 |
| world_border | 1,399,680 | 9 | 0 | 1,166,393 | 186,622 | 31,104 | 15,552 |
| plugins | 4,186,080 | 37 | 552,946 | 3,023,994 | 483,838 | 86,396 | 38,869 |
| block_log | 1,166,400 | 12 | 230,685 | 777,594 | 124,414 | 20,736 | 12,959 |
| advancements_stats | 972,000 | 9 | 0 | 809,992 | 129,599 | 21,600 | 10,800 |
| ui_presentation | 1,425,600 | 9 | 0 | 1,187,993 | 190,079 | 34,560 | 12,959 |
| entities_misc | 2,332,800 | 28 | 576,706 | 1,457,994 | 233,278 | 38,880 | 25,914 |
| sculk_trials | 1,360,800 | 9 | 0 | 1,133,993 | 181,438 | 30,240 | 15,120 |
| special_blocks | 1,594,080 | 9 | 231,118 | 1,133,994 | 181,439 | 30,240 | 17,280 |
| world_chunks | 1,810,080 | 12 | 231,117 | 1,313,995 | 210,240 | 35,039 | 19,677 |
| persistence | 1,360,800 | 14 | 384,472 | 809,994 | 129,600 | 21,600 | 15,120 |
| loader_content | 691,200 | 8 | 0 | 575,994 | 92,160 | 15,359 | 7,679 |
| computer | 429,120 | 14 | 0 | 357,594 | 57,358 | 11,037 | 3,117 |
| monitor | 583,200 | 9 | 384,475 | 161,996 | 25,920 | 4,320 | 6,480 |
| voice | 151,200 | 8 | 0 | 125,995 | 20,159 | 4,798 | 240 |
| resource_packs | 972,000 | 9 | 0 | 809,992 | 129,599 | 21,600 | 10,800 |
| commands | 17,133,120 | 78 | 0 | 14,277,592 | 2,284,415 | 400,885 | 170,150 |
| selectors | 4,043,520 | 19 | 0 | 3,369,591 | 539,135 | 96,767 | 38,008 |
| damage | 7,581,600 | 39 | 0 | 6,317,994 | 1,010,878 | 168,479 | 84,210 |
| effects | 7,776,000 | 74 | 0 | 6,479,992 | 1,036,799 | 172,766 | 86,369 |
| gamerules | 13,530,240 | 62 | 0 | 11,275,193 | 1,804,031 | 300,667 | 150,287 |
| mobs | 17,884,800 | 92 | 0 | 14,903,992 | 2,384,639 | 397,440 | 198,637 |
| settlements | 7,581,600 | 42 | 2,691,354 | 4,049,995 | 647,999 | 108,000 | 84,210 |
| load | 475,200 | 22 | 0 | 395,995 | 63,360 | 10,559 | 5,264 |
| **total** | **171,367,200** | **1,411** | **29,347,073** | **118,106,086** | **18,898,400** | **3,198,730** | **1,815,500** |

Ids: CELL = `subject/client_kind/game_mode/dimension/difficulty/player_count/network_condition/server_lifecycle/permission`; wave-1 id = `<pw|bl|tr.<origin>>:CELL`; full-set id = `F:CELL`. Content-derived, so they stay stable.
Queue: `python3 gen_matrix.py --full [--wave 2] [--client protocol_bot] [--driver native_mcp] [--domain containers] [--pending] [--limit N]` streams `wave<TAB>id` lazily (no file of all cells); waves 2..6 iterate contexts outer->inner server_lifecycle > network_condition > player_count > client_kind > game_mode > difficulty > permission > dimension with subjects innermost, so the first ids of wave 2 are kill-9/disconnect-during-menu dupes on every custody subject.

## Ledger contract (`ledger.jsonl`)

- One JSON object per line with exactly the keys `id, status, driver, server_sha, run_dir, ts, note`; append-only; the last line for an id wins.
- `status`: pass | fail | blocked | skipped. `driver` must equal the driver of the id's client_kind. `server_sha`: full sha256 of the release krasnokamen binary. `run_dir`: absolute harness run dir. `ts`: UTC ISO-8601 with `Z`. `note`: fail -> finding + evidence path; blocked -> missing capability name(s).
- Write through `python3 gen_matrix.py --record --id ID --status S --driver D --server-sha SHA --run-dir DIR [--note T]` or `--record-file batch.jsonl` (validates, flock on ledger.jsonl, one O_APPEND write, fsync). Ledger lines come from harness receipts; this tool does not run scenarios.
- `python3 gen_matrix.py --status` prints executed (pass+fail) / remaining for pairwise, baseline, triples, wave 1, waves 2..6 and the full set, plus wave 1 per client; exits 1 on any invalid line.

## How to use

- Execution path: tools/harness (single `cmd_run`, shared lifecycle/receipts, catalog_mixes for composite bot scenarios) on the release binary with `[plugins].directory = /run/media/kaiserroman/games/solaris-work/scenarios/live-plugins` (live VPS set of 10 plugins; see `scenario_template` for its global effects: auth login, graves, essentials /msg).
- Pick from `--full --wave 1 --client <yours> --pending`; `--show ID` prints the scenario (steps/oracle/driver/coverage) for any wave-1 or F: id.
- Re-generate after code changes: `python3 tools/harness/matrix_specs/gen_matrix.py` (the external mutable ledger is never truncated).
- Freeze: `MATRIX_FREEZE.json` pins the wave-1 id set (`wave1_id_set_sha256` 711552f5f156bbd7…, frozen 2026-11-14T22:09:25+07:00); a regenerate that would change it exits 1 without writing; `--refreeze` accepts the change and must be announced to the runners. Record the id-set hash per batch.
- Plugin profiles: 9 plugin-sensitive subjects (death_respawn.keep_inventory_false [graves], death_respawn.death_while_menu_open [graves], death_respawn.death_while_riding [graves], death_respawn.void_death [graves], commands.msg [krasnokamen-essentials], chat.private_message [krasnokamen-essentials], selectors.nonop_selector_msg [krasnokamen-essentials], session.mute_warn [krasnokamen-essentials], session.join_fresh_spawn [krasnokamen-auth]) carry @vanilla (live set minus the listed plugins) or @live (full live set) in every id; their ledger lines need `plugin_set` = `gen_matrix.py --plugin-fingerprint <deployed dir>` (pass `--plugin-set FP` to --record); expected fingerprints are in `ledger_contract.plugin_profiles`. Profile-less lines recorded earlier for these subjects are reported as LEGACY by --status and not counted; lines whose plugin_set is no longer their profile's fingerprint (an older plugin build) are reported as OUTDATED and not counted.
- Coverage cites real-server tests from `existing.py` (harness profiles/scenario ids) and unit test files by module; `e2e_other_context` shows the nearest existing test.
