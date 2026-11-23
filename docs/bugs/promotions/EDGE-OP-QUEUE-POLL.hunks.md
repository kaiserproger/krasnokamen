# EDGE-OP-QUEUE-POLL — fix-hunk map

The production-only patch `EDGE-OP-QUEUE-POLL.patch` is the forward-ported fix at HEAD, SHA-256 of its
non-test hunks `6ab4698a88d665762a76db7825f191c1650846274f6b52e4e41aa166cc6c8baa`. It replaces the row's
recorded `fix-patches/EDGE-OP-QUEUE-POLL.patch` (`patch:a87aecbd…`), which no longer reverses at
`crates/mc-edge/src/server.rs` because the op/deop handler was rewritten by the later batch work.

| Original production change | Current patch equivalent |
|---|---|
| `d330c4a707a59e93d6d8dc28e1aecad94ddfd9b7` (`fix: address inventory and admission critique`): reserve the authoritative queue permits for the whole selected batch before `admin.set_op` mutates, and send each snapshot/permission message through its reserved permit instead of a `hub.submit` shed loop | Patch's only hunk, the `"op" \| "deop"` arm of `handle_admin_command`: a/-side drops the `reserve_inputs` and restores the historical `while !self.hub.submit(...) { sheds += 1; ensure!(sheds < MAX_SUBMIT_SHEDS) }` loops for the operator snapshot and each permission update (mutation first); b/-side is HEAD's reservation and `permits.next().expect(..).send(..)`. |

Original non-test exclusions, reason (a) — another row's behaviour: the batch authorization fence moved in
front of the mutation (`if id != … && (locked || !permitted)`) is `EDGE-QUEUED-OP-AFTER-DEOP`'s and
`EDGE-OP-JOIN-STALE-PERMISSION`'s work and stays on both sides. Reason (c): the original per-profile
`for (uuid, name) in profiles` loop was rewritten by that later batch work; the HEAD equivalent is the
`for (uuid, name, sessions) in updates` loop the patch keeps.

Proving ref: `cargo:mc-edge:server::admin_tests::operator_updates_wait_for_capacity_before_mutating` — it
parks both commands on a full queue and asserts `server.admin.level(uuid) == 0` while they are pending
(`crates/mc-edge/src/admin_tests.rs:176`); the reversed handler mutates before it waits and that assertion
fails. `cargo:mc-edge:server::admin_tests::queued_op_is_rejected_after_initiator_is_deopped` stays green on
both sides (the post-wait permission re-check is kept).
