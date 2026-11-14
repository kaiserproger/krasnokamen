"""Subjects part 3: combat, death, mobs/world systems, redstone, fluids, environment."""
from subjects import domain

domain("combat", ["core:combat.rs", "core:physical_attacks.rs", "core:physical_defense.rs", "core:tick/damage.rs"], ["attack_entity", "read_world_state"], [
    ("melee_cooldown_crit_sweep", "attack at partial/full cooldown, falling crit, sword sweep", "damage scales by cooldown; crit x1.5; sweep hits neighbours", {}),
    ("knockback", "hit while sprinting, knockback enchant, knockback resistance", "knockback vector/strength per vanilla", {}),
    ("pvp", "player hits player with pvp gamerule true/false", "damage only when pvp=true; team friendlyFire respected", {"mp": True}),
    ("armor_mitigation", "take damage in each armor tier + toughness + protection enchants", "reduction per vanilla armor formula; armor durability", {"src": ["core:enchantments.rs"]}),
    ("shield_vs_axe", "attack a blocking player/mob with axe", "shield disabled 100 ticks", {"caps": ["release_use_item"]}),
    ("damage_enchants", "sharpness/smite/bane/impaling/fire aspect vs targets", "bonus damage by target type; fire aspect ignites", {"src": ["core:enchantments.rs"]}),
    ("thorns", "attack entity wearing thorns", "attacker damaged by chance; armor wear", {"src": ["core:enchantment_thorns.rs"]}),
    ("bow", "draw and release bow; power/punch/flame/infinity", "arrow velocity by draw; ammo debited unless infinity; damage", {"src": ["core:projectiles/weapons.rs"], "caps": ["release_use_item"]}),
    ("crossbow", "load/fire crossbow; multishot/piercing/quick charge; fireworks", "charged state; projectiles per enchant", {"src": ["core:projectiles/weapons.rs"], "caps": ["release_use_item"]}),
    ("trident", "throw trident; loyalty return; channeling in thunder; impaling", "trident entity; loyalty returns; lightning on hit in thunder", {"src": ["core:projectiles/weapons.rs"], "caps": ["release_use_item"]}),
    ("mace", "mace smash after fall; density/breach/wind burst", "damage by fall distance; fall reset; area knockback", {"src": ["core:mace.rs"]}),
    ("invulnerability_frames", "rapid repeated hits", "10-tick hurt invulnerability; only higher damage applies", {}),
    ("projectile_pickup", "pick up arrows (survival/creative/infinity)", "pickup rules per arrow pickup status", {"src": ["core:projectiles/embedded_projectiles.rs"]}),
    ("mob_targets_player", "hostile mob detects and attacks player", "target acquisition by follow range/line of sight; hits deal damage", {"src": ["core:hostile.rs"]}),
])

domain("death_respawn", ["core:survival.rs", "core:entity_life.rs"], ["respawn", "read_world_state"], [
    ("death_screen_respawn", "die then click Respawn", "death screen with message; respawn at world spawn with full health", {}),
    ("keep_inventory_false", "die with items and XP (keepInventory=false)", "items drop at death spot; XP orbs 7*level capped 100; inventory empty on respawn", {"custody": True}),
    ("keep_inventory_true", "die with keepInventory=true", "inventory and levels kept; nothing dropped", {"custody": True, "op": 2}),
    ("bed_spawn", "sleep/set spawn at bed, die", "respawn at bed; obstructed/missing bed falls back to world spawn with message", {"src": ["core:sleep.rs"]}),
    ("anchor_spawn", "set spawn at charged respawn anchor in nether, die anywhere", "respawn at anchor in nether; charge consumed", {"src": ["core:dimensions_rules.rs"], "dims": ["the_nether"]}),
    ("immediate_respawn", "doImmediateRespawn=true then die", "no death screen; respawn immediately", {"op": 2}),
    ("void_death", "fall below world bottom", "void damage until death; drops lost in void", {}),
    ("death_other_dimension", "die in nether/end without spawn there", "respawn in overworld; dimension change packet", {"src": ["core:dimensions_travel.rs"], "dims": ["the_nether", "the_end"]}),
    ("death_message", "die by several causes", "vanilla death message keys broadcast (showDeathMessages)", {"caps": ["read_chat"]}),
    ("death_while_menu_open", "die with a container open and cursor stack", "menu closed; cursor stack dropped/handled once", {"menu": True, "custody": True}),
    ("death_while_riding", "die while riding horse/boat", "dismounted; vehicle remains", {}),
    ("ender_pearls_on_death", "die with ender pearls in flight (enderPearlsVanishOnDeath)", "pearls removed when true", {"src": ["core:projectiles/flight.rs"]}),
    ("respawn_invulnerability", "mob hits within 3s after respawn", "spawn protection blocks mob hits briefly", {"src": ["core:tick_tests.rs"]}),
])

