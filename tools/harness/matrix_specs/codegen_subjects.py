"""Subjects enumerated directly from Krasnokamen source files (read at generation time)."""
import os
import json
import re
from pathlib import Path

from subjects import D, domain

SOL = Path(__file__).resolve().parents[3]
CORE = SOL / "crates/mc-core/src"
EDGE = SOL / "crates/mc-edge/src"
DATA = SOL / "crates/mc-data/src"


def read(p):
    return Path(p).read_text()


def enum_variants(path, name):
    src = read(path)
    m = re.search(r"pub enum " + name + r"\s*\{(.*?)\n\}", src, re.S)
    body = re.sub(r"//[^\n]*", "", m.group(1))
    body = re.sub(r"#\[[^\]]*\]", "", body)
    return re.findall(r"^\s+([A-Z]\w*)\s*(?:[,{(=]|$)", body, re.M)


# ------------------------------------------------------------------ extraction
def command_roots():
    src = read(CORE / "commands.rs")
    m = re.search(r"const ROOTS: &\[\(&str, u8\)\] = &\[(.*?)\n\];", src, re.S)
    return [(n, int(l)) for n, l in re.findall(r'\("([a-z_-]+)", (\d)\)', m.group(1))]


def settlement_help():
    src = read(EDGE / "command_tree.rs")
    m = re.search(r"const SETTLEMENT_HELP: &\[\(&str, &str\)\] = &\[(.*?)\n\];", src, re.S)
    out = re.findall(r'\(\s*"([a-z_ -]+)",\s*"((?:[^"\\]|\\.)*)"', m.group(1), re.S)
    m2 = re.search(r"const SETTLEMENT_ECONOMY: &\[\(&str, &str, &str\)\] = &\[(.*?)\n\];", src, re.S)
    eco = re.findall(r'\(\s*"([a-z_]+)",\s*"([a-z_]+)",\s*"((?:[^"\\]|\\.)*)"', m2.group(1), re.S)
    return out, eco


def gamerules():
    """Canonical 26.1.2 snake_case rules with their legacy camelCase aliases (core:command_rules.rs RULES)."""
    src = read(CORE / "command_rules.rs")
    body = re.search(r"const RULES: &\[Rule\] = &\[(.*?)\n\];", src, re.S).group(1)
    out = []
    for kind, args in re.findall(r"(boolean_rule|integer_rule)!\((.*?)\)", body, re.S):
        strings = re.findall(r'"([A-Za-z_]+)"', args)
        name = strings[0]
        aliases = strings[2:] if kind == "boolean_rule" else strings[1:]
        out.append((name, aliases))
    return out


def entity_contracts():
    ec = read(DATA / "entity_contract_26_1_2.rs")
    rows = re.findall(r'^\s+(\d+), "minecraft:([a-z_]+)", (\w+), (\w+), (\w+), (\d+), (true|false)', ec, re.M)
    cats = dict(re.findall(r'^\s+\d+, "minecraft:([a-z_]+)", [\d.]+, [\d.]+, [\w.]+, [\d.]+, \d+, \d+, (\w+),', ec, re.M))
    living = [(r[1], cats[r[1]], r[4]) for r in rows if r[2] == "Living" and r[1] != "player"]
    nonliving = [r[1] for r in rows if r[2] != "Living"]
    return living, nonliving


def villager_professions():
    return enum_variants(CORE / "villager/mod.rs", "VillagerProfession")


def plugin_manifests():
    out = {}
    work = Path(os.environ.get("KRASNOKAMEN_WORK_ROOT", "/run/media/kaiserroman/games/solaris-work"))
    roots = [work / "solaris-default-plugins",
             Path(os.environ.get("KRASNOKAMEN_MATRIX_DATA", work / "scenarios")) / "live-plugins"]
    for p in sorted(q for root in roots for q in root.glob("*/plugin.toml")):
        if p.parent.name in out:
            continue
        txt = p.read_text()
        api = re.search(r'^api\s*=\s*"([^"]+)"', txt, re.M)
        cmds = re.search(r"^player_commands\s*=\s*\[([^\]]*)\]", txt, re.M)
        out[p.parent.name] = {"api": api.group(1) if api else None,
                              "commands": re.findall(r'"([a-z]+)"', cmds.group(1)) if cmds else [],
                              "wasm": (p.parent / "plugin.wasm").exists()}
    return out


