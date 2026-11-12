"""Curated gameplay subjects per domain, grounded in Krasnokamen source modules.

Every subject names the module(s) implementing it (`core:` = crates/mc-core/src,
`edge:` = crates/mc-edge/src, `script:` = crates/mc-script/src,
`dp:` = ../solaris-default-plugins, `ldr:` = ../solaris-loader). Code-enumerated
families (commands, menus, damage kinds, effects, mobs, gamerules, ...) are
expanded by gen_matrix.py from the source files themselves; this file only
carries the per-subject action/oracle text that code cannot supply.

Tuple form: (name, do, expect, extra) where extra may hold:
  caps      extra required driver capabilities
  src       extra/override source modules
  menu      True: an open menu exists during the action (valid for disconnect_during_menu)
  custody   True: items change owner (dupe/loss risk)
  mp        True: needs >= 2 players
  dims      allowed dimensions (default all)
  clients   allowed client kinds (default all)
  op        minimum permission level for the tested action (default 0)
  status    implemented | no_code_hit | server_missing | not_deployable
  visual    True: oracle needs rendered/visual evidence
  plugins   required plugin packages
  inflight  True: leaves in-flight world state (restart-sensitive)
"""

D = {}


def domain(name, sources, caps, subjects, **defaults):
    D[name] = {"sources": sources, "caps": caps, "defaults": defaults, "subjects": subjects}


# ---------------------------------------------------------------- session / edge
domain("session", ["edge:login.rs", "edge:handshake.rs", "edge:configuration.rs", "core:tick/input/session.rs"], ["join_play"], [
    ("join_fresh_spawn", "fresh account joins a fresh world", "Login(Play) then spawn on solid, non-suffocating ground near world spawn; chunks around spawn loaded; others get yellow join message", {}),
    ("rejoin_restore_state", "move, take damage, gain XP and items, leave, rejoin", "position/rotation/health/food/XP/inventory/effects/game mode restored exactly (vanilla playerdata)", {"custody": True}),
    ("status_ping", "server-list ping (handshake next_state=status)", "status JSON: version 26.1.2 + protocol, MOTD, online/max players, sample; pong echoes payload", {"src": ["edge:status.rs"], "caps": ["admin_protocol_client"]}),
    ("online_mode_auth", "online-mode join with a Mojang-authenticated account", "encryption + session-server verification; profile UUID/skin properties from Mojang; offline accounts refused", {"src": ["edge:crypto.rs", "edge:login.rs"], "caps": ["online_mode_auth"]}),
    ("offline_mode_uuid", "offline-mode join by name", "UUID = name-based v3 (OfflinePlayer:<name>); same name -> same saved player", {}),
    ("whitelist_gate", "enable whitelist; join unlisted; /whitelist add; join again", "unlisted join refused with whitelist message; added player joins", {"op": 3, "src": ["edge:admin.rs"]}),
    ("ban_pardon", "/ban player reason; rejoin; /pardon; rejoin; repeat with /ban-ip", "online target kicked with reason; rejoin refused with ban reason; pardon restores access", {"op": 3, "src": ["edge:admin.rs"]}),
    ("kick", "/kick player reason", "target disconnected with reason; others see leave message; saved state intact", {"op": 3, "src": ["edge:admin.rs"]}),
    ("mute_warn", "/mute target; target chats; /unmute; /warn target", "muted chat and /me,/msg refused natively; unmute restores; warn delivered (Krasnokamen moderation, no vanilla analogue)", {"op": 3, "src": ["edge:admin.rs", "core:boundary.rs"]}),
    ("transfer", "/transfer <host> <port> target (accepts-transfers=true on target)", "client receives Transfer and reconnects to target server; source saves player", {"op": 3, "caps": ["transfer_follow"]}),
    ("connection_limit", "open connections beyond configured per-IP/global limit", "excess connections refused before login; existing players unaffected", {"src": ["edge:conn_limit.rs"], "caps": ["multi_client_N"]}),
    ("protocol_mismatch", "connect with a different protocol version", "login disconnect with outdated client/server message; no player created", {"src": ["edge:handshake.rs"], "clients": ["protocol_bot"]}),
    ("max_players_full", "join when max-players reached", "refused with server-full message (ops bypass is vanilla-optional)", {"src": ["edge:server_properties.rs"], "caps": ["multi_client_N"]}),
    ("view_distance_negotiation", "client render distance 2/8/32 vs server view-distance/simulation-distance", "server sends min(client, server) view; simulation radius bounded; no chunk holes", {"src": ["core:view_negotiation_tests.rs", "edge:server_view_distance_tests.rs"]}),
    ("locale_presentation", "client language ru_ru then en_us", "server-translated Krasnokamen texts follow ClientInformation locale; vanilla translatable components untouched", {"src": ["core:boundary.rs"]}),
    ("rcon", "RCON login and command; wrong password", "authenticated command output returned; bad password rejected; no gameplay authority bypass", {"src": ["edge:rcon.rs"], "caps": ["admin_protocol_client"], "op": 4}),
    ("query_gs4", "GameSpy4 basic/full stat query", "MOTD/map/player list/version returned; malformed query ignored", {"src": ["edge:query.rs"], "caps": ["admin_protocol_client"]}),
    ("velocity_forwarding", "join through Velocity modern forwarding; direct join when proxy-only", "forwarded IP/UUID/properties used; direct/unsigned join refused", {"src": ["edge:proxy.rs"], "caps": ["velocity_proxy"]}),
    ("msmp_management", "management-protocol requests: players, allowlist, bans, save, stop", "JSON-RPC results match server state; auth required", {"src": ["edge:msmp.rs"], "caps": ["admin_protocol_client"]}),
    ("dashboard_metrics", "read dashboard while players act", "player count/TPS/MSPT/memory values match server state; read-only", {"src": ["edge:dashboard.rs"], "caps": ["admin_protocol_client"]}),
    ("server_brand_f3", "open F3 debug overlay", "server brand shows Krasnokamen; client keeps play", {"src": ["edge:configuration.rs"], "visual": True, "caps": ["read_hud_ui"]}),
])

