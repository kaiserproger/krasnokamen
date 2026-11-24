# NATIVE-NAV-GROUNDED-EDGE-ORIGIN — hunk map for the committed reversal pair

Promotion receipt: `promotions/NATIVE-NAV-GROUNDED-EDGE-ORIGIN.json`.
Client pair: reversal `client:520f524b1b` → re-land `client:62de38d313` (the row's `fix_commit`).

## Why a reversal and not the pre-image pin

`895c63f3` fixes five rows at once, so its `non_test_patch` is the whole 38-file change and cannot
carry one row's control; the historical pre-image client is also unpinnable in the private graph
(wave 1). The committed reversal pair reverses only this row's planner hunk and re-lands it.

## Original commit

`client:895c63f3` (this row's part is the planner anchor hunk below plus the
`grounded_edge_pose_can_enter_a_supported_neighbor_column` test, which stays).

## Hunks

| file | hunk | content | disposition |
|---|---|---|---|
| `src/minecraft/navigation/planner.rs` | `@@ -207,16 +207,26 @@ impl Planner` | anchor the route in the center column first, then in every column the body footprint touches (`self.body(start)`, `'anchor` loop) instead of the center column only | REVERSED — the center-column-only loop is restored |
| `src/minecraft/navigation/planner.rs` | `@@ -441,6 +451,35 @@ mod tests` | the row's unit test + `unsupported_pose_cannot_anchor_across_a_gap` | KEPT (test hunks) |

## a/-side adaptation

None: the reversed lines are the pre-fix loop text from `895c63f3^`, the reverse patch applies with
intact context, and `git diff 520f524b1b 62de38d313` is exactly this patch
(patch-id `0912994e12200a52984a517ddf223541ce5ce952`).

## Line-by-line reconciliation with the origin hunk (review B-M3)

The patch-id above compares the pair with itself; this is the comparison with the historical hunk
that B1(b) of wave 2-1 asks for:

```
git show 895c63f3 -- src/minecraft/navigation/planner.rs | sed -n '/@@ -207,16/,/@@ -441,6/p'
git diff 520f524b1b 62de38d313 -U0 -- src/minecraft/navigation/planner.rs
```

Both sets of `+`/`-` lines (28 lines each) were extracted and sorted; `diff` between them is empty —
the pair's producer lines are **every** line of the historical hunk and nothing else. The two headers
differ only by the tip's line offset (`@@ -210 +210,8 @@`, `@@ -212,8 +219,11 @@` against
`@@ -207,16 +207,26 @@`), because three lines were added earlier in the file at the tip.

`-` lines (pre-image, restored by the reversal): `self.column(space, x, z, low, high)?;` ·
`let length = self.columns[&[x, z]].len();` · `for i in 0..length {` ·
`let node = self.columns[&[x, z]][i];` · `if (node.position()[1] - start[1]).abs() < 0.6` ·
`&& self.transition(space, start, node.position())?` · `{` · `root = Some(node);` · `break;`.

`+` lines (the fix): the comment pair · `let (min, max) = self.body(start);` · `min_x`/`max_x` ·
`let footprint = (min[2] …` · `.flat_map(…)` · `.filter(…)` · `'anchor: for [x, z] in std::iter::once([x, z]).chain(footprint) {` ·
`self.column(space, x, z, low, high)?;` · `let length = …` · `for i in 0..length {` · `let node = …` ·
`if …` · `&& self.transition(…)` · `{` · `root = Some(node);` · `break 'anchor;`.
```

## Proving ref

`harness:entities:native-v2-lease-navigation` → `edge_navigation_arrived`: with the client's center
over air and support only in the overlapping neighbour column, `minecraft_navigate_to_block` must
arrive at the target and the following observation must be grounded past it. The client unit ref
`minecraft::navigation::planner::tests::grounded_edge_pose_can_enter_a_supported_neighbor_column`
stays as the fast oracle.
