"""Resource packs: https://minecraft.wiki/w/Server.properties#resource-pack-required.
Server evidence: mc-edge/src/resource_pack.rs:84-148. A status packet alone is
not proof of downloading/rendering; the real client's pack receipt is required.
"""
from tools.harness.matrix_api import Adapter
from . import _platform as p


def negotiate(ctx, *, required=False, policy='accept', broken=None, prompt=None):
    configured = p.fixture(ctx, 'resource_pack', required=required, broken=broken,
                           prompt=prompt, asset_marker='matrix-purple-checker')
    p.rich_action(ctx, 'reconnect_with_resource_pack_policy', policy)
    receipt = p.observe(ctx, 'resource_pack')
    p.check(ctx, 'offer identity', receipt['id'], configured['id'])
    p.check(ctx, 'offer required', receipt['required'], required)
    return configured, receipt


def pack_accept(ctx):
    configured, receipt = negotiate(ctx)
    p.check(ctx, 'download SHA1', receipt['download_sha1'], configured['sha1'])
    p.check(ctx, 'terminal status', receipt['status'], 'successfully_loaded')
    p.check(ctx, 'client pack active', receipt['active'], True)
    p.check(ctx, 'play reached', p.observe(ctx, 'connection.phase'), 'play')
    p.check(ctx, 'pack asset loaded', p.rich_action(ctx, 'resource_asset_marker'), 'matrix-purple-checker')


def pack_decline_optional(ctx):
    _, receipt = negotiate(ctx, policy='decline')
    p.check(ctx, 'declined', receipt['status'], 'declined')
    p.check(ctx, 'not active', receipt['active'], False)
    p.check(ctx, 'play continues', p.observe(ctx, 'connection.phase'), 'play')
    p.client(ctx).chat('matrix-pack-optional')
    ctx.wait_until(lambda: any('matrix-pack-optional' in s for s in p.observe(ctx, 'events')),
                   description='optional pack chat round trip')
    p.test(ctx, 'gameplay after decline', True, 'received own server chat')


def pack_decline_required(ctx):
    _, receipt = negotiate(ctx, required=True, policy='decline')
    p.check(ctx, 'declined', receipt['status'], 'declined')
    p.check(ctx, 'required decline disconnected', p.observe(ctx, 'connection.phase'), 'disconnected')
    reason = p.observe(ctx, 'connection.disconnect_reason')
    p.test(ctx, 'required decline reason', 'required resource pack' in reason.lower(), reason)


def pack_download_fail(ctx):
    for broken in ('http_404', 'sha1_mismatch'):
        for required in (False, True):
            _, receipt = negotiate(ctx, required=required, broken=broken)
            p.test(ctx, broken + ' load rejected', receipt['status'] in ('failed_download', 'failed_reload'), receipt)
            p.check(ctx, broken + ' active', receipt['active'], False)
            p.check(ctx, broken + ' session policy', p.observe(ctx, 'connection.phase'),
                    'disconnected' if required else 'play')


def pack_prompt_text(ctx):
    component = {'text': 'Matrix pack consent', 'color': 'gold', 'bold': True}
    _, receipt = negotiate(ctx, policy='prompt', prompt=component)
    p.check(ctx, 'prompt component', receipt['prompt'], component)
    p.check(ctx, 'rendered prompt text', p.rich_action(ctx, 'read_resource_pack_prompt'), 'Matrix pack consent')
    p.rich_action(ctx, 'resource_pack_choose', 'decline')
    p.check(ctx, 'optional prompt can decline', p.observe(ctx, 'connection.phase'), 'play')


ADAPTERS = {'resource_packs.' + fn.__name__: Adapter(p.setup, fn, p.oracle) for fn in (
    pack_accept, pack_decline_optional, pack_decline_required, pack_download_fail, pack_prompt_text)}
