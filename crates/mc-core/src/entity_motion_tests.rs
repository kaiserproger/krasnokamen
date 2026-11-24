//! Behavior tests for the production native movement/control kernels.
use crate::blueprint::MovementClass;
use crate::entity_motion::{
    MotionInput, MotionResult, breathe, look, panic, steer, step, wander, wander_heading,
};
use crate::species::AquaticControl;
use mc_physics::{Aabb, BlockMaterial, BlockSampler, EntityBody, Vec3};

struct Habitat<F>(F);
impl<F: Fn(i32, i32, i32) -> BlockMaterial> BlockSampler for Habitat<F> {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        (self.0)(x, y, z)
    }
}

fn input(movement: MovementClass, position: Vec3) -> MotionInput {
    MotionInput {
        body: EntityBody {
            position,
            velocity: Vec3::ZERO,
            aabb: Aabb {
                half_width: 0.3,
                height: 0.6,
            },
            on_ground: movement == MovementClass::Walker,
        },
        movement,
        drive: [0.0; 3],
        impulse: [0.0; 2],
        rotation: [0.0; 3],
        look_target: None,
        facing: None,
        battle_facing: false,
        legacy_facing: false,
        head_follows_body: false,
        hops: false,
        hop_delay_scale: 1,
        hop_lift: 0.0,
        hop_aggressive: false,
        climbs_walls: false,
        aquatic_control: AquaticControl::Other,
        air_supply: 300,
        levitation: 0.0,
        slow_falling: false,
        no_gravity: false,
        id: 1,
        tick: 0,
    }
}

