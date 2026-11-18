//! Native bounded navigation and look/move/jump controls for blueprint bodies.
//! No session state or per-tick allocation: collision uses the shared physics
//! kernel; swimming probes require known water across the whole swept body.

use crate::blueprint::MovementClass;
use crate::species::AquaticControl;
use mc_physics::mth::{self, floor_i32};
use mc_physics::{Aabb, BlockMaterial, BlockSampler, EntityBody, PhysicsConfig, Vec3};

const EPSILON: f64 = 1.0e-7;
const WANDER_EPISODE: u64 = 120;
const WANDER_WALK: u64 = 40;
/// Shortest step (blocks per tick) a battle body without a look target turns
/// to face; see [`battle_holds_facing`].
const BATTLE_HOLD_STEP: f64 = 0.005;
/// A step-length threshold, not vanilla's wanted-distance test: the horizontal
/// distance the AI asks to travel this tick, in blocks per tick (vanilla
/// `MoveControl.MIN_SPEED`). Below it a body keeps its yaw; vanilla compares the
/// distance to the *wanted point* (`MIN_SPEED_SQR` 2.5e-7 on the square), which
/// this executor does not carry — it carries a drive velocity.
const MIN_WANTED_STEP: f64 = 5.0e-4;
/// Vanilla `LookControl`'s head turn rate toward the body, degrees per tick.
const HEAD_TURN: f32 = 10.0;
/// Vanilla `Mob.getMaxHeadYRot`: a head never looks further off the body.
const MAX_HEAD_YAW: f32 = 75.0;

pub(crate) struct MotionInput {
    pub body: EntityBody,
    pub movement: MovementClass,
    /// Standing locomotion decision, separate from combat momentum.
    pub drive: [f64; 3],
    pub impulse: [f64; 2],
    pub rotation: [f32; 3],
    pub look_target: Option<[f64; 3]>,
    /// A moving battle body: without a look target its facing follows
    /// [`battle_holds_facing`].
    pub battle_facing: bool,
    /// The mob's own body controller keeps the head on the body
    /// ([`Capabilities::head_follows_body`]); no head lag applies.
    pub head_follows_body: bool,
    pub climbs_walls: bool,
    pub aquatic_control: AquaticControl,
    pub air_supply: i32,
    pub levitation: f32,
    pub slow_falling: bool,
    pub no_gravity: bool,
    pub id: i32,
    pub tick: u64,
}

pub(crate) struct MotionResult {
    pub body: EntityBody,
    pub impulse: [f64; 2],
    pub rotation: [f32; 3],
    pub air_supply: i32,
    pub drowning: bool,
    /// Actual loaded-wall contact, including a climb clipped by a ceiling.
    pub wall_climbing: bool,
}

