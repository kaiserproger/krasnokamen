# ADR 0009 - Regional simulation behind the plugin boundary

**Status:** Isolation, value and admission principles retained; v1 implementation
superseded by [ADR 0013](0013-core-model-v2.md) and [ADR 0014](0014-v1-core-retirement.md).

Plugins run isolated from native tick authority and do not own world state or
region leases. They receive bounded immutable values and submit capability-gated
commands for authoritative admission; observation is not a commit. Handlers run
serially, without exposing simulation locks or internal ownership to guests.

The former v1 regional, actor, journal and Luau implementation was retired.
[ADR 0013](0013-core-model-v2.md) owns native commit and plugin boundaries;
[PLUGINS.md](../PLUGINS.md) owns the current Wasmtime/WIT deployment, security,
permissions, execution budgets and required-auth failure contracts.
