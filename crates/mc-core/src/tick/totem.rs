//! Lethal-hit interception, before death/drop/plugin notifications.
use crate::boundary::{Hand, PlayerChange};
use crate::effects::{ActiveEffect, EffectId};
use crate::player::PlayerState;
use crate::registry::CoreRegistry;
use crate::survival::DamageKind;
use mc_data::ItemStack;
use mc_data::data_component_types_26_1_2 as component_types;
use mc_data::item_stack::death_protection::{self, DeathEffect, EffectTargets, Sound};

pub(super) struct Rescue {
    pub used: ItemStack,
    pub changes: Vec<PlayerChange>,
    pub effects: Vec<DeathEffect>,
    default_effects: bool,
}

pub(super) fn rescue(
    player: &mut PlayerState,
    registry: &CoreRegistry,
    kind: DamageKind,
) -> Option<Rescue> {
    // LivingEntity.checkTotemDeathProtection rejects bypasses_invulnerability.
    if kind.bypasses_invulnerability() {
        return None;
    }
    let component_name = component_types::name(component_types::DEATH_PROTECTION as u32)
        .expect("death protection is in the canonical component catalogue");
    for (hand, slot) in [
        (Hand::Main, usize::from(player.inventory.selected)),
        (Hand::Off, crate::inventory::OFF_HAND),
    ] {
        let stack = &player.inventory.slots[slot];
        if stack.is_empty()
            || stack
                .extended()
                .preserved_components
                .iter()
                .any(|component| component.name.strip_prefix('!') == Some(component_name))
        {
            continue;
        }
        let custom = stack
            .extended()
            .preserved_components
            .iter()
            .find(|component| component.name == component_name);
        let effects = if let Some(component) = custom {
            let mut bytes = component.value.as_slice();
            let Ok(value) = mc_nbt::read_network_value(&mut bytes) else {
                continue;
            };
            if !bytes.is_empty() {
                continue;
            }
            let Ok(effects) = death_protection::parse(&value) else {
                continue;
            };
            effects
        } else if registry
            .items
            .name_of(stack.item_id)
            .is_none_or(|name| name.as_str() != "minecraft:totem_of_undying")
        {
            continue;
        } else {
            Vec::new()
        };
        let default_effects = custom.is_none();
        let used = stack.clone();
        let mut remaining = used.clone();
        remaining.count -= 1;
        if remaining.count == 0 {
            remaining = ItemStack::EMPTY;
        }
        let changes = player.replace_held(hand, remaining);
        player
            .statistics
            .increment(mc_data::statistics::StatType::Used, used.item_id, 1);
        return Some(Rescue {
            used,
            changes,
            effects,
            default_effects,
        });
    }
    None
}

fn active(effect: mc_data::item_stack::PotionEffect) -> Option<ActiveEffect> {
    Some(ActiveEffect {
        id: EffectId::from_name(effect.id.as_str())?,
        amplifier: effect.amplifier,
        duration: effect.duration,
        ambient: effect.ambient,
        show_particles: effect.show_particles,
        show_icon: effect.show_icon,
        hidden: effect
            .hidden
            .and_then(|hidden| active(*hidden))
            .map(Box::new),
    })
}

fn sound(state: &mut super::TickState, position: [f64; 3], event: Sound) {
    let range = f64::from(event.fixed_range.unwrap_or(16.0).max(0.0));
    let listeners: Vec<_> = state
        .players
        .iter()
        .filter_map(|(id, player)| {
            let slot = state.entities.slot_of(*id)?;
            let pos = state.entities.pos(slot);
            let squared: f64 = (0..3)
                .map(|axis| (pos[axis] - position[axis]).powi(2))
                .sum();
            (squared < range * range).then_some(player.connection)
        })
        .collect();
    for connection in listeners {
        state.push_player(
            connection,
            PlayerChange::CommandSound {
                sound: event.id.clone(),
                fixed_range: event.fixed_range,
                source: 7,
                position: position.map(|value| (value * 8.0).floor() as i32),
                volume: 1.0,
                pitch: 1.0,
            },
        );
    }
}