domain("spawning", ["core:natural_spawn.rs", "core:natural_spawn_hostile.rs", "core:dimensions_spawning.rs"], ["read_world_state"], [
    ("despawn_rules", "leave mobs > 128 blocks / idle 32..128", "instant despawn beyond 128; random despawn; persistent named/leashed mobs kept", {"inflight": True}),
    ("mob_cap", "fill area to mob cap", "spawning stops at category caps per loaded chunks", {"src": ["core:plugin_mob_cap_tests.rs"]}),
    ("light_rules", "dark vs lit areas at night", "monsters need block light 0; passive on grass light >= 9", {}),
    ("monster_spawner_block", "approach monster spawner", "spawns within 4 blocks when player within 16; delay 200-800", {"src": ["core:monster_spawner.rs"]}),
    ("patrols", "doPatrolSpawning in overworld", "pillager patrols spawn after day 5 per vanilla", {"src": ["core:raids.rs"], "dims": ["overworld"]}),
    ("wandering_trader_spawn", "doTraderSpawning", "trader + 2 llamas near player at intervals", {"src": ["core:mob_lanes_trades.rs"], "dims": ["overworld"]}),
    ("phantoms_insomnia", "stay awake 3+ days (doInsomnia)", "phantoms spawn at night above sky-exposed player", {"src": ["core:special_mobs/mod.rs"], "dims": ["overworld"]}),
    ("slime_chunks_swamp", "slime chunk below y40 / swamp at night", "slimes spawn per rules", {}),
    ("breeding_and_babies", "breed two animals with food", "baby spawned; parents 5-min cooldown; baby grows in 20 min", {"src": ["core:entity_life_breeding.rs"], "caps": ["interact_entity"]}),
    ("peaceful_removal", "switch difficulty to peaceful with hostiles", "hostiles removed; no monster spawns", {"op": 2}),
    ("spawnRadius_world_spawn", "new players with spawnRadius 0/10", "spawn point randomized within radius on safe ground", {"src": ["core:natural_spawn.rs"]}),
])

