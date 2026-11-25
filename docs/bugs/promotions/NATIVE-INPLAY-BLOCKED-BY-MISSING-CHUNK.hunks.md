# NATIVE-INPLAY-BLOCKED-BY-MISSING-CHUNK — hunk map for the committed re-land pair

Row fix: `client:fdc19309818760c2086494acabf2ed9108702f3b` (commit **B**, the re-land).
Control: `client:30183eef7bd61858524ca29a7f53dcc89b39ef5b` (commit **A**, the pre-image).
`git diff A B` = `non_test_patch(B)`, patch-id `ff511a9d83b149fcd2ba255f29f8bf786397ab70`
(checked with `tools.harness.ledger.non_test_patch` + `_split_non_test` + `git patch-id --verbatim`).
Both commits are on `wt/D123ClientNativeF`; A is B's parent, so the control is an ancestor and the
delta rule of `docs/bugs/README.md` (§1, "the control may be a descendant with a committed reversal")
sees exactly the fix's production patch.

## Why a re-land pair and not a reversal of the fix commit

The row's real fix is two commits, and the second one rewrote the first's added line:

- `803f0e00` "release the loading screen when the level is live" replaced the camera-block and
  biome-neighbour demands with `self.world.chunks().next().is_some()`.
- `108c8b2a` "enter play once the level is live, whatever chunks arrived" replaced that line with
  `self.player.pose_known && self.dimension.is_some()`.

Measured 2026-11-25: `git apply -R --check` of `non_test_patch(803f0e00)` fails at
`src/session/observation.rs:465` — the added line is gone. Reversing only `108c8b2a` would restore
`self.world.chunks().next().is_some()`, which is still true in this scene (the player's own chunk
arrives), so that control would stay green. The faithful control is the single hunk both commits
made, expressed at this tip: A restores the exact body the pair removed.

## The real commits, hunk by hunk

| real commit | file | hunk (@@) | HEAD equivalent / reason |
|---|---|---|---|
| `803f0e00` | `src/session/observation.rs` | `@@ -465,21 +465,15 @@ impl Session` | the production hunk. Its a/-side is A's body (byte-identical to `git show 803f0e00^:src/session/observation.rs`, checked by extracting the function and diffing). Its b/+side (`self.world.chunks().next().is_some()`) was rewritten by `108c8b2a`, so at HEAD it is folded into the single A→B hunk below — omission reason (c), "a later commit rewrote that code". |
| `803f0e00` | `src/session/observation.rs` | `@@ -1807,6 +1801,35 @@ mod destruction_tests` | test hunk (`a_hole_under_the_camera_does_not_hold_the_play_loading_screen`), kept out of the control by `non_test_patch`; the test still lives at HEAD and passes on B. |
| `108c8b2a` | `src/session/observation.rs` | `@@ -472,18 +472,16 @@ impl Session` | the production hunk. a/-side = `803f0e00`'s body; b/+side = the current body. At HEAD it is the same A→B hunk below (the pre-image lines and the current lines). |
| `108c8b2a` | `src/session/observation.rs` | `@@ -1813,21 +1811,21 @@ mod destruction_tests` | test hunk (the regression's own text), kept out by `non_test_patch`. |
| `108c8b2a` | `src/session/observation.rs` | `@@ -1840,6 +1838,11 @@ mod destruction_tests` | test hunk (a second regression case), kept out by `non_test_patch`. |

## The HEAD hunk (the pair's whole production patch)

```
diff --git a/src/session/observation.rs b/src/session/observation.rs
@@ -503,24 +503,16 @@ impl Session {
```

A's side is the pre-`803f0e00` body, line for line:

```rust
    pub(crate) fn play_world_presentation_ready(&self) -> bool {
        if !self.player.pose_known || self.dimension.is_none() {
            return false;
        }
        let camera = self.player.eye_position();
        let block = camera.map(|coordinate| coordinate.floor() as i32);
        if self.world.block(block).is_none()
            || self.world.light_levels(block).iter().any(Option::is_none)
        {
            return false;
        }
        // The environment renderer samples biome columns around the camera.
        // Keep the loading screen up until this required neighborhood is known.
        let floor = camera.map(|coordinate| (coordinate * 0.25 - 0.5).floor() as i32);
        let x_range = ((floor[0] - 2) * 4).div_euclid(16)..=((floor[0] + 3) * 4).div_euclid(16);
        x_range.into_iter().all(|x| {
            (((floor[2] - 2) * 4).div_euclid(16)..=((floor[2] + 3) * 4).div_euclid(16))
                .all(|z| self.world.chunk([x, z]).is_some())
        })
    }
```

B's side is the body at this tip (`108c8b2a`'s text, unchanged):

```rust
    pub(crate) fn play_world_presentation_ready(&self) -> bool {
        // Vanilla enters play once the level exists and the player is placed; …
        self.player.pose_known && self.dimension.is_some()
    }
```

Provenance of the a/-side lines: `git blame -C -C 30183eef7b -- src/session/observation.rs` attributes
them to `803f0e00`(renamed history) / the pre-fix file; they are copied from
`git show 803f0e00^:src/session/observation.rs` verbatim — no invented line, no stub, no early return.
a/-side adaptation: none.

## Proving ref and control

- Ref: `harness:entities:native-v2-corrupt-chunk-near-camera`, proving assertion
  `restart_client_entered_play` (the row's symptom: the client never leaves the loading screen).
- The scene refuses the chunk column the pre-`803f0e00` gate demanded (chunk 11 west of the player's
  chunk 12), so A holds "Preparing terrain and light" and the bounded play wait (cap 120 s from
  `ProbeSteps.remaining`) returns `in_play` false → the assertion is recorded false, never a timeout.
- Restore: the same scene on B, green on `restart_client_entered_play` and on every other assertion.
- Evidence: `notes/h2b-gates.tsv` and the artifacts named in the row's `repro_check`.

## Note on the retracted server half

The earlier row text claimed the join payload never arrives and asked for a server-side fix
(chunk batch isolation on quarantine). That is retracted (`REVIEW-15-Opus-delta` D-M1): the edge plan
serves 80 of 81 chunks, and `in_play` / `restart_client_entered_play` are true on both the fixed and
the reversed client in all six receipts of 2026-11-25.
