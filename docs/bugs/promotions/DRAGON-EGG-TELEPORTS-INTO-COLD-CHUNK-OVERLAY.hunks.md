# DRAGON-EGG-TELEPORTS-INTO-COLD-CHUNK-OVERLAY: patch-mode fix identity

Original fix: `c7bd44dd06` (`fix(residency): keep cold menu, mining, brush, copper and egg writes off the
overlay`) — its `vanilla_interact.rs -125,8 +125,11` hunk only ADDS the `has_chunk` term in front of the
air check that the pre-fix code already had:

```
         if !state
             .world
-            .block_at(target.x, target.y, target.z)
-            .is_some_and(|id| id == state.registry.states.air)
+            .has_chunk(target.x.div_euclid(16), target.z.div_euclid(16))
+            || !state
+                .world
+                .block_at(target.x, target.y, target.z)
+                .is_some_and(|id| id == state.registry.states.air)
             || !crate::plugin_player::allows_block(state, actor, target)
         {
             continue;
```

(Rewritten after review 6 B1: the earlier patch's a/-side deleted the vanilla air check and its
`@@ -123,7 +123,15 @@` header was the hand-built patch's own header, not an original hunk. Both are gone.)

`fix_commit = "patch:fe7e45fa5d7e03c8…"` names this hunk at HEAD
(`crates/mc-core/src/vanilla_interact.rs:125-137`, context identical, so `git apply -R` applies
verbatim — checked). Re-audit:
`python3 -m tools.harness ledger promote DRAGON-EGG-TELEPORTS-INTO-COLD-CHUNK-OVERLAY --fix-patch docs/bugs/promotions/DRAGON-EGG-TELEPORTS-INTO-COLD-CHUNK-OVERLAY.patch --dry-run`.

## Carried

- `vanilla_interact.rs -125,8 +125,11` — the `has_chunk` term exactly as `c7bd44dd06` writes it. HEAD
  `:125-137`. The a/-side is the real pre-image (air check + `allows_block` intact, only `has_chunk`
  absent); there is **no** a/-side adaptation and no invented code.

## Commit roles

The `has_chunk` term's chain on this path is `c7bd44dd06` (adds the guard) -> `608f10d002` (reverts it) ->
`b460f313cc` (re-adds it); `git blame -C -C` on the surviving lines names `b460f313cc` as the origin of
the hunk now at HEAD, and `c7bd44dd06` is where the row's symptom was first closed. This row's carried
hunk is the one at HEAD (`vanilla_interact.rs:125-137`); its pre-image is HEAD minus the `has_chunk` term,
which is exactly the state `608f10d002` left. `b460f313cc`'s other hunks (`inventory.rs`, `stations.rs`,
`vanilla_interact.rs` outside this term) belong to the other residency rows listed under "Not carried",
matching the MINING patch's model (`MINING-...hunks.md` "Commit roles").

## Not carried

- The rest of `c7bd44dd06` (cold menu, mining, brush and copper hunks, `docs/bugs/rows/*`): (a) other rows'
  behaviour, each carried by its own control (`review/c7bd44dd0.patch` lists them).
- Test files: (b) kept.

## Proving refs

- `cargo:mc-core:interact_parity_tests::interact_parity_dragon_egg_never_teleports_into_a_cold_overlay`
  (`crates/mc-core/src/interact_parity_tests.rs:671-675`): the fixture leaves air only in cold chunks
  (`:638-655`), so without `has_chunk` the egg leaves the sampled pad into the cold overlay and the
  assertion reds; with the term it stays. This is the row's own loss assertion.
- `harness:entities:native-v2-ledger-dragon-egg` — **not evidence** for this row. The scene runs at view
  distance 4, where the whole sampled frame is resident, so it cannot observe the cold overlay; it proves
  only that the ordinary parity path still teleports the egg. The ledger still requires a passed receipt
  for the row's registered real-client ref, and this re-promote carries a fresh one on the candidate tree.


## Regenerated 2026-11-25 (REVIEW-wave2-13 m1)

`70d39713e` moved the residency test into its own branch ahead of the air check (a cold sample is
recorded before it can be judged), so this patch's old b-side (`has_chunk(..) ||` inside the air
condition) no longer matched HEAD. The patch is regenerated from the current `dragon_egg`: the
reverse removes the cold-target branch, which is exactly the pre-fix behaviour (the air check sees
`None` for a non-resident cell and the sample is skipped, never written into the overlay).
`git apply -R --check` passes on HEAD; sha256 `87e8dbd32fafc93b…` (full digest in the promotion JSON).
The citations above that name `interact_parity_tests.rs` lines are historical; the cold-overlay test
is `interact_parity_dragon_egg_never_teleports_into_a_cold_overlay` (search by name).
