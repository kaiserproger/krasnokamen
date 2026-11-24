# NATIVE-ACTIVE-WRITER-IDLE-EXPIRY — hunk map for the committed reversal pair

Promotion receipt: `promotions/NATIVE-ACTIVE-WRITER-IDLE-EXPIRY.json`.
Client pair: reversal `client:0051108980` → re-land `client:88a5f4f0c4` (the row's `fix_commit`).

## Why a reversal and not the pre-image pin

`895c63f3` is the historical fix of five rows at once, so `non_test_patch` of it is the whole 38-file
change and no single-row control can differ from the restore by that patch; its pre-image client is
also unpinnable in the private graph (wave 1). The committed reversal pair reverses the one hunk that
makes this row's claim and re-lands it.

## Original commit

`client:895c63f3` (this row's part is the `lease_verdict` arm below; the `active_actions`
bookkeeping the same commit added stays in place on both sides of the pair, so the control is a
working client whose only difference is the idle deadline rule).

## Hunks

| file | hunk | content | disposition |
|---|---|---|---|
| `src/automation/control.rs` | `@@ -286,14 +286,14 @@ impl Core` (`lease_verdict`) | revoke only when `l.active_actions == 0 && l.expires <= Instant::now()`; an admitted action keeps the fence and its idle TTL starts after the last response | REVERSED — the pre-fix arm `Some(l) if l.expires <= Instant::now() =>` is restored verbatim from `895c63f3^` |

## a/-side adaptation

None: the reversed arm is the historical pre-image line (`git show 895c63f3^:src/automation/control.rs`,
line 249), and `git diff 0051108980 88a5f4f0c4` is exactly this patch.

## Proving ref

`harness:entities:native-v2-lease-navigation` → `active_writer_survived_idle_deadline`, which the
scene now evaluates **before** its generic `long_action_completed` check (core `889f4346fb`): a client
that loses the controller at the deadline never returns the bounded action, so the row's own
assertion is the one that flips. Control receipt `…-y9_uwtlz` records the assertion false;
restore `…-fs_l96qn` passed.
