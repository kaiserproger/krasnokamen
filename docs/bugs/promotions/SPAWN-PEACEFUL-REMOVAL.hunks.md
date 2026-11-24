# SPAWN-PEACEFUL-REMOVAL — the control is the guard at HEAD, not e2e05b33e

## Why there is no fix commit

The row is a **guard older than the finding** (`docs/bugs/README.md`, "Guard older than the
finding"). The reported live failure is from the 2026-10-24 DomMobs protocol-matrix run
(`scenarios/domains/spawning-bugs.md` §SPAWN-PEACEFUL-REMOVAL): after `difficulty peaceful`
the bot still reported zombies present for its 30 s action timeout.

The code that keeps that out of the current tree is older than the report:

- `crates/mc-core/src/natural_spawn.rs` `despawn` — `if peaceful { gone.push(slot); continue; }`,
  blamed to `1ada2ceb9` (2026-10-18, "feat: Solaris v0.0.8 — core v2 server, native storage,
  entity layout"), together with the `!peaceful` admission in the same pass and the
  `!peaceful_difficulty &&` condition of `no_work_before_category`.
- `e2e05b33e` (the candidate named in the row) changed *spawn eligibility* (`can_spawn`) and
  added the row's test; the despawn path predates it and `2dfebd7ab` restructured the candidate
  sweep around it.

So the row's `fix_commit` is `patch:9055c23a…`, the guarding hunk at HEAD, kept at
`docs/bugs/fix-patches/SPAWN-PEACEFUL-REMOVAL.patch`.

## The patch

One non-test hunk, `crates/mc-core/src/natural_spawn.rs`:

```
@@ -763,6 +763,10 @@ fn despawn(
         if !peaceful && (no_players || retained && row.no_action_time == 0) {
             continue;
         }
+        if peaceful {
+            gone.push(slot);
+            continue;
+        }
         if players.table().is_empty() {
```

It adds exactly the branch `1ada2ceb9` wrote; its reverse removes the branch, so a Peaceful
monster reaches the ordinary distance/idle despawn decision. With the reporting player inside
`no_despawn_distance` (which the row's own test arranges: `hostile_tests::setup(2.0)` and a
20-block spawn) the reversed tree keeps the zombie and the test fails at
`"hostile {id:?} survives peaceful"` — the row's own assertion, not a setup or compile failure.

The two complementary Peaceful guards are left in place and are not part of the patch:

- `no_work_before_category`'s `!peaceful_difficulty &&` — admits a *retained/persistent* Peaceful
  monster into the candidate list. Reversing it does not change this row's red (the test's
  zombie is not retained), and without the removal branch retained monsters would still survive,
  so it is not the row's proving hunk.
- the `!peaceful &&` on the `no_players || retained …` skip — keeps a Peaceful monster past the
  retained idle short-circuit. Same reasoning. (c): a later commit (`16a0e79371`) rewrote the
  `retained` expression this line reads, so a HEAD-equivalent reversal would have to rewrite the
  shape, not just drop a condition, for no extra evidence.

## Proving refs

- `cargo:mc-core:natural_spawn_tests::fix_mobs_peaceful_command_removes_hostiles_with_spawning_disabled`
  — added by `e2e05b33e`; red on the reversed tree at its first hostile assertion.
- `harness:entities:native-v2-peaceful-removal` — "after the peaceful command the same zombie
  disappears and the same cow remains visible" on a real native client (`entities` scenario,
  registered in `tools/harness/scenarios.py`).
