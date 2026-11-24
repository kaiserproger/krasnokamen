use super::*;
use std::cell::{Cell, RefCell};

fn unit_body() -> WorldAabb {
    WorldAabb {
        min_x: 0.0,
        min_y: 0.0,
        min_z: 0.0,
        max_x: 1.0,
        max_y: 1.0,
        max_z: 1.0,
    }
}

fn wall_x(min_x: f64) -> WorldAabb {
    WorldAabb {
        min_x,
        min_y: -1.0,
        min_z: -10.0,
        max_x: min_x + 1.0,
        max_y: 2.0,
        max_z: 10.0,
    }
}

fn wall_z(min_z: f64) -> WorldAabb {
    WorldAabb {
        min_x: -10.0,
        min_y: -1.0,
        min_z,
        max_x: 10.0,
        max_y: 2.0,
        max_z: min_z + 1.0,
    }
}

#[test]
fn displacement_budget_is_euclidean_and_fails_closed() {
    assert!(displacement_within_limit(Vec3::new(6.0, 8.0, 0.0), 10.0));
    assert!(!displacement_within_limit(
        Vec3::new(6.0, 8.000_001, 0.0),
        10.0,
    ));
    assert!(!displacement_within_limit(
        Vec3::new(f64::NAN, 0.0, 0.0),
        10.0,
    ));
    assert!(!displacement_within_limit(Vec3::ZERO, f64::INFINITY));
    assert!(!displacement_within_limit(Vec3::ZERO, -1.0));
}

#[test]
fn authoritative_pose_limits_accept_edges_and_reject_invalid_values() {
    assert!(authoritative_pose_within_limits(
        Vec3::new(30_000_000.0, 20_000_000.0, -30_000_000.0),
        180.0,
        -90.0,
        30_000_000.0,
        20_000_000.0,
    ));
    assert!(!authoritative_pose_within_limits(
        Vec3::new(30_000_000.001, 0.0, 0.0),
        0.0,
        0.0,
        30_000_000.0,
        20_000_000.0,
    ));
    assert!(!authoritative_pose_within_limits(
        Vec3::ZERO,
        f32::NAN,
        0.0,
        30_000_000.0,
        20_000_000.0,
    ));
    assert!(!authoritative_pose_within_limits(
        Vec3::ZERO,
        0.0,
        f32::INFINITY,
        30_000_000.0,
        20_000_000.0,
    ));
    assert!(!authoritative_pose_within_limits(
        Vec3::ZERO,
        0.0,
        0.0,
        -1.0,
        20_000_000.0,
    ));
}

#[test]
fn sweep_sample_count_bounds_axis_progress_and_rejects_invalid_inputs() {
    assert_eq!(sweep_sample_count(Vec3::ZERO, 1.0 / 32.0), Some(1));
    assert_eq!(
        sweep_sample_count(Vec3::new(1.0, -0.25, 0.5), 1.0 / 32.0),
        Some(32)
    );
    assert_eq!(sweep_sample_count(Vec3::new(1.01, 0.0, 0.0), 0.5), Some(3));
    assert_eq!(sweep_sample_count(Vec3::new(f64::NAN, 0.0, 0.0), 0.5), None);
    assert_eq!(sweep_sample_count(Vec3::ZERO, 0.0), None);
    assert_eq!(sweep_sample_count(Vec3::ZERO, f64::INFINITY), None);
}

#[test]
fn horizontal_movement_residual_matches_vanilla_moved_wrongly_budget() {
    let requested = Vec3::new(0.24, 0.8, -0.24);
    assert!(horizontal_movement_residual_within_limit(
        requested,
        Vec3::new(0.0, -2.0, -0.24),
        0.0625,
    ));
    assert!(!horizontal_movement_residual_within_limit(
        requested,
        Vec3::new(-0.02, 0.8, -0.24),
        0.0625,
    ));
    assert!(!horizontal_movement_residual_within_limit(
        requested,
        Vec3::new(f64::NAN, 0.0, 0.0),
        0.0625,
    ));
    assert!(!horizontal_movement_residual_within_limit(
        requested,
        requested,
        f64::NAN,
    ));
}

#[test]
fn rotation_and_position_normalization_fail_closed() {
    assert!(rotation_is_finite(180.0, -90.0));
    assert!(!rotation_is_finite(f32::NAN, 0.0));
    assert_eq!(
        clamp_world_position(Vec3::new(12.0, -30.0, 40.0), 20.0, 25.0),
        Some(Vec3::new(12.0, -25.0, 20.0))
    );
    assert_eq!(
        clamp_world_position(Vec3::new(f64::NAN, 0.0, 0.0), 20.0, 25.0),
        None
    );
    assert_eq!(clamp_world_position(Vec3::ZERO, -1.0, 25.0), None);
}

