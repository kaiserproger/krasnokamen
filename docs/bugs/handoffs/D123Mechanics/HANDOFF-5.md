# D123Mechanics HANDOFF-5 — remaining delta review work (2026-11-24, continuation)

Branch `wt/D123Mechanics`; worktree `/run/media/kaiserroman/games/solaris-work/wt/D123Mechanics`.
Head after this handoff: `de12a6f7b` + the handoff commit. Code tree = `f818585a99` where fmt and
code-health PASS (`20261123T201731-fmt-73wbyd6t`, `20261123T201839-code-health-jujjbyu2`). 20 of 60
rows promoted; 8 new-handler rows are open pending the owner decision.

Read `BRIEF.md` and `AGENTS.md` first. Heavy commands only via
`/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/mem-run.sh -m 6G env RUSTC_WRAPPER= python3 -m tools.harness run …`,
one at a time, `set -o pipefail`; never full `correctness`; commit only on `wt/D123Mechanics` (append-only:
no amend, no rebase; merge-only integration by Main). Never promote a row whose control is a stub.

## 1. N5 (cheap, code+test) — take_finished fast path

Reviewer text: "in take_finished check `progress >= 1.0 && finish.is_some()` BEFORE the 25-lookup frame
scan; add the positive control 'finish delivered after the chunk returns'; player-chunk gate as the row
source promises or trim the source". Locate it:
`rg -n "take_finished|finish" crates/mc-core/src crates/mc-edge/src --type rust | rg -n "fn |progress"`
(the furnace reserve scan in `crates/mc-core/src/tick/containers.rs:509-540` is a near neighbour, not the
target). Hoist the shortcut, add the test in the same file's `_tests.rs`, then `harness run test -p mc-core
--lib <file>` and `cargo fmt --all`. Record the receipt in PROGRESS.md.

## 2. N4 (pick the cheaper honest option) — menu closes

Reviewer: "add tests for the new menu closes (crafter slot-state, enchant/lectern button on a cold menu
coordinate: MenuClosed, empty open_containers, cursor returned) or soften containers.rs:8 'every menu
input' and the rows' ingress_proof (place_recipe, select_bundle_item, beacon::configure not covered)."
Decision: soften the doc comment `crates/mc-core/src/redstone/containers.rs:8` and the rows' `ingress_proof`
to name exactly the covered handlers and the three uncovered ones; docs-only, no heavy run. Note it in
PROGRESS.md. If time allows afterwards, add the crafter test (cheapest of the three: slot-state toggle
needs no cold coordinate).

## 3. N3 — COPPER and ENDER-CHEST dry runs

`python3 -m tools.harness ledger promote COPPER --dry-run` and `… ENDER-CHEST --dry-run` (heavy, one at a
time, no --client). If the whole-commit reversal (fix_commit = c7bd44dd06 for COPPER, the Ender-Chest
commit) still fails the listed assertion at HEAD → leave as is, record the receipt. If it is green, does
not apply, or breaks a fixture → convert to scoped patch-mode following `HANDOFF-2.md` (temp edit, `git
diff -R`, header normalize, `git apply` restore) and add hunks.md in the SHELF form with the real commit
sha and the omitted hunks (a)/(b)/(c). Never amend; append a docs commit.

## 4. N2 — hunks.md rewrite (form of SHELF-HYDRATE-PANIC.hunks.md)

Needed for: the 8 gate-wave files (DAMAGE-COMMAND, DYE-WRONG, LIMESTONE, JIGSAW-STATE, SMITHING-TEMPLATE,
ANVIL hopper yields, TRANSPORT directions, REPLACE-FLOW / CONDITION-OPERAND — see
`docs/bugs/promotions/*.hunks.md` modified in commits 9ef63ddd9f..b460f313cc) and for ANVIL/MINING
(a) omission lists; PROTO proving ref = `crates/mc-protocol/src/frame_tests.rs:110`.

Rules from the reviewer: the "original commit" must be a real resolvable sha (`git cat-file -t`), taken
from the row's DIAGNOSIS / probe plan; the control is a HEAD re-break with no commit, so the file must say
"the control reversed X on HEAD" (the carried symbol plus its file:line on HEAD) and every non-test hunk of
the original fix is either carried (with the HEAD line proving it) or excluded with exactly one of (a)
another row's behaviour, (b) API the kept test calls, (c) rewritten by a later commit. Remove my one-line
form. `python3 -m tools.harness ledger promote <ID> --fix-patch … --dry-run` still re-audits.

## 5. New-handler rows — one bounded attempt each, then record the verdict

Rows: FALL-002, INTERACT-001, SPONGE, COMMAND-CHILD-SENDER-DROPPED, TOTEM, DEATH-PROTECTION-COMPONENT,
SPECIAL-001, DATA-QUERY. They stay open. For each, ONE attempt to find the honest pre-image, then write the
verdict into `receipts/D123/OWNER-DECISIONS-NEEDED.md` and copy it into the repo
(`docs/bugs/handoffs/OWNER-DECISIONS-NEEDED.md`):

- TOTEM/DEATH-PROTECTION: `git show de780c882^:crates/mc-core/src/tick.rs | rg -n "death|damage|rescue"` —
  what did the old death path do? If the pre-image is a `death` handler in tick.rs, reverse only its
  rescue call (kept API) and promote if the test fails at its assertion.
- SPECIAL-001: `git cat-file -t 35308579c; git show 35308579c^:crates/mc-core/src/special_storage.rs` — if
  the module is new, there is no pre-image: record that.
- DATA-QUERY: the real pre-image of `5c060d5df` (`git show 5c060d5df^:crates/mc-core/src/command_data.rs`
  and the same for `0c6f68d8`); build the HEAD patch from it, never a synthetic a/-side.
- FALL-002/INTERACT-001/SPONGE/COMMAND-CHILD-SENDER: `git cat-file -t` each hash in the row/hunks history;
  if the fix is a digest-only patch or a brand-new module, verdict = "no pre-image".

## 6. Remaining open rows by cheapness

Unpromoted earlier: the 15 cheap client-repro rows (no registered graphical ref) — Main's rule: define
real-client refs + manifest entries for the integrated sweep rather than per-row pins; DRAGON-EGG (needs
±15 chunk admission before the write; client gate red); FALL-001 (native probe red); PLACE-001 (spawn-radius
red); HAPPY-GHAST (sandbox env); AUDIT-SCULK (blame-based hunks); DATAPACK-HOT-RELOAD (HEAD-equivalent
control); LOADER-SOUND-MIXIN / DOC-CHISEL / SVC-BRIDGE / PISTON-QC (other repos / Main).

## 7. Close-out

`python3 -m tools.harness run fmt`, `… run clippy` (workspace), `… run code-health` on the committed tree;
commit docs; update `receipts/D123/D123Mechanics/review/INDEX.md` with the new commits only; message Main
with the delta verdicts. Final YAML: honest promoted/open counts, receipts per row, remaining gaps with
their blockers.
