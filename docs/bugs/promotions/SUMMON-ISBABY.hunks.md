# SUMMON-ISBABY — promotion patch mapping

- fix: `patch:1d24d42295a3d1140a14a4109683c73aea94df4fe083c22efe1669c4f4cf88ac` (patch mode; `docs/bugs/fix-patches/SUMMON-ISBABY.patch`, digest recorded on the row)
- control: reversing those non-test hunks on the promoted HEAD (`0218c529e8d9`) made the row's own ref red (see `SUMMON-ISBABY.json`).

## Reversed files

- `crates/mc-core/src/command_summon.rs` — the fix's non-test hunk for this row, as it stands at HEAD.

## Omitted hunks and reasons

- Test files and inline `#[cfg(test)]` hunks: (b) they must stay so the control compiles and runs the ref.
- Other files of the original fix commit, if any: (a) another row's behaviour, or (c) a later commit rewrote that code so no HEAD equivalent exists.
