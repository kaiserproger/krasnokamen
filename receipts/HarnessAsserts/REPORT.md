# HarnessAsserts — one real-client assertion per open row (2026-11-28)

Thirteen open ledger rows carried a fix patch and only `cargo:` refs, so the ledger
(`tools/harness/regressions.py`: `needs_real_client` / `is_graphical_ref`) could not promote
them. Each row now carries one named assertion inside an **existing** scenario that reads the
quantity its fix changes. This page records, per row: the scenario, the assertion, the
observation that makes it red (the row's own symptom, driven through the same code the live
gate runs) and the observation that makes it green, with the assertion's source line.

No `status` was changed anywhere: promotion stays with `python3 -m tools.harness ledger
promote`. Every ref below is marked in its row as **new and not yet seen in a full scenario
run**; the red/green here are the assertion's decision path on recorded observations, not a
scenario pass.

Raw evidence beside this file: `signals-red-green.log`, `verge-red-green.log`,
`test_civ_row_signals.log`, `test_datapack_worldgen_skip.log`,
`test_entity_relative_trajectory.log`, `worldgen-reversed-cargo.log`.

## What "red" means for these new refs

The eight civilization rows share one run (`civilization-seed-qa`), whose sprint record already
holds the Construction dialog body, the building list, the storage totals and the resident
census. `tools/harness/backends/civ_row_signals.py` turns that record into one verdict per row
and publishes it in `details.verdicts`, exactly where `regressions._profile_verdict` reads it.
A signal answers `False` only for the row's documented stall across a full game day
(`DAY = 24000` ticks) and `None` when its surface was not observed — never a false red for a run
that is merely early.

## Rows

