# ADR 0005 - Regional simulation ownership

**Status:** Superseded by [ADR 0013](0013-core-model-v2.md) and [ADR 0014](0014-v1-core-retirement.md).

V1 partitioned moving entities into leased, single-writer regions and routed
cross-region operations through owner lanes. That runtime was retired; ADR 0013
defines current data-parallel native ownership and durability.