# ---------------------------------------------------------------- movement
domain("movement", ["core:tick/input/movement.rs", "core:contact_hazards.rs"], ["walk_move", "read_world_state"], [
    ("walk_sprint_jump", "walk, sprint, sprint-jump across flat ground and over 1-block steps", "moves accepted without correction; speeds within vanilla bounds (sprint ~5.6 b/s); food exhaustion accrues in survival", {"caps": ["jump_sneak_sprint_inputs"]}),
    ("sneak_edge", "sneak to a block edge and keep pushing outward", "player does not fall off the edge while sneaking (vanilla edge clamp)", {"caps": ["jump_sneak_sprint_inputs"]}),
    ("swim_and_sink", "enter deep water, swim up/down, sprint-swim", "swim pose, buoyancy, air supply decreases underwater", {"src": ["core:entity_motion.rs"]}),
    ("climb", "climb ladder, vine, scaffolding; sneak on ladder", "climbable blocks allow vertical motion; sneak holds position; no fall damage while climbing", {}),
    ("step_up_slab_stairs", "walk onto slabs/stairs", "0.6-block step-up without jump; no correction", {}),
    ("surface_modifiers", "walk on ice/blue ice, slime, honey, soul sand, magma (sneaking)", "slip/bounce/slow per block; soul sand slow; magma hurts unless sneaking", {}),
    ("entangling_blocks", "walk into cobweb, powder snow, sweet berry bush", "slowdown; powder snow freezing (unless leather boots); berry bush damage when moving", {}),
    ("bubble_column", "enter soul-sand and magma bubble columns", "upward / downward drag; boats affected", {"src": ["core:fluids.rs"]}),
    ("chunk_boundary_crossing", "sprint across chunk seams and toward ungenerated chunks", "no rubber-banding; client held until chunk sent; position authority preserved", {"src": ["edge:chunks.rs", "core:chunk_loader.rs"]}),
    ("illegal_move_correction", "send a too-fast / into-block move (raw)", "server corrects with teleport (vanilla moved-too-quickly/wrongly); disablePlayerMovementCheck gamerule bypasses", {"clients": ["protocol_bot"], "caps": ["malformed_packet_injection"]}),
    ("teleport_confirm", "server teleport (tp, ender pearl, respawn) while moving", "moves before ConfirmTeleport ignored; after confirm positions sync", {}),
    ("elytra_glide", "equip elytra, jump from height, glide, use firework", "fall-flying admitted only by server; firework boosts; durability decreases; landing ends glide", {"src": ["core:tick/elytra.rs", "core:elytra_tests.rs"], "caps": ["elytra_glide"]}),
    ("riptide", "use riptide trident in rain / water", "player launched along look vector; no launch when dry", {"src": ["core:projectiles/weapons.rs"], "caps": ["release_use_item"], "dims": ["overworld"]}),
    ("movement_enchantments", "depth strider, soul speed, swift sneak boots/leggings", "speed modifiers applied per enchant level", {"src": ["core:enchantments.rs"]}),
    ("collision_entities", "push against mobs/players; cramming (maxEntityCramming)", "entities push apart; cramming damage over limit", {"src": ["core:tick_collision_tests.rs"]}),
])

