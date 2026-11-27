use super::*;
use crate::mounts::tests::{fixture, slot, spawn};
use crate::tick::run_tick;
use mc_data::entity_data_value::EntityDataValue;

#[test]
fn frog_shoots_tongue_at_small_slime_and_clears_target() {
    let (mut state, _, pos) = fixture();
    let frog = spawn(&mut state, "frog", pos);
    let slime = spawn(&mut state, "slime", [pos[0] + 1.0, pos[1], pos[2]]);
    let frog_slot = slot(&state, frog);
    let slime_slot = slot(&state, slime);
    let crate::entities::EntityKind::Mob { type_id } = state.entities.body(slime_slot).kind else {
        panic!("slime mob");
    };
    assert!(
        state.registry.tags.contains_raw_id(
            "minecraft:entity_type",
            "minecraft:frog_food",
            type_id as i32
        ),
        "embedded frog_food must include slime"
    );
    state.entities.row_mut(slime_slot).controls.no_ai = true;
    state.entities.row_mut(slime_slot).loot.slime_size = Some(1);
    state.entities.row_mut(slime_slot).health = 5.0;
    state.tick = (20 - (frog.0 as u64 % 20)) % 20;
    hunt(&mut state, frog_slot);
    let appearance = &state.entities.row(frog_slot).appearance;
    assert_eq!(appearance.pose, 9);
    assert!(appearance.extra.contains(&(
        TONGUE_INDEX,
        EntityDataValue::OptionalUnsignedInt(Some(slime.0 as u32))
    )));
    for _ in 0..20 {
        run_tick(&mut state, &[]);
    }
    assert!(
        state
            .entities
            .slot_of(slime)
            .is_none_or(|slot| !state.entities.body(slot).alive),
        "frog attack_damage={} hunt={:?} prey_health={:?}",
        state.entities.row(frog_slot).attack_damage,
        state.entities.row(frog_slot).frog_prey,
        state
            .entities
            .slot_of(slime)
            .map(|s| state.entities.row(s).health)
    );
    let appearance = &state.entities.row(frog_slot).appearance;
    assert_eq!(appearance.pose, 0);
    assert!(
        appearance
            .extra
            .contains(&(TONGUE_INDEX, EntityDataValue::OptionalUnsignedInt(None)))
    );
}

#[test]
fn frog_does_not_target_large_slimes() {
    let (mut state, _, pos) = fixture();
    let frog = spawn(&mut state, "frog", pos);
    let slime = spawn(&mut state, "slime", [pos[0] + 1.0, pos[1], pos[2]]);
    let frog_slot = slot(&state, frog);
    let slime_slot = slot(&state, slime);
    state.entities.row_mut(slime_slot).loot.slime_size = Some(2);
    state.tick = (20 - (frog.0 as u64 % 20)) % 20;
    hunt(&mut state, frog_slot);
    assert!(state.entities.row(frog_slot).frog_prey.is_none());
}

#[test]
fn frog_variants_eat_magma_cube_and_drop_exact_froglight() {
    for (variant, expected) in [
        ("minecraft:warm", "minecraft:pearlescent_froglight"),
        ("minecraft:cold", "minecraft:verdant_froglight"),
        ("minecraft:temperate", "minecraft:ochre_froglight"),
    ] {
        let (mut state, actor, pos) = fixture();
        let actor_slot = slot(&state, actor);
        state
            .entities
            .set_pos(actor_slot, [pos[0] + 7.0, pos[1], pos[2] + 7.0]);
        let frog = spawn(&mut state, "frog", pos);
        let cube = spawn(&mut state, "magma_cube", [pos[0] + 1.0, pos[1], pos[2]]);
        let frog_slot = slot(&state, frog);
        let cube_slot = slot(&state, cube);
        mc_nbt::write_named(
            &mut state.entities.row_mut(frog_slot).opaque_entity_fields,
            "",
            &mc_nbt::Tag::Compound(vec![(
                "variant".into(),
                mc_nbt::Tag::String(variant.into()),
            )]),
        )
        .unwrap();
        state.entities.row_mut(cube_slot).controls.no_ai = true;
        state.entities.row_mut(cube_slot).loot.slime_size = Some(1);
        state.entities.row_mut(cube_slot).health = 1.0;
        state.tick = (20 - (frog.0 as u64 % 20)) % 20;
        hunt(&mut state, frog_slot);
        for _ in 0..20 {
            run_tick(&mut state, &[]);
        }
        assert!(
            state
                .entities
                .slot_of(cube)
                .is_none_or(|slot| !state.entities.body(slot).alive),
            "magma cube must be killed by frog"
        );
        let drops: Vec<_> = state
            .entities
            .slots()
            .filter_map(|s| {
                state.entities.row(s).item.as_ref().map(|stack| {
                    (
                        state
                            .registry
                            .items
                            .name_of(stack.item_id)
                            .unwrap()
                            .as_str()
                            .to_owned(),
                        stack.count,
                    )
                })
            })
            .collect();
        assert_eq!(drops, vec![(expected.to_owned(), 1)], "variant {variant}");
    }
}
