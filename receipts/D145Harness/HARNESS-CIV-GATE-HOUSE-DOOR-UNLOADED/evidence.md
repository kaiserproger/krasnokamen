# HARNESS-CIV-GATE-HOUSE-DOOR-UNLOADED — evidence

- Branch: `wt/D145Harness`, base `2f4d802c08eb6aa9ab18222582c9c39f13f39d55`.
- Fix commit in tree: `aa2afc814`
  (`harness(civ): reach a remote house door and read a remote storage by load, hops and the console teleport`).
  The row file already carries its four `python:` refs (`RemoteHouseDoorTests`).
- Production hunks: `tools/harness/backends/civilization_seed_qa.py`
  (`_walk_to_door` lateral hop candidates + `_DoorOutOfReach` + the console-teleport
  fallback in `capture_house`).

## Negative control (red)

A full `git apply -R` of `aa2afc814` is clean on the candidate but leaves two of
the kept tests **unrunnable** (`TypeError: capture_house() got an unexpected
keyword argument 'teleport'`, `KeyError: 'observer_teleported'`): the fix added
that API and the kept tests call it, so the mechanical reversal cannot carry
this row (promotion must use patch mode with an a/-side kept-API adaptation).

The faithful pre-image control instead reverts only the behaviour the fix added:
the hop candidate set back to the single straight-ahead candidate used before
`aa2afc814` (a0c0f8902's `_walk_to_door`), keeping the `teleport` parameter and
the `observer_teleported` field. Command:

    python3 -m unittest tools.harness.test_civilization_seed_qa.RemoteHouseDoorTests

Result: `FAILED (errors=1)` —
`test_a_refused_straight_walk_takes_a_lateral_hop_then_the_door` raises
`_DoorOutOfReach: house door (96, 64, -297) is out of the client's loaded radius:
hop 1 was refused on every candidate ((115, 77, -253): ... target is unreachable
...); the direct walk was refused (... target chunk is not client-loaded ...)`.
That error text is the row's own symptom (the door is not reached and the run
would abort before `new_house`). Full log: `control-red.log`.

## Green after restore

`RemoteHouseDoorTests` — `Ran 4 tests ... OK` (`control-green.log`). Whole module
on HEAD: `Ran 50 tests ... OK` (`../HEAD-full-module-green.log`).

## Promotion blocker (outside this lane)

`repro = "client"`: promotion needs a real-client `civilization-seed-qa` receipt
(`--client native --windowed`, matching pin) and, because of the kept-API
reversal above, patch mode with the `promotions/<ID>.hunks.md` map. Exact gate:

    <work>/solaris/tools/dev/scene-slot.sh <work>/solaris/tools/dev/mem-run.sh -m 8G \
      python3 -m tools.harness run civilization-seed-qa --client native --windowed
