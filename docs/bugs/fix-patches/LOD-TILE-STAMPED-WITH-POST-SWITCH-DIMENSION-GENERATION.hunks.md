# LOD-TILE-STAMPED-WITH-POST-SWITCH-DIMENSION-GENERATION — patch-mode fix identity

Fix: `docs/bugs/fix-patches/LOD-TILE-STAMPED-WITH-POST-SWITCH-DIMENSION-GENERATION.patch`
(`fix_commit = "patch:c8c4f61d372c4e7bf73db864ba25c54d0ccb78432221bedb300d5669920f6101"`).
It is a diff from the pre-fix form to this candidate; `git apply -R` restores
the pre-fix code. Re-audit:
`python3 -m tools.harness ledger promote LOD-TILE-STAMPED-WITH-POST-SWITCH-DIMENSION-GENERATION --fix-patch docs/bugs/fix-patches/LOD-TILE-STAMPED-WITH-POST-SWITCH-DIMENSION-GENERATION.patch --dry-run`

## Original fix

`167dccf8d` "fix(edge): fence LOD tiles with the generation captured before
sampling": `WorldSender::generation()` and `try_send_at()` in
`crates/mc-edge/src/server/world_queue.rs`, and in `crates/mc-edge/src/lod.rs`
the generation is captured after the worker permit and before
`spawn_blocking`, then the tile is enqueued through `enqueue_sampled` under that
generation.

## Hunk mapping

| Patch hunk | Original | What it is |
|---|---|---|
| `lod.rs` `enqueue_sampled` | 167dccf8d | the sampled tile is stamped with the generation captured before the sample, so a reliable dimension reset during the sample makes the writer discard it |

## Exclusions

- `world_queue.rs` `generation()`/`try_send_at()`: (b) the kept test
  `lod::tests::tile_sampled_before_a_dimension_reset_is_dropped` calls both, so
  reversing them stops the control from compiling. They stay on both sides.
- The `lod.rs` generation capture and the call site stay on both sides: (b) the
  same reason; only the helper's stamping behaviour is reverted.

## a/-side adaptation

The pre-fix `enqueue_sampled` ignores its `sampled_at` argument and calls
`try_send`, which stamps the frame at enqueue time — the original bug
(`reset()` between capture and send no longer fences the tile).

## Proving ref

`cargo:mc-edge:lod::tests::tile_sampled_before_a_dimension_reset_is_dropped`:
with the pre-fix helper the stale tile is delivered after `reset()` and the
test fails at its `Err(TryRecvError::Empty)` assertion; on the candidate the
writer discards it and releases its queued bytes.
