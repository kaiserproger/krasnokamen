"""One named real-client assertion per open civilization ledger row.

`civilization-seed-qa` already records, once per sprint, the settlement's own dialogs and
the client's received entities: the Construction status body, the storage totals, the
building list and the resident census. Each function
here turns those records into the verdict a row registers as
`harness:civilization-seed-qa:<signal name>`.

The rules are pure functions of the recorded sprints, so the same code the live gate runs
can be driven with a recorded pre-fix observation (the row's own numbers) and with the fixed
one: `test_civ_row_signals` watches every signal go red on the row's symptom and green on
the recovery. A signal answers `None` when its surface was not observed, which the ledger
reads as "not evidence" and never as red.

False is therefore only ever the row's documented symptom, never "no progress yet":
every stall rule needs a full game day of unchanged observation (`DAY` ticks) *and* the
row's own qualifier (a funded house waiting on logs, a Farm naming 0 wheat seeds, a
prepared project whose revision never moves, a worker past the simulated boundary, ...).
"""

from __future__ import annotations

import re
from typing import Any

# One complete day/night cycle: a settlement that has not moved in this long is stalled,
# not merely between work shifts (the scenario's own STALL_TICKS).
DAY = 24000
# The client's simulation-boundary proxy: entities past this horizontal distance are
# outside every loaded/simulated window the gate observes.
SIMULATED_RADIUS = 64

# Food/seed/wool/log item prefixes the signals read from the storage totals.
LOG_ITEM = re.compile(r"(?:^|:)(?:\w+_)?log$")
WOOD_SPECIES = re.compile(r"(?:^|:)(\w+)_log$")

STREET_BLUEPRINTS = frozenset({"road", "lamp", "meadow"})
HOUSE_BLUEPRINTS = frozenset({"house", "storage"})


def _sprints(observation: dict[str, Any]) -> list[dict[str, Any]]:
    rows = observation.get("sprints") if isinstance(observation, dict) else None
    return [row for row in rows or [] if isinstance(row, dict)]


def _ticks(row: dict[str, Any]) -> int | None:
    value = row.get("ticks")
    return value if isinstance(value, int) else None


def _days(rows: list[dict[str, Any]]) -> list[list[dict[str, Any]]]:
    """Every maximal run of sprints that spans at least one game day, in order.

    A run is maximal so a stall interrupted by an observation gap is judged on the
    samples that exist; the caller's predicate must hold on every sample.
    """
    out = []
    start = None
    for index, row in enumerate(rows):
        ticks = _ticks(row)
        if ticks is None:
            continue
        if start is None:
            start = index
            continue
        first = _ticks(rows[start])
        if first is not None and ticks - first >= DAY:
            out.append(rows[start:index + 1])
    return out


def _storage(row: dict[str, Any]) -> dict[str, int]:
    totals = row.get("storage")
    if not isinstance(totals, dict):
        return {}
    return {str(item): int(count) for item, count in totals.items()
            if isinstance(count, int)}


def _count(storage: dict[str, int], pattern: re.Pattern[str]) -> int:
    return sum(count for item, count in storage.items() if pattern.search(item))


def _logs(storage: dict[str, int]) -> int:
    return _count(storage, LOG_ITEM)


def _projects(row: dict[str, Any]) -> list[dict[str, Any]]:
    projects = row.get("projects")
    return [project for project in projects or [] if isinstance(project, dict)]


def _waiting(row: dict[str, Any]) -> str:
    """The Construction status body the client received, lower-cased for token checks."""
    body = row.get("dialog")
    if isinstance(body, str):
        return body.lower()
    if isinstance(body, list):
        return "\n".join(str(line) for line in body).lower()
    return ""


def _blueprint(project: dict[str, Any]) -> str:
    return str(project.get("blueprint") or "")


def _done(project: dict[str, Any]) -> int:
    value = project.get("done")
    return value if isinstance(value, int) else 0


def _total(project: dict[str, Any]) -> int:
    value = project.get("total")
    return value if isinstance(value, int) else 0


def _phase(project: dict[str, Any]) -> str:
    return str(project.get("phase") or "").lower()


