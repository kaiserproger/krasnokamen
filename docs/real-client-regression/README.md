# Real-client regression manifests

A manifest specifies a scenario, not evidence that it passed. The server is
`mc-edge`'s `solaris-v2` over native `mc-core`. See the
[real-client gate playbook](../development/real-client-gates.md) for current
prerequisites, lifecycle and acceptance; protocol bots, mocks and prepared
artifacts are not real-client evidence.

## Manifests

- [`core-replay-seed-81.json`](manifests/core-replay-seed-81.json) is registered
  as `core-replay-seed-81` in the harness; its scenario is marked deferred by
  owner in the registry. Check `python3 -m tools.harness explain core-replay-seed-81`
  for current availability.
- The `wasm-p2-*` and `wasm-p3-*` manifests describe plugin scenarios. Their
  existence alone does not establish that a corresponding current gate passes;
  consult `python3 -m tools.harness list` for registered scenarios.
- [`m94-regression-pack.json`](manifests/m94-regression-pack.json) is the
  historical 26.1.2 M94 pack. The backend still uses it by default, but its
  old fixture assumptions and observations do not establish native-runtime
  acceptance. Some broad scenarios are intentionally blocked.

## Runner contract

The tracked backend is `tools/harness/backends/regression.sh`, reached through
`python3 -m tools.harness run regression`. Its `--check`, `--prepare`, `--run`
and `--validate-run DIR` modes are distinct: preparation is not a pass. The
backend selects the sibling loader's Gradle `:fabric-agent:runClientAgent` and
uses the in-client driver. `SOLARIS_LOADER_ROOT` selects the loader checkout;
`SOLARIS_REAL_CLIENT_MANIFEST`, `SOLARIS_REAL_CLIENT_AGENT_SCENARIO`, and
`SOLARIS_REAL_CLIENT_SERVER_CONFIG` select the manifest, scenario, and config.
The run requires a graphical display and rejects protocol bots and mocks.

A run records the manifest, server/client logs, observations and required PNG
screenshots in its local artifact directory. Validation checks artifact shape,
scenario IDs and screenshot structure; it does not authenticate manually edited
artifacts or judge the screenshot contents. A passing real-client result requires
an actual executed scenario and the corresponding assertions and review, not
just a bridge connection or screenshot. No-debug manifests must run without
operator grants. See the gate playbook for the current acceptance contract.
