# Owner decisions: CP-058 / CP-059 / CP-068 (RULESET DECISION)

Source: the council's RULESET DECISION (Astra + GLM second opinion; the owner amends if desired).
The values below are verbatim from the decision; implement exactly this. CP058/059/068 are unblocked
on these values. Only the native profile for feed/HP/stamina/collider/seat/impulse remains open.

## CP058 — war

- Declared by: the charter holder, with a committed declaration + a delivered reason.
- Grace: 72000 simulation ticks.
- Windows: ≤8, each ≤72000, pause ≥12000, absolute [start,end).
- Targets: ≤8 lawful.
- Record size: ≤4096 UTF-8 bytes.
- Protection: Private/Neutral/Unknown = refusal; after peace/revoke, 0 new damage commits.
- GLM amendments APPLY:
  - (a) NO automatic victory by holding points — victory ONLY by a committed peace/surrender with a recorded outcome; window expiry without an outcome = effects stop, no winner.
  - (b) auto-contribution = 0 (voluntary payment only, by a real inventory transfer).
  - (c) victory does NOT grant a separate right to breach/salvage.
  - (d) AdmittedConflictContinues: logout does NOT grant immunity in an already admitted lawful conflict of the current window, but there are no new offline targets.

## CP059 — supply

- Saddle: 3 leather + 1 iron = 1 saddle, crafting_table, 240 ticks.
- Hay: 9 wheat → block 80 ticks; block → 9 wheat 40 ticks.
- Turnover: atomic debit/output, no replay minting.
- NO fixed price of 10–12 (price comes from secured demand).
- NO quota of 4 per settlement (limiters = materials + worker + station).
- Authorized trade deliveries are allowed.

## CP068 — mounts

- Horse — combat; donkey/mule — pack animals only, no charge.
- Speeds: individual native speeds (do NOT declare Astra's numbers to be vanilla gait profiles).
- Charge: max +3HP, spear/pike only, after 40 ticks of build-up.
- NO forced dismount at 2 blocks (collision limits movement; dismount only to a safe position).
- Lawful transit is allowed (passage ≠ a right to damage).

## CP068-R2 — catalog

- Values: OWNER_DECISIONS §1 of the plugin repo (`../solaris-default-plugins/docs/settlements/OWNER_DECISIONS.md`) unchanged.
- Spear is explicitly two-handed.
- Fractional wear accumulates.
- One arrow is returned at most once.
- Swap/reconnect/deploy never restore stamina.
- Define a native arrow range limit.

## Charge — accepted numbers (verbatim)

- Trigger: ≥0.45 blocks/sim-tick, sustained 40 ticks.
- Bonus: linear 0→3 HP from 0.45→0.60 blocks/tick, capped at +3 beyond that.
- Spear/pike profile only.
- One hit per target per charge.
- Re-accelerate after contact.
- Mounted feed/HP/stamina/collider/seat/impulse native profile — still OPEN.
