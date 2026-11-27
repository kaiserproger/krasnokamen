# Long decision records

The decision table is [DECISIONS.md](../../DECISIONS.md): one row per choice
with its reason, grouped by area. This folder holds only the records whose
design needs more than a row; each is linked from its row.

| Record | Rows |
| --- | --- |
| [0010](0010-krasnokamen-loader-contract.md) one untrusted client-bundle contract | C5 |
| [0013](0013-core-model-v2.md) native core model v2 | K1–K4, W1–W3, R1 |
| [0015](0015-multiversion-protocol-translation.md) adjacent protocol translation | P1, C4 |
| [0016](0016-mob-capabilities.md) mob capabilities and blueprints | M1, M2 |
| [0017](0017-tick-system-graph.md) tick system graph (proposed) | K8 |
| [0018](0018-in-game-computer.md) in-game computer | E1 |
| [0019](0019-voice-chat.md) proximity voice chat | E2 |
| [0020](0020-entity-checkpoints-at-one-million.md) entity checkpoints at one million | W5 |
| [0021](0021-build-time-and-crate-split.md) build time and the crate split | B1, B2 |
| [0022](0022-content-without-java.md) content without Java (proposed) | C6 |
| [0023](0023-offscreen-settlement-simulation.md) off-screen settlement simulation | R3 |
| [0024](0024-engine-and-gameplay-packages.md) engine and gameplay packages | X2 |

Retired records (0003–0009, 0011, 0012, 0014) are in Git history, for example
`git show 973e6e60e:docs/decisions/0012-earth-worldplan-and-native-overlay.md`;
their binding content is in the table. 0001 and 0002 were folded into rows C1
and C2: a code comment citing "ADR 0001" or "ADR 0002" means that row.

A change to authority, threading, waiting, persistence ordering or module
policy updates the row and, where one exists, the record in the same change.
A record keeps its decision and binding rules; implementation detail lives in
the code and the [architecture pages](../ARCHITECTURE.md#find-the-contract).
A new external crate, toolchain, language or runtime, persistent format,
daemon or service, or required prerequisite binary needs a row first:
[new dependencies and components](../development/adding-a-feature.md#new-dependencies-and-components).
