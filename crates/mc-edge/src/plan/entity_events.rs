//! Visual entity events and explosion packets.
use super::*;

/// A sound played at an entity's position, as vanilla `ClientboundSoundEntity`.
/// `source` is the vanilla `SoundSource` id (7 = players); the entity id seeds
/// the pitch jitter, so a resend sounds the same.
fn push_entity_sound(
    out: &mut Vec<PlannedPacket>,
    conn: ConnectionId,
    entity: EntityId,
    sound: &str,
    source: i32,
    volume: f32,
) {
    push_best_effort(
        out,
        conn,
        &mc_protocol::packets::play::ClientboundSoundEntity {
            sound: mc_protocol::codec::Identifier::parse(sound).expect("static entity sound"),
            source,
            entity_id: entity.0,
            fixed_range: None,
            volume,
            pitch: 1.0,
            seed: i64::from(entity.0),
        },
    );
}

pub(super) fn push_entity_event(
    out: &mut Vec<PlannedPacket>,
    conn: ConnectionId,
    event: EntityEvent,
) {
    match event {
        EntityEvent::Status { entity, event } => {
            push_best_effort(
                out,
                conn,
                &WireEntityEvent {
                    entity_id: entity.0,
                    event_id: event,
                },
            );
        }
        EntityEvent::SweepAttack {
            entity,
            position,
            yaw,
        } => {
            let angle = f64::from(yaw).to_radians();
            let dx = -angle.sin();
            let dz = angle.cos();
            push_entity_sound(
                out,
                conn,
                entity,
                "minecraft:entity.player.attack.sweep",
                7,
                1.0,
            );
            push_best_effort(
                out,
                conn,
                &mc_protocol::packets::play::ClientboundSweepParticle {
                    position,
                    direction: [dx as f32, 0.0, dz as f32],
                },
            );
        }
        EntityEvent::ShieldBlock { entity, disabled } => {
            push_entity_sound(
                out,
                conn,
                entity,
                if disabled {
                    "minecraft:item.shield.break"
                } else {
                    "minecraft:item.shield.block"
                },
                7,
                if disabled { 0.8 } else { 1.0 },
            );
        }
        EntityEvent::GoldenDandelion { entity, locked } => {
            push_entity_sound(
                out,
                conn,
                entity,
                if locked {
                    "minecraft:item.golden_dandelion.use"
                } else {
                    "minecraft:item.golden_dandelion.unuse"
                },
                7,
                1.0,
            );
        }
        EntityEvent::Bottle {
            entity,
            empty,
            cauldron,
        } => {
            push_entity_sound(
                out,
                conn,
                entity,
                if empty {
                    "minecraft:item.bottle.empty"
                } else {
                    "minecraft:item.bottle.fill"
                },
                if cauldron { 4 } else { 6 },
                1.0,
            );
        }
        EntityEvent::Swing {
            entity,
            hand,
            animation,
            duration_ticks,
        } => {
            push_best_effort(
                out,
                conn,
                &ClientboundSwingAnimation {
                    entity_id: entity.0,
                    off_hand: matches!(hand, Hand::Off),
                    animation: match animation {
                        mc_domain::SwingKind::None => {
                            mc_protocol::packets::play::SwingAnimationType::None
                        }
                        mc_domain::SwingKind::Whack => {
                            mc_protocol::packets::play::SwingAnimationType::Whack
                        }
                        mc_domain::SwingKind::Stab => {
                            mc_protocol::packets::play::SwingAnimationType::Stab
                        }
                    },
                    duration: duration_ticks,
                },
            );
        }
        EntityEvent::Damage {
            entity,
            source_type_id,
            cause,
            direct,
            source_position,
        } => {
            push_best_effort(
                out,
                conn,
                &mc_protocol::packets::play::ClientboundDamageEvent {
                    entity_id: entity.0,
                    source_type_id,
                    source_cause_id: cause.map_or(-1, |id| id.0),
                    source_direct_id: direct.map_or(-1, |id| id.0),
                    source_position,
                },
            );
        }
        EntityEvent::Hurt { entity } => {
            push_best_effort(
                out,
                conn,
                &WireEntityEvent {
                    entity_id: entity.0,
                    event_id: 2,
                },
            );
        }
        EntityEvent::Death { entity } => {
            push_best_effort(
                out,
                conn,
                &WireEntityEvent {
                    entity_id: entity.0,
                    event_id: 3,
                },
            );
        }
        EntityEvent::Pickup {
            item,
            collector,
            count,
        } => {
            push_best_effort(
                out,
                conn,
                &ClientboundTakeItemEntity {
                    item_entity_id: item.0,
                    player_entity_id: collector.0,
                    amount: i32::from(count),
                },
            );
        }
    }
}

/// Visual-only blast range (vanilla 64); addressed impulses bypass this bound.
pub(super) const EXPLOSION_RANGE_SQ: f64 = 64.0 * 64.0;

/// The blast packet, shaped as the vanilla TNT fixture sends it: 26.3 ids
/// `explosion_emitter` 29, sound `entity.generic.explode` 703, block particles
/// poof 69 and smoke 72. The publish-wide index supplies an already summed
/// recipient impulse.
pub(super) fn explode_packet(
    explosion: &mc_core::boundary::ExplosionEvent,
    knockback: Option<[f64; 3]>,
) -> ClientboundExplode {
    let [x, y, z] = explosion.center;
    ClientboundExplode {
        center: EntityVec3 { x, y, z },
        radius: explosion.radius,
        block_count: i32::try_from(explosion.blocks).unwrap_or(i32::MAX),
        knockback: knockback.map(|[x, y, z]| EntityVec3 { x, y, z }),
        explosion_particle_id: 29,
        sound_reference_id: 703,
        block_particles: vec![
            ExplosionBlockParticle {
                particle_id: 69,
                scaling: 0.5,
                speed: 1.0,
                weight: 1,
            },
            ExplosionBlockParticle {
                particle_id: 72,
                scaling: 1.0,
                speed: 1.0,
                weight: 1,
            },
        ],
        play_sound: true,
    }
}