/// Apply in list order between damage pieces, not after the damage batch.
pub(super) fn apply(
    state: &mut super::TickState,
    id: crate::boundary::EntityId,
    rescue: &mut Rescue,
) {
    fn flush(
        state: &mut super::TickState,
        id: crate::boundary::EntityId,
        changes: &mut Vec<PlayerChange>,
    ) {
        if let Some(connection) = state.connections.entity_connection(id) {
            for change in changes.drain(..) {
                state.push_player(connection, change);
            }
        }
    }
    if let Some(player) = state.players.get_mut(&id) {
        player.vitals.health = 1.0;
        if rescue.default_effects {
            rescue.changes.extend(player.clear_effects(None));
            for (effect, duration, amplifier) in [
                (EffectId::Regeneration, 900, 1),
                (EffectId::Absorption, 100, 1),
                (EffectId::FireResistance, 800, 0),
            ] {
                rescue
                    .changes
                    .extend(player.give_effect(ActiveEffect::new(effect, duration, amplifier)));
            }
        }
    }
    for effect in std::mem::take(&mut rescue.effects) {
        flush(state, id, &mut rescue.changes);
        match effect {
            DeathEffect::Apply {
                effects,
                probability,
            } => {
                let Some(player) = state.players.get_mut(&id) else {
                    return;
                };
                if crate::enchantments::random(&mut player.enchantment_seed)
                    < f64::from(probability)
                {
                    for effect in effects.into_iter().filter_map(active) {
                        rescue.changes.extend(player.give_effect(effect));
                    }
                }
            }
            DeathEffect::Remove(targets) => {
                let Some(player) = state.players.get_mut(&id) else {
                    return;
                };
                let mut index = 0;
                while index < player.active_effects.entries.len() {
                    let effect = player.active_effects.entries[index].id;
                    let matches = match &targets {
                        EffectTargets::Direct(ids) => ids
                            .iter()
                            .any(|id| EffectId::from_name(id.as_str()) == Some(effect)),
                        EffectTargets::NamedTag(tag) => state.registry.tags.contains_raw_id(
                            mc_data::tags::MOB_EFFECT_REGISTRY,
                            tag.as_str(),
                            effect as i32,
                        ),
                    };
                    if matches {
                        rescue.changes.extend(player.clear_effects(Some(effect)));
                    } else {
                        index += 1;
                    }
                }
            }
            DeathEffect::Clear => {
                if let Some(player) = state.players.get_mut(&id) {
                    rescue.changes.extend(player.clear_effects(None));
                }
            }
            DeathEffect::Teleport { diameter } => {
                let Some(slot) = state.entities.slot_of(id) else {
                    return;
                };
                let Some(player) = state.players.get_mut(&id) else {
                    return;
                };
                let seed = (crate::enchantments::random(&mut player.enchantment_seed)
                    * u64::MAX as f64) as u64;
                let old = state.entities.pos(slot);
                if let Some(position) =
                    crate::consumables::random_destination(state, old, seed, f64::from(diameter))
                {
                    crate::mounts::dismount(state, id);
                    let Some(player) = state.players.get_mut(&id) else {
                        return;
                    };
                    player.fall_distance = 0.0;
                    player.current_impulse_impact_pos = None;
                    let connection = player.connection;
                    let [yaw, pitch, _] = state.entities.rot(slot);
                    state.entities.set_pos(slot, position);
                    state.correct_position(connection, position, yaw, pitch);
                    crate::vibration::emit(
                        state,
                        old,
                        crate::vibration::GameEvent::Teleport,
                        Some(id),
                    );
                    sound(
                        state,
                        position,
                        Sound {
                            id: mc_data::Identifier::parse(
                                mc_data::builtin_sounds_26_3::CHORUS_FRUIT_TELEPORT,
                            )
                            .expect("canonical sound identifier parses"),
                            fixed_range: None,
                            registered: true,
                        },
                    );
                }
            }
            DeathEffect::PlaySound(event) => {
                let Some(slot) = state.entities.slot_of(id) else {
                    return;
                };
                let position = state.entities.pos(slot).map(|value| value.floor() + 0.5);
                sound(state, position, event);
            }
        }
    }
    flush(state, id, &mut rescue.changes);
}