def _complete(project: dict[str, Any]) -> bool:
    total = _total(project)
    return total > 0 and _done(project) >= total


def _resident_positions(row: dict[str, Any]) -> list[tuple[float, float, float]]:
    out = []
    for entity in row.get("entities") or []:
        if not isinstance(entity, dict) or entity.get("entity_type") != "minecraft:villager":
            continue
        try:
            out.append((float(entity["x"]), float(entity["y"]), float(entity["z"])))
        except (KeyError, TypeError, ValueError):
            continue
    return out


def _anchor(row: dict[str, Any]) -> tuple[float, float, float] | None:
    value = row.get("anchor")
    if isinstance(value, (list, tuple)) and len(value) == 3:
        try:
            return (float(value[0]), float(value[1]), float(value[2]))
        except (TypeError, ValueError):
            return None
    return None


def _logs_flat_over_a_day(rows: list[dict[str, Any]], *, require_project: bool = True) -> bool:
    """The row's lumber stall: a funded house/storage project waits a full day with no
    delivered log beyond the founding deposit."""
    for run in _days(rows):
        if require_project and not any(
                _blueprint(project) in HOUSE_BLUEPRINTS and not _complete(project)
                for row in run for project in _projects(row)):
            continue
        baseline = _logs(_storage(run[0]))
        if baseline <= 0:
            continue
        if all(_logs(_storage(row)) <= baseline for row in run):
            return True
    return False


# --- the signals -------------------------------------------------------------------

def lumberjack_delivers_logs_within_a_day(rows: list[dict[str, Any]]) -> bool | None:
    """CIV-SEED24-CROSS-VILLAGE-TREE-SWEEP: a lumberjack's tree reaches the settlement's
    storage. Red: a funded house/storage project waits a full day while storage never
    gains a log past the founding deposit (the sweep walked 85 blocks past a tree 5.5
    blocks away and carried the load until the shift ended)."""
    if not rows:
        return None
    baseline = _logs(_storage(rows[0]))
    if any(_logs(_storage(row)) > baseline for row in rows[1:]):
        return True
    if _logs_flat_over_a_day(rows):
        return False
    return None


def first_farm_gets_seeds_from_cleared_ground(rows: list[dict[str, Any]]) -> bool | None:
    """CIV-SEED24-FARM-SEEDS-NEED-A-FARM: the first Farm's 12 wheat seeds come from
    clearing plants, never from a farm. Red: the Construction body names a Farm short of
    `wheat seeds 0/` for a full day while storage holds no wheat seeds (the row's
    'Wheat Seeds 0/12 — needs a farm' from the first sprint to the last)."""
    if not rows:
        return None
    for run in _days(rows):
        stalled = all(
            "farm" in _waiting(row) and re.search(r"wheat seeds 0/", _waiting(row))
            and _storage(row).get("minecraft:wheat_seeds", 0) == 0
            for row in run)
        if stalled:
            return False
    if any(_storage(row).get("minecraft:wheat_seeds", 0) > 0 for row in rows):
        return True
    return None


def wool_reaches_a_wool_bill_without_a_shepherd(rows: list[dict[str, Any]]) -> bool | None:
    """CIVILIZATION-EMPLOYED-SHEARING: a wool bill is filled by an otherwise eligible
    working adult. Red: the Construction body reports a wool shortage at 0 for a full day
    while storage holds no wool and no resident is a shepherd."""
    if not rows:
        return None

    def not_a_shepherd(row: dict[str, Any]) -> bool:
        return not any("shepherd" in str(entity.get("profession") or "").lower()
                       for entity in row.get("entities") or []
                       if isinstance(entity, dict) and entity.get("entity_type") == "minecraft:villager")

    for run in _days(rows):
        stalled = all(
            re.search(r"(?:^|\s)wool\b.*\b0/", _waiting(row))
            and _count(_storage(row), re.compile(r"(?:^|:)(?:\w+_)?wool$")) == 0
            and not_a_shepherd(row)
            for row in run)
        if stalled:
            return False
    if any(_count(_storage(row), re.compile(r"(?:^|:)(?:\w+_)?wool$")) > 0 for row in rows):
        return True
    return None


