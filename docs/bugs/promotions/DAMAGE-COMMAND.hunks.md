# DAMAGE-COMMAND — promotion patch mapping

- fix: `patch:ff8339d548af2b08289112d15575ac1aeff71dd04da4dfd85585245b9a18c6f5` (patch mode; `docs/bugs/fix-patches/DAMAGE-COMMAND.patch`, digest recorded on the row)
- control: reversing those non-test hunks on the promoted HEAD (`0218c529e8d9`) made the row's own ref red (see `DAMAGE-COMMAND.json`).

## Reversed files

- `crates/mc-core/src/commands.rs` — the fix's non-test hunk for this row, as it stands at HEAD.

## Omitted hunks and reasons

- Test files and inline `#[cfg(test)]` hunks: (b) they must stay so the control compiles and runs the ref.
- Other files of the original fix commit, if any: (a) another row's behaviour, or (c) a later commit rewrote that code so no HEAD equivalent exists.