#[test]
fn ground_moves_on_support_and_stops_at_a_tall_wall() {
    let world = Habitat(|x, y, _| {
        if y < 0 || (x == 1 && y < 5) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut state = input(MovementClass::Walker, Vec3::new(0.0, 0.0, 0.0));
    state.drive = [4.0, 0.0, 0.0];
    let moved = step(state, &world, |_, _| true);
    assert!(moved.body.position.x > 0.0);
    assert!(moved.body.on_ground);
    let mut state = input(MovementClass::Walker, Vec3::new(0.65, 0.0, 0.0));
    state.drive = [4.0, 0.0, 0.0];
    let blocked = step(state, &world, |_, _| true);
    assert!(blocked.body.position.x <= 0.7 + 1.0e-7);
    assert_eq!(blocked.body.velocity.x, 0.0);
    assert_eq!(blocked.body.position.y, 0.0);
}

#[test]
fn slime_hops_without_walking_between_jumps() {
    let world = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut body = input(MovementClass::Walker, Vec3::new(0.5, 0.0, 0.5)).body;
    let start = body.position.x;
    let mut airborne = 0;
    let mut launches = 0;
    let mut walked_while_grounded = 0;
    for tick in 0..200 {
        let mut state = input(MovementClass::Walker, body.position);
        state.body = body;
        state.hops = true;
        state.tick = tick;
        state.drive = [4.0, 0.0, 0.0];
        let before = state.body;
        let moved = step(state, &world, |_, _| true);
        if !moved.body.on_ground {
            airborne += 1;
        }
        if before.on_ground {
            if moved.body.velocity.y > 0.0 {
                launches += 1;
            } else if (moved.body.position.x - before.position.x).abs() > 1e-12 {
                // `SlimeMoveControl` has no traction between hops.
                walked_while_grounded += 1;
            }
        }
        body = moved.body;
    }
    assert!(airborne > 5, "a slime must leave the ground: {airborne}");
    assert!(launches >= 1, "a hop is an upward launch");
    assert_eq!(
        walked_while_grounded, 0,
        "SlimeMoveControl must not walk between hops"
    );
    let distance = body.position.x - start;
    assert!(
        distance > 0.5,
        "hops must still advance toward the wanted point: {distance}"
    );
}

#[test]
fn slime_hop_climbs_a_full_block_step() {
    let world = Habitat(|x, y, _| {
        if y < 0 || (x == 1 && y == 0) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut body = input(MovementClass::Walker, Vec3::new(0.5, 0.0, 0.5)).body;
    let mut highest = 0.0_f64;
    for tick in 0..250 {
        let mut state = input(MovementClass::Walker, body.position);
        state.body = body;
        state.hops = true;
        state.tick = tick;
        state.drive = [4.0, 0.0, 0.0];
        body = step(state, &world, |_, _| true).body;
        highest = highest.max(body.position.y);
    }
    assert!(
        highest > 1.0,
        "a slime jump must clear a full block, peak {highest}"
    );
    assert!(
        body.position.y >= 1.0 && body.position.x > 1.0,
        "the slime must finish on top of the block: {:?}",
        body.position
    );
}

#[test]
fn airborne_knockback_is_not_added_twice() {
    let world = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut state = input(MovementClass::Walker, Vec3::new(0.5, 2.0, 0.5));
    state.hops = true;
    state.body.on_ground = false;
    state.body.velocity = Vec3::new(8.0, 0.0, 0.0);
    state.impulse = [3.0, 0.0];
    state.drive = [0.0; 3];
    let moved = step(state, &world, |_, _| true);
    assert!(
        moved.body.velocity.x <= 8.0 + 1.0e-9,
        "stored impulse was counted again: {}",
        moved.body.velocity.x
    );
}

#[test]
fn grounded_knockback_lift_survives_a_non_hop_tick() {
    let world = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut body = input(MovementClass::Walker, Vec3::new(0.5, 0.0, 0.5)).body;
    body.velocity = Vec3::new(0.0, 3.0, 0.0);
    let mut checked = false;
    for tick in 0..200 {
        let mut state = input(MovementClass::Walker, body.position);
        state.body = body;
        state.hops = true;
        state.tick = tick;
        if crate::entity_motion::hop_due(state.id, tick, 1, false) {
            continue;
        }
        let moved = step(state, &world, |_, _| true);
        assert!(
            moved.body.velocity.y > 0.0 || moved.body.position.y > 0.0,
            "grounded knockback lift was zeroed"
        );
        checked = true;
        break;
    }
    assert!(checked, "found a non-hop tick");
}

#[test]
fn magma_cube_hop_delay_is_four_times_longer() {
    let slime = (0..400)
        .filter(|tick| crate::entity_motion::hop_due(9, *tick, 1, false))
        .count();
    let magma = (0..400)
        .filter(|tick| crate::entity_motion::hop_due(9, *tick, 4, false))
        .count();
    assert!(slime > 0 && magma > 0);
    assert!(
        magma * 3 < slime,
        "MagmaCube.getJumpDelay multiplies the slime delay by four: {magma} vs {slime}"
    );
}

#[test]
fn hop_delay_period_matches_the_blueprint_scale() {
    // Vanilla draws `random(20)+10` per jump and counts it down only while on
    // the ground, so its observed period is delay plus flight. This kernel keeps
    // a deterministic phase on an absolute tick clock: the period between hop
    // ticks is exactly the delay, and a hop tick that lands mid-flight is simply
    // not a launch.
    for scale in [1_u64, 4] {
        let ticks: Vec<u64> = (0..20_000)
            .filter(|tick| crate::entity_motion::hop_due(11, *tick, scale, false))
            .collect();
        assert!(ticks.len() > 20, "ticks={ticks:?}");
        let period = ticks[1] - ticks[0];
        assert!(
            (10 * scale..=29 * scale).contains(&period),
            "Slime.getJumpDelay is random(20)+10, MagmaCube x4: period {period}"
        );
        assert!(
            ticks.windows(2).all(|pair| pair[1] - pair[0] == period),
            "a fixed delay must keep its period: {ticks:?}"
        );
    }
}

#[test]
fn an_aggressive_body_divides_its_scaled_jump_delay_by_three() {
    // `Slime$SlimeMoveControl.tick`: `jumpDelay = getJumpDelay(); if (isAggressive)
    // jumpDelay /= 3`. `MagmaCube.getJumpDelay` already multiplied by four, and
    // `MagmaCube.canDealDamage` is `isEffectiveAi`, so a chasing cube's delay is
    // (10..29)*4/3 = 13..38 ticks; a chasing slime's is 3..9.
    for (scale, low, high) in [(1_u64, 3_u64, 9_u64), (4, 13, 38)] {
        let ticks: Vec<u64> = (0..20_000)
            .filter(|tick| crate::entity_motion::hop_due(11, *tick, scale, true))
            .collect();
        let period = ticks[1] - ticks[0];
        assert!(
            (low..=high).contains(&period),
            "aggressive delay for scale {scale} out of {low}..={high}: {period}"
        );
        let calm: Vec<u64> = (0..20_000)
            .filter(|tick| crate::entity_motion::hop_due(11, *tick, scale, false))
            .collect();
        assert!(
            ticks.len() >= calm.len() * 2,
            "aggression must shorten the delay: {} vs {}",
            ticks.len(),
            calm.len()
        );
        let idle_period = calm[1] - calm[0];
        assert!(
            (10 * scale..=29 * scale).contains(&idle_period),
            "an idle hopper keeps the full delay: {idle_period}"
        );
    }
}

#[test]
fn a_hop_tick_advances_a_slime_toward_its_drive() {
    // A hop tick is the only tick a slime moves on the ground: the launch keeps
    // the wanted horizontal speed, so a mid/far slime's hop tick must displace it.
    let world = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut body = input(MovementClass::Walker, Vec3::new(0.5, 0.0, 0.5)).body;
    let tick = (0..400)
        .find(|tick| crate::entity_motion::hop_due(7, *tick, 1, false))
        .expect("a hop tick");
    let mut state = input(MovementClass::Walker, body.position);
    state.body = body;
    state.hops = true;
    state.id = 7;
    state.tick = tick;
    state.drive = [4.0, 0.0, 0.0];
    body = step(state, &world, |_, _| true).body;
    assert!(
        body.position.x > 0.5 + 1.0e-9,
        "the hop tick must move the slime toward its drive: {:?}",
        body.position
    );
}

#[test]
fn hop_tick_rises_the_vanilla_jump_speed_on_its_first_tick() {
    let world = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    // Vanilla `getJumpPower()` is the float 0.42f, i.e. 0.419_999_986_886_978_15
    // blocks/tick, plus `MagmaCube`'s `size * 0.1`.
    let base = mc_physics::LIVING_JUMP_SPEED_BLOCKS_PER_SECOND * mc_physics::TICK_SECONDS;
    for (lift, expected) in [(0.0, base), (0.4, base + 0.4)] {
        let mut body = input(MovementClass::Walker, Vec3::new(0.5, 0.0, 0.5)).body;
        let id = 11;
        let tick = (0..400)
            .find(|tick| crate::entity_motion::hop_due(id, *tick, 1, false))
            .expect("a hop tick");
        let mut state = input(MovementClass::Walker, body.position);
        state.body = body;
        state.hops = true;
        state.id = id;
        state.tick = tick;
        state.hop_lift = lift;
        body = step(state, &world, |_, _| true).body;
        assert!(
            (body.position.y - expected).abs() < 1.0e-9,
            "vanilla jumpFromGround moves {} blocks on the jump tick, got {}",
            expected,
            body.position.y
        );
    }
}

#[test]
fn ground_obstacle_jump_finishes_climbing_a_full_block() {
    let world = Habitat(|x, y, _| {
        if y < 0 || (x == 1 && y == 0) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut body = input(MovementClass::Walker, Vec3::new(0.65, 0.0, 0.0)).body;
    let mut jumped = false;
    let mut atop = false;
    for tick in 0..32 {
        let mut state = input(MovementClass::Walker, body.position);
        state.body = body;
        state.drive = [4.0, 0.0, 0.0];
        state.tick = tick;
        body = step(state, &world, |_, _| true).body;
        jumped |= body.velocity.y > 0.0;
        atop |= body.position.y >= 1.0 && body.position.x > 1.0;
    }
    assert!(jumped);
    assert!(
        atop,
        "ground jump must cross, not repeatedly bump, a one-block obstacle"
    );
}

#[test]
fn zombie_and_animal_blueprints_walk_onto_one_block_rise() {
    let registry = crate::registry::CoreRegistry::load(None).expect("registry");
    let world = Habitat(|x, y, _| {
        if y < 0 || (x >= 1 && y == 0) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    for name in [
        "minecraft:zombie",
        "minecraft:cow",
        "minecraft:pig",
        "minecraft:sheep",
    ] {
        let type_id = registry
            .entity_types
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap();
        let blueprint = registry.blueprint(type_id).unwrap();
        assert_eq!(blueprint.movement, MovementClass::Walker, "{name}");
        let mut body = input(blueprint.movement, Vec3::new(0.5, 0.0, 0.5)).body;
        body.aabb = Aabb {
            half_width: blueprint.half_width,
            height: blueprint.height,
        };
        for tick in 0..60 {
            let mut state = input(blueprint.movement, body.position);
            state.body = body;
            state.drive = steer(
                blueprint.movement,
                [body.position.x, body.position.y, body.position.z],
                [3.5, 1.0, 0.5],
                f64::from(blueprint.params.speed),
            );
            state.tick = tick;
            body = step(state, &world, |_, _| true).body;
        }
        assert!(body.position.x > 3.0, "{name}: {body:?}");
        assert!((body.position.y - 1.0).abs() < 1.0e-7, "{name}: {body:?}");
        assert!(body.on_ground, "{name} must land on the raised floor");
    }
}

#[test]
fn hostile_flat_ground_chase_distance_matches_tuned_speed() {
    let registry = crate::registry::CoreRegistry::load(None).expect("registry");
    let world = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    for (name, expected) in [("minecraft:zombie", 2.4), ("minecraft:spider", 5.0)] {
        let type_id = registry
            .entity_types
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap();
        let blueprint = registry.blueprint(type_id).unwrap();
        let mut body = input(blueprint.movement, Vec3::new(0.5, 0.0, 0.5)).body;
        body.aabb = Aabb {
            half_width: blueprint.half_width,
            height: blueprint.height,
        };
        for tick in 0..20 {
            let mut state = input(blueprint.movement, body.position);
            state.body = body;
            state.drive = steer(
                blueprint.movement,
                [body.position.x, body.position.y, body.position.z],
                [100.5, 0.0, 0.5],
                f64::from(blueprint.params.speed),
            );
            state.tick = tick;
            body = step(state, &world, |_, _| true).body;
        }
        let distance = body.position.x - 0.5;
        assert!(
            (distance - expected).abs() < 1e-5,
            "{name}: {distance} blocks per second"
        );
        assert!(body.on_ground);
    }
}

#[test]
fn low_ceiling_prevents_ground_obstacle_jump() {
    let world = Habitat(|x, y, _| {
        if y < 0 || y == 1 || (x == 1 && y == 0) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut state = input(MovementClass::Walker, Vec3::new(0.65, 0.0, 0.0));
    state.drive = [4.0, 0.0, 0.0];
    let result = step(state, &world, |_, _| true);
    assert_eq!(result.body.position.y, 0.0);
    assert!(result.body.position.x <= 0.7 + 1.0e-7);
}

#[test]
fn spider_wall_climb_is_bounded_and_clipped_at_ceiling() {
    let world = Habitat(|x, y, _| {
        if y < 0 || y == 2 || (x == 1 && y < 5) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut body = input(MovementClass::Walker, Vec3::new(0.65, 0.0, 0.0)).body;
    for tick in 0..16 {
        let before = body.position.y;
        let mut state = input(MovementClass::Walker, body.position);
        state.body = body;
        state.drive = [4.0, 0.0, 0.0];
        state.climbs_walls = true;
        state.tick = tick;
        let result = step(state, &world, |_, _| true);
        assert!(
            result.wall_climbing,
            "ceiling contact still resets fall distance"
        );
        body = result.body;
        assert!(body.position.y - before <= 0.2 + 1.0e-7);
        assert!(body.position.y + body.aabb.height <= 2.0 + 1.0e-7);
    }
    assert!(body.position.y > 1.0);
}

#[test]
fn missing_current_chunk_freezes_every_native_kind_without_air_loss() {
    let world = Habitat(|_, _, _| BlockMaterial::Solid);
    for kind in [
        MovementClass::Walker,
        MovementClass::Swimmer,
        MovementClass::Flyer,
        MovementClass::Amphibious,
        MovementClass::Stationary,
    ] {
        let mut state = input(kind, Vec3::new(18.0, 5.0, 0.0));
        state.drive = [4.0, 3.0, 2.0];
        state.body.velocity = Vec3::new(4.0, -6.0, 2.0);
        state.air_supply = -19;
        let result = step(state, &world, |_, _| false);
        assert_eq!(result.body.position, Vec3::new(18.0, 5.0, 0.0));
        assert_eq!(result.body.velocity, Vec3::ZERO);
        assert_eq!(result.air_supply, -19);
        assert!(!result.drowning);
        assert!(!result.wall_climbing);
    }
}

#[test]
fn spider_does_not_climb_missing_neighbor_chunk_boundary() {
    let world = Habitat(|x, y, _| {
        if x >= 16 || y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut state = input(MovementClass::Walker, Vec3::new(15.65, 0.0, 0.0));
    state.climbs_walls = true;
    state.drive = [4.0, 0.0, 0.0];
    let result = step(state, &world, |x, _| x < 16);
    assert_eq!(result.body.position.y, 0.0);
    assert!(result.body.position.x <= 15.7 + 1.0e-7);
    assert!(!result.wall_climbing);
}

#[test]
fn flying_moves_vertically_without_gravity_and_clips_ceiling() {
    let air = Habitat(|_, _, _| BlockMaterial::Air);
    let mut state = input(MovementClass::Flyer, Vec3::new(0.0, 1.0, 0.0));
    state.drive = [2.0, 3.0, 0.0];
    let result = step(state, &air, |_, _| true);
    assert_eq!(result.body.velocity, Vec3::new(2.0, 3.0, 0.0));
    assert_eq!(result.body.position, Vec3::new(0.1, 1.15, 0.0));
    assert!(result.rotation[1] < 0.0);
    let ceiling = Habitat(|_, y, _| {
        if y >= 2 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut state = input(MovementClass::Flyer, Vec3::new(0.0, 1.35, 0.0));
    state.drive = [0.0, 4.0, 0.0];
    let result = step(state, &ceiling, |_, _| true);
    assert!(result.body.position.y <= 1.4 + 1.0e-7);
    assert_eq!(result.body.velocity.y, 0.0);
    let idle = step(
        input(MovementClass::Flyer, Vec3::new(0.0, 5.0, 0.0)),
        &air,
        |_, _| true,
    );
    assert_eq!(idle.body.position.y, 5.0);
}

#[test]
fn aquatic_and_amphibious_steer_vertically_in_water() {
    let water = Habitat(|_, _, _| BlockMaterial::Water);
    for kind in [MovementClass::Swimmer, MovementClass::Amphibious] {
        let mut state = input(kind, Vec3::new(0.0, 1.0, 0.0));
        state.drive = [2.0, -3.0, 1.0];
        state.air_supply = 0;
        let result = step(state, &water, |_, _| true);
        assert!(result.body.position.x > 0.0);
        assert!(result.body.position.y < 1.0);
        assert!(result.body.position.z > 0.0);
        assert_eq!(result.air_supply, 300);
        assert!(!result.drowning);
    }
}

#[test]
fn aquatic_navigation_stays_in_known_water_and_avoids_dry_or_solid_destination() {
    for boundary in [
        BlockMaterial::Air,
        BlockMaterial::Lava,
        BlockMaterial::Solid,
    ] {
        let world = Habitat(|x, y, _| {
            if x >= 1 || y >= 2 {
                boundary
            } else {
                BlockMaterial::Water
            }
        });
        let mut state = input(MovementClass::Swimmer, Vec3::new(0.65, 1.35, 0.0));
        state.drive = [4.0, 4.0, 0.0];
        let result = step(state, &world, |_, _| true);
        assert!(result.body.position.x + result.body.aabb.half_width <= 1.0 + 1.0e-7);
        assert!(result.body.position.y + result.body.aabb.height <= 2.0 + 1.0e-7);
        assert_ne!(
            result.body.velocity,
            Vec3::ZERO,
            "bounded alternatives should escape the shore"
        );
    }
}

#[test]
fn fish_flops_on_land_while_squid_does_not_and_amphibious_walks() {
    let ground = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut fish = input(MovementClass::Swimmer, Vec3::ZERO);
    fish.aquatic_control = AquaticControl::Fish;
    fish.body.on_ground = true;
    let fish = step(fish, &ground, |_, _| true);
    assert!(fish.body.position.y > 0.0);
    assert!(fish.body.velocity.y > 0.0);
    let mut squid = input(MovementClass::Swimmer, Vec3::ZERO);
    squid.aquatic_control = AquaticControl::Squid;
    squid.body.on_ground = true;
    squid.drive = [3.0, 3.0, 0.0];
    let squid = step(squid, &ground, |_, _| true);
    assert_eq!(squid.body.position, Vec3::ZERO);
    let mut amphibious = input(MovementClass::Amphibious, Vec3::ZERO);
    amphibious.body.on_ground = true;
    amphibious.drive = [3.0, 3.0, 0.0];
    let amphibious = step(amphibious, &ground, |_, _| true);
    assert!(amphibious.body.position.x > 0.0);
    assert_eq!(amphibious.body.position.y, 0.0);
    assert_eq!(amphibious.air_supply, 300);
}

#[test]
fn stationary_never_wanders_or_integrates_but_can_look_at_target() {
    let air = Habitat(|_, _, _| BlockMaterial::Air);
    for tick in 0..240 {
        assert_eq!(wander(MovementClass::Stationary, 1, tick, 4.0), [0.0; 3]);
    }
    let mut state = input(MovementClass::Stationary, Vec3::new(0.0, 5.0, 0.0));
    state.drive = [3.0, 3.0, 3.0];
    state.impulse = [4.0, 4.0];
    state.look_target = Some([1.0, 6.0, 1.0]);
    let result = step(state, &air, |_, _| true);
    assert_eq!(result.body.position, Vec3::new(0.0, 5.0, 0.0));
    assert_eq!(result.body.velocity, Vec3::ZERO);
    assert!(result.rotation[0] < 0.0 && result.rotation[1] < 0.0);
}

#[test]
fn aquatic_air_damage_boundary_resets_and_amphibious_never_suffocates_on_land() {
    let mut air = 300;
    for _ in 0..319 {
        (air, _) = breathe(air, false, MovementClass::Swimmer);
    }
    assert_eq!(air, -19);
    assert_eq!(breathe(air, false, MovementClass::Swimmer), (0, true));
    assert_eq!(breathe(air, true, MovementClass::Swimmer), (300, false));
    assert_eq!(breathe(air, false, MovementClass::Amphibious), (300, false));
}

#[test]
fn look_and_move_controls_use_target_height_and_bound_turn_rate() {
    let drive = steer(MovementClass::Flyer, [0.0; 3], [0.0, 2.0, 2.0], 4.0);
    assert!(drive[1] > 0.0 && drive[2] > 0.0);
    let ground = steer(MovementClass::Walker, [0.0; 3], [0.0, 2.0, 2.0], 4.0);
    assert_eq!(ground[1], 0.0);
    let rotation = look([0.0; 3], Vec3::ZERO, Vec3::ZERO, Some([2.0, 2.0, 0.0]));
    assert_eq!(rotation[0], -90.0);
    assert_eq!(rotation[1], -40.0);
    assert_eq!(rotation[2], rotation[0]);
}

const TICK: f64 = mc_physics::TICK_SECONDS;

/// Vanilla `MoveControl` turns a body toward the wanted position at 90 deg/tick
/// and never toward the step physics managed. Reverting the facing hunk in
/// `step` fails this: the previous rule faced the post-collision velocity.
#[test]
fn move_facing_faces_the_wanted_direction_not_the_resolved_step() {
    let world = Habitat(|x, y, _| {
        if y < 0 || (x == 1 && y < 5) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    // Touching a wall on +x: the diagonal drive keeps only its +z half.
    let mut state = input(MovementClass::Walker, Vec3::new(0.65, 0.0, 0.5));
    state.drive = [4.0, 0.0, 4.0];
    let moved = step(state, &world, |_, _| true);
    assert_eq!(moved.body.velocity.x, 0.0, "the wall zeroes +x");
    assert!(moved.body.velocity.z > 0.0, "and +z still carries");
    assert_eq!(
        moved.rotation[0], -45.0,
        "faces the wanted diagonal; the resolved step alone would face +z"
    );
}

/// Vanilla `MoveControl.MIN_SPEED` / `MIN_SPEED_SQR`: 5.0E-4 blocks per tick.
/// Below it the wanted point is reached and the body does not turn.
#[test]
fn move_facing_holds_below_the_vanilla_dead_zone() {
    let flat = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut slow = input(MovementClass::Walker, Vec3::new(0.5, 0.0, 0.5));
    slow.rotation = [37.0, 9.0, 37.0];
    slow.drive = [0.006, 0.0, 0.006]; // 4.2E-4 blocks per tick
    let held = step(slow, &flat, |_, _| true);
    assert_eq!(held.rotation[0], 37.0);
    let mut faster = input(MovementClass::Walker, Vec3::new(0.5, 0.0, 0.5));
    faster.rotation = [37.0, 9.0, 37.0];
    faster.drive = [0.012, 0.0, 0.012]; // 8.5E-4 blocks per tick
    let turned = step(faster, &flat, |_, _| true);
    assert_ne!(turned.rotation[0], 37.0, "above the dead zone it turns");
}

/// A knockback moves the body, not the wanted position: vanilla `MoveControl`
/// never turns toward it (Main, 2026-11-17).
#[test]
fn move_facing_ignores_a_knockback() {
    let flat = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut state = input(MovementClass::Walker, Vec3::new(0.5, 0.0, 0.5));
    state.impulse = [8.0, 0.0];
    let knocked = step(state, &flat, |_, _| true);
    assert!(knocked.body.position.x > 0.5, "it really slides +x");
    assert_eq!(knocked.rotation[0], 0.0, "and keeps facing +z");
    assert_eq!(
        look([0.0; 3], knocked.body.position, knocked.body.velocity, None)[0],
        -90.0,
        "the previous rule faced the slide"
    );
}

/// A body shoved in water keeps its facing: the wanted direction is the AI's
/// drive, never the knockback the swim navigation mixed in.
#[test]
fn move_facing_does_not_turn_in_water_from_a_knockback() {
    let water = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Water
        }
    });
    let mut state = input(MovementClass::Swimmer, Vec3::new(0.5, 1.0, 0.5));
    state.aquatic_control = AquaticControl::Fish;
    state.impulse = [8.0, 0.0];
    let knocked = step(state, &water, |_, _| true);
    assert!(knocked.body.velocity.x > 0.0, "it is pushed +x");
    assert_eq!(knocked.rotation[0], 0.0, "and keeps facing +z");
}

/// A body whose own controller keeps the head on the body (shulker, phantom,
/// happy ghast) never lags it.
#[test]
fn move_facing_keeps_the_head_on_the_body_for_a_special_controller() {
    let flat = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut state = input(MovementClass::Flyer, Vec3::new(0.5, 3.0, 0.5));
    state.head_follows_body = true;
    state.drive = [4.0, 0.0, 0.0];
    let moved = step(state, &flat, |_, _| true);
    assert_eq!(moved.rotation[0], -90.0);
    assert_eq!(moved.rotation[2], -90.0, "no head lag for that controller");
}

/// A swimmer that detours around a wall turns the way it swims: the drive is
/// mapped through the candidate the navigation picked (+90 deg here), not left
/// pointing into the wall. Reverting that mapping fails this.
#[test]
fn move_facing_follows_the_swim_detour_around_a_wall() {
    // Water only in the layer y 1..2; solid floor, ceiling and an x wall at 1.
    let world = Habitat(|x, y, _| {
        if !(1..2).contains(&y) || x >= 1 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Water
        }
    });
    let mut state = input(MovementClass::Swimmer, Vec3::new(0.6, 1.0, 0.5));
    state.aquatic_control = AquaticControl::Fish;
    state.rotation = [-90.0, 0.0, -90.0]; // facing the wall it wants to swim at
    state.drive = [4.0, 0.0, 0.0];
    let detoured = step(state, &world, |_, _| true);
    assert!(
        detoured.body.velocity.z > 0.0,
        "the navigation takes the +90 candidate"
    );
    assert!(
        detoured.body.position.z > 0.5,
        "and the body really swims sideways"
    );
    assert_eq!(
        detoured.rotation[0], 0.0,
        "the yaw follows the sideways run"
    );
}

/// A body in an unloaded column cannot move, so its head settles onto its body
/// instead of being stranded away from it (which would keep it out of the rest).
#[test]
fn unloaded_column_settles_the_head_on_the_body() {
    let air = Habitat(|_, _, _| BlockMaterial::Air);
    let mut state = input(MovementClass::Walker, Vec3::new(0.5, 64.0, 0.5));
    state.rotation = [30.0, 5.0, 70.0];
    let unloaded = step(state, &air, |_, _| false);
    assert_eq!(unloaded.rotation, [30.0, 5.0, 30.0]);
    assert_eq!(unloaded.body.velocity, Vec3::ZERO);
}

/// The yaw dead zone never freezes the pitch: a fall with nothing to walk to
/// still tips the head down.
#[test]
fn move_facing_holds_yaw_but_still_pitches_a_fall() {
    let air = Habitat(|_, _, _| BlockMaterial::Air);
    let mut state = input(MovementClass::Walker, Vec3::new(0.5, 5.0, 0.5));
    state.body.on_ground = false;
    let falling = step(state, &air, |_, _| true);
    assert_eq!(falling.rotation[0], 0.0, "no wanted horizontal travel");
    assert!(
        falling.rotation[1] > 10.0,
        "the fall pitches the body down (positive xRot is down)"
    );
}

/// An open-ground walk keeps the previous yaw and pitch bit for bit: only a
/// clipped or reversed step changes the facing.
#[test]
fn move_facing_steady_walk_keeps_the_old_yaw_and_pitch() {
    let flat = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut state = input(MovementClass::Walker, Vec3::new(0.5, 0.0, 0.5));
    state.rotation = [30.0, 5.0, 30.0];
    state.drive = [3.0, 0.0, 1.0];
    let moved = step(state, &flat, |_, _| true);
    let old = look(
        [30.0, 5.0, 30.0],
        moved.body.position,
        moved.body.velocity,
        None,
    );
    assert_eq!(
        [moved.rotation[0].to_bits(), moved.rotation[1].to_bits()],
        [old[0].to_bits(), old[1].to_bits()]
    );
}

/// Vanilla `LookControl` turns a head with nothing to look at toward the body at
/// 10 deg/tick; `BodyRotationControl` keeps it inside a 75 deg cone.
#[test]
fn move_facing_head_follows_the_body_at_the_vanilla_rate() {
    let flat = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut state = input(MovementClass::Walker, Vec3::new(0.5, 0.0, 0.5));
    state.drive = [4.0, 0.0, 0.0];
    let first = step(state, &flat, |_, _| true);
    assert_eq!(first.rotation[0], -90.0, "the body turns 90 deg at once");
    assert_eq!(first.rotation[2], -15.0, "the head stops 75 deg behind");
    let mut next = input(MovementClass::Walker, first.body.position);
    next.body = first.body;
    next.drive = [4.0, 0.0, 0.0];
    next.rotation = first.rotation;
    let second = step(next, &flat, |_, _| true);
    assert_eq!(second.rotation[0], -90.0);
    assert_eq!(
        second.rotation[2], -25.0,
        "10 deg/tick back toward the body"
    );
}

/// A body with nothing to look at and nothing to walk to keeps its facing while
/// the head settles back to the front.
#[test]
fn stationary_body_holds_its_facing_while_the_head_settles() {
    let flat = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut state = input(MovementClass::Stationary, Vec3::new(0.5, 0.0, 0.5));
    state.rotation = [0.0, 15.0, 60.0];
    let still = step(state, &flat, |_, _| true);
    assert_eq!([still.rotation[0], still.rotation[1]], [0.0, 15.0]);
    assert_eq!(still.rotation[2], 50.0);
}

/// Aiming keeps today's behaviour: the head rides the body once the body faces
/// what it looks at, which is vanilla's own steady state.
#[test]
fn look_keeps_the_head_on_the_body_while_a_target_is_aimed() {
    let aimed = look(
        [10.0, 0.0, 40.0],
        Vec3::ZERO,
        Vec3::ZERO,
        Some([5.0, 0.0, 0.0]),
    );
    assert_eq!(aimed[2], aimed[0]);
    assert_eq!(aimed[0], -80.0);
}

/// A battle body standing at (0.5, 0, 0.5) on flat ground: `previous` is the
/// velocity (blocks/s) it left the last tick with, `drive` (blocks/s) this
/// tick's standing velocity.
fn battle(rotation: [f32; 3], previous: [f64; 2], drive: [f64; 2]) -> MotionInput {
    let mut state = input(MovementClass::Walker, Vec3::new(0.5, 0.0, 0.5));
    state.body.velocity = Vec3::new(previous[0], 0.0, previous[1]);
    state.drive = [drive[0], 0.0, drive[1]];
    state.rotation = rotation;
    state.battle_facing = true;
    state.legacy_facing = true;
    state
}

fn battle_step(input: MotionInput, wall: bool) -> MotionResult {
    let world = Habitat(move |x, y, _| {
        if y < 0 || (wall && x == 1) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    step(input, &world, |_, _| true)
}

/// The same input with the battle rule off: the shared MoveControl facing.
fn old_step(mut input: MotionInput, wall: bool) -> MotionResult {
    input.battle_facing = false;
    input.legacy_facing = false;
    battle_step(input, wall)
}

fn bits(rotation: [f32; 3]) -> [u32; 3] {
    rotation.map(f32::to_bits)
}

/// Everything but the rotation is bit-identical: the rule never moves a body.
fn assert_same_motion(a: &MotionResult, b: &MotionResult) {
    let body = |r: &MotionResult| {
        [
            r.body.position.x,
            r.body.position.y,
            r.body.position.z,
            r.body.velocity.x,
            r.body.velocity.y,
            r.body.velocity.z,
            r.impulse[0],
            r.impulse[1],
        ]
        .map(f64::to_bits)
    };
    assert_eq!(body(a), body(b));
    assert_eq!(a.body.on_ground, b.body.on_ground);
    assert_eq!(a.air_supply, b.air_supply);
    assert_eq!(a.wall_climbing, b.wall_climbing);
}

fn turn(from: f32, to: f32) -> f32 {
    (to - from + 180.0).rem_euclid(360.0) - 180.0
}

/// Regression for the battle jitter (receipts/BattleJitterClient): a soldier
/// stepping back and forth around its place swung its facing up to 90 degrees
/// a tick with every step. Reverting the gate in `step` fails this test.
#[test]
fn battle_facing_holds_through_slot_oscillation_that_old_look_swings() {
    let rotation = [30.0, 12.0, 30.0];
    for per_tick in [0.007, 0.02, 0.07] {
        let speed = per_tick / TICK;
        let mut body = battle(rotation, [speed, 0.0], [0.0; 2]).body;
        let mut old_rotation = rotation;
        let mut widest_old_turn = 0.0_f32;
        for tick in 0..8 {
            let drive = [if tick % 2 == 0 { -speed } else { speed }, 0.0];
            let mut state = battle(rotation, [0.0; 2], drive);
            state.body = body;
            let held = battle_step(state, false);
            assert_eq!(
                bits(held.rotation),
                bits(rotation),
                "{per_tick} tick {tick}"
            );
            let mut state = battle(old_rotation, [0.0; 2], drive);
            state.body = body;
            let old = old_step(state, false);
            assert_same_motion(&held, &old);
            widest_old_turn = widest_old_turn.max(turn(old_rotation[0], old.rotation[0]).abs());
            (body, old_rotation) = (held.body, old.rotation);
        }
        assert_eq!(widest_old_turn, 90.0, "the old facing swings at {per_tick}");
    }
}

#[test]
fn battle_facing_turns_on_steady_walks_from_rest_like_the_shared_rule() {
    let rotation = [180.0, 12.0, 180.0];
    for per_tick in [0.0051, 0.02, 0.04, 0.1] {
        let speed = per_tick / TICK;
        let (mut body, mut facing) = (battle(rotation, [0.0; 2], [0.0; 2]).body, rotation);
        for tick in 0..4 {
            let mut state = battle(facing, [0.0; 2], [speed, 0.0]);
            state.body = body;
            let moved = battle_step(state, false);
            let mut state = battle(facing, [0.0; 2], [speed, 0.0]);
            state.body = body;
            let shared = old_step(state, false);
            assert_same_motion(&moved, &shared);
            // The head is the only axis the two rules split (the shared rule
            // lags it at LookControl's 10 deg/tick).
            assert_eq!(
                [moved.rotation[0].to_bits(), moved.rotation[1].to_bits()],
                [shared.rotation[0].to_bits(), shared.rotation[1].to_bits()],
                "{per_tick} tick {tick}"
            );
            assert_ne!(moved.rotation[0], facing[0], "{per_tick} tick {tick} turns");
            (body, facing) = (moved.body, moved.rotation);
            if facing[0].rem_euclid(360.0) == 270.0 {
                break;
            }
        }
        assert_eq!(facing[0].rem_euclid(360.0), 270.0, "{per_tick} faces +x");
    }
}

#[test]
fn battle_facing_holds_a_turn_around_for_one_tick_then_turns() {
    let speed = 0.04 / TICK;
    let walking = [-90.0, 0.0, -90.0];
    let first = battle_step(battle(walking, [speed, 0.0], [-speed, 0.0]), false);
    assert!(first.body.position.x < 0.5);
    assert_eq!(
        bits(first.rotation),
        bits(walking),
        "the reversing step holds"
    );
    let mut state = battle(first.rotation, [0.0; 2], [-speed, 0.0]);
    state.body = first.body;
    let second = battle_step(state, false);
    assert_eq!(second.rotation[0], -180.0, "the continuing step turns");
    let mut state = battle(second.rotation, [0.0; 2], [-speed, 0.0]);
    state.body = second.body;
    let third = battle_step(state, false);
    assert_eq!(third.rotation[0].rem_euclid(360.0), 90.0, "faces -x");
}

#[test]
fn battle_facing_floor_and_reversal_boundaries() {
    let rotation = [30.0, 12.0, 30.0];
    let holds = |previous: [f64; 2], per_tick: [f64; 2]| {
        let drive = per_tick.map(|v| v / TICK);
        let previous = previous.map(|v| v / TICK);
        bits(battle_step(battle(rotation, previous, drive), false).rotation) == bits(rotation)
    };
    assert!(holds([0.0; 2], [0.0; 2]), "still");
    assert!(holds([0.0; 2], [0.0049, 0.0]), "below the floor");
    assert!(
        !holds([0.0; 2], [0.0051, 0.0]),
        "above the floor, from rest"
    );
    assert!(
        holds([0.04, 0.0], [0.0049, 0.0]),
        "below the floor, continuing"
    );
    assert!(
        !holds([0.0, 0.04], [0.02, 0.0]),
        "perpendicular to the last step"
    );
    assert!(
        holds([-0.04, 0.04], [0.02, 0.0199]),
        "just against the last step"
    );
    assert!(
        !holds([-0.04, 0.04], [0.02, 0.0201]),
        "just along the last step"
    );
}

/// A step of exactly the floor holds; this pins `<=` against `<`.
#[test]
fn battle_facing_holds_a_step_exactly_on_the_floor() {
    let rotation = [30.0, 12.0, 30.0];
    // From x = 0 the resolved step is the new position itself; some speed
    // within a few ulps of .005 block/tick lands on exactly 0.005.
    let speed = (0.005 / TICK).to_bits();
    let exact = (speed - 32..speed + 32)
        .map(f64::from_bits)
        .find_map(|speed| {
            let mut state = battle(rotation, [0.0; 2], [speed, 0.0]);
            state.body.position.x = 0.0;
            let moved = battle_step(state, false);
            (moved.body.position.x == 0.005 && moved.body.position.z == 0.5).then_some(moved)
        })
        .expect("a speed whose step is exactly the floor");
    assert_eq!(bits(exact.rotation), bits(rotation));
}

#[test]
fn battle_facing_with_a_target_turns_like_old_look_even_without_a_step() {
    let rotation = [180.0, 12.0, 180.0];
    let make = || {
        let mut state = battle(rotation, [0.8, 0.0], [0.0; 2]);
        state.look_target = Some([5.5, 1.0, 0.5]);
        state
    };
    let with_rule = battle_step(make(), false);
    let old = old_step(make(), false);
    assert_same_motion(&with_rule, &old);
    assert_eq!(bits(with_rule.rotation), bits(old.rotation));
    assert_eq!(with_rule.rotation[0].rem_euclid(360.0), 270.0);
}

/// A knockback is not a wanted position. The battle rule holds as before and the
/// shared rule now holds too; the previous tree turned the body toward the slide.
#[test]
fn battle_facing_holds_against_knockback_without_changing_motion() {
    let speed = 0.04 / TICK;
    let walking = [-90.0, 0.0, -90.0];
    let make = || {
        let mut state = battle(walking, [speed, 0.0], [speed, 0.0]);
        state.impulse = [-8.0, 0.0];
        state
    };
    let held = battle_step(make(), false);
    let shared = old_step(make(), false);
    assert!(held.body.position.x < 0.5, "knocked back");
    assert_same_motion(&held, &shared);
    assert_eq!(bits(held.rotation), bits(walking));
    assert_eq!(
        shared.rotation[0], -90.0,
        "the shared rule faces the drive, not the knockback"
    );
    assert_eq!(
        look(walking, held.body.position, held.body.velocity, None)[0],
        -180.0,
        "the previous tree faced the knockback"
    );
}

/// A standing battle row is not hold-eligible, but it is still a battle row and
/// keeps the legacy look at the resolved velocity: an impulse turns it. Tying
/// the legacy facing to the hold's selector would freeze its yaw instead.
#[test]
fn standing_battle_row_keeps_the_legacy_facing_on_an_impulse() {
    let flat = Habitat(|_, y, _| {
        if y < 0 {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    });
    let mut state = input(MovementClass::Walker, Vec3::new(0.5, 0.0, 0.5));
    state.battle_facing = false;
    state.legacy_facing = true;
    state.impulse = [8.0, 0.0];
    let moved = step(state, &flat, |_, _| true);
    let legacy = look([0.0; 3], moved.body.position, moved.body.velocity, None);
    assert_eq!(
        moved.rotation[0], legacy[0],
        "the legacy look, not the hold"
    );
    assert_eq!(moved.rotation[1], legacy[1], "pitch as well");
    assert!(moved.rotation[0] < -10.0, "the impulse turns the body");
}

/// The battle policy stays frozen: a sliding step faces the resolved velocity
/// (as `de660fabd` did), while a non-battle body faces the direction it wants.
#[test]
fn battle_facing_against_a_wall_follows_the_resolved_velocity() {
    let speed = 0.04 / TICK;
    let rotation = [30.0, 12.0, 30.0];
    let make = |x: f64| {
        let mut state = battle(rotation, [speed, speed], [speed, speed]);
        state.body.position.x = x;
        state
    };
    let slid = battle_step(make(0.69), true);
    let shared = old_step(make(0.69), true);
    assert_same_motion(&slid, &shared);
    assert_eq!(slid.rotation[0], 0.0, "battle: along the wall");
    assert_eq!(shared.rotation[0], -45.0, "shared: the wanted diagonal");
    // Head-on at contact: no step, so both keep the facing.
    let make = || {
        let mut state = battle(rotation, [speed, 0.0], [speed, 0.0]);
        state.body.position.x = 0.7;
        state
    };
    let blocked = battle_step(make(), true);
    let shared = old_step(make(), true);
    assert_same_motion(&blocked, &shared);
    assert_eq!(bits(blocked.rotation), bits(rotation));
    assert_eq!(
        shared.rotation[0], -60.0,
        "the shared rule faces the wall it keeps pushing"
    );
}

/// The battle policy is `look` of the resolved body plus the hold: on every grid
/// case the motion is identical, and the rotation is either the held input
/// rotation or exactly `look` of the resolved velocity.
#[test]
fn battle_facing_is_the_previous_look_or_the_hold() {
    let per_tick = [0.0, 0.003, 0.0049, 0.0051, 0.02, 0.07, 0.3];
    let rotations = [[0.0; 3], [30.0, 12.0, 30.0], [-170.0, -45.0, -170.0]];
    let mut held = 0;
    let mut followed = 0;
    for wall in [false, true] {
        for rotation in rotations {
            for &p in &per_tick {
                for previous in [[0.0, 0.0], [p, 0.0], [-p, 0.5 * p]] {
                    for d in [[0.0, 0.0], [p, 0.0], [-p, 0.0], [0.6 * p, -0.8 * p]] {
                        for impulse in [[0.0, 0.0], [-8.0, 3.0]] {
                            for target in [None, Some([2.5, 1.0, -3.5])] {
                                let make = || {
                                    let mut state = battle(
                                        rotation,
                                        previous.map(|v| v / TICK),
                                        d.map(|v| v / TICK),
                                    );
                                    state.body.position.x = if wall { 0.68 } else { 0.5 };
                                    state.impulse = impulse;
                                    state.look_target = target;
                                    state
                                };
                                let start = make().body;
                                let on = battle_step(make(), wall);
                                let shared = old_step(make(), wall);
                                assert_same_motion(&on, &shared);
                                let dx = on.body.position.x - start.position.x;
                                let dz = on.body.position.z - start.position.z;
                                let hold = target.is_none()
                                    && (dx * dx + dz * dz <= 0.005 * 0.005
                                        || dx * start.velocity.x + dz * start.velocity.z < 0.0);
                                let expected = if hold {
                                    held += 1;
                                    rotation
                                } else {
                                    followed += 1;
                                    look(rotation, on.body.position, on.body.velocity, target)
                                };
                                assert_eq!(bits(on.rotation), bits(expected));
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(held > 0 && followed > 0);
}

#[test]
fn episodic_wander_produces_vertical_drive_only_for_three_dimensional_kinds() {
    for kind in [
        MovementClass::Flyer,
        MovementClass::Swimmer,
        MovementClass::Amphibious,
    ] {
        assert!((0..240).any(|tick| wander(kind, 1, tick, 4.0)[1].abs() > 0.001));
    }
    assert!((0..240).all(|tick| wander(MovementClass::Walker, 1, tick, 4.0)[1] == 0.0));
}

struct NativeHabitat {
    air: u32,
    ground: u32,
    water: u32,
}

impl mc_world::ChunkGenerator for NativeHabitat {
    fn generate(&self, pos: mc_world::ChunkPos) -> mc_world::Chunk {
        let mut chunk = mc_world::Chunk::empty(
            pos,
            mc_world::BlockStateId(self.air),
            mc_data::Identifier::parse("minecraft:plains").unwrap(),
        );
        for x in 0..16 {
            for z in 0..16 {
                chunk.set_block(x, 64, z, mc_world::BlockStateId(self.ground));
                if (8..16).contains(&(pos.x * 16 + x as i32)) {
                    for y in 65..73 {
                        chunk.set_block(x, y, z, mc_world::BlockStateId(self.water));
                    }
                }
            }
        }
        chunk
    }
}

fn native_state() -> crate::tick::TickState {
    use std::sync::Arc;
    let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let generator: Arc<dyn mc_world::ChunkGenerator> = Arc::new(NativeHabitat {
        air: registry.states.air,
        ground: registry.states.stone,
        water: registry.states.water,
    });
    let world = crate::world::ChunkWorld::new(
        42,
        generator,
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [4, 65, 4],
        mc_world::OVERWORLD_GEOMETRY,
    );
    let mut state = crate::tick::TickState::new(registry, world, 1, 512);
    state.tickets.insert(
        "motion".to_owned(),
        (crate::boundary::ChunkPos { x: 0, z: 0 }, 1),
    );
    let wanted = state.desired_chunks();
    state.world.ensure_blocking(&wanted);
    state
}

fn spawn_named(
    state: &mut crate::tick::TickState,
    name: &str,
    position: [f64; 3],
) -> crate::entities::EntitySlot {
    let type_id = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse(name).unwrap())
        .unwrap();
    let id = state.spawn_mob(type_id, position, 20.0);
    state.entities.slot_of(id).unwrap()
}

#[test]
fn native_tick_projects_blueprint_movement_and_rotation_in_each_habitat() {
    let mut state = native_state();
    let cow = spawn_named(&mut state, "minecraft:cow", [4.0, 65.0, 4.0]);
    let cod = spawn_named(&mut state, "minecraft:cod", [12.0, 68.0, 4.0]);
    let bat = spawn_named(&mut state, "minecraft:bat", [4.0, 69.0, 4.0]);
    let frog = spawn_named(&mut state, "minecraft:frog", [4.0, 65.0, 4.0]);
    let stand = spawn_named(&mut state, "minecraft:armor_stand", [6.0, 65.0, 4.0]);
    let slots = [cow, cod, bat, frog, stand];
    let starts = slots.map(|slot| state.entities.pos(slot));
    let mut moved = [false; 5];
    let mut vertical = [false; 5];
    for _ in 0..120 {
        crate::tick::run_tick(&mut state, &[]);
        for (index, slot) in slots.into_iter().enumerate() {
            let position = state.entities.pos(slot);
            moved[index] |= position != starts[index];
            vertical[index] |= (position[1] - starts[index][1]).abs() > 0.001;
        }
    }
    assert_eq!(moved, [true, true, true, true, false]);
    assert!(vertical[1] && vertical[2]);
    assert_eq!(state.entities.rot(stand), [0.0; 3]);
    assert_ne!(state.entities.rot(cod), [0.0; 3]);
    assert_ne!(state.entities.rot(bat), [0.0; 3]);
}

#[test]
fn native_tick_commits_aquatic_dry_air_damage_and_retains_counter() {
    let mut state = native_state();
    let cod = spawn_named(&mut state, "minecraft:cod", [4.0, 65.0, 4.0]);
    state.entities.body_mut(cod).air_supply = -19;
    state.entities.set_ground(cod, true);
    crate::tick::run_tick(&mut state, &[]);
    assert_eq!(state.entities.body(cod).air_supply, 0);
    assert_eq!(state.entities.row(cod).health, 18.0);
}

#[test]
fn panic_has_no_wander_idle_window_and_moves_away_from_known_attacker() {
    for tick in 0..240 {
        let drive = panic(MovementClass::Walker, 1, tick, 5.0, [0.0; 3], None);
        // The mth sin/cos tables are not exactly unit length (angle step 2π/65536).
        assert!((mc_physics::mth::length2(drive[0], drive[2]) - 5.0).abs() < 1.0e-3);
    }
    assert_eq!(
        panic(
            MovementClass::Walker,
            1,
            60,
            5.0,
            [1.0, 0.0, 0.0],
            Some([0.0; 3])
        ),
        [5.0, 0.0, 0.0]
    );
    assert_eq!(
        panic(
            MovementClass::Stationary,
            1,
            60,
            5.0,
            [1.0, 0.0, 0.0],
            Some([0.0; 3])
        ),
        [0.0; 3]
    );
}

#[test]
fn native_flee_fire_goal_panics_after_hurt_or_while_burning_even_during_wander_idle() {
    for burning in [false, true] {
        let mut state = native_state();
        let cow = spawn_named(&mut state, "minecraft:cow", [4.0, 65.0, 4.0]);
        let id = state.entities.body(cow).id;
        let speed = state.entities.row(cow).speed;
        state.tick = 60;
        assert_eq!(
            wander(MovementClass::Walker, id.0, state.tick, speed),
            [0.0; 3]
        );
        let body = state.entities.body_mut(cow);
        if burning {
            body.burn_ticks = 20;
        } else {
            body.hurt_at = Some(60);
        }
        state.entities.set_ground(cow, true);
        crate::tick::run_tick(&mut state, &[]);
        let velocity = state.entities.vel(cow);
        let expected = f64::from(speed) * if burning { 2.0 } else { 1.25 };
        assert!((mc_physics::mth::length2(velocity[0], velocity[2]) - expected).abs() < 1.0e-7);
    }
}

struct CowPenTerrain {
    air: u32,
    stone: u32,
}

impl mc_world::ChunkGenerator for CowPenTerrain {
    fn generate(&self, pos: mc_world::ChunkPos) -> mc_world::Chunk {
        let mut chunk = mc_world::Chunk::empty(
            pos,
            mc_world::BlockStateId(self.air),
            mc_data::Identifier::parse("minecraft:plains").unwrap(),
        );
        for x in 0..16 {
            for z in 0..16 {
                chunk.set_block(x, 48, z, mc_world::BlockStateId(self.stone));
                // The same pen spans air over low terrain and a hill to clear.
                if pos == (mc_world::ChunkPos { x: 0, z: 0 })
                    && (8..=11).contains(&x)
                    && (4..=5).contains(&z)
                {
                    for y in 49..=65 {
                        chunk.set_block(x, y, z, mc_world::BlockStateId(self.stone));
                    }
                }
            }
        }
        chunk
    }
}

#[test]
fn same_tick_fill_and_summon_keeps_cows_healthy_above_the_pen_floor() {
    use std::sync::Arc;
    let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let generator = Arc::new(CowPenTerrain {
        air: registry.states.air,
        stone: registry.states.stone,
    });
    let world = crate::world::ChunkWorld::new(
        42,
        generator,
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [8, 64, 8],
        mc_world::OVERWORLD_GEOMETRY,
    );
    let mut state = crate::tick::TickState::new(registry, world, 1, 512);
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    state
        .gamerules
        .insert("random_tick_speed".into(), "0".into());
    state.tickets.insert(
        "cow-pen".into(),
        (crate::boundary::ChunkPos { x: 0, z: 0 }, 1),
    );
    state.world.ensure_blocking(&state.desired_chunks());
    // coreperf sends fill and the first summons together in a 32-command batch.
    let commands = [
        "fill 0 63 0 15 63 15 minecraft:grass_block",
        "fill 0 64 0 15 68 15 minecraft:air",
        "fill -1 64 -1 16 65 -1 minecraft:stone",
        "fill -1 64 16 16 65 16 minecraft:stone",
        "fill -1 64 0 -1 65 15 minecraft:stone",
        "fill 16 64 0 16 65 15 minecraft:stone",
        "summon minecraft:cow 4.5 64 4.5",
        "summon minecraft:cow 6.5 64 4.5",
        "summon minecraft:cow 8.5 64 4.5",
        "summon minecraft:cow 10.5 64 4.5",
    ];
    let inputs: Vec<_> = commands
        .into_iter()
        .map(|text| {
            crate::boundary::PlayerInput::ConsoleCommand(
                crate::command_console::ConsoleCommand::new(text.into()).0,
            )
        })
        .collect();
    crate::tick::run_tick(&mut state, &inputs);
    let cows: Vec<_> = state
        .entities
        .slots()
        .map(|slot| state.entities.body(slot).id)
        .collect();
    assert_eq!(cows.len(), 4);
    let mut first_breach = None;
    let mut first_damage = None;
    for _ in 0..80 {
        for &id in &cows {
            let slot = state.entities.slot_of(id).expect("cow survives");
            let pos = state.entities.pos(slot);
            let width = state.entities.body(slot).half_width;
            if first_breach.is_none()
                && (pos[1] < 64.0
                    || pos[0] - width < 0.0
                    || pos[0] + width > 16.0
                    || pos[2] - width < 0.0
                    || pos[2] + width > 16.0)
            {
                first_breach = Some((state.tick, id, pos));
            }
        }
        let mut hazards = Vec::new();
        crate::contact_hazards::run(&mut state, &mut hazards);
        if first_damage.is_none() {
            first_damage = hazards
                .into_iter()
                .next()
                .map(|intent| (state.tick, intent.kind));
        }
        crate::tick::run_tick(&mut state, &[]);
    }
    let final_cows: Vec<_> = cows
        .iter()
        .map(|&id| {
            let slot = state.entities.slot_of(id).expect("cow survives");
            (
                id,
                state.entities.pos(slot),
                state.entities.row(slot).health,
            )
        })
        .collect();
    eprintln!(
        "cow-pen: ticks={} first_breach={first_breach:?} first_damage={first_damage:?} final={final_cows:?}",
        state.tick
    );
    assert!(
        first_breach.is_none(),
        "accepted command blocks must contain cows before physics"
    );
    assert!(
        first_damage.is_none(),
        "cows must not suffocate in cleared terrain"
    );
    assert!(final_cows.iter().all(|(_, _, health)| *health == 10.0));
}

#[test]
fn wall_climbing_spiders_do_not_accumulate_separate_drops_before_landing() {
    let mut state = native_state();
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    state
        .gamerules
        .insert("random_tick_speed".into(), "0".into());
    let air = state.registry.states.air;
    let stone = state.registry.states.stone;
    // Three blocks of headroom: a spider's largest possible free fall is
    // 2.1 blocks. Repeated wall/ceiling contact must not add those drops.
    for x in -1..=16 {
        for z in -1..=16 {
            for y in 64..=72 {
                let solid = y == 64 || y == 68 || x == -1 || x == 16 || z == -1 || z == 16;
                state
                    .world
                    .set_block(x, y, z, if solid { stone } else { air });
            }
        }
    }
    let spiders: Vec<_> = (0..64)
        .map(|n| {
            let slot = spawn_named(
                &mut state,
                "minecraft:spider",
                [
                    0.8 + f64::from(n % 8) * 2.0,
                    65.0,
                    0.8 + f64::from(n / 8) * 2.0,
                ],
            );
            state.entities.row_mut(slot).health = 16.0;
            state.entities.row_mut(slot).persistent = true;
            state.entities.body(slot).id
        })
        .collect();
    let mut climbed = false;
    for _ in 0..4000 {
        crate::tick::run_tick(&mut state, &[]);
        for &id in &spiders {
            let slot = state.entities.slot_of(id).expect("fixture spider survives");
            climbed |= state.entities.pos(slot)[1] > 66.0;
            assert_eq!(
                state.entities.row(slot).health,
                16.0,
                "spider {id:?} took damage in a fall-safe pen at tick {}",
                state.tick,
            );
        }
    }
    assert!(
        climbed,
        "the ordinary AI must actually exercise wall climbing"
    );
}

/// D36: a projected battle row writes the published facing whatever its other
/// rules would do; on a tick it really moves (the client's `isMoving`) its
/// yaw is the published body, which the client draws from the sent yaw.
#[test]
fn projected_facing_is_written_and_a_moving_tick_writes_the_body_yaw() {
    use crate::entity_motion::Facing;
    let set = Facing::Set {
        rotation: [10.0, -4.0, 40.0],
        body: 25.0,
    };
    let mut standing = battle([0.0; 3], [0.0, 0.0], [0.0, 0.0]);
    standing.look_target = Some([2.5, 1.0, -3.5]);
    standing.facing = Some(set);
    let held = battle_step(standing, false);
    assert_eq!(bits(held.rotation), bits([10.0, -4.0, 40.0]));
    let mut pushed = battle([0.0; 3], [0.0, 0.0], [0.0, 0.0]);
    pushed.look_target = Some([2.5, 1.0, -3.5]);
    pushed.impulse = [-8.0, 3.0];
    pushed.facing = Some(set);
    let moved = battle_step(pushed, false);
    assert!(moved.body.position.x < 0.5, "the impulse moves the body");
    assert_eq!(bits(moved.rotation), bits([25.0, -4.0, 40.0]));
    let mut stationary = input(MovementClass::Stationary, Vec3::new(0.5, 0.0, 0.5));
    stationary.look_target = Some([2.5, 1.0, -3.5]);
    stationary.facing = Some(set);
    let stationary = battle_step(stationary, false);
    assert_eq!(
        bits(stationary.rotation),
        bits([10.0, -4.0, 40.0]),
        "Stationary too"
    );
}

/// D36 3.3: a row leaving the projection under the lazy policy starts its own
/// rule from the drawn body, not from its stale stored yaw.
#[test]
fn a_row_leaving_the_projection_turns_from_the_drawn_body() {
    use crate::entity_motion::Facing;
    // Stored yaw 0 (stale), drawn body 60, head 70; the target is near -80.
    let mut driven = battle([0.0, 0.0, 70.0], [0.0, 0.0], [-1.0, 0.0]);
    driven.look_target = Some([5.5, 0.0, 1.38]);
    driven.facing = Some(Facing::From { yaw: 60.0 });
    let from_body = battle_step(driven, false);
    let mut stale = battle([0.0, 0.0, 70.0], [0.0, 0.0], [-1.0, 0.0]);
    stale.look_target = Some([5.5, 0.0, 1.38]);
    let from_stale = battle_step(stale, false);
    let expected = look(
        [60.0, 0.0, 70.0],
        from_body.body.position,
        from_body.body.velocity,
        Some([5.5, 0.0, 1.38]),
    );
    assert_eq!(bits(from_body.rotation), bits(expected));
    assert_ne!(from_body.rotation[0], from_stale.rotation[0]);
}

/// D36 3.4.3: with nothing projected (no target, battle paused or stopped) a
/// battle row's head left off its body returns at 10 degrees per tick and
/// lands on it bit for bit, so the row can rest again within 8 ticks.
#[test]
fn a_battle_head_off_its_body_returns_without_a_target() {
    let mut rotation: [f32; 3] = [20.0, 0.0, 95.0];
    let mut ticks = 0;
    while rotation[2].to_bits() != rotation[0].to_bits() {
        let state = battle(rotation, [0.0, 0.0], [0.0, 0.0]);
        let next = battle_step(state, false).rotation;
        assert_eq!(next[0].to_bits(), rotation[0].to_bits(), "the body holds");
        assert!((next[2] - rotation[2]).abs() <= 10.0 + 1.0e-4);
        rotation = next;
        ticks += 1;
        assert!(ticks <= 8, "{rotation:?}");
    }
    // A head already on its body keeps the old rule (no recentring path).
    let on_body = battle_step(battle([20.0, 0.0, 20.0], [0.0, 0.0], [0.0, 0.0]), false);
    assert_eq!(bits(on_body.rotation), bits([20.0, 0.0, 20.0]));
}

#[test]
fn a_hoppers_wander_heading_survives_the_walker_dwell_phase() {
    // Vanilla keeps a wanted direction for a whole stroll and `SlimeMoveControl`
    // has no ground traction between hops, so a hop is a hopper's only
    // translation. A dwell-phase heading of zero therefore freezes a slime
    // wherever its jump clock happens not to line up with the walker window.
    let mut dwell = 0;
    for tick in 0..240 {
        let heading = wander_heading(MovementClass::Walker, 7, tick, 0.52);
        let walker = wander(MovementClass::Walker, 7, tick, 0.52);
        assert_eq!(heading[1], 0.0, "a walker hop keeps its feet level: {tick}");
        if walker == [0.0; 3] {
            dwell += 1;
            assert_ne!(
                heading, [0.0; 3],
                "tick {tick}: a dwell-phase hop still carries the stroll heading"
            );
        }
    }
    assert!(dwell > 0, "the fixture must cover dwell ticks");
}
