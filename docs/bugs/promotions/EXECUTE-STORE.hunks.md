# EXECUTE-STORE — promotion patch mapping

- fix: `patch:ddb8f835cbaeb31923bef075d0c9fa05d057fb858c6628eaee1828d278ef94bd` (patch mode; `docs/bugs/fix-patches/EXECUTE-STORE.patch`, digest recorded on the row)
- control: reversing those non-test hunks on the promoted HEAD (`0218c529e8d9`) made the row's own ref red (see `EXECUTE-STORE.json`).

## Reversed files

- `crates/mc-core/src/command_execution.rs` — the fix's non-test hunk for this row, as it stands at HEAD.

## Omitted hunks and reasons

- Test files and inline `#[cfg(test)]` hunks: (b) they must stay so the control compiles and runs the ref.
- Other files of the original fix commit, if any: (a) another row's behaviour, or (c) a later commit rewrote that code so no HEAD equivalent exists.