#[test]
fn deflated_obstacle_intersection_is_strict_and_fail_closed() {
    let body = [0.0, 0.0, 0.0, 0.6, 1.8, 0.6];
    assert!(aabb_intersects_deflated_obstacle(
        body,
        [0.59, 0.0, 0.0, 1.59, 1.0, 1.0],
        1.0e-5,
    ));
    assert!(!aabb_intersects_deflated_obstacle(
        body,
        [0.6, 0.0, 0.0, 1.6, 1.0, 1.0],
        0.0,
    ));
    assert!(!aabb_intersects_deflated_obstacle(
        body,
        [f64::NAN, 0.0, 0.0, 1.0, 1.0, 1.0],
        0.0,
    ));
    assert!(!aabb_intersects_deflated_obstacle(
        body,
        [1.0, 0.0, 0.0, 0.0, 1.0, 1.0],
        0.0,
    ));
    assert!(!aabb_intersects_deflated_obstacle(
        body,
        [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        -1.0,
    ));
}

#[test]
fn fall_start_height_tracks_ground_water_and_airborne_history() {
    assert_eq!(
        next_fall_start_y(12.0, 15.0, false, false, 10.0, true, false),
        Some(10.0)
    );
    assert_eq!(
        next_fall_start_y(12.0, 15.0, false, false, 10.0, false, true),
        Some(10.0)
    );
    assert_eq!(
        next_fall_start_y(12.0, 15.0, true, false, 10.0, false, false),
        Some(12.0)
    );
    assert_eq!(
        next_fall_start_y(12.0, 15.0, false, true, 13.0, false, false),
        Some(13.0)
    );
    assert_eq!(
        next_fall_start_y(12.0, 15.0, false, false, 10.0, false, false),
        Some(15.0)
    );
    assert_eq!(
        next_fall_start_y(f64::NAN, 15.0, false, false, 10.0, false, false),
        None
    );
}

#[test]
fn horizontal_sweep_preserves_partial_x_travel_and_reports_collision() {
    let result = resolve_movement(
        unit_body(),
        Vec3::new(1.0, 0.0, 0.0),
        &[wall_x(1.4)],
        false,
        0.0,
    );

    assert!((result.delta.x - 0.4).abs() < 1.0e-12);
    assert_eq!(result.delta.z, 0.0);
    assert!(result.collided);
}

#[test]
fn horizontal_sweep_preserves_partial_z_travel_symmetrically() {
    let result = resolve_movement(
        unit_body(),
        Vec3::new(0.0, 0.0, 1.0),
        &[wall_z(1.4)],
        false,
        0.0,
    );

    assert!((result.delta.z - 0.4).abs() < 1.0e-12);
    assert_eq!(result.delta.x, 0.0);
    assert!(result.collided);
}

#[test]
fn horizontal_sweep_slides_along_wall_after_partial_contact() {
    let result = resolve_movement(
        unit_body(),
        Vec3::new(1.0, 0.0, 1.0),
        &[wall_x(1.4)],
        false,
        0.0,
    );

    assert!((result.delta.x - 0.4).abs() < 1.0e-12);
    assert!((result.delta.z - 1.0).abs() < 1.0e-12);
    assert!(result.collided);
}

#[test]
fn horizontal_sweep_stops_at_diagonal_corner_without_discarding_travel() {
    let result = resolve_movement(
        unit_body(),
        Vec3::new(1.0, 0.0, 1.0),
        &[wall_x(1.4), wall_z(1.4)],
        false,
        0.0,
    );

    assert!((result.delta.x - 0.4).abs() < 1.0e-12);
    assert!((result.delta.z - 0.4).abs() < 1.0e-12);
    assert!(result.collided);
}

#[test]
fn vanilla_axis_order_allows_face_flush_sprint_jump_around_corner() {
    let result = resolve_collision_displacement(
        &IsolatedCornerWorld { x: 1, z: 1 },
        Vec3::new(0.7, 64.0, 0.94),
        Aabb {
            half_width: 0.3,
            height: 1.8,
        },
        Vec3::new(0.26, 0.42, -0.36),
        true,
        STEP_HEIGHT,
    )
    .expect("bounded replay resolves");

    assert_eq!(result.delta, Vec3::new(0.26, 0.42, -0.36));
    assert!(!result.collided);
}

#[test]
fn vanilla_axis_order_keeps_straight_tunnelling_blocked() {
    let result = resolve_collision_displacement(
        &SweptWallWorld,
        Vec3::new(0.5, 64.0, 0.5),
        Aabb {
            half_width: 0.3,
            height: 1.8,
        },
        Vec3::new(3.0, 0.0, 0.0),
        true,
        STEP_HEIGHT,
    )
    .expect("bounded tunnelling replay resolves");

    assert!((result.delta.x - 1.2).abs() < 1.0e-12);
    assert!(result.collided);
}

#[test]
fn vanilla_axis_order_steps_onto_half_block_terrace() {
    let result = resolve_collision_displacement(
        &BottomSlabEdgeWorld,
        Vec3::new(0.5, 63.0, 0.5),
        Aabb {
            half_width: 0.3,
            height: 1.8,
        },
        Vec3::new(0.5, 0.0, 0.0),
        true,
        STEP_HEIGHT,
    )
    .expect("bounded terrace replay resolves");

    assert_eq!(result.delta.x, 0.5);
    assert!((result.delta.y - 0.5).abs() < 1.0e-12);
    assert_eq!(result.delta.z, 0.0);
    assert!(result.stepped);
}

#[test]
fn captured_worker_steps_from_path_under_overhead_stone() {
    let position = Vec3::new(149.7, 71.9375, 4.500000000000001);
    let body = Aabb {
        half_width: 0.3,
        height: 1.95,
    };
    let result = resolve_collision_displacement(
        &CapturedPathStepWorld { low_ceiling: false },
        position,
        body,
        Vec3::new(0.25, -0.08, 0.0),
        true,
        STEP_HEIGHT,
    )
    .expect("captured worker movement resolves");

    assert!((result.delta.x - 0.25).abs() < 1.0e-12);
    assert!((result.delta.y - 0.0625).abs() < 1.0e-12);
    assert_eq!(result.delta.z, 0.0);
    assert!(result.grounded);
    assert!(result.stepped);
    // The final feet clear both the path and next floor, and the head
    // remains below the overhead block while overlapping its x/z footprint.
    let feet = position.y + result.delta.y;
    assert!(feet >= 72.0);
    assert!(feet + body.height <= 74.0);
}

#[test]
fn full_block_rise_remains_blocked_by_step_height() {
    let result = resolve_collision_displacement(
        &OneBlockStepWorld,
        Vec3::new(0.7, 64.0, 0.5),
        Aabb {
            half_width: 0.3,
            height: 1.95,
        },
        Vec3::new(0.25, -0.08, 0.0),
        true,
        STEP_HEIGHT,
    )
    .expect("full-block movement resolves");

    assert!(result.delta.x.abs() < 1.0e-12);
    assert!(result.delta.y.abs() < 1.0e-12);
    assert!(result.grounded);
    assert!(!result.stepped);
    assert!(result.collided);
}

#[test]
fn captured_path_step_is_blocked_by_genuinely_low_ceiling() {
    let result = resolve_collision_displacement(
        &CapturedPathStepWorld { low_ceiling: true },
        Vec3::new(149.7, 71.9375, 4.500000000000001),
        Aabb {
            half_width: 0.3,
            height: 1.95,
        },
        Vec3::new(0.25, -0.08, 0.0),
        true,
        STEP_HEIGHT,
    )
    .expect("low-ceiling movement resolves");

    assert!(result.delta.x.abs() < 1.0e-12);
    assert!(result.delta.y.abs() < 1.0e-12);
    assert!(result.grounded);
    assert!(!result.stepped);
    assert!(result.collided);
}

#[test]
fn horizontal_sweep_partial_travel_is_symmetric_for_negative_direction() {
    let obstacle = WorldAabb {
        min_x: -1.4,
        min_y: -1.0,
        min_z: -10.0,
        max_x: -0.4,
        max_y: 2.0,
        max_z: 10.0,
    };
    let result = resolve_movement(
        unit_body(),
        Vec3::new(-1.0, 0.0, 0.0),
        &[obstacle],
        false,
        0.0,
    );

    assert!((result.delta.x + 0.4).abs() < 1.0e-12);
    assert!(result.collided);
}

#[test]
fn horizontal_sweep_exact_endpoint_contact_keeps_full_delta() {
    let result = resolve_movement(
        unit_body(),
        Vec3::new(1.0, 0.0, 0.0),
        &[wall_x(2.0)],
        false,
        0.0,
    );

    assert_eq!(result.delta, Vec3::new(1.0, 0.0, 0.0));
    assert!(!result.collided);
}

#[test]
fn public_collision_constructors_reject_invalid_geometry_without_panicking() {
    assert_eq!(BlockCollisionHeight::from_sixteenths(0), None);
    assert_eq!(BlockCollisionHeight::from_sixteenths(17), None);
    assert!(BlockCollisionHeight::from_sixteenths(16).is_some());
    assert_eq!(BlockCollisionBox::from_sixteenths(1, 0, 0, 1, 1, 1), None);
    assert_eq!(BlockCollisionBox::from_fixed_4096([0, 0, 0, 0, 1, 1]), None);
    assert!(BlockCollisionBox::from_sixteenths(0, 0, 0, 16, 24, 16).is_some());
}

#[test]
fn invalid_physics_coefficients_fail_closed() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(1.0, 0.0, 0.0),
        aabb: Aabb::COW,
        on_ground: true,
    };
    let invalid = [
        PhysicsConfig {
            gravity: -1.0,
            ..PhysicsConfig::default()
        },
        PhysicsConfig {
            terminal_velocity: 1.0,
            ..PhysicsConfig::default()
        },
        PhysicsConfig {
            ground_friction: -0.1,
            ..PhysicsConfig::default()
        },
        PhysicsConfig {
            air_drag: 1.1,
            ..PhysicsConfig::default()
        },
        PhysicsConfig {
            vertical_air_drag: -0.1,
            ..PhysicsConfig::default()
        },
        PhysicsConfig {
            water_drag: 1.1,
            ..PhysicsConfig::default()
        },
        PhysicsConfig {
            water_buoyancy: -1.0,
            ..PhysicsConfig::default()
        },
        PhysicsConfig {
            step_height: -0.1,
            ..PhysicsConfig::default()
        },
        PhysicsConfig {
            jump_speed: -0.1,
            ..PhysicsConfig::default()
        },
    ];

    for config in invalid {
        let result = step_entity(
            body,
            &FlatWorld {
                ground_y: 63,
                water_y: None,
            },
            config,
        );
        assert_eq!(result.body.position, body.position);
        assert_eq!(result.body.velocity, Vec3::ZERO);
    }
}

#[test]
fn invalid_sampler_collision_height_fails_closed_without_panicking() {
    struct InvalidSampler;
    impl BlockSampler for InvalidSampler {
        fn material_at(&self, _x: i32, _y: i32, _z: i32) -> BlockMaterial {
            BlockMaterial::Air
        }

        fn max_collision_box_y(&self) -> u8 {
            0
        }
    }

    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(1.0, 0.0, 0.0),
        aabb: Aabb::COW,
        on_ground: false,
    };
    let result = step_entity(body, &InvalidSampler, PhysicsConfig::default());
    assert_eq!(result.body.position, body.position);
    assert_eq!(result.body.velocity, Vec3::ZERO);
    assert!(result.horizontal_collision);
}

#[test]
fn cow_fallback_aabb_matches_vanilla_adult_dimensions() {
    assert_eq!(
        Aabb::COW,
        Aabb {
            half_width: 0.45,
            height: 1.4,
        }
    );
}

struct FlatWorld {
    ground_y: i32,
    water_y: Option<i32>,
}

impl BlockSampler for FlatWorld {
    fn material_at(&self, _x: i32, y: i32, _z: i32) -> BlockMaterial {
        if self.water_y == Some(y) {
            BlockMaterial::Water
        } else if y <= self.ground_y {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    }
}

struct OneBlockStepWorld;

impl BlockSampler for OneBlockStepWorld {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        if y <= 63 || (x == 1 && y == 64 && z == 0) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    }
}

struct FarmlandToFullBlockStepWorld;

impl BlockSampler for FarmlandToFullBlockStepWorld {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        if y == 63 && z == 0 && matches!(x, 0 | 1) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    }

    fn collision_height_at(&self, x: i32, y: i32, z: i32) -> Option<BlockCollisionHeight> {
        self.material_at(x, y, z).is_solid().then_some(if x == 0 {
            BlockCollisionHeight::from_sixteenths(15).expect("valid test collision height")
        } else {
            BlockCollisionHeight::FULL_BLOCK
        })
    }
}

// Five captured collision boxes, plus an optional lower ceiling subshape.
struct CapturedPathStepWorld {
    low_ceiling: bool,
}

impl BlockSampler for CapturedPathStepWorld {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        if z == 4
            && (matches!((x, y), (149 | 150, 70) | (149 | 150, 71) | (150, 74))
                || (self.low_ceiling && (x, y) == (150, 73)))
        {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    }

    fn collision_boxes_at(&self, x: i32, y: i32, z: i32, emit: &mut dyn FnMut(BlockCollisionBox)) {
        if !self.material_at(x, y, z).is_solid() {
            return;
        }
        if (x, y, z) == (149, 71, 4) {
            emit(
                BlockCollisionBox::from_sixteenths(0, 0, 0, 16, 15, 16)
                    .expect("valid path collision box"),
            );
        } else if (x, y, z) == (150, 73, 4) {
            emit(
                BlockCollisionBox::from_sixteenths(0, 15, 0, 16, 16, 16)
                    .expect("valid low ceiling collision box"),
            );
        } else {
            emit(BlockCollisionBox::FULL_BLOCK);
        }
    }
}

struct BottomSlabEdgeWorld;

impl BlockSampler for BottomSlabEdgeWorld {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        if y <= 62 || (x == 1 && y == 63 && z == 0) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    }

    fn collision_boxes_at(&self, x: i32, y: i32, z: i32, emit: &mut dyn FnMut(BlockCollisionBox)) {
        if y <= 62 {
            emit(BlockCollisionBox::FULL_BLOCK);
        } else if x == 1 && y == 63 && z == 0 {
            emit(
                BlockCollisionBox::from_sixteenths(0, 0, 0, 16, 8, 16)
                    .expect("valid test collision box"),
            );
        }
    }
}

struct IsolatedFenceWorld;

impl BlockSampler for IsolatedFenceWorld {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        if y <= 62 || (x == 1 && y == 63 && z == 0) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    }

    fn collision_boxes_at(&self, x: i32, y: i32, z: i32, emit: &mut dyn FnMut(BlockCollisionBox)) {
        if y <= 62 {
            emit(BlockCollisionBox::FULL_BLOCK);
        } else if x == 1 && y == 63 && z == 0 {
            emit(
                BlockCollisionBox::from_sixteenths(6, 0, 6, 10, 24, 10)
                    .expect("valid test collision box"),
            );
        }
    }

    fn max_collision_box_y(&self) -> u8 {
        24
    }
}

