"""Solaris Loader contracts (not vanilla). mc-edge/src/configuration.rs:95-97,212-216;
live-plugins/chisel/plugin.toml:10-19 is authoritative over the stale README.
"""
from tools.harness.matrix_api import Adapter
from . import _platform as p


def bundle_handshake_cache(ctx):
    bundle = p.fixture(ctx, 'loader_bundle', package='chisel', cache='empty')
    p.client(ctx).reconnect()
    first = p.observe(ctx, 'loader.bundle')
    p.check(ctx, 'manifest package', first['package'], 'chisel')
    p.check(ctx, 'verified bundle', first['sha256'], bundle['sha256'])
    p.check(ctx, 'first join download', first['download_count'], 1)
    p.client(ctx).disconnect()
    p.client(ctx).reconnect()
    second = p.observe(ctx, 'loader.bundle')
    p.check(ctx, 'cache identity', second['sha256'], bundle['sha256'])
    p.check(ctx, 'cache hit no redownload', second['download_count'], 0)
    p.check(ctx, 'cached pack activated', second['active'], True)


def optional_bundle_vanilla(ctx):
    from .plugins import deploy
    deploy(ctx, 'chisel', 'computer')
    p.rich_action(ctx, 'reconnect_profile', 'vanilla_26_1_2')
    p.check(ctx, 'optional bundle permits vanilla', p.observe(ctx, 'connection.phase'), 'play')
    p.check(ctx, 'vanilla uses no loader content', p.observe(ctx, 'loader.active_packages'), [])


def required_bundle_vanilla_refused(ctx):
    p.fixture(ctx, 'loader_bundle', package='chisel', required=True)
    p.rich_action(ctx, 'reconnect_profile', 'vanilla_26_1_2')
    p.check(ctx, 'required bundle denies vanilla', p.observe(ctx, 'connection.phase'), 'disconnected')
    reason = p.observe(ctx, 'connection.disconnect_reason')
    p.test(ctx, 'actionable Loader reason', 'Solaris Loader' in reason, reason)


def permission_prompt_cache(ctx):
    bundle = p.fixture(ctx, 'loader_bundle', package='chisel', required=False, cache='empty')
    p.rich_action(ctx, 'loader_permission_policy', {'register_items': False, 'load_assets': False})
    p.client(ctx).disconnect()
    p.client(ctx).reconnect()
    p.check(ctx, 'denied optional content inactive', p.observe(ctx, 'loader.active_packages'), [])
    p.check(ctx, 'Loader denial disconnects', p.observe(ctx, 'connection.phase'), 'disconnected')
    reason = p.observe(ctx, 'connection.disconnect_reason')
    p.test(ctx, 'stored denial reason', 'permissions were denied for' in reason, reason)
    p.client(ctx).disconnect()
    p.client(ctx).reconnect()
    p.check(ctx, 'denial cached', p.observe(ctx, 'loader.prompt_count'), 0)
    p.check(ctx, 'cached denial still rejects', p.observe(ctx, 'connection.phase'), 'disconnected')
    p.rich_action(ctx, 'loader_reset_permission_decision')
    p.rich_action(ctx, 'loader_permission_policy', {'register_items': True, 'load_assets': True})
    p.client(ctx).disconnect()
    p.client(ctx).reconnect()
    p.test(ctx, 'allowed content active', 'chisel' in p.observe(ctx, 'loader.active_packages'), p.observe(ctx, 'loader.active_packages'))
    p.check(ctx, 'allowed bundle verified', p.observe(ctx, 'loader.bundle.sha256'), bundle['sha256'])
    p.check(ctx, 'allowed first download', p.observe(ctx, 'loader.bundle.download_count'), 1)
    p.client(ctx).disconnect()
    p.client(ctx).reconnect()
    p.check(ctx, 'permissions cached', p.observe(ctx, 'loader.prompt_count'), 0)
    p.check(ctx, 'cached allowed bundle not downloaded', p.observe(ctx, 'loader.bundle.download_count'), 0)


def loader_view_request(ctx):
    view = p.fixture(ctx, 'loader_view', package='solaris-towns')
    p.rich_action(ctx, 'loader_view_request', view['id'])
    p.check(ctx, 'view identity', p.observe(ctx, 'loader.view.id'), view['id'])
    p.check(ctx, 'view authoritative data', p.observe(ctx, 'loader.view.data'), view['expected_data'])


def custom_items_render(ctx):
    p.fixture(ctx, 'loader_bundle', package='chisel')
    p.admin(ctx, f'give {p.client(ctx).name} chisel:chisel 1')
    p.check(ctx, 'custom model rendered', p.rich_action(ctx, 'item_model', 'chisel:chisel'), 'chisel:chisel')
    p.rich_action(ctx, 'select_inventory_item', 'chisel:chisel')
    p.check(ctx, 'held custom model', p.rich_action(ctx, 'held_item_model'), 'chisel:chisel')
    p.client(ctx).drop()
    p.check(ctx, 'world custom model', p.rich_action(ctx, 'dropped_item_model', 'chisel:chisel'), 'chisel:chisel')


def stale_bundle_update(ctx):
    first = p.fixture(ctx, 'loader_bundle', package='chisel', revision='A')
    p.client(ctx).reconnect()
    p.check(ctx, 'revision A', p.observe(ctx, 'loader.bundle.sha256'), first['sha256'])
    p.client(ctx).disconnect()
    second = p.fixture(ctx, 'loader_bundle', package='chisel', revision='B')
    p.test(ctx, 'fixture really changed', first['sha256'] != second['sha256'], [first, second])
    p.client(ctx).reconnect()
    p.check(ctx, 'revision B verified', p.observe(ctx, 'loader.bundle.sha256'), second['sha256'])
    p.check(ctx, 'revision B downloaded', p.observe(ctx, 'loader.bundle.download_count'), 1)
    p.check(ctx, 'revision B rendered', p.rich_action(ctx, 'loader_asset_marker'), second['marker'])


ADAPTERS = {'loader_content.' + fn.__name__: Adapter(p.setup, fn, p.oracle) for fn in (
    bundle_handshake_cache, optional_bundle_vanilla, required_bundle_vanilla_refused,
    permission_prompt_cache, loader_view_request, custom_items_render, stale_bundle_update)}
