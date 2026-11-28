# returned — no acceptable control shape

The behaviour is real and gate-proven: the row's real-client ref
`harness:entities:native-v2-ledger-datapack-hover` passed at
`.analysis/validation/20261125T044847-native-v2-ledger-datapack-hover-7_bn3a53`
(description line `The default data for Minecraft`), and the scoped cargo ref
`datapack_commands_tests::list_entries_are_chat_links_with_title_and_description_hover`
is green at `bbf5b71e5` (106/106 datapack tests, this worktree).

What no control can show, re-checked against the row's own assertion
`datapack_commands_tests.rs:206` (`assert_eq!(key(alpha), "chat.square_brackets")`):

- The real pre-image (`ba2b3a98a^:crates/mc-core/src/datapacks.rs:549-551`) is one
  plain `feedback("Enabled directory datapacks: {names}")` line. Reversing the
  emission at HEAD to plain names makes the test helper `listed()` find no
  `translate` entries, so `enabled[1]` panics before line 206 — a crash, which
  `ledger promote` refuses as a control.
- Reversing only the hover (`hover_event` removed) keeps line 206 green and the
  next assertion panics on the missing field; it also is not the pre-image.
- The enabled/available group split and the translated `commands.datapack.list.*`
  keys come from `32069cbb9`, a later listing rewrite; removing them to reach the
  plain line would reverse another row's/feature's hunks, which the
  mechanical-adaptation rule refuses.

This matches the recorded 2026-11-25 decision (`repro_check`). No new commit has
changed `datapack_management.rs`'s listing emission since. Proof-shape decision
for Main (POL-2 family).