struct StraightBottomStairWorld;

impl BlockSampler for StraightBottomStairWorld {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        if y <= 62 || (x == 1 && y == 63 && z == 0) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    }

    fn collision_boxes_at(&self, x: i32, y: i32, z: i32, emit: &mut dyn FnMut(BlockCollisionBox)) {
        if y <= 62 {
            emit(BlockCollisionBox::FULL_BLOCK);
        } else if x == 1 && y == 63 && z == 0 {
            emit(
                BlockCollisionBox::from_sixteenths(0, 0, 0, 16, 8, 16)
                    .expect("valid test collision box"),
            );
            emit(
                BlockCollisionBox::from_sixteenths(0, 8, 0, 16, 16, 8)
                    .expect("valid test collision box"),
            );
        }
    }
}

struct SlabUnderLowCeilingWorld;

impl BlockSampler for SlabUnderLowCeilingWorld {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        if y <= 62 || (x == 1 && y == 63 && z == 0) || (x == 1 && y == 64 && z == 0) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    }

    fn collision_boxes_at(&self, x: i32, y: i32, z: i32, emit: &mut dyn FnMut(BlockCollisionBox)) {
        if y <= 62 {
            emit(BlockCollisionBox::FULL_BLOCK);
        } else if x == 1 && y == 63 && z == 0 {
            emit(
                BlockCollisionBox::from_sixteenths(0, 0, 0, 16, 8, 16)
                    .expect("valid test collision box"),
            );
        } else if x == 1 && y == 64 && z == 0 {
            emit(
                BlockCollisionBox::from_sixteenths(0, 12, 0, 16, 16, 16)
                    .expect("valid test collision box"),
            );
        }
    }
}

struct SweptWallWorld;

impl BlockSampler for SweptWallWorld {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        if y <= 63 || (x == 2 && matches!(y, 64 | 65) && z == 0) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    }
}

struct IsolatedCornerWorld {
    x: i32,
    z: i32,
}

impl BlockSampler for IsolatedCornerWorld {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        if x == self.x && y == 64 && z == self.z {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    }
}

struct JumpCeilingWorld;

impl BlockSampler for JumpCeilingWorld {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        if z == 0 && ((x == 0 && y == 63) || (x == 1 && y == 64) || (x == 0 && y == 67)) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    }
}

struct ProjectileWallWorld;

impl BlockSampler for ProjectileWallWorld {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        if x == 2 && y == 64 && matches!(z, 0..=2) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    }
}

struct PanicSampler;

impl BlockSampler for PanicSampler {
    fn material_at(&self, _x: i32, _y: i32, _z: i32) -> BlockMaterial {
        panic!("invalid movement must be rejected before sampling")
    }
}

struct CountingAirWorld {
    material_samples: Cell<usize>,
}

impl BlockSampler for CountingAirWorld {
    fn material_at(&self, _x: i32, _y: i32, _z: i32) -> BlockMaterial {
        self.material_samples
            .set(self.material_samples.get().saturating_add(1));
        BlockMaterial::Air
    }
}

struct FluidRecordingSampler {
    calls: RefCell<Vec<(i32, i32, i32)>>,
    fluid_at: (i32, i32, i32),
}

impl FluidRecordingSampler {
    fn all_air() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            fluid_at: (i32::MAX, i32::MAX, i32::MAX),
        }
    }

    fn with_fluid_at(x: i32, y: i32, z: i32) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            fluid_at: (x, y, z),
        }
    }

    fn sampled(&self) -> Vec<(i32, i32, i32)> {
        self.calls.borrow().clone()
    }
}

impl BlockSampler for FluidRecordingSampler {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        self.calls.borrow_mut().push((x, y, z));
        if (x, y, z) == self.fluid_at {
            BlockMaterial::Water
        } else {
            BlockMaterial::Air
        }
    }
}

struct DenseCollisionWorld {
    emitted: Cell<usize>,
}

impl BlockSampler for DenseCollisionWorld {
    fn material_at(&self, _x: i32, _y: i32, _z: i32) -> BlockMaterial {
        BlockMaterial::Air
    }

    fn collision_boxes_at(
        &self,
        _x: i32,
        _y: i32,
        _z: i32,
        emit: &mut dyn FnMut(BlockCollisionBox),
    ) {
        for _ in 0..1_024 {
            self.emitted.set(self.emitted.get().saturating_add(1));
            emit(BlockCollisionBox::FULL_BLOCK);
        }
    }
}

struct RandomCollisionWorld(u64);

impl BlockSampler for RandomCollisionWorld {
    fn material_at(&self, _x: i32, _y: i32, _z: i32) -> BlockMaterial {
        BlockMaterial::Air
    }

    fn collision_boxes_at(&self, x: i32, y: i32, z: i32, emit: &mut dyn FnMut(BlockCollisionBox)) {
        let bits = u64::from(x as u32).wrapping_mul(0x9E37_79B9)
            ^ u64::from(y as u32).wrapping_mul(0x85EB_CA6B)
            ^ u64::from(z as u32).wrapping_mul(0xC2B2_AE35)
            ^ self.0;
        if bits
            .wrapping_mul(0xD6E8_FEB8_6659_FD93)
            .rotate_left(17)
            .is_multiple_of(5)
        {
            emit(BlockCollisionBox::FULL_BLOCK);
        }
    }
}

struct Lcg(u64);

impl Lcg {
    fn unit(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.0 >> 11) as f64 / (1_u64 << 53) as f64
    }

    fn between(&mut self, min: f64, max: f64) -> f64 {
        min + (max - min) * self.unit()
    }
}

fn unit_tick_config() -> PhysicsConfig {
    PhysicsConfig {
        tick_seconds: 1.0,
        gravity: 0.0,
        air_drag: 1.0,
        vertical_air_drag: 1.0,
        step_height: 0.0,
        ..PhysicsConfig::default()
    }
}

#[test]
fn gravity_lands_body_on_solid_ground() {
    let world = FlatWorld {
        ground_y: 63,
        water_y: None,
    };
    let mut body = EntityBody {
        position: Vec3::new(0.5, 66.0, 0.5),
        velocity: Vec3::ZERO,
        aabb: Aabb::COW,
        on_ground: false,
    };

    for _ in 0..40 {
        body = step_entity(body, &world, PhysicsConfig::default()).body;
    }

    assert_eq!(body.position.y, 64.0);
    assert_eq!(body.velocity.y, 0.0);
    assert!(body.on_ground);
}

#[test]
fn air_drag_damps_horizontal_motion() {
    let world = FlatWorld {
        ground_y: -64,
        water_y: None,
    };
    let body = EntityBody {
        position: Vec3::new(0.5, 70.0, 0.5),
        velocity: Vec3::new(1.0, 0.0, 0.0),
        aabb: Aabb::COW,
        on_ground: false,
    };

    let stepped = step_entity(body, &world, PhysicsConfig::default()).body;

    assert!(stepped.velocity.x < body.velocity.x);
    assert!(stepped.velocity.y < 0.0);
}

#[test]
fn horizontal_collision_stops_body_at_wall() {
    let world = LocalBlocks {
        solids: vec![
            GridPos::new(0, 63, 0),
            GridPos::new(1, 64, 0),
            GridPos::new(1, 65, 0),
        ],
        fluids: Vec::new(),
    };
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(20.0, 0.0, 0.0),
        aabb: Aabb::COW,
        on_ground: true,
    };

    let result = step_entity(body, &world, PhysicsConfig::default());
    let stepped = result.body;

    assert!(result.horizontal_collision);
    assert!((stepped.position.x - 0.55).abs() < 1.0e-9);
    assert_eq!(stepped.velocity.x, 0.0);
    assert_eq!(stepped.velocity.y, 0.0);
    assert!(stepped.on_ground);
}

#[test]
fn swept_motion_stops_before_a_wall_crossed_in_one_tick() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(80.0, 0.0, 0.0),
        aabb: Aabb::COW,
        on_ground: true,
    };

    let result = step_entity(body, &SweptWallWorld, PhysicsConfig::default());

    assert!(result.horizontal_collision);
    assert!((result.body.position.x - 1.55).abs() < 1.0e-9);
    assert_eq!(result.body.velocity.x, 0.0);
}

#[test]
fn diagonal_swept_motion_slides_along_a_wall() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(80.0, 0.0, 10.0),
        aabb: Aabb::COW,
        on_ground: true,
    };

    let result = step_entity(body, &SweptWallWorld, PhysicsConfig::default());

    assert!(result.horizontal_collision);
    assert!((result.body.position.x - 1.55).abs() < 1.0e-9);
    assert!(result.body.position.z > body.position.z);
    assert_eq!(result.body.velocity.x, 0.0);
    assert!(result.body.velocity.z > 0.0);
}

#[test]
fn diagonal_sweep_stops_at_an_isolated_positive_corner() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(2.0, 0.0, 2.0),
        aabb: Aabb {
            half_width: 0.25,
            height: 0.5,
        },
        on_ground: false,
    };

    let result = step_entity(
        body,
        &IsolatedCornerWorld { x: 1, z: 1 },
        unit_tick_config(),
    );

    assert!(result.horizontal_collision);
    assert!((result.body.position.x - 0.75).abs() < 1.0e-9);
    assert!((result.body.position.z - 0.75).abs() < 1.0e-9);
    assert_eq!(result.body.velocity.x, 0.0);
    assert_eq!(result.body.velocity.z, 0.0);
}

