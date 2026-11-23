# DATAPACK-WARNINGS-CHAT-FLOOD — promotion patch mapping

- fix: `patch:7b0fdad349e409f18ff3f8c5bfd69f20a84bb80e79befdebf55e98e59ca7c485` (patch mode; `docs/bugs/fix-patches/DATAPACK-WARNINGS-CHAT-FLOOD.patch`, digest recorded on the row)
- control: reversing those non-test hunks on the promoted HEAD (`0218c529e8d9`) made the row's own ref red (see `DATAPACK-WARNINGS-CHAT-FLOOD.json`).

## Reversed files

- `crates/mc-core/src/datapacks.rs` — the fix's non-test hunk for this row, as it stands at HEAD.

## Omitted hunks and reasons

- Test files and inline `#[cfg(test)]` hunks: (b) they must stay so the control compiles and runs the ref.
- Other files of the original fix commit, if any: (a) another row's behaviour, or (c) a later commit rewrote that code so no HEAD equivalent exists.