# ------------------------------------------------------------------ commands
CMD_EX = {
    "help": ("/help [command]", "usage lines filtered by permission"),
    "list": ("/list", "online player names/count"),
    "me": ("/me waves", "'* name waves' broadcast"),
    "msg": ("/msg <player> hi", "private message to target only"),
    "tell": ("/tell <player> hi", "alias of msg"),
    "w": ("/w <player> hi", "alias of msg"),
    "teammsg": ("/teammsg hi", "message to own team only; error if no team"),
    "tm": ("/tm hi", "alias of teammsg"),
    "settlement": ("/settlement guide", "Krasnokamen settlement root (see settlements domain)"),
    "trigger": ("/trigger <objective> [add|set n]", "only enabled trigger objectives; once"),
    "gamemode": ("/gamemode creative|survival|adventure|spectator [target]", "game mode changes; abilities packet; tab list"),
    "teleport": ("/teleport <targets> <x y z | entity> [rotation]", "targets moved; cross-dimension when target elsewhere"),
    "tp": ("/tp <...>", "alias of teleport"),
    "execute": ("/execute as|at|in|positioned|rotated|facing|anchored|align|if|unless ... run <cmd>; /execute store ...",
                "context modifiers applied; conditions gate run; 'execute store' currently answers 'execute store is not implemented' (core:command_context.rs) — server gap until DataCommands lands"),
    "give": ("/give <targets> <item>[components] [count]", "items inserted; overflow dropped; components kept"),
    "clear": ("/clear <targets> [item] [max]", "matching items removed; count reported"),
    "kill": ("/kill [targets]", "targets die from minecraft:generic_kill (vanilla 26.1.2 DamageSources.genericKill; death message death.attack.genericKill; bypasses armor/creative). "
                                "Known Krasnokamen gap: survival.rs maps /kill to Void -> minecraft:out_of_world, so the damage type/death message differ until fixed"),
    "time": ("/time set day|noon|night|midnight|<n>[d|s|t]; add; query", "time updated and broadcast"),
    "weather": ("/weather clear|rain|thunder [duration]", "weather changes (overworld)"),
    "difficulty": ("/difficulty peaceful|easy|normal|hard", "difficulty set; peaceful removes hostiles"),
    "gamerule": ("/gamerule <rule> [value]", "query/set per gamerules domain"),
    "summon": ("/summon <entity> [pos] [nbt]", "entity spawned with NBT"),
    "worldborder": ("/worldborder set|add|center|damage|warning|get", "border updated (see world_border)"),
    "setblock": ("/setblock <pos> <state> [destroy|keep|replace]", "block placed with modes"),
    "fill": ("/fill <from> <to> <state> [hollow|outline|keep|replace|destroy]", "region filled; volume limit (commandModificationBlockLimit)"),
    "experience": ("/experience add|set|query <targets> <n> points|levels", "XP updated"),
    "xp": ("/xp ...", "alias of experience"),
    "spawnpoint": ("/spawnpoint [targets] [pos] [angle]", "personal respawn point set"),
    "setworldspawn": ("/setworldspawn [pos] [angle]", "world spawn moved"),
    "forceload": ("/forceload add|remove|query", "chunks force-loaded/ticked"),
    "say": ("/say text", "[name] text broadcast"),
    "seed": ("/seed", "world seed shown (clickable copy)"),
    "enchant": ("/enchant <targets> <enchantment> [level]", "applied when compatible; error otherwise"),
    "effect": ("/effect give|clear <targets> <effect> [seconds|infinite] [amp] [hideParticles]", "effect applied/removed"),
    "advancement": ("/advancement grant|revoke <targets> everything|only|from|through|until", "advancements changed with criteria"),
    "debug": ("/debug (Krasnokamen fixture: hotbar give)", "DIVERGENCE: Krasnokamen test fixture (hotbar give) occupies the root; vanilla 26.1.2 /debug is the start|stop|function profiler; op only"),
    "team": ("/team add|remove|join|leave|modify|list|empty", "team state updated (see scoreboard_teams)"),
    "tps": ("/tps", "Krasnokamen: ticks-per-second report"),
    "mspt": ("/mspt", "Krasnokamen: milliseconds-per-tick report"),
    "scoreboard": ("/scoreboard objectives|players ...", "scoreboard updated (see scoreboard_teams)"),
    "locate": ("/locate structure|biome|poi <id>", "nearest location reported or not-found"),
    "tellraw": ("/tellraw <targets> <component>", "component delivered"),
    "title": ("/title <targets> title|subtitle|actionbar|times|clear|reset", "title shown"),
    "bossbar": ("/bossbar add|set|remove|list|get", "bossbar managed"),
    "function": ("/function <id>", "datapack function runs (or unsupported error)"),
    "schedule": ("/schedule function <id> <time>", "function scheduled"),
    "datapack": ("/datapack list|enable|disable", "datapack state reported"),
    "reload": ("/reload", "datapacks reloaded without disconnects"),
    "data": ("/data get entity <single> [path] [scale]; /data get block x y z [path] [scale]",
             "vanilla 26.1.2 (pinned by DataCommands from the jar): root/unscaled -> '<name> has the following entity data: <SNBT>' / 'x, y, z has the following block data: <SNBT>'; "
             "scaled numeric -> '<path> on <name> after scale factor of <%.2f> is <floor(v*scale)>' / '<path> on block x, y, z after scale factor of <%.2f> is <int>'; "
             "result numeric=floor, list/compound=size, string=UTF-16 length, root=1; zero or multiple path matches fail; 26.1 paths (active_effects, equipment, respawn). "
             "Krasnokamen advertises the root but has no handler yet (server gap; DataCommands implementing)"),
    "tick": ("/tick query|rate|freeze|unfreeze|step|sprint", "tick rate control; frozen world steps"),
    "kick": ("/kick <player> [reason]", "player disconnected"),
    "mute": ("/mute <player> (Krasnokamen)", "player chat blocked"),
    "unmute": ("/unmute <player> (Krasnokamen)", "chat restored"),
    "warn": ("/warn <player> <reason> (Krasnokamen)", "warning delivered/recorded"),
    "transfer": ("/transfer <host> [port] [players]", "players transferred"),
    "ban": ("/ban <player> [reason]", "banned + kicked"),
    "ban-ip": ("/ban-ip <ip|player> [reason]", "IP banned"),
    "pardon": ("/pardon <player>", "ban lifted"),
    "pardon-ip": ("/pardon-ip <ip>", "IP ban lifted"),
    "op": ("/op <player>", "player gets configured op level; command tree resent"),
    "deop": ("/deop <player>", "op removed; command tree shrinks immediately"),
    "whitelist": ("/whitelist on|off|add|remove|list|reload", "whitelist managed"),
    "banlist": ("/banlist [ips|players]", "ban entries listed"),
    "co": ("/co inspect|lookup|rollback|restore", "Krasnokamen block log (see block_log)"),
    "stop": ("/stop", "players saved and disconnected; clean shutdown; checkpoint written"),
    "save-all": ("/save-all [flush]", "checkpoint written; 'Saved the game'"),
}
NOT_CONSOLE = {"trigger", "settlement", "teammsg", "tm"}