def funded_project_advances_within_a_day(rows: list[dict[str, Any]]) -> bool | None:
    """CIVILIZATION-FUNDED-PLANNER-IDLE: a funded candidate is put to work. Red: a project
    sits in `Prepared` at 0 done with the settlement revision unchanged for a full day
    (the row's revision 15, cursor 0 through the funded idle interval)."""
    if not rows:
        return None
    seen: dict[tuple[str, int, tuple[Any, ...]], tuple[int, int]] = {}
    for index, row in enumerate(rows):
        revision = row.get("revision")
        for project in _projects(row):
            if _phase(project) not in ("prepared", "building") or _done(project) > 0:
                continue
            key = (_blueprint(project), int(project.get("level") or 1), tuple(project.get("origin") or ()))
            first = seen.setdefault(key, (index, _ticks(row) or 0))
            ticks = _ticks(row) or 0
            if ticks - first[1] >= DAY and revision == rows[first[0]].get("revision"):
                return False
    for index, row in enumerate(rows):
        for project in _projects(row):
            if _phase(project) in ("prepared", "building") and _done(project) > 0:
                return True
    return None


def no_worker_stranded_past_the_simulated_boundary(rows: list[dict[str, Any]]) -> bool | None:
    """CIVILIZATION-INACTIVE-SHEEP-TARGET: nobody walks to a sheep beyond the simulation
    distance. Red: for a full day a resident stands more than 64 blocks from the anchor
    while no wool arrives (the row's worker stranded at the inactive boundary)."""
    if not rows:
        return None

    def stranded(row: dict[str, Any]) -> bool:
        anchor = _anchor(row)
        if anchor is None:
            return False
        return any(((x - anchor[0]) ** 2 + (z - anchor[2]) ** 2) ** 0.5 > SIMULATED_RADIUS
                   for x, _, z in _resident_positions(row))

    for run in _days(rows):
        stalled = all(
            stranded(row)
            and _count(_storage(row), re.compile(r"(?:^|:)(?:\w+_)?wool$")) == 0
            for row in run)
        if stalled:
            return False
    if any(_count(_storage(row), re.compile(r"(?:^|:)(?:\w+_)?wool$")) > 0 for row in rows):
        return True
    return None


def mixed_wood_load_reaches_storage(rows: list[dict[str, Any]]) -> bool | None:
    """CIVILIZATION-MIXED-LOG-HAUL-STARVATION: a carried load of more than one species is
    delivered. Red: for a full day storage holds only the founding oak while a funded
    house/storage project waits (61 birch and 6 oak stayed carried, returning=false)."""
    if not rows:
        return None

    def species(row: dict[str, Any]) -> set[str]:
        found = set()
        for item in _storage(row):
            match = WOOD_SPECIES.search(item)
            if match:
                found.add(match.group(1))
        return found

    for run in _days(rows):
        waiting = any(_blueprint(project) in HOUSE_BLUEPRINTS and not _complete(project)
                      for row in run for project in _projects(row))
        if waiting and all(len(species(row)) <= 1 for row in run):
            return False
    if any(len(species(row)) >= 2 for row in rows):
        return True
    return None


def natural_forest_site_is_felled_and_housing_grows(rows: list[dict[str, Any]]) -> bool | None:
    """CIVILIZATION-NATURAL-FOREST-SITE-PREPARATION: a natural forest lot is felled into
    storage and a house then rises. Red: a queued House order waits a full day with no
    house project and no log delivered past the founding deposit (185 lots, none free)."""
    if not rows:
        return None
    for run in _days(rows):
        pending_house = all("pending" in _waiting(row) and "house" in _waiting(row)
                            for row in run)
        no_house = all(not any(_blueprint(project) in HOUSE_BLUEPRINTS
                               for project in _projects(row)) for row in run)
        baseline = _logs(_storage(run[0]))
        flat = all(_logs(_storage(row)) <= baseline for row in run)
        if pending_house and no_house and flat:
            return False
    for row in rows:
        if (_logs(_storage(row)) > _logs(_storage(rows[0]))
                and any(_blueprint(project) in HOUSE_BLUEPRINTS and _done(project) > 0
                        for project in _projects(row))):
            return True
    return None


