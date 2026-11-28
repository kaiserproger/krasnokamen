# D155 outcome — MODEL-OPUS-N3 is open, not restored from published history

Owner D155 (2026-11-28 18:15) rejected the closure on two grounds; both were checked here
before acting.

## What was wrong

1. `docs/bugs/promotions/MODEL-OPUS-N3.hunks.md` never existed (247 other promotions have
   their map). README:309-317 makes it mandatory for a historical client reversal.
2. `git -C solaris-client merge-base --is-ancestor bc7429a36c29 origin/main` → false. The
   measured restore was a local commit, so the row would have claimed "restored from main"
   without a published restore.

## What the row says now

Status `open`, `fix_commit = "client:e8fe40c2"` — a commit that *is* in client
`origin/main` and carries the same hat asset — with the measured pair kept as diagnostic
evidence only. The invalid promotion record is preserved as
`receipts/D155/MODEL-OPUS-N3.INVALID-PROMOTION.json`.

## Why a published restore could not be measured today

- `pin-release --client` on `e8fe40c20f6b` exits 1: "is not a krasnokamen-client tree;
  omit --client for the server" — that tree's `Cargo.toml` names the package
  `solaris-client`, the rename to `krasnokamen-client` came later. So no published client
  pin exists for the historical fix.
- Independently, `ledger._check_client_control_delta` (`tools/harness/ledger.py:2251-2268`)
  requires the control→restore production patch to have the *same patch-id* as the fix's
  own non-test patch. A historical bulk commit that also moves binary assets cannot satisfy
  that against a single-file control, whatever pin exists.
- The valid `035c0dadf4e1` → `bc7429a36c29` pair stays historical evidence: it reds
  `placed_head_hat_layer_pixels` exactly, but its restore is not published.

## Count

`fixed` went 554 → **553** when the row was reopened; that is the state on `main` after
`b5dafb4bb`. No other closure moved in that window.