# ------------------------------------------------------------------ damage
DMG = {
    "Generic": ("/damage-like generic source via plugin/command fixture", "generic damage bypasses armor (vanilla; Krasnokamen survival.rs agrees)"),
    "Melee": ("mob/player melee hit", "armor + protection reduce; mob_attack/player_attack type"),
    "Thorns": ("attack a thorns-armored target", "thorns damage to attacker"),
    "Projectile": ("arrow/trident/thrown hit", "projectile protection applies; arrow/trident/thrown types"),
    "Fireball": ("ghast/blaze fireball hit", "fireball type; fire + projectile tags"),
    "Lightning": ("lightning strike (trident channeling or /summon lightning_bolt)", "5 damage + fire; passes armor per tag"),
    "Explosion": ("TNT/creeper explosion", "distance/exposure-based damage; blast protection"),
    "InFire": ("stand in fire block", "in_fire damage ticks; fire protection"),
    "OnFire": ("burning after leaving fire", "on_fire 1/s until extinguished"),
    "Lava": ("stand in lava", "4 damage/0.5s; fire res immune"),
    "Suffocation": ("block placed into head (sand fall / setblock)", "in_wall 1/0.5s"),
    "OutsideBorder": ("move outside the world border", "minecraft:outside_border damage"),
    "GenericKill": ("/kill @s", "minecraft:generic_kill bypasses invulnerability"),
    "Freeze": ("stand in powder snow without leather", "minecraft:freeze damage"),
    "FlyIntoWall": ("elytra flight into a wall", "minecraft:fly_into_wall damage"),
    "Cramming": ("exceed max_entity_cramming", "minecraft:cramming damage"),
    "SonicBoom": ("warden sonic boom", "minecraft:sonic_boom bypasses armor and shield"),
    "IndirectMagic": ("indirect damaging magic", "minecraft:indirect_magic damage"),
    "Stalagmite": ("fall onto pointed dripstone", "minecraft:stalagmite damage"),
    "DragonBreath": ("stand in dragon breath", "minecraft:dragon_breath damage"),
    "Cactus": ("touch cactus", "cactus damage 1"),
    "SweetBerryBush": ("move through berry bush", "damage only while moving"),
    "HotFloor": ("stand on magma block not sneaking", "hot_floor damage; sneaking/frost walker immune"),
    "Campfire": ("stand on lit campfire", "campfire damage"),
    "Fall": ("fall > 3 blocks; land in water/hay/slime", "damage = distance-3 (feather falling); water/slime negate, hay 80%"),
    "FallingAnvil": ("anvil falls on player", "falling_anvil damage by distance, helmet reduces"),
    "FallingStalactite": ("pointed dripstone falls on player", "falling_stalactite damage"),
    "Drowning": ("stay underwater past air", "2 damage/s after air 0 (respiration/water breathing)"),
    "Starvation": ("food 0 at each difficulty", "easy stops at 10hp, normal 1hp, hard can kill"),
    "Poison": ("poison effect", "damage down to 1 hp, never kills"),
    "Wither": ("wither effect", "damage can kill"),
    "Magic": ("instant damage potion / evoker fangs / guardian beam", "magic bypasses armor"),
    "Void": ("fall below the world (not /kill: that is generic_kill, see commands.kill)", "minecraft:out_of_world; unreduced; kills creative"),
}
VANILLA_DMG_GAPS = {
    "freeze": ("stand in powder snow 7s without leather", "freeze damage (freezeDamage gamerule)"),
    "fly_into_wall": ("elytra into wall at speed", "fly_into_wall kinetic damage"),
    "cramming": ("> maxEntityCramming entities in one block", "cramming damage"),
    "sonic_boom": ("warden sonic boom", "sonic_boom ignores armor/shield"),
    "dragon_breath": ("stand in dragon breath cloud", "dragon_breath magic damage"),
    "stalagmite": ("fall onto pointed dripstone", "stalagmite damage (fall x2)"),
}

