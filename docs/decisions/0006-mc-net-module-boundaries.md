# ADR 0006 - mc-net module boundaries

**Status:** Superseded by [ADR 0013](0013-core-model-v2.md) and [ADR 0014](0014-v1-core-retirement.md).

V1 organized `mc-net` as a modular monolith split by gameplay domain with
one-way dependencies and post-commit publication. The crate was retired;
ADR 0013 defines the current core and edge boundaries.
