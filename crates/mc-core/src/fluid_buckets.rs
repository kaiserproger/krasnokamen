//! Clicked bucket/cauldron plans share one inventory debit with block custody.
use super::{Flow, Fluid, contained, flow_into, offset, waterlogged};
use crate::boundary::{BlockPos, Hand};
use crate::intent::{Footprint, Intent, IntentKind, InvClaim};
use crate::tick::{TickState, held_slot_index};
use mc_data::{Identifier, ItemStack};
use mc_domain::EntityId;

/// Player inventory claim for one bucket exchange, plus the give-back item the
/// bucket intent hands to the actor when the held stack is not consumed whole.
fn exchange(
    state: &TickState,
    entity: EntityId,
    hand: Hand,
    held: &ItemStack,
    give: u32,
) -> Option<(InvClaim, Option<u32>)> {
    let player = state.players.get(&entity)?;
    if player.spectator() {
        return None;
    }
    let slot = held_slot_index(player, hand);
    let creative = player.creative();
    let single = held.count == 1;
    let kept_in_hand = !creative && single;
    Some((
        InvClaim {
            owner: entity,
            slot: slot as u16,
            expect_item: held.item_id,
            expect_count: held.count,
            take: i32::from(!creative),
            put_item: kept_in_hand.then_some(give),
            put_count: i32::from(kept_in_hand),
        },
        give_whole_stack(creative, single, give),
    ))
}

/// Item returned to the actor when the exchange replaced a single held bucket.
fn give_whole_stack(creative: bool, single: bool, give: u32) -> Option<u32> {
    (!creative && !single).then_some(give)
}

/// Block state of a cauldron filled to `level=3`.
fn filled_cauldron(state: &TickState, name: &str) -> Option<u32> {
    let id = Identifier::parse(format!("minecraft:{name}")).ok()?;
    state
        .registry
        .blocks
        .by_name_and_props(&id, &[("level".into(), "3".into())])
        .map(|state| state.0)
}

/// Powder-snow bucket exchange with cauldrons: a snow bucket fills an empty
/// cauldron; an empty bucket collects a full powder-snow cauldron.
fn powder_snow_use(
    state: &TickState,
    entity: EntityId,
    hand: Hand,
    pos: BlockPos,
    held: &ItemStack,
    empty: u32,
    snow: u32,
) -> Option<Intent> {
    let current = state.world.block_at(pos.x, pos.y, pos.z)?;
    let entry = state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(current))?;
    let path = entry.block.id.path();
    let (new, give) = if held.item_id == snow {
        if path != "cauldron" {
            return None;
        }
        (filled_cauldron(state, "powder_snow_cauldron")?, empty)
    } else {
        if path != "powder_snow_cauldron" {
            return None;
        }
        let full = entry
            .properties
            .iter()
            .any(|(key, value)| key == "level" && value == "3");
        if !full {
            return None;
        }
        let id = Identifier::parse("minecraft:cauldron").ok()?;
        (state.registry.blocks.block(&id)?.default.0, snow)
    };
    let (claim, give) = exchange(state, entity, hand, held, give)?;
    Some(Intent {
        actor: entity,
        seq: 0,
        kind: IntentKind::Bucket { pos, give },
        footprint: Footprint {
            block_expect: vec![(pos, current)],
            block_write: vec![(pos, new)],
            inv: vec![claim],
            ..Default::default()
        },
    })
}

pub(crate) fn use_on(
    state: &mut TickState,
    entity: EntityId,
    hand: Hand,
    pos: BlockPos,
    face: crate::placement::Face,
    held: &ItemStack,
) -> Option<Intent> {
    let mut intent = plan_use_on(state, entity, hand, pos, face, held)?;
    intent.seq = state.next_seq(entity);
    Some(intent)
}

