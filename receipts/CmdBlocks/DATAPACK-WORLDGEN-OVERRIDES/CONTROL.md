# DATAPACK-WORLDGEN-OVERRIDES — control on bbf5b71e5

Fix identity `patch:05d3c72bf147c5be1c901d7c01aa9145caba07dad26d7579a5b253e09b0c858a`
(`docs/bugs/fix-patches/DATAPACK-WORLDGEN-OVERRIDES.patch`, one production
file). The patch collects every `ResourceError::Unsupported(entry)` of a pack
into one bounded list and refuses once; reversing it restores the pre-image
first-entry refusal.

- `git apply -R --check` passes at `bbf5b71e5`.
- RED (prior tree): `20261126T131704-test-2kqpijbt` — the test panics naming only
  `mod/dimension/a.json`, i.e. the first entry, exactly the row's symptom.
- RED on `bbf5b71e5`: `.analysis/validation/20261128T092127-test-afysgogm` FAIL,
  exit 100, the row's own loop assertion at
  `datapacks_worldgen_tests.rs:465` with the first-entry error
  `file/bad: mod/dimension/a.json: mod/dimension/a.json` — only the first
  unsupported entry named.
- GREEN: `.analysis/validation/20261128T092429-test-07wzg06v` PASS on this tree.

## Remaining gate

`harness:datapack-reload:native-datapack-reload` is a real-client gate (release
pin + windowed native client). It, not the cargo ref, is the receipt a
`repro = manual` row needs; that run is outside this worker's reach.
