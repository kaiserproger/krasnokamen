# ADR 0003 - Runtime world lock architecture

**Status:** Superseded by [ADR 0013](0013-core-model-v2.md) and [ADR 0014](0014-v1-core-retirement.md).

The v1 runtime retained a shared world lock with explicit lock ordering as a
transitional authority for world access. The implementation was retired with v1;
current native tick authority and persistence are defined by ADR 0013.
