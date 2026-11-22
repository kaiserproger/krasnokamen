# Guide for coding agents working outside the owner host

Use this route when working from a fork on your own machine. The
[Agent Constitution](../AGENT_CONSTITUTION.md) and the shared rules in
[AGENTS.md](../../AGENTS.md) apply. Instructions marked *owner host* and
Main's integration, queue and publication duties do not apply to your fork.

## What this repository is

Start with the [project README](../../README.md) for the supported Java
releases and the difference between the core server and optional clients.

## Layout

The [architecture overview](../ARCHITECTURE.md) explains the native core and
edge; the [agent guide](../../AGENTS.md#where-to-look) maps the crates and tools.

## Where to look

Use the [documentation index](../README.md) for the owning guide. Check
[open transitions](../STATUS.md#open-transitions) before changing an area.

## Build and validation

Follow [CONTRIBUTING](../../CONTRIBUTING.md#build) to build the server and
import content into the checkout's `data/vanilla`, not the per-user operator
cache. Its [validation steps](../../CONTRIBUTING.md#validation) give L0, L1,
L2, the docs-only link check and the draft-PR path when L2 cannot run locally.
Read the [harness prerequisites](../AGENT_TOOLING.md#prerequisites) before
running a real-client gate; missing inputs are not passing evidence.

## Project invariants

Follow the [shared invariants](../../AGENTS.md#project-invariants), especially
the rules for Mojang data, native persistence and exact protocol evidence.
See [licensing](../../CONTRIBUTING.md#licence).

## Testing rules

Use [making a change](../../CONTRIBUTING.md#making-a-change) for regressions,
negative controls, real-client evidence and ledger rows. Leave `fix_commit`
to the maintainer for a fork PR; only `ledger promote` closes a bug row.

## Pull-request etiquette

Follow [pull requests](../../CONTRIBUTING.md#pull-requests). Work on a branch
in your fork and leave changes to upstream `main`, tags and releases to the
maintainer.

## If you are an agent: the constitution in one paragraph

Read the [full constitution](../AGENT_CONSTITUTION.md), not a paraphrase.
