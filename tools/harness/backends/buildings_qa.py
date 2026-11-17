"""In-game QA of authored blueprint construction and upgrades.

The sequential flow founds a settlement, stocks a real storage container,
orders a build and observes its completion. The windowed house path uses the
Projects and Buildings menus and their confirmation dialogs. A pending,
unfundable owner order fences the planner while each level is examined.

Each level requires a new same-run building ID, that exact level's completed
certificate and client-observed block IDs, an allowed entrance block after
teleport, a sampled light value and an instantaneous hostile census. Block
properties, traversal, door operation and hostile exclusion are not tested;
natural mob spawning is disabled. House bed capacity is checked, while sleep
is a non-gating observation. Farm, lumber, mine, mason, storage and barracks
have separate function checks; other workshop kinds do not prove job use.
Windowed runs retain an exterior frame per level for review, not an automatic
visual-quality verdict.

BUILDINGS_QA_SELECT chooses one member of BLUEPRINTS, or ALL (the default).

Owner modules, by domain (each is imported below; ``__getattr__`` forwards their names):
- ``buildings_qa_consts``: the selectable blueprints, menu slots and fixture geometry.
- ``buildings_qa_blueprint``: authored cells, rotation frame and orientation fits.
- ``buildings_qa_probe``: client and report primitives (blocks, chat, census, chests).
- ``buildings_qa_project``: Projects/Buildings menus, certificates, order retries.
- ``buildings_qa_build``: listings, hand shell, funding, anchors and captures.
- ``buildings_qa_scenarios``: the profile entry point and the parts split.
- ``buildings_qa_flows``: the sequential, parallel and per-blueprint flows.
- ``buildings_qa_verify``: the per-level oracle and the restart-and-resume.
- ``buildings_qa_functions``: per-kind function checks of a certified level.

The tests patch the client/report seams on *this* module
(``mock.patch.object(buildings_qa, "_read")``), so an owner module reaches them as
``_qa.<name>`` and never binds them; ``tools/harness/test_owner_modules.py`` holds that
line.

``re`` and ``native_client`` stay imported here because the gates read
``buildings_qa.re`` and patch ``buildings_qa.native_client.capture``.
"""

from __future__ import annotations

import re
from typing import Any

from . import native_client

from . import (
    buildings_qa_blueprint,
    buildings_qa_build,
    buildings_qa_consts,
    buildings_qa_flows,
    buildings_qa_functions,
    buildings_qa_probe,
    buildings_qa_project,
    buildings_qa_scenarios,
    buildings_qa_verify,
)

_OWNER_MODULES = (
    buildings_qa_consts,
    buildings_qa_blueprint,
    buildings_qa_probe,
    buildings_qa_project,
    buildings_qa_build,
    buildings_qa_scenarios,
    buildings_qa_flows,
    buildings_qa_verify,
    buildings_qa_functions,
)


def __getattr__(name: str) -> Any:
    """Forward a name from the one owner module that defines it (tests patch names here)."""
    for module in _OWNER_MODULES:
        try:
            return getattr(module, name)
        except AttributeError:
            continue
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
