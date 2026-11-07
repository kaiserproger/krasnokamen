"""Java 26.1.2 scoreboard contracts: https://minecraft.wiki/w/Scoreboard."""
from tools.harness.matrix_api import Adapter
from . import _platform as p

SOURCE = 'https://minecraft.wiki/w/Scoreboard'


def objectives_display(ctx):
    p.admin(ctx, 'scoreboard objectives add matrix dummy')
    p.admin(ctx, f'scoreboard players set {p.client(ctx).name} matrix 7')
    for slot in ('sidebar', 'list', 'below_name'):
        p.client(ctx).command(f'scoreboard objectives setdisplay {slot} matrix')
        for index in range(len(ctx.clients)):
            ctx.wait_until(lambda: p.observe(ctx, 'scoreboard.displays', index).get(slot) == 'matrix',
                           description=f'client {index} {slot} objective packet')
            p.check(ctx, f'client {index} {slot}', p.observe(ctx, 'scoreboard.displays', index).get(slot), 'matrix')
            p.check(ctx, f'client {index} display score',
                    p.observe(ctx, 'scoreboard.scores', index)[p.client(ctx).name]['matrix'], 7)


def scores_modify(ctx):
    name = p.client(ctx).name
    p.admin(ctx, 'scoreboard objectives add matrix dummy')
    if ctx.factors.get('permission') in ('non_op_0', 'op_1'):
        p.admin(ctx, f'scoreboard players set {name} matrix 7')
        p.admin(ctx, 'scoreboard players set factor matrix 3')
        commands = [f'scoreboard players {verb} {name} matrix {value}'
                    for verb, value in [('set', 9), ('add', 4), ('remove', 2)]]
        commands += [f'scoreboard players operation {name} matrix {op} factor matrix'
                     for op in ('+=', '-=', '*=', '/=', '%=', '<', '>', '=', '><')]
        commands.append(f'scoreboard players reset {name} matrix')
        for command in commands:
            p.response(ctx, command, r'Unknown or incomplete command|permission')
            p.query(ctx, f'scoreboard players get {name} matrix', r'\bhas 7 \[matrix\]')
            p.query(ctx, 'scoreboard players get factor matrix', r'\bhas 3 \[matrix\]')
        return
    for operation, expected in [('set 9', 9), ('add 4', 13), ('remove 2', 11)]:
        verb, value = operation.split()
        p.response(ctx, f'scoreboard players {verb} {name} matrix {value}', r'Updated scores for 1 holders')
        p.query(ctx, f'scoreboard players get {name} matrix', rf'\bhas {expected} \[matrix\]')
    p.admin(ctx, 'scoreboard players set factor matrix 3')
    for operation, expected in [('+=', 14), ('-=', 11), ('*=', 33), ('/=', 11), ('%=', 2), ('<', 2), ('>', 3), ('=', 3)]:
        p.response(ctx, f'scoreboard players operation {name} matrix {operation} factor matrix', r'Updated scores for 1 holders')
        p.query(ctx, f'scoreboard players get {name} matrix', rf'\bhas {expected} \[matrix\]')
    p.admin(ctx, 'scoreboard players set factor matrix 8')
    p.response(ctx, f'scoreboard players operation {name} matrix >< factor matrix', r'Updated scores for 1 holders')
    p.query(ctx, f'scoreboard players get {name} matrix', r'\bhas 8 \[matrix\]')
    p.query(ctx, 'scoreboard players get factor matrix', r'\bhas 3 \[matrix\]')
    p.response(ctx, f'scoreboard players reset {name} matrix', r'Updated scores for 1 holders')
    p.query(ctx, f'scoreboard players get {name} matrix', 'Score is not set')


def criteria_auto(ctx):
    name = p.client(ctx).name
    for objective, criterion in [('deaths', 'deathCount'), ('kills', 'playerKillCount'), ('hp', 'health'), ('food', 'food')]:
        reply = ctx.server.command(f'scoreboard objectives add {objective} {criterion}')
        p.test(ctx, 'criterion admitted ' + criterion, 'Created objective' in reply, reply)
    # Food is a known server gap; keep the failed assertion rather than turn it into a skip.
    p.query(ctx, f'scoreboard players get {name} hp', rf'has {int(p.observe(ctx, "health"))} \[hp\]')
    p.query(ctx, f'scoreboard players get {name} food', rf'has {int(p.observe(ctx, "food"))} \[food\]')
    p.admin(ctx, f'kill {name}')
    p.query(ctx, f'scoreboard players get {name} deaths', r'has 1 \[deaths\]')
    p.rich_action(ctx, 'respawn')
    other = p.client(ctx, 1)
    p.rich_action(ctx, 'attack_until_dead', p.observe(ctx, 'entity_id', 1))
    p.query(ctx, f'scoreboard players get {name} kills', r'has 1 \[kills\]')
    p.query(ctx, f'scoreboard players get {other.name} deaths', r'has 1 \[deaths\]')


