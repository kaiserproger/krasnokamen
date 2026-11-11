//! Pregnant frogs seek land beside water and lay frogspawn.
use crate::boundary::BlockPos;
use crate::entities::ColdRow;
use crate::tick::TickState;

static TYPE: std::sync::LazyLock<u32> =
    std::sync::LazyLock::new(|| crate::piglin::type_id_of("minecraft:frog").expect("frog type"));

pub(crate) fn is_frog(type_id: u32) -> bool {
    type_id == *TYPE
}

pub(crate) fn drive(row: &ColdRow) -> Option<([f64; 3], f64)> {
    if let Some((_, target)) = row.entity_life.frog_prey {
        return Some((
            target,
            if row.entity_life.frog_tongue_ticks > 0 {
                0.0
            } else {
                f64::from(row.speed) * 2.0
            },
        ));
    }
    if !row.entity_life.frog_pregnant {
        return None;
    }
    let target = row.entity_life.frog_spawn_target?;
    Some((
        [
            f64::from(target[0]) + 0.5,
            f64::from(target[1]),
            f64::from(target[2]) + 0.5,
        ],
        f64::from(row.speed),
    ))
}

fn spawn_at(state: &TickState, shore: [i32; 3]) -> Option<BlockPos> {
    let [x, y, z] = shore;
    let ground = state.world.block_at(x, y - 1, z)?;
    if state.registry.materials.classify(ground) != mc_physics::BlockMaterial::Solid
        || state.world.block_at(x, y, z) != Some(state.registry.states.air)
    {
        return None;
    }
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let water = state.world.block_at(x + dx, y - 1, z + dz);
        if water.is_some_and(|id| {
            state.registry.materials.classify(id) == mc_physics::BlockMaterial::Water
        }) && state.world.block_at(x + dx, y, z + dz) == Some(state.registry.states.air)
        {
            return Some(BlockPos {
                x: x + dx,
                y,
                z: z + dz,
            });
        }
    }
    None
}

fn site(state: &TickState, pos: [f64; 3]) -> Option<[i32; 3]> {
    let origin = pos.map(|v| v.floor() as i32);
    let mut best = None;
    for x in origin[0] - 8..=origin[0] + 8 {
        for z in origin[2] - 8..=origin[2] + 8 {
            for y in origin[1] - 3..=origin[1] + 3 {
                let shore = [x, y, z];
                if spawn_at(state, shore).is_none() {
                    continue;
                }
                let distance = (f64::from(x) + 0.5 - pos[0]).powi(2)
                    + (f64::from(y) - pos[1]).powi(2)
                    + (f64::from(z) + 0.5 - pos[2]).powi(2);
                if best.is_none_or(|(_, old)| distance < old) {
                    best = Some((shore, distance));
                }
            }
        }
    }
    best.map(|(shore, _)| shore)
}

pub(crate) fn run(state: &mut TickState) {
    let mut slots = std::mem::take(&mut state.serial_entity_slots);
    slots.extend(state.entities.mob_slots(*TYPE));
    for &slot in &slots {
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        if !body.alive || row.controls.no_ai {
            continue;
        }
        let id = body.id;
        let pos = state.entities.pos(slot);
        if !state.world.is_active(
            (pos[0].floor() as i32).div_euclid(16),
            (pos[2].floor() as i32).div_euclid(16),
        ) {
            continue;
        }
        hunt(state, slot);
        if !state.entities.row(slot).entity_life.frog_pregnant {
            continue;
        }
        let here = pos.map(|v| v.floor() as i32);
        if state.entities.ground(slot)
            && let Some(at) = spawn_at(state, here)
        {
            let name = mc_data::Identifier::parse("minecraft:frogspawn").expect("frogspawn id");
            if let Some(block) = state.registry.blocks.block(&name) {
                let block = block.default.0;
                state.write_block(at, block);
                let life = &mut state.entities.row_mut(slot).entity_life;
                life.frog_pregnant = false;
                life.frog_spawn_target = None;
            }
        } else if state.tick.wrapping_add(id.0 as u64).is_multiple_of(20) {
            let target = site(state, pos);
            state.entities.row_mut(slot).entity_life.frog_spawn_target = target;
        }
    }
    slots.clear();
    state.serial_entity_slots = slots;
}

/// `Frog.DATA_TONGUE_TARGET_ID`: Animal ends at 17, the variant is 18 (local javap).
const TONGUE_INDEX: u8 = 19;