# ------------------------------------------------------------------ effects
EFF = {
    "Speed": "movement speed +20%/level", "Slowness": "speed -15%/level", "Haste": "mining speed +20%/level",
    "MiningFatigue": "mining speed drastically reduced", "Strength": "+3 melee/level", "InstantHealth": "heal 4<<amp (damages undead)",
    "InstantDamage": "damage 6<<amp (heals undead)", "JumpBoost": "jump height up; fall damage reduced", "Nausea": "screen wobble (client)",
    "Regeneration": "heal 1 per 50>>amp ticks", "Resistance": "-20% damage/level", "FireResistance": "immune to fire/lava",
    "WaterBreathing": "no air loss", "Invisibility": "invisible to others; mobs detect less", "Blindness": "fog; no sprint/crit",
    "NightVision": "full brightness (client)", "Hunger": "food exhaustion +0.005/tick/level", "Weakness": "-4 melee/level",
    "Poison": "damage to 1 hp", "Wither": "damage, can kill", "HealthBoost": "+4 max health/level", "Absorption": "absorption hearts",
    "Saturation": "food+saturation per tick", "Glowing": "outline visible to all", "Levitation": "float up 0.9*(amp+1) b/s",
    "Luck": "loot luck", "Unluck": "bad luck", "SlowFalling": "slow descent, no fall damage", "ConduitPower": "water breathing + mining + vision underwater",
    "DolphinsGrace": "faster swimming", "BadOmen": "converts to raid omen in village", "HeroOfTheVillage": "trade discounts, gifts",
    "Darkness": "pulsing darkness (warden)", "TrialOmen": "ominous trial spawners", "RaidOmen": "raid starts after 30s",
    "WindCharged": "wind burst on death", "Weaving": "cobweb spawns on death, faster in webs", "Oozing": "slimes on death",
    "Infested": "silverfish on hurt", "BreathOfTheNautilus": "nautilus air effect",
}

# ------------------------------------------------------------------ mobs
SPECIAL_SRC = {
    "allay": ["core:allay.rs"], "bee": ["core:bees.rs"], "creaking": ["core:creaking.rs"], "dolphin": ["core:dolphin.rs"],
    "enderman": ["core:enderman.rs"], "piglin": ["core:piglin.rs"], "piglin_brute": ["core:piglin.rs"], "warden": ["core:warden.rs"],
    "shulker": ["core:shulker.rs"], "copper_golem": ["core:copper_golem.rs"], "iron_golem": ["core:golems.rs"], "snow_golem": ["core:golems.rs"],
    "villager": ["core:villager/mod.rs"], "wither": ["core:special_mobs/wither.rs"], "ender_dragon": ["core:end_fight.rs"],
    "pillager": ["core:special_mobs/raiders.rs", "core:raids.rs"], "vindicator": ["core:special_mobs/raiders.rs", "core:raids.rs"],
    "evoker": ["core:special_mobs/summons.rs", "core:raids.rs"], "ravager": ["core:special_mobs/raiders.rs", "core:raids.rs"],
    "witch": ["core:special_mobs/shooters.rs", "core:raids.rs"], "illusioner": ["core:special_mobs/raiders.rs"], "vex": ["core:special_mobs/summons.rs"],
    "blaze": ["core:special_mobs/shooters.rs"], "ghast": ["core:special_mobs/shooters.rs"], "breeze": ["core:special_mobs/shooters.rs"],
    "guardian": ["core:special_mobs/shooters.rs"], "elder_guardian": ["core:special_mobs/shooters.rs"],
    "horse": ["core:mounts.rs", "core:taming.rs"], "donkey": ["core:mounts.rs", "core:taming.rs"], "mule": ["core:mounts.rs", "core:taming.rs"],
    "llama": ["core:mounts.rs", "core:taming.rs"], "trader_llama": ["core:mounts.rs"], "camel": ["core:mounts.rs", "core:taming.rs"],
    "strider": ["core:mounts.rs"], "pig": ["core:mounts.rs"], "wolf": ["core:taming.rs"], "cat": ["core:taming.rs"], "parrot": ["core:taming.rs"],
    "fox": ["core:taming.rs"], "creeper": ["core:hostile.rs"], "skeleton": ["core:hostile.rs"], "zombie": ["core:hostile.rs"],
    "slime": ["core:hostile.rs"], "magma_cube": ["core:hostile.rs"], "wandering_trader": ["core:mob_lanes_trades.rs"],
    "zombie_villager": ["core:villager/conversion.rs"], "armor_stand": ["core:armor_stand.rs"],
}
SPECIAL_COMBAT = {"creeper": "CreeperFuse", "skeleton": "Arrow", "stray": "Arrow", "bogged": "Arrow", "blaze": "SmallFireball", "breeze": "WindCharge",
                  "elder_guardian": "GuardianBeam", "guardian": "GuardianBeam", "ender_dragon": "DragonBoss", "evoker": "EvokerFangs", "ghast": "LargeFireball",
                  "pillager": "Crossbow", "shulker": "ShulkerBullet", "warden": "SonicBoom", "witch": "ThrownPotion", "wither": "WitherSkull"}
