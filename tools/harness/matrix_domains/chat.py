"""Chat probes; Java 26.1.2 TickThrottler(20, 200) and wiki/Commands/msg.
Negative delivery checks use dashboard tick fences, never sleeps.
"""
from functools import partial

from ..matrix_api import Adapter, Check, MissingCapability
from .commands import actor, field, fixture, permission_level, require_event_history, ticks


def verb(client, name, *args, **kwargs):
    method = getattr(client, name, None)
    if not callable(method):
        raise MissingCapability('client:' + name)
    return method(*args, **kwargs)


def setup(ctx, subject):
    require_event_history(ctx)
    if subject in {'public_chat', 'private_message', 'team_message', 'join_leave_messages', 'advancement_announce'} and len(ctx.clients) < 2:
        raise MissingCapability('multi_client_2')
    if subject == 'team_message':
        if len(ctx.clients) < 3:
            raise MissingCapability('multi_client_3:team_nonrecipient')
        fixture(ctx, 'team add matrix_chat')
        for c in ctx.clients[:2]:
            fixture(ctx, 'team join matrix_chat ' + c.name)
    if subject == 'advancement_announce':
        fixture(ctx, 'gamerule minecraft:show_advancement_messages true')
        fixture(ctx, f'advancement revoke {actor(ctx).name} only minecraft:story/root')
    ctx.state['starts'] = [len(field(c, 'events')) for c in ctx.clients]


def received(ctx, client, start, marker):
    return ctx.wait_until(lambda: any(marker in str(e) for e in field(client, 'events')[start:]),
                          description='received chat marker ' + marker)


def act(ctx, subject):
    p = actor(ctx)
    if subject == 'public_chat':
        p.chat('MatrixPublicOne')
        received(ctx, p, ctx.state['starts'][0], 'MatrixPublicOne')
        ctx.boundary()
        p.chat('MatrixPublicTwo')
    elif subject == 'private_message':
        p.command('msg ' + ctx.clients[1].name + ' MatrixPrivate')
    elif subject == 'team_message':
        p.command('teammsg MatrixTeam')
    elif subject == 'emote_say':
        p.command('me MatrixEmote')
        received(ctx, p, ctx.state['starts'][0], 'MatrixEmote')
        ctx.boundary()
        p.command('say MatrixSay')
    elif subject == 'join_leave_messages':
        ctx.clients[1].disconnect()
        ctx.wait_until(lambda: any(ctx.clients[1].name in str(e) and
                                  ('left' in str(e) or 'multiplayer.player.left' in str(e))
                                  for e in field(p, 'events')[ctx.state['starts'][0]:]),
                       description='observer receives named departure')
        ctx.boundary()
        ctx.clients[1].reconnect()
        ctx.wait_until(lambda: any(ctx.clients[1].name in str(e) and
                                  ('joined' in str(e) or 'multiplayer.player.joined' in str(e))
                                  for e in field(p, 'events')[ctx.state['starts'][0]:]),
                       description='observer receives named rejoin')
    elif subject == 'advancement_announce':
        fixture(ctx, f'advancement grant {p.name} only minecraft:story/root')
    elif subject == 'chat_spam':
        verb(p, 'chat_burst', ['MatrixSpam' + str(i) for i in range(10)])
    elif subject == 'secure_chat_session':
        verb(p, 'send_signed_chat', 'MatrixSigned')
    elif subject == 'chat_validation':
        ctx.state['invalid_results'] = []
        for text in ('x' * 257, 'Matrix\u00a7Illegal'):
            verb(p, 'send_raw_chat', text)
            ctx.wait_until(lambda: field(p, 'connected') is False,
                           description='raw invalid chat disconnects actor')
            ctx.state['invalid_results'].append(field(p, 'disconnect_reason'))
            p.reconnect()
        verb(p, 'send_raw_chat', '')
        ticks(ctx, 2)
        ctx.state['empty_connected'] = field(p, 'connected')
    elif subject == 'chat_visibility_setting':
        if len(ctx.clients) < 2:
            raise MissingCapability('multi_client_2')
        receiver = ctx.clients[1]
        ctx.state['visibility_results'] = []
        for visibility in ('full', 'commands_only', 'hidden'):
            verb(receiver, 'configure_chat', visibility=visibility)
            start = len(field(receiver, 'events'))
            p.chat('MatrixVisibility' + visibility)
            receiver.command('list')
            ticks(ctx, 2)
            ctx.state['visibility_results'].append((visibility, field(receiver, 'events')[start:]))
    marker = {'private_message': 'MatrixPrivate', 'team_message': 'MatrixTeam',
              'public_chat': 'MatrixPublicTwo', 'secure_chat_session': 'MatrixSigned'}.get(subject)
    if marker:
        received(ctx, p, ctx.state['starts'][0], marker)
    if subject not in {'public_chat', 'emote_say', 'join_leave_messages'}:
        ctx.boundary()
    if subject not in {'chat_validation', 'chat_spam'}:
        ticks(ctx, 2)


