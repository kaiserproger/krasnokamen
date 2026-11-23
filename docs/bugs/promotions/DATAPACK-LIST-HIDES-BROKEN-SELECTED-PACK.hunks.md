# DATAPACK-LIST-HIDES-BROKEN-SELECTED-PACK — promotion patch mapping

- fix: `patch:4b74ef2fefca7443e10acdf6e42eb5d4a52250cb4d7b4674b0c54b97dc5246fb` (patch mode; `docs/bugs/fix-patches/DATAPACK-LIST-HIDES-BROKEN-SELECTED-PACK.patch`, digest recorded on the row)
- control: reversing those non-test hunks on the promoted HEAD (`0218c529e8d9`) made the row's own ref red (see `DATAPACK-LIST-HIDES-BROKEN-SELECTED-PACK.json`).

## Reversed files

- `crates/mc-core/src/datapacks.rs` — the fix's non-test hunk for this row, as it stands at HEAD.

## Omitted hunks and reasons

- Test files and inline `#[cfg(test)]` hunks: (b) they must stay so the control compiles and runs the ref.
- Other files of the original fix commit, if any: (a) another row's behaviour, or (c) a later commit rewrote that code so no HEAD equivalent exists.
