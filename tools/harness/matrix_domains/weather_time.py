"""Weather/time oracles; simulation ticks, never wall-clock sleeps."""
import re

from ..matrix_api import MissingCapability
from ._world import (adapter, block, boundary, check, command, condition, data,
                     entity_field, exists, observe, ticks, use)


def player_command(ctx, text):
    if ctx.factors['permission'] == 'console':
        command(ctx, text)
    else:
        ctx.clients[0].command(text)
    boundary(ctx)


def daytime(ctx):
    result = command(ctx, 'time query daytime')
    match = re.search(r'(?:The time is|commands\.time\.query[^0-9]*)(\d+)', result)
    if match is None:
        raise MissingCapability('console_time_query_numeric_feedback')
    return int(match[1])


def weather_cycle(ctx):
    for weather, rain, thunder in [('clear', False, False), ('rain', True, False), ('thunder', True, True)]:
        player_command(ctx, 'weather ' + weather)
        ticks(ctx, 100)
        actual = observe(ctx, 'weather')
        check(ctx, weather + ':rain-flag', actual.get('raining') == rain, actual, rain, 'Weather')
        check(ctx, weather + ':thunder-flag', actual.get('thundering') == thunder, actual, thunder, 'Weather')
    player_command(ctx, 'gamerule advance_weather false')
    before = observe(ctx, 'weather')
    ticks(ctx, 200)
    after = observe(ctx, 'weather')
    check(ctx, 'weather-cycle-frozen', after == before, after, before, 'Weather')
    player_command(ctx, 'gamerule advance_weather true')


def lightning_effects(ctx):
    for kind, converted in [('pig', 'zombified_piglin'), ('villager', 'witch')]:
        command(ctx, f'summon {kind} 8 100 8')
        command(ctx, 'summon lightning_bolt 8 100 8')
        ticks(ctx, 3)
        exists(ctx, f'@e[type=minecraft:{converted}]', source='Lightning#Effects_on_mobs')
        exists(ctx, f'@e[type=minecraft:{kind}]', source='Lightning#Effects_on_mobs', expected=False)
        command(ctx, f'kill @e[type=minecraft:{converted}]')
    command(ctx, 'summon creeper 8 100 8')
    command(ctx, 'summon lightning_bolt 8 100 8')
    ticks(ctx, 3)
    powered = entity_field(ctx, 'creeper', 'powered')
    check(ctx, 'charged-creeper', powered == 1, powered, 1, 'Creeper#Charged_creeper')
    command(ctx, 'kill @e[type=minecraft:creeper]')
    command(ctx, 'summon mooshroom 8 100 8 {Type:"red"}')
    command(ctx, 'summon lightning_bolt 8 100 8')
    ticks(ctx, 3)
    variant = entity_field(ctx, 'mooshroom', 'Type')
    check(ctx, 'mooshroom-lightning-toggle', variant == 'brown', variant, 'brown', 'Mooshroom')
    command(ctx, 'setblock 12 100 8 lightning_rod')
    command(ctx, 'summon lightning_bolt 12 101 8')
    ticks(ctx, 2)
    block(ctx, (12, 100, 8), 'lightning_rod[powered=true]', source='Lightning_Rod')


def snow_ice_accumulation(ctx):
    # Biome mutation is not implemented on the old release; fail rather than
    # interpreting a warm fixture's lack of snow as a weather bug.
    biome = observe(ctx, 'biome')
    if biome not in ('minecraft:snowy_plains', 'minecraft:ice_spikes', 'minecraft:snowy_taiga'):
        raise MissingCapability('fixture:cold_biome_at_test_coordinates')
    command(ctx, 'setblock 8 99 8 water')
    player_command(ctx, 'weather rain')
    command(ctx, 'gamerule max_snow_accumulation_height 2')
    ticks(ctx, 12000)
    block(ctx, (8, 99, 8), 'ice', source='Ice#Post-generation')
    found = any(condition(ctx, f'block {x} 100 {z} snow') for x in range(6, 11) for z in range(6, 11))
    check(ctx, 'snow-accumulated', found, found, True, 'Snow#Natural_generation')
    command(ctx, 'gamerule max_snow_accumulation_height 1')


