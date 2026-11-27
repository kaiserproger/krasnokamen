//! One admitted row's decision step and the pickup intents it stages.
//! Byte-exact move out of `entity_phase.rs`; the decision order is unchanged.

use super::*;

/// Ticks between a mob's melee hits (vanilla `MeleeAttackGoal`).
pub(in crate::tick) const MELEE_ATTACK_INTERVAL: u32 = 20;

/// Intent without a sequence number (assigned at drain time).
pub(in crate::tick) struct RawIntent {
    pub(in crate::tick) actor: EntityId,
    pub(in crate::tick) kind: IntentKind,
    pub(in crate::tick) footprint: Footprint,
}

/// Step one entity: returns its movement write plus raw intents.
/// Shared read-only context for entity stepping (no locks, no copies).
pub(in crate::tick) struct StepCtx<'a> {
    pub(in crate::tick) rows: &'a [ReadRow],
    /// Immutable through the indexed worker join; writes/compaction follow it.
    pub(in crate::tick) entities: &'a crate::entities::EntityStore,
    pub(in crate::tick) battle: &'a crate::battle::BattleState,
    pub(in crate::tick) retained_targets:
        &'a rustc_hash::FxHashMap<EntityId, (crate::target_grid::Target, f64)>,
    pub(in crate::tick) players: crate::target_grid::TargetGrid<'a>,
    pub(in crate::tick) passive: passive_follow::Context<'a>,
    pub(in crate::tick) dolphin_toys: &'a [[f64; 3]],
    pub(in crate::tick) dolphin_targets: &'a std::collections::HashMap<EntityId, [f64; 3]>,
    pub(in crate::tick) villagers: crate::target_grid::TargetGrid<'a>,
    pub(in crate::tick) golem_targets: crate::target_grid::TargetGrid<'a>,
    pub(in crate::tick) piglin_targets: crate::target_grid::TargetGrid<'a>,
    pub(in crate::tick) zoglin_targets: crate::target_grid::TargetGrid<'a>,
    pub(in crate::tick) cow_cells: &'a HashMap<(i32, i32), Vec<usize>>,
    pub(in crate::tick) world: &'a ChunkWorld,
    pub(in crate::tick) chunk_loaded: &'a (dyn Fn(i32, i32) -> bool + Sync),
    pub(in crate::tick) registry: &'a CoreRegistry,
    pub(in crate::tick) tick: u64,
    /// Content revision of a resident chunk (`u64::MAX` when absent).
    pub(in crate::tick) chunk_revision: &'a (dyn Fn(i32, i32) -> u64 + Sync),
    /// World revision the previous entity phase read.
    pub(in crate::tick) unchanged_since: u64,
    pub(in crate::tick) difficulty: mc_survival::Difficulty,
    pub(in crate::tick) sky_darken: u8,
}

