# Krasnokamen documentation

Krasnokamen 1.0.0-rc.5 "Pauline" is the current release candidate for Krasnokamen 1.0,
for Minecraft Java 26.3 (26.1.2 clients can still join with a retained cache; see
the [operating guide](OPERATING.md)). Start with the [project README](../README.md)
for installation and a source quick start, and [project status](STATUS.md) for
priorities, the work queue and known gaps before 1.0.

## Running and extending Krasnokamen

- [Operating guide](OPERATING.md): configuration, content, console, backups and world conversion.
- [Migration from vanilla or Paper](MIGRATION.md): configuration mapping, offline import, preservation limits and rollback.
- [Settlements player guide](SETTLEMENTS_GUIDE.md): `/settlement guide`, villager jobs and membership.
- [Server-driven dialogs](DIALOGS.md): `show_dialog` UI API and where clicks arrive.
- [Plugins](PLUGINS.md): Wasmtime package deployment and authoring.
- [Krasnokamen Loader](KRASNOKAMEN_LOADER.md): optional client adapters and client bundles.
- [Project scope](PROJECT_SPEC.md) and [village generation](VILLAGE_GENERATION.md).
- [Release notes](releases/README.md): one page per tag, not current configuration reference.

## Architecture

- [Architecture overview](ARCHITECTURE.md), [runtime map](architecture/runtime.md),
  [state ownership](architecture/state-ownership.md), [mutation paths](architecture/mutations.md),
  [persistence](architecture/persistence.md), [residency](architecture/residency.md) and
  [settlements](architecture/settlements.md).
- [ADRs](decisions/README.md): binding decisions and explicit supersession status.
- [Generated crate graph](generated/crate-graph.md), [contract index](generated/contract-index.md)
  and [scenario index](generated/scenario-index.md): source-derived navigation.
- [Physics oracles](physics-oracles.md), [reproducible profiles](performance/PROFILES.md),
  [player capacity](performance/PLAYER_CAPACITY.md) and
  [weak-hardware / scaling / worldgen / VPS benchmarks](performance/BENCHMARKS.md).
- [Settlements owner decisions](settlements/OWNER_DECISIONS.md); per-domain settlement pages
  are listed in the [settlements player guide](SETTLEMENTS_GUIDE.md).
- [Archive](archive/ARCHIVE.md): superseded designs, old release notes, parked research.

## Development and validation

- [Contributing](../CONTRIBUTING.md) and [agent working rules](../AGENTS.md) (task routing table).
- [Agent tooling and harness](AGENT_TOOLING.md): commands, profiles and prerequisites.
- [Development playbooks and lessons](development/README.md): adding a feature, protocol changes, heavy-command runner, worktrees, gates, releases, test server, client QA.
- [Agent Constitution](AGENT_CONSTITUTION.md) (binding for every agent), [definition of done](DEFINITION_OF_DONE.md), [bug ledger](bugs/README.md) and [break-it QA protocol](QA_AGENT_PROTOCOL.md): breaker role, attack matrix, gate hygiene.
- [Quirk and farm parity gate](QUIRK_PARITY.md): vanilla-oracle proof that player-relied quirks and farm rates are preserved.
- [Vanilla compatibility matrix](COMPATIBILITY.md): 26.1.2 mechanics coverage per area.
- [Playable scenarios](playable/README.md) and [real-client regression manifests](real-client-regression/README.md).
- [Validation ledger](VALIDATION_LEDGER.md): retained input to `coverage_audit`, not a claim of current acceptance.
- [Optional core audit runbook](agents/PRO_CORE_AUDIT_RUNBOOK.md).

JSON files under `playable/` and `real-client-regression/manifests/`, generated
indexes, the validation ledger and referenced worldgen mosaic images are retained
because tooling consumes them. Historical process logs, old plans and audit dumps
are no longer published as current docs; archived versions remain in Git history.
Local agent state lives in ignored `.analysis/`, not in this documentation tree.