#[test]
fn diagonal_sweep_stops_at_an_isolated_negative_corner() {
    let body = EntityBody {
        position: Vec3::new(2.5, 64.0, 2.5),
        velocity: Vec3::new(-2.0, 0.0, -2.0),
        aabb: Aabb {
            half_width: 0.25,
            height: 0.5,
        },
        on_ground: false,
    };

    let result = step_entity(
        body,
        &IsolatedCornerWorld { x: 1, z: 1 },
        unit_tick_config(),
    );

    assert!(result.horizontal_collision);
    assert!((result.body.position.x - 2.25).abs() < 1.0e-9);
    assert!((result.body.position.z - 2.25).abs() < 1.0e-9);
    assert_eq!(result.body.velocity.x, 0.0);
    assert_eq!(result.body.velocity.z, 0.0);
}

#[test]
fn auto_jump_scans_the_full_vertical_displacement() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(1.0, 0.0, 0.0),
        aabb: Aabb {
            half_width: 0.25,
            height: 1.0,
        },
        on_ground: true,
    };
    let config = PhysicsConfig {
        jump_speed: 4.0,
        ..unit_tick_config()
    };

    let result = step_entity(body, &JumpCeilingWorld, config);

    assert!(result.horizontal_collision);
    assert_eq!(result.body.position.y, 66.0);
    assert_eq!(result.body.velocity.y, 0.0);
    assert!(result.body.position.y + result.body.aabb.height <= 67.0);
}

#[test]
fn stop_on_solid_stops_at_the_first_impact_point() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(2.0, 0.0, 1.0),
        aabb: Aabb {
            half_width: 0.25,
            height: 0.5,
        },
        on_ground: false,
    };
    let config = PhysicsConfig {
        stop_on_solid: true,
        ..unit_tick_config()
    };

    let result = step_entity(body, &ProjectileWallWorld, config);

    assert!(result.horizontal_collision);
    assert_eq!(result.body.position.x, 1.75);
    assert_eq!(result.body.position.z, 1.125);
    assert_eq!(result.body.velocity, Vec3::ZERO);
}

#[test]
fn living_entity_does_not_jump_when_the_raised_path_is_blocked() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(80.0, 0.0, 0.0),
        aabb: Aabb::COW,
        on_ground: true,
    };

    let result = step_entity(body, &SweptWallWorld, PhysicsConfig::living_entity());

    assert!(result.horizontal_collision);
    assert!((result.body.position.x - 1.55).abs() < 1.0e-9);
    assert_eq!(result.body.position.y, body.position.y);
    assert_eq!(result.body.position.z, body.position.z);
    assert_eq!(result.body.velocity, Vec3::ZERO);
    assert!(result.body.on_ground);
}

#[test]
fn non_finite_displacement_is_rejected_before_world_sampling() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(f64::NAN, 0.0, 0.0),
        aabb: Aabb::COW,
        on_ground: true,
    };

    let result = step_entity(body, &PanicSampler, PhysicsConfig::default());

    assert_eq!(result.body.position, body.position);
    assert_eq!(result.body.velocity, Vec3::ZERO);
}

#[test]
fn oversized_displacement_is_rejected_before_world_sampling() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(17.0, 0.0, 0.0),
        aabb: Aabb::COW,
        on_ground: true,
    };

    let result = step_entity(body, &PanicSampler, unit_tick_config());

    assert_eq!(result.body.position, body.position);
    assert_eq!(result.body.velocity, Vec3::ZERO);
    assert!(result.horizontal_collision);
}

#[test]
fn oversized_scan_volume_is_rejected_before_candidate_collection() {
    let world = CountingAirWorld {
        material_samples: Cell::new(0),
    };
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(16.0, 0.0, 0.0),
        aabb: Aabb {
            half_width: 10.0,
            height: 16.0,
        },
        on_ground: false,
    };

    let result = step_entity(body, &world, unit_tick_config());

    assert_eq!(result.body.position, body.position);
    assert_eq!(result.body.velocity, Vec3::ZERO);
    assert!(world.material_samples.get() <= 100);
}

#[test]
fn body_overlaps_fluid_samples_narrow_body_once_per_column_layer() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::ZERO,
        aabb: Aabb {
            half_width: 0.25,
            height: 1.4,
        },
        on_ground: false,
    };

    let miss = FluidRecordingSampler::all_air();
    assert!(!body_overlaps_fluid(body, &miss));
    assert_eq!(miss.sampled(), vec![(0, 64, 0), (0, 65, 0)]);

    let hit = FluidRecordingSampler::with_fluid_at(0, 64, 0);
    assert!(body_overlaps_fluid(body, &hit));
    assert_eq!(hit.sampled(), vec![(0, 64, 0)]);
}

#[test]
fn body_overlaps_fluid_samples_two_distinct_columns_in_order() {
    let body = EntityBody {
        position: Vec3::new(0.0, 64.0, 0.5),
        velocity: Vec3::ZERO,
        aabb: Aabb {
            half_width: 0.25,
            height: 1.4,
        },
        on_ground: false,
    };

    let miss = FluidRecordingSampler::all_air();
    assert!(!body_overlaps_fluid(body, &miss));
    assert_eq!(
        miss.sampled(),
        vec![(-1, 64, 0), (-1, 65, 0), (0, 64, 0), (0, 65, 0)]
    );

    let hit = FluidRecordingSampler::with_fluid_at(0, 64, 0);
    assert!(body_overlaps_fluid(body, &hit));
    assert_eq!(hit.sampled(), vec![(-1, 64, 0), (-1, 65, 0), (0, 64, 0)]);
}

#[test]
fn body_overlaps_fluid_samples_four_distinct_columns_without_duplicates() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::ZERO,
        aabb: Aabb {
            half_width: 0.55,
            height: 1.4,
        },
        on_ground: false,
    };

    let miss = FluidRecordingSampler::all_air();
    assert!(!body_overlaps_fluid(body, &miss));
    assert_eq!(
        miss.sampled(),
        vec![
            (-1, 64, -1),
            (-1, 65, -1),
            (-1, 64, 1),
            (-1, 65, 1),
            (1, 64, -1),
            (1, 65, -1),
            (1, 64, 1),
            (1, 65, 1),
        ]
    );

    let hit = FluidRecordingSampler::with_fluid_at(1, 64, -1);
    assert!(body_overlaps_fluid(body, &hit));
    assert_eq!(
        hit.sampled(),
        vec![
            (-1, 64, -1),
            (-1, 65, -1),
            (-1, 64, 1),
            (-1, 65, 1),
            (1, 64, -1),
        ]
    );
}

#[test]
fn collision_candidate_collection_stops_at_its_cap() {
    let world = DenseCollisionWorld {
        emitted: Cell::new(0),
    };
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(16.0, 0.0, 0.0),
        aabb: Aabb {
            half_width: 0.25,
            height: 0.5,
        },
        on_ground: false,
    };

    let result = step_entity(body, &world, unit_tick_config());

    assert_eq!(result.body.position, body.position);
    assert_eq!(result.body.velocity, Vec3::ZERO);
    assert!(world.emitted.get() <= 8_192 + 1_024);
}

#[test]
fn collision_axis_sweep_matches_volume_scan_on_random_worlds_aabbs_and_motions() {
    let mut rng = Lcg(0xDDA2_6074_3BAD_F00D);
    for case in 0..2_048 {
        let world = RandomCollisionWorld(rng.0);
        let body = WorldAabb {
            min_x: rng.between(-8.0, 8.0),
            min_y: rng.between(56.0, 72.0),
            min_z: rng.between(-8.0, 8.0),
            max_x: 0.0,
            max_y: 0.0,
            max_z: 0.0,
        };
        let body = WorldAabb {
            max_x: body.min_x + rng.between(0.05, 2.0),
            max_y: body.min_y + rng.between(0.05, 3.0),
            max_z: body.min_z + rng.between(0.05, 2.0),
            ..body
        };
        let desired = Vec3::new(
            rng.between(-4.0, 4.0),
            rng.between(-4.0, 4.0),
            rng.between(-4.0, 4.0),
        );
        let step_height = rng.between(0.0, 1.0);
        let mut swept = SmallVec::new();
        let mut volume = SmallVec::new();
        assert_eq!(
            collision_boxes_for_motion(&world, body, desired, step_height, &mut swept),
            collision_boxes_for_motion_volume_scan(&world, body, desired, step_height, &mut volume),
            "candidate admission differs for case {case}"
        );
        let sweep = resolve_movement(body, desired, &swept, case & 1 == 0, step_height);
        let legacy = resolve_movement(body, desired, &volume, case & 1 == 0, step_height);
        assert_eq!(
            (
                sweep.delta.x.to_bits(),
                sweep.delta.y.to_bits(),
                sweep.delta.z.to_bits(),
                sweep.grounded,
                sweep.stepped,
                sweep.collided,
            ),
            (
                legacy.delta.x.to_bits(),
                legacy.delta.y.to_bits(),
                legacy.delta.z.to_bits(),
                legacy.grounded,
                legacy.stepped,
                legacy.collided,
            ),
            "movement differs for case {case}: body={body:?}, desired={desired:?}"
        );
        let sweep = resolve_until_first_impact(body, desired, &swept);
        let legacy = resolve_until_first_impact(body, desired, &volume);
        assert_eq!(
            (
                sweep.delta.x.to_bits(),
                sweep.delta.y.to_bits(),
                sweep.delta.z.to_bits(),
                sweep.grounded,
                sweep.collided,
            ),
            (
                legacy.delta.x.to_bits(),
                legacy.delta.y.to_bits(),
                legacy.delta.z.to_bits(),
                legacy.grounded,
                legacy.collided,
            ),
            "first impact differs for case {case}: body={body:?}, desired={desired:?}"
        );
    }
}

#[test]
fn arrow_projectile_applies_gravity_and_vertical_drag() {
    let world = FlatWorld {
        ground_y: -64,
        water_y: None,
    };
    let body = EntityBody {
        position: Vec3::new(0.5, 70.0, 0.5),
        velocity: Vec3::new(0.0, 1.0, 0.0),
        aabb: Aabb {
            half_width: 0.25,
            height: 0.25,
        },
        on_ground: false,
    };

    let stepped = step_entity(body, &world, PhysicsConfig::arrow_projectile()).body;

    assert!(stepped.velocity.y < body.velocity.y);
    assert!(stepped.position.y > body.position.y);
}

