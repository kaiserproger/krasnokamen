# TAGS-REFUSAL-MESSAGES: patch-mode fix identity

Original fixes: `c224df8de` (`fix(core): normalize scoreboard tags and explain summon refusals`) split the
merged `Tags:` pre-flight into a typed element check, a count check and a per-element Modified UTF-8 check,
and `0c51b3a44` (`fix: fence stale world frames and reject ambiguous tag records`) re-worded the count
refusal to `expected at most 1024 tags` and made `validate_tags` `pub(crate)`. The whole of either commit no
longer reverses on HEAD (they also carry TAGS-DUPLICATES' normalization and duplicate-key refusals and
other rows' world/data changes), so the row is a scoped patch:
`fix_commit = "patch:a1dae3762dffce388825c288c8bf9ecf9250f89df7af7882a0feffacbf21c55a"` in
`docs/bugs/promotions/TAGS-REFUSAL-MESSAGES.patch`. Its a/-side is the real pre-image:
`git log -S"expected at most 1024 strings" -- crates/mc-core/src/command_summon.rs` names
`3a79a7535`/`0c51b3a44` as the only history of that text, and the reversed tree reproduces the merged
count-or-type refusal. Re-audit:
`python3 -m tools.harness ledger promote TAGS-REFUSAL-MESSAGES --fix-patch docs/bugs/promotions/TAGS-REFUSAL-MESSAGES.patch --dry-run`.

## Carried

- `crates/mc-core/src/command_summon.rs @@ -100,13 +100,34 @@ fn validate_tags` — the split refusal
  (`every list element must be a string` before the count), the count wording
  (`expected at most 1024 tags`) and the per-element Modified UTF-8 limit
  (`tag at index {index} is {bytes} Modified UTF-8 bytes; maximum is 65535`). HEAD:
  `crates/mc-core/src/command_summon.rs:99-135`.

## Not carried

- `retained_tags`'s `crate::command_tag::normalized(tags)` (c224df8de, second hunk) and the three
  `crate::command_tag::validate_fields(fields)` calls (0c51b3a44) — (a) TAGS-DUPLICATES' set normalization
  and duplicate-key refusal; that row carries them in its own patch, and its own control reds on
  `duplicate_tags_keys_are_refused_by_summon_and_data_before_mutation`.
- `fn validate_tags` -> `pub(crate) fn validate_tags` (0c51b3a44) — (b) the visibility TAGS-DUPLICATES'
  `command_tag` module and this row's kept tests call; the reversed tree would not compile without it.
- Every other hunk of `c224df8de`/`0c51b3a44` (`command_summon_data.rs`, `command_data/*`, `command_tag.rs`,
  `world.rs`, `edge/*`, protocol) — (a) other rows' behaviour; see their own rows and controls.
- Test files of both commits — (b) kept so the control compiles and runs the registered ref.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:command_summon::tests::scoreboard_tags_tests::tag_refusals_name_element_type_count_and_modified_utf8_limit_before_spawn`
  — with the hunk reversed the `[1b]` case answers the merged `expected at most 1024 strings` instead of
  `every list element must be a string`, and the 70000-byte tag has no pre-flight at all, so the ref fails
  at its own `missing specific refusal {expected}` assertion
  (`crates/mc-core/src/command_summon_scoreboard_tags_tests.rs:122`, entity-count assertion at `:121`).
- `harness:entities:native-v2-ledger-tag-refusals` — the real client sends the player's own
  `/summon … {Tags:[1b]}` and reads `Invalid Tags: every list element must be a string` back on the mob and
  the experience-orb spawn paths, with no entity created, and a well-formed tag list is accepted without a
  refusal and its body exists (`tools/harness/backends/native_ledger_tags.py`). Passed receipt: see the
  promotion JSON. `tag <uuid> list` and `data get entity <selector>` answers were observed NOT to reach the
  client in this gate, so acceptance rests on the body, not a chat read-back.
  The client chat tool caps a message at 256 characters, so the 1025-element count list and the
  70000-byte string cannot be typed from a client: those classes rest on the Cargo ref only, and the
  scenario's `limits` says so.
