# NATIVE-KICK-REASON-BROKEN-PIPE — hunk map for the committed reversal pair

Client pair: reversal `client:7c9e84721f` → re-land `client:900da09902` (the row's `fix_commit`).
Promotion receipt: `promotions/NATIVE-KICK-REASON-BROKEN-PIPE.json`.

The row is closed by **two** client commits, because the first one alone still lost the reason in a
narrower race (measured, see "Why the second commit"):

| fix | what it closes |
|---|---|
| `client:bf55a58f` `fix(client): keep the server kick reason over a failed write` | the I/O owner returned its own outbound write failure at the top of the loop and during packet handling, before a Play Disconnect already in the receive buffer was decoded — the reported "Broken pipe (os error 32)" |
| `client:be484c28` `fix(client): keep a kick that arrives as the writer ends` | a session-side `Network::send` that races the I/O owner's exit on the kick packet reported "outbound queue closed: network thread ended" before the next frame drained the published reason |

## Why a reversal and not the pre-image pin

`bf55a58f` is post-rename and its non-test patch reverses on the wave-2 client tip, but the row is a
client surface: `ledger promote` for `client:<sha>` needs a real-client red/green pair, and no client
before the wave-2 tip is pinned for this lane. The committed pair keeps the tip's code and changes
only the I/O owner's write-failure handling and the send-side precedence.

## The pair's own diff

`git diff 7c9e84721422 900da0990290` is 13 414 bytes. Its changed-line multiset equals the union of
the two origin patches: `non_test_patch(bf55a58f)` (patch-id `50f53aed…`, the `network.rs:228-249`
kick-reason latch) and `non_test_patch(be484c28)` (patch-id `4e0a2609…`, hunks `338+` and `554-783`,
the closed-queue-with-terminal-packet arm). The hunks do not overlap, and `git diff` between the
sorted changed lines of the pair and of the union is empty; the pair's own patch-id
`766b0865…` differs from the union's `9eff60f6…` only because the pair orders the `be484c28`
hunk before the second half of the `bf55a58f` hunk. Comparing the pair against
`non_test_patch(900da09902)` would be the commit against itself and is not evidence.
`non_test_patch(client repository, 900da09902)` (checked locally: equal strings, sha256
`f6c010c3759b4c6b…`). The pair's two commits:

- `7c9e84721f` `test(client): revert the kick-reason latch and its terminal precedence` —
  `git apply -R` of that patch (37 insertions, 107 deletions).
- `900da09902` `fix(client): keep the server kick reason over a failed write and a closing writer
  (re-land)` — `git apply` of the same patch.

Every hunk's `a/-` side is the historical pre-image or the pre-follow-up tip: the reversal applies at
the tip with intact context (`git apply -R --check` clean), so no line was hand-written and there is
no adaptation.

## Hunks — `src/protocol/network.rs`

### `bf55a58f` (10 hunks, all REVERSED)

| # | hunk (`-`/`+` header) | tip lines | content |
|---|---|---|---|
| 1 | `@@ -331,17 +338,37 @@` | 338-374 | `flush_outbound` returns `()` and takes `&mut Option<String> failure`; a new `send` latches the first write error instead of returning it |
| 2 | `@@ -527,8 +554,17 @@` | 554-570 | `let mut write_failure: Option<String> = None;`; the top-of-loop `flush_outbound(...)` no longer `?` |
| 3 | `@@ -536,7 +572,13 @@` | 572-584 | the periodic `CLIENT_INFORMATION` write goes through `send` |
| 4 | `@@ -545,7 +587,13 @@` | 587-599 | the per-packet `flush_outbound(...)` inside the extract loop no longer `?` |
| 5 | `@@ -558,9 +606,9 @@` | 606-614 | `LOGIN_FINISHED`'s acknowledged/`CLIENT_INFORMATION` writes go through `send` |
| 6 | `@@ -570,7 +618,7 @@` | 618-624 | `CUSTOM_QUERY_ANSWER` goes through `send` |
| 7 | `@@ -578,7 +626,7 @@` | 626-632 | Login `COOKIE_RESPONSE` goes through `send` |
| 8 | `@@ -587,44 +635,49 @@` | 635-683 | Configuration phase: `FINISH_CONFIGURATION`, `KEEP_ALIVE`, `PING`, `SELECT_KNOWN_PACKS`, the Loader probe payload and `COOKIE_RESPONSE` go through `send` |
| 9 | `@@ -632,41 +685,49 @@` | 685-733 | Play phase: `KEEP_ALIVE`, `PING`, `START_CONFIGURATION`'s acknowledgement and `CLIENT_INFORMATION` go through `send` |
| 10 | `@@ -702,12 +763,21 @@` | 763-783 | the Windows `try_read` error and the Unix `Ok(0)` EOF keep a latched write message instead of reporting the close alone |

### `be484c28` (1 hunk, REVERSED)

| # | hunk (`-`/`+` header) | tip lines | content |
|---|---|---|---|
| 11 | `@@ -228,14 +228,21 @@ impl Network` | 228-249 | `Network::send`: a closed outgoing queue with a published terminal packet in `urgent` is not a send error (the command is dropped with the writer); only a closed queue without a reason reports `terminal_error` or "outbound queue closed: network thread ended" |

Test hunks of both commits are **KEPT** by both pair commits: `bf55a58f`'s
`kick_reason_survives_a_failed_client_write` and `be484c28`'s
`kick_published_before_the_writer_ends_is_not_replaced_by_a_queue_error` /
`a_send_after_a_silent_writer_exit_reports_the_closed_queue`. `non_test_patch` excludes them from
the pair's diff, so the reversal leaves them in the tree, where the first two are red.

## Why the second commit (measured)

The first promotion attempt used the `bf55a58f`-only pair (`bcb316dd1f` → `eee240792a`) at core
`97e82ca79` with the registered server pin. The control red the row's own assertion with exactly the
reported symptom, and the restore — the fixed client — failed it too:

| run | client | `nether_kick.disconnect_reason` | assertion |
|---|---|---|---|
| `20261124T164519-native-v2-duplicate-login-wtpb71kj` | `bcb316dd1f` (control) | `Broken pipe (os error 32)` | `kick_reason_preserved` false |
| `20261124T164600-native-v2-duplicate-login-1pj1krt9` | `eee240792a` (restore) | `outbound queue closed: network thread ended` | `kick_reason_preserved` false |

The server had flushed the reason in both runs
(`server.log`: `core-ordered disconnect flushed … reason=You logged in from another location`), so the
remaining loss was the client's own send text winning the race — `be484c28` fixes that, and its
regression test reds on the reversed hunk with that exact text
(`src/protocol/network.rs:866`, "the published kick is the outcome, not a queue error").

## Proving ref

`harness:entities:native-v2-duplicate-login` → `kick_reason_preserved` (row `proving_assertions`).
The scene's default path teleports the incumbent to the Nether, replaces the session with a second
headless native client of the same account, waits (bounded 120 s) for the client's own `error` phase
and reads the kicked client's `disconnect_reason`; `kick_reason_not_broken_pipe` follows it. A
transient leave of Play is not read as the kick.

## Gate receipts

Promoted at core `0ccfdd946` with server pin
`pins/0ccfdd946084-server-x86_64-unknown-linux-gnu-incr`; promotion artifact
`20261124T174449-ledger-promote-e9zcqgs3` (fix patch-id `766b086534bbe0a6…`).

| run | client | `nether_kick.disconnect_reason` | assertion |
|---|---|---|---|
| `20261124T173901-native-v2-duplicate-login-ox4g1dzs` | `7c9e84721f` (control) | the reason | passed |
| `20261124T173938-native-v2-duplicate-login-ne0whc4n` | `7c9e84721f` (control) | the reason | passed |
| `20261124T174024-native-v2-duplicate-login-4wzjrf_i` | `7c9e84721f` (control) | `Broken pipe (os error 32)` | `kick_reason_preserved` **false** |
| `20261124T174056-native-v2-duplicate-login-8kja4dw0` | `900da09902` (restore) | the reason | passed |

The first two control runs are why the leg now starts the replacement during the travel burst
(`d2c6fdf35`): with the destination world settled, the pre-fix client delivered the reason in 5 of 5
runs and the control stopped discriminating.