SPECIAL_GOAL = {"blaze": "BlazeBurst", "ghast": "SiegeShot", "pillager": "Crossbow", "witch": "Alchemist", "breeze": "WindCharge", "guardian": "Beam",
                "elder_guardian": "ElderBeam", "warden": "SonicBoom", "evoker": "Fangs", "shulker": "HomingShot", "wither": "WitherBarrage",
                "vex": "VexLife", "creeper": "Swell", "snow_golem": "Projectile"}
COMBAT_ORACLE = {
    "None": "no attack; panics/flees when hurt where vanilla does",
    "Melee": "acquires target in follow range with line of sight, chases, melee damage per attribute",
    "Arrow": "keeps range, draws and shoots arrows (~1-3 s cadence); strafes",
    "Crossbow": "charges crossbow and fires; joins raids/patrols",
    "GuardianBeam": "locks beam for 2 s then magic damage; thorns-like spikes",
    "CreeperFuse": "swells within 3 blocks, explodes after 1.5 s unless player leaves",
    "SmallFireball": "3-fireball bursts igniting targets",
    "SonicBoom": "sonic boom through blocks when it can't reach; anger by vibration",
    "ShulkerBullet": "homing bullets causing levitation",
    "Snowball": "throws snowballs (knockback, damages blazes)",
    "EvokerFangs": "summons fang lines/circles and vexes",
    "LargeFireball": "explosive fireball (deflectable by hitting)",
    "WindCharge": "wind charge bursts with knockback",
    "ThrownPotion": "throws harming/poison/slowness/weakness; drinks healing",
    "WitherSkull": "wither skull barrage; blue skulls break blocks",
    "DragonBoss": "boss phases (see portals_dimensions.dragon_fight)",
}
MOVE_ORACLE = {
    "Immobile": "does not move on its own", "GroundWander": "wanders on ground, avoids hazards, pathfinds around obstacles",
    "FlyingWander": "flies/hover-wanders", "AquaticWander": "swims in water; suffocates/flops on land",
    "AmphibiousWander": "walks on land and swims", "HostilePursuit": "pursues targets via pathfinding at attribute speed",
    "VillagerSchedule": "follows villager schedule (work/meet/sleep)",
}


def mob_policies(name, sim, cat):
    hostile = cat == "Monster"
    if name in ("armor_stand", "mannequin", "giant"):  # giant: 26.1.2 Giant has no registerGoals override, so no AI goals
        move = "Immobile"
    elif name == "villager":
        move = "VillagerSchedule"
    else:
        move = {"Immobile": "Immobile", "LivingAttached": "Immobile", "LivingAquatic": "AquaticWander",
                "LivingAmphibious": "AmphibiousWander", "LivingFlying": "FlyingWander"}.get(sim)
        if move is None:
            move = ("HostilePursuit" if hostile else "GroundWander") if sim == "LivingGround" else "Immobile"
    if name == "giant":
        combat = "None"
    elif name == "snow_golem":
        combat = "Snowball"
    elif name == "iron_golem":
        combat = "Melee"
    elif not hostile:
        combat = "None"
    else:
        combat = SPECIAL_COMBAT.get(name, "Melee")
    return move, combat


def mob_goals(name, move, combat, breeds):
    if name in SPECIAL_GOAL:
        return [SPECIAL_GOAL[name]]
    if name == "giant":
        return []
    if move == "Immobile":
        return ["Idle"]
    if combat == "Arrow":
        return ["FleeFire", "RangedAttack", "Wander"]
    if combat == "Melee" and breeds:
        return ["Breed", "MeleeAttack", "MeleeChase", "Wander"]
    if combat == "Melee":
        return ["FleeFire", "MeleeAttack", "MeleeChase", "Wander"]
    if breeds:
        return ["FleeFire", "Breed", "Wander", "Swim"]
    return ["Wander"]


NATURAL_DIMS = {"blaze": ["the_nether"], "ghast": ["the_nether"], "magma_cube": ["the_nether"], "hoglin": ["the_nether"], "piglin": ["the_nether"],
                "piglin_brute": ["the_nether"], "strider": ["the_nether"], "zoglin": ["the_nether"], "wither_skeleton": ["the_nether"],
                "zombified_piglin": ["the_nether"], "shulker": ["the_end"], "endermite": ["the_end"], "ender_dragon": ["the_end"]}