domain("villagers", ["core:villager/mod.rs", "core:villager/brain.rs", "core:villager/trades.rs"], ["interact_entity", "read_world_state"], [
    ("trade_level_restock", "trade until villager levels; wait for restock at workstation", "XP/level up unlocks tiers; restock twice daily", {"caps": ["select_trade"], "menu": True, "custody": True}),
    ("job_site_claim", "place workstation near unemployed villager", "profession acquired; nitwit refuses", {"src": ["core:villager/building.rs"]}),
    ("schedule_sleep_work", "observe villager day/night", "work, gather at bell, sleep in bed at night; doors used", {"src": ["core:villager/brain.rs", "core:villager/doors.rs"], "dims": ["overworld"]}),
    ("villager_breeding", "beds + food in village", "villager babies when willing", {"src": ["core:villager/mod.rs"]}),
    ("farmer_harvest", "farmer near crops", "harvests and replants, shares food", {"src": ["core:villager/harvest.rs"]}),
    ("zombie_conversion_cure", "zombie kills villager (normal/hard); cure with weakness + golden apple", "zombie villager keeps profession; cure discounts trades", {"src": ["core:villager/conversion.rs"]}),
    ("lightning_to_witch", "lightning strikes villager", "villager becomes witch", {"src": ["core:weather_lightning.rs"]}),
    ("iron_golem_spawn_defend", "villagers panic / gossip; hostile approaches", "iron golem spawns; attacks hostiles and players with bad reputation", {"src": ["core:golems.rs", "core:villager/defense.rs"]}),
    ("snow_golem", "build snow golem; let it fight", "snowballs knock back; snow trail; melts in hot biomes/nether", {"src": ["core:golems.rs"]}),
    ("copper_golem", "build copper golem; observe sorting into copper chests", "golem moves items from copper chest to chests; oxidizes to statue", {"src": ["core:copper_golem.rs"]}),
    ("raid_start", "enter village with Bad Omen / ominous bottle", "raid omen -> raid starts; bossbar", {"src": ["core:raids.rs", "core:omens.rs"]}),
    ("raid_waves_victory", "defeat all waves", "Hero of the Village effect; discounts", {"src": ["core:raids.rs"], "inflight": True}),
    ("raid_loss", "let raiders kill all villagers", "raid lost; bossbar removed", {"src": ["core:raids.rs"], "inflight": True}),
    ("solaris_professions", "observe Solaris-only Miner/Lumberjack/Builder/Soldier villagers", "settlement work loops (mining/lumber/building/soldier) run on native tick", {"src": ["core:villager/mining.rs", "core:villager/lumber.rs", "core:villager/soldier.rs"]}),
])

domain("farming", ["core:farming.rs", "core:plants.rs", "core:plants_growth.rs"], ["use_item_on_block", "read_world_state"], [
    ("wheat_carrot_potato_beetroot", "plant on hydrated farmland and random-tick", "age advances; light >= 9 required; hydration speeds growth", {"inflight": True}),
    ("stem_crops", "melon/pumpkin stems", "fruit spawns on adjacent dirt; stem attaches", {}),
    ("nether_wart", "nether wart on soul sand", "grows to age 3 in any dimension", {}),
    ("sugar_cane_cactus_bamboo", "plant next to water / sand", "grow up to 3/3/16; cactus breaks on adjacent block", {}),
    ("kelp_seagrass", "plant kelp underwater", "grows to max age", {}),
    ("sweet_berries_cocoa_glow_berries", "plant berries, cocoa on jungle log, cave vines", "age; harvest by use", {}),
    ("chorus", "chorus flower on end stone", "grows plant tree", {"src": ["core:chorus.rs"], "dims": ["the_end"]}),
    ("mushrooms_vines_spread", "mushrooms in dark, vines, twisting/weeping vines", "spread limits per rules", {}),
    ("saplings_trees", "plant saplings incl. 2x2", "tree grows with space and light", {}),
    ("grass_mycelium_spread", "dirt next to grass in light", "grass spreads; dies under opaque", {}),
    ("farmland_trample_dry", "jump on farmland; remove water", "trampled to dirt; dries without water", {}),
    ("random_tick_speed", "set randomTickSpeed 0 / 3 / 100", "growth rate scales; 0 stops", {"op": 2}),
    ("shear_sheep_wool_regrow", "shear sheep, let graze", "1-3 wool; regrows after eating grass", {"src": ["core:entity_life.rs"], "caps": ["interact_entity"]}),
    ("milk_cow", "bucket on cow/mooshroom; bowl on mooshroom", "milk bucket / stew", {"caps": ["interact_entity"], "custody": True}),
    ("chicken_eggs", "chickens lay eggs", "egg every 5-10 minutes", {"inflight": True}),
    ("bees_pollinate", "bees with flowers and hive", "pollinate, honey level increases, crops grow", {"src": ["core:bees.rs"]}),
    ("taming", "tame wolf/cat/parrot/horse/llama/fox trust", "tamed owner set; sitting; follows", {"src": ["core:taming.rs"], "caps": ["interact_entity"]}),
    ("golden_dandelion", "use golden dandelion (26.x feature)", "behaviour per golden_dandelion.rs", {"src": ["core:golden_dandelion.rs"]}),
])

