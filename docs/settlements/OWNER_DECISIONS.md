# Settlement war, mount and equipment decisions

The owner approved these values for settlement war, mounts and mounted charge. The native core does not implement war declarations, mounts or charge: there is no war state, window or grace timer, soldiers fight only on foot with native melee, and the army order parser accepts only `follow`, `guard`, `patrol` and `return` (see [army](army.md)). Treat everything below as the design target to implement exactly, not as current behavior.

The supply recipes from the same decision (saddle, hay block) are implemented; see [production](production.md#startup-work-recipes).

## War

- Declared by the settlement's charter holder with a committed declaration and a delivered reason.
- Grace before damage: 72,000 simulation ticks.
- Windows: at most 8, each at most 72,000 ticks long, at least 12,000 ticks apart, absolute `[start, end)`.
- Targets: at most 8 lawful targets. A record is at most 4,096 UTF-8 bytes.
- Private, neutral and unknown targets are refused. After peace or revoke, no new damage commits.
- Victory only by a committed peace or surrender with a recorded outcome. Holding points never wins; a window that expires without an outcome stops its effects and has no winner.
- Contribution is voluntary only, by a real inventory transfer; nothing is taken automatically.
- Victory grants no separate right to breach or salvage.
- Logging out gives no immunity in a lawful conflict already admitted in the current window; no new offline targets are added.

## Mounts

- The horse is the combat mount. Donkey and mule are pack animals only and never charge.
- Each mount has its own native speed; none is declared a vanilla gait profile.
- No forced dismount at 2 blocks: collision limits movement, and a dismount only goes to a safe position.
- Lawful transit through a settlement is allowed; passage is not a right to damage.
- Open: the native profile for mounted feed, health, stamina, collider, seat and impulse.

## Charge

- Spear or pike profile only, after 40 ticks of sustained speed of at least 0.45 blocks per simulation tick.
- Bonus rises linearly from 0 to +3 HP between 0.45 and 0.60 blocks per tick and is capped at +3.
- One hit per target per charge; the mount re-accelerates after contact.

## Equipment

- The spear is two-handed.
- Fractional wear accumulates.
- One arrow is returned at most once.
- Swapping, reconnecting or redeploying never restores stamina.
- A native arrow range limit is defined.

## Forest site preparation — 2026-11-08

> Да, расчищать лес под стройку

- Funded construction may physically clear natural forest from its lot and access path; admission itself changes no blocks.
- Logs, saplings and other vanilla drops go to settlement storage without loss or duplication.
- A block must match this world's deterministic generator and have no retained player placement record. Imported/unknown generator identities and persisted lumberjack sapling roots are protected after restart.
- Water, lava, steep terrain and other existing terrain refusals remain refusals.
- An identical player replacement at a former natural coordinate after history expiry may be felled with its drops conserved; see [the construction policy](construction.md#planning-and-streets).

## Daughter founding — owner approved 2026-11-15

Status: the owner approved `SettlementRecord.founding` with all five committee
amendments. Main requires an independent persistence review before integration.
This is not an approved war/faction model and must not be described as one.

- The existing owner UUID groups teaser settlements. Only completed native
  Spanish blueprint provenance establishes that owner's style.
- The durable addition is a serde-defaulted optional `founding` on a
  settlement, containing only `destination: [i32; 3]` and an existing `household`
  key. Stage, payment, materials, construction and culture are derived from
  existing native state; there is no persisted lineage or culture flag.
- At 24 residents, a whole eligible household of two or three members can
  commission an outside-radius outpost: three paid Spanish houses and a plaza.
  There must be at least two adults; the three-bed plan cannot receive a larger
  household. Missing or relocated members cancel the expedition.
- Site rights, overlap admission and recovery validation are explicit. Civil
  certificates are withheld until physical completion. Generic plugin structure
  preparation cannot reserve an expedition's site.
- One native WAL tick must publish all civil records, same-body membership,
  correctly reissued building IDs, unchanged inventories and transferred
  committed ledger provenance, or none of them. Admission creates no blocks,
  residents, inventory or duplicate receipts.
- Existing checkpoint/WAL rows decode absent `founding` as `None`. Downgrade
  after writing the new schema requires restoring the matching pre-upgrade
  backup, not opening the new world with a binary that rejects the field.

Implementation and gate evidence belong to the SpainCulture report. Public
claims require the actual pinned-server run, not this design decision.