pub(crate) fn plan_use_on(
    state: &TickState,
    entity: EntityId,
    hand: Hand,
    pos: BlockPos,
    face: crate::placement::Face,
    held: &ItemStack,
) -> Option<Intent> {
    let item = |name: &str| {
        state
            .registry
            .items
            .id_of(&Identifier::parse(format!("minecraft:{name}")).ok()?)
    };
    let empty = item("bucket")?;
    let water = item("water_bucket")?;
    let lava = item("lava_bucket")?;
    let snow = item("powder_snow_bucket")?;
    // Powder snow fills/empties cauldrons directly; it is not a fluid.
    // An empty hand on a powder cauldron also routes here (collection).
    let current = state.world.block_at(pos.x, pos.y, pos.z)?;
    let powder_cell = state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(current))
        .is_some_and(|entry| entry.block.id.path() == "powder_snow_cauldron");
    if (held.item_id == snow || (held.item_id == empty && powder_cell))
        && let Some(intent) = powder_snow_use(state, entity, hand, pos, held, empty, snow)
    {
        return Some(intent);
    }
    let carried = if held.item_id == empty {
        None
    } else if held.item_id == water {
        Some(Fluid::Water)
    } else if held.item_id == lava {
        Some(Fluid::Lava)
    } else {
        return None;
    };
    let entry = state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(current))?;
    let path = entry.block.id.path();
    let resolve = |name: &str| {
        state
            .registry
            .blocks
            .block(&Identifier::parse(format!("minecraft:{name}")).ok()?)
            .map(|block| block.default.0)
    };
    let (target, new, give) = if matches!(
        path,
        "cauldron" | "water_cauldron" | "lava_cauldron" | "powder_snow_cauldron"
    ) {
        match carried {
            Some(Fluid::Water) => (pos, filled_cauldron(state, "water_cauldron")?, empty),
            Some(Fluid::Lava) => (pos, resolve("lava_cauldron")?, empty),
            None => {
                let full = entry
                    .properties
                    .iter()
                    .any(|(key, value)| key == "level" && value == "3");
                let give = if path == "lava_cauldron" {
                    lava
                } else if path == "water_cauldron" && full {
                    water
                } else {
                    // Powder-snow collection rides `powder_snow_use` above.
                    return None;
                };
                (pos, resolve("cauldron")?, give)
            }
        }
    } else if let Some(fluid) = carried {
        if fluid == Fluid::Water
            && let Some(wet) = waterlogged(state, current, true)
        {
            (pos, wet, empty)
        } else {
            let target = if matches!(flow_into(state, pos, fluid), Some(Flow::Wash(_))) {
                pos
            } else {
                offset(pos, face.offset())
            };
            let target_state = state.world.block_at(target.x, target.y, target.z)?;
            let new = if fluid == Fluid::Water
                && let Some(wet) = waterlogged(state, target_state, true)
            {
                wet
            } else {
                flow_into(state, target, fluid)?;
                fluid.source(&state.registry.states)
            };
            (target, new, empty)
        }
    } else {
        let (fluid, level) = contained(state, current)?;
        if level != 0 {
            return None;
        }
        let new = waterlogged(state, current, false).unwrap_or(state.registry.states.air);
        (pos, new, if fluid == Fluid::Water { water } else { lava })
    };
    let evaporate = carried == Some(Fluid::Water)
        && state.dimension.water_evaporates()
        && !matches!(
            path,
            "cauldron" | "water_cauldron" | "lava_cauldron" | "powder_snow_cauldron"
        );
    let expected = state.world.block_at(target.x, target.y, target.z)?;
    if expected == new && !evaporate {
        return None;
    }
    let (claim, give) = exchange(state, entity, hand, held, give)?;
    Some(Intent {
        actor: entity,
        seq: 0,
        kind: IntentKind::Bucket { pos: target, give },
        footprint: Footprint {
            block_expect: vec![(target, expected), (pos, current)],
            block_write: if evaporate {
                Vec::new()
            } else {
                vec![(target, new)]
            },
            inv: vec![claim],
            ..Default::default()
        },
    })
}
