# Module extraction and architecture audits

Rules for where code belongs are in [AGENTS.md](../../AGENTS.md) "Project
invariants", the [Constitution](../AGENT_CONSTITUTION.md) and the ADRs. These
are the two procedures used for large Rust files and for architecture reviews
(2026-09/10). File names in older receipts refer to the retired v1 crates.

## Splitting a giant module or test root

1. **Pick one coherent slice** (one lifecycle or contract plus its private
   helpers), grouped by contract, not line count. Check who calls each symbol: a
   helper the parent still calls stays in the parent. Exclude shared routers
   and helpers from the slice.
2. **Compute exact spans, never hand-copy.** Script it: start at the first
   `///` or `#[` line above the item, end where brace depth returns to zero,
   assert balanced braces, assert each chunk occurs exactly once before removing
   it, delete spans in reverse order.
3. **Module shape.** `foo.rs` stays the module root with children in `foo/`
   declared by `mod bar;` (`foo.rs` plus `foo/mod.rs` is E0761). Moved methods
   keep their `impl` block but name the absolute type path
   (`impl crate::path::Type`), because `super` inside the child is `foo`.
   Rewrite `super::<sibling>::` to absolute paths. Child header: a short doc
   comment and `use super::*;`.
4. **Minimal visibility.** `pub(super)` only where the parent or a sibling child
   really uses an item; `pub(in crate::path)` for types the parent reaches
   through an existing `pub(super)` item; child-only helpers stay private.
5. **Tests.** Keep the big test root; move one contiguous block into
   `foo/bar_tests.rs` declared from the root with
   `#[path = "foo/bar_tests.rs"] mod bar_tests;` and `use super::*;`, so the
   child reuses the root's private fixture. A test move is byte-exact plus the
   header; never weaken or rename an assertion.
6. **Verify in order:** `cargo check -p <crate> --lib`, `cargo fmt --all`,
   `run fmt` and `run code-health`, the module's test count equal to before,
   affected harness scenarios, updated architecture `file:line` pointers, then
   one independent reviewer on the bounded diff.
7. **Report** shape (new files, line counts), moved symbols, visibility
   rationale and receipts. Never claim a compile-time improvement from a move:
   the crate is still one compilation unit. Measured on 2026-09-27, a focused
   test loop cost about 58 s of build and 0.4 s of tests; splitting test files
   does not speed rustc.

## Read-only architecture audit to ADR and plan

Docs-only; the code tree stays unchanged and no Cargo or client gates run.

1. **Frame it yourself:** read the architecture docs, ADR index and the prior
   program's explicit non-goals; list crate sizes and dependencies (a candidate
   list only).
2. **Fan out read-only scouts in one batch**, one per slice of the system
   (ingress, simulation owner, entities, plugin storage, composition/world,
   tests and tooling, threading and waiting). Shared rules: read-only;
   code-graph navigation first; a finding is a violated invariant plus
   `file:line` plus a reachable caller or consequence (lexical counts are only
   candidates); tag ownership/authority, coupling, threading/waiting/durability,
   or dead/duplicate; record strengths to keep; give a direction of fix only, no
   new buses, traits or crates without a concrete problem.
3. **Consume** each report into ignored provenance and spot-check two or three
   load-bearing claims yourself.
4. **Write, in order:** the evidence document (verdict, findings table,
   strengths), the ADR (dependency edges and absences, threading model, durable
   streams with the acknowledgement each needs, enforcement, rejected options,
   consequences), the plan (frozen and forbidden write sets, "move first, then
   change semantics", a move map, an ordered checkpoint queue with acceptance),
   then the links (architecture pointer, ADR index, superseded ADR headers) and
   a link check.
5. **One independent reviewer** with mandatory spot-checks of every blocking
   finding's cited lines; fix its findings; no second reviewer.

Traps reviewers caught: "module X does not import Y" is not a valid move
criterion (`use super::*` and transitive types hide dependencies); the criterion
is `cargo check -p <new crate>` after an explicit dependency cut, scheduled
before the move. Shutdown order: the final save flushes the journal, so the
journal writer is joined after it, not before. Ordinary world and entity
decisions are write-behind; only receipt-bearing operations, flushes and
checkpoints wait for fsync, so never write "fsync before ack" for every WAL.
Count durable streams by their journal files: plugin storage had its own.
