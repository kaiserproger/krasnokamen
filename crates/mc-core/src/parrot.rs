//! Parrot.imitateNearbyMobs; dancing is client-local from jukebox level events.
use crate::{
    boundary::{ConnectionId, PlayerChange},
    tick::TickState,
};

pub(crate) fn run(state: &mut TickState) {
    let Some(kind) = crate::piglin::type_id_of("minecraft:parrot") else {
        return;
    };
    let mut slots = std::mem::take(&mut state.serial_entity_slots);
    slots.extend(state.entities.mob_slots(kind));
    for &slot in &slots {
        let pos = state.entities.pos(slot);
        if !state.entities.body(slot).alive
            || state.entities.row(slot).controls.no_ai
            || !state.world.is_active(
                (pos[0].floor() as i32).div_euclid(16),
                (pos[2].floor() as i32).div_euclid(16),
            )
            || crate::taming::roll_at(state.entities.row(slot), state.tick, 50) != 0
        {
            continue;
        }
        let mut best = None;
        for x in ((pos[0] - 20.0).floor() as i32).div_euclid(16)
            ..=((pos[0] + 20.0).floor() as i32).div_euclid(16)
        {
            for z in ((pos[2] - 20.0).floor() as i32).div_euclid(16)
                ..=((pos[2] + 20.0).floor() as i32).div_euclid(16)
            {
                if let Some(ids) = state
                    .entities
                    .brain_sensor_chunks()
                    .get(&crate::boundary::ChunkPos { x, z })
                {
                    for id in ids {
                        let Some(other) = state.entities.slot_of(*id) else {
                            continue;
                        };
                        let name = crate::mounts::name(state, other);
                        let path = name.strip_prefix("minecraft:").unwrap_or(name);
                        let path = if path == "cave_spider" {
                            "spider"
                        } else {
                            path
                        };
                        let sound = match path {
                            "blaze" | "bogged" | "breeze" | "camel_husk" | "creaking"
                            | "creeper" | "drowned" | "elder_guardian" | "ender_dragon"
                            | "endermite" | "evoker" | "ghast" | "guardian" | "hoglin" | "husk"
                            | "illusioner" | "magma_cube" | "parched" | "phantom" | "piglin"
                            | "piglin_brute" | "pillager" | "ravager" | "shulker"
                            | "silverfish" | "skeleton" | "slime" | "spider" | "stray" | "vex"
                            | "vindicator" | "warden" | "witch" | "wither" | "wither_skeleton"
                            | "zoglin" | "zombie" | "zombie_horse" | "zombie_nautilus"
                            | "zombie_villager" => path,
                            _ => continue,
                        };
                        let distance = crate::bees::distance(pos, state.entities.pos(other));
                        if distance < 400.0
                            && best.is_none_or(|(old, old_id, _)| {
                                distance < old || distance == old && *id < old_id
                            })
                        {
                            best = Some((distance, *id, sound));
                        }
                    }
                }
            }
        }
        if let Some((_, _, sound)) = best {
            let sound =
                mc_data::Identifier::parse(format!("minecraft:entity.parrot.imitate.{sound}"))
                    .unwrap();
            let viewers: Vec<ConnectionId> = state
                .connections
                .all_connections()
                .filter(|connection| {
                    state
                        .connections
                        .entity_of(*connection)
                        .and_then(|id| state.entities.slot_of(id))
                        .is_some_and(|viewer| {
                            crate::bees::distance(pos, state.entities.pos(viewer)) < 256.0
                        })
                })
                .collect();
            for connection in viewers {
                state.push_player(
                    connection,
                    PlayerChange::CommandSound {
                        sound: sound.clone(),
                        fixed_range: None,
                        source: 6,
                        position: pos.map(|v| (v * 8.0) as i32),
                        volume: 0.7,
                        pitch: 1.0,
                    },
                );
            }
        }
    }
    slots.clear();
    state.serial_entity_slots = slots;
}
