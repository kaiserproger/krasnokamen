//! Axolotl play-dead response to attacks in water.
use crate::boundary::EntityDataValue;
use crate::entities::{ColdRow, EntityKind, EntitySlot};
use crate::natural_spawn::Rng;
use crate::special_mobs::Retained;
use crate::tick::{DamagePiece, TickState};

static AXOLOTL: std::sync::LazyLock<u32> = std::sync::LazyLock::new(|| {
    crate::piglin::type_id_of("minecraft:axolotl").expect("axolotl type")
});

pub(crate) fn playing_dead(row: &ColdRow) -> bool {
    matches!(row.special_mob.as_deref(), Some(Retained::Axolotl(_)))
}

pub(crate) fn hurt(state: &mut TickState, slot: EntitySlot, piece: &DamagePiece) {
    let body = state.entities.body(slot);
    let EntityKind::Mob { type_id } = body.kind else {
        return;
    };
    let Some(blueprint) = state.registry.blueprint(type_id) else {
        return;
    };
    let row = state.entities.row(slot);
    if type_id != *AXOLOTL
        || row.controls.no_ai
        || playing_dead(row)
        || piece.amount >= row.health
        || !has_entity_source(piece.source)
    {
        return;
    }
    let pos = state.entities.pos(slot);
    if !state
        .world
        .physics_block_at(
            pos[0].floor() as i32,
            pos[1].floor() as i32,
            pos[2].floor() as i32,
        )
        .is_some_and(|block| {
            state.registry.materials.classify(block) == mc_physics::BlockMaterial::Water
        })
    {
        return;
    }
    let mut rng = Rng(state.world.seed() as u64 ^ state.tick.rotate_left(17) ^ body.id.0 as u64);
    if rng.below(3) != 0
        || (rng.below(3) as f32 >= piece.amount && row.health >= blueprint.params.max_health / 2.0)
    {
        return;
    }
    let id = body.id;
    state.entities.row_mut(slot).special_mob = Some(Box::new(Retained::Axolotl(200)));
    crate::special_mobs::metadata(state, slot, 19, EntityDataValue::Boolean(true));
    state.appearance_changed.push(id);
    state
        .hostile
        .targets
        .retain(|_, ((target, _), _)| *target != id);
    crate::projectiles::payloads::give_effect(
        state,
        id,
        crate::effects::ActiveEffect::new(crate::effects::EffectId::Regeneration, 200, 0),
    );
}

fn has_entity_source(source: Option<crate::intent::DamageSource>) -> bool {
    match source {
        Some(crate::intent::DamageSource::Explosion {
            source,
            kill_credit,
            ..
        }) => source.is_some() || kill_credit.is_some(),
        Some(crate::intent::DamageSource::Command { direct, cause, .. }) => {
            direct.is_some() || cause.is_some()
        }
        Some(_) => true,
        None => false,
    }
}

pub(crate) fn run(state: &mut TickState) {
    let slots: Vec<_> = state.entities.mob_slots(*AXOLOTL).collect();
    for slot in slots {
        let pos = state.entities.pos(slot);
        if !state.entities.body(slot).alive
            || !state.world.is_active(
                (pos[0].floor() as i32).div_euclid(16),
                (pos[2].floor() as i32).div_euclid(16),
            )
        {
            continue;
        }
        let Some(Retained::Axolotl(ticks)) = state.entities.row(slot).special_mob.as_deref() else {
            continue;
        };
        let ticks = *ticks;
        if ticks > 1 {
            if let Some(Retained::Axolotl(remaining)) =
                state.entities.row_mut(slot).special_mob.as_deref_mut()
            {
                *remaining -= 1;
            }
        } else {
            state.entities.row_mut(slot).special_mob = None;
            crate::special_mobs::metadata(state, slot, 19, EntityDataValue::Boolean(false));
            state.appearance_changed.push(state.entities.body(slot).id);
        }
    }
}
