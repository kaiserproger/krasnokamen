# HARNESS-AUDIBLE-SINK-IGNORED — fix-hunk map

Fix: `patch:b26ba4d3c876957a968830f9a94c6551845d9f17541f67e45131e8d8328d24e2`, the launch-options
hunk of the row's independent fix (`tools/harness/runtime.py`, the `start_client` options write).
The original patch carried two hunks of that file:

| original hunk | HEAD equivalent | kept? |
|---|---|---|
| `@@ -955,7 +955,10 @@` — `soundCategory_master: 1.0` and `soundDevice: <sink>` when `sound_sink is not None` | verbatim | **carried as a split** (see below) |
| `@@ -994,9 +997,12 @@` — `require_java_private_audio` branch replaces the unconditional isolation pair | verbatim | **omitted, reason (b)** |

**Split of hunk 1 (a/-side surviving half, reason (b)).** The carried patch reverses only the
`soundCategory_master` expression to its real pre-image (`"0.0"` unconditionally). The
`if sound_sink is not None: audio_options["soundDevice"] = sound_sink` lines stay: the kept
`require_java_private_audio` (hunk 2) refuses a Java launch whose OpenAL device is not the named
private sink, so a control tree without the `soundDevice` write fails at
`"FabricViewer OpenAL opened ['KrasnokamenAgentsNull'] instead of its own sink fabric_viewer_out"`
before any gate verdict — measured `20261126T175732-screens-voice-qa-2p4a8sey` (verdicts `{}`).
The a/-side line is the pre-image's own text; nothing else is adapted.

Omission of hunk 2 (reason (b)): the kept refs
`python:tools.harness.test_runtime_profiles:ProfileTaskTests.test_audible_client_without_private_graph_is_refused_before_spawn`
and `python:tools.harness.test_private_audio:PrivateAudioTest.test_audible_java_refuses_a_sink_outside_verified_graph`
call `require_java_private_audio`; reversing it removes the API and leaves them `unrunnable`
(`receipts/D123PackHarnessRows/control-audible-python.log`) instead of red. With it kept they stay
green on the control and are excluded from `proving_refs`.

## Control (measured on the candidate head, `wt/D123PackHarnessRowsCtlAudible`)

- `test_audible_client_uses_only_verified_named_private_sink` reds at its own assertion
  (`'soundCategory_master:1.0' not found in '…soundCategory_master:0.0…soundDevice:viewer'`); the two
  retained-API refs stay green.
- `harness:screens-voice-qa:fabric_loader_plays_screen_audio` must be red on the control: the Fabric
  viewer still opens `fabric_viewer_out`, but master volume is `0.0`, so its OpenAL output carries no
  tone (`silent(quiet) and heard(playing)` is false). On the candidate the same verdict is true
  (`20261126T173809-screens-voice-qa-fu2q437p`, both legs).
