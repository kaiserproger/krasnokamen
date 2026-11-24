# NATIVE-ACTIVE-WRITER-IDLE-EXPIRY — hunk map for the committed reversal pair

Client pair: reversal `client:2e4f355e59` → re-land `client:7ac3be5d7c` (the row's `fix_commit`).
Promotion receipt: `promotions/NATIVE-ACTIVE-WRITER-IDLE-EXPIRY.json` (the first promotion, pair
`0051108980` → `88a5f4f0c4`); this pair supersedes its control, whose kept `lease_verdict` arms 2-3
and `controller()` report were not the pre-image (review B-M1).

## Disposition classes (README `docs/bugs/README.md:222-235`)

- **(a)** another row's or owner's behaviour: its reversal would turn the control red on someone
  else's assertion;
- **(b)** API the kept code calls: reversing it stops the control from compiling;
- **(c)** a later commit rewrote that code, so no HEAD equivalent exists.

## The pair's own diff

```
git diff 2e4f355e59 7ac3be5d7c -- src/automation/control.rs
@@ -184,10 +184,16 @@ impl Core {
     pub fn controller(&self) -> Value {
         let c = self.control.lock();
+        let now = Instant::now();
         match &c.lease {
-            Some(l) if l.expires > Instant::now() => {
+            Some(l) if l.active_actions != 0 || l.expires > now => {
+                let remaining = if l.active_actions != 0 {
+                    WRITER_IDLE_TTL
+                } else {
+                    l.expires.saturating_duration_since(now)
+                };
                 json!({"active":true,"controller":l.controller,
-                "remaining_ms":l.expires.saturating_duration_since(Instant::now()).as_millis(),
+                "remaining_ms":remaining.as_millis(),
                 "scopes":["observe","control","capture"]})
@@ -319,14 +325,14 @@ impl Core {
         match &c.lease {
-            Some(l) if l.expires <= Instant::now() => {
+            Some(l) if l.active_actions == 0 && l.expires <= Instant::now() => {
@@
-            Some(l) => (None, Some(l.expires)),
-            None => (None, None),
+            Some(l) if l.active_actions == 0 => (None, Some(l.expires)),
+            Some(_) | None => (None, None),
```

`git diff 2e4f355e59 7ac3be5d7c` is 1 786 bytes and is byte-for-byte
`non_test_patch(client repository, 7ac3be5d7c)` (sha256 `9e426ef3c2bb0518…`). Every changed line's
`a/-` side is the historical pre-image (`git show 895c63f3^:src/automation/control.rs`). No
adaptation.

## Why a reversal and not the pre-image pin

`895c63f3` fixes five client rows at once, so `non_test_patch` of it is the whole 38-file change and
no single-row control can differ from the restore by that patch; its pre-image client cannot start in
the current private graph. The committed pair changes only this row's deadline rule and the two
places where the tip's `active_actions` bookkeeping had leaked into the pre-image.

## Every `src/automation/control.rs` hunk of `895c63f3`, line by line

| # | hunk (historical → tip) | changed lines | disposition |
|---|---|---|---|
| 1 | `@@ -7,6 +7,8 @@` | `+const WRITER_IDLE_TTL: Duration = Duration::from_secs(120);` (tip 11) | **KEPT (b)** — `Drop for WriterAction`, `admit`, `capture_fence` and the `controller()` report call it; reversing it stops the control from compiling |
| 2 | `@@ -23,6 +25,7 @@` | `+    pub active_actions: usize,` on `Lease` (tip 29) | **KEPT (b)** — every other hunk of this row and of later rows reads the field; removing it stops the control from compiling |
| 3 | `@@ -68,6 +71,32 @@` | `+pub(super) struct WriterAction<'a>` · `+impl Debug` · `+impl Drop` with `lease.active_actions -= 1;` (97) · `if lease.active_actions != 0 { return; }` (98-100) · `lease.expires = Instant::now() + WRITER_IDLE_TTL;` (101) · `notify()` | **KEPT (b)** — `admit` returns `WriterAction` and later code (`capture_fence`, the release path) constructs and drops it; reversing it stops the control from compiling |
| 4 | `@@ -154,10 +183,16 @@` | `+let now = Instant::now();` · `Some(l) if l.active_actions != 0 \|\| l.expires > now =>` · `let remaining = if l.active_actions != 0 { WRITER_IDLE_TTL } …` · `"remaining_ms": remaining.as_millis()` | **REVERSED** — see "The pair's own diff": the pre-image `controller()` is restored, so the control's report is the pre-image's, not the tip's |
| 5 | `@@ -170,7 +205,7 @@` | `-    ) -> Result<Arc<AtomicBool>, String> {` → `+    ) -> Result<WriterAction<'_>, String> {` | **KEPT (b)** — `admit`'s signature is called by the release path; reversing it stops the control from compiling |
| 6 | `@@ -191,7 +226,8 @@` | `-&& (lease.expires <= Instant::now() \|\| !lease.live…)` → `+&& ((lease.active_actions == 0 && lease.expires <= Instant::now()) \|\| !lease.live.load(…))` (262) | **KEPT (a)** — the reclaim condition of NATIVE-REPLACEMENT-INPUT-CLEARED / NATIVE-CANCELLED-OWNER-REVOKES-NEW-WRITER; reversing it changes their cases, not this row's deadline rule |
| 7 | `@@ -207,7 +243,9 @@` | `-if lease.expires <= Instant::now() \|\| !lease.live…` → `+if (lease.active_actions == 0 && lease.expires <= Instant::now()) \|\| !lease.live.load(…)` (279) | **KEPT (a)** — the re-admit expiry check of the same two rows |
| 8 | `@@ -218,22 +256,24 @@` | `-lease.expires = Instant::now() + Duration::from_secs(120); -return Ok(lease.live.clone());` → `+lease.active_actions += 1; +lease.expires = Instant::now() + WRITER_IDLE_TTL; +return Ok(WriterAction { core: self, fence: lease.live.clone() });` · new lease `+active_actions: 1,` · `expires:` uses the const · `-Ok(live)` → `+Ok(WriterAction { core: self, fence: live })` | **KEPT (b)** — `admit` returns `WriterAction`; reversing it stops the control from compiling |
| 9 | `@@ -246,14 +286,14 @@` (`lease_verdict`) | arm 1 `-Some(l) if l.expires <= Instant::now() =>` → `+Some(l) if l.active_actions == 0 && l.expires <= Instant::now() =>` (328) | **REVERSED** — the row's claim: only an idle lease is revoked at its deadline |
| 9 | same hunk, arms 2-3 | `-Some(l) => (None, Some(l.expires)), -None => (None, None),` → `+Some(l) if l.active_actions == 0 => (None, Some(l.expires)), +Some(_) \| None => (None, None),` (333-334) | **REVERSED** — the scheduling arms are part of the same pre-image: kept in the tip's form an active lease would fall into `(None, None)` and the watchdog would wait for `changed` with no timeout, a behaviour the pre-image never had (review B-M1) |

Hunks 1-3, 5 and 8 are kept only because the tip's later code calls them; their behaviour is
inherited from the same commit and is not part of this row's claim. The row's control now differs
from the pre-image in nothing: `lease_verdict`'s three arms and `controller()`'s report are the
historical text.

## Proving ref

`harness:entities:native-v2-lease-navigation` → `active_writer_survived_idle_deadline`, evaluated
before the scene's generic `long_action_completed`. A client that loses the controller at the idle
deadline never returns the bounded action, so the row's own assertion is the one that flips.

The scene's action is now **150 s** with `elapsed >= 125` and `join(165)` (review B-M2): the approach
used to be 120 s, exactly `WRITER_IDLE_TTL`, so a revocation and the action's own timeout were
indistinguishable. Controls: <control receipts, ≥3>; restore: <restore receipt>.

## Gate hygiene note (review N8)

The scene retries `minecraft_press_inputs` up to 25 times while the client answers
`writer_action_busy`; that retry can hide a *late* writer release (a handoff that only succeeds after
several decades of milliseconds). The run report keeps `replacement_press_errors` with the last
error when every attempt failed, but a slow unwind is not surfaced per attempt; the next scene that
measures handoff latency should record the attempt count.
