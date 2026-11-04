//! Llama ranged retaliation and wandering-trader defense.
use crate::boundary::EntityId;
use crate::entities::{EntityKind, EntitySlot};
use crate::tick::TickState;
use mc_data::ItemStack;

static TYPES: std::sync::LazyLock<[u32; 2]> = std::sync::LazyLock::new(|| {
    ["minecraft:llama", "minecraft:trader_llama"].map(|name| {
        mc_data::entity_types::entity_type_contract_26_1_2_by_name(name)
            .expect("llama type")
            .protocol_id
    })
});

pub(crate) fn is_llama(type_id: u32) -> bool {
    TYPES.contains(&type_id)
}

pub(crate) fn defend_trader(state: &mut TickState, victim: EntitySlot, attacker: EntityId) {
    let EntityKind::Mob { type_id } = state.entities.body(victim).kind else {
        return;
    };
    if crate::piglin::name(type_id) != "minecraft:wandering_trader" {
        return;
    }
    let owner = state.entities.row(victim).uuid;
    let defenders: Vec<_> = state
        .entities
        .mob_slots(TYPES[1])
        .filter(|&slot| {
            state.entities.row(slot).entity_life.leash == Some(crate::leash::Holder::Entity(owner))
        })
        .collect();
    for slot in defenders {
        crate::hostile::retaliate(state, slot, attacker);
    }
}

pub(crate) fn run(state: &mut TickState) {
    let llamas: Vec<_> = TYPES
        .iter()
        .flat_map(|&kind| state.entities.mob_slots(kind))
        .collect();
    for slot in llamas {
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        let id = body.id;
        let pos = state.entities.pos(slot);
        if !body.alive
            || row.controls.no_ai
            || !state.world.is_active(
                (pos[0].floor() as i32).div_euclid(16),
                (pos[2].floor() as i32).div_euclid(16),
            )
        {
            continue;
        }
        if !state.hostile.targets.contains_key(&id)
            && state.tick.wrapping_add(id.0 as u64).is_multiple_of(16)
        {
            let wolf = state
                .entities
                .slots()
                .filter_map(|candidate| {
                    let body = state.entities.body(candidate);
                    let EntityKind::Mob { type_id } = body.kind else {
                        return None;
                    };
                    if crate::piglin::name(type_id) != "minecraft:wolf"
                        || !body.alive
                        || state.entities.row(candidate).entity_life.tamed
                    {
                        return None;
                    }
                    let at = state.entities.pos(candidate);
                    let d = (0..3)
                        .map(|axis| (at[axis] - pos[axis]).powi(2))
                        .sum::<f64>();
                    (d <= 16.0).then_some((body.id, d))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((wolf, _)) = wolf {
                crate::hostile::retaliate(state, slot, wolf);
            }
        }
        if state.entities.body(slot).attack_cooldown != 0 {
            continue;
        }
        let Some(&((target, _), _)) = state.hostile.targets.get(&id) else {
            continue;
        };
        let Some(victim) = state.entities.slot_of(target) else {
            state.hostile.targets.remove(&id);
            continue;
        };
        if !state.entities.body(victim).alive
            || state
                .players
                .get(&target)
                .is_some_and(|p| p.creative() || p.spectator())
        {
            state.hostile.targets.remove(&id);
            continue;
        }
        let at = state.entities.pos(victim);
        let muzzle = [
            pos[0],
            pos[1] + state.entities.body(slot).height * 0.75,
            pos[2],
        ];
        let aim = [
            at[0],
            at[1] + state.entities.body(victim).height / 3.0,
            at[2],
        ];
        let delta = [aim[0] - muzzle[0], aim[1] - muzzle[1], aim[2] - muzzle[2]];
        let horizontal = (delta[0] * delta[0] + delta[2] * delta[2]).sqrt();
        if horizontal > 20.0 || !crate::hostile::sight_clear(state, muzzle, aim) {
            continue;
        }
        let aimed = [delta[0], delta[1] + horizontal * 0.2, delta[2]];
        let length = aimed.iter().map(|v| v * v).sum::<f64>().sqrt().max(0.001);
        let mut projectile = crate::projectiles::ProjectileState::new(
            crate::projectiles::ProjectileKind::LlamaSpit,
            Some(state.entities.row(slot).uuid),
            ItemStack::EMPTY,
        );
        projectile.mob_damage = true;
        if crate::projectiles::launch(state, projectile, muzzle, aimed.map(|v| v / length * 1.5))
            .is_some()
        {
            state.hostile.targets.remove(&id);
            state.entities.body_mut(slot).attack_cooldown = 40;
        }
    }
}
