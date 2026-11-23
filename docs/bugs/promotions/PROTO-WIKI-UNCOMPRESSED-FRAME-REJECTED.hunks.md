# PROTO-WIKI-UNCOMPRESSED-FRAME-REJECTED: patch-mode fix identity

Original fix: `dd5662e81c` (`fix(protocol): accept an uncompressed frame at or above the threshold like
vanilla`); `git cat-file -t dd5662e81c` = commit. That commit carries both a product hunk in
`crates/mc-protocol/src/frame.rs` and the parity test hunks in `crates/mc-protocol/src/frame_tests.rs`.
`fix_commit = "patch:38f4b9936bf6543ca1…"` names the HEAD equivalents of the product hunks in
`docs/bugs/promotions/PROTO-WIKI-UNCOMPRESSED-FRAME-REJECTED.patch` (a diff from the pre-fix form to the
candidate; `git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote PROTO-WIKI-UNCOMPRESSED-FRAME-REJECTED --fix-patch docs/bugs/promotions/PROTO-WIKI-UNCOMPRESSED-FRAME-REJECTED.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `frame.rs @@ -131,9 +131,6 @@ pub enum FramingError` — drops the `UncompressedAboveThreshold` variant. HEAD
  `crates/mc-protocol/src/frame.rs:131` (`CompressedBelowThreshold` followed directly by
  `IncompleteCompressedStream`): carried verbatim.
- `frame.rs @@ -224,10 +221,13 @@ pub fn try_decode_frame_plan` — `data_length == 0` passes the body through at any
  size instead of rejecting `actual >= threshold`. HEAD `crates/mc-protocol/src/frame.rs:222-233` reads
  the varint id and returns `FrameDecodePlan::Ready` for the zero-length case (the comment citing
  `receipts/ProtocolWikiCheck/REPORT.md` is at HEAD `:225-230`): carried verbatim.

Proving ref: `cargo:mc-protocol:frame::tests::decoder_accepts_uncompressed_frame_at_or_above_threshold`.
Its own assertion is `crates/mc-protocol/src/frame_tests.rs:110` —
`.expect("vanilla accepts an uncompressed frame at or above the threshold")` in the body of
`fn decoder_accepts_uncompressed_frame_at_or_above_threshold` (`frame_tests.rs:96`); with the reverse
applied the decoder returns `Err(UncompressedAboveThreshold)` and that expect panics. Nothing else in the
control asserts.

## Original non-test hunks (complete)

```
## dd5662e81c non-test hunks (kept test files: crates/mc-protocol/src/frame_tests.rs)
- crates/mc-protocol/src/frame.rs: -131,9 +131,6 pub enum FramingError; -224,10 +221,13 pub fn try_decode_frame_plan(
```

## Not carried

- `frame_tests.rs @@ -93,23 +93,26 @@ fn compressed_threshold_edge` — (b) test hunk, kept so the
  control compiles and runs the ref. Its `round_trip` helper and the new
  `decoder_accepts_uncompressed_frame_at_or_above_threshold` are what the red runs.
- `docs/bugs/rows/PROTO-WIKI-UNCOMPRESSED-FRAME-REJECTED.toml` hunk — (b) ledger text, never reversed.

## Omitted hunks and reasons

None beyond the two above; every non-test hunk of the original fix is carried.
