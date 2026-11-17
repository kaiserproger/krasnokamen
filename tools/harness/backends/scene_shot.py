"""Script a windowed native or vanilla-Java client scene on an isolated v2 world.

Owner modules, by domain (each is imported below; ``__getattr__`` forwards their names):
- ``scene_shot_client``: launch options, window helpers and the frame captures.
- ``scene_shot_steps``: the step vocabulary and its validation.
- ``scene_shot_execute``: the dispatcher that runs one step on the client.
- ``scene_shot_checks``: the retained-frame picture checks.
- ``scene_shot_scenario``: the scenario contract and the profile run.

The tests patch the capture and shot seams on *this* module
(``patch.object(scene_shot, "capture_native")``), so an owner module reaches them as
``_qa.<name>`` and never binds them; ``tools/harness/test_owner_modules.py`` holds that
line.

``subprocess``, ``runtime``, ``java_p5`` and ``native_client`` stay imported here
because the gates patch and read them through this module.
"""

from __future__ import annotations

import subprocess
from typing import Any

from .. import runtime
from . import java_p5, native_client

from . import (
    scene_shot_checks,
    scene_shot_client,
    scene_shot_execute,
    scene_shot_scenario,
    scene_shot_steps,
)

_OWNER_MODULES = (
    scene_shot_client,
    scene_shot_steps,
    scene_shot_execute,
    scene_shot_checks,
    scene_shot_scenario,
)


def __getattr__(name: str) -> Any:
    """Forward a name from the one owner module that defines it (tests patch names here)."""
    for module in _OWNER_MODULES:
        try:
            return getattr(module, name)
        except AttributeError:
            continue
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
