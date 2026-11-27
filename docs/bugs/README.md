# Bug regression ledger

The ledger is one file per row: `docs/bugs/rows/<ID>.toml`, one `[[bug]]` table
whose file name is its `id`; the fields are listed under
[filing a bug](#filing-a-bug). `status` becomes `fixed` only through
[`ledger promote`](#the-fixed-rule-single-home). Historical reports are not
proof of the current candidate.

Read and edit a row with the tool, never by reassembling the whole ledger:

```sh
python3 -m tools.harness ledger show <ID>               # the row file
python3 -m tools.harness ledger show <ID> --field status
python3 -m tools.harness ledger list [--area gameplay] [--status open]
python3 -m tools.harness ledger edit <ID> --set 'title = "..."' [--set ...]
```

`ledger edit` refuses `id`, `status` and `promotion` (the file name and the
promotion tooling own them), refuses unknown fields, and re-parses the row
before writing, so a typo cannot corrupt the gate. A plain editor on the one
row file is equally fine: the file holds one row and nothing else.

## Filing a bug

Copy the nearest row file, rename it to the new `id`, and fill in every
required field (or start from this block):

```toml
[[bug]]
id = "AREA-SHORT-SYMPTOM"
title = "Describe the observable failure"
area = "gameplay"
severity = "major"
status = "open"
source = "Repo-relative report or plain description"
repro = "manual"
regression = []
failing_first = "not recorded"
```

- `id`: unique, stable uppercase hyphenated symptom identifier; `title`: concise symptom.
- `area`: `gameplay`, `client`, or `harness`, assigned at filing and not changed by fixers.
- `severity`: `blocker`, `major`, or `minor`; `status`: file as `open`.
- `source`: repo-relative location or plain description, not an owner-local path.
- `repro`: `manual`, `client`, `command`, `cargo`, or `source`, according to the original report
  (`command`: an offline operator command with no client surface).
- `regression`: a list of green `cargo:<crate>:<exact test path>`,
  `harness:<profile>:<scenario>`, or `harness:<profile>:<verdict>` checks;
  initially it may be empty. Profile verdicts run the windowed native client,
  except `settlement-map-qa:localized_text`, which needs the Russian locale and
  runs vanilla Java with `--language ru_ru`, and `settlement-map-qa:starter_chest_race`,
  which runs vanilla Java with `--scenario quickwins-c1-race`.
  A profile may publish boolean `details.verdicts` keyed by behaviour; the
  referenced key must be exactly `true`. The `all` key denotes the whole
  profile; when no boolean verdict map exists, `all` requires a passing
  profile receipt. Legacy profiles expose their existing `PASS` behaviour
  maps: `village-qa` has `door_walk`, `sleep_return`, `night_door`,
  `farmer_harvest`, `miner_deposit`, and `lumberjack`; autonomy, claim and
  army expose their named behaviours. `buildings-qa` uses `<blueprint>.<behaviour>`, for
  example `wall.L1_built`; `perimeter.<behaviour>` (`well_built`, `gates_built`, `wall_built`,
  `enclosed`, ...) selects the settlement's own wall, gates and well, not a blueprint
  (`BUILDINGS_QA_SELECT=perimeter`). Partial or resumed profile overrides are rejected.
  A passing profile is evidence only for symptoms its assertions actually cover.
- `repro_check` is optional: a nonempty string or list of nonempty strings for
  known-red reproducers, kept out of the green regression gate.
- `ingress_proof` is optional: a nonempty string or list of strings (`file:line` write paths with what each
  validates). See the unreachable-state rule below.
- `failing_first` is optional and, when present, is `reported` or `not recorded`;
  report the actual pre-fix evidence, never infer it from a passing test.

A ref is `cargo:<crate>:<test>` and `harness:<profile>:<scenario|verdict>` for this repository,
`python:<dotted module>:<TestClass.test_method>` for a fix in the harness/tooling code itself
(`ledger promote` runs it with `unittest` in the candidate tree), and
`loader:python:<dir>:<module>.<TestClass.test_method>` (a Loader python test, run with `unittest` from
`<loader>/<dir>` so the module imports its siblings) or `loader:gradle:<module>:<Java test class>`
(one Gradle `:module:test --tests` selector, judged from its JUnit report) for a test in the Loader
repository; see [loader fixes](#loader-fixes-loadersha). And
`client:<crate>:<test>` for a fix in the client repository (`ledger promote` uses
`cargo metadata --no-deps --format-version 1` in `KRASNOKAMEN_CLIENT_ROOT` or the sibling
`solaris-client` checkout, then runs `cargo test -p <crate> --lib -- <test>` for a
library or `--bin <name>` for a bin-only package with one testable binary; unknown or
ambiguous targets fail closed naming the package). A `cargo:` registration names
the full exact, non-ignored test function: [`regressions.py`](../../tools/harness/regressions.py)
uses `test(=name)`, so a module literal is not a wildcard. Client selectors use
the client runner's filter and must execute at least one test with none failed.
Unknown or ignored tests are unrunnable, never green. A successful discovery/list
only proves the selector exists, not that its test executed or passed.

`fix_commit` (optional until promotion) is the lowercase hex id of the commit
that fixes the row; `promotion` is set only by `ledger promote`, see
[the `fixed` rule](#the-fixed-rule-single-home).

Add a regression check for a fix; `ledger promote` runs the negative control.
A `harness:<profile>:<scenario>` ref names an existing runnable scenario. A
profile verdict runs that profile once for all rows citing it:
the child receipt must pass and the named behaviour must be true in its
boolean verdict map, or `PASS` in its legacy behaviour map. Unknown profiles
and verdicts fail validation. `harness:loader-live:<scenario>` is valid for
graphical `loader-live` scenarios: the runner executes them exactly as the
loader-live gate does (Fabric platform, the tree's own `pin-release` server
via `--server-bin`, an empty `--plugins-dir`, the private audio wrapper, and
the Loader checkout/Gradle home/client jar from `KRASNOKAMEN_LOADER_ROOT`,
`GRADLE_USER_HOME` and `KRASNOKAMEN_CLIENT_JAR`), records the harness Xvfb
display in the receipt, and fails closed (unrunnable, never green) when any
of those are missing or the scenario is not a registered loader-live entry.
`regressions-client` never builds a release server. Run
`<work>/solaris/tools/dev/pin-release <tree>` first: the gate discovers a
server manifest in `<work>/pins/`, or uses `KRASNOKAMEN_SERVER_BINARY` when
explicitly supplied. It checks the manifest, executable SHA-256 and that pin's
`.analysis/ledger-builds/<sha256>-<tree sha256>.json` registration against the tested tree's
`tree_fingerprint`. `KRASNOKAMEN_LEDGER_ROOT` selects the registry root when a
control pin was registered there. A missing, unregistered or mismatched pin
fails closed and names the exact `pin-release` command. A negative control
needs its own reverted-tree pin, never the candidate's. The receipt retains
the manifest path, binary path, SHA-256 and tested tree. Pin any supplied client
binary separately.
Every green Cargo check in `regression`, even for an open row, runs on every
integration; client checks run in the client tier. Keep a known-red reproducer
in `repro_check` instead; it is not a gate. Never weaken or delete checks to
make a run green.

A manual/source row whose state no ingress can produce (importer, commands, WAL replay, plugins,
structure placement all validate) is closed by a Cargo check through the real tick path plus a
recorded ingress proof (refs to each validating write path).

A row is fixed only when its check passes and is the same or stricter than the
original repro. Owner/manual and real-client repros, and all client-surface
bugs, require a real-client check; source/test findings may use a Cargo test
through the same real tick/gameplay path or a TCP/native scenario. A `command`
repro (an offline operator command such as `world prune` or `world census`,
with no client surface) needs a Cargo test that drives the same command path on
an equivalent input (validation refuses a `fixed` command row without a `cargo:`
ref), and the original command rerun on the fixed build with its receipt named
in `source`; the reviewer checks that receipt. Tests must
assert vanilla or owner-accepted behavior, not merely the current output.
One row describes one symptom: split covered behavior from remaining gaps.
The phase reviewer confirms each promotion batch matches the symptom and has
the required failing negative control; seeing the original failure before
editing is recorded when available, not required. A unit test cannot close a
known client failure.
Client promotion requires an assertion matching the repro; visual-review
scenarios require recorded review.

`python3 -m tools.harness run regressions` is part of L2 (`correctness`); run it
alone only after an integration that skips L2. Run
`python3 -m tools.harness run regressions-client` when a client-surface fix
is promoted and before release. The independent breaker of the
[QA protocol](../QA_AGENT_PROTOCOL.md) remains a release requirement.

## Write-ups for blockers, data loss, dupes and agent incidents

For a `blocker` row, a data-loss or dupe bug, or an incident an agent caused,
the write-up lists the contributing factors and which defense layers (tests,
gates, reviews, guards) held or failed, not a single root cause: each fault was
necessary and only together they were sufficient
([Cook](https://how.complexsystems.fail/), points 3 and 7). Write what was
known when each action was taken, without "should have" (point 8). Judge a
fix by whether it removes a factor or adds a defense layer, not by who erred.

## The `fixed` rule (single home)

A row becomes `fixed` only through `python3 -m tools.harness ledger promote
<ID>`. Nobody sets `status = "fixed"` or `promotion` by hand: `run regressions`
fails a `fixed` row without a `promotion` receipt, and a receipt that does not
match the row's `id` and `fix_commit`. The only rows exempt are those already
`fixed` at base `ffd52010` (frozen list `tools/harness/ledger_base_fixed.py`; a
reopened row leaves the list in the same edit).

**What workers do:** add `regression` refs (and `repro_check`, `failing_first`
evidence) and record `fix_commit`: the one commit with the fix (the test may
be in it or earlier, never only in later commits), or the `patch:<sha256>` of
step 1 for a forward-ported fix, a fix in several commits or a guard older
than the finding - and keep that patch committed at
`docs/bugs/fix-patches/<ID>.patch`, so the promote that later runs needs no
receipt outside the tree. They do not edit `status` or
`promotion`, and they do not decide that a row is fixed: a green test that also
passes without the fix proves nothing, which is why the script, not the
worker, runs the negative control.

**What `ledger promote` checks**, all of which must hold:

1. `fix_commit` is recorded and is contained in the current tree, or, for a fix
   not yet committed, is `patch:<sha256>` of the non-test hunks of the git diff
   passed as `--fix-patch` (a mismatch prints the value to record; the patch
   must still apply in reverse on the candidate, so a later edit or re-break of
   it refuses). A **client** fix is `client:<sha>` instead: the commit of the
   client repository (`KRASNOKAMEN_CLIENT_ROOT`, else `solaris-client` beside
   the core checkout), resolved there and refused when unknown. A fix whose only
   observable is a client unit test — no scene assertion can flip on it — is
   `client-test:<sha>` instead, proved by reversing the fix's own production
   hunks (see [client fixes proved by a unit-test
   reversal](#client-fixes-proved-by-a-unit-test-reversal-client-testsha)). A
   `client:<sha>` fix is a client
   behaviour change, so it is proved by receipts, not by reversing server
   hunks: `--client-control-receipt` is a real-client `result.json` of the
   candidate client with only the fix's non-test hunks reversed, whose client ref
   is **red**; `--client-receipt` is a `passed` one of the client that contains
   `<sha>` with the same ref **green**. Their pinned clean commits must differ by
   exactly the fix's non-test patch (verbatim patch ID); an unrelated change, even
   on the same path, refuses. Both ran on the tree being
   promoted (the same `tree` identity and server-binary registry checks as rule
   3). The client is bound to its source through the pin-release manifest, not
   the checkout beside it: each receipt records `client_binary` at run start
   (the binary's sha256, hashed then, its `manifest.json`, and the manifest's
   `head`/`dirty`), and promotion refuses a receipt without it, a manifest that
   pins another binary than the one that ran, a `dirty` pin, or a pin without a
   head. The restore's pinned head must contain `<sha>`; the control may be a
   descendant with a committed reversal. **Red is an observed assertion failure**:
   the control receipt has to be `status = "failed"` with the registered proving
   assertion recorded false. For a named scenario, record it on the row as an
   inline map, for example `proving_assertions = { "harness:entities:native-v2-craft-drag" = "initial_drag_keeps_server_cursor" }`.
   Choose the assertion that proves this row's symptom, not a setup assertion.
   A missing mapping, timeout, launch error or crash cannot carry the control.
   The restore receipt covers only its reported refs; every other registered ref
   must run green on the candidate. A row whose only refs are unit tests cannot
   use this form. The candidate is the
   exact tree: `HEAD` plus its uncommitted diff and untracked files
   (`docs/bugs/` aside), identified by a fingerprint. No commit is needed for
   the production patch, but test files, inline Rust tests, and nextest config
   must be clean; `promote` names any dirty ones it refuses. The control and
   the restore worktrees reproduce the candidate, verified by that fingerprint.

   Patch mode is also the only route for a committed fix whose hunks no longer
   reverse-apply on the candidate: such a commit is refused ("fix no longer
   reverses on the candidate; use patch: mode"), never controlled at its own
   historical tree (D131). The patch
   is then the fix's non-test hunks forward-ported to the candidate: the HEAD
   equivalent of each original hunk, nothing else. It is committed beside the
   promotion receipt as `promotions/<ID>.patch` (a re-audit passes it as
   `--fix-patch`), and `promotions/<ID>.hunks.md` maps it: the original
   commit(s), each original hunk and its HEAD equivalent, and every original
   hunk left out with one of these reasons:

   - (a) it is another row's or owner's behaviour (its reversal would turn
     the control red on someone else's assertion);
   - (b) it is API the kept tests call (reversing it stops the control from
     compiling);
   - (c) a later commit rewrote that code, so no HEAD equivalent exists.

   Any other omission refuses the patch. Further rules:

   - **a/-side adaptation.** Leaving out a (b) hunk can force the reversed
     code to construct the kept API (for example an empty field). That line
     is the only a/-side text allowed outside the original pre-image. It must
     not change behaviour, and `hunks.md` names it. Accepted classes (D123
     review rounds, 2026-11-24): the kept-API constructor above; an
     instrumented kept type that stays structurally identical to the
     pre-image (for example an admission part); a surviving half of a mixed
     hunk after an (a) split; a type annotation that only lets the reversed
     tree compile; and a kept helper whose callers are all among the
     carried hunks. **Accepted kept-helper class:** a whole module or helper
     the fix created (for example `mounts/menus.rs` or `lectern.rs`) may stay
     in the control tree, unreferenced by the reversed dispatch, when every
     caller of it is inside the carried hunks; the control then proves the
     dispatch is what the row lacked, not that the module is absent. Nothing else is
     an adaptation: an invented stub or
     guard is refused. **A hand re-break is not a control**: an a/-side line
     that invents the assumed pre-fix outcome (a counter, a comment, a branch
     the original code never ran, or a behaviour the pre-image did not have)
     proves only the hand edit. Every a/-side line is either the real
     pre-image (the base blob, or its exact HEAD equivalent) or a named
     constructor line that changes no behaviour. When no real pre-image
     exists (the fix created the function or file), the row has no faithful
     control and stays `open` with the reason recorded, as
     TIME-LEGACY-FROZEN-CLOCK-CONVERSION does (its conversion module has no
     value-level pre-image; every reversal of the remaining wiring refuses
     before an assertion, which is a fixture panic, not a control).
   - **Proving ref.** `promote.json` records every red ref as plain `FAIL`.
     So `hunks.md` names the ref and `file:line` whose assertion is the row's
     symptom. A ref that goes red only on an assertion of the kept API
     (emptied by the adaptation) is listed as not evidence.
   - **Fix in several commits.** When the path the row describes was closed
     by more than one commit (a later commit completing it), the patch
     carries the non-test hunks of each of them. `hunks.md` lists every
     commit, says which part of the row's path each one closes, and records
     the control of any commit reversed on its own.
   - **Guard older than the finding.** When the code that keeps the bug out
     predates the row (the finding was a missing test, not a missing fix),
     there is no fix commit. `hunks.md` says so and names the commit that
     introduced the guard (`git blame -C -C`). The patch is exactly the
     guarding hunks at HEAD. The control must go red on the row's own
     loss/panic assertion, not on a refusal or a precondition.
2. The row has at least one registered check.
3. For `area = "client"` or `repro` in `manual`/`client`: a graphical harness
   ref, and `--client-receipt <result.json>` of a real-client run whose status
   is `passed`, which covers one of the row's real-client refs (a scenario ref
   by `named_scenario.id`; a profile ref by profile plus the named verdict),
   and whose `tree` field, written by the harness at run start, is the
   candidate's fingerprint (HEAD and the hash of the uncommitted state). A
   receipt without that field, from another revision or with other
   uncommitted changes is refused: rerun the gate on the candidate, and after
   any later edit. The receipt must also name a release server binary that
   `pin-release` registered for exactly that candidate in
   `.analysis/ledger-builds/<sha256>-<tree sha256>.json` (a legacy
   `<sha256>.json` is still read). An unregistered or foreign
   `KRASNOKAMEN_SERVER_BINARY`, or a binary pinned from an older source, proves
   nothing about this source and is refused.

   **Client rows whose real fix predates the client rename** (`f0a6abf6`,
   2026-11-12) have no buildable pre-fix client, because `pin-release --client`
   builds the crate name of the tree. Their control is a committed **reversal**
   on the current tip: one commit removes exactly the guarded behaviour the real
   fix added - `git apply -R` of that fix's non-test hunks as far as they still
   apply, and where the code moved, the same deletion hand-ported with the
   original lines named by `git blame -C -C` - and the next commit re-lands it,
   becoming the row's `fix_commit`. The reversal must be that deletion, never an
   invented stub or an early return: the two commits' diff has to equal the
   forward-ported real fix. **The same `promotions/<ID>.hunks.md` map as patch
   mode is mandatory here**: the real commit (a bulk one such as
   `client:e8fe40c2` is usual), every one of its non-test hunks with the file,
   the `@@` header and its HEAD equivalent or the (a)/(b)/(c) omission reason,
   and the a/-side adaptation (for example `_`-parameter renames when the
   reversed function keeps its signature because kept tests call it). A client
   reversal promotion without that map is refused. The promotions record the
   re-landing sha, which must reach the client main without squash or rebase so
   `fix_commit` stays resolvable.
   The same reversal pair is also the route for a fix made AFTER the rename
   when the ledger cannot compute the fix commit's non-test patch because one
   hunk straddles a `#[cfg(test)]` boundary (`QA5-FAST-CLICK`, client fix
   `a2d0c81d`: the `actions.rs` hunk crosses it): the pair splits the fix into
   its production lines, the map names the real commit and every omitted test
   hunk, and the pair's diff must equal those production lines verbatim
   (decided by Main under D123, owner may veto).
4. Automatic negative control: the fix commit's non-test hunks (test files,
   `docs/` and Rust hunks inside inline `#[cfg(test)]` modules stay) are
   reverse-applied onto the candidate in a temporary worktree
   (`control_tree: head` in `promote.json`); a fix whose hunks no longer apply
   there is refused and re-recorded as a `patch:` (above). Then every ref runs
   and at least one must be **red** at its assertion. A control that stays
   green is refused. A ref that cannot build or run in the reversed tree is
   recorded `unrunnable`, not red — reversing a hunk that removes the API the
   ref's own test file calls leaves that ref unrunnable — so the control is
   also refused when no ref reds, which is the same rule seen from the other
   side. Split the fix commit so tests do not need its new API when nothing
   else reds, or promote nothing. A hunk straddling
   the test-module boundary is refused. Graphical refs run in the control on
   the real client.
   A deletion at an inline module boundary is ambiguous and is refused.
5. The restore, always on a worktree of the exact candidate (never at
   `fix_commit`): every ref is **green**, so a regression after the fix
   refuses promotion. A ref already passed by the supplied client receipt (same
   `HEAD`) is not rerun, unless it carried the red control. A client ref is red
   only when the failed receipt carries an explicitly failing verdict for that
   profile check (a failed run's `details.scenario` is read from the probe's own
   `<artifact>/<probe>/scenario.json`, so a verdict recorded before the aggregate
   assertion failed counts, e.g. village-qa `miner_deposit` before
   `all_behaviours_pass`); a timeout, a client that never reached play (`in_play`), a
   missing prerequisite or any failure without a negative verdict is
   unrunnable and cannot be a control. A scenario ref has no verdict map; it is
   red only when the failed receipt's `details.named_scenario.id` is the ref and
   its own `error` reads `<id> assertion failed: <name>` (never the argv of the
   child), with `<name>` not a connection check (`in_play`, `*_entered_play`,
   join) and the failure not tagged before the scenario body (`join`, start-up
   phases). Any other failure, a timeout or a join flake never counts. A profile
   ref whose run cannot name a failing verdict follows the same assertion rule:
   `all` of a profile without a verdict map (`autocraft-qa`,
   `village-autonomy-qa`) is red when the failed receipt's `error` reads
   `<id> assertion failed: <name>`, and a `buildings-qa` `<blueprint>.L<n>_<behaviour>`
   that was never recorded is red only for a same-blueprint, same-level
   server/client-observed product outcome in `NEGATIVE_CONTROL_STEPS`
   (`order_available`, `upgrade_available`, `project`, `built`, `blocks`).
   Stocking, planner fencing, observation races, unknown steps and failures at
   another level remain unrunnable.
   A buildings-qa `<blueprint>.<behaviour>` ref runs that one blueprint
   (`BUILDINGS_QA_SELECT=<blueprint>` in the command's argv, 1800 s deadline): the
   20-blueprint default run has one 600 s deadline and cannot pass.
   Control worktrees
   are temporary `git worktree`s beside the other worktrees (`<work>/wt/`), in
   one of three independent promotion sets (`ledger-control-pool-<k>.lock`,
   k = 1..3). A promote takes the first free set and, when all three are held,
   blocks on every set at once and takes whichever frees first, so three
   promotions run concurrently instead of queueing on one host-wide lock. Each
   set has its own fixed paths (`ledger-control-p<k>-candidate`,
   `ledger-control-p<k>-fix`). Every cargo build or test for such a tree runs
   through `mem-run.sh` with `cwd=<tree>` and without the caller's
   `CARGO_TARGET_DIR`, so the runner gives the tree its own slot (marks it,
   touches on a switch) and the fixed path reuses it instead of growing the
   target dir; the release server build checks the slot marker against the tree
   actually built. No incremental cache.
   Each has its own copy of `data/vanilla` (never links into the source tree)
   and is removed afterwards. Old-version leftovers from before the pool
   (`wt/ledger-control.lock` and the `ledger-control-{candidate,fix}`,
   `ledger-client-control`, `ledger-loader-control` trees) and a set 2/3 tree
   left by a killed promotion are not removed by the pool (a lone promote
   always takes set 1): delete them by hand once no old-version lane runs.
   A real-client run in such a tree first copies its
   evidence (reports, logs, `result.json`, screenshots; no worlds or binaries)
   to `client-artifacts/<run>/` in the promote artifact dir, and the per-ref
   `receipt` in `promote.json` points there, for red and unrunnable runs too.

On success it writes `docs/bugs/promotions/<ID>.json` (commit, files reversed,
per-ref control and restore results, client receipt) and sets
`status = "fixed"` and `promotion = "docs/bugs/promotions/<ID>.json"` in
`docs/bugs/rows/<ID>.toml`; commit both. `--dry-run` runs everything and writes only its receipts under
`.analysis/validation/<stamp>-ledger-promote-*/` (`promote.json`), never the
ledger; on an already `fixed` row it re-audits the fix (a reopened-row check).
A refusal prints the reason and exits 1.

```
python3 -m tools.harness ledger promote ID [--client-receipt PATH] [--fix-patch PATH] [--dry-run]
python3 -m tools.harness ledger graphical-control ID (--receipt PATH | --control-tree DIR) [--dry-run]
```

Heavy: on the owner host run it under `mem-run.sh` (`mem-run.sh -m 12G python3
-m tools.harness ledger promote ...`), which caps its memory; each build stage
runs `mem-run.sh` again and reuses the promote's scope. In a worktree under
`<work>/wt/` the tool refuses to start outside `mem-run.sh`
(`KRASNOKAMEN_HARNESS_CPU_SCOPE` unset) and takes `wt/ledger-control.lock`, so
promotions sharing the fixed control trees run one at a time.

### Test-only fixes (`test_only_fix`)

Some rows are test-oracle defects: the product already matches vanilla and the guard is the corrected
test. When **every** hunk of `fix_commit` is in a test file (no production hunk, and no inline
`#[cfg(test)]` hunk in a production file), a row may set `test_only_fix = true` and name a committed
defect-injection patch in `control_b_patch` (repo-relative, normally
`docs/bugs/test-only/<ID>-defect.patch`). Promotion runs two controls and records both in
`promote.json` (`control: "test_hunks"`, `control_b`):

- **A, the old oracle**: reverse the fix's test-file hunks onto the candidate and require a registered
  ref to go red. Only a real assertion failure counts; a removed test is unrunnable, never red.
- **B, the defect injection**: apply `control_b_patch` — a minimal mutation of the production behaviour
  the test claims to guard — to the restored candidate, require a registered ref red, then restore and
  require every ref green. A control that stays green refuses promotion.

The gate also refuses the flag when the fix commit changes any production file, when a changed test file
holds no test one of the row's refs runs, when the defect patch is missing or does not apply, and when
the row lacks `repro_check`. Stricter still:

- **The defect patch is the committed one.** It is read with `git show HEAD:<control_b_patch>`, its
  worktree copy must be identical (an uncommitted edit refuses), and the row records
  `control_b_sha256`, which `regressions.run` checks against the committed bytes and
  `_promotion_errors` checks against `control_b.sha256` in the receipt.
- **It mutates production behaviour only.** Split like a fix patch, it may not touch a test file, an
  inline `#[cfg(test)]` module, a `docs/` path other than `docs/bugs/`, a `*.md`, or build
  configuration (`Cargo.toml`, `build.rs`, `.cargo/`, `.config/`); a hunk it would leave out refuses.
- **Red means an observed assertion failure in a changed test file.** Cargo's `panicked at <path>` must
  name a reversed test file; a python innermost traceback frame must be a changed test file and an
  `AssertionError`. A production-code panic or a build failure is unrunnable, never red.
- **The same test must flip both ways.** `control_b.proving` is the non-empty intersection of control
  A's red refs and control B's red refs, and each proving ref's test must be defined in a reversed
  file (python by module path, cargo by `crates/<crate>/` plus module path). Coverage is by that exact
  mapping, never by a bare function name.
- **Control A always runs on current code** (`control_tree: head`): a test-only fix whose test hunks do
  not reverse onto the candidate refuses. A
  `client:<sha>` or `client-test:<sha>` fix can never be `test_only_fix`.
- **A row may narrow control A to its own file.** One commit often corrects several oracles; a row
  names them in `control_a_files` and control A reverses only those hunks, with every listed file
  required to be a test file of the fix commit and to hold a ref's test. Without the field control A
  reverses every changed test file (and every one needs a ref).
- A corrected test in a test file the fix deleted or renamed has no control A and refuses.

`repro_check` states what the old oracle asserted and the independent evidence (vanilla
data/bytecode, spec, arithmetic) that the new one is right; the review of a test-only batch lists
old versus new assertions and justifies every removed or loosened one. A row whose defect cannot be
injected stays `fixed-unpromoted(test-only: oracle unproven)` — control A alone never promotes.

### Client fixes proved by a unit-test reversal (`client-test:<sha>`)

A client-repository fix whose guard only a client unit test observes (a `#[cfg(test)]` save path, a
test-build-only guard) has no scene assertion that can flip on it: a scene run reads the client it
launched, so the same pinned client passes with the guard reverted
(`CLIENT-SETTINGS-TEST-POLLUTION`, owner decision CL-7 option (a)). Such a row records
`fix_commit = "client-test:<sha>"` and is proved by a reversal pair in the client repository. It
registers no graphical ref, so the row must carry `client_test_reason`: the exact observable no scene
can flip (or the owner decision that says so), which `regressions` enforces — the exemption is not a
blanket waiver:

- **Control A (real pre-image).** The fix commit's own non-test hunks are `git apply -R`'d in a
  disposable copy of the client checkout. The copy is the same source as the checkout (a dirty
  `src/`, `Cargo.toml`, `Cargo.lock` or `build.rs` refuses), the checkout's `HEAD` must contain the
  fix, and a hunk that no longer reverses refuses: the a/- side is the commit's pre-image, never a
  hand-written re-break and never another tree. The registered `client:` ref must go **red**, and the
  red must be an assert macro inside test code — a test file, or an inline `#[cfg(test)]` module of a
  production file, whose path the fix changed (a fixture panic, a production panic or an assert in
  another file refuses).
  The reversed copy runs in a scratch environment: `HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`,
  `XDG_CACHE_HOME`, `APPDATA` and `LOCALAPPDATA` point into `.analysis/client-sandbox/<stamp>` of the
  candidate, `KRASNOKAMEN_CLIENT_SETTINGS` points at a scratch file and `KRASNOKAMEN_CLIENT_ROOT` is
  dropped. A reversed test-build guard re-enables the side effect it suppresses, so a control that ran
  in the operator's environment wrote `hotbar_2` into the owner's real
  `~/.local/share/krasnokamen-client/client-settings.json` (review B3, 2026-11-24); no control may
  touch the host.
- **Restore.** Every ref runs in the client checkout that contains the fix and must be green.

The receipt records `control = client_test_reversal`, the reversed files, the patch digest of the
reversed hunks and the client checkout head; `regressions` refuses the receipt on a row that is not
`client-test:` and refuses a row whose receipt does not carry it. `--client-receipt` and
`--client-control-receipt` are refused here (they prove a behaviour a real client observes), and
`test_only_fix` is refused as well: that flag is a core oracle plus a defect patch.
`CLIENT-SETTINGS-TEST-POLLUTION` is the first row promoted this way.

### Strict client gates (`client-gate:<id>`)

A client-repository gate is one whole-repository command — a strict lint, not a scene a pinned client
could show. It registers as `client-gate:<id>`; the command is registered by id in
[`client_gates.py`](../../tools/harness/client_gates.py) (`clippy-workspace-all-targets`,
`clippy-client-all-targets`), never inline on the row, so the tooling refuses an id it does not know
by name. A gate ref runs in the client checkout (`KRASNOKAMEN_CLIENT_ROOT`, else the sibling) and
passes only on exit 0 with no compiler diagnostic. Its red is the compiler's own located diagnostic
(`error: <lint>` or `warning: ...` with a `--> file:line:column` block); a missing checkout, an
unregistered id, a dirty checkout (any modified `.rs`/`.toml`/`.lock`/`build.rs`), a checkout with no
HEAD, a build failure or a timeout is unrunnable, never evidence. Gates run only in the client tier
(`regressions --include-regression-client`) and in `ledger promote`, never in the default L2.

A gate row is proved like a client unit row: `fix_commit = "client-test:<sha>"` and
`client_test_reason` (no scene can flip a repository command), and the promotion reverses that client
commit's own non-test hunks in a disposable copy of the checkout; the gate must red there with the
located diagnostic and be green on the checkout that contains the fix. `CODE-CLIENT-CLIPPY` is the
first row promoted this way. A bulk commit whose hunks straddle a `#[cfg(test)]` module cannot be
split, so such a row names the latest lint-fix commit that reverses cleanly as its pair and says so in
`repro_check`.

### Loader fixes (`loader:<sha>`)

The Loader (`../solaris-loader`, `KRASNOKAMEN_LOADER_ROOT`) is its own repository with its own python
tests, so a fix there has no core ref kind. Such a row records `fix_commit = "loader:<sha>"`, a
`loader:python:<dir>:<module>.<test>` or `loader:gradle:<module>:<class>` ref, and `control_files`: the non-test files of that commit whose
hunks the control reverses. One Loader commit often bundles a Gradle dependency, an agent facade or a
message-shape change another row owns; reversing the build file can stop the test run and reversing
another row's behaviour proves the wrong thing, so the row names the production files its own guard
depends on. Every listed path must be a non-test file the commit changes; a test path, a `docs/` path,
or a file the commit creates or deletes refuses.

Control: the narrowed real pre-image (those files' own hunks, `git apply -R`) is reversed in a
disposable copy of the Loader checkout — a dirty `*.py`, `*.java`, `*.kts`, `*.gradle`, `*.properties`
or `*.toml`, a checkout that does not contain the fix, or a hunk that no longer reverses refuses — and
the loader ref must go **red at an `AssertionError` inside a test file of that copy** (a production
import error or a unittest setup error is unrunnable). The restore runs every ref green in the Loader
checkout. Receipt `control = loader_reversal` with `reversed_files`, the patch digest and the Loader
head. `loader:` refs run in the client tier (`run regressions-client`), never in the ordinary L2 cargo
lane, so the Loader source stays in its own repository. A `loader:gradle:` red counts only when the
JUnit report says `AssertionError` and names the source line (a compile error, a NullPointerException
or a missing report is unrunnable), and it is recorded with the test file and line.

### Recording a graphical control later (backfill)

A client-visible row whose promotion recorded its graphical refs `unrunnable` in
`promotions/<ID>.json` (no reverted-tree pin was available for the control run; see rule 3) can
have the control recorded afterwards without promoting anything:

```
python3 -m tools.harness ledger graphical-control <ID> --receipt <result.json> [--dry-run]
python3 -m tools.harness ledger graphical-control <ID> --control-tree <worktree> [--dry-run]
```

Only an already `fixed` row qualifies, and only its graphical refs still recorded `unrunnable` (or
missing) are touched. `--control-tree` is a worktree at the row's promotion `tree_head` with the fix
hunks reversed: the tool checks that reversal by applying the row's patch **forward** to it (a tree
the fix does not apply to is not the control), refuses a tree whose fingerprint is still the
candidate's, then runs only those refs there and needs the same reverted-tree `pin-release` as a
promotion (the error names the `pin-release` command). `--receipt` records a failed `result.json` of
such a run instead. Either way the tree must be exactly the one the promotion recorded:
`promote` writes `control_tree_sha256`, the fingerprint of the candidate worktree after `apply -R`
(read before control B resets it), and a receipt whose `tree.sha256` or a `--control-tree` whose
`tree_fingerprint` differs refuses — a tree that merely looks reversed, or a run on any other dirty
tree, is not that control. A receipt must also name a server binary the tooling built from that
reversed tree (`.analysis/ledger-builds/<sha256>-<tree sha256>.json`), carry a clean client pin, and
be red at the row's proving assertion (a scenario needs `proving_assertions`; a profile needs its
verdict). A promotion that predates `control_tree_sha256` cannot be backfilled: re-promote the row
from a candidate whose promotion records it.

On success it replaces the `unrunnable` record with the red one (receipt path, assertion) and appends
a `graphical_control_backfill` entry to the promotion receipt; the row's status and `promotion` field
are untouched. It never accepts green, timeout, launch-error or unrelated-assertion evidence, and it
never mass-runs: one row per invocation.

### How a red is classified (control A and B, every row)

Panics are captured with their message and source line, keyed by `(crate, test)`, and sorted into
three classes before `valid_red`/`valid_b` are computed:

- **(a) `fixture-failure` — refused.** A confirmed fixture/setup panic: a registry or world-builder
  message (`registry loads`, `invalid map color registry`, `test_registry(`, `fixture_store(`) or a
  `#[track_caller]` `Option/Result::unwrap`/`expect` panic. The registered test body never ran, so the
  red says nothing about the fix.
- **(b) assertion — accepted.** The panicking source line is an `assert!`/`assert_eq!`/`assert_ne!`/
  `debug_assert!`/`panic!(`. A shared helper inside a `*_tests.rs` file counts (for example
  `core_journal_backpressure_tests.rs`), because the row's own test calls it. A line inside an inline
  `#[cfg(test)] mod` of a production file counts too (`settlements/ui_text.rs`'s `assert_ne!`); an
  assert in the production code beside it does not.
- **(c) `unproven-panic` — refused unless named.** Any other panic (a helper without an assert macro,
  a `-> Result<()>` test failing through `?` with no panic line at all): the row must name it as
  `proving_assertions = { "<ref>" = "path:line" }` and the recorded location must equal that value.
  A red without any panic location on a cargo ref is class (c) too.

**Every ref the row claims as proof must be red on the control.** By default that is every ref in
`regression`: one red ref cannot carry a second check that stayed green, and a crashed, timed-out or
otherwise unrunnable run is never red. A row that carries a check which cannot flip (a metadata fence,
an elided client test that passes with and without the fix) names the refs that do prove it with
`proving_refs = ["<ref>", ...]`, must keep them a subset of `regression`, and says in its hunk map why
the others cannot flip (LEDGER-CONTROL-COVERS-ONE-REF).

A test-only row's control B must additionally red **inside a hunk the fix reversed**: the gate reads
the diff the reversal actually applied (git offsets the hunk once the file has grown) and requires
the red line, together with the operator starting there, to overlap a changed line; a red elsewhere in
the same file is another assertion and refuses with the range printed. `control_a_files` narrows what
control A reverses and never what the gate checks: every non-doc path of a test-only fix commit must
be a test file, and `_promotion_errors` refuses a receipt whose `reversed_files` differ from the row's
`control_a_files`.

Patch-mode rows (`fix_commit = "patch:<sha256>"`) are re-audited by reversing their committed
`docs/bugs/fix-patches/<ID>.patch` on the tree being promoted: `ledger promote` and a `--dry-run`
refuse a patch that no longer reverses. `regressions` (L2) only reports that backlog of stale
patch-mode receipts (missing committed patch, or a patch a later commit rewrote): it prints "stale
patch-mode receipts: N, M without patch" and writes the list. A committed patch whose non-test hunks
differ from the row's `fix_commit` digest is an error in both.

## Probing rows before assignment

`python3 -m tools.harness ledger probe <ID...>|--open [--client] [--out PATH]`
runs each row's registered refs (`regression`) on the current tree, once per
distinct ref, and prints one line per row:

- `green`: every ref passed;
- `red [ref: status]`: a ref failed; the row is still broken here;
- `needs-real-client [cargo ref green, ...]`: a harness ref was not run (pass
  `--client` to run it with its tree's existing release pin and drive the real client);
- `unrunnable(reason)`: no registered check, unknown or ignored test, build error,
  or a ref that cannot run.

Cargo discovery and execution select only the packages named by the requested
refs (`-p <crate>`), not the whole workspace. Exact-name matching and refusal
of missing or ignored tests still apply.
For Cargo refs, only a nextest `FAIL` result is red; `TIMEOUT`, `ABORT`,
`TMPASS` and other non-pass statuses are unrunnable, not negative controls.
`FAIL` alone does not prove the original symptom: review its failure and
assertion against the repro.

`--out` writes the full per-ref result as JSON (also valid YAML). The receipt
records the server candidate fingerprint before and after the probe. If they
differ, `tree_stable` is false, every row is `unrunnable` and the command exits
2: per-ref observations remain in the receipt but cannot be attributed to one
server tree. The fingerprint excludes `docs/bugs` and ignored files, and
matching endpoints do not rule out a transient edit during the run.
`client:<crate>:<test>` refs run in the selected client checkout even without
`--client`; the server fingerprint does not identify that checkout or detect
changes to it. Record and verify the client tree separately when attributing
those results. `client_run` records the `--client` option, not proof that a
graphical client ran.
Probe exits 0 after classification even when a row is red or unrunnable: inspect
each row and its refs. A green check alone does not close a row; compare it with
the original symptom and required client evidence. Probe never changes the
ledger and is not promotion.

`--client` (and `promote` of a client row) needs the real-client environment of
[real-client gates](../development/real-client-gates.md#3-environment-block):
a pinned client/server pair of the same protocol, the data, resource and asset
roots, and a fresh `XDG_DATA_HOME` whose `krasnokamen-client/` symlinks `facts
assets data resources mojang`; run it through `scene-slot.sh` and `mem-run.sh`.
The tool consumes the tree's registered `pin-release` server, including an
explicit `KRASNOKAMEN_SERVER_BINARY` only when it matches that registration.
Control and restore trees each need their own pin; missing pins fail closed.
A stale `target/debug` client fails with `target protocol mismatch`, a missing
XDG with `missing content file`: both are reported `unrunnable`, never red.

## Block-entity packet privacy

Chunk records and block-entity update packets must expose only the relevant
vanilla update-tag fields, not saved container contents, loot seeds, commands,
bees or unrelated components. Open container menus have their own viewer-scoped
content packets. Decorated pots intentionally withhold their item and loot data
even where vanilla sends it. The `mc-world` wire privacy and registry tests and
the `mc-edge` direct-packet tests cover the fields in their fixtures; they do not
establish exhaustive coverage of every nested value or every real-client path.

## Java parrot probe

The `java-parrot-jukebox` scenario uses the vanilla Java client's local
`isPartyParrot` state to check record start, stop, and leaving the jukebox
radius. Vanilla 26.1.2 uses level events rather than a dancing metadata
accessor; the probe requires the Loader Java-agent observation of that state.
It does not cover hostile-mob sound imitation or the native client.

## Native promotion probes

The `entities` profile includes windowed native-client probes:

- `native-v2-lease-navigation`: in a private operator-built fixture, a 120-second
  native action retains its controller, a cancelled writer is replaced without
  losing the replacement's held input, and navigation starts at a supported edge
  and traverses an open door from an offset pose. Cancellation scheduling is
  nondeterministic; deterministic owner-fence ordering remains a client unit test.

- `native-v2-end-restart`: enter the End on hard difficulty, identify the live
  dragon, switch to peaceful, save and restart, then require the same player,
  End world and dragon UUID. This covers live peaceful-fight persistence, not
  the dead-dragon branch or dragon model parity.
- `native-v2-entity-gallery`: remove a projectile during a mixed live-entity
  tick, then require the other fixture UUIDs and an ordered server response.
  Its negative control must show the original body-slot compaction panic,
  not merely a disconnected client.
- `native-v2-custody-mount-export`: create imported donkey cargo using the
  shipping export/import CLI, withdraw seven diamonds, deposit five emeralds,
  export/import again and assert both mount and player custody.
- `native-v2-p2-optional-pack`: observe a Configuration pack offer, a matching
  decline and Play on one error-free connection, then require playable chat.
- `native-v2-destroyed-chest`: records visible menu closure and custody after
  chest destruction. The MCP refusal to click an already-closed menu is not
  a stale server-packet test; this probe alone cannot close HS-1.
- `native-v2-destroyed-chest-race`: the HS-1 packet race. A second headless client populates the
  Nether so commands run through the shared (multi-world) path. In six rounds per branch the
  extraction or deposit click is issued right behind the destroying command, before the client has
  seen the menu close, with the simulation frozen. Whichever the server handles first, the one
  diamond must remain exactly once (inventory or ground) and the menu must be closed; a branch
  whose click never left the client fails the run. Custody is read from the final inventory and
  ground items, not from a server reply.
- `native-v2-duplicate-login`: a second headless native client of the same offline account
  joins while the first is in Play. The second reaches Play with the same player and exactly
  one diamond, the first leaves Play, and the first client's re-login replaces the second.
- `native-v2-auth-budget`: the shipping `krasnokamen-auth` package behind the native login gate.
  `/register`, and after a reconnect `/login`, run while the server's plugin worker thread shares
  one core with seven CPU hogs (the descheduling of a loaded host), so Argon2 takes several
  times its idle wall time. The plugin's reply must arrive and ordinary chat, refused while
  locked, must then be echoed. The unfixed host retires the plugin and kicks the player.
- `native-v2-corrupt-block-entities`: a jukebox, sculk shrieker and beacon, each alone in its own
  chunk, are saved, exported to Anvil, corrupted by one field in the exported copy, imported
  again and loaded beside the rejoining player. The server must refuse exactly the two
  undecodable chunks (one error naming each record), never panic, keep ticking past the beacon's
  80-tick pass and keep the player's own chunk. The checkpoint after the refusal and the shutdown
  checkpoint must both commit (one error lists every refused chunk), and a restarted server
  refuses the same chunks again. The beacon record is decodable by construction: that fixture is
  the runtime-path control and must not be refused. Crafter, shelf, shulker box and conduit
  records are validated by the importer through their block and never reach the server that way:
  a crafter and an oak shelf, each alone in its own chunk, instead reach it as forged opaque images
  in a tick-WAL batch (`wal_forge.py`) published over a clean shutdown checkpoint, under a foreign
  record id that replay's id-based validator does not check. The next start must refuse exactly
  those two chunks (one error naming each owner), never panic and keep ticking; a hydration owner
  that panics instead kills the tick thread before the server listens, which fails
  `server_starts_over_the_forged_wal_batch`. A conduit record has no such route (replay
  validation and `conduit::validate_loaded` refuse it at start, imports validate it), so
  CONDUIT-RUNTIME-PANIC has no check in this scenario that goes red under its fix.
- `native-v2-corrupt-chunk-near-camera`: known-red repro, not a gate. The same corruption in the
  player's own chunk keeps the native client on its loading screen.
- `native-v2-cursor-close`: pick up exactly three apples, close and reopen both
  ordinary inventory and crafting table, and require an empty received cursor
  before any repair click while retaining exactly three inventory apples.
- `native-v2-banner-placement`: ordinary placement transfers the red-cross /
  blue-stripe banner layers and both patterned/plain custom names into received
  block-entity NBT; the actual patterned and plain banners are captured for review.
- `native-v2-snow-golem-shear`: ordinary shears clear the same golem's received
  pumpkin flag, reveal its face and yield one carved-pumpkin pickup.
- `native-v2-peaceful-removal`: creates a zombie and cow with natural spawning
  disabled, switches to peaceful and checks that the same zombie disappears
  while the same cow remains. A graphical receipt and negative control are
  required before this scenario can support promotion.
- `native-v2-command-score-store`: checks generic damage, main-hand totem and
  custom death protection, writable death/player/total kill counters and a swap,
  stored result/success for give, clear and failed clear, and inventory counts.
  A graphical receipt and negative control are required before promotion.
- `native-v2-happy-ghast-harness`: an ordinary survival harness debit admits the
  graphical player and three independent native peers; a fifth peer is refused.
  Received passenger packets prove distinct seats and independent dismount.
  The public NBT view hides `krasnokamen:` keys, so the refusal is fenced by the
  fourth rider's later dismount in the same ordered passenger stream: no
  passenger event before it may carry a fifth id. The focused native
  checkpoint regression separately checks harness/rider custody on disk.
- `native-v2-mob-metadata-spot`: one frog, strider, camel and sniffer stand in
  frame on a private pad, a small slime is then eaten by the frog; the client
  must stay in play and its log must carry no decode failure (sniffer state is
  accessor 18; frog tongue, strider cold and camel dash are 19).
- `native-v2-p4-identities`: seven named `/damage` identities must reach the
  actual client's damage stream with health loss, followed by a lever-provoked
  warden's sonic attack through a preplaced wall, death screen and respawn. Command cases prove identity
  admission/emission, not freeze, wall-collision or cramming mechanics.
- `native-v2-p4-death-protection`: a held stick with nonempty custom consume
  actions saves the holder at one health, consumes one item, clears a preexisting
  slowness effect and applies authored speed without ordinary totem effects. The full P4 damage
  scenario still includes this check and the separate physical-source probes.
- `native-v2-team-deaths`: two windowed native clients (victim in team `red`,
  peer in team `blue`) read the chat their own client renders for a player
  `/kill` ("was killed", generic_kill rather than out_of_world), a zombie kill
  ("was slain by Zombie", the vanilla translation component rather than plain
  English text) and one death per `deathMessageVisibility` value (`always`,
  `never`, `hideForOtherTeams`, `hideForOwnTeam`); the victim's death screen
  is asserted by class and a retained capture, its message text is not exposed
  by the client observation.
- `native-v2-automation-storage`: the sculk latch section tests ordinary and
  calibrated sensors at multiple distances, including a rejected frequency.
  Comparators face their input: east toward the sensor for frequency readout,
  south toward the cake for calibration. Downward bulbs retain each pulse;
  the perpendicular bulb is a negative geometry control. The fixture supplies
  solid support beneath every component, removes ambient mobs and disables
  natural spawning so unrelated footsteps cannot toggle the latches. First
  bulbs must be unlit before the note is powered. The latch fixture runs
  unfrozen so scheduled comparator updates keep their ordinary timing; the
  preceding sensor probe observes active, then inactive (the ten-tick
  cooldown is asserted by the Rust sculk tests, not sampled by the client).
- `native-v2-map-*`: separate frame waypoint, banner removal/locked
  toggle, inventory clone tracking, pumpkin hiding, terrain shading and Nether
  terrain repros. Read-only receipts reconstruct the graphical viewer's exact
  map packets; screenshots still require review, especially marker visibility.
- `native-v2-input-menus`: physical number/F/Q/Ctrl-Q keys and sub-tick pointer
  edges in survival and creative inventory. Server snapshots check the source,
  reopened destination and survival carried stack. Creative cursors are local:
  captures must show the held stack, then its exact count must survive placement
  and reopening.
- `native-v2-input-pick`: physical middle-click and gameplay F in both modes;
  block/entity picking must not create free items in survival.
- `native-v2-input-camera`: frozen-world teleport and F1 hide/restore, checking
  captured camera and HUD pixels while the local client loop continues.
  `admin-commands` checks frozen/stepped server game time, not a stopped client
  clock: native input and rendering must keep running while the world is frozen.
- `native-v2-input-sand`: `/setblock` airborne sand in a frozen world becomes a
  rendered entity after `/tick step`, and the server holds it between steps:
  `data get entity … Pos` read in successive server ticks must stop changing
  (a server that keeps dropping it never does). The client predicts gravity
  between received position updates, then the sand lands and clears its source.
  Vanilla resends a moved falling block every 20 server ticks even while frozen,
  so the gate runs one tick per second, starts on a received update and steps
  twice in one command: the predicted position must equal vanilla's two-tick
  fall and the drawn pose must lie on that path, below the received one.
  Received, predicted and presented positions are distinct observations; only
  the captured frame and its reviewed pixels prove drawing.
- Axolotl play-dead and potion custom-impact probes are unregistered prototypes;
  they cannot be run from the harness, and `AXOLOTL-PLAY-DEAD` and
  `POTION-CUSTOM-PAYLOAD` stay open.

`MODEL-BANNER-PLACEMENT` covers the banner symptom only. The profiled-head
fixture still needs a verified listed player's published skin
(`MODEL-PLACED-HEAD-PROFILE`); decorated-pot presentation has a separate
string-versus-item-template sherd mismatch (`MODEL-PLACED-POT-DECORATIONS`).
Neither open split is closed by the banner's wire or screenshot assertions.

The shared native wire observer uses the pinned 26.3 packet IDs and vanilla
`ByteBufCodecs.BIT_SET` byte-array masks. Heightmap long arrays are a different
field. KeepAlive is not chunk data.

The operator-only `/debug menu` probe replies privately with one
`KRASNOKAMEN_MENU` JSON line containing the current container ID, carried stack,
and every menu slot. Each nonempty stack reports its item identifier, count and
complete serialized stack data under `components`; an empty stack has a null
item, zero count and null components. The image comes from server-owned menu
and backing storage, not client prediction. If the backing state cannot be
resolved, the command reports an error rather than inventing empty slots.
The craft-drag probes use ordered replies after pointer press, movement, release
and menu close; the crafting result slot is virtual and excluded from custody
totals. The paced Java and native probes also check right-button pickup and
release outside the menu. Screenshots remain necessary for visual review.

Client-tier scenarios run the native client at `KRASNOKAMEN_CLIENT_ROOT` (default:
the sibling `solaris-client` checkout).

A probe's registration or screenshot alone is not acceptance evidence; repeat
timing-variable probes at least three times before `ledger promote`.
An unchanged green pre-fix scene or a capture-only visual comparison is not a
negative control: the scenario must fail at the affected assertion. Written
visual review supplements that machine check; it does not replace it.