#[test]
fn arrow_projectile_stops_on_solid_without_step_up() {
    let world = LocalBlocks {
        solids: vec![GridPos::new(1, 64, 0)],
        fluids: Vec::new(),
    };
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(20.0, 0.0, 0.0),
        aabb: Aabb {
            half_width: 0.25,
            height: 0.25,
        },
        on_ground: true,
    };

    let stepped = step_entity(body, &world, PhysicsConfig::arrow_projectile()).body;

    assert!((stepped.position.x - 0.75).abs() < 1.0e-9);
    let desired_x = body.velocity.x * ARROW_DRAG * TICK_SECONDS;
    let impact_time = (1.0 - (body.position.x + body.aabb.half_width)) / desired_x;
    let desired_y =
        -ARROW_GRAVITY_BLOCKS_PER_SECOND_SQUARED * TICK_SECONDS * ARROW_DRAG * TICK_SECONDS;
    let expected_y = body.position.y + desired_y * impact_time;
    assert!((stepped.position.y - expected_y).abs() < 1.0e-9);
    assert!((stepped.position.z - body.position.z).abs() < 1.0e-9);
    assert_eq!(stepped.velocity, Vec3::ZERO);
    assert!(stepped.on_ground);
}

#[test]
fn stopped_arrow_projectile_remains_stopped() {
    let world = FlatWorld {
        ground_y: 63,
        water_y: None,
    };
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::ZERO,
        aabb: Aabb {
            half_width: 0.25,
            height: 0.25,
        },
        on_ground: true,
    };

    let stepped = step_entity(body, &world, PhysicsConfig::arrow_projectile()).body;

    assert_eq!(stepped, body);
}

struct SupportShortcut<'a> {
    sampler: &'a dyn BlockSampler,
    collision_samples: Cell<usize>,
}

impl BlockSampler for SupportShortcut<'_> {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        self.sampler.material_at(x, y, z)
    }

    fn max_collision_box_y(&self) -> u8 {
        self.sampler.max_collision_box_y()
    }

    fn collision_boxes_at(&self, x: i32, y: i32, z: i32, emit: &mut dyn FnMut(BlockCollisionBox)) {
        self.collision_samples.set(self.collision_samples.get() + 1);
        self.sampler.collision_boxes_at(x, y, z, emit);
    }

    fn stationary_downward_clip(
        &self,
        body: EntityBody,
        displacement_y: f64,
        max_scan_cells: usize,
        max_candidates: usize,
    ) -> Option<f64> {
        stationary_support_clip(
            self,
            body,
            displacement_y,
            max_scan_cells,
            max_candidates,
            4,
        )
    }
}

#[test]
fn support_shortcut_matches_full_scan_for_support_edits_shapes_and_fluids() {
    let flat = FlatWorld {
        ground_y: 63,
        water_y: None,
    };
    let wet = FlatWorld {
        ground_y: 63,
        water_y: Some(64),
    };
    let absent = LocalBlocks {
        solids: Vec::new(),
        fluids: Vec::new(),
    };
    let cases: [(&dyn BlockSampler, Vec3); 7] = [
        (&flat, Vec3::new(-16.0, 64.0, -16.0)),
        (&BottomSlabEdgeWorld, Vec3::new(1.5, 63.5, 0.5)),
        (&BottomSlabEdgeWorld, Vec3::new(1.5, 64.0, 0.5)),
        (&IsolatedFenceWorld, Vec3::new(1.5, 64.5, 0.5)),
        (&FarmlandToFullBlockStepWorld, Vec3::new(0.5, 64.0, 0.5)),
        (&absent, Vec3::new(0.5, 64.0, 0.5)),
        (&wet, Vec3::new(0.5, 64.0, 0.5)),
    ];
    for (sampler, position) in cases {
        let body = EntityBody {
            position,
            velocity: Vec3::ZERO,
            aabb: Aabb::COW,
            on_ground: true,
        };
        let shortcut = SupportShortcut {
            sampler,
            collision_samples: Cell::new(0),
        };
        let mut boxes = SmallVec::new();
        let reference = WorldAabb::for_body(body);
        let dy = -0.08;
        if let Some(proven) = stationary_support_clip(&shortcut, body, dy, 8192, 8192, 4) {
            collision_boxes_for_motion(
                &shortcut,
                reference,
                Vec3::new(0.0, dy, 0.0),
                0.0,
                &mut boxes,
            )
            .unwrap();
            assert_eq!(proven.to_bits(), clip_y(reference, dy, &boxes).to_bits());
        }
        let actual = step_entity(body, &shortcut, PhysicsConfig::living_entity());
        // A transparent wrapper with no hook is the ordinary implementation.
        struct Reference<'a>(&'a dyn BlockSampler);
        impl BlockSampler for Reference<'_> {
            fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
                self.0.material_at(x, y, z)
            }
            fn max_collision_box_y(&self) -> u8 {
                self.0.max_collision_box_y()
            }
            fn collision_boxes_at(
                &self,
                x: i32,
                y: i32,
                z: i32,
                emit: &mut dyn FnMut(BlockCollisionBox),
            ) {
                self.0.collision_boxes_at(x, y, z, emit);
            }
        }
        assert_eq!(
            actual,
            step_entity(body, &Reference(sampler), PhysicsConfig::living_entity())
        );
    }
}

#[test]
fn support_shortcut_samples_one_cell_and_preserves_both_scan_budgets() {
    let flat = FlatWorld {
        ground_y: 63,
        water_y: None,
    };
    let sampler = SupportShortcut {
        sampler: &flat,
        collision_samples: Cell::new(0),
    };
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::ZERO,
        aabb: Aabb::COW,
        on_ground: true,
    };
    assert_eq!(
        stationary_support_clip(&sampler, body, -0.08, 8192, 8192, 1),
        Some(0.0)
    );
    assert_eq!(sampler.collision_samples.get(), 1);
    assert_eq!(
        stationary_support_clip(&sampler, body, -0.08, 0, 8192, 1),
        None
    );
    assert_eq!(
        stationary_support_clip(&sampler, body, -0.08, 8192, 0, 1),
        None
    );
    assert_eq!(
        stationary_support_clip(&sampler, body, -0.08, 8192, 8192, usize::MAX),
        None
    );
    assert_eq!(
        sampler.collision_samples.get(),
        1,
        "budget refusals never sample"
    );
}

#[test]
fn settled_living_body_follows_lowered_support_like_the_general_resolver() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::ZERO,
        aabb: Aabb::COW,
        on_ground: true,
    };
    let config = PhysicsConfig::living_entity();
    let general = step_entity(
        EntityBody {
            on_ground: false,
            ..body
        },
        &FarmlandToFullBlockStepWorld,
        config,
    );
    let settled = step_entity(body, &FarmlandToFullBlockStepWorld, config);
    assert_eq!(
        general.body.position.y.to_bits(),
        (63.0_f64 + 15.0 / 16.0).to_bits()
    );
    assert!(general.body.on_ground);
    assert_eq!(
        settled.body.position.y.to_bits(),
        general.body.position.y.to_bits()
    );
    assert_eq!(settled, general);
}

#[test]
fn settled_living_body_falls_when_support_is_removed() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::ZERO,
        aabb: Aabb::COW,
        on_ground: true,
    };
    let world = LocalBlocks {
        solids: Vec::new(),
        fluids: Vec::new(),
    };
    let config = PhysicsConfig::living_entity();
    let general = step_entity(
        EntityBody {
            on_ground: false,
            ..body
        },
        &world,
        config,
    );
    let settled = step_entity(body, &world, config);
    assert!(!settled.body.on_ground);
    assert!(settled.body.position.y < body.position.y);
    assert_eq!(settled, general);
}

#[test]
fn stopped_arrow_embedding_is_not_a_living_support_test() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::ZERO,
        aabb: Aabb {
            half_width: 0.25,
            height: 0.25,
        },
        on_ground: true,
    };
    let world = LocalBlocks {
        solids: Vec::new(),
        fluids: Vec::new(),
    };
    assert_eq!(
        step_entity(body, &world, PhysicsConfig::arrow_projectile()).body,
        body
    );
}

#[test]
fn clip_y_snaps_a_body_resting_within_the_contact_tolerance_of_a_face() {
    // Vanilla `VoxelShape.collide` covers every axis with the 1.0E-7 face
    // tolerance; `clip_y` used to skip a face it sat within that tolerance of,
    // so a body resting one ULP inside a floor kept its whole downward step.
    let floor = WorldAabb {
        min_x: -10.0,
        min_y: -1.0,
        min_z: -10.0,
        max_x: 10.0,
        max_y: 0.0,
        max_z: 10.0,
    };
    let body = WorldAabb {
        min_x: 0.0,
        min_y: -1.0e-9,
        min_z: 0.0,
        max_x: 0.6,
        max_y: 1.8,
        max_z: 0.6,
    };
    assert!(
        clip_y(body, -0.1, std::slice::from_ref(&floor)) >= 0.0,
        "a body resting inside the floor must be snapped out, not stepped through"
    );
    // Deeper than the tolerance is not a contact: the step is untouched.
    let deep = WorldAabb {
        min_y: -1.0e-3,
        max_y: 1.8 - 1.0e-3,
        ..body
    };
    assert_eq!(clip_y(deep, -0.1, std::slice::from_ref(&floor)), -0.1);
    // Clearly above the floor: the full step survives.
    let clear = WorldAabb {
        min_y: 0.5,
        max_y: 2.3,
        ..body
    };
    assert_eq!(clip_y(clear, -0.1, std::slice::from_ref(&floor)), -0.1);
    // Ceiling: a head one ULP inside blocks the rise, a clear one does not.
    let ceiling = WorldAabb {
        min_x: -10.0,
        min_y: 1.0,
        min_z: -10.0,
        max_x: 10.0,
        max_y: 2.0,
        max_z: 10.0,
    };
    let head_inside = WorldAabb {
        max_y: 1.0 + 1.0e-9,
        min_y: 1.0 + 1.0e-9 - 1.8,
        ..body
    };
    assert!(
        clip_y(head_inside, 0.1, std::slice::from_ref(&ceiling)) <= 0.0,
        "a head resting inside the ceiling must not rise"
    );
    let head_clear = WorldAabb {
        max_y: 0.5,
        min_y: 0.5 - 1.8,
        ..body
    };
    // 1.0 - 0.5 is exact in f64, so the clip equals the request.
    assert_eq!(clip_y(head_clear, 0.1, std::slice::from_ref(&ceiling)), 0.1);
}

