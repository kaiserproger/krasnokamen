# Hunk map — CLIENT-FALLING-BLOCK-PRESENTATION-FIELDS-DROPPED

Real fix: `client:13f1f28b8720eaf1a619fe35545b9210e05c1db8` ("fix(client): restore falling
prediction and authoritative creative routing", 2026-11-11, before the client rename `f0a6abf6`).
The stage0 merge-drift repair `54b79f73` deleted that block; the wave-2 re-land restores it in the
current file's shape.

Reversal route (`docs/bugs/README.md`, "Client rows whose real fix predates the client rename"): the
committed reversal `client:f8c6eef376cf7258d23c6af202ec0f8bad14a2fa` removes the presentation
contract, and `client:89f2223e` re-lands it as the row's `fix_commit`. `git diff f8c6eef3 89f2223e`
is the ten lines below, byte for byte.

## Hunk 1 — `src/minecraft/entities.rs`, `@@ -252,6 +252,12 @@` in `13f1f28b`

`fn list_entity_value`, guarded by `entity.type_name == "minecraft:falling_block"`:

```
+    if entity.type_name == "minecraft:falling_block" {
+        value["predicted_position"] = json!(entity.transform.position);
+        value["presented_position"] = json!(entity.presented_transform(
+            session.entities.frame_tick, session.entities.frame_partial).position);
+        value["presentation_tick"] = json!(session.world_tick);
+    }
```

HEAD equivalent (what the pair adds back, `89f2223e`):

```
+    // The falling-block automation contract (restored from 13f1f28b, which the stage0 merge-drift
+    // repair 54b79f73 dropped): the sand gate reads both poses and the presentation tick to prove
+    // that a frozen block is drawn at its previous predicted pose while the client steps ticks.
     if entity.type_name == "minecraft:falling_block" {
+        value["predicted_position"] = json!(entity.transform.position);
+        value["presented_position"] = json!(
+            entity
+                .presented_transform(session.entities.frame_tick, session.entities.frame_partial)
+                .position
+        );
         value["predicted_velocity"] = json!(entity
             .physical
             .falling_velocity
             .unwrap_or(entity.velocity));
+        value["presentation_tick"] = json!(session.world_tick);
     }
```

Adaptations, per the (a)/(b)/(c) classes:

- (a) **`presented_position` formatting.** The real hunk put `presented_transform(...)` on two
  lines inside `json!`; HEAD's rustfmt shape is the five-line form above. Same expression, same
  value, same call site. The pair's diff is the HEAD form.
- (a) **The `if` opener and its closing brace are not in the pair's diff: a surviving half after
  the (a) split.** They already exist at HEAD for the kept field of the sibling row
  CLIENT-FALLING-BLOCK-PREDICTION-REGRESSED (`predicted_velocity`), which the reversal must not
  remove; removing the guard while keeping a statement inside it is not a patch, and reversing the
  guard would red that row's own assertion. The three-line provenance comment is added by the
  re-land commit for the same reason (it documents why the block exists) and is removed by the
  reversal, so it is part of the ten-line pair.
- The `predicted_velocity` assignment between the two poses is deliberately untouched: it belongs
  to the other row and its own reversal (`client:593f7957` → `client:e541641c`).

## Omitted hunks of `13f1f28b`

Test and documentation hunks never enter a reversal patch (the split rule keeps test files,
`docs/` and inline `#[cfg(test)]` modules), so they carry no class:

- `src/entities/falling_tests.rs` `@@ -0,0 +1,95 @@` — the commit's own fences; the reversal's
  harness, not the contract.
- `docs/AGENT_TESTING.md` `@@ -1532,3 +1532,31 @@` — prose.

Every remaining hunk is **(a) other behaviour of that bulk commit** (prediction integration,
creative routing and their own fences); reversing any of them would turn a control red on another
row's assertion, and none is part of this row's presentation contract. HEAD already carries evolved
forms of the `physical.rs` and `entities.rs` pieces, so they have a HEAD equivalent and are not (c):

- `src/entities.rs` `@@ -17,6 +17,8 @@`, `@@ -267,6 +269,8 @@`, `@@ -988,6 +992,8 @@` — `mod local;`,
  the `Store` dirty-tracking fields and their accessors: prediction integration state.
- `src/entities/physical.rs` `@@ -762,6 +762,9 @@`, `@@ -778,6 +781,7 @@`, `@@ -785,6 +789,55 @@` —
  the integrator and the no-gravity move; `client:f7483f34` rewrote the last one (see the sibling
  map's omitted hunk), the first two still exist in evolved form.
- `src/inventory.rs` `@@ -449,7 +449,6 @@`, `@@ -488,7 +487,6 @@`, `@@ -511,8 +509,9 @@`,
  `@@ -1271,4 +1270,47 @@` — creative inventory routing and its fences.
- `src/inventory/actions.rs` `@@ -700,7 +700,7 @@` — creative action routing.
- `src/inventory/creative.rs` `@@ -2,7 +2,7 @@`, `@@ -14,11 +14,7 @@` — the authoritative
  creative packet path that gives the commit its title.
