# SUMMON-AGE — promotion patch mapping

- fix: `patch:c9e6a2f3467c0e78076dea00299c9d6e1693d5eed3a065324d36e1bb3d60636f` (patch mode; `docs/bugs/fix-patches/SUMMON-AGE.patch`, digest recorded on the row)
- control: reversing those non-test hunks on the promoted HEAD (`0218c529e8d9`) made the row's own ref red (see `SUMMON-AGE.json`).

## Reversed files

- `crates/mc-core/src/entity_life.rs` — the fix's non-test hunk for this row, as it stands at HEAD.

## Omitted hunks and reasons

- Test files and inline `#[cfg(test)]` hunks: (b) they must stay so the control compiles and runs the ref.
- Other files of the original fix commit, if any: (a) another row's behaviour, or (c) a later commit rewrote that code so no HEAD equivalent exists.