#[test]
fn living_body_starts_full_block_climb_with_jump() {
    let world = LocalBlocks {
        solids: vec![GridPos::new(0, 63, 0), GridPos::new(1, 64, 0)],
        fluids: Vec::new(),
    };
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(20.0, 0.0, 0.0),
        aabb: Aabb::COW,
        on_ground: true,
    };

    let result = step_entity(body, &world, PhysicsConfig::living_entity());
    let stepped = result.body;

    assert!(result.horizontal_collision);
    assert!((stepped.position.x - 0.55).abs() < 1.0e-9);
    assert!(
        stepped.position.y > 64.0 && stepped.position.y < 65.0,
        "{stepped:?}"
    );
    assert!((stepped.velocity.y - LIVING_JUMP_SPEED_BLOCKS_PER_SECOND).abs() < 1.0e-9);
    assert!(!stepped.on_ground);
}

#[test]
fn living_body_steps_from_farmland_to_full_block_without_starting_jump() {
    let body = EntityBody {
        position: Vec3::new(0.5, 63.0 + 15.0 / 16.0, 0.5),
        velocity: Vec3::new(20.0, 0.0, 4.0),
        aabb: Aabb::COW,
        on_ground: true,
    };

    let result = step_entity(
        body,
        &FarmlandToFullBlockStepWorld,
        PhysicsConfig::living_entity(),
    );

    assert!(!result.horizontal_collision);
    assert_eq!(result.body.position.y, 64.0);
    assert_eq!(result.body.velocity.y, 0.0);
    assert!(result.body.on_ground);
    assert!(result.body.position.x > body.position.x);
    let expected_z = body.position.z + body.velocity.z * AIR_DRAG * TICK_SECONDS;
    let expected_velocity_z = body.velocity.z * AIR_DRAG * GROUND_FRICTION;
    assert!((result.body.position.z - expected_z).abs() < 1.0e-9);
    assert!((result.body.velocity.z - expected_velocity_z).abs() < 1.0e-9);
}

#[test]
fn living_body_walks_over_vanilla_bottom_slab_height_difference() {
    let body = EntityBody {
        position: Vec3::new(0.5, 63.0, 0.5),
        velocity: Vec3::new(20.0, 0.0, 0.0),
        aabb: Aabb::COW,
        on_ground: true,
    };

    let result = step_entity(body, &BottomSlabEdgeWorld, PhysicsConfig::living_entity());

    assert!(!result.horizontal_collision);
    assert_eq!(result.body.position.y, 63.5);
    assert!(result.body.position.x > body.position.x);
    assert_eq!(result.body.velocity.y, 0.0);
    assert!(result.body.on_ground);
}

#[test]
fn diagonal_step_path_over_slab_lands_on_support() {
    let body = EntityBody {
        position: Vec3::new(0.5, 63.0, 0.5),
        velocity: Vec3::new(20.0, 0.0, 20.0),
        aabb: Aabb {
            half_width: 0.2,
            height: 0.7,
        },
        on_ground: true,
    };

    let result = step_entity(body, &BottomSlabEdgeWorld, PhysicsConfig::living_entity());

    assert!(!result.horizontal_collision);
    assert!((result.body.position.x - 1.48).abs() < 1.0e-9);
    assert!((result.body.position.z - 1.48).abs() < 1.0e-9);
    assert!((result.body.position.y - 63.0).abs() < 1.0e-9);
    assert!(result.body.on_ground);
}

#[test]
fn small_body_can_move_beside_an_isolated_fence_post() {
    let body = EntityBody {
        position: Vec3::new(0.5, 63.0, 0.1),
        velocity: Vec3::new(20.0, 0.0, 0.0),
        aabb: Aabb {
            half_width: 0.2,
            height: 0.7,
        },
        on_ground: true,
    };

    let result = step_entity(body, &IsolatedFenceWorld, PhysicsConfig::living_entity());

    assert!(!result.horizontal_collision);
    assert!(result.body.position.x > body.position.x);
    assert_eq!(result.body.position.y, 63.0);
}

#[test]
fn isolated_fence_post_blocks_its_occupied_center() {
    let body = EntityBody {
        position: Vec3::new(0.5, 63.0, 0.5),
        velocity: Vec3::new(20.0, 0.0, 0.0),
        aabb: Aabb {
            half_width: 0.2,
            height: 0.7,
        },
        on_ground: true,
    };

    let result = step_entity(body, &IsolatedFenceWorld, PhysicsConfig::living_entity());

    assert!(result.horizontal_collision);
    assert!((result.body.position.x - 1.175).abs() < 1.0e-9);
    assert_eq!(result.body.velocity.x, 0.0);
}

#[test]
fn overheight_fence_rooted_below_body_still_blocks_motion() {
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(20.0, 0.0, 0.0),
        aabb: Aabb {
            half_width: 0.2,
            height: 0.7,
        },
        on_ground: false,
    };

    let result = step_entity(body, &IsolatedFenceWorld, PhysicsConfig::living_entity());

    assert!(result.horizontal_collision);
    assert!((result.body.position.x - 1.175).abs() < 1.0e-9);
    assert_eq!(result.body.velocity.x, 0.0);
}

#[test]
fn swept_motion_cannot_tunnel_through_an_overheight_fence() {
    let body = EntityBody {
        position: Vec3::new(0.5, 63.0, 0.5),
        velocity: Vec3::new(60.0, 0.0, 0.0),
        aabb: Aabb {
            half_width: 0.2,
            height: 0.7,
        },
        on_ground: true,
    };

    let result = step_entity(body, &IsolatedFenceWorld, PhysicsConfig::living_entity());

    assert!(result.horizontal_collision);
    assert!((result.body.position.x - 1.175).abs() < 1.0e-9);
    assert_eq!(result.body.velocity.x, 0.0);
}

#[test]
fn step_path_is_rejected_when_a_low_ceiling_blocks_the_rise() {
    let body = EntityBody {
        position: Vec3::new(0.5, 63.0, 0.5),
        velocity: Vec3::new(60.0, 0.0, 0.0),
        aabb: Aabb::COW,
        on_ground: true,
    };

    let result = step_entity(body, &SlabUnderLowCeilingWorld, PhysicsConfig::default());

    assert!(result.horizontal_collision);
    assert!((result.body.position.x - 0.55).abs() < 1.0e-9);
    assert!((result.body.position.y - 63.0).abs() < 1.0e-9);
}

#[test]
fn body_steps_on_lower_tread_of_composed_stair_shape() {
    let body = EntityBody {
        position: Vec3::new(0.5, 63.0, 0.75),
        velocity: Vec3::new(20.0, 0.0, 0.0),
        aabb: Aabb {
            half_width: 0.2,
            height: 0.7,
        },
        on_ground: true,
    };

    let result = step_entity(
        body,
        &StraightBottomStairWorld,
        PhysicsConfig::living_entity(),
    );

    assert!(!result.horizontal_collision);
    assert!(result.body.position.x > body.position.x);
    assert_eq!(result.body.position.y, 63.5);
    assert!(result.body.on_ground);
}

#[test]
fn living_jump_clears_blocked_axis_and_keeps_tangent_motion() {
    let world = LocalBlocks {
        solids: vec![GridPos::new(0, 63, 0), GridPos::new(1, 64, 0)],
        fluids: Vec::new(),
    };
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(20.0, 0.0, 4.0),
        aabb: Aabb::COW,
        on_ground: true,
    };

    let result = step_entity(body, &world, PhysicsConfig::living_entity());

    assert!(result.horizontal_collision);
    assert!((result.body.position.x - 0.55).abs() < 1.0e-9);
    assert!(result.body.position.z > body.position.z);
    assert_eq!(result.body.velocity.x, 0.0);
    assert!(result.body.velocity.z > 0.0);
    assert!(result.body.velocity.y > 0.0);
}

#[test]
fn passive_livestock_finish_climbing_one_block() {
    let livestock = [
        (
            "cow",
            Aabb {
                half_width: 0.45,
                height: 1.4,
            },
        ),
        (
            "sheep",
            Aabb {
                half_width: 0.45,
                height: 0.9,
            },
        ),
        (
            "chicken",
            Aabb {
                half_width: 0.2,
                height: 0.7,
            },
        ),
    ];

    for (name, aabb) in livestock {
        let mut body = EntityBody {
            position: Vec3::new(0.5, 64.0, 0.5),
            velocity: Vec3::ZERO,
            aabb,
            on_ground: true,
        };
        let mut reached_step = false;

        for _ in 0..40 {
            body.velocity.x = 2.0;
            body = step_entity(body, &OneBlockStepWorld, PhysicsConfig::living_entity()).body;
            if body.position.x > 1.0 && body.position.y >= 65.0 {
                reached_step = true;
                break;
            }
        }

        assert!(reached_step, "{name} must jump onto a full block");
    }
}