pub(crate) fn unit(id: i32, epoch: u64) -> f64 {
    let mut hash = (id as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(epoch.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    hash ^= hash >> 29;
    hash = hash.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    hash ^= hash >> 32;
    hash as f64 / u64::MAX as f64
}

pub(crate) fn wander(movement: MovementClass, id: i32, tick: u64, speed: f32) -> [f64; 3] {
    if matches!(
        movement,
        MovementClass::Stationary
            | MovementClass::Item
            | MovementClass::Player
            | MovementClass::Projectile
    ) {
        return [0.0; 3];
    }
    let phase = tick.wrapping_add((id as u64).wrapping_mul(0x9E37_79B9));
    if phase % WANDER_EPISODE >= WANDER_WALK {
        return [0.0; 3];
    }
    let epoch = phase / WANDER_EPISODE;
    let angle = unit(id, epoch) * std::f64::consts::TAU;
    let speed = f64::from(speed) * 0.25;
    [
        f64::from(mth::cos(angle)) * speed,
        if matches!(
            movement,
            MovementClass::Flyer | MovementClass::Swimmer | MovementClass::Amphibious
        ) {
            (unit(id, epoch.wrapping_add(0x41)) * 2.0 - 1.0) * speed * 0.5
        } else {
            0.0
        },
        f64::from(mth::sin(angle)) * speed,
    ]
}

pub(crate) fn steer(
    movement: MovementClass,
    position: [f64; 3],
    target: [f64; 3],
    speed: f64,
) -> [f64; 3] {
    if movement == MovementClass::Stationary {
        return [0.0; 3];
    }
    let dy = if matches!(
        movement,
        MovementClass::Flyer | MovementClass::Swimmer | MovementClass::Amphibious
    ) {
        target[1] - position[1]
    } else {
        0.0
    };
    let delta = [target[0] - position[0], dy, target[2] - position[2]];
    let length = (delta[0] * delta[0] + dy * dy + delta[2] * delta[2]).sqrt();
    let scale =
        (speed * mc_physics::TICK_SECONDS / length.max(0.001)).min(1.0) / mc_physics::TICK_SECONDS;
    delta.map(|value| value * scale)
}

/// Panic has no idle part of a wander episode. A known attacker supplies the
/// escape direction; otherwise use the entity's deterministic episode heading.
pub(crate) fn panic(
    movement: MovementClass,
    id: i32,
    tick: u64,
    speed: f64,
    position: [f64; 3],
    source: Option<[f64; 3]>,
) -> [f64; 3] {
    if movement == MovementClass::Stationary {
        return [0.0; 3];
    }
    if let Some(source) = source {
        let target = [
            position[0] + (position[0] - source[0]) * 16.0,
            position[1] + (position[1] - source[1]) * 16.0,
            position[2] + (position[2] - source[2]) * 16.0,
        ];
        let drive = steer(movement, position, target, speed);
        if drive.iter().any(|value| value.abs() > EPSILON) {
            return drive;
        }
    }
    let epoch = tick / WANDER_EPISODE;
    let angle = unit(id, epoch) * std::f64::consts::TAU;
    [
        f64::from(mth::cos(angle)) * speed,
        if matches!(
            movement,
            MovementClass::Flyer | MovementClass::Swimmer | MovementClass::Amphibious
        ) {
            (unit(id, epoch.wrapping_add(0x41)) * 2.0 - 1.0) * speed * 0.5
        } else {
            0.0
        },
        f64::from(mth::sin(angle)) * speed,
    ]
}

/// AbstractFish's dry-air boundary; amphibious bodies do not suffocate on land.
pub(crate) fn breathe(air: i32, water: bool, movement: MovementClass) -> (i32, bool) {
    if movement != MovementClass::Swimmer || water {
        return (300, false);
    }
    let next = air.saturating_sub(1);
    if next <= -20 {
        (0, true)
    } else {
        (next, false)
    }
}

fn water_body<S: BlockSampler>(sampler: &S, position: Vec3, aabb: Aabb) -> bool {
    if !position.x.is_finite()
        || !position.y.is_finite()
        || !position.z.is_finite()
        || !aabb.half_width.is_finite()
        || !aabb.height.is_finite()
        || aabb.half_width <= 0.0
        || aabb.height <= 0.0
    {
        return false;
    }
    let min = [
        floor_i32(position.x - aabb.half_width + EPSILON),
        floor_i32(position.y + EPSILON),
        floor_i32(position.z - aabb.half_width + EPSILON),
    ];
    let max = [
        floor_i32(position.x + aabb.half_width - EPSILON),
        floor_i32(position.y + aabb.height - EPSILON),
        floor_i32(position.z + aabb.half_width - EPSILON),
    ];
    let cells = (i64::from(max[0]) - i64::from(min[0]) + 1)
        .saturating_mul(i64::from(max[1]) - i64::from(min[1]) + 1)
        .saturating_mul(i64::from(max[2]) - i64::from(min[2]) + 1);
    if !(1..=4096).contains(&cells) {
        return false;
    }
    for x in min[0]..=max[0] {
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                if sampler.material_at(x, y, z) != BlockMaterial::Water {
                    return false;
                }
            }
        }
    }
    true
}

fn water_path<S: BlockSampler>(sampler: &S, body: EntityBody, velocity: Vec3) -> bool {
    let dt = mc_physics::TICK_SECONDS;
    let span = velocity.x.abs().max(velocity.y.abs()).max(velocity.z.abs()) * dt;
    if !span.is_finite() || span > 2.0 {
        return false;
    }
    let steps = (span / 0.2).ceil().max(1.0) as u32;
    (1..=steps).all(|step| {
        let t = dt * f64::from(step) / f64::from(steps);
        water_body(
            sampler,
            Vec3::new(
                body.position.x + velocity.x * t,
                body.position.y + velocity.y * t,
                body.position.z + velocity.z * t,
            ),
            body.aabb,
        )
    })
}