# ---------------------------------------------------------------- block break
domain("block_break", ["core:tick/mining.rs", "core:block_drops.rs", "core:block_drop_context.rs", "core:block_tools.rs"], ["break_block", "read_world_state"], [
    ("instant_break", "break flowers, torches, crops, saplings by hand", "breaks in one tick (hardness 0); item drops", {}),
    ("tool_tier_required", "mine stone/iron/diamond/obsidian ore with wood..netherite pickaxes", "drops only with sufficient tier; break time follows hardness/tool speed", {"custody": True}),
    ("wrong_tool_no_drop", "break stone by hand", "slow break, no drop", {}),
    ("silk_touch", "silk-touch ore, glass, grass, ice", "block itself drops", {"src": ["core:enchantments.rs"]}),
    ("fortune_drops", "fortune III on coal/diamond ore and crops", "drop count distribution increased per vanilla formula", {}),
    ("container_spill", "break a filled chest/barrel/furnace/hopper", "all contents drop as item entities exactly once; block item drops", {"custody": True}),
    ("shulker_keeps_contents", "break filled shulker box", "shulker item drops with container contents component; no spill", {"custody": True, "src": ["core:shulker.rs"]}),
    ("double_block", "break top/bottom of door, bed, tall flower", "both halves removed; one item drop; creative bottom-half no drop", {}),
    ("support_cascade", "break block under torch/rail/redstone/sign", "attached blocks pop and drop", {"src": ["core:placement.rs"]}),
    ("gravity_trigger", "break block under sand/gravel stack", "falling blocks fall and land", {"src": ["core:falling.rs"]}),
    ("ice_to_water", "break ice above a solid block (not silk)", "ice becomes water source (not in nether)", {}),
    ("leaves_shears", "break leaves by hand vs shears", "hand: sapling/stick/apple chance; shears: leaf block", {"src": ["core:leaves.rs"]}),
    ("glass_no_drop", "break glass/panes without silk touch", "no drop", {}),
    ("unbreakable", "attempt bedrock/barrier/end portal frame in survival", "never breaks; no drop; client resynced", {}),
    ("creative_break", "creative break any block; creative with sword", "instant break, no drops; sword cannot break blocks", {}),
    ("ore_xp", "mine coal/lapis/redstone/diamond/emerald/quartz/nether gold ore", "XP orbs spawn per vanilla range (none with silk)", {"src": ["core:experience_orbs.rs"]}),
    ("spawner_xp", "break monster spawner", "15-43 XP, no item drop", {"src": ["core:monster_spawner.rs"]}),
    ("block_entity_drop", "break sign/banner/skull/bee nest", "block entity data kept where vanilla keeps it (banner patterns, bees with silk)", {"src": ["core:block_entity_defaults.rs"]}),
    ("crop_drops_by_age", "break wheat/carrot/potato/beetroot at age 0 and max", "immature drops seed only; mature drops produce + seeds", {"src": ["core:farming.rs"]}),
    ("waterlogged_break", "break waterlogged slab/stairs", "water source remains", {"src": ["core:fluids.rs"]}),
    ("infested_block", "break infested stone", "silverfish spawns; no drop", {}),
    ("mining_speed_modifiers", "mine with haste/mining fatigue, underwater, in air, aqua affinity", "break time scales per vanilla multipliers", {"src": ["core:effects.rs", "core:enchantments.rs"]}),
    ("abort_dig", "start digging then abort/look away", "block intact; no drop; crack animation clears", {"caps": ["timed_mining_survival"]}),
    ("tool_durability_on_break", "break blocks with a 1-durability tool", "durability consumed per block; tool breaks with break sound/animation", {"custody": True}),
])