#[test]
fn living_jump_stops_below_solid_ceiling() {
    let world = LocalBlocks {
        solids: vec![GridPos::new(0, 63, 0), GridPos::new(0, 66, 0)],
        fluids: Vec::new(),
    };
    let mut body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(0.0, LIVING_JUMP_SPEED_BLOCKS_PER_SECOND, 0.0),
        aabb: Aabb::COW,
        on_ground: false,
    };

    for _ in 0..4 {
        body = step_entity(body, &world, PhysicsConfig::living_entity()).body;
    }

    assert!(body.position.y + body.aabb.height <= 66.0);
    assert!(body.velocity.y <= 0.0);
}

#[test]
fn water_applies_buoyancy_and_stronger_drag() {
    let world = FlatWorld {
        ground_y: 60,
        water_y: Some(64),
    };
    let body = EntityBody {
        position: Vec3::new(0.5, 64.0, 0.5),
        velocity: Vec3::new(1.0, -1.0, 0.0),
        aabb: Aabb::COW,
        on_ground: false,
    };

    let result = step_entity(body, &world, PhysicsConfig::default());

    assert!(result.in_fluid);
    assert!(result.body.velocity.x < 1.0);
    assert!(result.body.velocity.y > -1.0);
}

#[test]
fn material_ids_classify_fluids_apart_from_solids() {
    let ids = BlockMaterialIds::new(0, Some(5), Some(6));

    assert_eq!(ids.classify(0), BlockMaterial::Air);
    assert_eq!(ids.classify(5), BlockMaterial::Water);
    assert_eq!(ids.classify(6), BlockMaterial::Lava);
    assert_eq!(ids.classify(7), BlockMaterial::Solid);
}

#[test]
fn material_ids_classify_passable_apart_from_fluids() {
    let ids = BlockMaterialIds::new(0, Some(5), Some(6)).with_passable(vec![6, 7]);

    assert_eq!(ids.classify(5), BlockMaterial::Water);
    assert_eq!(ids.classify(6), BlockMaterial::Lava);
    assert_eq!(ids.classify(7), BlockMaterial::Air);
}

#[derive(Debug)]
struct LocalBlocks {
    solids: Vec<GridPos>,
    fluids: Vec<GridPos>,
}

impl BlockSampler for LocalBlocks {
    fn material_at(&self, x: i32, y: i32, z: i32) -> BlockMaterial {
        let pos = GridPos::new(x, y, z);
        if self.fluids.contains(&pos) {
            BlockMaterial::Water
        } else if self.solids.contains(&pos) {
            BlockMaterial::Solid
        } else {
            BlockMaterial::Air
        }
    }
}

#[test]
fn long_fall_clamps_velocity_before_ground_impact() {
    let world = LocalBlocks {
        solids: Vec::new(),
        fluids: Vec::new(),
    };
    let mut body = EntityBody {
        position: Vec3::new(0.5, 120.0, 0.5),
        velocity: Vec3::ZERO,
        aabb: Aabb::COW,
        on_ground: false,
    };

    for _ in 0..200 {
        body = step_entity(body, &world, PhysicsConfig::default()).body;
    }

    assert!(body.velocity.y >= TERMINAL_VELOCITY_BLOCKS_PER_SECOND);
    assert!(body.velocity.y < -10.0);
    assert!(!body.on_ground);
}

#[test]
fn representative_movement_boundary_matrix_uses_step_entity_authority() {
    #[derive(Clone, Copy)]
    enum Boundary {
        StepUp,
        LongFall,
        NonFinite,
    }

    for (name, boundary) in [
        ("farmland step-up", Boundary::StepUp),
        ("terminal long fall", Boundary::LongFall),
        ("non-finite rejection", Boundary::NonFinite),
    ] {
        match boundary {
            Boundary::StepUp => {
                let body = EntityBody {
                    position: Vec3::new(0.5, 63.0 + 15.0 / 16.0, 0.5),
                    velocity: Vec3::new(20.0, 0.0, 4.0),
                    aabb: Aabb::COW,
                    on_ground: true,
                };
                let result = step_entity(
                    body,
                    &FarmlandToFullBlockStepWorld,
                    PhysicsConfig::living_entity(),
                );
                assert!(!result.horizontal_collision, "{name}");
                assert_eq!(result.body.position.y, 64.0, "{name}");
                assert_eq!(result.body.velocity.y, 0.0, "{name}");
                assert!(result.body.on_ground, "{name}");
            }
            Boundary::LongFall => {
                let world = LocalBlocks {
                    solids: Vec::new(),
                    fluids: Vec::new(),
                };
                let mut body = EntityBody {
                    position: Vec3::new(0.5, 120.0, 0.5),
                    velocity: Vec3::ZERO,
                    aabb: Aabb::COW,
                    on_ground: false,
                };
                for _ in 0..200 {
                    body = step_entity(body, &world, PhysicsConfig::default()).body;
                }
                assert_eq!(
                    body.velocity.y, TERMINAL_VELOCITY_BLOCKS_PER_SECOND,
                    "{name} must reach the exact configured clamp"
                );
                let clamped = step_entity(body, &world, PhysicsConfig::default()).body;
                assert_eq!(
                    clamped.velocity.y, TERMINAL_VELOCITY_BLOCKS_PER_SECOND,
                    "{name} must remain at the clamp on the next step"
                );
                assert!(!clamped.on_ground, "{name}");
            }
            Boundary::NonFinite => {
                let body = EntityBody {
                    position: Vec3::new(0.5, 64.0, 0.5),
                    velocity: Vec3::new(f64::NAN, 0.0, 0.0),
                    aabb: Aabb::COW,
                    on_ground: true,
                };
                let result = step_entity(body, &PanicSampler, PhysicsConfig::default());
                assert_eq!(result.body.position, body.position, "{name}");
                assert_eq!(result.body.velocity, Vec3::ZERO, "{name}");
            }
        }
    }
}

