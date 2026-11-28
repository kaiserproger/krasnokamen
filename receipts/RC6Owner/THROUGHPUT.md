# Release lane throughput — D149/D150

Final D150 update after native-03 completed at approximately 2026-11-28T08:37:34Z. No in-flight lane work remains; three distinct assigned gates passed.

| UTC window | Gates passed in window | In flight at end | Refused / failed attempt and reason |
|---|---|---|---|
| 2026-11-28 08:05:55–08:35:55 | **3: A player path, B restart/custody, C clean owner package with real native client**; native-02 completed exit 0 | **0** | Earlier immutable-pin rebuild refused (existing pin has different recorded build inputs); obeyed Main's retained-pin/no-rebuild decision. native-01 failed before client startup because runner selected a reports-only client directory as server content; corrected runner to retained server cache and native-02 passed. |
| D150 follow-up, through 2026-11-28 08:37:34 | **2 repeat gates: A and B PASS on mandatory sourced c879f0 client stand**, native-03 exit 0. C retained from native-02, not rerun; total remains **3 distinct gates**, not 5 | **0** | No refusal or failed attempt in native-03. No build: immutable e1f87c server and existing mandatory client stand used. |

- Gate D benchmark: **NOT RUN**, Main-owned and awaiting its registered quiet window; not carried as release-lane work.
- Existing loopback smoke: **PASS from prior receipt**, headless, not rerun and not counted as a new pass in this window.
- Earlier broader-scope installer passed; harness-check failed on inherited mimalloc stdout diagnostics; correctness was interrupted when Main narrowed scope. These are not silently counted as A–D passes.
- Gate table: `/run/media/kaiserroman/games/solaris-work/wt/D145Release/receipts/RC6Owner/RC6-GATE-STATUS.md`.
- Successful result: `/run/media/kaiserroman/games/solaris-work/wt/D145Release/receipts/RC6Owner/native-02/result.json`; SHA-256 `31e8771d43d987c58e487e099a535c63ab47c8a8b9c7d25e098a9e0bed15ed7b`.
- Historical C1 runner: `/run/media/kaiserroman/games/solaris-work/wt/D145Release/receipts/RC6Owner/native-02/runner.py`; SHA-256 `043b479516e5b108e98d4ccc15125b2da47712935b9bf26a7c667e684cfeadfd`. Current `/run/media/kaiserroman/games/solaris-work/wt/D145Release/receipts/RC6Owner/run_native_gates.py` now requires D150 stand exports and supports A–B-only execution.
- No zero-throughput window since this D149 reporting instruction arrived: all three assigned gates had just completed and are recorded now. No periodic background task is left running after completion.
- D150 A–B receipt: `/run/media/kaiserroman/games/solaris-work/wt/D145Release/receipts/RC6Owner/native-03/result.json`. Stand was sourced, no content/resource/asset paths overridden.
- D151 arrived after gate completion: no new Cargo/pin-release command, no build-admission wait, no other lane's process touched. Gate D stays outside this lane.