fn hunt(state: &mut TickState, frog: crate::entities::EntitySlot) {
    use crate::boundary::EntityDataValue;
    let pos = state.entities.pos(frog);
    let id = state.entities.body(frog).id;
    let previous = state
        .entities
        .row(frog)
        .entity_life
        .frog_prey
        .map(|(id, _)| id);
    let prey = state
        .registry
        .blueprints
        .iter()
        .filter(|(_, blueprint)| {
            matches!(
                blueprint.capabilities.size_profile,
                crate::blueprint::SizeProfile::Slime | crate::blueprint::SizeProfile::Armored
            )
        })
        .flat_map(|(kind, _)| state.entities.mob_slots(*kind))
        .filter(|slot| {
            state.entities.body(*slot).alive && state.entities.row(*slot).loot.slime_size == Some(1)
        })
        .filter_map(|slot| {
            let at = state.entities.pos(slot);
            let distance = crate::bees::distance(pos, at);
            (distance < 100.0 && crate::hostile::sight_clear(state, pos, at)).then_some((
                distance,
                state.entities.body(slot).id,
                slot,
            ))
        })
        .min_by(|a, b| {
            (Some(b.1) == previous)
                .cmp(&(Some(a.1) == previous))
                .then(a.0.total_cmp(&b.0))
                .then(a.1.cmp(&b.1))
        });
    let Some((distance, target, prey)) = prey else {
        let row = state.entities.row(frog);
        if row.entity_life.frog_prey.is_none()
            && row.entity_life.frog_tongue_ticks == 0
            && row.appearance.pose != 9
            && !row
                .appearance
                .extra
                .iter()
                .any(|(index, _)| *index == TONGUE_INDEX)
        {
            return;
        }
        let row = state.entities.row_mut(frog);
        row.entity_life.frog_prey = None;
        row.entity_life.frog_tongue_ticks = 0;
        if row.appearance.pose == 9 {
            std::sync::Arc::make_mut(&mut row.appearance).pose = 0;
            state.appearance_changed.push(id);
        }
        crate::special_mobs::metadata(
            state,
            frog,
            TONGUE_INDEX,
            EntityDataValue::OptionalUnsignedInt(None),
        );
        return;
    };
    let target_pos = state.entities.pos(prey);
    let row = state.entities.row_mut(frog);
    if previous != Some(target) {
        row.entity_life.frog_tongue_ticks = 0;
    }
    let started = row.entity_life.frog_tongue_ticks > 0;
    row.entity_life.frog_prey = Some((target, target_pos));
    if !started && distance >= 1.75 * 1.75 {
        return;
    }
    let row = state.entities.row_mut(frog);
    row.entity_life.frog_tongue_ticks = row.entity_life.frog_tongue_ticks.saturating_add(1);
    let ticks = row.entity_life.frog_tongue_ticks;
    if ticks == 1 {
        std::sync::Arc::make_mut(&mut row.appearance).pose = 9;
        state.appearance_changed.push(id);
        crate::special_mobs::metadata(
            state,
            frog,
            TONGUE_INDEX,
            EntityDataValue::OptionalUnsignedInt(Some(target.0 as u32)),
        );
    }
    if ticks == 1 || ticks == 8 {
        let sound = mc_data::Identifier::parse(if ticks == 1 {
            "minecraft:entity.frog.tongue"
        } else {
            "minecraft:entity.frog.eat"
        })
        .unwrap();
        let viewers: Vec<_> = state
            .connections
            .all_connections()
            .filter(|connection| {
                state
                    .connections
                    .entity_of(*connection)
                    .and_then(|id| state.entities.slot_of(id))
                    .is_some_and(|viewer| {
                        crate::bees::distance(pos, state.entities.pos(viewer)) < 1024.0
                    })
            })
            .collect();
        for connection in viewers {
            state.push_player(
                connection,
                crate::boundary::PlayerChange::CommandSound {
                    sound: sound.clone(),
                    fixed_range: None,
                    source: 6,
                    position: pos.map(|v| (v * 8.0) as i32),
                    volume: 2.0,
                    pitch: 1.0,
                },
            );
        }
    }
    if ticks == 8 {
        state.merge_damage.push((
            prey,
            crate::tick::DamagePiece {
                amount: 10.0,
                kind: crate::survival::DamageKind::Melee,
                physical: None,
                custom_armor: false,
                source: Some(crate::intent::DamageSource::Melee {
                    attacker: id,
                    position: pos,
                    yaw: state.entities.rot(frog)[0],
                    bonus_knockback: 0.0,
                    sweep: false,
                    fire_ticks: 0,
                    bane_ticks: 0,
                }),
            },
        ));
    }
    if ticks >= 12 {
        let row = state.entities.row_mut(frog);
        row.entity_life.frog_tongue_ticks = 0;
        std::sync::Arc::make_mut(&mut row.appearance).pose = 0;
        state.appearance_changed.push(id);
        crate::special_mobs::metadata(
            state,
            frog,
            TONGUE_INDEX,
            EntityDataValue::OptionalUnsignedInt(None),
        );
    }
}