# ---------------------------------------------------------------- block place
domain("block_place", ["core:placement.rs", "core:placement_shapes.rs", "core:build_precommit.rs"], ["place_block", "read_world_state"], [
    ("full_block", "place a full block on each face", "state placed; item debited (not in creative); neighbors updated", {"custody": True}),
    ("slab_double", "place slab top/bottom then second slab into it", "type bottom/top then double; one item each", {}),
    ("stairs_shapes", "place stairs in corners", "facing/half/shape (inner/outer) per neighbors", {}),
    ("door", "place door", "two halves, hinge by neighbors, facing player", {}),
    ("trapdoor", "place trapdoor on upper/lower half of face", "half top/bottom; facing", {}),
    ("fence_wall_pane_connect", "place fences/walls/panes/bars around blocks", "connection properties update both sides; wall post rules", {}),
    ("fence_gate", "place fence gate between walls", "in_wall lowered; facing", {}),
    ("bed", "place bed", "head+foot; refused when foot space blocked", {}),
    ("attached_switches", "place button/lever on floor/wall/ceiling", "face property; refused on non-sturdy face", {}),
    ("torch_variants", "place torch/soul torch/redstone torch on floor and wall", "torch vs wall_torch selection; refused on non-sturdy", {}),
    ("signs_all_kinds", "place standing/wall/hanging/wall-hanging signs", "rotation/facing per player; editor opens", {"src": ["core:signs.rs"]}),
    ("banner", "place standing and wall banner", "rotation 0-15 / facing", {}),
    ("rail_shapes", "place rails/powered/detector/activator rails in curves and slopes", "shape property auto-connects", {"src": ["core:redstone/rails.rs"]}),
    ("support_required_blocks", "place carpet, pressure plate, flower, sapling on invalid ground", "refused or popped per canSurvive", {}),
    ("directional_blocks", "place observer, piston, dispenser, repeater, comparator, anvil, shulker box, end/lightning rod, amethyst cluster", "facing derived from look/face per vanilla", {}),
    ("waterlogged_place", "place slab/stairs/fence into water", "waterlogged=true; water keeps flowing logic", {"src": ["core:fluids.rs"]}),
    ("sneak_place_on_interactive", "sneak-place a block against chest/crafting table", "places block instead of opening menu", {"caps": ["jump_sneak_sprint_inputs"]}),
    ("place_into_entity_refused", "place block where a player/mob stands", "refused; item kept; client resynced", {}),
    ("out_of_reach_refused", "place/break > reach distance or behind wall (raw)", "refused + BlockChangedAck + resync; no item loss", {"caps": ["malformed_packet_injection"]}),
    ("build_height_limits", "place at y=319/-64 in overworld and at nether/end limits", "refused above/below limits with action-bar message", {}),
    ("replaceable_targets", "place into tall grass, snow layer, water, lava", "replaces replaceable block", {}),
    ("rejected_resync", "client predicts a placement the server refuses (protected zone/occupied)", "server answers block update + ack; client ghost removed; item restored", {"src": ["core:build_precommit.rs"]}),
    ("adventure_can_place_on", "adventure mode with can_place_on component", "placement only on listed blocks", {"src": ["core:placement.rs"]}),
    ("spawn_protection_radius", "non-op places/breaks within spawn-protection radius", "vanilla: refused for non-ops within radius", {"status": "no_code_hit"}),
])

