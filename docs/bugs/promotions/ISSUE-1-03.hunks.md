# ISSUE-1-03 — hunk map

Row: `ISSUE-1-03` (client, major) — the F3 drawn-index count must stay bounded instead of growing
with camera direction. Refs:
`client:krasnokamen-client:render::shader_tests::submitted_index_count_is_per_frame_not_cumulative`,
`client:krasnokamen-client:render::frame::terrain_translucency_tests::frustum_rejection_precedes_instance_and_indirect_submission`,
`harness:scene-shot:issue1-03-drawn-index` (proving assertion `render_drawn_index_bounded`).

## Guard older than the finding

The line the pair carries is `src/render.rs`, in the per-frame render path:

```rust
self.stats = FrameStats::default();
```

It has been in the client since the 26.1.2 import: `git blame -C -C -L 1406,1406 67a2e0f5^ --
src/render.rs` names `4ccb68fb` ("feat: Solaris native Minecraft 26.1.2 client", 2026-10-19), and
`git log -S 'self.stats = FrameStats::default()' -- src/render.rs` lists no other origin. The checkout
that preceded the Issue1Client merge (`4e986e3d`, `src/render.rs:1365`) already has it. `4403f0b9`
("revert(ISSUE-1-03 control): drop the per-frame FrameStats reset") is an earlier **control
experiment** on a Stage2 branch (child of `34a85940`), not the commit that introduced the reset; the
previous version of this map was wrong about that (`REVIEW-wave2-9` finding 2).

So there is no fix commit for the reset: the code keeping the symptom out predates the row, and the
finding was a missing check. The row is promoted on the guard's committed reversal, and the control
goes red on the row's own count assertion:

- reversal `67a2e0f55be6c9f4a22c3199a573a8489e3ff5` — the line dropped; the unit counter test is red
  and the real-client scene accumulates.
- re-land `dad387e6893ef6049d1391025ce6232bea941040` — the line restored; row `fix_commit`.
- `git diff 67a2e0f5c5 dad387e689` is that one line.

## The original symptom: direction dependence is geometry, not a leak

The owner's comment on issue #1 reports a counter that rises sharply when looking **down** and a
frustum-test-order defect. The owner-host capture set behind that report
(`receipts/LaptopArchive/analysis/solaris-client/issue1/idx/SUMMARY.md`, 2026-10-19, baseline
`client-before` at `bb17d06`) measures it:

- standing still, 6 samples over 60 s: 473,220..473,580 (±0.04%), resident sections constant — no
  accumulation;
- orientation sweep at one spot (RD12): pitch −45° 45k..67k, horizon 245k..450k, pitch +45°
  0.87M..1.59M (up to 693 visible sections); the RD12/RD8 ratio equals the column-area ratio, so the
  direction dependence is visible **geometry**, not a leak;
- streaming after a long `/tp` peaks and plateaus (667k → 972k stable, resident 2527..2596);
- the only defect found in the submission path: `prepare_visibility` pushed instance slots and MDI
  candidates before the frustum test, so off-frustum sections took slots and candidates. That is the
  frustum-order change the owner named. It does **not** change `index_count`; it is fixed by
  `97e988d2` ("render: frustum rejection precedes instance/indirect-candidate submission") and
  proven by `render::frame::terrain_translucency_tests::frustum_rejection_precedes_instance_and_indirect_submission`
  (`src/render/frame.rs`, asserts one visible and one culled section, exactly one instance slot and at
  most one MDI candidate, no transparent sort entry).

So the row's pointer claim ("stays bounded rather than accumulating") is what the pair verifies, while
the direction-dependent count the owner saw stays inside per-frame bounds at every view, and the
frustum order has its own regression ref. `REVIEW-wave2-11` asked either for the horizon → down →
horizon check on fresh F3 frames tied to the frustum change, or for an owner-narrowing question; the
gate below does the check and the frustum ref is registered on the row.

## Gate

`scene-shot`, scenario `issue1-03-drawn-index` (`tools/harness/issue1_03_drawn_index_scene.json`): a
native client with `gamerule advance_time false` (the day clock is held; the tick rate is not frozen),
F3 shown, yaw 0 sampled twice, then 45, 90, 135, 180, −135, −90, −45 and a return to yaw 0 — **ten
samples over eight views, with two repeats** — then the horizon (pitch 0) sampled, the camera put
60° down and sampled, and the horizon sampled again.

`render_draw_sample` records `draw_calls`, `index_count`, `entity_submitted_indices`, the client's
`submitted_frame_id`, and the client's reported yaw **and pitch**: the sample must report the
requested yaw and pitch, and its frame counter must be newer than the previous sample's, so a frozen
renderer that keeps reporting one non-zero count cannot pass (`REVIEW-wave2-11` finding 3).
`assert_render_bounded` then judges every sample: `render_drawn_index_bounded` (all counts non-zero,
≤ 260000, and ≤ 1.5 × the first) and `render_repeat_view_stable` (a view repeated at the same yaw
**and pitch** may not grow by more than 1.5 × its earlier count). A down look is therefore its own
view, and returning to the horizon is judged against the first horizon sample, not against the much
smaller down count. F3 is captured at yaw 0, yaw 180, the return, and at the three horizon/down/
horizon samples.

Fixture correction (`REVIEW-wave2-11` minor): the client samples the natural surface at **Y = 76**,
which both F3 frames show (`XYZ: 1000.500 / 76.000 / …`); the y=159 stone fill runs while the
teleport is still settling, so the player is not standing on it at the sample point. This does not
feed the threshold: the bound is absolute (260000) plus growth, calibrated on this fixed seed, so the
pad's Y does not enter it.

Measured on the promoted pair: candidate receipt
`.analysis/validation/20261124T162443-issue1-03-drawn-index-jk4xbjg_` (client pin `dad387e689`, core
`f1e61cb286`) has ten samples, all yaw checks true, counts yaw-0 170676, yaw-0 repeat 175404, 45°
167340, 90° 178626, 135° 146208, 180° 163830, −135° 148980, −90° 162894, −45° 139560, return to yaw 0
175098 — every count non-zero, under 260000 and within 1.5 × the first, and both yaw-0 revisits within
1.03 × the first.

Control receipt `.analysis/validation/20261124T162521-issue1-03-drawn-index-pq3nxgxe` (client pin
`67a2e0f5`, same core tree and server binary): the reset dropped, the counts accumulate — yaw-0
5289168, yaw-0 repeat 5636880, 45° 6317196, 90° 7023264, 135° 7612080, 180° 8253762, −135° 9512952,
−90° 10154574, −45° 11021634, return to yaw 0 11690628 — and the only false assertion is
`render_drawn_index_bounded` (the aggregate check aborts there, so `render_repeat_view_stable` is not
reached). The pre-image therefore reproduces the reported symptom on the real client.

DOWN_LOOK_MEASUREMENTS_PLACEHOLDER
