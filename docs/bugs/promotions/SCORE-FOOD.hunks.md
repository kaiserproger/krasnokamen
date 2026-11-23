# SCORE-FOOD — promotion patch mapping

- fix: `patch:73a2d42c2f1c0e6287e457bb9fa61fd7b3058ef35dc810c889c6e0d7e8cda669` (patch mode; `docs/bugs/fix-patches/SCORE-FOOD.patch`, digest recorded on the row)
- control: reversing those non-test hunks on the promoted HEAD (`0218c529e8d9`) made the row's own ref red (see `SCORE-FOOD.json`).

## Reversed files

- `crates/mc-core/src/scoreboard.rs` — the fix's non-test hunk for this row, as it stands at HEAD.

## Omitted hunks and reasons

- Test files and inline `#[cfg(test)]` hunks: (b) they must stay so the control compiles and runs the ref.
- Other files of the original fix commit, if any: (a) another row's behaviour, or (c) a later commit rewrote that code so no HEAD equivalent exists.