NONLIVING_MAP = {
    "boat": "entities_misc.boats", "raft": "entities_misc.boats", "chest_minecart": "containers.chest_minecart", "hopper_minecart": "containers.hopper_minecart",
    "tnt_minecart": "explosions.tnt_minecart", "minecart": "entities_misc.minecarts", "furnace_minecart": "entities_misc.minecarts",
    "command_block_minecart": "entities_misc.minecarts", "spawner_minecart": "entities_misc.minecarts", "arrow": "combat.bow", "spectral_arrow": "combat.bow",
    "trident": "combat.trident", "snowball": "items.throwables", "egg": "items.throwables", "ender_pearl": "items.throwables",
    "experience_bottle": "items.throwables", "splash_potion": "items.throwables", "lingering_potion": "items.throwables",
    "fireball": "explosions.ghast_wither_fireballs", "small_fireball": "mobs.blaze", "wither_skull": "explosions.ghast_wither_fireballs",
    "dragon_fireball": "portals_dimensions.dragon_fight", "wind_charge": "explosions.wind_charge", "breeze_wind_charge": "mobs.breeze",
    "firework_rocket": "items.firework_rocket", "fishing_bobber": "items.fishing", "item": "inventory.pickup_item_entity",
    "experience_orb": "entities_misc.experience_orbs", "falling_block": "falling_blocks.sand_gravel", "tnt": "explosions.tnt_prime_chain",
    "end_crystal": "block_interact.end_crystal_place", "eye_of_ender": "portals_dimensions.eye_of_ender_throw", "item_frame": "entities_misc.item_frame_entity",
    "glow_item_frame": "entities_misc.item_frame_entity", "leash_knot": "entities_misc.leash", "lightning_bolt": "weather_time.lightning_effects",
    "area_effect_cloud": "entities_misc.area_effect_cloud", "evoker_fangs": "mobs.evoker", "llama_spit": "mobs.llama", "shulker_bullet": "mobs.shulker",
    "block_display": "entities_misc.display_interaction_markers", "item_display": "entities_misc.display_interaction_markers",
    "text_display": "entities_misc.display_interaction_markers", "interaction": "entities_misc.display_interaction_markers",
    "marker": "entities_misc.display_interaction_markers", "painting": "entities_misc.paintings", "ominous_item_spawner": "sculk_trials.ominous_trials",
    "cushion": "entities_misc.cushion",
}


def map_nonliving(name):
    if name in NONLIVING_MAP:
        return NONLIVING_MAP[name]
    if name.endswith("_chest_boat") or name.endswith("_chest_raft"):
        return "containers.chest_boat"
    if name.endswith("_boat") or name.endswith("_raft"):
        return "entities_misc.boats"
    raise KeyError(name)


GAMERULE_EFFECT = {
    "keepInventory": "death keeps inventory/XP", "doDaylightCycle": "time advances", "doWeatherCycle": "weather changes",
    "doMobSpawning": "natural spawning", "doFireTick": "fire spreads/burns out", "mobGriefing": "creepers/endermen/ghasts alter terrain",
    "randomTickSpeed": "crop/leaf/copper random ticks", "pvp": "player-vs-player damage", "doImmediateRespawn": "skip death screen",
    "naturalRegeneration": "health regen from food", "fallDamage": "fall damage", "fireDamage": "fire damage", "drowningDamage": "drowning damage",
    "freezeDamage": "freeze damage", "showDeathMessages": "death messages", "announceAdvancements": "advancement chat",
    "sendCommandFeedback": "command feedback to the actor, and the grey italic chat.type.admin '[<actor>: <result>]' broadcast to other online ops (vanilla CommandSourceStack.broadcastToAdmins gates that broadcast on this rule)",
    "playersSleepingPercentage": "sleep quorum", "doTileDrops": "block drops",
    "doEntityDrops": "non-mob entity drops", "doMobLoot": "mob loot/XP", "tntExplodes": "TNT explosions", "spawnRadius": "spawn spread",
    "maxEntityCramming": "cramming threshold", "doInsomnia": "phantoms", "doPatrolSpawning": "patrols", "doTraderSpawning": "wandering traders",
    "doWardenSpawning": "warden emergence", "disableRaids": "raids", "raids": "raids", "spawnMonsters": "monster spawning",
    "lavaSourceConversion": "lava sources", "waterSourceConversion": "water sources", "snowAccumulationHeight": "snow layers",
    "spawnChunkRadius": "spawn chunks", "doLimitedCrafting": "recipe locking", "enderPearlsVanishOnDeath": "pearls on death",
    "universalAnger": "neutral mob anger", "forgiveDeadPlayers": "anger reset", "commandModificationBlockLimit": "/fill volume",
    "minecartMaxSpeed": "minecart speed", "projectilesCanBreakBlocks": "projectile block breaking", "locatorBar": "locator bar",
    "reducedDebugInfo": "F3 coordinates hidden", "spectatorsGenerateChunks": "spectator chunk generation",
    "disablePlayerMovementCheck": "move correction", "disableElytraMovementCheck": "elytra move correction",
    "playersNetherPortalDefaultDelay": "portal delay", "playersNetherPortalCreativeDelay": "creative portal delay",
    "allowEnteringNetherUsingPortals": "nether portals", "commandBlocksEnabled": "command blocks", "enableCommandBlocks": "command blocks",
    "commandBlockOutput": "command block chat", "spawnerBlocksEnabled": "spawner blocks", "blockExplosionDropDecay": "explosion drops",
    "mobExplosionDropDecay": "explosion drops", "tntExplosionDropDecay": "TNT drops", "doVinesSpread": "vine spread",
    "globalSoundEvents": "global sounds",
    "logAdminCommands": "a player's command result is written to the server log/console as '[<actor>: <result>]' when true and not when false (vanilla broadcastToAdmins -> MinecraftServer.sendSystemMessage); "
                        "it does not control chat to other ops — that is send_command_feedback; observe via the server log",
    "maxCommandChainLength": "command chain limit",
    "maxCommandForkCount": "execute fork limit",
}
# Rules whose behaviour switch can only be observed with a second player (role in the check).
GAMERULE_SECOND_PLAYER = {
    "pvp": "a real second player is the victim: player-vs-player melee/projectile damage lands when true and is refused when false",
    "players_sleeping_percentage": "two players, one sleeping: at 50 the night is skipped, at 100 it is not (quorum needs a second player)",
    "universal_anger": "a neutral mob hurt by player A also targets nearby player B when true, only A when false",
    "send_command_feedback": "a second online op receives the grey '[<actor>: <result>]' admin broadcast when true and nothing when false (log_admin_commands does not affect it)",
}


