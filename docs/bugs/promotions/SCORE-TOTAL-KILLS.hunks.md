# SCORE-TOTAL-KILLS — promotion patch mapping

- fix: `patch:f8dd23107df8d86196bd9f2eb2526329eb997ef64e6e3a82573f2dc6d3c1ac0b` (patch mode; `docs/bugs/fix-patches/SCORE-TOTAL-KILLS.patch`, digest recorded on the row)
- control: reversing those non-test hunks on the promoted HEAD (`0218c529e8d9`) made the row's own ref red (see `SCORE-TOTAL-KILLS.json`).

## Reversed files

- `crates/mc-core/src/scoreboard.rs` — the fix's non-test hunk for this row, as it stands at HEAD.
- `crates/mc-core/src/statistics.rs` — the fix's non-test hunk for this row, as it stands at HEAD.

## Omitted hunks and reasons

- Test files and inline `#[cfg(test)]` hunks: (b) they must stay so the control compiles and runs the ref.
- Other files of the original fix commit, if any: (a) another row's behaviour, or (c) a later commit rewrote that code so no HEAD equivalent exists.
