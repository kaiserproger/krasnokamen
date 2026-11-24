# OP-DEOP-SELECTOR-LITERAL: patch-mode fix identity

Original fix: `d0ae64284` (`fix(edge): resolve operator and whitelist profile selectors`) — it replaced the
literal target lookup in the `/op`, `/deop` and `/whitelist` handlers with `profile_targets`, which expands
a selector through `hub.resolve_moderation_selector` and answers `No player was found` for an empty
selection, and it moved the whitelist to the same resolved profiles. The commit no longer reverses on HEAD
(the whitelist and moderation paths were rewritten by later fence commits), so the row is a scoped patch:
`fix_commit = "patch:b69e31e85a2d333c95235b2cef5bedc8a6489fb6fee306cea9f0632eddea0c0e"` in
`docs/bugs/fix-patches/OP-DEOP-SELECTOR-LITERAL.patch`. Its a/-side is the pre-image: before `d0ae64284`
the op/deop handler had no selector branch at all, so the reversed tree restores the literal
`admin.find(target)`/`offline_uuid("@s")` behaviour, exactly the reported symptom (`/deop @s` left the
player in `ops.json`). Re-audit:
`python3 -m tools.harness ledger promote OP-DEOP-SELECTOR-LITERAL --fix-patch docs/bugs/fix-patches/OP-DEOP-SELECTOR-LITERAL.patch --dry-run`.

## Carried

- `crates/mc-edge/src/server.rs @@ -972,6 +972,74 @@` — the `op | deop` resolved-profile branch
  (`profile_targets`, the reserved operator/permission inputs, the `Command {root} completed for {target}`
  answer). HEAD: `crates/mc-edge/src/server.rs:1033-1102`.
- `crates/mc-edge/src/server.rs @@ -1316,31 +1381,51 @@` — `profile_targets` itself
  (`resolve_moderation_selector`, the empty-selection `No player was found` answer, the literal fallback for
  a real profile name). HEAD: `crates/mc-edge/src/server.rs:1358-1400`.
- `crates/mc-edge/src/server.rs @@ -1279,11 +1295,60 @@` — the whitelist handed to the same resolved
  profile set (its own row's ref runs in this row's control and stays green; it is the same helper).

## Not carried

- `crates/mc-edge/src/server.rs @@ -1169,59 +1238,6 @@` — the deleted literal op/deop/whitelist block:
  that IS the a/-side of the carried hunks, so reversing the carried branch restores it in place; carrying
  a deletion of code the reversed branch needs would not compile.
- `crates/mc-edge/src/admin.rs` — the `Admin` profile lookup helpers (`find`, `profile`, `sessions_for_uuid`)
  — (b) the kept handler calls them; reversing them stops the control from compiling.
- `crates/mc-edge/src/server.rs @@ -895,7 +895,8 @@` and `crates/mc-edge/src/server.rs @@ -971,6 +972,74 @@`
  fence/queue plumbing that later commits rewrote — (c).
- `docs/COMPATIBILITY.md`, `admin_tests.rs` — (b) docs and the kept tests; the tests are the control's refs.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-edge:server::admin_tests::profile_selectors_op_deop_and_whitelist_use_native_players` — with
  the branch reversed `/deop @s` resolves to the literal offline `@s` profile, so the connected player
  keeps level 4; the ref fails at its own operator-level assertions.
- `cargo:mc-edge:server::admin_tests::profile_selectors_empty_targets_use_vanilla_error_without_fake_profiles`
  — the reversed handler reports a fake success instead of `No player was found`.
- `harness:entities:native-v2-ledger-op-selector` — the real client's own `/deop @s` answers completion and
  a following level-2 command leaves its received game mode survival
  (`tools/harness/backends/native_ledger_op_selector.py`). Passed receipt: see the promotion JSON.
