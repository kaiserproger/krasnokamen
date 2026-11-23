# LOD-TILE-STAMPED-WITH-POST-SWITCH-DIMENSION-GENERATION — patch-mode fix identity (reworked)

Fix: `docs/bugs/fix-patches/LOD-TILE-STAMPED-WITH-POST-SWITCH-DIMENSION-GENERATION.patch`
(`fix_commit = "patch:ea201453aa5025dd6d73c236af85e0283bb1e540687300c126d4e60d6370c149"`).
It is a diff from the pre-fix form to this candidate; `git apply -R` restores
the pre-fix queue. Re-audit:
`python3 -m tools.harness ledger promote LOD-TILE-STAMPED-WITH-POST-SWITCH-DIMENSION-GENERATION --fix-patch docs/bugs/fix-patches/LOD-TILE-STAMPED-WITH-POST-SWITCH-DIMENSION-GENERATION.patch --dry-run`

## Why patch mode

The fix landed in `167dccf8d` (the generation stamp through the queue and the
LOD task) and `7920fc66b` (capture in `request()` and sample synchronously
under it, so the capture is production code rather than a test-only seam).
The kept test needs both halves, so the identity is the whole queue path as
this candidate has it.

## Commits carried and what each closes

| Commit | Closes | Hunks in the patch |
|---|---|---|
| `167dccf8d` | the row's path: a tile sampled before a dimension reset is dropped instead of delivered in the new dimension | `server/world_queue.rs`: `WorldSender::generation()`, `try_send_at(generation, frame)` and the frame's `generation` field from the argument (no longer the live counter at enqueue); `lod.rs`: the `enqueue_sampled`/`sample_tile` helpers |
| `7920fc66b` | the capture position (before the permit wait / before the sample) and the sampler seam | `lod.rs`: `request()` captures `handles.world_tx.generation()` before spawning, and the blocking sample runs inside `sample_tile`, which enqueues under the captured value |

No original hunk is left out.

## a/-side adaptation

The kept test — `a_reset_during_the_sample_discards_the_tile` — calls
`sample_tile(&sender, Some(generation), sampler)` and the `WorldSender`
accessors, so the pre-image cannot be restored verbatim:

- (b) `sample_tile`/`enqueue_sampled`/`WorldSender::generation()`/
  `try_send_at` keep their signatures (the test calls them; deleting them
  stops the control building).
- Their a/-side bodies are the pre-fix behaviour: `try_send_at` stamps the
  frame with `self.generation.load(Acquire)` at enqueue time (`let _ =
  generation`), and `sample_tile` ignores the captured value and passes the
  live generation after the sampler returns. That is the pre-fix
  `try_send(frame)` inside the task, expressed through the kept names.

## Proving ref (crates/mc-edge/src/lod.rs)

- `lod::tests::a_reset_during_the_sample_discards_the_tile` — the sampler
  calls `sender.reset()`; the a/-side then queues the stale tile under the new
  generation and the test fails at its own
  `Err(TryRecvError::Empty)` assertion (the frame is delivered) instead of the
  empty queue and released credit the candidate keeps. Moving the capture
  after the sample inside `sample_tile` reddens the same assertion, so the
  order is part of the identity and the control tests it.
