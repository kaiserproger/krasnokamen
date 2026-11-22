# CORE-DROP-RELEASES-LEASE-BEFORE-OWNERS-JOIN — control

[Control patch](../fix-patches/CORE-DROP-RELEASES-LEASE-BEFORE-OWNERS-JOIN.patch) for `7c1a96e92a6de2680bc8efe45bffec6e6d6adf86`.
`fix_commit = "patch:ac21dca494074f48dbc8ef64828f6745734d9989b4cbafbfd253601bd6758bb6"` names the canonical non-test patch (uncommitted fix).

## The fix

`CoreInner` had no `Drop`: a plain `drop(core)` detached the tick and journal `JoinHandle`s and closed the world lease `File` while an owner could still write. The patch moves the documented stop order from `Core::shutdown` into `CoreInner::stop_owners` (signal; join tick; join journal; release the lease) and runs it from `impl Drop for CoreInner`, so plain drop and explicit shutdown share one idempotent order.

Two hunks, one file (`crates/mc-core/src/core.rs`): `shutdown` delegates to `stop_owners`, and `impl CoreInner`/`impl Drop` are added. The stop order itself is unchanged; only its owner moved from the public handle to `CoreInner` so a destructor can run it.

## Kept out of the control — test wiring (category (b))

`OwnerAliveGuard`, the `#[cfg(test)] owners` field and its guards, the `#[cfg(test)] impl Core` probes (`owners_probe`, `shutdown_sender_probe`) and the `mod core_drop_join_tests;` registration are the reproducer's API. Reversing them would make the refs uncompilable or absent; only the two production order hunks are in the patch. The patch reverse-applies with `git apply -R --check` rc 0 on this tree.

## Proving assertion

- `cargo:mc-core:core::core_drop_join_tests::plain_drop_holds_the_world_lease_until_the_publication_commits` — `crates/mc-core/src/core/core_drop_join_tests.rs:157`. It installs a real path-scoped native checkpoint observer for this world, writes a block through the running core (join and `setblock` gated on published tick outputs), then drops the core on its own thread while the writer is held at `before_native_commit`. One unbounded channel multiplexes the exact terminal event: the tick owner's broadcast closing (`TickStreamClosed`) or the drop returning (`DropReturned`), with `mpsc::recv_timeout` bounds and no sleeps.
- Fixed path (`:252`): the tick owner's broadcast closes first while the publication is still held. Broadcast closure does not itself prove the tick join has completed. At that boundary the test probes the real world lease with `mc_world::storage::open_for_lease(dir).try_lock()` and requires `WouldBlock` — the lease is still held because the release follows the journal join. Only after the test releases the writer may the held checkpoint commit, the drop return, and the census `owners.wait_zero` (`:269`) confirm no owner outlived the joined drop. It then recovers the dimension state, loads the chunk, requires the saved block state, drops the store handle and reopens the world.
- Reversal (control): no `Drop`, so `DropReturned` arrives first while the publication is held; the test requires `try_lock()` to be `Ok` (`:305`, the lease was already free) and then panics naming the defect (`:317-319`). It releases the writer, witnesses the late commit, stops the retired owners through the retained `shutdown_sender_probe` sender and waits for their end, so no thread or world leaks.
- The live-owner census remains as a secondary implementation check only (`:269`), never the red.