domain("redstone", ["core:redstone/mod.rs", "core:redstone/signal.rs"], ["use_item_on_block", "read_world_state"], [
    ("wire_power_levels", "lever -> 15 wires -> lamp", "power decays 1 per wire; lamp lights at >= 1", {}),
    ("torch_inversion_burnout", "torch inverter; rapid clock", "inverts; burns out after 8 toggles in 60 ticks", {}),
    ("repeater_delay_lock", "repeater delays 1-4; side-lock", "delay ticks; locked holds output", {}),
    ("comparator_modes", "comparator read chest/cake/item frame; subtract", "signal by fullness; subtract mode", {"src": ["core:redstone/containers.rs"]}),
    ("pressure_plates_buttons", "wood/stone/weighted plates; buttons; arrows on buttons", "entity-type rules; weighted by count", {}),
    ("observer_pulse", "observer facing changing block", "2-tick pulse on update", {}),
    ("piston_push_pull", "push 12 blocks; 13 refused; immovable (obsidian); sticky pull", "push limit 12; block entities immovable", {"src": ["core:redstone/piston.rs", "core:redstone/piston_motion.rs"]}),
    ("slime_honey_flying_machine", "slime/honey contraption", "sticky groups move; honey/slime don't stick together", {"src": ["core:redstone/piston.rs"]}),
    ("quasi_connectivity", "piston powered diagonally (QC/BUD)", "vanilla QC behaviour (quirk parity)", {"src": ["core:quirk_parity_tests.rs"]}),
    ("dispenser_behaviours", "dispense arrows, fire charge, buckets, bone meal, armor, shulker, TNT, minecart, boat, shears", "per-item dispense behaviour", {"src": ["core:redstone/dispenser_blocks.rs", "core:redstone/dispenser_equipment.rs"], "custody": True}),
    ("hopper_chains", "hopper chain into furnace; locked hopper; pull from above", "8-tick transfer; locked stops; furnace slot rules", {"src": ["core:redstone/containers.rs"], "custody": True}),
    ("rails_powered_detector_activator", "minecart over powered/detector/activator rails", "boost/brake; detector outputs; activator ejects/primes", {"src": ["core:redstone/rails.rs"]}),
    ("daylight_target_bulb_lamp", "daylight detector, copper bulb toggle, redstone lamp", "signal by sky light; bulb toggles per pulse", {}),
    ("sculk_sensor_signal", "vibrations near sculk/calibrated sensor", "signal strength by distance/frequency", {"src": ["core:sculk.rs", "core:vibration.rs"]}),
    ("trapped_chest_note_door_power", "trapped chest, note block, iron door, TNT powered", "each reacts to power", {}),
    ("command_block", "command block impulse/chain/repeat (enableCommandBlocks)", "runs command as block; output per commandBlockOutput", {"src": ["core:command_blocks.rs"], "op": 2}),
    ("update_order_quirks", "vanilla update-order farms (TNT duper, BUD)", "behaviour matches 26.1.2 oracle", {"src": ["core:quirk_parity_tests.rs"]}),
])

domain("fluids", ["core:fluids.rs", "core:fluid_ticks.rs", "core:fluid_buckets.rs"], ["use_item_on_block", "read_world_state"], [
    ("water_flow", "place water source on flat/stepped ground", "spreads 7 blocks, falls, flows toward nearest drop", {"inflight": True}),
    ("lava_flow_by_dimension", "lava in overworld vs nether", "overworld: 3 blocks/30 ticks; nether: 7 blocks/10 ticks", {}),
    ("infinite_water_source", "2x1 source pattern (waterSourceConversion)", "new source forms between two sources", {}),
    ("lava_source_conversion", "lavaSourceConversion gamerule", "lava sources form only when true", {"op": 2}),
    ("water_lava_interaction", "flowing water meets lava source/flow", "obsidian / cobblestone / stone; basalt with soul soil+blue ice", {}),
    ("buckets", "fill/empty water, lava, powder snow buckets", "source removed/placed; bucket exchanged", {"src": ["core:fluid_buckets.rs"], "custody": True}),
    ("water_in_nether", "empty water bucket in nether", "evaporates; bucket emptied", {"dims": ["the_nether"]}),
    ("waterlogging", "waterlog/drain slabs, stairs, fences", "waterlogged property; flows out on break", {}),
    ("entity_push_items", "items and mobs in flowing water", "pushed along flow", {"src": ["core:item_motion.rs", "core:entity_motion.rs"]}),
    ("fire_extinguish", "water flows onto fire/campfire", "fire removed; campfire extinguished", {"src": ["core:fire.rs"]}),
    ("concrete_powder", "concrete powder touches water", "hardens to concrete", {"src": ["core:falling.rs"]}),
    ("sponge", "place sponge in water", "vanilla: absorbs water within 7, becomes wet sponge", {"status": "no_code_hit"}),
])