/// Step one prepared row. The row is borrowed (a `ReadRow` is ~248 B and a
/// per-entity copy is pure snapshot traffic) and the physics reader belongs to
/// the calling lane, so its last-chunk/last-section memo survives between rows.
pub(in crate::tick) fn step_entity(
    row: &ReadRow,
    ctx: &StepCtx,
    reader: &crate::world::PhysicsReader<'_>,
) -> (MoveWrite, Vec<RawIntent>) {
    let rows = ctx.rows;
    let players = ctx.players.table();
    let cow_cells = ctx.cow_cells;
    let block_at = &|x, y, z| reader.block_at(x, y, z);
    let chunk_loaded = ctx.chunk_loaded;
    let registry = ctx.registry;
    let tick = ctx.tick;
    let mut intents = Vec::new();
    let mut chase_target = None;
    if row.item_entity {
        let sampler = WorkerSampler { block_at, registry };
        let body = crate::item_motion::step(
            mc_physics::EntityBody {
                position: mc_physics::Vec3::new(row.pos[0], row.pos[1], row.pos[2]),
                velocity: mc_physics::Vec3::new(row.vel[0], row.vel[1], row.vel[2]),
                aabb: mc_physics::Aabb {
                    half_width: row.half_width,
                    height: row.height,
                },
                on_ground: row.ground,
            },
            &sampler,
            row.no_gravity,
        );
        return (
            MoveWrite {
                slot: row.slot,
                chase_target: None,
                pos: [body.position.x, body.position.y, body.position.z],
                vel: [body.velocity.x, body.velocity.y, body.velocity.z],
                ground: body.on_ground,
                wall_climbing: false,
                horizontal_impulse: [0.0; 2],
                rotation: row.rotation,
                air_supply: row.air_supply,
            },
            intents,
        );
    }
    if row.special == Some(crate::hostile::Special::Projectile) {
        // Arrows fly in `hostile::run`: they read the world and every entity.
        return (
            MoveWrite {
                slot: row.slot,
                chase_target: None,
                pos: row.pos,
                vel: row.vel,
                ground: row.ground,
                wall_climbing: false,
                horizontal_impulse: [0.0; 2],
                rotation: row.rotation,
                air_supply: row.air_supply,
            },
            intents,
        );
    }
    if row.player {
        return (
            MoveWrite {
                chase_target: None,
                slot: row.slot,
                pos: row.pos,
                vel: [0.0; 3],
                ground: row.ground,
                wall_climbing: false,
                horizontal_impulse: [0.0; 2],
                rotation: row.rotation,
                air_supply: row.air_supply,
            },
            intents,
        );
    }
    // Mobs decide on their activation period, phase-shifted by id so each
    // tick carries an even share. Between decisions the last one keeps
    // executing (its horizontal velocity); physics and collision run every
    // tick either way.
    let think = row.think;
    let mut look_target = None;
    let mut vel = [0.0; 3];
    if row.battle {
        if !row.standing {
            let index = row.slot.0 as usize;
            vel = ctx
                .battle
                .columns
                .drive_by_slot
                .get(index)
                .copied()
                .unwrap_or([0.0; 3]);
            look_target = ctx
                .battle
                .columns
                .look_by_slot
                .get(index)
                .copied()
                .flatten();
        }
    } else if let RareMotion::Plugin(target) = row.rare_motion {
        vel = crate::entity_motion::steer(row.movement, row.pos, target, f64::from(row.speed));
        look_target = Some(target);
    } else if row.standing {
        // Riding/sitting suppress decisions, never gravity or collision.
        if row.leash_follow {
            vel = row.vel;
        }
    } else if row.panic_multiplier > 0.0 {
        // Rare branch only: the one `last_player_hit` cold read on the
        // worker path.
        let panic_source = ctx
            .entities
            .row(row.slot)
            .last_player_hit
            .filter(|(_, at)| tick.saturating_sub(*at) < 100)
            .and_then(|(attacker, _)| ctx.entities.slot_of(attacker))
            .map(|attacker| ctx.entities.pos(attacker));
        vel = crate::entity_motion::panic(
            row.movement,
            row.id.0,
            tick,
            f64::from(row.speed) * row.panic_multiplier,
            row.pos,
            panic_source,
        );
    } else if row.avoids_armadillos
        && let Some(threat) =
            crate::armadillo::avoidance_target(ctx.entities, ctx.world, ctx.registry, row.pos)
    {
        let target = [
            row.pos[0] + row.pos[0] - threat[0],
            row.pos[1],
            row.pos[2] + row.pos[2] - threat[2],
        ];
        vel =
            crate::entity_motion::steer(row.movement, row.pos, target, f64::from(row.speed) * 1.2);
    } else if crate::goat::is_goat(row.type_id)
        && let Some((target, speed)) = crate::goat::drive(ctx.entities.row(row.slot))
    {
        vel = crate::entity_motion::steer(row.movement, row.pos, target, speed);
        look_target = Some(target);
    } else if crate::turtle::is_turtle(row.type_id)
        && let Some((target, speed)) = crate::turtle::drive(ctx.entities.row(row.slot))
    {
        vel = crate::entity_motion::steer(row.movement, row.pos, target, speed);
        look_target = Some(target);
    } else if crate::blueprint::is_frog(row.type_id)
        && let Some((target, speed)) = crate::frog::drive(ctx.entities.row(row.slot))
    {
        vel = crate::entity_motion::steer(row.movement, row.pos, target, speed);
        look_target = Some(target);
    } else if crate::cat::is_cat(row.type_id)
        && let Some((target, speed)) = crate::cat::drive(
            ctx.entities.row(row.slot).entity_life.cat_relax.as_ref(),
            ctx.entities.row(row.slot).speed,
        )
    {
        vel = crate::entity_motion::steer(row.movement, row.pos, target, speed);
        look_target = Some(target);
    } else if !row.breed_ready
        && let Some((target, speed)) = crate::sniffer::drive(ctx.entities.row(row.slot))
    {
        vel = crate::entity_motion::steer(row.movement, row.pos, target, speed);
        look_target = Some(target);
    } else if !think {
        // A mob standing on the ground with no standing decision has nothing
        // to integrate while every chunk under its footprint is unchanged
        // since the last entity phase read it (a block edit, load or
        // eviction there wakes it this tick).
        if row.movement == crate::blueprint::MovementClass::Walker
            && row.ground
            && row.vel == [0.0; 3]
            && row.levitation <= 0.0
            && row.horizontal_impulse == [0.0; 2]
            && unchanged_entity_footprint(
                row.pos,
                row.half_width,
                ctx.chunk_revision,
                ctx.world.pending_block_chunks(),
                ctx.unchanged_since,
            )
        {
            return (
                MoveWrite {
                    chase_target: None,
                    slot: row.slot,
                    pos: row.pos,
                    vel: [0.0; 3],
                    ground: true,
                    wall_climbing: false,
                    horizontal_impulse: [0.0; 2],
                    rotation: row.rotation,
                    air_supply: row.air_supply,
                },
                intents,
            );
        }
        vel = [
            row.vel[0] - row.horizontal_impulse[0],
            row.vel[1],
            row.vel[2] - row.horizontal_impulse[1],
        ];
    } else if let RareMotion::Gold(target) = row.rare_motion {
        vel = crate::entity_motion::steer(row.movement, row.pos, target, f64::from(row.speed));
        look_target = Some(target);
    } else if crate::piglin::is_hoglin_type(row.type_id)
        && let Some(target) = crate::piglin::warped_fungus(row.pos, registry, block_at)
    {
        let away = [
            row.pos[0] * 2.0 - target[0],
            row.pos[1],
            row.pos[2] * 2.0 - target[2],
        ];
        vel = crate::entity_motion::steer(row.movement, row.pos, away, f64::from(row.speed));
    } else if registry
        .blueprint(row.type_id)
        .is_some_and(|bp| bp.capabilities.avoids_armadillos)
        && let Some(target) = crate::armadillo::flee_target(ctx.entities, row.pos)
    {
        vel =
            crate::entity_motion::steer(row.movement, row.pos, target, f64::from(row.speed) * 1.2);
    } else if row.chaser && !row.breed_ready {
        // Goals by priority: MeleeAttack, MeleeChase, Wander. Only chasers
        // need the nearest live player (compact table, no full-row scan).
        let mut nearest = if row.has_retained_target {
            ctx.retained_targets.get(&row.id).copied()
        } else {
            None
        };
        let sight = f64::from(if row.is_enderman {
            64.0
        } else {
            row.sight_range
        });
        let hunts_villagers = row.hunts_villagers;
        let golem = row.is_iron_golem;
        let players = if crate::piglin::is_piglin_type(row.type_id) {
            &ctx.piglin_targets
        } else {
            &ctx.players
        };
        // Chain order (players, villagers, golem targets, zoglin targets)
        // and table order break distance ties, as one linear scan did.
        if nearest.is_none() && !row.battle {
            if !golem {
                players.consider(&mut nearest, row.pos, sight, |(id, _)| {
                    row.enderman_target.is_none_or(|target| target == *id)
                });
            }
            if hunts_villagers {
                ctx.villagers
                    .consider(&mut nearest, row.pos, sight, |_| true);
            }
            if golem {
                // Vanilla IronGolem.canAttackType: a player-created golem never
                // targets players, even ones it was hit by. Player creation is
                // rare cold state; read it only on this branch.
                let spared = matches!(
                    ctx.entities.row(row.slot).special_mob.as_deref(),
                    Some(crate::special_mobs::Retained::PlayerIronGolem)
                );
                ctx.golem_targets
                    .consider(&mut nearest, row.pos, sight, |(id, _)| {
                        !(spared
                            && ctx.entities.slot_of(*id).is_some_and(|slot| {
                                matches!(ctx.entities.body(slot).kind, EntityKind::Player { .. })
                            }))
                    });
            }
            if row.hunts_living {
                ctx.zoglin_targets
                    .consider(&mut nearest, row.pos, sight, |(id, _)| *id != row.id);
            }
        }
        let target_admitted = !row.dark_aggro
            || ctx
                .entities
                .row(row.slot)
                .last_player_hit
                .is_some_and(|(_, tick)| ctx.tick.saturating_sub(tick) < 100)
            || ctx
                .world
                .light_at(
                    row.pos[0].floor() as i32,
                    row.pos[1].floor() as i32,
                    row.pos[2].floor() as i32,
                )
                .is_some_and(|(sky, block)| block.max(sky.saturating_sub(ctx.sky_darken)) < 8);
        if row.retain_target && !row.has_retained_target && target_admitted {
            chase_target = nearest;
        }
        let mut acted = false;
        if let Some(((target, target_pos), dist)) = nearest
            && target_admitted
        {
            look_target = Some(target_pos);
            if row.standing {
                // A creeper mid-fuse stands still.
                acted = true;
            } else if dist <= f64::from(row.attack_reach) {
                // In reach the mob stands and swings once per interval; a
                // bow user backs off from a close target instead.
                let ranged = matches!(
                    row.special,
                    Some(crate::hostile::Special::Ranged | crate::hostile::Special::Native(_))
                ) && !matches!(
                    row.special,
                    Some(crate::hostile::Special::Native(
                        crate::blueprint::GoalKind::SonicBoom | crate::blueprint::GoalKind::VexLife
                    ))
                );
                let attack_damage = if row.is_slime_size_one {
                    0.0
                } else {
                    (row.attack_damage + row.melee_bonus).max(0.0)
                };
                if !ranged && attack_damage > 0.0 && row.attack_cooldown == 0 {
                    intents.push(RawIntent {
                        actor: row.id,
                        kind: IntentKind::DamageEntity {
                            target,
                            amount: if ctx.entities.slot_of(target).is_some_and(|slot| {
                                matches!(ctx.entities.body(slot).kind, EntityKind::Player { .. })
                            }) {
                                ctx.difficulty.scale_mob_damage(attack_damage)
                            } else {
                                attack_damage
                            },
                            kind: mc_survival::DamageKind::Melee,
                            source: Some(crate::intent::DamageSource::Melee {
                                attacker: row.id,
                                position: row.pos,
                                yaw: 0.0,
                                bonus_knockback: 0.0,
                                sweep: false,
                                fire_ticks: 0,
                                bane_ticks: 0,
                            }),
                        },
                        footprint: Footprint {
                            entity_reads: vec![row.id],
                            entity_writes: vec![target],
                            ..Default::default()
                        },
                    });
                }
                if ranged && dist < crate::hostile::BOW_BACKOFF {
                    vel = crate::entity_motion::steer(
                        row.movement,
                        row.pos,
                        target_pos,
                        f64::from(row.speed) * crate::hostile::BACKOFF_SPEED,
                    )
                    .map(|value| -value);
                }
                acted = true;
            } else if dist > 0.5 {
                vel = crate::entity_motion::steer(
                    row.movement,
                    row.pos,
                    target_pos,
                    f64::from(row.speed),
                );
                acted = true;
            }
        }
        if !acted {
            vel = passive_follow::idle(row, ctx, &mut look_target);
        }
    } else if row.breed_ready {
        // Goals by priority: Breed, Wander.
        let mut bred = false;
        let cell = (
            mc_physics::mth::floor_i32(row.pos[0]) / 4,
            mc_physics::mth::floor_i32(row.pos[2]) / 4,
        );
        let mut found: Option<&ReadRow> = None;
        'cells: for dx in -2..=2 {
            for dz in -2..=2 {
                let Some(members) = cow_cells.get(&(cell.0 + dx, cell.1 + dz)) else {
                    continue;
                };
                for index in members {
                    let other = &rows[*index];
                    if other.id == row.id
                        || crate::entity_life::breeding::offspring(
                            registry,
                            row.type_id,
                            other.type_id,
                        )
                        .is_none()
                    {
                        continue;
                    }
                    let ox = other.pos[0] - row.pos[0];
                    let oy = other.pos[1] - row.pos[1];
                    let oz = other.pos[2] - row.pos[2];
                    if ox * ox + oy * oy + oz * oz < 64.0 {
                        found = Some(other);
                        break 'cells;
                    }
                }
            }
        }
        if let Some(other) = found {
            let delta = [
                other.pos[0] - row.pos[0],
                other.pos[1] - row.pos[1],
                other.pos[2] - row.pos[2],
            ];
            let distance = delta
                .into_iter()
                .map(|component| component * component)
                .sum::<f64>()
                .sqrt();
            look_target = Some(other.pos);
            vel = [
                delta[0] / distance.max(0.001) * f64::from(row.speed),
                0.0,
                delta[2] / distance.max(0.001) * f64::from(row.speed),
            ];
            bred = true;
            if distance < 3.0
                && (row.id < other.id || other.standing)
                && row.love <= crate::entity_life::LOVE_TICKS - 60
                && other.love <= crate::entity_life::LOVE_TICKS - 60
                && let Some(child_type) =
                    crate::entity_life::breeding::offspring(registry, row.type_id, other.type_id)
            {
                let partner = other.id;
                intents.push(RawIntent {
                    actor: row.id,
                    kind: IntentKind::Breed {
                        partner,
                        child_pos: std::array::from_fn(|axis| {
                            (row.pos[axis] + other.pos[axis]) * 0.5
                        }),
                        child_type,
                    },
                    footprint: Footprint {
                        entity_reads: vec![row.id, partner],
                        entity_writes: vec![row.id, partner],
                        ..Default::default()
                    },
                });
            }
        }
        if !bred {
            vel = passive_follow::idle(row, ctx, &mut look_target);
        }
    } else {
        vel = passive_follow::idle(row, ctx, &mut look_target);
    }
    if !row.standing
        && crate::dolphin::is_dolphin(row.type_id)
        && let Some(drive) = crate::dolphin::drive(
            row.pos,
            row.air_supply,
            row.speed,
            ctx.dolphin_targets.get(&row.id).copied(),
            players.iter().map(|(_, pos)| *pos),
            ctx.dolphin_toys.iter().copied(),
            block_at,
            registry,
        )
    {
        vel = drive;
    }
    let sampler = WorkerSampler { block_at, registry };
    // Only a hopping body needs its blueprint's delay scale and jump bonus; the
    // registry lookup and the cold row's slime size would otherwise be paid by
    // every mob every tick.
    let hop = row.hops.then(|| {
        let scale = registry
            .blueprint(row.type_id)
            .map_or(1, |bp| bp.capabilities.hop_delay_scale.max(1));
        let lift = registry.blueprint(row.type_id).map_or(0.0, |bp| {
            f64::from(ctx.entities.row(row.slot).loot.slime_size.unwrap_or(1))
                * bp.capabilities.hop_lift_per_size
        });
        (scale, lift)
    });
    let motion = crate::entity_motion::step(
        crate::entity_motion::MotionInput {
            body: mc_physics::EntityBody {
                position: mc_physics::Vec3::new(row.pos[0], row.pos[1], row.pos[2]),
                velocity: mc_physics::Vec3::new(row.vel[0], row.vel[1], row.vel[2]),
                aabb: mc_physics::Aabb {
                    half_width: row.half_width,
                    height: row.height,
                },
                on_ground: row.ground,
            },
            movement: row.movement,
            drive: vel,
            impulse: row.horizontal_impulse,
            rotation: row.rotation,
            look_target,
            facing: (row.battle && !row.standing)
                .then(|| ctx.battle.facing.of(row.slot))
                .flatten(),
            battle_facing: row.battle && !row.standing,
            legacy_facing: row.battle,
            head_follows_body: row.head_follows_body,
            hops: row.hops && !row.standing,
            hop_delay_scale: hop.map_or(1, |(delay, _)| delay),
            hop_lift: hop.map_or(0.0, |(_, lift)| lift),
            hop_aggressive: row.hop_aggressive,
            climbs_walls: row.climbs_walls,
            aquatic_control: row.aquatic_control,
            air_supply: row.air_supply,
            levitation: row.levitation,
            slow_falling: row.slow_falling,
            no_gravity: row.no_gravity,
            id: row.id.0,
            tick,
        },
        &sampler,
        chunk_loaded,
    );
    if motion.drowning {
        intents.push(RawIntent {
            actor: row.id,
            kind: IntentKind::DamageEntity {
                target: row.id,
                amount: 2.0,
                kind: mc_survival::DamageKind::Drowning,
                source: None,
            },
            footprint: Footprint {
                entity_writes: vec![row.id],
                ..Default::default()
            },
        });
    }
    (
        MoveWrite {
            chase_target,
            slot: row.slot,
            pos: [
                motion.body.position.x,
                motion.body.position.y,
                motion.body.position.z,
            ],
            vel: [
                motion.body.velocity.x,
                motion.body.velocity.y,
                motion.body.velocity.z,
            ],
            ground: motion.body.on_ground,
            wall_climbing: motion.wall_climbing,
            horizontal_impulse: motion.impulse,
            rotation: motion.rotation,
            air_supply: motion.air_supply,
        },
        intents,
    )
}

