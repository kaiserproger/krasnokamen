# SCORE-CRITERIA-WRITABLE — promotion patch mapping

- fix: `patch:49684fbf13f63f651932adb59ed33616b0f7178faac54b7af4ccc8182c5f7bf5` (patch mode; `docs/bugs/fix-patches/SCORE-CRITERIA-WRITABLE.patch`, digest recorded on the row)
- control: reversing those non-test hunks on the promoted HEAD (`0218c529e8d9`) made the row's own ref red (see `SCORE-CRITERIA-WRITABLE.json`).

## Reversed files

- `crates/mc-core/src/scoreboard.rs` — the fix's non-test hunk for this row, as it stands at HEAD.

## Omitted hunks and reasons

- Test files and inline `#[cfg(test)]` hunks: (b) they must stay so the control compiles and runs the ref.
- Other files of the original fix commit, if any: (a) another row's behaviour, or (c) a later commit rewrote that code so no HEAD equivalent exists.