domain("falling_blocks", ["core:falling.rs"], ["place_block", "read_world_state"], [
    ("sand_gravel", "place sand/gravel/red sand over air", "falls, lands as block; breaks on torch/slab and drops item", {"inflight": True}),
    ("suspicious_blocks", "suspicious sand/gravel unsupported; brush them", "falls/breaks without loot; brushing yields loot", {"src": ["core:plants.rs"]}),
    ("anvil_fall_damage", "drop anvil onto mob/player", "falling_anvil damage by distance; anvil may chip", {}),
    ("dragon_egg", "click dragon egg; break support", "teleports on click; falls", {"dims": ["the_end", "overworld"]}),
    ("pointed_dripstone", "break support of stalactite above player", "falls and deals falling_stalactite damage", {}),
    ("falling_into_void_or_unloaded", "falling block crosses into unloaded chunk/void", "entity removed or persisted per vanilla; no ghost block", {"inflight": True}),
])

domain("explosions", ["core:explosion.rs", "core:tnt.rs"], ["use_item_on_block", "read_world_state"], [
    ("tnt_prime_chain", "prime TNT next to more TNT", "80-tick fuse; chain primes with short random fuse; craters", {"inflight": True}),
    ("creeper", "creeper explosion; charged creeper; mobGriefing false", "damage + terrain per gamerule; charged drops heads", {"src": ["core:hostile.rs"]}),
    ("bed_anchor_wrong_dimension", "use bed in nether/end; anchor in overworld/end", "intentional game design explosion, fire in nether", {"src": ["core:dimensions_rules.rs"]}),
    ("end_crystal_explosion", "shoot end crystal", "power-6 explosion", {"src": ["core:end_fight.rs"]}),
    ("tnt_minecart", "activator rail / fall", "explodes by speed", {"src": ["core:vehicles/mod.rs"]}),
    ("ghast_wither_fireballs", "ghast fireball, wither skull", "explosion power 1 / wither skull damage", {"src": ["core:special_mobs/wither.rs"]}),
    ("blast_resistance", "TNT next to obsidian, water, bedrock", "resistant blocks survive; water cancels terrain damage", {}),
    ("explosion_drop_decay", "blockExplosionDropDecay/mobExplosionDropDecay/tntExplosionDropDecay", "drops 1/power vs all", {"op": 2}),
    ("tnt_explodes_rule", "tntExplodes=false", "TNT does not explode", {"op": 2}),
    ("blast_protection_knockback", "explosion vs player with blast protection", "damage/knockback reduced", {"src": ["core:enchantments.rs"]}),
    ("wind_charge", "throw wind charge / breeze shot", "knockback burst, no block damage; opens doors/activates buttons", {"src": ["core:projectiles/weapons.rs"]}),
])

domain("fire", ["core:fire.rs"], ["use_item_on_block", "read_world_state"], [
    ("fire_spread", "light fire in wooden structure (doFireTick)", "spreads/burns flammables; stops when false", {"inflight": True}),
    ("eternal_fire", "fire on netherrack/soul sand/bedrock in end", "never burns out", {}),
    ("lava_ignites", "lava near flammable blocks", "nearby flammables ignite", {}),
    ("entity_burning", "stand in fire/lava, undead in sunlight", "on_fire ticks; extinguished by water/rain", {"src": ["core:contact_hazards.rs"]}),
    ("candles_campfire_light", "light/extinguish candles and campfires", "lit state; light level", {}),
])

