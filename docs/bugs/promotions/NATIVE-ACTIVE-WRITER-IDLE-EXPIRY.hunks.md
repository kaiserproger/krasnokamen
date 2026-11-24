# NATIVE-ACTIVE-WRITER-IDLE-EXPIRY — hunk map for the committed reversal pair

Promotion receipt: `promotions/NATIVE-ACTIVE-WRITER-IDLE-EXPIRY.json`.
Client pair: reversal `client:0051108980` → re-land `client:88a5f4f0c4` (the row's `fix_commit`).

## The pair's own diff

```
git diff 0051108980 88a5f4f0c4 -- src/automation/control.rs
@@ -325,7 +325,7 @@ impl Core {
-            Some(l) if l.expires <= Instant::now() => {
+            Some(l) if l.active_actions == 0 && l.expires <= Instant::now() => {
```

One line each way, at tip line 328 (`Core::lease_verdict`), inside the historical hunk
`@@ -246,14 +286,14 @@ impl Core` of `895c63f3`. The `a/-` side is the historical line
`git show 895c63f3^:src/automation/control.rs` line 249, byte for byte (no adaptation).

## Why a reversal and not the pre-image pin

`895c63f3` is the historical fix of five client rows at once, so `non_test_patch` of it is the whole
38-file change and no single-row control can differ from the restore by that patch; its pre-image
client cannot start in the current private graph. The committed reversal pair changes only the row's
own deadline rule.

## Every `src/automation/control.rs` hunk of `895c63f3`, line by line

Disposition classes: **(a)** kept because later tip code depends on it — reversing it alone would not
compile or would change every action's lifetime rather than this row's deadline rule; **(b)**
reversed: the row's own claim; **(c)** kept: the line is another row's fix or its bookkeeping.

| # | hunk (historical → tip) | changed lines | disposition |
|---|---|---|---|
| 1 | `@@ -7,6 +7,8 @@` | `+const WRITER_IDLE_TTL: Duration = Duration::from_secs(120);` (tip 11) | **KEPT (a)** — used by `Drop for WriterAction`, `admit` and `capture_fence`; reversing it alone does not compile |
| 2 | `@@ -23,6 +25,7 @@` | `+    pub active_actions: usize,` on `Lease` (tip 29) | **KEPT (a)** — the field every other hunk of this row and of later rows reads |
| 3 | `@@ -68,6 +71,32 @@` | `+pub(super) struct WriterAction<'a>` · `+impl Debug` · `+impl Drop` with `lease.active_actions -= 1;` (97) · `if lease.active_actions != 0 { return; }` (98-100) · `lease.expires = Instant::now() + WRITER_IDLE_TTL;` (101) · `notify()` | **KEPT (a)** — this is the TTL reset on the **last response**; the row's control does not reverse it, and `capture_fence` (a later row's code) reads `active_actions`, so reversing it would not compile |
| 4 | `@@ -154,10 +183,16 @@` | `+let now = Instant::now();` · `Some(l) if l.active_actions != 0 \|\| l.expires > now =>` (189) · `let remaining = if l.active_actions != 0 { WRITER_IDLE_TTL } …` (190-194) · `"remaining_ms": remaining.as_millis()` | **KEPT (a)** — the observation side: the control must still report a truthful controller, otherwise the assertion would measure the reporting instead of the deadline rule |
| 5 | `@@ -170,7 +205,7 @@` | `-    ) -> Result<Arc<AtomicBool>, String> {` → `+    ) -> Result<WriterAction<'_>, String> {` | **KEPT (a)** — compile support for hunk 3 |
| 6 | `@@ -191,7 +226,8 @@` | `-&& (lease.expires <= Instant::now() \|\| !lease.live…)` → `+&& ((lease.active_actions == 0 && lease.expires <= Instant::now()) \|\| !lease.live.load(…))` (262) | **KEPT (a)** — the reclaim condition of the same mechanism; reversing it alone changes reclaim for other rows' cases |
| 7 | `@@ -207,7 +243,9 @@` | `-if lease.expires <= Instant::now() \|\| !lease.live…` → `+if (lease.active_actions == 0 && lease.expires <= Instant::now()) \|\| !lease.live.load(…)` (279) | **KEPT (a)** — the re-admit expiry check, same helper |
| 8 | `@@ -218,22 +256,24 @@` | `-lease.expires = Instant::now() + Duration::from_secs(120); -return Ok(lease.live.clone());` → `+lease.active_actions += 1; +lease.expires = Instant::now() + WRITER_IDLE_TTL; +return Ok(WriterAction { core: self, fence: lease.live.clone() });` (292-294) · new lease `+active_actions: 1,` (305) · `expires:` uses the const (303) · `-Ok(live)` → `+Ok(WriterAction { core: self, fence: live })` | **KEPT (a)** — bookkeeping that every later row's code and the whole action lifetime depend on |
| 9 | `@@ -246,14 +286,14 @@` (tip 325-335, `lease_verdict`) | arm 1 `-Some(l) if l.expires <= Instant::now() =>` → `+Some(l) if l.active_actions == 0 && l.expires <= Instant::now() =>` (328) | **REVERSED (b)** — the row's claim: only an idle lease is revoked at its deadline |
| 9 | same hunk, arms 2-3 | `-Some(l) => (None, Some(l.expires)), -None => (None, None),` → `+Some(l) if l.active_actions == 0 => (None, Some(l.expires)), +Some(_) \| None => (None, None),` (333-334) | **KEPT (a)** — the scheduling arms of the same match; they compile with either arm 1 and keeping them leaves the control's only difference at arm 1 |

No other file is touched by the pair, and `git diff 0051108980 88a5f4f0c4` is exactly the one line
above.

## Proving ref

`harness:entities:native-v2-lease-navigation` → `active_writer_survived_idle_deadline`, evaluated
before the scene's generic `long_action_completed` (core `889f4346fb`): a client that loses the
controller at the deadline never returns the bounded action, so the row's own assertion is the one
that flips. Control `20261124T120447-native-v2-lease-navigation-y9_uwtlz` records it false; restore
`20261124T120206-native-v2-lease-navigation-fs_l96qn` passed.

## Gate hygiene note (review N8)

The scene retries `minecraft_press_inputs` up to 25 times while the client answers
`writer_action_busy`; that retry can hide a *late* writer release (a handoff that only succeeds after
several decades of milliseconds). The run report keeps `replacement_press_errors` with the last
error when every attempt failed, but a slow unwind is not surfaced per attempt; the next scene that
measures handoff latency should record the attempt count.