/// Mob activation rings in chunks around each player (see the entity
/// phase): every tick within [`MOB_NEAR_CHUNKS`], every 2 ticks within
/// [`MOB_MID_CHUNKS`], every [`MOB_FAR_PERIOD`] ticks beyond.
pub(in crate::tick) const MOB_NEAR_CHUNKS: i32 = 2;
pub(in crate::tick) const MOB_MID_CHUNKS: i32 = 4;
pub(in crate::tick) const MOB_FAR_PERIOD: u64 = 4;

/// Item pickup intents: live players collect mature item entities in reach.
pub(in crate::tick) fn collect_pickup_intents(state: &TickState, intents: &mut Vec<Intent>) {
    collect_pickup_intents_from(
        state.entities.bodies(),
        state.entities.positions(),
        state
            .entities
            .player_slots()
            .filter(|slot| !state.entity_session_locked(state.entities.body(*slot).id)),
        state.entities.item_slots(),
        |player, item| state.can_pickup_item(player, item),
        &state.seq_counter,
        intents,
    );
}

/// Inner pickup matcher over borrowed Body/position slices: exact kind lanes
/// (players, then items), authoritative maturity and post-move positions.
pub(in crate::tick) fn collect_pickup_intents_from(
    bodies: &[Body],
    positions: &[[f64; 3]],
    player_slots: impl Iterator<Item = EntitySlot>,
    item_slots: impl Iterator<Item = EntitySlot>,
    can_pickup: impl Fn(EntityId, EntityId) -> bool,
    seq_counter: &HashMap<EntityId, u32>,
    intents: &mut Vec<Intent>,
) {
    let mut players: Vec<(EntityId, [f64; 3])> = Vec::new();
    let mut items: Vec<(EntityId, [f64; 3])> = Vec::new();
    for slot in player_slots {
        let body = &bodies[slot.0 as usize];
        if body.alive {
            players.push((body.id, positions[slot.0 as usize]));
        }
    }
    for slot in item_slots {
        let body = &bodies[slot.0 as usize];
        if body.alive && body.pickup_delay == 0 {
            items.push((body.id, positions[slot.0 as usize]));
        }
    }
    for (player, player_pos) in &players {
        for (item, item_pos) in &items {
            let dx = item_pos[0] - player_pos[0];
            let dy = item_pos[1] - player_pos[1];
            let dz = item_pos[2] - player_pos[2];
            // Vanilla Player.aiStep checks the player's box inflated by
            // (1, .5, 1), intersecting the item's .25-wide/.25-high box.
            if dx.abs() >= 1.425 || dz.abs() >= 1.425 || dy <= -0.75 || dy >= 2.3 {
                continue;
            }
            if !can_pickup(*player, *item) {
                continue;
            }
            let next = seq_counter.get(player).copied().unwrap_or(0);
            intents.push(Intent {
                actor: *player,
                seq: next,
                kind: IntentKind::PickupItem {
                    item: *item,
                    count: i32::MAX,
                },
                footprint: Footprint {
                    entity_reads: vec![*player],
                    item_take: Some(*item),
                    ..Default::default()
                },
            });
            break;
        }
    }
}
