# SETBLOCK-SNBT — promotion patch mapping

- fix: `patch:43dcf616a9d89ec5444b0e93311f364ea0d3b0efa9f89339236702c1de6dd054` (patch mode; `docs/bugs/fix-patches/SETBLOCK-SNBT.patch`, digest recorded on the row)
- control: reversing those non-test hunks on the promoted HEAD (`0218c529e8d9`) made the row's own ref red (see `SETBLOCK-SNBT.json`).

## Reversed files

- `crates/mc-core/src/command_world.rs` — the fix's non-test hunk for this row, as it stands at HEAD.

## Omitted hunks and reasons

- Test files and inline `#[cfg(test)]` hunks: (b) they must stay so the control compiles and runs the ref.
- Other files of the original fix commit, if any: (a) another row's behaviour, or (c) a later commit rewrote that code so no HEAD equivalent exists.
