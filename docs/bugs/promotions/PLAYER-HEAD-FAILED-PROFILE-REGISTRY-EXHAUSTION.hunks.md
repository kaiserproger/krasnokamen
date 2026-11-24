# PLAYER-HEAD-FAILED-PROFILE-REGISTRY-EXHAUSTION — hunk map for the tip reversal pair

Promotion receipt: `promotions/PLAYER-HEAD-FAILED-PROFILE-REGISTRY-EXHAUSTION.json`.
Client pair: reversal `client:f01f4330` → re-land `client:7d2e8212` (the row's `fix_commit`).
Both pins clean (`f01f4330a863`, `7d2e8212945b`); the pair's net tree is client `main` `530556e8`.

## Why the pair exists

The real fix is `client:19259ce8` ("fix: evict failed and least-recently-used skull profiles",
2026-11-15) — **after** the rename. The ledger could not use it directly because one of its hunks in
`src/session/profile_textures.rs` straddles that file's inline `#[cfg(test)]` module, which promote
refuses (`non_test_patch`). The pair therefore splits the fix into its production lines: the control
puts the pre-fix refusal back and the re-land restores the fix, so the pair's diff is exactly the
guarded behaviour the row names.

## Hunks of `19259ce8` and their disposition

`src/profile_textures/heads.rs`:

| hunk | content | disposition |
|---|---|---|
| `@@ -14,8 +14,19 @@ const MAX_HEADS` | `Entries.skins` becomes `Entry` (state/generation/last_access), `clock`, `enum State` | NOT carried — (b): the reversed refusal sits inside a `match entry.state`, so the reversed tree keeps these types and the kept registry tests construct them |
| `@@ -51,18 +62,32 @@ impl HeadTextures` | the lookup's `last_access`/`State` match **and** `if entries.skins.len() >= MAX_HEADS { return Err("head texture registry full") }` → LRU/failed-victim eviction + pending-queue cleanup, `Entry { state: Pending, generation: access, … }` insertion | **CARRIED in part** — the pair's `a/`-side is the refusal line exactly as `19259ce8^` had it, and its `+`-side is HEAD's eviction block (11 added, 1 replaced line); the rest of the hunk (lookup/insertion) is NOT carried — (b): the kept tests read `last_access`/`generation`, and reversing them without the reconciliation changes nothing the row asserts. The `+`-side is not byte-identical to `19259ce8`'s own lines: see "pair-only lines" below |
| `@@ -80,14 +105,35 @@ impl HeadTextures` | `retry()` re-queues with the request's generation and drops a superseded fence | NOT carried — (b): the kept test `head_registry_eviction_bounds_queue_and_fences_readmitted_keys` calls `retry` and asserts the fence, so the reversed tree keeps the body |
| `@@ -117,7 +163,7 @@ mod tests`, `@@ -130,13 +176,9 @@ mod tests`, `@@ -147,4 +189,62 @@ mod tests` | test-module fixtures and the two registry tests | test hunks (excluded from the non-test patch) |

`src/session/profile_textures.rs` (the file whose hunk straddles the test module):

| hunk | content | disposition |
|---|---|---|
| `@@ -79,7 +79,10 @@ impl Session` | on `Closed`/`Stale`, mark the head `fail(&key, generation)` | NOT carried — (a): this is the commit's *separate* late-completion fence, not the full-registry refusal the row names; its own assertion lives in the kept heads.rs fence test |
| `@@ -89,6 +92,22 @@ impl Session` | split `publish_profile_texture` into a wrapper that fails the head on an error plus `install_profile_texture` | NOT carried — (a), the same fence behaviour |
| `@@ -104,6 +123,12 @@ impl Session` | skip installing a completion whose head generation is no longer current | NOT carried — (a), the same fence behaviour |
| `@@ -127,7 +152,7 @@ impl Session` | the publish path's error plumbing that the split needs | NOT carried — (a)/(b): only the fence split calls it |
| `@@ -175,7 +200,7 @@ mod tests`, `@@ -266,4 +291,111 @@ mod tests` | test fixtures and the session fence test | test hunks; the second is the hunk that straddles the module boundary and is why the ledger refused `19259ce8` as `fix_commit` |

## Pair-only lines (Main's mechanical check: 9 of 12)

The pair's changed lines (`git diff f01f4330 7d2e8212 -U0`) are 11 added and 1 removed. Against
`19259ce8`'s own diff, 9 of the 12 are absent; `git blame -C -C 530556e8` names them all to one
commit:

| pair-only line (`src/profile_textures/heads.rs`) | blame commit | class |
|---|---|---|
| `let victim = *entries` / `.skins` / `.iter()` | `6bcb8cb2a` | (c) rustfmt |
| `.min_by_key(|(_, entry)| (!matches!(entry.state, State::Failed), entry.last_access))` | `6bcb8cb2a` | (c) rustfmt |
| `.expect("full registry has an eviction candidate")` / `.0;` | `6bcb8cb2a` | (c) rustfmt |
| `entries` / `.pending` / `.retain(|request| request.target != ProfileTextureTarget::Head(victim));` | `6bcb8cb2a` | (c) rustfmt |

`6bcb8cb2a` is "style(client): apply cargo fmt drift on four files" (2026-11-24): it reflowed
`19259ce8`'s eviction block (`min_by_key`, the `Entry { … }` insertion) without changing a token of
its logic, so the behaviour is identical and no second fix commit joins the map. The statement that
the `+`-side is "the eviction block verbatim" is therefore wrong as written: it is `19259ce8`'s block
in HEAD's rustfmt form.

## a/-side adaptation

None. The control's `a/`-side line is the byte-exact pre-fix refusal
(`return Err("head texture registry full".into())`) taken from `19259ce8^`; the re-land restores the
eviction behaviour in HEAD's (rustfmt-drifted) form. Everything else in the control tree is HEAD's.

## Proving ref

`harness:scene-shot:native-player-head-registry` → `valid_head_after_flood`
(`tools/harness/backends/scene_shot_checks.py`, `check_player_head_registry`): after 261 distinct
malformed `textures` values through nine visible hotbar slots, a valid head must change the avatar's
head region by more than 2000 px against the default-skin head. Measured: control `51` px (the same
skin — the registry refused the valid key), re-land `6765` px. The kept client unit ref
`profile_textures::heads::tests::head_registry_eviction_prefers_failed_then_least_recently_accessed`
covers the victim choice.
