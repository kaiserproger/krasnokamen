# CIVILIZATION-STREET-BUILDER-RETENTION — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIVILIZATION-STREET-BUILDER-RETENTION.patch`
(`patch:8c7807a9008fb24d…`), the row's `fix_commit`. It reverses on the committed
tree (`git apply -R --check`).

## What the fix is (origin `898af4725`, `settlements/construction.rs`)

A completed road or lamp (street work, `layout::infra`) releases its builder like
a finished furnace; before the fix only the furnace branch released, so street
work kept the only reachable adult bound as Builder and starved extraction and
wool.

## HEAD equivalent

HEAD restructured `finish_project` (project list, `access::take_project`, a
cloned project): the original pre-image ("release only for the furnace, complete
the infra project without releasing") no longer exists textually. The HEAD
equivalent keeps the completion path and only skips the release for non-furnace
infra:

```rust
if project.blueprint == furnace::NAME {
    release_builder(state, id, project.builder);
}
```

The other nine construction.rs hunks of the same slice are **omitted (a)**: they
are STREET-VERGE / EMPLOYED-SHEARING / INACTIVE-SHEEP / funded-kind behaviour.

## Reverse-apply evidence

`git apply -R --check` on the committed tree succeeds; the real-client red is the
integrated sweep's control run, not claimed here.
