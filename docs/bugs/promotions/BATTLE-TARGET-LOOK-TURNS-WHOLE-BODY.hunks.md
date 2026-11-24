# BATTLE-TARGET-LOOK-TURNS-WHOLE-BODY — fix-hunk map

The production-only patch `BATTLE-TARGET-LOOK-TURNS-WHOLE-BODY.patch` is one hunk, SHA-256
`patch:9ba59d256ba805a9c924686930abea620dd04d827feb041a5ca7b8e833dc5b7e`. Reverse-applies on the
candidate (`git apply -R --check` on `c26aa0c0ea`, clean tree).

## The fix and the hunk the patch carries

`crates/mc-core/src/battle/facing.rs::body_rule` is the client's `BodyRotationControl`: while the head
turns at up to `HEAD_STABLE` per tick the drawn body holds inside the `CONE`, then aligns over
`ALIGN_DELAY`+`ALIGN_TICKS` stable ticks. Before D36 the look turned the whole body (`Facing::Look`
in `entity_motion`), which is the row's defect. The patch reverses `body_rule` to that mechanism:

```rust
fn body_rule(row: &mut Row, head: f32) -> f32 {
    row.stable = 0;
    row.last_stable = head;
    head
}
```

## The a/-side against a pre-image

`facing.rs` is new in `1a66a4c90` (`feat(battle): D36 head/body facing for standing soldiers`); there is
no pre-fix version of this module to restore, so the a/-side is the defect's own mechanism — the body is
the head every tick — expressed in the module's terms. This is the ONE hunk difference and it is a
reconstruction, recorded here as such (README: a defect with no historical text may be controlled by
its mechanism). Nothing else differs; the rest guard, the old-look path, the cone and the projection are
HEAD's.

## Why this control is honest

The registered ref is
`cargo:mc-core:battle::facing::tests::head_turns_while_the_body_holds_then_the_body_aligns_after_ten_stable_ticks_in_ten`.
Its first loop asserts, tick by tick while the head turns, `row.stored[0] == 0.0` with the message
"the body holds", and its second loop asserts the drawn `body` stays at 0 through the delay. With the
defect mechanism the stored yaw and the drawn body follow the head, so the row's own hold assertion
fails — not a counter, not a precondition. The other facing refs (`virtual_body_trails_the_head_at_the_cone`,
`dead_zone_holds_at_19_degrees_and_turns_at_21`, `a_turn_releases_within_5_degrees_of_the_current_bearing`)
stay registered.

## Observed control and restore

Observed control and restore (`20261124T043035-ledger-promote-wpdt3vov`, `tree_dirty: false`,
`reversed_files` = `battle/facing.rs`):

- control (`ledger-1-run.log`) **FAIL** — `assertion \`left == right\` failed: tick 0: the drawn body
  holds through the align delay`, `left: 1106246720` (`30.0_f32` bits), `right: 0`, at
  `facing_tests.rs:104` — the row's own hold assertion under the defect mechanism, not an invented
  text; the sibling `virtual_body_trails_the_head_at_the_cone` reds with it;
- restore green (`ledger-3-run.log`).

## What stays out of the row

The head's own turn (smoothing, dead zone, release) is untouched by the hunk; the patch reverses only the
standing-body rule.
