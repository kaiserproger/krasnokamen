# Hunk map — CLIENT-FALLING-BLOCK-PREDICTION-REGRESSED

Real fix: `client:d03374260edf8d738f23daf8dd148beecd8e0873`. After review
`receipts/D123/D123Harness/review/REVIEW-wave2-8.md` (M1) this row is narrowed to the gap that
commit really closes: the automation contract had no `predicted_velocity`, so no consumer could
model the prediction the client runs between position packets.

Reversal route: the committed reversal `client:f1168d66f87a22c36ca200b16a9f3f538196a9a2` removes
the field and `client:bf54c2417480ebf56dfbf228d6f3fcbd1b38129a` re-lands it as the row's
`fix_commit`. `git diff f1168d66 bf54c241` is the four lines below, and the reversed blob is
`ffe8a4c1`, the exact pre-image `d0337426` started from.

## Hunk 1 — `src/minecraft/entities.rs`, `@@ -265,6 +265,10 @@` in `d0337426`

`fn list_entity_value`, inside the existing `minecraft:falling_block` guard:

```
+        value["predicted_velocity"] = json!(entity
+            .physical
+            .falling_velocity
+            .unwrap_or(entity.velocity));
```

No adaptation: the pair's diff is the hunk verbatim (the guard and the presentation fields it sits
between belong to CLIENT-FALLING-BLOCK-PRESENTATION-FIELDS-DROPPED and stay in both commits).

## Omitted hunk of `d0337426`: `src/entities/physical.rs`, `@@ -1057,8 +1057,15 @@`

(c) **Not part of this fix.** That hunk made a `DATA_NO_GRAVITY` falling block move on the received
velocity with no drag. The server (`crates/mc-core/src/falling.rs`: the gravity term is zeroed, the
`* 0.98` drag is applied on every branch) does keep integrating, and the stale-velocity cause it
described cannot occur on the packet path (`SET_ENTITY_MOTION` and `move_to` with a velocity both
already cleared `falling_velocity`). `client:a5769870` removes that hunk and restores
`falling_velocity.unwrap_or(velocity)` for every block, with the 0.98 drag pinned by
`falling_block_no_gravity_keeps_air_drag_without_gravity`, and the freeze fence now feeds its stop
through `SET_ENTITY_MOTION` instead of writing `entity.velocity` directly.

## The scene half

`harness:entities:native-v2-input-sand` proves the field through
`falling_block_predicted_velocity_field` (`tools/harness/backends/native_player_input.py`): the
prediction model reads `predicted_velocity` alone, so a client without it fails that named
assertion before the model runs. `sand_predicts_between_received_updates` stays the row's second
oracle on the candidate.