def rain_effects(ctx):
    command(ctx, 'setblock 8 99 8 oak_planks')
    command(ctx, 'setblock 8 100 8 fire')
    command(ctx, 'setblock 10 100 8 cauldron')
    command(ctx, 'summon cow 12 100 8 {Fire:1000s}')
    player_command(ctx, 'weather rain')
    ticks(ctx, 200)
    block(ctx, (8, 100, 8), 'air', source='Rain')
    fire = entity_field(ctx, 'cow', 'Fire')
    check(ctx, 'rain-extinguishes-entity', fire <= 0, fire, '<=0', 'Rain')
    ticks(ctx, 12000)
    filled = any(condition(ctx, f'block 10 100 8 water_cauldron[level={n}]') for n in (1, 2, 3))
    check(ctx, 'rain-fills-cauldron-sample', filled, filled, True, 'Cauldron#Filling_with_water')


def no_weather_nether_end(ctx):
    player_command(ctx, 'weather rain')
    ticks(ctx, 100)
    precipitation = observe(ctx, 'weather.precipitation')
    check(ctx, 'no-nonoverworld-precipitation', precipitation == 'none', precipitation, 'none', 'Weather')


def day_night_cycle(ctx):
    player_command(ctx, 'gamerule advance_time false')
    player_command(ctx, 'time set 6000')
    before = daytime(ctx)
    ticks(ctx, 100)
    after = daytime(ctx)
    check(ctx, 'daytime-frozen', before == after == 6000, [before, after], [6000, 6000], 'Daylight_cycle')
    player_command(ctx, 'gamerule advance_time true')
    before = daytime(ctx)
    ticks(ctx, 100)
    after = daytime(ctx)
    check(ctx, 'daytime-advances', after > before, after-before, '>=100 plus command ticks', 'Daylight_cycle')


def sleep_skip_night(ctx):
    command(ctx, 'time set midnight')
    command(ctx, 'weather rain')
    command(ctx, 'gamerule players_sleeping_percentage 100')
    for i, client in enumerate(ctx.clients):
        x = 3 + 4*i
        command(ctx, f'setblock {x} 100 3 red_bed[part=foot,facing=south]')
        command(ctx, f'setblock {x} 100 4 red_bed[part=head,facing=south]')
        command(ctx, f'tp {client.name} {x+.5} 100 2.5')
        client.use_item_on((x, 100, 3))
    boundary(ctx)
    ticks(ctx, 120)
    now = daytime(ctx)
    permitted = ctx.factors['game_mode'] != 'spectator'
    check(ctx, 'sleep-quorum-skips-night', now < 2000 if permitted else now > 12000, now, 'morning' if permitted else 'still night', 'Bed#Sleeping')
    if permitted:
        check(ctx, 'sleep-clears-weather', observe(ctx, 'weather.raining') is False, observe(ctx, 'weather.raining'), False, 'Bed#Sleeping')


def sleep_refused(ctx):
    command(ctx, 'setblock 3 100 3 red_bed[part=foot,facing=south]')
    command(ctx, 'setblock 3 100 4 red_bed[part=head,facing=south]')
    for reason in ('day', 'monster', 'distance'):
        command(ctx, 'time set noon' if reason == 'day' else 'time set midnight')
        command(ctx, 'kill @e[type=minecraft:zombie]')
        if reason == 'monster':
            command(ctx, 'summon zombie 5 100 3')
        if reason == 'distance':
            command(ctx, f'tp {ctx.clients[0].name} 20 100 20')
        use(ctx, (3, 100, 3))
        ticks(ctx, 5)
        sleeping = observe(ctx, 'pose') == 'sleeping'
        check(ctx, 'sleep-refused:' + reason, not sleeping, sleeping, False, 'Bed#Sleeping')


ADAPTERS = {'weather_time.' + fn.__name__: adapter(fn) for fn in (
    weather_cycle, lightning_effects, snow_ice_accumulation, rain_effects,
    no_weather_nether_end, day_night_cycle, sleep_skip_night, sleep_refused,
)}
