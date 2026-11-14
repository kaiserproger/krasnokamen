//! Vanilla parrot imitation of nearby mob sounds. Dance is client-local.
use crate::boundary::PlayerChange;
use crate::entities::{EntityKind, EntitySlot};
use crate::tick::TickState;

static TYPE: std::sync::LazyLock<u32> = std::sync::LazyLock::new(|| {
    crate::piglin::type_id_of("minecraft:parrot").expect("parrot type")
});

fn sound_name(name: &str) -> Option<&str> {
    match name.strip_prefix("minecraft:")? {
        "cave_spider" => Some("spider"),
        "happy_ghast" => Some(""),
        name @ ("blaze" | "bogged" | "breeze" | "camel_husk" | "creaking" | "creeper"
        | "drowned" | "elder_guardian" | "ender_dragon" | "endermite" | "evoker" | "ghast"
        | "guardian" | "hoglin" | "husk" | "illusioner" | "magma_cube" | "parched" | "phantom"
        | "piglin" | "piglin_brute" | "pillager" | "ravager" | "shulker" | "silverfish"
        | "skeleton" | "slime" | "spider" | "stray" | "vex" | "vindicator" | "warden"
        | "witch" | "wither" | "wither_skeleton" | "zoglin" | "zombie" | "zombie_horse"
        | "zombie_nautilus" | "zombie_villager") => Some(name),
        _ => None,
    }
}

fn silent(row: &crate::entities::ColdRow) -> bool {
    if row.opaque_entity_fields.is_empty() { return false; }
    let Ok((_, mc_nbt::Tag::Compound(fields))) = mc_nbt::read_named(&mut std::io::Cursor::new(&row.opaque_entity_fields)) else { return false; };
    fields.iter().any(|(key, value)| key == "Silent" && matches!(value, mc_nbt::Tag::Byte(v) if *v != 0))
}

fn imitate(state: &mut TickState, slot: EntitySlot, random: u64) {
    if !state.entities.body(slot).alive || silent(state.entities.row(slot)) { return; }
    let at = state.entities.pos(slot);
    let chunk = |v: f64| mc_physics::mth::floor_i32(v).div_euclid(16);
    let mut selected = None;
    let mut seen = 0_u64;
    let mut random = random;
    for z in chunk(at[2] - 20.0)..=chunk(at[2] + 20.0) {
        for x in chunk(at[0] - 20.0)..=chunk(at[0] + 20.0) {
            state.entities.for_each_slot_in_chunk(crate::boundary::ChunkPos { x, z }, |other| {
                let body = state.entities.body(other);
                let EntityKind::Mob { type_id } = body.kind else { return; };
                if !body.alive { return; }
                let Some(name) = state.registry.blueprint(type_id).and_then(|bp| sound_name(&bp.name)) else { return; };
                let pos = state.entities.pos(other);
                if !(0..3).all(|i| (pos[i] - at[i]).abs() <= 20.0 + body.half_width) { return; }
                seen += 1;
                random = mix(random);
                if random % seen == 0 { selected = Some((other, name)); }
            });
        }
    }
    let Some((other, name)) = selected else { return; };
    if name.is_empty() || silent(state.entities.row(other)) { return; }
    let sound = mc_data::Identifier::parse(format!("minecraft:entity.parrot.imitate.{name}")).expect("sound id");
    let a = ((mix(random) >> 40) as f32) / ((1_u32 << 24) as f32);
    let b = ((mix(mix(random)) >> 40) as f32) / ((1_u32 << 24) as f32);
    let pitch = (a - b) * 0.2 + 1.0;
    let listeners: Vec<_> = state.players.iter().filter_map(|(id, player)| {
        let listener = state.entities.slot_of(*id)?;
        let pos = state.entities.pos(listener);
        ((0..3).map(|i| (pos[i] - at[i]).powi(2)).sum::<f64>() <= 256.0).then_some(player.connection)
    }).collect();
    for connection in listeners {
        state.push_player(connection, PlayerChange::CommandSound {
            sound: sound.clone(), source: 5, position: at.map(|v| (v * 8.0).floor() as i32), volume: 0.7, pitch,
        });
    }
}

fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

pub(crate) fn run(state: &mut TickState) {
    let parrots: Vec<_> = state.entities.mob_slots(*TYPE).collect();
    for slot in parrots {
        let at = state.entities.pos(slot);
        if !state.world.is_active((at[0].floor() as i32).div_euclid(16), (at[2].floor() as i32).div_euclid(16)) { continue; }
        let random = mix(state.tick ^ state.entities.row(slot).uuid.as_u128() as u64 ^ state.world.seed() as u64);
        if random % 800 == 0 { imitate(state, slot, random); }
    }
}

#[cfg(test)]
#[path = "parrot_tests.rs"]
mod tests;
