# ISSUE-1-03 — hunk map

Row: `ISSUE-1-03` (client, major) — the F3 drawn-index count must stay bounded instead of growing
with camera direction. Refs:
`client:krasnokamen-client:render::shader_tests::submitted_index_count_is_per_frame_not_cumulative`,
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

So there is no fix commit: the code keeping the symptom out predates the row, and the finding was a
missing check. The row is promoted on the guard's committed reversal, and the control goes red on the
row's own count assertion:

- reversal `67a2e0f55be6c9f4a22c3199a573a8489e3ff5` — the line dropped; the unit counter test is red
  and the real-client scene accumulates.
- re-land `dad387e6893ef6049d1391025ce6232bea941040` — the line restored; row `fix_commit`.
- `git diff 67a2e0f5c5 dad387e689` is that one line.

Without the reset, `stats.submitted_indices` (mirrored through `update_metrics` into
`data.render.index_count`, which F3 also renders) accumulates across frames, so a turn through the
yaws multiplies the reported count — the symptom the issue describes.

## The historical pre-image reproduces the symptom

The reversed tree *is* the pre-image at HEAD, and its real-client scene run fails on
`render_drawn_index_bounded` while the candidate passes (counts and receipt paths in the section
below). The owner comment on the issue describes a direction-dependent counter and a frustum-order
change; that direction dependence stays inside the per-frame bounds measured here (each view is a
fresh submission), while the reversal's cross-view growth is the accumulation this row verifies.

## Gate

`scene-shot`, scenario `issue1-03-drawn-index` (`tools/harness/issue1_03_drawn_index_scene.json`): a
native client on a stone pad with `gamerule advance_time false` (the day clock is held; the tick rate
is not frozen), F3 shown, yaw 0 sampled twice, then 45, 90, 135, 180, −135, −90, −45 and a return to
yaw 0 — nine samples over eight views, with two repeats (the second yaw-0 sample and the return).
`render_draw_sample` records `draw_calls`, `index_count` and `entity_submitted_indices` and checks the
client's reported yaw is the requested view. `assert_render_bounded` then judges every sample:
`render_drawn_index_bounded` (all counts non-zero, ≤ 260000, and ≤ 1.5 × the first) and
`render_repeat_view_stable` (a repeated or revisited view may not grow by more than 1.5 × its earlier
count). F3 is captured at yaw 0, yaw 180 and the return; the screenshots show the counter the
observation reads. The reversal fails `render_drawn_index_bounded` at the accumulated counts; the
candidate passes with per-view counts in the range below.

Measured on the promoted pair (see the promotion receipt for artifact paths): candidate
`render_samples` counts are bounded and the reversal's grow past 260000. Both receipts also record the
pinned client heads `dad387e689…` and `67a2e0f5…` and one server binary for the candidate core tree.

Candidate receipt `.analysis/validation/20261124T162443-issue1-03-drawn-index-jk4xbjg_` (client pin
`dad387e689`, core `f1e61cb286`): nine samples, all yaw checks true, counts yaw-0 170676, yaw-0 repeat
175404, 45° 167340, 90° 178626, 135° 146208, 180° 163830, −135° 148980, −90° 162894, −45° 139560,
return to yaw 0 175098 — every count non-zero, under 260000 and within 1.5 × the first, and both
yaw-0 revisits within 1.03 × the first, so `render_drawn_index_bounded` and
`render_repeat_view_stable` pass with F3 shown. The control receipt is named in the promotion JSON.

Control receipt `.analysis/validation/20261124T162521-issue1-03-drawn-index-pq3nxgxe` (client pin
`67a2e0f5`, same core tree and server binary): the reset dropped, the counts accumulate — yaw-0
5289168, yaw-0 repeat 5636880, 45° 6317196, 90° 7023264, 135° 7612080, 180° 8253762, −135° 9512952,
−90° 10154574, −45° 11021634, return to yaw 0 11690628 — and the only false assertion is
`render_drawn_index_bounded` (the aggregate check aborts there, so `render_repeat_view_stable` is not
reached). The pre-image therefore reproduces the reported symptom on the real client, which is what
`REVIEW-wave2-9` finding 2 asked to prove.