/// The linear candidate map [`swim_navigation`] chooses from. Every candidate is
/// a fixed linear image of the requested velocity, so the same map applies to the
/// drive when the body picks its facing: a swimmer turns the way it actually
/// swims, not into the wall its request points at.
fn swim_candidate(index: usize, wanted: Vec3) -> Vec3 {
    match index {
        1 => Vec3::new(wanted.x, 0.0, wanted.z),
        2 => Vec3::new(0.0, wanted.y, 0.0),
        3 => Vec3::new(-wanted.z, wanted.y, wanted.x),
        4 => Vec3::new(wanted.z, wanted.y, -wanted.x),
        5 => Vec3::new(-wanted.x, -wanted.y, -wanted.z),
        _ => wanted,
    }
}

/// Deterministic local alternatives, not an unbounded path search or A*.
/// Returns the index of the first candidate that stays in water; `None` when no
/// candidate does (the body then stops).
fn swim_navigation<S: BlockSampler>(sampler: &S, body: EntityBody, wanted: Vec3) -> Option<usize> {
    (0..6).find(|&index| {
        let candidate = swim_candidate(index, wanted);
        candidate != Vec3::ZERO && water_path(sampler, body, candidate)
    })
}

fn rotate(current: f32, wanted: f32, limit: f32) -> f32 {
    let difference = (wanted - current + 180.0).rem_euclid(360.0) - 180.0;
    current + difference.clamp(-limit, limit)
}

/// Body yaw toward `direction`, at MoveControl's 90 deg/tick; a direction with
/// no horizontal part cannot steer a body.
fn yaw_toward(current: f32, direction: Vec3) -> f32 {
    let horizontal = mth::length2(direction.x, direction.z);
    if horizontal <= EPSILON {
        return current;
    }
    rotate(
        current,
        -(mth::atan2(direction.x, direction.z) as f32) * mth::RAD_TO_DEG,
        90.0,
    )
}

/// Pitch toward `direction`, at 40 deg/tick. `look`'s pitch axis, shared with
/// [`move_look`] so both feed the same presentation.
fn pitch_toward(current: f32, direction: Vec3) -> f32 {
    let horizontal = mth::length2(direction.x, direction.z);
    if horizontal <= EPSILON && direction.y.abs() <= EPSILON {
        return current;
    }
    rotate(
        current,
        -(mth::atan2(direction.y, horizontal) as f32) * mth::RAD_TO_DEG,
        40.0,
    )
    .clamp(-90.0, 90.0)
}

/// Vanilla `LookControl`/`BodyRotationControl`: a head with nothing to look at
/// turns back toward the body at [`HEAD_TURN`] degrees per tick and never leaves
/// a [`MAX_HEAD_YAW`] cone around it.
fn follow_head(head: f32, yaw: f32) -> f32 {
    let turned = rotate(head, yaw, HEAD_TURN);
    let off = (turned - yaw + 180.0).rem_euclid(360.0) - 180.0;
    yaw + off.clamp(-MAX_HEAD_YAW, MAX_HEAD_YAW)
}

pub(crate) fn look(
    rotation: [f32; 3],
    position: Vec3,
    velocity: Vec3,
    target: Option<[f64; 3]>,
) -> [f32; 3] {
    let direction = target.map_or(velocity, |target| {
        Vec3::new(
            target[0] - position.x,
            target[1] - position.y,
            target[2] - position.z,
        )
    });
    let horizontal = mth::length2(direction.x, direction.z);
    if horizontal <= EPSILON && direction.y.abs() <= EPSILON {
        return rotation;
    }
    let yaw = yaw_toward(rotation[0], direction);
    [yaw, pitch_toward(rotation[1], direction), yaw]
}

/// The facing of a mob with nothing to look at. Approximation of vanilla's three
/// controls, not parity: the body yaw follows `wanted` — the direction this
/// tick's AI asked to travel, never a knockback — at 90 deg/tick and holds under
/// [`MIN_WANTED_STEP`]; `resolved` still sets the pitch, so a vertical hop or a
/// fall keeps updating it; the head follows the body at [`HEAD_TURN`] inside the
/// [`MAX_HEAD_YAW`] cone.
fn move_look(
    rotation: [f32; 3],
    wanted: Vec3,
    resolved: Vec3,
    head_follows_body: bool,
) -> [f32; 3] {
    let yaw = if mth::length2(wanted.x, wanted.z) * mc_physics::TICK_SECONDS < MIN_WANTED_STEP {
        rotation[0]
    } else {
        yaw_toward(rotation[0], wanted)
    };
    let head = if head_follows_body {
        yaw
    } else {
        follow_head(rotation[2], yaw)
    };
    [yaw, pitch_toward(rotation[1], resolved), head]
}