# ---------------------------------------------------------------- block interaction
domain("block_interact", ["core:tick/input/interact.rs", "core:placement.rs"], ["use_item_on_block", "read_world_state"], [
    ("door_trapdoor_gate_toggle", "use wooden door/trapdoor/fence gate; try iron door by hand", "open toggles both halves; iron variants ignore hand", {}),
    ("lever_button", "use lever and stone/wooden button", "lever toggles powered; button powers 20/30 ticks", {"src": ["core:redstone/signal.rs"]}),
    ("note_block_tune", "use note block repeatedly; place over different blocks", "note 0..24 cycles; instrument by block below", {}),
    ("repeater_comparator_daylight", "use repeater, comparator, daylight detector", "delay 1..4 cycles; compare/subtract toggles; inverted toggles", {}),
    ("jukebox", "insert/eject music disc", "has_record true; disc ejected as one stack; comparator output", {"src": ["core:jukebox.rs"], "custody": True}),
    ("sign_edit_decorate", "edit sign text; apply dye, glow ink, honeycomb wax; edit waxed", "text stored; color/glow applied; wax prevents edit", {"src": ["core:signs.rs"], "caps": ["text_input_screen"]}),
    ("vault_key", "use trial key / ominous key on vault", "per-player reward once; repeat use refused", {"src": ["core:vault.rs"], "custody": True}),
    ("ignite", "flint&steel / fire charge on block, TNT, candle, campfire, nether portal frame", "fire placed / TNT primed / candle lit / portal lit; durability or charge consumed", {"src": ["core:fire.rs"]}),
    ("axe_strip_scrape_unwax", "axe on logs, copper (oxidized/waxed)", "stripped log; copper scraped one stage or unwaxed", {"src": ["core:block_tools.rs", "core:copper.rs"]}),
    ("shovel_path", "shovel on grass/dirt; on lit campfire", "dirt path; campfire extinguished", {"src": ["core:block_tools.rs"]}),
    ("hoe_till", "hoe on dirt/grass/coarse dirt/rooted dirt", "farmland / dirt / hanging roots drop per vanilla", {"src": ["core:block_tools.rs", "core:farming.rs"]}),
    ("bone_meal", "bone meal crops, saplings, grass, moss, flowers, mushrooms", "growth stage advances / feature generates; item consumed", {"src": ["core:farming.rs"]}),
    ("beehive_harvest", "shears/bottle on full hive with/without campfire", "honeycomb/honey bottle; bees angry without campfire", {"src": ["core:bees.rs"]}),
    ("cauldron", "fill/empty cauldron with buckets and bottles; dye leather in water", "levels change; items exchanged exactly", {"src": ["core:bottles.rs", "core:fluid_buckets.rs"], "custody": True}),
    ("campfire_cook", "place raw food on campfire (4 slots)", "cooks after 30s; item pops; no duplicate on break", {"src": ["core:campfire.rs"], "custody": True}),
    ("composter", "compost items to level 8; collect bone meal", "level increases by chance; level 8 -> bone meal", {"src": ["core:composter.rs"], "custody": True}),
    ("armor_stand_use", "place armor stand; equip/take armor; name tag", "equipment custody exact; stand drops on two punches", {"src": ["core:armor_stand.rs"], "custody": True}),
    ("item_frame_use", "place item frame; insert item; rotate; punch out", "item displayed; rotation 0-7; drop exactly once; comparator reads rotation", {"src": ["core:item_frame.rs"], "custody": True}),
    ("spawn_egg_on_spawner", "use spawn egg on monster spawner and on ground", "spawner type changes; ground spawns mob", {"src": ["core:spawn_egg.rs"]}),
    ("leash_fence", "lead a mob and click fence", "leash knot created; mob tethered", {"src": ["core:leash.rs"]}),
    ("banner_map_marker", "use banner with filled map", "map marker added", {"src": ["core:maps/banner.rs"]}),
    ("shelf", "swap/store items on shelf block", "shelf slots swap with hotbar; contents conserved", {"src": ["core:shelf.rs"], "custody": True}),
    ("ender_chest_personal", "open ender chest from two players", "per-player inventory; contents follow player", {"src": ["core:ender_chest.rs"], "menu": True, "custody": True}),
    ("cake_candle", "eat cake slices; place candle on cake; light", "food restored per slice; candle cake lit", {"src": ["core:consumables.rs", "core:fire.rs"]}),
    ("copper_wax_oxidize", "wax copper; wait oxidation; lightning on oxidized", "waxed stops aging; random ticks age; lightning scrapes", {"src": ["core:copper.rs"]}),
    ("end_portal_eye", "insert eyes into end portal frames", "eye=true; 12 eyes activate portal", {"src": ["core:end_portals.rs"], "dims": ["overworld"]}),
    ("end_crystal_place", "place end crystal on obsidian/bedrock", "crystal entity spawned; refused elsewhere", {"src": ["core:end_fight.rs"]}),
    ("respawn_anchor", "charge anchor with glowstone; set spawn in nether; use in overworld", "charges 0..4; sets spawn in nether; explodes in overworld/end", {"src": ["core:dimensions_rules.rs"]}),
    ("block_log_inspect_click", "with /co inspect on, click blocks", "history of block shown, block not changed", {"src": ["core:block_log.rs"], "op": 3}),
])
