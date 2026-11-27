# Krasnokamen documentation

Krasnokamen 1.0.0-rc.5 "Pauline" is the current release candidate for
Krasnokamen 1.0, for Minecraft Java 26.3; 26.1.2 and 26.2 clients join through
the translation chain ([operating guide](OPERATING.md)). Start with the
[project README](../README.md) for installation and a source quick start and
[project status](STATUS.md) for priorities, open transitions and known gaps.

## Running and extending Krasnokamen

- [Operating guide](OPERATING.md): configuration, content, console, backups and world conversion.
- [Migration from vanilla or Paper](MIGRATION.md): configuration mapping, offline import, preservation limits, rollback.
- [Settlements player guide](SETTLEMENTS_GUIDE.md): `/settlement guide`, villager jobs, membership; per-domain pages under [settlements/](settlements/OWNER_DECISIONS.md).
- [Server-driven dialogs](DIALOGS.md): the `show_dialog` UI API and where clicks arrive.
- [Plugins](PLUGINS.md): Wasmtime package deployment and authoring.
- [Krasnokamen Loader](KRASNOKAMEN_LOADER.md): optional client adapters and client bundles.
- [Product contract](PROJECT_SPEC.md) and [village generation](VILLAGE_GENERATION.md).
- [Release notes](releases/README.md): one page per tag, not a configuration reference.

## Architecture

- [Architecture overview](ARCHITECTURE.md), [runtime map](architecture/runtime.md),
  [state ownership](architecture/state-ownership.md), [mutation paths](architecture/mutations.md),
  [persistence](architecture/persistence.md), [residency](architecture/residency.md),
  [settlements](architecture/settlements.md).
- [ADRs](decisions/README.md): binding decisions and their supersession status.
- Source-derived navigation: [crate graph](generated/crate-graph.md),
  [contract index](generated/contract-index.md), [scenario index](generated/scenario-index.md).
- [Settlements owner decisions](settlements/OWNER_DECISIONS.md): binding owner text.

## Development and validation

- [Contributing](../CONTRIBUTING.md) (prerequisites, build, L0–L2, pull requests)
  and [agent working rules](../AGENTS.md) with its task routing table.
- [Development playbooks](development/README.md): feature and protocol work, the
  owner-host runner, worktrees and integration, real-client gates, releases,
  test server, client QA, reviews, lessons.
- [Agent tooling and harness](AGENT_TOOLING.md): profiles, evidence boundaries,
  receipts, code-health ratchets, resource bounds.
- [Agent Constitution](AGENT_CONSTITUTION.md) (binding for every agent),
  [definition of done](DEFINITION_OF_DONE.md), [bug ledger](bugs/README.md) and
  [break-it QA protocol](QA_AGENT_PROTOCOL.md).
- [Vanilla compatibility matrix](COMPATIBILITY.md),
  [quirk and farm parity gate](QUIRK_PARITY.md), [physics oracles](physics-oracles.md).
- Performance: [reproducible profiles](performance/PROFILES.md),
  [benchmarks](performance/BENCHMARKS.md), [player capacity](performance/PLAYER_CAPACITY.md).
- Tool inputs: [playable scenarios](playable/README.md),
  [real-client regression manifests](real-client-regression/README.md),
  [worldgen evidence](evidence/worldgen-diff/README.md).

Retained because a tool still reads them, not as current acceptance:
[validation ledger](VALIDATION_LEDGER.md) (frozen v1 input to `coverage-audit`)
and the [core audit runbook](agents/PRO_CORE_AUDIT_RUNBOOK.md)
(`tools/prepare-pro-core-audit.sh`). Superseded designs are summarized in the
[archive](archive/ARCHIVE.md); their full text and all older documentation
remain in Git history. Local agent state lives in ignored `.analysis/`.
