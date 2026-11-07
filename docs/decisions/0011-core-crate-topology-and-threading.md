# ADR 0011 - Core crate topology and threading model

**Status:** V1 topology, threading and durability superseded by [ADR 0013](0013-core-model-v2.md) and [ADR 0014](0014-v1-core-retirement.md).

V1 assigned core responsibilities to separate crates, dedicated threads and
multiple durable streams. The v1 runtime was retired; ADR 0013 owns the current
native model and ADR 0014 records the retirement.