| Row | Scenario | Assertion (`file:line`) | Red observation | Green observation |
|---|---|---|---|---|
| CIV-SEED24-CROSS-VILLAGE-TREE-SWEEP | civilization-seed-qa | `lumberjack_delivers_logs_within_a_day` (`civ_row_signals.py:191`) | the 32 founding oak logs unchanged for one day while a funded House waits (`False`) | storage oak logs rise above the founding deposit (`True`) |
| CIV-SEED24-FARM-SEEDS-NEED-A-FARM | civilization-seed-qa | `first_farm_gets_seeds_from_cleared_ground` (`civ_row_signals.py:206`) | `Farm: Wheat Seeds 0/12 — needs a farm` in the Construction body for one day with 0 wheat seeds stored (`False`) | wheat seeds appear in storage from cleared ground (`True`) |
| CIVILIZATION-EMPLOYED-SHEARING | civilization-seed-qa | `wool_reaches_a_wool_bill_without_a_shepherd` (`civ_row_signals.py:225`) | a wool bill at `0/` for one day, 0 wool stored, no shepherd in the census (`False`) | wool reaches storage with employed non-shepherd adults (`True`) |
| CIVILIZATION-FUNDED-PLANNER-IDLE | civilization-seed-qa | `funded_project_advances_within_a_day` (`civ_row_signals.py:250`) | a `Prepared` project at 0 done with `revision=15` unchanged for a full day (`False`) | the project's done rises and the revision moves (`True`) |
| CIVILIZATION-INACTIVE-SHEEP-TARGET | civilization-seed-qa | `no_worker_stranded_past_the_simulated_boundary` (`civ_row_signals.py:285`) | a resident stands >64 blocks from the anchor for a day with no wool (`False`) | wool arrives and no resident is past 64 blocks (`True`) |
| CIVILIZATION-MIXED-LOG-HAUL-STARVATION | civilization-seed-qa | `mixed_wood_load_reaches_storage` (`civ_row_signals.py:311`) | only the founding oak species stored for a day while a funded House waits (`False`) | a second log species (the delivered mixed load) is stored (`True`) |
| CIVILIZATION-NATURAL-FOREST-SITE-PREPARATION | civilization-seed-qa | `natural_forest_site_is_felled_and_housing_grows` (`civ_row_signals.py:336`) | a pending House order and no house project for a day, logs flat (`False`) | logs rise and a house project has done > 0 (`True`) |
| CIVILIZATION-STREET-BUILDER-RETENTION | civilization-seed-qa | `street_completion_releases_the_builder` (`civ_row_signals.py:359`) | one day after the road completed, no non-street progress and no stock movement (`False`) | a non-street project's done rises after the completion (`True`) |
| CIVILIZATION-STREET-VERGE | settlement-map-qa `--scenario verge` | `verge_connected` = `verge_project_admitted_on_the_status_page` + `verge_obstacle_stands` (`settlement_map_qa.py:323`, published `:459`) | the funded Mine is not admitted on the status page while the wall stands (`False`) | both verge assertions true (`True`) |
| DATAPACK-NEW-PACK-WORLDGEN-SKIP | datapack-reload (`native-datapack-reload`) | `new_worldgen_pack_skipped_and_named` (`datapack_reload.py:368`, with `write_worldgen_pack:85`, `wait_log_lines:95`) | the restarted server's log holds only the worldgen start refusal, so the port never opens (`False`) | the restarted server's log says `Skipping new datapack file/newwg, it is not enabled: … gameplay cache` and the client rejoins (`True`) |
| ENTITY-RELATIVE-MOVEMENT-DRIFT | scene-shot (`native-entity-relative-trajectory`) | `relative_trajectory_returns_the_client_to_the_origin` (`scene_shot_checks.py:1801`, drift rule `:1767`) | 192 closed ±0.01 cycles with a flooring encoder leave the client 0.0469 blocks out (0.25 at the cargo oracle's 1024) (`False`) | the client's received y is within 0.02 of the server endpoint (`True`) |
| CIVILIZATION-GENERATED-DOOR-TREE | scene-shot (`native-village-exit-foliage`) | `door_headroom_lower_clear` + `door_headroom_upper_clear` (`village_exit_foliage_scene.json:71`, `:82`) | the generator places a solid block at (124,73,-276) outside the generated door (`door_headroom_upper_clear` false) | HEAD generates the same cell as `minecraft:air` |
| VILLAGER-HOUSE-EXIT-FOLIAGE | scene-shot (`native-village-exit-foliage`) | same two assertions (one fix `6d225aa5` for both rows) | same obstructed approach column, the row's "oak leaves at (124,73,-276)" | same clear headroom |

## How each was driven red and green

```
$ PYTHONPATH=. python3 …            # receipts/HarnessAsserts/signals-red-green.log
CIV-SEED24-CROSS-VILLAGE-TREE-SWEEP
  ref: harness:civilization-seed-qa:lumberjack_delivers_logs_within_a_day  graphical=True
  RED observation  -> False
  GREEN observation-> True
  ledger read (red): not met, value=False
  ledger read (green): verdict met
… (one block per row; every row prints False/True and the same two ledger reads)
```

The ledger read is the real gate, not a local helper: `regressions.is_graphical_ref(ref)` accepts
each ref (it runs `_profile_command` validation), and `regressions._profile_verdict` rejects a
`False` verdict with `VerdictNotMet(value=False)` and accepts `True` — that is what
`ledger._failing_verdict` turns into RED and what `promote` reads.

* `tools/harness/test_civ_row_signals.py` (12 tests) — one red/green case per signal, the
  registered-ref test for all eight names, and the unobserved-is-not-evidence rule.
* `tools/harness/test_datapack_worldgen_skip.py` (4 tests) — the pack bytes match the cargo
  oracle's trigger; a log with only the start refusal does not satisfy `wait_log_lines`, a log
  with the skip line does; the scenario's `started and named` expression is false unless both.
* `tools/harness/test_entity_relative_trajectory.py` (4 tests) — the row's 1024-cycle 0.25-block
  drift, the scene's 192-cycle 0.0469 drift above the 0.02 tolerance, and the fixed endpoint.
* `receipts/HarnessAsserts/verge-red-green.log` — the existing verdict's red/green through
  `regressions._profile_verdict`.
* `receipts/HarnessAsserts/worldgen-reversed-cargo.log` — the foliage rows' discriminator, proven
  at the generator: with only the worldgen hunk of
  `promotions/VILLAGER-HOUSE-EXIT-FOLIAGE.patch` reversed (and restored afterwards),
  `seed24_village_entrance_keeps_headroom_after_decoration` fails at
  `village_seed_tests.rs:240` — `left: BlockStateId(274)` (solid) at (124,73,-276) against the air
  HEAD generates. The scene's `door_headroom_upper_clear` reads that same cell from the client.

## Known limits of this pass (honest)

* None of the thirteen refs has been observed in a **full** scenario run on a candidate tree yet.
  Each row's `repro_check` says so. The civilization run is 7200 s on a pinned release server with
  a windowed native client; the datapack run and the two scene-shot scenarios need the same
  infrastructure. The controls that would flip them live are the cargo negative controls the rows
  already carry plus these assertions' first live receipts.
* The foliage rows' earlier demotion ("a freshly generated seed-24 world never overhangs this
  entrance") is contradicted by this generator control: the reversed worldgen hunk really does
  place a solid block at (124,73,-276). One client run pinned to a reversed tree is still owed to
  reconcile the two; this control proves the fixture, not the client execution.
* `civilization-seed-qa` runs on seed 24 by default, so its two door reads were dropped from the
  signals in favour of the direct scene-shot assertion; the run's `all` verdict is unchanged.
