# HARNESS-ROUNDTRIP-FORCES-X — patch mode, one commit, forward-ported

`fix_commit = patch:54d1444c9d7a0ffdb5869a44ee2c433329d15be38f35e4d0cc58bd8fa58c6d2a`
(`docs/bugs/fix-patches/HARNESS-ROUNDTRIP-FORCES-X.patch`, sha256 `54d1444c9d7a0ffdb5869a44ee2c433329d15be38f35e4d0cc58bd8fa58c6d2a`).

## The row's path

The correctness roundtrip passed the runner's exclusive measurement lane (`-x`) to every launch, so a
long correctness run waited for a quiet window it did not need. The fix removes that token.

## Original commit and its HEAD equivalent

| commit | original hunk | HEAD equivalent | part of the row's path |
|---|---|---|---|
| `f645ca39a` | `tools/harness/vanilla_world_roundtrip.py` `v2()`: `command = [str(runner), "-x", "-m", "8G", ...]` → `command = [str(runner), "-m", "8G", ...]` | `limited()` (line 128-131): `return [str(MEM_RUN), "-m", memory] if MEM_RUN.exists() else []` — the same behaviour (no `-x`) after a later refactor folded the two launch helpers into `limited()` | yes — this is the whole fix in its HEAD form |

The original hunk's lines no longer exist (a later commit replaced `runner`/`v2()`'s inline command
with `limited()`), so the patch carries the HEAD equivalent, exactly as patch mode requires: `hunks.md`
names the original hunk and the line the forward port stands on.

## Original hunks left out

`f645ca39a` also changed `tools/harness/test_vanilla_world_roundtrip.py` (RoundtripLaneTests). Test
files are never part of a fix patch (a control reverses production behaviour and the oracle stays as
the current tree has it), so it is not listed as an omission with a reason: it is not eligible.

## Proving ref and line

`python:tools.harness.test_vanilla_world_roundtrip:RoundtripLaneTests.test_commands_use_normal_lane_of_the_canonical_runner`
— `self.assertNotIn("-x", command)` at `tools/harness/test_vanilla_world_roundtrip.py:45` (and the
`java` lane assertion at :43). With the patch reversed the command carries `-x` again and this ref is
red with one failure; on the tree being promoted it is green.