pub(crate) fn step<S: BlockSampler>(
    input: MotionInput,
    sampler: &S,
    loaded: impl Fn(i32, i32) -> bool,
) -> MotionResult {
    let mut body = input.body;
    let mut impulse = input.impulse;
    if !loaded(floor_i32(body.position.x), floor_i32(body.position.z)) {
        body.velocity = Vec3::ZERO;
        return MotionResult {
            body,
            impulse: [0.0; 2],
            // The body cannot move, so its head settles onto its body: an
            // unloaded column must not strand a lagging head out of the rest.
            rotation: [input.rotation[0], input.rotation[1], input.rotation[0]],
            air_supply: input.air_supply,
            drowning: false,
            wall_climbing: false,
        };
    }
    let water = matches!(
        input.movement,
        MovementClass::Swimmer | MovementClass::Amphibious
    ) && water_body(sampler, body.position, body.aabb);
    let (air_supply, drowning) = if input.aquatic_control == AquaticControl::Dolphin {
        let eye = Vec3::new(
            body.position.x,
            body.position.y + body.aabb.height * 0.9,
            body.position.z,
        );
        crate::dolphin::breathe(
            input.air_supply,
            sampler.material_at(floor_i32(eye.x), floor_i32(eye.y), floor_i32(eye.z))
                == BlockMaterial::Water,
        )
    } else {
        breathe(input.air_supply, water, input.movement)
    };
    if input.movement == MovementClass::Stationary {
        body.velocity = Vec3::ZERO;
        let rotation = if input.look_target.is_some() {
            look(input.rotation, body.position, Vec3::ZERO, input.look_target)
        } else {
            move_look(
                input.rotation,
                Vec3::ZERO,
                Vec3::ZERO,
                input.head_follows_body,
            )
        };
        return MotionResult {
            body,
            impulse: [0.0; 2],
            rotation,
            air_supply,
            drowning,
            wall_climbing: false,
        };
    }
    let mut config = PhysicsConfig::living_entity();
    // AI locomotion is a standing velocity, not decaying combat momentum.
    config.air_drag = 1.0;
    config.ground_friction = 1.0;
    config.vertical_air_drag = 0.98;
    if input.climbs_walls {
        config.jump_speed = 0.0;
    }
    body.velocity.x = input.drive[0] + impulse[0];
    body.velocity.z = input.drive[2] + impulse[1];
    // The direction this tick's AI wants to travel: vanilla `MoveControl` turns
    // toward the wanted position, which a combat impulse never moves. It is the
    // drive alone, so no movement-class branch can leak an impulse into it.
    let mut wanted = Vec3::new(input.drive[0], 0.0, input.drive[2]);
    if input.movement == MovementClass::Flyer
        || water
        || (input.aquatic_control == AquaticControl::Dolphin
            && sampler.material_at(
                floor_i32(body.position.x),
                floor_i32(body.position.y),
                floor_i32(body.position.z),
            ) == BlockMaterial::Water)
    {
        config.gravity = 0.0;
        config.jump_speed = 0.0;
        config.step_height = 0.0;
        config.water_buoyancy = 0.0;
        config.water_drag = 1.0;
        config.vertical_air_drag = 1.0;
        body.velocity.y = input.drive[1];
        if water && input.aquatic_control != AquaticControl::Dolphin {
            let request = body.velocity;
            let picked = swim_navigation(sampler, body, request);
            body.velocity = picked.map_or(Vec3::ZERO, |index| swim_candidate(index, request));
            if let Some(index) = picked {
                // The AI's own navigation picked this detour: the body faces the
                // way it swims, not the wall its raw drive points at.
                let intent = swim_candidate(
                    index,
                    Vec3::new(input.drive[0], input.drive[1], input.drive[2]),
                );
                wanted = Vec3::new(intent.x, 0.0, intent.z);
            }
        }
    } else if input.movement == MovementClass::Swimmer {
        config.jump_speed = 0.0;
        config.step_height = 0.0;
        match input.aquatic_control {
            AquaticControl::Fish if body.on_ground => {
                body.velocity.x = (unit(input.id, input.tick) * 2.0 - 1.0) * 0.05 * 20.0;
                body.velocity.z =
                    (unit(input.id, input.tick.wrapping_add(1)) * 2.0 - 1.0) * 0.05 * 20.0;
                body.velocity.y = 0.400_000_005_960_464_5 * 20.0;
            }
            _ => {
                body.velocity.x = impulse[0];
                body.velocity.z = impulse[1];
            }
        }
    }
    if input.levitation > 0.0 {
        config.gravity = 0.0;
        body.velocity.y += (f64::from(input.levitation) - body.velocity.y) * 0.2;
    } else if input.slow_falling
        && body.velocity.y <= 0.0
        && input.movement != MovementClass::Flyer
        && !water
    {
        config.gravity = 4.0;
    }
    if input.no_gravity {
        config.gravity = 0.0;
    }
    let requested = body.velocity;
    let mut resolved = mc_physics::step_entity(body, sampler, config);
    // Spiders climb only against a real horizontal obstacle; the same swept
    // collision resolver clips climb at ceilings and missing chunk columns.
    let wall_climbing = input.climbs_walls
        && resolved.horizontal_collision
        && !water
        && loaded(
            floor_i32(
                body.position.x
                    + requested.x * mc_physics::TICK_SECONDS
                    + requested.x.signum() * body.aabb.half_width,
            ),
            floor_i32(
                body.position.z
                    + requested.z * mc_physics::TICK_SECONDS
                    + requested.z.signum() * body.aabb.half_width,
            ),
        );
    if wall_climbing {
        let climb = (body.position.y + 0.2 - resolved.body.position.y).max(0.0);
        if let Some(rise) = mc_physics::resolve_collision_displacement(
            sampler,
            resolved.body.position,
            body.aabb,
            Vec3::new(0.0, climb, 0.0),
            false,
            0.0,
        ) {
            resolved.body.position.y += rise.delta.y;
            resolved.body.velocity.y =
                (resolved.body.position.y - body.position.y) / mc_physics::TICK_SECONDS;
            resolved.body.on_ground = false;
        }
    }
    body = resolved.body;
    for (axis, asked, actual) in [
        (0, requested.x, body.velocity.x),
        (1, requested.z, body.velocity.z),
    ] {
        if actual.abs() < EPSILON && asked.abs() > EPSILON {
            impulse[axis] = 0.0;
        } else {
            let drag = if water {
                0.9
            } else if body.on_ground {
                0.91 * 0.6
            } else {
                0.91
            };
            let next = impulse[axis] * drag;
            let next = if next.abs() < 0.06 { 0.0 } else { next };
            if axis == 0 {
                body.velocity.x += next - impulse[axis];
            } else {
                body.velocity.z += next - impulse[axis];
            }
            impulse[axis] = next;
        }
    }
    // The battle policy is frozen for this freeze (the candidate A/B measured
    // more rotation and head traffic on the A1 scene): battle rows keep facing
    // the resolved velocity, the hold above unchanged.
    let rotation = if input.battle_facing {
        if input.look_target.is_none() && battle_holds_facing(&input.body, &body) {
            input.rotation
        } else {
            look(
                input.rotation,
                body.position,
                body.velocity,
                input.look_target,
            )
        }
    } else if input.look_target.is_some() {
        look(input.rotation, body.position, Vec3::ZERO, input.look_target)
    } else {
        move_look(
            input.rotation,
            wanted,
            body.velocity,
            input.head_follows_body,
        )
    };
    MotionResult {
        body,
        impulse,
        rotation,
        air_supply,
        drowning,
        wall_climbing,
    }
}

/// Battle-only facing rule. A battle body without a look target keeps its
/// rotation (yaw, pitch and head) when this tick's resolved step `after -
/// before` is no longer than [`BATTLE_HOLD_STEP`] or runs against the velocity
/// the body left the previous tick with. Collisions and slot dressing step such
/// bodies back and forth around their place, and facing every step swung them
/// up to 90 degrees per tick (receipts/BattleJitterClient). A steady walk turns
/// at any speed above the floor: each step agrees with the previous velocity. A
/// deliberate turn-around holds for one tick, then turns.
///
/// Cavalry residual: horse turns at gallop speed are steering, not
/// reversals, and this rule leaves them.
fn battle_holds_facing(before: &EntityBody, after: &EntityBody) -> bool {
    let dx = after.position.x - before.position.x;
    let dz = after.position.z - before.position.z;
    dx * dx + dz * dz <= BATTLE_HOLD_STEP * BATTLE_HOLD_STEP
        || dx * before.velocity.x + dz * before.velocity.z < 0.0
}