def street_completion_releases_the_builder(rows: list[dict[str, Any]]) -> bool | None:
    """CIVILIZATION-STREET-BUILDER-RETENTION: after a street/lamp/meadow project completes
    the only reachable adult is free for extraction and wool. Red: a full day after that
    completion with no non-street project progress and no stock movement."""
    if not rows:
        return None
    completed_at: list[int] = []
    for index, row in enumerate(rows):
        if any(_blueprint(project) in STREET_BLUEPRINTS and _complete(project)
               for project in _projects(row)):
            completed_at.append(index)
    if not completed_at:
        return None
    cut = completed_at[0]
    after = rows[cut:]
    if len(after) < 2:
        return None

    def moved(before: dict[str, Any], after_row: dict[str, Any]) -> bool:
        for project in _projects(after_row):
            if _blueprint(project) in STREET_BLUEPRINTS:
                continue
            if _done(project) > 0:
                return True
        return _logs(_storage(after_row)) > _logs(_storage(before)) or \
            _count(_storage(after_row), re.compile(r"(?:^|:)(?:\w+_)?wool$")) > \
            _count(_storage(before), re.compile(r"(?:^|:)(?:\w+_)?wool$"))

    if any(moved(after[index], after[index + 1]) for index in range(len(after) - 1)):
        return True
    for run in _days(after):
        if not any(moved(run[0], row) for row in run[1:]):
            return False
    return None


SIGNALS: dict[str, Any] = {
    "lumberjack_delivers_logs_within_a_day": lumberjack_delivers_logs_within_a_day,
    "first_farm_gets_seeds_from_cleared_ground": first_farm_gets_seeds_from_cleared_ground,
    "wool_reaches_a_wool_bill_without_a_shepherd": wool_reaches_a_wool_bill_without_a_shepherd,
    "funded_project_advances_within_a_day": funded_project_advances_within_a_day,
    "no_worker_stranded_past_the_simulated_boundary": no_worker_stranded_past_the_simulated_boundary,
    "mixed_wood_load_reaches_storage": mixed_wood_load_reaches_storage,
    "natural_forest_site_is_felled_and_housing_grows": natural_forest_site_is_felled_and_housing_grows,
    "street_completion_releases_the_builder": street_completion_releases_the_builder,
}

# The rows each signal is registered for; `test_civ_row_signals` and the ledger row files
# name the same pairs.
ROWS: dict[str, str] = {
    "lumberjack_delivers_logs_within_a_day": "CIV-SEED24-CROSS-VILLAGE-TREE-SWEEP",
    "first_farm_gets_seeds_from_cleared_ground": "CIV-SEED24-FARM-SEEDS-NEED-A-FARM",
    "wool_reaches_a_wool_bill_without_a_shepherd": "CIVILIZATION-EMPLOYED-SHEARING",
    "funded_project_advances_within_a_day": "CIVILIZATION-FUNDED-PLANNER-IDLE",
    "no_worker_stranded_past_the_simulated_boundary": "CIVILIZATION-INACTIVE-SHEEP-TARGET",
    "mixed_wood_load_reaches_storage": "CIVILIZATION-MIXED-LOG-HAUL-STARVATION",
    "natural_forest_site_is_felled_and_housing_grows": "CIVILIZATION-NATURAL-FOREST-SITE-PREPARATION",
    "street_completion_releases_the_builder": "CIVILIZATION-STREET-BUILDER-RETENTION",
}


def evaluate(observation: dict[str, Any]) -> dict[str, bool | None]:
    """Every signal's verdict for one run's recorded observation (None = not observed)."""
    rows = _sprints(observation)
    return {name: signal(rows) for name, signal in SIGNALS.items()}


def observation_from_run(detail: dict[str, Any]) -> dict[str, Any]:
    """The observation contract `evaluate` reads, built from a civilization run's detail.

    The scenario appends one sprint row per observation; a sprint whose dialog could not
    be read keeps the run's last readable body so a failed read is not counted as a stall
    sample (it is dropped here when the sprint has no `ticks`).
    """
    return {"sprints": [row for row in detail.get("row_signals") or []
                        if isinstance(row, dict) and isinstance(row.get("ticks"), int)]}
