# SUMMON-ITEMS-HORSE-CARGO — fix-hunk map

Patch `patch:089fd1039796720221490f84e3ca93cca354d4aaf742567dab9c02c945136e86`
(`docs/bugs/fix-patches/SUMMON-ITEMS-HORSE-CARGO.patch`). The patch reverses
cleanly on this head (`git apply --reverse --check`, 2026-11-28); the earlier
re-promote's control pin `84fc5c955ca0-5fc5d61a` belonged to that tree and is
**not** reusable — this lane needs its own fresh pin at re-promotion.

## The fix

The real fix is `ee71ee684`; the patch is its forward port, because the commit's `prepare` block was
removed by the later refactor `9edc3c3ac` (behaviour-identical: `EntityStateImage::from_record`
already runs `mounts::persistence::from_fields` + `cargo_from_record`). At HEAD the admission is
exactly the field-set predicate:

| file | hunk | content |
|---|---|---|
| `crates/mc-core/src/command_summon.rs` | `@@ -378,8 +378,15 @@` (`supports`) | `"Items"` also admitted for vehicles/mounts via `mounts::persistence::owns_cargo_field(name, "Items")`, plus the `"ChestedHorse"` and `"Strength"` arms |

Omitted hunks:

| original hunk | reason |
|---|---|
| `ee71ee684` `command_summon.rs` `prepare` mount-cargo block | **(c)** a later commit (`9edc3c3ac`) removed the block; `from_record` does the same work, so there is no HEAD equivalent to carry |
| `ee71ee684` `command_summon_tests.rs` | a test hunk; `_split_non_test` drops it (the tests stay and are the row's cargo refs) |

## Control (measured on the 2026-11-27 tree)

- `cargo:mc-core:command_summon::tests::chested_horse_summon_carries_vanilla_items_cargo` red at
  `crates/mc-core/src/command_summon_tests.rs:26`.
- `harness:entities:native-v2-ledger-summon-horse-cargo` red at `summoned_cargo_mount_appears`
  (BASE's `973e6e60e` turned the fixture wait into `call_checked`), with the client still in play
  (`in_play: true`). The control scenario's own report carries
  `summon_feedback: ["Unknown or unsupported summon NBT field ChestedHorse", …]`, so the red is the
  server refusing the mount's field — the row's symptom — not a bare MCP timeout; the fixture
  command is recorded in `fixture_commands`.

Both refs are green on the restore; the candidate receipt's `summon_feedback` reads
`Summoned new Donkey` and the menu assertions (`cargo_menu_has_53_slots`,
`summoned_items_land_in_the_first_cargo_slot`, `summoned_items_land_in_the_last_cargo_column`,
`cargo_summon_does_not_fill_the_saddle_slot`) are true.

## New in this lane

`cargo:mc-core:command_summon::tests::summon_mule_accepts_items_without_a_chest_and_keeps_no_cargo`
(`crates/mc-core/src/command_summon_tests.rs:1698`) keeps the chest-free `mule`
`{Tame:1b,Items:[…]}` case out of the chested suite, so a tree that refuses `Items`
fails naming the field instead of stopping earlier at `ChestedHorse`. Its control
is the same patch: reverse it and this test's `summon` call panics with
`Unknown or unsupported summon NBT field Items` printed from `state.outbox`.
That control is **not yet run** on this head; it belongs to the re-promotion and
its `proving_assertions` entry is `crates/mc-core/src/command_summon_tests.rs:27`.
