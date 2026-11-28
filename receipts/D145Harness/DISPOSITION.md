# D145 LedgerHarness — area=harness disposition (44 open rows)

Branch `wt/D145Harness`, base `2f4d802c08eb6aa9ab18222582c9c39f13f39d55`.
Scope (owner D146): exactly `area=harness`; gameplay rows belong to `LedgerGameplay`.

Validation on this tree:
`python3 -m unittest` over all 68 registered `python:` refs of the 44 open
harness rows — `Ran 68 tests ... OK` (25.4 s), log `ALL-PYTHON-REFS-green.log`.
`ledger probe` of the five rows this lane edited — all `green`
(`.analysis/validation/20261128T082351-ledger-probe-wx1v_4om/probe.json`).

## Advanced by this lane (5)

| Row | Change | Control |
|---|---|---|
| HARNESS-CIV-GATE-LOCAL-SCAN-UNLOADED | recorded `fix_commit 457d77778`, added the python ref that was only named in `repro_check` | red `'minecraft_scan_blocks' != 'minecraft_wait_for_loaded_block'`, green after restore |
| HARNESS-CIV-GATE-HOUSE-DOOR-UNLOADED | recorded `fix_commit aa2afc814` (4 python refs already on the row) | red: straight-walk refusal makes the door unreachable; green 4/4 |
| HARNESS-CIV-GATE-STORAGE-READ-TIMEOUT | recorded `fix_commit a0c0f8902` | red: 15 s storage timeout on a reply that is on disk mid-line; green |
| HARNESS-CIV-GATE-PLANNER-READ-BLOCKED-BY-STATUS-DIALOG | recorded `fix_commit a0c0f8902` | red: `AssertionError: 'storage_error' unexpectedly found`, `AssertionError: 1 != 2`; green |
| PERF-RECEIPT-HAS-NO-CLIENT-OBSERVED-TICK-ARRIVAL | recorded `fix_commit 2941bc91f`, added a green python ref (row had none), completed the clipped `repro_check` | none constructible: the fix created `client_tick_arrival`, reversal leaves the ref unrunnable |

Evidence: `receipts/D145Harness/<ID>/` (`evidence.md`, `control-red.log`,
`control-green.log`, `ref-green.log`). All red/green controls ran in a
disposable detached worktree; the lane tree was never used as the control tree.

The four civ-gate rows still cannot promote here: `repro = "client"` requires a
real-client `civilization-seed-qa` receipt on a matching pin
(`python3 -m tools.harness run civilization-seed-qa --client native --windowed`
through `scene-slot.sh`/`mem-run.sh`). That is the next gate run, not a code gap.

## Already fixed in tree, waiting on Main promotion / real-client evidence (20)

APPEND-HALT-TEST-ORDERS-WITH-SLEEP (test-only fix, no honest red),
CIV-FIXTURE-INACTIVE-MINE, CIV-HOUSE-CAPTURE-CLOSED-ENTRANCE,
CIV-IRON-CAPTURE-OUTPUT-COLLECTED, CIV-STALL-ORACLE-WAITING-MATERIALS,
HARNESS-BATTLE-CRASH-HANDOFF-LOST-ON-EARLY-SIGKILL,
HARNESS-CIV-GATE-BIRTH-CAPTURE-HITS-THE-WALL,
HARNESS-CIV-GATE-IRON-EVIDENCE-WINDOW,
HARNESS-CIV-GATE-OBSERVER-WAIT-HANGS,
HARNESS-CIV-GATE-REMOTE-SCAN-UNLOADED,
HARNESS-DAMAGE-GATE-FIXTURE-SELF-KILL,
HARNESS-EXCLUSIVE-WINDOW-SCENE-SLOT-DEADLOCK (manual fleet repro only),
HARNESS-JAVA-MATRIX-26-1-2-EVENTS-NONE-ADAPTER-TYPEERROR,
HARNESS-JAVA-MATRIX-FOREIGN-PORT-AUTH-ACK,
HARNESS-LOADER-SHARED-BUILD-DIR-RACE, HARNESS-SCENE-SHOT-MISSING-SUBJECT,
HARNESS-VOICE-RECONNECT-CAPABILITY-REGISTRATION,
JAVA-BATTLE-TAKE-END-CAMERA, TEST-JAVA-CAPABILITIES, TEST-NATIVE-HEALTH.

## Blocked outside this lane (19)

- Policy/control rule (no faithful control, fix is test-only or created the
  surface): HARNESS-ADMIN-TEST-TEMP-DIR-LEAK, HARNESS-REGRESSIONS-CLIENT-REBUILDS,
  TEST-MATRIX-CONSOLE-FENCE-CHAT, TEST-NATIVE-IMPORT.
- Needs a real client / hardware this lane does not own:
  JAVA-BATTLE-END-CAMERA-EMPTY-FRAME, JAVA-BATTLE-SEED46-APPROACH,
  LOADER-GAIT-SIDECAR-EMPTY, TEST-JAVA-PREDICTION-ACK-LANES,
  TEST-WINQA-OBSERVATION-TRIGGERS, DAMAGE-GATE-REJECTED-REGENERATION-RULE-REMAINDER.
- Loader repository fixes: JAVA-BATTLE-SPEAR-PLAIN-ATTACK,
  LOADER-MCP-ATTACK-IGNORES-USE-ITEM-GATE, LOADER-OBSERVATION-EQUIPMENT.
- Performance window / measurement (Main's `-x`):
  COREPERF-REGION-ATTRIBUTION-PERTURBS-MEASURED-MEMORY-TRAFFIC,
  PERF1M-LADDER-DOUBLES-RESIDENTS-AND-OBSERVERS,
  PERF1M-LADDER-LEGS-RAN-WITHOUT-QUIET-WINDOW, RELEASE-BUILD-HAS-NO-TARGET-CPU.
- No deterministic reproduction: CORE-DROP-TICK-STREAM-CLOSED-BEFORE-DROP,
  PLUGIN-ENGINE-CACHE-TEST-COUNTS-ASYNC-FILE.