/// Vanilla 26.3 `VoxelShape.collideX` (javap) picks the obstacle cells across
/// a move from the moving box shrunk by `1.0E-7` on each cross axis: a body
/// whose side lies a rounding error (3e-15) past a block boundary moves past
/// that block on every axis, in the axis resolver and in the sweep mobs use;
/// a side 2e-7 past the boundary is stopped. G2 relief take: a front-rank man
/// at x 32.3 (box from 31.999999999999996) stood for good against the trunk
/// at x 31 he never touched.
#[test]
fn cross_axis_overlap_under_the_vanilla_epsilon_does_not_collide() {
    let block = |min: [f64; 3]| WorldAabb {
        min_x: min[0],
        min_y: min[1],
        min_z: min[2],
        max_x: min[0] + 1.0,
        max_y: min[1] + 1.0,
        max_z: min[2] + 1.0,
    };
    for (overlap, blocked) in [(3.0e-15, false), (2.0e-7, true)] {
        // Walking -z 0.079 past a trunk at x 31, z 38, starting 0.01 from its
        // z face, the body's west side `overlap` into the trunk's column.
        let trunk = block([31.0, 0.0, 38.0]);
        let body = WorldAabb {
            min_x: 32.0 - overlap,
            min_y: 0.0,
            min_z: 39.01,
            max_x: 32.6 - overlap,
            max_y: 1.95,
            max_z: 39.61,
        };
        let expected = if blocked { 39.0 - 39.01 } else { -0.079 };
        let dz = clip_z(body, -0.079, &[trunk]);
        assert!(
            (dz - expected).abs() < 1e-12,
            "clip_z overlap {overlap}: {dz}"
        );
        let swept = resolve_horizontal_sweep(body, Vec3::new(0.0, 0.0, -0.079), &[trunk]);
        assert!(
            (swept.z - expected).abs() < 1e-12,
            "sweep overlap {overlap}: {swept:?}"
        );
        // Walking +x 0.5 toward a block at x 10, z 5 from 0.4 short of it, the
        // body's north side `overlap` into the block's row.
        let post = block([10.0, 0.0, 5.0]);
        let body = WorldAabb {
            min_x: 9.0,
            min_y: 0.0,
            min_z: 4.4 + overlap,
            max_x: 9.6,
            max_y: 1.95,
            max_z: 5.0 + overlap,
        };
        let expected = if blocked { 10.0 - 9.6 } else { 0.5 };
        let dx = clip_x(body, 0.5, &[post]);
        assert!(
            (dx - expected).abs() < 1e-12,
            "clip_x overlap {overlap}: {dx}"
        );
        // Falling 0.5 past a block whose column the body's west side overlaps,
        // starting 0.2 above its top.
        let ground = block([0.0, 63.0, 0.0]);
        let body = WorldAabb {
            min_x: 1.0 - overlap,
            min_y: 64.2,
            min_z: 0.2,
            max_x: 1.6 - overlap,
            max_y: 66.15,
            max_z: 0.8,
        };
        let expected = if blocked { 64.0 - 64.2 } else { -0.5 };
        let dy = clip_y(body, -0.5, &[ground]);
        assert!(
            (dy - expected).abs() < 1e-12,
            "clip_y overlap {overlap}: {dy}"
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GridPos {
    x: i32,
    y: i32,
    z: i32,
}

impl GridPos {
    #[must_use]
    const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
}

/// A body within the collision tolerance of a block face is touching that block.
/// `VoxelShape.collideX` (26.1.2) treats `d >= -1.0E-7` / `d <= 1.0E-7` as a
/// contact and `Shapes.collide` returns `0.0` for it; the entity step must never
/// advance it *through* the block, and it stays at most `COLLISION_EPSILON`
/// inside the face (the contact clamp blocks without pushing the body out).
///
/// The offset below is what an ordinary clip writes: at the pen's coordinates
/// `512.3 + (512.0 - (512.3 - 0.3)) - 0.3` is `-5.68e-14` below the face, and the
/// production evidence (prep11-125000 pen 64) has bodies 0.04..0.35 blocks inside
/// the same wall column, lifted onto it by a jump.
#[test]
fn wall_face_snap_never_lets_a_body_cross_the_face() {
    struct Pen;
    impl BlockSampler for Pen {
        fn material_at(&self, _x: i32, y: i32, z: i32) -> BlockMaterial {
            // Ground for y <= 63 (grass block at 63) and the pen's `-z` wall
            // column at z = 511 (blocks 511..512) for y 64..=70.
            if y <= 63 || (z == 511 && (64..=70).contains(&y)) {
                BlockMaterial::Solid
            } else {
                BlockMaterial::Air
            }
        }
    }
    let half_width = 0.3;
    let face = 512.0;
    let mut config = PhysicsConfig::living_entity();
    config.air_drag = 1.0;
    config.ground_friction = 1.0;
    config.vertical_air_drag = 0.98;
    for round_ulp in [0.0, -1.0e-13, -5.0e-8] {
        let mut body = EntityBody {
            position: Vec3::new(11.5, 64.0, face + half_width + round_ulp),
            velocity: Vec3::ZERO,
            aabb: Aabb {
                half_width,
                height: 1.95,
            },
            on_ground: true,
        };
        let mut deepest = f64::NEG_INFINITY;
        for tick in 0..80 {
            body.velocity.x = 0.0;
            body.velocity.z = -0.23;
            body = step_entity(body, &Pen, config).body;
            let penetration = face - (body.position.z - half_width);
            deepest = deepest.max(penetration);
            assert!(
                penetration <= COLLISION_EPSILON,
                "offset {round_ulp:e} tick {tick}: body is {penetration:e} behind the wall face \
                 (pos z {})",
                body.position.z
            );
        }
        assert!(
            deepest <= COLLISION_EPSILON,
            "offset {round_ulp:e}: deepest penetration {deepest:e}"
        );
        // The production state (prep11-125000 pen 64) is a body standing at
        // y = 65.00 *on* the wall's lowest block after jumping from inside it.
        // With no step and no support across the face, only the floor at y = 64
        // can hold the body.
        assert!(
            body.position.y < 64.5,
            "offset {round_ulp:e}: body rests on the wall block at y {}",
            body.position.y
        );
    }
}

/// The tolerance must not become a push-out: a body already embedded deeper
/// than `COLLISION_EPSILON` keeps today's semantics (vanilla has no unstick for
/// mobs; only items and experience orbs call `moveTowardsClosestSpace`).
#[test]
fn deeply_embedded_body_keeps_moving_without_a_push_out() {
    struct Pen;
    impl BlockSampler for Pen {
        fn material_at(&self, _x: i32, y: i32, z: i32) -> BlockMaterial {
            if y <= 63 || (z == 511 && (64..=70).contains(&y)) {
                BlockMaterial::Solid
            } else {
                BlockMaterial::Air
            }
        }
    }
    let mut config = PhysicsConfig::living_entity();
    config.air_drag = 1.0;
    config.ground_friction = 1.0;
    config.vertical_air_drag = 0.98;
    let mut body = EntityBody {
        position: Vec3::new(11.5, 64.0, 511.7),
        velocity: Vec3::ZERO,
        aabb: Aabb {
            half_width: 0.3,
            height: 1.95,
        },
        on_ground: true,
    };
    let start = body.position.z;
    for _ in 0..10 {
        body.velocity.z = -0.23;
        body = step_entity(body, &Pen, config).body;
    }
    assert!(
        body.position.z < start - 0.05,
        "an embedded body must keep its vanilla free movement, not be pushed out: {start} -> {}",
        body.position.z
    );
}

/// The clamp in the unit it is stated in: the signed face gap in blocks, tested
/// before the division. Both movement directions clamp at the boundary
/// (`-COLLISION_EPSILON`), a gap one step deeper than the tolerance keeps its
/// own entry time, and a non-negative gap is an approach, not a contact.
#[test]
fn contact_entry_clamps_the_face_gap_within_the_tolerance_for_both_directions() {
    let step = 0.0115;
    for movement in [step, -step] {
        assert_eq!(contact_entry(0.0, movement), 0.0, "{movement}");
        assert_eq!(contact_entry(-5.684_341_886_080_802e-14, movement), 0.0);
        assert_eq!(contact_entry(-COLLISION_EPSILON, movement), 0.0);
        assert!(
            contact_entry(-2.0 * COLLISION_EPSILON, movement) < 0.0,
            "a body deeper than the tolerance keeps its contact in the past"
        );
        assert_eq!(contact_entry(step, movement).abs(), 1.0, "{movement}");
    }
}

/// The invariant behind the contact clamp: over repeated ticks, on all four
/// horizontal faces, a body that starts at or within the contact tolerance of a
/// face never ends deeper than the tolerance behind it. Measured on the body the
/// resolver returns, not on the delta it chose, so any implementation that keeps
/// the face contact may satisfy it (`sweep_axis_interval` returns `entry = 0.0`
/// for a near-face penetration up to `COLLISION_EPSILON`).
#[test]
fn a_contact_never_grows_deeper_than_the_tolerance_behind_a_face() {
    let wall = WorldAabb {
        min_x: 10.0,
        min_y: 64.0,
        min_z: 512.0,
        max_x: 11.0,
        max_y: 65.0,
        max_z: 513.0,
    };
    let half = 0.3;
    let depth = |body: &WorldAabb, axis: usize, face: f64, side: f64| {
        let (min, max) = if axis == 0 {
            (body.min_x, body.max_x)
        } else {
            (body.min_z, body.max_z)
        };
        if side > 0.0 { max - face } else { face - min }
    };
    for axis in [0_usize, 2] {
        let (wall_lo, wall_hi) = if axis == 0 {
            (wall.min_x, wall.max_x)
        } else {
            (wall.min_z, wall.max_z)
        };
        for side in [1.0_f64, -1.0] {
            let face = if side > 0.0 { wall_lo } else { wall_hi };
            // Flush against the face, one rounding step inside, and halfway
            // into the tolerance.
            for start in [0.0, 5.684_341_886_080_802e-14, 5.0e-8] {
                let lo = if side > 0.0 {
                    face - 2.0 * half + start
                } else {
                    face - start
                };
                let hi = lo + 2.0 * half;
                let mut body = if axis == 0 {
                    WorldAabb {
                        min_x: lo,
                        max_x: hi,
                        min_y: 64.0,
                        max_y: 65.95,
                        min_z: 512.2,
                        max_z: 512.8,
                    }
                } else {
                    WorldAabb {
                        min_z: lo,
                        max_z: hi,
                        min_y: 64.0,
                        max_y: 65.95,
                        min_x: 10.2,
                        max_x: 10.8,
                    }
                };
                let step = if side > 0.0 { 0.0115 } else { -0.0115 };
                let desired = if axis == 0 {
                    Vec3::new(step, 0.0, 0.0)
                } else {
                    Vec3::new(0.0, 0.0, step)
                };
                for tick in 0..40 {
                    let moved = resolve_horizontal_sweep(body, desired, &[wall]);
                    body = body.moved(moved);
                    let behind = depth(&body, axis, face, side);
                    assert!(
                        behind <= COLLISION_EPSILON,
                        "axis {axis} face {face} side {side} start {start:e}: \
                         tick {tick} ended {behind:e} behind the face"
                    );
                }
            }
        }
    }
}

#[test]
fn a_sub_epsilon_axis_step_is_zero_on_every_axis() {
    // Vanilla `VoxelShape.collideX` returns 0.0 for `|d| < 1.0E-7` once the shape
    // is non-empty (javap, offsets 9..21); `collideY`/`collideZ` reach the same
    // body through `AxisCycle`. This kernel used to sweep the sub-epsilon step.
    let wall = WorldAabb {
        min_x: 1.0,
        min_y: 0.0,
        min_z: 0.0,
        max_x: 2.0,
        max_y: 1.0,
        max_z: 1.0,
    };
    let floor = WorldAabb {
        min_x: -10.0,
        min_y: -1.0,
        min_z: -10.0,
        max_x: 10.0,
        max_y: 0.0,
        max_z: 10.0,
    };
    let body = WorldAabb {
        min_x: 0.0,
        min_y: 0.5,
        min_z: 0.0,
        max_x: 0.6,
        max_y: 1.8,
        max_z: 0.6,
    };
    // A wall 0.4 blocks away is not reached by a 1e-8 step, but vanilla still
    // refuses the motion outright instead of moving by the tiny amount.
    assert_eq!(clip_x(body, 1.0e-8, std::slice::from_ref(&wall)), 0.0);
    assert_eq!(clip_x(body, -1.0e-8, std::slice::from_ref(&wall)), 0.0);
    // A floor 0.5 below the feet: the same refusal on the vertical axis.
    assert_eq!(clip_y(body, -1.0e-8, std::slice::from_ref(&floor)), 0.0);
    assert_eq!(clip_y(body, 1.0e-8, std::slice::from_ref(&floor)), 0.0);
    // And on z, through its own entry point.
    assert_eq!(clip_z(body, 1.0e-8, std::slice::from_ref(&wall)), 0.0);
    assert_eq!(clip_z(body, -1.0e-8, std::slice::from_ref(&wall)), 0.0);
    // The tolerance is strict and applies to a *non-empty* shape: `|d| == 1e-7`
    // does not take the early return, and just above it a box 0.4 away still
    // leaves the step alone (a guard `< 1e-3` or `<=` fails these).
    assert_eq!(clip_x(body, 1.0e-7, std::slice::from_ref(&wall)), 1.0e-7);
    assert_eq!(clip_x(body, 2.0e-7, std::slice::from_ref(&wall)), 2.0e-7);
    assert_eq!(clip_y(body, -2.0e-7, std::slice::from_ref(&floor)), -2.0e-7);
    // An empty shape still returns the motion unchanged (vanilla's empty early
    // return), sub-epsilon motion included.
    assert_eq!(clip_x(body, 2.0e-7, &[]), 2.0e-7);
    assert_eq!(clip_y(body, -2.0e-7, &[]), -2.0e-7);
    assert_eq!(clip_x(body, 1.0e-8, &[]), 1.0e-8);
}
