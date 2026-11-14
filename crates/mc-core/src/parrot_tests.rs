use super::*;
use crate::mounts::tests::{fixture, slot, spawn};

#[test]
fn nearby_hostile_mob_is_imitated_at_parrot_position() {
    let (mut state, actor, pos) = fixture();
    let parrot = spawn(&mut state, "parrot", pos);
    spawn(&mut state, "cave_spider", [pos[0] + 2.0, pos[1], pos[2]]);
    let parrot_slot = slot(&state, parrot);
    state.outbox.clear();
    imitate(&mut state, parrot_slot, 12345);
    let connection = state.players[&actor].connection;
    assert!(
        state.outbox[&connection]
            .iter()
            .any(|change| matches!(change,
        PlayerChange::CommandSound { sound, source: 5, volume, .. }
            if sound.as_str() == "minecraft:entity.parrot.imitate.spider" && *volume == 0.7))
    );
}

#[test]
fn parrot_tick_emits_nearby_hostile_imitation_to_player() {
    let (mut state, actor, pos) = fixture();
    let parrot = spawn(&mut state, "parrot", pos);
    let spider = spawn(&mut state, "cave_spider", [pos[0] + 2.0, pos[1], pos[2]]);
    let parrot_slot = slot(&state, parrot);
    let spider_slot = slot(&state, spider);
    state.entities.row_mut(spider_slot).controls.no_ai = true;
    let uuid = state.entities.row(parrot_slot).uuid.as_u128() as u64;
    let seed = state.world.seed() as u64;
    state.tick = (0..100_000)
        .find(|tick| mix(*tick ^ uuid ^ seed) % 800 == 0)
        .expect("deterministic imitation tick");
    let expected_position = state
        .entities
        .pos(parrot_slot)
        .map(|coordinate| (coordinate * 8.0).floor() as i32);
    let connection = state.players[&actor].connection;
    let (_, drains, _) = crate::tick::run_tick(&mut state, &[]);
    let sounds: Vec<_> = drains
        .iter()
        .filter(|(id, _)| *id == connection)
        .flat_map(|(_, changes)| changes.iter())
        .filter_map(|change| match change {
            PlayerChange::CommandSound {
                sound,
                source,
                position,
                volume,
                pitch,
            } if sound.as_str() == "minecraft:entity.parrot.imitate.spider" => {
                Some((*source, *position, *volume, *pitch))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        sounds.len(),
        1,
        "exactly one imitation should reach the nearby player"
    );
    assert_eq!(sounds[0].0, 5);
    assert_eq!(sounds[0].1, expected_position);
    assert_eq!(sounds[0].2, 0.7);
    assert!((0.8..=1.2).contains(&sounds[0].3));
}

#[test]
fn ordinary_animals_do_not_trigger_mimicry() {
    let (mut state, actor, pos) = fixture();
    let parrot = spawn(&mut state, "parrot", pos);
    spawn(&mut state, "cow", [pos[0] + 1.0, pos[1], pos[2]]);
    let parrot_slot = slot(&state, parrot);
    state.outbox.clear();
    imitate(&mut state, parrot_slot, 12345);
    let connection = state.players[&actor].connection;
    assert!(state.outbox.get(&connection).is_none_or(|changes| {
        !changes
            .iter()
            .any(|change| matches!(change, PlayerChange::CommandSound { .. }))
    }));
}