def teams_membership(ctx):
    name = p.client(ctx).name
    p.client(ctx).command('team add matrix')
    p.client(ctx).command(f'team join matrix {name}')
    p.query(ctx, 'team list matrix', rf'Members:.*\b{name}\b')
    p.client(ctx).command('team modify matrix color red')
    p.check(ctx, 'red team packet', p.observe(ctx, 'teams')['matrix']['color'], 'red')
    p.client(ctx).command(f'team leave {name}')
    p.query(ctx, 'team list matrix', r'Members:\s*$')
    p.client(ctx).command(f'team join matrix {name}')
    p.client(ctx).command('team empty matrix')
    p.query(ctx, 'team list matrix', r'Members:\s*$')
    p.client(ctx).command('team remove matrix')
    p.test(ctx, 'team removed packet', 'matrix' not in p.observe(ctx, 'teams'), p.observe(ctx, 'teams'))


def team_options(ctx):
    p.admin(ctx, 'team add matrix')
    for actor in ctx.clients[:2]:
        p.admin(ctx, f'team join matrix {actor.name}')
    options = {'friendlyFire': 'false', 'color': 'red', 'prefix': '"[M]"', 'suffix': '"!"',
               'nametagVisibility': 'never', 'collisionRule': 'never',
               'seeFriendlyInvisibles': 'false', 'deathMessageVisibility': 'never'}
    for key, value in options.items():
        reply = ctx.server.command(f'team modify matrix {key} {value}')
        p.test(ctx, 'team option accepted ' + key, 'Modified team' in reply, reply)
        if 'Modified team' in reply:
            p.check(ctx, 'team option observed ' + key,
                    p.observe(ctx, 'teams')['matrix'][key],
                    {'false': False}.get(value, value.strip('"')))
    health = p.observe(ctx, 'health', 1)
    p.client(ctx).attack(p.observe(ctx, 'entity_id', 1))
    p.ticks(ctx, 10)
    p.check(ctx, 'friendly fire prevented', p.observe(ctx, 'health', 1), health)
    # Rendering and collision enforcement require actual-client observations, not just flags.
    p.check(ctx, 'nameplate hidden', p.rich_action(ctx, 'nameplate_visible', p.client(ctx, 1).name), False)
    p.rich_action(ctx, 'walk_through_player', p.client(ctx, 1).name)
    p.check(ctx, 'collision disabled', p.observe(ctx, 'last_collision.player_blocked'), False)
    p.admin(ctx, f'effect give {p.client(ctx, 1).name} minecraft:invisibility 30 0')
    p.check(ctx, 'invisible teammate hidden', p.rich_action(ctx, 'entity_render_visible', p.observe(ctx, 'entity_id', 1)), False)
    before = len(p.observe(ctx, 'death_messages'))
    p.admin(ctx, f'kill {p.client(ctx, 1).name}')
    p.ticks(ctx, 2)
    p.check(ctx, 'team death message hidden', p.observe(ctx, 'death_messages')[before:], [])


def trigger(ctx):
    name = p.client(ctx).name
    p.admin(ctx, 'scoreboard objectives add matrix trigger')
    p.admin(ctx, f'scoreboard players enable {name} matrix')
    p.response(ctx, 'trigger matrix add 1', r'Triggered matrix with value 1')
    p.query(ctx, f'scoreboard players get {name} matrix', r'has 1 \[matrix\]')
    p.response(ctx, 'trigger matrix add 1', r'That trigger is not enabled')
    p.query(ctx, f'scoreboard players get {name} matrix', r'has 1 \[matrix\]')


def scoreboard_persistence(ctx):
    name = p.client(ctx).name
    p.admin(ctx, 'scoreboard objectives add matrix dummy')
    p.admin(ctx, 'team add matrix')
    p.response(ctx, f'scoreboard players set {name} matrix 23', r'Updated scores for 1 holders')
    p.response(ctx, 'scoreboard objectives setdisplay sidebar matrix', r'Updated display slot')
    p.response(ctx, f'team join matrix {name}', r'Updated team membership for 1 players')
    p.response(ctx, 'team modify matrix color red', r'Modified team matrix')
    p.restart(ctx)
    p.query(ctx, f'scoreboard players get {name} matrix', r'has 23 \[matrix\]')
    p.query(ctx, 'team list matrix', rf'Members:.*\b{name}\b')
    p.check(ctx, 'display restored', p.observe(ctx, 'scoreboard.displays')['sidebar'], 'matrix')
    p.check(ctx, 'team color restored', p.observe(ctx, 'teams')['matrix']['color'], 'red')


ADAPTERS = {'scoreboard_teams.' + fn.__name__: Adapter(p.setup, fn, p.oracle) for fn in (
    objectives_display, scores_modify, criteria_auto, teams_membership, team_options, trigger, scoreboard_persistence)}