def oracle(ctx, subject):
    p = actor(ctx)
    events = [field(c, 'events')[start:] for c, start in zip(ctx.clients, ctx.state['starts'])]
    if subject == 'chat_spam':
        nonop = permission_level(ctx) == 0
        connected = field(p, 'connected')
        return [Check('spam_threshold', connected is not nonop, {'nonop': nonop, 'connected': connected})]
    if subject == 'chat_validation':
        return [Check('overlength_disconnected', bool(ctx.state['invalid_results'][0]), ctx.state['invalid_results'][0]),
                Check('illegal_character_disconnected', bool(ctx.state['invalid_results'][1]), ctx.state['invalid_results'][1]),
                Check('empty_chat_does_not_disconnect', ctx.state['empty_connected'] is True)]
    if subject == 'chat_visibility_setting':
        checks = []
        for visibility, seen in ctx.state['visibility_results']:
            public = any('MatrixVisibility' in str(e) for e in seen)
            checks.append(Check('public_chat_visibility:' + visibility, public == (visibility == 'full'), seen))
            command = any('players online' in str(e) or 'commands.list.players' in str(e) for e in seen)
            checks.append(Check('command_feedback_visibility:' + visibility, command == (visibility != 'hidden'), seen))
        return checks
    if subject == 'join_leave_messages':
        target = ctx.clients[1].name
        checks = [Check('leave_notification', any(target in str(e) and
                        ('left' in str(e) or 'multiplayer.player.left' in str(e)) for e in events[0]), events[0]),
                  Check('join_notification', any(target in str(e) and
                        ('joined' in str(e) or 'multiplayer.player.joined' in str(e)) for e in events[0]), events[0])]
        if not all(check.passed for check in checks):
            return checks
        for key in ('multiplayer.player.left', 'multiplayer.player.joined'):
            matching = [c for c in field(p, 'chat_components') if c.get('translate') == key and
                        any(arg.get('text') == target for arg in c.get('with') or [])]
            if not matching or 'color' not in matching[-1]:
                raise MissingCapability('snapshot:chat_components.join_leave_color')
            checks.append(Check('yellow_notification:' + key, matching[-1]['color'] == 'yellow', matching[-1]))
        return checks
    if subject == 'advancement_announce':
        return [Check('advancement_announcement:' + c.name,
                      any(p.name in str(e) and ('advancement' in str(e).lower() or 'Stone Age' in str(e) or 'Minecraft' in str(e)) for e in seen), seen)
                for c, seen in zip(ctx.clients, events)]
    if subject == 'emote_say':
        return [check for client, seen in zip(ctx.clients, events) for check in (
            Check('emote_broadcast:' + client.name,
                  any(f'* {p.name} MatrixEmote' in str(e) or
                      ('chat.type.emote' in str(e) and p.name in str(e) and 'MatrixEmote' in str(e))
                      for e in seen), seen),
            Check('say_permission:' + client.name, any('MatrixSay' in str(e) for e in seen) ==
                  (permission_level(ctx) >= 2), seen))]
    marker = {'public_chat': 'MatrixPublic', 'private_message': 'MatrixPrivate',
              'team_message': 'MatrixTeam', 'secure_chat_session': 'MatrixSigned'}[subject]
    checks = []
    for index, (c, seen) in enumerate(zip(ctx.clients, events)):
        matches = [str(e) for e in seen if marker in str(e)]
        expected = 2 if subject == 'public_chat' else int(subject not in {'private_message', 'team_message'} or index < 2)
        checks.append(Check('delivery_count:' + c.name, len(matches) == expected, matches))
        if subject == 'private_message' and expected and ctx.factors.get('plugin_profile') == 'live':
            prefix = f'[to {ctx.clients[1].name}]' if index == 0 else f'[from {p.name}]'
            checks.append(Check('essentials_private_format:' + c.name,
                                any(prefix in message for message in matches), matches))
        if subject == 'public_chat':
            checks.append(Check('delivery_order:' + c.name, len(matches) == 2 and 'One' in matches[0] and 'Two' in matches[1], matches))
            checks.append(Check('public_sender_format:' + c.name,
                                all(f'<{p.name}>' in message or
                                    ('chat.type.text' in message and p.name in message)
                                    for message in matches), matches))
    if subject == 'secure_chat_session':
        checks.append(Check('valid_profile_not_disconnected', field(p, 'connected') is True))
    if subject == 'private_message' and all(check.passed for check in checks):
        if permission_level(ctx) >= 2:
            raise MissingCapability('nonop_client:selector_permission_probe')
        starts = [len(field(client, 'events')) for client in ctx.clients]
        p.command('msg @a MatrixPrivateForbidden')
        expected = 'Online player not found or ambiguous.' if ctx.factors.get('plugin_profile') == 'live' else 'Selector not allowed'
        received(ctx, p, starts[0], expected)
        ticks(ctx, 2)
        for client, start in zip(ctx.clients, starts):
            seen = field(client, 'events')[start:]
            checks.append(Check('private_selector_no_delivery:' + client.name,
                                not any('MatrixPrivateForbidden' in str(event) for event in seen), seen))
    return checks


ADAPTERS = {'chat.' + s: Adapter(partial(setup, subject=s), partial(act, subject=s), partial(oracle, subject=s))
            for s in ('public_chat', 'chat_validation', 'chat_spam', 'secure_chat_session',
                      'private_message', 'emote_say', 'team_message', 'join_leave_messages',
                      'advancement_announce', 'chat_visibility_setting')}
