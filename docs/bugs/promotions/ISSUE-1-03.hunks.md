# ISSUE-1-03 — hunk map

Row: `ISSUE-1-03` (client, major) — the F3 drawn-index count must stay bounded instead of growing
with camera direction. Refs:
`client:krasnokamen-client:render::shader_tests::submitted_index_count_is_per_frame_not_cumulative`,
`harness:scene-shot:issue1-03-drawn-index` (`render_drawn_index_bounded`).

## The hunk

`src/render.rs`, in the per-frame `render` path, one line:

```rust
self.stats = FrameStats::default();
```

Without it `stats.submitted_indices` (mirrored through `update_metrics` into `data.render.index_count`
for both the app-published observation and `minecraft_observe`) accumulates across frames.

The historical control `4403f0b911e5` ("revert(ISSUE-1-03 control): drop the per-frame FrameStats
reset") is exactly this one-line drop, child of `34a85940`; it predates the 26.3 cutover and cannot
join as a build, so the pre-image is rebuilt as a committed reversal at HEAD:

- reversal `67a2e0f5c55be6c9f4a22c3199a573a8489e3ff5` — the line dropped (unit ref red; scene red);
- re-land `dad387e6893ef6049d1391025ce6232bea941040` — the line restored; row `fix_commit`.
- `git diff 67a2e0f5c5 dad387e689` is that one line.

## Gate

`scene-shot` scenario `issue1-03-drawn-index`, eight yaws 45° apart on a frozen-tick stone pad, `low`
preset: `render_drawn_index_bounded` at each yaw with `max_index_count = 260000`,
`max_growth = 1.5` (calibrated: fixed samples 138534..177288, first 175068). Measured: fix PASS,
reversal FAIL at the first sample (7,280,424 indices). Counts are in `render_samples` in the receipt.