domain("weather_time", ["core:weather.rs", "core:weather_lightning.rs", "core:weather_surface.rs", "core:sleep.rs"], ["command_send", "read_world_state"], [
    ("weather_cycle", "/weather clear|rain|thunder; doWeatherCycle", "weather changes; rain level packets; cycle stops when false", {"op": 2, "dims": ["overworld"]}),
    ("lightning_effects", "lightning on pig, villager, creeper, mooshroom, turtle, lightning rod, trident channeling", "piglin/witch/charged creeper/brown mooshroom; rod powers", {"src": ["core:weather_lightning.rs"], "dims": ["overworld"]}),
    ("snow_ice_accumulation", "snow in cold biomes; water freezes (snowAccumulationHeight)", "snow layers up to limit; ice forms", {"src": ["core:weather_surface.rs"], "dims": ["overworld"]}),
    ("rain_effects", "rain extinguishes fire/entities; cauldron fills", "fire out; cauldron level up", {"dims": ["overworld"]}),
    ("no_weather_nether_end", "/weather rain while in nether/end", "no precipitation in nether/end; overworld state changes", {"dims": ["the_nether", "the_end"], "op": 2}),
    ("day_night_cycle", "doDaylightCycle true/false; /time set", "time advances 1/tick; frozen when false", {"op": 2}),
    ("sleep_skip_night", "players sleep at night (playersSleepingPercentage)", "night skipped when quorum sleeps; weather cleared", {"src": ["core:sleep.rs"], "dims": ["overworld"]}),
    ("sleep_refused", "sleep in day / monsters nearby / bed far", "refusal messages; no skip", {"src": ["core:sleep.rs"], "dims": ["overworld"]}),
])

domain("portals_dimensions", ["core:portals.rs", "core:portals_nether.rs", "core:portals_travel.rs", "core:dimensions_travel.rs"], ["walk_move", "portal_travel", "read_world_state"], [
    ("nether_portal_build_light", "build obsidian frame (min 4x5..23x23), light it", "portal blocks fill frame; invalid frames don't light", {"dims": ["overworld", "the_nether"]}),
    ("nether_portal_travel_link", "stand in portal (4s survival / creative delay)", "travel with 8:1 scaling; existing portal reused or new created", {"dims": ["overworld", "the_nether"], "inflight": True}),
    ("portal_gamerules", "allowEnteringNetherUsingPortals false; playersNetherPortal*Delay", "no travel when false; delays honored", {"dims": ["overworld", "the_nether"], "op": 2}),
    ("entities_through_portals", "push item/mob through nether portal", "entity transferred with cooldown", {"dims": ["overworld", "the_nether"]}),
    ("end_portal_travel", "jump into activated end portal", "arrive on obsidian platform in end", {"src": ["core:end_portals.rs"], "dims": ["overworld"]}),
    ("end_exit_return", "enter exit portal after dragon", "return to overworld spawn/bed", {"src": ["core:end_portals.rs"], "dims": ["the_end"]}),
    ("end_gateway", "throw pearl into gateway / walk in", "teleported to outer islands and back", {"src": ["core:end_portals.rs"], "dims": ["the_end"]}),
    ("dragon_fight", "destroy crystals, fight dragon to death", "perching/strafing phases; death -> XP, exit portal, egg, gateway", {"src": ["core:end_fight.rs", "core:end_fight/flight.rs", "core:end_fight/pillars.rs"], "dims": ["the_end"], "caps": ["attack_entity"], "inflight": True}),
    ("dragon_respawn", "place 4 end crystals on exit portal", "dragon respawns, pillars rebuilt", {"src": ["core:end_fight/persistence.rs"], "dims": ["the_end"]}),
    ("eye_of_ender_throw", "throw eye of ender", "flies toward nearest stronghold; drops or shatters", {"src": ["core:projectiles/eye.rs"], "dims": ["overworld"]}),
    ("command_dimension_travel", "/execute in <dim> run tp / tp to other dimension player", "dimension change, entities/chunks of new world, own chat works", {"src": ["core:dimensions_commands.rs"], "op": 2}),
])
