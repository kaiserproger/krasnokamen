# Hunk map — CLIENT-FALLING-BLOCK-PRESENTATION-FIELDS-DROPPED

Real fix: `client:13f1f28b8720eaf1a619fe35545b9210e05c1db8` ("fix(client): restore falling
prediction and authoritative creative routing", 2026-11-11, before the client rename `f0a6abf6`).
The stage0 merge-drift repair `54b79f73` deleted that block; the wave-2 re-land restores it in the
current file's shape.

Reversal route (`docs/bugs/README.md`, "Client rows whose real fix predates the client rename"): the
committed reversal `client:a8cf7f70a06939218704f202594c8901879dcbe9` removes the presentation
contract, and `client:6a5dbb67` re-lands it as the row's `fix_commit`. `git diff a8cf7f70 6a5dbb67`
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

HEAD equivalent (what the pair adds back, `6a5dbb67`):

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
- (b) **The `if` opener and its closing brace are not in the pair's diff.** They already exist at
  HEAD for the kept field of the sibling row CLIENT-FALLING-BLOCK-PREDICTION-REGRESSED
  (`predicted_velocity`), which the reversal must not remove. Removing the guard while keeping a
  statement inside it is not a patch. The three-line provenance comment is added by the re-land
  commit for the same reason (it documents why the block exists) and is removed by the reversal,
  so it is part of the ten-line pair.
- The `predicted_velocity` assignment between the two poses is deliberately untouched: it belongs
  to the other row and its own reversal (`client:f1168d66` → `client:bf54c241`).

## Omitted hunks of `13f1f28b`

- `src/entities.rs` (6 lines), `src/entities/physical.rs` (53), `src/entities/falling_tests.rs`
  (95), `src/inventory.rs` (50), `src/inventory/actions.rs` (2), `src/inventory/creative.rs` (8),
  `docs/AGENT_TESTING.md` (28): (c) other behaviour of that bulk commit (prediction integration,
  creative routing, its own fences), not the presentation contract this row names. Its
  `physical.rs` addition still exists at HEAD in evolved form; the row does not depend on it.