def build():
    roots = command_roots()
    domain("commands", ["core:commands.rs", "edge:command_tree.rs", "edge:admin.rs"], ["command_send", "read_chat"], [
        (r, f"{CMD_EX.get(r, (f'/{r}',))[0]} (requires op {lvl}; probe also at op {max(lvl - 1, 0)})",
         f"{CMD_EX.get(r, ('', 'per vanilla'))[1]}; below op {lvl}: root not advertised and 'Unknown or incomplete command'",
         {"op": lvl, "console": r not in NOT_CONSOLE, **({"status": "server_missing"} if r == "data" else {}),
          **({"caps": ["server_clean_restart"]} if r == "stop" else {}),
          **({"mp": True} if r in ("msg", "tell", "w", "teammsg", "tm", "kick", "ban", "pardon", "op", "deop", "transfer") else {})})
        for r, lvl in roots])
    sel = [("at_s", "@s", "executor"), ("at_p", "@p", "nearest player"), ("at_a", "@a", "all players"), ("at_r", "@r", "random player"),
           ("at_e", "@e", "all entities"), ("player_name", "<name>", "exact player"), ("opt_xyz", "@e[x=,y=,z=]", "origin"),
           ("opt_dxdydz", "@e[dx=,dy=,dz=]", "volume"), ("opt_type", "@e[type=cow] / [type=!cow]", "type filter incl. negation"),
           ("opt_name", "@e[name=\"x\"] / [name=!x]", "name filter"), ("opt_distance", "@e[distance=..5] / 2..5", "distance range; negative rejected"),
           ("opt_limit", "@e[limit=2]", "limit >= 1"), ("sort_nearest", "@e[sort=nearest]", "ordering"), ("sort_furthest", "@e[sort=furthest]", "ordering"),
           ("sort_random", "@e[sort=random]", "ordering"), ("sort_arbitrary", "@e[sort=arbitrary]", "ordering"),
           ("invalid_selector", "@x / @e[foo=1] / duplicate option", "'Unknown selector type' / 'Option isn't applicable' errors"),
           ("nonop_selector_msg", "non-op /msg @a hi", "'Selector not allowed' for non-ops")]
    domain("selectors", ["core:commands.rs"], ["command_send", "read_chat"], [
        (k, f"/tp or /effect with {s} against several players/entities", f"targets = {o} per vanilla EntitySelector",
         {"op": 0 if k == "nonop_selector_msg" else 2, "console": k != "nonop_selector_msg", **({"mp": True} if k in ("at_a", "at_r", "at_p", "sort_random") else {})})
        for k, s, o in sel])
    kinds = enum_variants(CORE / "survival.rs", "DamageKind")
    domain("damage", ["core:survival.rs", "core:tick/damage.rs"], ["read_world_state"],
           [(k, DMG[k][0], f"{DMG[k][1]}; death message + damage type id on wire", {}) for k in kinds] +
           [(f"vanilla_{k}", v[0], f"vanilla: {v[1]}", {"status": "no_code_hit"}) for k, v in VANILLA_DMG_GAPS.items()])
    effects = enum_variants(DATA / "mob_effects_26_1_2.rs", "MobEffect")
    domain("effects", ["core:effects.rs", "core:effects_trigger.rs", "crates/mc-data/src/mob_effects_26_1_2.rs"], ["command_send", "read_world_state"],
           [(e, f"/effect give @s {e} 30 1; observe; /effect clear; also via potion/source where vanilla has one",
             f"{EFF.get(e, 'per vanilla')}; icon/particles to observers; expires on time; cleared by milk", {"op": 2}) for e in effects])
    rules = gamerules()
    domain("gamerules", ["core:command_rules.rs", "core:dimensions_rules.rs"], ["command_send"],
           [(r, f"/gamerule minecraft:{r}" + (f" (legacy alias {', '.join(al)})" if al else "") + " query, then set non-default and exercise",
             f"canonical name queried/set and persisted (vanilla command tree uses canonical names only); legacy alias acceptance is a Krasnokamen compatibility extension, not a vanilla requirement (vanilla GameRuleRegistryFix only upgrades saved data) — record it, don't fail on it; behaviour '{next((GAMERULE_EFFECT[a] for a in al if a in GAMERULE_EFFECT), 'per vanilla GameRules description')}' switches"
             + (f"; second player: {GAMERULE_SECOND_PLAYER[r]}" if r in GAMERULE_SECOND_PLAYER else ""),
             {"op": 2, "console": True, "aliases": al, **({"second_player": True} if r in GAMERULE_SECOND_PLAYER else {})})
            for r, al in rules])
    assert set(GAMERULE_SECOND_PLAYER) <= {r for r, _ in rules}, set(GAMERULE_SECOND_PLAYER) - {r for r, _ in rules}
    living, nonliving = entity_contracts()
    data_breeds = read(CORE / "species.rs")
    mobs = []
    for name, cat, sim in living:
        move, combat = mob_policies(name, sim, cat)
        breeds = f'"minecraft:{name}"' in data_breeds
        goals = mob_goals(name, move, combat, breeds)
        dims = NATURAL_DIMS.get(name)
        mobs.append((name, f"/summon minecraft:{name} near actor (natural spawn where possible); observe 600 ticks; interact/attack; kill and collect loot",
                     f"category {cat}; movement {move}: {MOVE_ORACLE[move]}; combat {combat}: {COMBAT_ORACLE[combat]}; loot table entities/{name}"
                     + ("; vanilla 26.1.2 Giant overrides no registerGoals: no AI goals at all (no wander, look, panic or attack); "
                        "only physics (gravity, knockback, fluid push) moves it" if name == "giant" else ""),
                     {"src": SPECIAL_SRC.get(name, []), "ai_goals": goals, "category": cat, "move_policy": move, "combat_policy": combat,
                      **({"natural_dims": dims} if dims else {}), "op": 2, "caps": ["attack_entity"]}))
    domain("mobs", ["core:species.rs", "core:entities.rs", "core:hostile.rs", "crates/mc-data/src/mob_behavior_26_1_2.rs"], ["command_send", "read_world_state"], mobs)
    D["entities_misc"]["subjects"].append((
        "cushion", "/summon minecraft:cushion; observe interaction, collision and restart",
        "26.3 cushion identity and collision persist across restart",
        {"caps": ["read_world_state", "server_clean_restart"]}))
    for cat in sorted({c for _, c, _ in living}):
        D["spawning"]["subjects"].append((f"natural_{cat}", f"observe natural {cat} spawning in a suitable biome/dimension",
                                          f"{cat} mobs spawn per biome spawn table, light and cap rules", {"src": ["core:natural_spawn.rs"]}))
    profs = villager_professions()
    for p in profs:
        D["villagers"]["subjects"].append((f"profession_{p}", f"villager with profession {p}: open trades / observe work",
                                           f"{p} trade table/work loop per vanilla (Krasnokamen-only: Miner/Lumberjack/Builder/Soldier)",
                                           {"caps": ["select_trade"], "menu": p not in ("None", "Nitwit"), "custody": True}))
    helps, eco = settlement_help()
    subs = [(h.replace(" ", "_"), f"/settlement {h} ...: {desc[:140]}", "native settlement state/dialog updated per command; persisted across restart; non-owner refused where owned",
             {"caps": ["dialog_interaction"] if h in ("menu", "guide") else []}) for h, desc in helps]
    subs += [(f"{a}_{b}", f"/settlement {a} {b} ...: {d[:120]}", "economy/caravan/payroll state transitions exactly once; refused when preconditions fail",
              {"custody": a in ("commerce", "caravan", "payroll")}) for a, b, d in eco]
    subs += [("autonomy_growth", "generated village adopts and grows without player gifts", "ownerless growth; claim fencing", {"src": ["core:settlements/autonomy.rs"]}),
             ("persistence_restart", "settlement with buildings/households/economy across restart", "state identical", {"caps": ["server_clean_restart"]})]
    domain("settlements", ["core:settlements/mod.rs", "core:settlements/commands.rs", "core:settlements/ui.rs"], ["command_send"], subs)
    cl = json.loads(read(SOL / "tools/harness/catalog_load_scenarios.json"))["scenarios"]
    lm = {p.stem: json.loads(p.read_text()).get("description", "") for p in (SOL / "tools/harness/load_matrix_scenarios").glob("*.json")}
    domain("load", ["tools/harness/catalog_load_scenarios.json", "tools/harness/load_matrix_scenarios/"], ["multi_client_N"], [
        (s["id"], f"catalog-load scenario {s['id']} ({s.get('kind')}): {lm.get(s['id'], s.get('summary', ''))[:150]}",
         "classified receipt; MSPT/TPS/RSS within budget; zero protocol errors; world consistent after", {"src": ["tools/harness/backends/catalog_load.py"]})
        for s in cl])
    return {"roots": roots, "kinds": kinds, "effects": effects, "rules": rules, "living": living, "nonliving": nonliving,
            "professions": profs, "settlement_help": helps, "settlement_eco": eco,
            "menu_kinds": enum_variants(CORE / "inventory.rs", "MenuKind"),
            "click_modes": enum_variants(CORE / "boundary.rs", "ClickMode"),
            "player_inputs": enum_variants(CORE / "boundary.rs", "PlayerInput"),
            "goal_kinds": enum_variants(CORE / "blueprint.rs", "GoalKind"),
            "dimensions": enum_variants(CORE / "dimensions.rs", "Dimension"),
            "game_modes": enum_variants(CORE / "commands.rs", "GameMode"),
            "difficulties": enum_variants(CORE / "survival.rs", "Difficulty"),
            "weathers": enum_variants(CORE / "commands.rs", "Weather"),
            "mob_categories": enum_variants(DATA / "entity_contract_26_1_2.rs", "MobCategory"),
            "plugins": plugin_manifests(), "map_nonliving": map_nonliving}
