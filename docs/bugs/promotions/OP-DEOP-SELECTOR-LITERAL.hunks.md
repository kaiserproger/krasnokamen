# OP-DEOP-SELECTOR-LITERAL: patch-mode fix identity

Original fix: `d0ae64284` (`fix(edge): resolve operator and whitelist profile selectors`) — it replaced the
literal target lookup in the `/op` and `/deop` handlers with `profile_targets`, which expands a selector
through `hub.resolve_moderation_selector` and answers `No player was found` for an empty selection. The commit
no longer reverses on HEAD (the whitelist and moderation paths were rewritten by later fence commits), so the
row is a scoped patch:
`fix_commit = "patch:b69e31e85a2d333c95235b2cef5bedc8a6489fb6fee306cea9f0632eddea0c0e"` in
`docs/bugs/promotions/OP-DEOP-SELECTOR-LITERAL.patch`. Its a/-side is the pre-image: before `d0ae64284` the
op/deop handler had no selector branch, and the patch's deletion hunk is that literal block itself. Re-audit:
`python3 -m tools.harness ledger promote OP-DEOP-SELECTOR-LITERAL --fix-patch docs/bugs/promotions/OP-DEOP-SELECTOR-LITERAL.patch --dry-run`.

## Carried (the patch is exactly these two hunks)

- `crates/mc-edge/src/server.rs @@ -972,6 +972,74 @@` — the `op | deop` resolved-profile branch: it calls
  `profile_targets`, reserves the operator inputs, answers `{root} completed for {target}`, and keeps the
  session lock/permission fence. HEAD: `crates/mc-edge/src/server.rs:1033-1102`.
- `crates/mc-edge/src/server.rs @@ -1170,59 +1238,6 @@` — the deletion of the literal op/deop block inside
  the earlier permission-fence body (53 removed lines). Reversing the patch restores exactly that block, so
  `/deop @s` again resolves the literal `@s` profile and leaves the connected operator at level 4 — the row's
  symptom.

## Not carried

- `profile_targets` itself — the original commit's `crates/mc-edge/src/server.rs @@ -1316,31 +1381,51 @@` —
  (b) the kept whitelist path calls it, so reversing it stops the control from compiling; it stays in both
  trees and is not the row's behaviour.
- The whitelist handed to the same resolved profile set — the original commit's
  `crates/mc-edge/src/server.rs @@ -1279,11 +1295,60 @@` — (a) the whitelist's own behaviour; its ref runs in
  this row's control and stays green.
- `crates/mc-edge/src/admin.rs` — the `Admin` profile lookup helpers (`find`, `profile`, `sessions_for_uuid`)
  — (b) the kept handler calls them.
- `crates/mc-edge/src/server.rs @@ -895,7 +895,8 @@` — the connection/moderation fence plumbing that later
  commits rewrote — (c).
- `docs/COMPATIBILITY.md`, `admin_tests.rs` — (b) docs and the kept tests; the tests are the control's refs.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-edge:server::admin_tests::profile_selectors_op_deop_and_whitelist_use_native_players` — with
  the branch reversed `/deop @s` resolves to the literal offline `@s` profile, so the connected player
  keeps level 4; the ref fails at its own operator-level assertions
  (`admin_tests.rs:1382`, `:1501`, `:1544` per the promotion receipt).
- `cargo:mc-edge:server::admin_tests::profile_selectors_empty_targets_use_vanilla_error_without_fake_profiles`
  — the reversed handler reports a fake success instead of `No player was found` (green in the control, as the
  receipt records: the empty-selection answer came with the kept `profile_targets`).
- `harness:entities:native-v2-ledger-op-selector` — the real client's own `/deop @s` answers
  `Command deop completed for @s` and a following `/gamemode creative` is refused
  (`Unknown or incomplete command`), while the same command answers before the deop
  (`tools/harness/backends/native_ledger_op_selector.py`). Passed receipt: see the promotion JSON.
