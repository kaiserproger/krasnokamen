# SHUTDOWN-ERROR-LOG: patch-mode fix identity

Original fix: `3acf6374` "fix(runtime): distinguish orderly shutdown from owner failures".
`fix_commit = "patch:4e71a013fe840e68dfdc60984bc67d130cecabc43b28c84c77e8544f7d704af5"`
names that fix's log-level hunks on the candidate in
`docs/bugs/promotions/SHUTDOWN-ERROR-LOG.patch` (a diff from the pre-fix form
to the candidate; `git apply -R` restores ERROR on an orderly stop).
Re-audit:
`python3 -m tools.harness ledger promote SHUTDOWN-ERROR-LOG --fix-patch docs/bugs/promotions/SHUTDOWN-ERROR-LOG.patch --dry-run`

## Why not the commit itself

The fix is a commit, but its files moved: `crates/mc-core/src/core.rs` was
split into `core/journal.rs` + `core/tick_loop.rs` and
`crates/mc-edge/src/bin/solaris-v2.rs` became `.../bin/krasnokamen.rs`, so the
commit's hunks no longer apply in reverse on this tip and its historical tree
would not build with the kept refs. The carried patch is the HEAD equivalent of
each behavioural hunk; nothing else.

## Carried (4 behavioural flips, byte-identical messages and levels to the pre-image)

| original (3acf6374 a/-side) | HEAD |
|---|---|
| `core.rs` `send_shutdown`: `tracing::error!(tick, cause = "shutdown requested", "core tick owner stopping")` | `core/tick_loop.rs:695` `tracing::info!` |
| `core.rs` `journal_loop`: `tracing::error!(cause = "shutdown requested", "core journal owner stopped")` | `core/journal.rs:1057` `tracing::info!` |
| `bin/solaris-v2.rs`: `tracing::error!(cause = "edge serve loop returned", "server stopping")` and `tracing::error!(cause, "shutdown requested; stopping core")` | `bin/krasnokamen.rs:89,97` `info!` |
| `server.rs` broadcast closure: unconditional `tracing::error!(connections, "core tick broadcast closed; core halted or stopped")` | `server/pump.rs:224-235` `if hub.shutdown_requested() { info!(…) } else { error!(…) }` |

The broadcast hunk is the one structural difference: the pre-fix code logged
the closure unconditionally at error level with the quoted message, and the fix
replaced that single statement with the shutdown-request conditional. The
`a/`-side restores the pre-fix statement verbatim (message text and level) and
keeps the HEAD comment block above it as context; nothing is invented.

## Excluded

- `core.rs` `Core::shutdown_requested()` — (b) API: `server/pump.rs:224` and the
  kept tests (`core_stop_tests.rs:126-133`, `admin_tests.rs:1283`, pump tests)
  read it; reversing it stops the control from compiling. It is a pure getter
  and changes no behaviour on either side.
- `core.rs` `NativeWork::Shutdown`, the journal→native handoff, the
  `run_native_work` `ControlFlow` return and the writer's `Break` arm — (b) API:
  the kept refs `core::stop_tests::native_writer_explicit_shutdown_logs_info_without_waiting_for_channel_close`
  and `native_writer_channel_close_logs_stop_cause` construct `NativeWork::Shutdown`
  and call `native_writer_loop`/`run_native_work`; reversing removes them
  (E0425/E0308), so no assertion red is possible. This is also the site that
  removed the checkpoint writer's accidental ERROR: the pre-fix writer logged
  `tracing::error!(cause = "journal work channel closed", "native checkpoint writer stopped")`
  when the journal channel closed, which is still the HEAD text at
  `core/journal.rs:1149-1152` for a genuine channel close. Reproducing an ERROR
  on the *orderly* stop therefore needs this (b) machinery, not a level flip:
  the line was never the pre-image for the explicit-shutdown arm. The row's
  symptom is still red on the four carried flips (see below).
- `docs/decisions/0013-core-model-v2.md` — prose, not a control.

## Proving refs

- `cargo:mc-edge:console::tests::console_stop_has_no_error_logs_and_keeps_checkpoint`
  — red at `console_tests.rs:118-121` (`assert!(!log.contains("ERROR"))`) with
  the four flips reversed; the same real tick/journal child process, so the red
  is the row's own symptom.
- `cargo:mc-core:core::stop_tests::native_writer_explicit_shutdown_logs_info_without_waiting_for_channel_close`
  — not evidence here: the writer arm is (b) and stays on both sides, so this
  ref is green in the control (recorded as a green regression).
- `cargo:mc-core:core::stop_tests::native_writer_channel_close_logs_stop_cause`
  and `journal_io_failure_still_logs_error_and_never_acknowledges` — green on
  both sides (unexpected-failure paths are unchanged).
- `harness:entities:native-v2-console-shutdown`, proving assertion
  `shutdown_has_no_error`
  (`tools/harness/backends/native_promotion_world.py:31`): the real console
  `stop` log must contain no ERROR for the graphical native session.
