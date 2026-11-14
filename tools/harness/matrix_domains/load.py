"""Catalog loads are not replaceable by a generic join/chat smoke test.

The actual fixtures, schedules, floors and numeric predicates already live in
catalog_controls.py, catalog_mixes.py and catalog_load_scenarios.json. The matrix
server facade has no canonical catalog-load context bridge. Until it does, these
rows fail with that specific missing capability BEFORE any fixture is changed.
A nested runner would lose matrix mode/permission/dimension and lifecycle identity.

Full per-subject expected state and existing validator references are in
scenarios/domains/load.yaml. CHUNK-01 additionally lacks the native generated-
chunk revision oracle (catalog_load_scenarios.json's explicit INFRA_BLOCKED).
"""
from tools.harness.matrix_domains.session import unsupported

CONTROL_IDS = (
    'dense-hostile', 'spread-passive', 'explore', 'built-base', 'fluids', 'village',
    'combat', 'projectiles', 'farms', 'plugin-heavy', 'checkpoint-restart',
    'long-session', 'MIX-01', 'MIX-02', 'MIX-04', 'MIX-08', 'MIX-18', 'MIX-21',
    'mixed-survival-long', 'JOIN-01', 'join-ready-assisted',
)
ADAPTERS = {
    'load.' + name: unsupported('catalog_load_matrix_context_bridge:' + name)
    for name in CONTROL_IDS
}
ADAPTERS['load.CHUNK-01'] = unsupported('native_generation_revision_oracle')
