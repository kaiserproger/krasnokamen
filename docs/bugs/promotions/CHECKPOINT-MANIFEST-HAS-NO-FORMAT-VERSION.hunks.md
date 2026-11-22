# CHECKPOINT-MANIFEST-HAS-NO-FORMAT-VERSION — combined negative control

[Control patch](../fix-patches/CHECKPOINT-MANIFEST-HAS-NO-FORMAT-VERSION.patch) for `7c1a96e92a6de2680bc8efe45bffec6e6d6adf86`.
`fix_commit = "patch:1ba1572cba9c78958a91d24e3cde0b78af6a82e961271712d4ffceb9c00c9afa"` names the canonical non-test patch.

## Original commits and exact mapping

- `9742adeba133c9dad0e9ae3bc3d28d6c1d65b33b` — `feat(persistence): require a manifest format version in the checkpoint`:
  root `manifest` entry in `domain_versions()` and the writer publishing it; the root reader refuses a manifest without it.
- `13deed939e452640a99fde9dc4c737a48020d5c6` — `fix(persistence): gate the manifest version before any other row decode`:
  the version is checked before any other row decode, both child images carry it, and maintenance writes it.

The canonical splitter takes the non-test hunks of the diff `9742adeba^1..13deed939` (the two commits are adjacent: `13deed939`'s parent is `9742adeba`). Reversing only `13deed939` leaves the root check and its tests green, which is why the control is the pair.

| Original commit | File | Original hunk | Combined hunk at candidate |
|---|---|---|---|
| `9742adeba` | `crates/mc-core/src/native_checkpoint.rs` | `-209,8 +209,13` | `-209,8 +209,16`: adjacent to the follow-up's hunk, one hunk after the range diff |
| `13deed939` | `crates/mc-core/src/native_checkpoint.rs` | `-209,13 +209,16` | `-209,8 +209,16`: second step of the same region |
| `13deed939` | `crates/mc-core/src/native_checkpoint.rs` | `-229,7 +232,9` | `-224,7 +232,9`: same hunk bytes, shifted by the merged region |
| `13deed939` | `crates/mc-world/src/native/checkpoint.rs` | `-23,6 +23,9` | `-23,6 +23,9` |
| `13deed939` | `crates/mc-world/src/native/checkpoint.rs` | `-82,6 +85,25` | `-82,6 +85,25` |
| `13deed939` | `crates/mc-world/src/native/checkpoint.rs` | `-265,6 +287,7` | `-265,6 +287,7` |
| `9742adeba` | `crates/mc-world/src/native/checkpoint.rs` | `-447,6 +447,7` | `-447,6 +470,7`: same hunk bytes, shifted by the hunks above it |
| `9742adeba` + `13deed939` | `crates/mc-world/src/native/checkpoint.rs` | `-467,6 +468,9` and `-468,9 +491,7` | `-467,6 +491,7`: the two adjacent hunks of one region, merged by the range diff |
| `13deed939` | `crates/mc-world/src/native/checkpoint.rs` | `-919,6 +940,7` | `-915,6 +940,7`: same hunk bytes, shifted by the hunks above it |
| `13deed939` | `crates/mc-world/src/native/maintenance.rs` | `-215,6 +215,9` | `-215,6 +215,9` |

No non-test hunk is omitted, no hunk is left out for another row's behaviour, and there is no pre-image adaptation: every combined hunk is the original hunk bytes at their candidate position. The canonical splitter leaves both commits' test files (`crates/mc-core/src/native_checkpoint_tests.rs`, `crates/mc-core/src/world_tests.rs`, `crates/mc-world/src/native/checkpoint_tests.rs`) and the docs (`docs/architecture/persistence.md`, `docs/MIGRATION.md`, `docs/bugs/ledger.toml`) out of the patch.

## Proving assertions

The four registered refs stay on the [row](../rows/CHECKPOINT-MANIFEST-HAS-NO-FORMAT-VERSION.toml):

- `cargo:mc-core:native_checkpoint::tests::manifest_format_version_is_required_in_root_and_children` — the root refusal: with the combined reversal there is no `manifest` entry in `domain_versions()` and the reader does not require one, so a manifest without the version is accepted and the assertion fails. `13deed939` alone cannot turn this ref red (the row's own note).
- `cargo:mc-core:native_checkpoint::tests::manifest_format_version_round_trips_through_the_writer`
- `cargo:mc-world:native::checkpoint_tests::native_checkpoint_requires_the_manifest_format_version`
- `cargo:mc-world:native::checkpoint_tests::native_checkpoint_round_trips_the_manifest_format_version`

## Verification performed while preparing

`git apply -R --check` of the stored patch on the clean candidate `7c1a96e92` exits 0. Reversing the patch restores the pre-`9742adeba` text of `crates/mc-world/src/native/maintenance.rs` byte-for-byte; in the other two files later commits changed regions outside these hunks, and those regions are untouched by the reversal. Preparation checked application and digest only; it is not a control or restore run, and later receipts do not substitute for the canonical ones.
