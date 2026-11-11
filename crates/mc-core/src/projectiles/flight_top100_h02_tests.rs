use super::*;
use mc_data::{Identifier, ItemStack};

fn item(state: &TickState, name: &str) -> ItemStack {
    ItemStack::new(
        state
            .registry
            .items
            .id_of(&Identifier::parse(name).unwrap())
            .unwrap(),
        1,
    )
}

fn mob(state: &mut TickState, name: &str, pos: [f64; 3]) -> EntityId {
    let kind = state
        .registry
        .entity_types
        .id_of(&Identifier::parse(name).unwrap())
        .unwrap();
    state.spawn_mob(kind, pos, 20.0)
}

fn scene() -> (TickState, EntityId) {
    let (mut state, player, _) = crate::projectiles::flight_tests::setup();
    let chunks: Vec<_> = (-5..=5)
        .flat_map(|x| (-2..=2).map(move |z| crate::boundary::ChunkPos { x, z }))
        .collect();
    state
        .world
        .ensure_blocking(&chunks.iter().map(|&chunk| (chunk, 0)).collect());
    for chunk in chunks {
        state.tickets.insert(format!("h02:{chunk:?}"), (chunk, 0));
    }
    state.refresh_active_chunks(state.tick);
    let slot = state.entities.slot_of(player).unwrap();
    state.entities.set_pos(slot, [-40.0, 201.0, 20.0]);
    (state, player)
}

fn arrow(
    state: &mut TickState,
    player: EntityId,
    from: [f64; 3],
    velocity: [f64; 3],
    piercing: u32,
) -> EntityId {
    let owner = state
        .entities
        .row(state.entities.slot_of(player).unwrap())
        .uuid;
    let mut record = ProjectileState::new(
        ProjectileKind::Arrow,
        Some(owner),
        item(state, "minecraft:arrow"),
    );
    record.pickup = true;
    record.piercing = piercing;
    record.punch = 2;
    super::super::launch(state, record, from, velocity).unwrap()
}

/// The exhaustive snapshot: every hittable body, with items whenever any live
/// fishing hook exists anywhere in the store.
fn every_body(state: &TickState, bodies: &mut Vec<Hittable>) {
    let hook = state.entities.slots().any(|slot| {
        state.entities.body(slot).alive
            && state
                .entities
                .row(slot)
                .projectile
                .as_deref()
                .is_some_and(|record| record.kind == ProjectileKind::FishingHook)
    });
    hittables(state, hook, None, bodies);
}

fn exhaustive(state: &mut TickState, intents: &mut Vec<Intent>, ids: &[EntityId]) {
    if ids.is_empty() {
        return;
    }
    let mut bodies = Vec::new();
    every_body(state, &mut bodies);
    let candidates: Vec<_> = (0..bodies.len()).collect();
    run_shots(
        state,
        intents,
        ids.iter().map(|&id| (id, candidates.as_slice())),
        &bodies,
        &mut Vec::new(),
    );
}

fn identities(state: &TickState) -> Vec<(String, String)> {
    state
        .entities
        .slots()
        .map(|slot| {
            let body = state.entities.body(slot);
            (
                state.entities.row(slot).uuid.to_string(),
                Uuid::from_u128(body.id.0 as u128).to_string(),
            )
        })
        .collect()
}

fn normalize(mut text: String, identities: &[(String, String)]) -> String {
    for (uuid, stable) in identities {
        text = text.replace(uuid, stable);
    }
    text
}

/// Includes ordered damage/source/sequence, hit UUID order and ownership, all
/// saved projectile fields, motion, surviving bodies, dropped stacks and custody.
fn digest(state: &TickState, intents: &[Intent], names: &[(String, String)]) -> String {
    let mut result = normalize(format!("{intents:?}"), names);
    for slot in state.entities.slots() {
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        result.push_str(&format!(
            "{:?}:{:?}:{:?}:{:?}:{:?}:{:?};",
            body.id,
            body.alive,
            row.health,
            state.entities.pos(slot),
            state.entities.vel(slot),
            row.item
        ));
        if let Some(record) = row.projectile.as_deref() {
            let stable = normalize(serde_json::to_string(record).unwrap(), names);
            let stable: ProjectileState = serde_json::from_str(&stable).unwrap();
            result.push_str(&format!(
                "{:?}",
                crate::projectiles::persistence::to_fields(&stable, &state.registry.items, row.age)
            ));
            result.push_str(&format!("{stable:?}"));
        }
    }
    let mut players: Vec<_> = state.players.iter().collect();
    players.sort_unstable_by_key(|(id, _)| **id);
    for (id, player) in players {
        result.push_str(&format!("{id:?}:{:?};", player.inventory.slots));
    }
    result
}

#[test]
fn exhaustive_flight_oracle_covers_negative_seams_long_sweeps_ties_and_swap_removals() {
    for (origin, speed, piercing, shots) in [
        (-16.7, 2.0, 0, 1),
        (-48.0, 96.0, 7, 1),
        (-32.0, 64.0, 1, 3),
        (-16.7, 2.0, 0, 16),
        (-48.0, 96.0, 7, 64),
        (-32.0, 64.0, 1, 256),
    ] {
        let simulate = |indexed| {
            let (mut state, player) = scene();
            for (x, z) in [(-15.8, -0.1), (-15.8, 0.1), (0.2, 0.0), (31.9, 0.0)] {
                mob(&mut state, "minecraft:cow", [x, 201.0, z]);
            }
            // An out-of-range body must not affect either the hit or its order.
            mob(&mut state, "minecraft:cow", [70.0, 201.0, 25.0]);
            let ids: Vec<_> = (0..shots)
                .map(|_| {
                    arrow(
                        &mut state,
                        player,
                        [origin, 201.7, -0.1],
                        [speed, 0.0, 0.0],
                        piercing,
                    )
                })
                .collect();
            let names = identities(&state);
            let mut intents = Vec::new();
            if indexed {
                run(&mut state, &mut intents, ids.into_iter());
                if shots >= FineGrid::MIN_SHOTS {
                    assert_ne!(
                        state.projectiles.grid.size, [0; 2],
                        "oracle must exercise the fine grid"
                    );
                }
            } else {
                exhaustive(&mut state, &mut intents, &ids);
            }
            assert!(!intents.is_empty(), "fixture must actually hit a body");
            digest(&state, &intents, &names)
        };
        assert_eq!(
            simulate(true),
            simulate(false),
            "origin={origin}, speed={speed}, piercing={piercing}, shots={shots}"
        );
    }
}

#[test]
fn neighbouring_chunk_omission_changes_boundary_hit_oracle() {
    let (mut state, player) = scene();
    let victim = mob(&mut state, "minecraft:cow", [-15.8, 201.0, -0.1]);
    let id = arrow(&mut state, player, [-16.7, 201.7, -0.1], [2.0, 0.0, 0.0], 0);
    let mut bodies = Vec::new();
    every_body(&state, &mut bodies);
    let mut candidates = Vec::new();
    shot_candidates(
        &state,
        id,
        &bodies,
        &[],
        None,
        &mut Vec::new(),
        &mut candidates,
    );
    assert!(candidates.iter().any(|&i| bodies[i].id == victim));
    // Negative-control mutant: retain only the projectile's starting chunk.
    let bad: Vec<_> = candidates
        .iter()
        .copied()
        .filter(|&i| {
            bodies[i]
                .slot
                .is_some_and(|s| state.entities.chunk_of(s).x == -2)
        })
        .collect();
    let hit = |indices: &[usize]| {
        indices
            .iter()
            .filter_map(|&i| {
                let body = &bodies[i];
                segment_enters(
                    [-16.7, 201.7, -0.1],
                    [2.0, 0.0, 0.0],
                    body.min.map(|v| v - 0.3),
                    body.max.map(|v| v + 0.3),
                )
                .map(|t| (t, body.id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|(_, id)| id)
    };
    assert_eq!(hit(&candidates), Some(victim));
    assert_ne!(
        hit(&bad),
        hit(&candidates),
        "omitting the neighbouring chunk must break the oracle"
    );
}

#[test]
fn wide_moved_body_and_duplicate_memberships_remain_conservative() {
    let (mut state, player) = scene();
    let victim = mob(&mut state, "minecraft:cow", [50.0, 201.0, 0.0]);
    let slot = state.entities.slot_of(victim).unwrap();
    state.entities.set_half_width(slot, 36.0);
    state.entities.set_pos(slot, [18.0, 201.0, -0.1]);
    let id = arrow(&mut state, player, [-20.0, 201.7, -0.1], [4.0, 0.0, 0.0], 0);
    let mut bodies = Vec::new();
    every_body(&state, &mut bodies);
    let mut candidates = Vec::new();
    shot_candidates(
        &state,
        id,
        &bodies,
        &[],
        None,
        &mut Vec::new(),
        &mut candidates,
    );
    assert!(candidates.iter().any(|&i| bodies[i].id == victim));
}

#[test]
fn stuck_pickup_keeps_dense_slot_priority_and_trident_ownership() {
    for kind in [ProjectileKind::Arrow, ProjectileKind::Trident] {
        let (mut state, owner) = scene();
        let pos = [-16.1, 201.0, -0.1];
        let slot = state.entities.slot_of(owner).unwrap();
        state.entities.set_pos(slot, [-15.9, 201.0, -0.1]);
        let uuid = state.entities.row(slot).uuid;
        let mut record = ProjectileState::new(
            kind,
            Some(uuid),
            item(
                &state,
                if kind == ProjectileKind::Arrow {
                    "minecraft:arrow"
                } else {
                    "minecraft:trident"
                },
            ),
        );
        record.pickup = true;
        record.stuck = true;
        assert!(pickup(&mut state, &record, pos, &mut Vec::new()));
        assert_eq!(
            state.players[&owner]
                .inventory
                .slots
                .iter()
                .map(|s| s.count.max(0))
                .sum::<i32>(),
            1
        );
    }
}

#[test]
fn riptide_candidates_match_exhaustive_swept_targets_in_dense_order() {
    let (mut state, actor) = scene();
    for x in [-48.0, -16.1, -15.9, 0.0, 31.9, 65.0] {
        mob(&mut state, "minecraft:cow", [x, 201.0, -0.1]);
    }
    let start = [-50.0, 201.0, -0.1];
    let end = [48.0, 201.0, -0.1];
    let slot = state.entities.slot_of(actor).unwrap();
    state.entities.set_pos(slot, end);
    let expected: Vec<_> = state
        .entities
        .slots()
        .filter_map(|slot| {
            let body = state.entities.body(slot);
            let at = state.entities.pos(slot);
            let min = [
                at[0] - body.half_width - 0.5,
                at[1] - 1.8,
                at[2] - body.half_width - 0.5,
            ];
            let max = [
                at[0] + body.half_width + 0.5,
                at[1] + body.height,
                at[2] + body.half_width + 0.5,
            ];
            (body.id != actor && segment_enters(start, [98.0, 0.0, 0.0], min, max).is_some())
                .then_some(body.id)
        })
        .collect();
    let stack = item(&state, "minecraft:trident");
    state.projectiles.riptides.insert(
        actor,
        super::super::weapons::Riptide {
            until: state.tick + 20,
            previous: start,
            hit: Vec::new(),
            stack,
        },
    );
    let mut intents = Vec::new();
    super::super::weapons::tick(&mut state, &mut intents);
    let actual: Vec<_> = intents
        .iter()
        .filter_map(|i| match i.kind {
            IntentKind::DamageEntity { target, .. } => Some(target),
            _ => None,
        })
        .collect();
    assert_eq!(actual, expected);
    assert!(!actual.is_empty());
    let hit = &state.projectiles.riptides[&actor].hit;
    assert_eq!(hit.len(), expected.len());
    super::super::weapons::tick(&mut state, &mut intents);
    assert_eq!(
        intents.len(),
        expected.len(),
        "riptide must not damage a UUID twice"
    );
}

#[test]
fn rejected_fast_or_nonfinite_flight_does_not_expand_world_query() {
    let (mut state, player) = scene();
    let id = arrow(&mut state, player, [-16.7, 201.7, -0.1], [2.0, 0.0, 0.0], 0);
    let slot = state.entities.slot_of(id).unwrap();
    let mut bodies = Vec::new();
    every_body(&state, &mut bodies);
    for speed in [f64::INFINITY, f64::NAN, 1e20] {
        state.entities.set_vel(slot, [speed; 3]);
        let mut candidates = Vec::new();
        shot_candidates(
            &state,
            id,
            &bodies,
            &[],
            None,
            &mut Vec::new(),
            &mut candidates,
        );
        assert!(candidates.is_empty());
    }
}

#[test]
fn world_spanning_sweep_visits_only_existing_chunk_memberships() {
    let (mut state, _) = scene();
    mob(&mut state, "minecraft:cow", [30.0, 201.0, 30.0]);
    let mut slots = Vec::new();
    swept_slots(
        &state,
        [-30_000_000.0, 201.0, -30_000_000.0],
        [30_000_000.0, 201.0, 30_000_000.0],
        0.5,
        &mut slots,
    );
    assert_eq!(slots, state.entities.slots().collect::<Vec<_>>());
}

/// Diagnostic M1/M3-shaped owner micro, not the native-100 capacity gate.
/// Both paths include snapshot construction, collision, ordered intents and
/// retained projectile writeback. Scene creation is outside each timed window.
#[test]
#[ignore = "run in mem-run -x with --ignored --nocapture; diagnostic only"]
fn projectile_candidate_cpu_micro() {
    let count: usize = std::env::var("H02_RESIDENTS")
        .unwrap_or_else(|_| "100000".into())
        .parse()
        .unwrap();
    let (mut state, player) = scene();
    for index in 0..count {
        let x = 256.0 + (index % 1024) as f64 * 3.0;
        let z = 256.0 + (index / 1024) as f64 * 3.0;
        mob(&mut state, "minecraft:cow", [x, 201.0, z]);
    }
    // Real local collision and piercing work, not a Q=0-only comparison.
    mob(&mut state, "minecraft:cow", [-15.8, 201.0, -0.1]);
    mob(&mut state, "minecraft:cow", [-14.8, 201.0, -0.1]);
    let cpu_ticks = || {
        let stat = std::fs::read_to_string("/proc/thread-self/stat").unwrap();
        let fields: Vec<_> = stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .collect();
        fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
    };
    for shots in [0usize, 100, 1000] {
        for indexed in [false, true, false, true] {
            let ids: Vec<_> = (0..shots)
                .map(|_| arrow(&mut state, player, [-16.7, 201.7, -0.1], [3.0, 0.0, 0.0], 4))
                .collect();
            let mut bodies = Vec::new();
            every_body(&state, &mut bodies);
            let mut h = 0;
            let mut slots = Vec::new();
            let mut candidates = Vec::new();
            for &id in &ids {
                candidates.clear();
                shot_candidates(&state, id, &bodies, &[], None, &mut slots, &mut candidates);
                h += candidates.len();
            }
            assert!(
                shots == 0 || h <= 2 * shots,
                "distant population must not enter per-shot narrow phase"
            );
            let mut intents = Vec::new();
            let cpu = cpu_ticks();
            let start = std::time::Instant::now();
            if indexed {
                run(&mut state, &mut intents, ids.iter().copied());
            } else {
                exhaustive(&mut state, &mut intents, &ids);
            }
            let wall_us = start.elapsed().as_micros();
            let cpu = cpu_ticks() - cpu;
            assert_eq!(intents.len(), 2 * shots);
            eprintln!(
                "H02_MICRO residents={count} shots={shots} indexed={indexed} candidates={} exhaustive_candidates={} wall_us={wall_us} thread_cpu_ticks={cpu} cpu_tick_hz=100",
                h,
                bodies.len() * shots
            );
            for id in ids {
                if state.entities.slot_of_any(id).is_some() {
                    super::super::remove(&mut state, id);
                }
            }
        }
    }
}

#[test]
fn fishing_item_and_arrow_saved_state_match_exhaustive_snapshot() {
    let simulate = |indexed| {
        let (mut state, owner) = scene();
        let owner_slot = state.entities.slot_of(owner).unwrap();
        state.entities.set_pos(owner_slot, [-18.0, 201.0, -0.1]);
        let rod = item(&state, "minecraft:fishing_rod");
        state.players.get_mut(&owner).unwrap().inventory.slots[0] = rod.clone();
        let uuid = state.entities.row(owner_slot).uuid;
        let stack = item(&state, "minecraft:diamond");
        let dropped = crate::tick::spawn_item_entity(&mut state, [-15.8, 201.0, -0.1], stack);
        let arrow = arrow(&mut state, owner, [-16.7, 201.3, -0.1], [2.0, 0.0, 0.0], 0);
        let record = ProjectileState::new(ProjectileKind::FishingHook, Some(uuid), rod);
        let hook = super::super::launch(&mut state, record, [-16.7, 201.3, -0.1], [2.0, 0.0, 0.0])
            .unwrap();
        let names = identities(&state);
        let mut intents = Vec::new();
        if indexed {
            run(&mut state, &mut intents, [arrow, hook].into_iter());
        } else {
            exhaustive(&mut state, &mut intents, &[arrow, hook]);
        }
        let slot = state.entities.slot_of(hook).unwrap();
        let super::super::ProjectileData::Fishing(fishing) =
            &state.entities.row(slot).projectile.as_deref().unwrap().data
        else {
            panic!("hook retained fishing state")
        };
        assert_eq!(
            fishing.hooked,
            Some(
                state
                    .entities
                    .row(state.entities.slot_of(dropped).unwrap())
                    .uuid
            )
        );
        assert!(intents.is_empty(), "arrow must pass the dropped item");
        digest(&state, &intents, &names)
    };
    assert_eq!(simulate(true), simulate(false));
}

#[test]
fn dragon_multipart_hit_and_retained_uuid_match_exhaustive_snapshot() {
    let simulate = |indexed| {
        let (mut state, owner) = scene();
        let dragon = mob(&mut state, "minecraft:ender_dragon", [32.0, 210.0, -0.1]);
        let slot = state.entities.slot_of(dragon).unwrap();
        state.end_fight.dragon_uuid = Some(state.entities.row(slot).uuid);
        let (min, max) = crate::end_fight::part_bounds(&state, EntityId(dragon.0 + 1)).unwrap();
        let at: [f64; 3] = std::array::from_fn(|axis| (min[axis] + max[axis]) * 0.5);
        let ids: Vec<_> = (0..FineGrid::MIN_SHOTS)
            .map(|_| {
                arrow(
                    &mut state,
                    owner,
                    [at[0] - 8.0, at[1], at[2]],
                    [16.0, 0.0, 0.0],
                    4,
                )
            })
            .collect();
        let id = ids[0];
        let names = identities(&state);
        let mut intents = Vec::new();
        if indexed {
            run(&mut state, &mut intents, ids.iter().copied());
            assert_ne!(state.projectiles.grid.size, [0; 2]);
        } else {
            exhaustive(&mut state, &mut intents, &ids);
        }
        assert!(!intents.is_empty());
        let record = state
            .entities
            .row(state.entities.slot_of(id).unwrap())
            .projectile
            .as_deref()
            .unwrap();
        assert_eq!(record.hit, vec![state.end_fight.dragon_uuid.unwrap()]);
        digest(&state, &intents, &names)
    };
    assert_eq!(simulate(true), simulate(false));
}

#[test]
fn deployed_candidate_outside_body_chunks_is_not_pruned() {
    let (mut state, owner) = scene();
    let stand = mob(&mut state, "minecraft:armor_stand", [70.0, 201.0, 25.0]);
    let id = arrow(&mut state, owner, [-16.7, 201.7, -0.1], [2.0, 0.0, 0.0], 0);
    let mut bodies = Vec::new();
    every_body(&state, &mut bodies);
    let exceptional: Vec<_> = bodies
        .iter()
        .enumerate()
        .filter_map(|(i, b)| b.deployed_candidate.then_some(i))
        .collect();
    let mut grid = FineGrid::default();
    assert!(grid.build(&bodies, 0));
    for grid in [None, Some(&grid)] {
        let mut candidates = Vec::new();
        shot_candidates(
            &state,
            id,
            &bodies,
            &exceptional,
            grid,
            &mut Vec::new(),
            &mut candidates,
        );
        assert!(candidates.iter().any(|&i| bodies[i].id == stand));
    }
}

#[test]
fn shooter_grid_preserves_linear_target_ties_and_projectile_ownership() {
    let simulate = |indexed| {
        let (mut state, _) = scene();
        let shooter = mob(&mut state, "minecraft:shulker", [-16.0, 205.0, -0.1]);
        let mut targets = Vec::new();
        for pos in [[-14.0, 205.0, -0.1], [-18.0, 205.0, -0.1]] {
            let id = mob(&mut state, "minecraft:cow", pos);
            targets.push((id, pos));
        }
        for index in 0..64 {
            let at = [70.0, 205.0, index as f64 * 3.0];
            let id = mob(&mut state, "minecraft:cow", at);
            targets.push((id, at));
        }
        state.hostile.native.attacks.insert(
            shooter,
            crate::special_mobs::Attack {
                deadline: state.tick,
                ..Default::default()
            },
        );
        let grid = if indexed {
            crate::target_grid::TargetGrid::indexed(&targets)
        } else {
            crate::target_grid::TargetGrid::linear(&targets)
        };
        crate::special_mobs::run(
            &mut state,
            &mut Vec::new(),
            shooter,
            crate::blueprint::GoalKind::HomingShot,
            &grid,
        );
        let record = state
            .entities
            .slots()
            .find_map(|slot| state.entities.row(slot).projectile.as_deref())
            .expect("shooter must actually launch");
        assert_eq!(
            record.owner,
            Some(
                state
                    .entities
                    .row(state.entities.slot_of(shooter).unwrap())
                    .uuid
            )
        );
        assert_eq!(
            record.data,
            super::super::ProjectileData::Homing {
                target: state
                    .entities
                    .row(state.entities.slot_of(targets[0].0).unwrap())
                    .uuid
            }
        );
        normalize(serde_json::to_string(record).unwrap(), &identities(&state))
    };
    assert_eq!(simulate(true), simulate(false));
}

#[test]
fn pickup_matches_exhaustive_player_priority_after_swap_remove() {
    let simulate = |indexed, kind| {
        let (mut state, first) = scene();
        let removed = mob(&mut state, "minecraft:cow", [40.0, 201.0, 20.0]);
        crate::tick::run_tick(
            &mut state,
            &[crate::boundary::PlayerInput::Join {
                connection: crate::boundary::ConnectionId(2),
                uuid: Uuid::from_u128(200),
                name: "h02-second".into(),
                permission_level: 0,
            }],
        );
        let second = state
            .connections
            .entity_of(crate::boundary::ConnectionId(2))
            .unwrap();
        state
            .entities
            .remove(state.entities.slot_of(removed).unwrap());
        for (id, x) in [(first, -15.9), (second, -16.2)] {
            let slot = state.entities.slot_of(id).unwrap();
            state.entities.set_pos(slot, [x, 201.0, -0.1]);
        }
        let second_uuid = state
            .entities
            .row(state.entities.slot_of(second).unwrap())
            .uuid;
        let mut record =
            ProjectileState::new(kind, Some(second_uuid), item(&state, "minecraft:trident"));
        record.pickup = true;
        let pos = [-16.1, 201.0, -0.1];
        let names = identities(&state);
        let picked = if indexed {
            pickup(&mut state, &record, pos, &mut Vec::new())
        } else {
            let mut picked = false;
            for index in 0..state.entities.len() as u32 {
                let slot = EntitySlot(index);
                let body = state.entities.body(slot);
                let id = body.id;
                if body.alive
                    && state.players.get(&id).is_some_and(|p| !p.spectator())
                    && state
                        .entities
                        .pos(slot)
                        .iter()
                        .zip(pos)
                        .map(|(a, b)| (a - b).powi(2))
                        .sum::<f64>()
                        < 2.25
                    && (kind != ProjectileKind::Trident
                        || super::super::owner_id(&state, &record) == Some(id))
                    && give(&mut state, id, record.stack.clone())
                {
                    picked = true;
                    break;
                }
            }
            picked
        };
        assert!(picked);
        let recipient = if kind == ProjectileKind::Trident {
            second
        } else {
            first
        };
        assert_eq!(
            state.players[&recipient]
                .inventory
                .slots
                .iter()
                .map(|s| s.count.max(0))
                .sum::<i32>(),
            1
        );
        digest(&state, &[], &names)
    };
    for kind in [ProjectileKind::Arrow, ProjectileKind::Trident] {
        assert_eq!(simulate(true, kind), simulate(false, kind));
    }
}

#[test]
fn fine_grid_twenty_thousand_bodies_bounds_work_and_rejects_other_heights() {
    let bodies: Vec<_> = (0..20_000)
        .map(|i| {
            let x = (i % 200) as f64 - 100.0;
            let z = (i / 200) as f64 * 2.0 - 100.0;
            let y = if i % 3 == 0 { 90.0 } else { 64.0 };
            Hittable {
                slot: Some(EntitySlot(i)),
                id: EntityId(i as i32 + 1),
                uuid: Uuid::from_u128(i as u128 + 1),
                deployed_candidate: false,
                item: false,
                battle_faction: 0,
                min: [x - 0.3, y, z - 0.3],
                max: [x + 0.3, y + 1.8, z + 0.3],
            }
        })
        .collect();
    let mut grid = FineGrid::default();
    assert!(grid.build(&bodies, 0));
    assert_eq!(
        grid.items.len(),
        bodies.len(),
        "one counting-sort entry per body"
    );
    assert!(grid.offsets.len() <= 10_101);
    let storage = grid.items.as_ptr();
    assert!(grid.build(&bodies, 0));
    assert_eq!(
        grid.items.as_ptr(),
        storage,
        "rebuild reuses retained scratch"
    );
    let mut examined = 0;
    let mut candidates = Vec::new();
    for shot in 0..400 {
        let from = [
            (shot % 100) as f64 * 1.8 - 99.5,
            64.7,
            (shot / 100) as f64 * 20.0 - 60.0,
        ];
        let delta = [4.5, -0.1, 0.0];
        let end = std::array::from_fn(|axis| from[axis] + delta[axis]);
        candidates.clear();
        examined += grid.query(&bodies, from, end, 0.3, &mut candidates);
        candidates.sort_unstable();
        let hits = |indices: Vec<usize>| {
            indices
                .into_iter()
                .filter(|&i| {
                    segment_enters(
                        from,
                        delta,
                        bodies[i].min.map(|v| v - 0.3),
                        bodies[i].max.map(|v| v + 0.3),
                    )
                    .is_some()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(hits(candidates.clone()), hits((0..bodies.len()).collect()));
    }
    assert!(
        examined < 400 * 32,
        "candidate visits={examined}; no per-shot full/chunk scan"
    );
    candidates.clear();
    assert!(
        grid.query(
            &bodies,
            [0.0, 80.0, 0.0],
            [4.5, 80.0, 0.0],
            0.3,
            &mut candidates
        ) > 0
    );
    assert!(
        candidates.is_empty(),
        "XZ neighbours at other heights must be rejected"
    );
}

#[test]
fn fine_grid_includes_wide_bodies_across_negative_cell_edges_and_bounds_sparse_storage() {
    let mut bodies = vec![Hittable {
        slot: Some(EntitySlot(0)),
        id: EntityId(1),
        uuid: Uuid::from_u128(1),
        deployed_candidate: false,
        item: false,
        battle_faction: 0,
        min: [-9.0, 64.0, -3.0],
        max: [3.0, 70.0, 9.0],
    }];
    let mut grid = FineGrid::default();
    assert!(grid.build(&bodies, 0));
    let mut candidates = Vec::new();
    grid.query(
        &bodies,
        [-9.2, 65.0, -3.2],
        [-8.9, 65.0, -2.9],
        0.3,
        &mut candidates,
    );
    assert_eq!(
        candidates,
        [0],
        "center is several fine cells outside the sweep"
    );
    bodies.push(Hittable {
        slot: Some(EntitySlot(1)),
        id: EntityId(2),
        uuid: Uuid::from_u128(2),
        deployed_candidate: false,
        item: false,
        battle_faction: 0,
        min: [29_999_999.0, 64.0, 29_999_999.0],
        max: [30_000_000.0, 66.0, 30_000_000.0],
    });
    assert!(
        !grid.build(&bodies, 0),
        "sparse worlds use the conservative native chunk path"
    );
    assert_eq!(grid.size, [0; 2]);
    assert!(
        grid.offsets.capacity() < 100,
        "do not allocate empty world space before fallback"
    );
}

#[test]
fn grounded_arrow_idle_ticks_do_not_touch_rows_or_appearance_and_pickup_still_works() {
    let (mut state, player, centre) = crate::projectiles::flight_tests::setup();
    let id = arrow(
        &mut state,
        player,
        [centre[0] + 4.0, centre[1] + 0.5, centre[2]],
        [0.0, -1.0, 0.0],
        0,
    );
    run(&mut state, &mut Vec::new(), [id].into_iter());
    let slot = state.entities.slot_of(id).unwrap();
    let saved = state
        .entities
        .row(slot)
        .projectile
        .as_deref()
        .unwrap()
        .clone();
    assert!(
        saved.stuck && saved.grounded_at_age.is_some(),
        "fixture must hit a real block"
    );
    state.entities.take_touched();
    state.appearance_changed.clear();
    let mut mutations = 0;
    for _ in 0..100 {
        state.entities.advance_age(slot);
        run(&mut state, &mut Vec::new(), [id].into_iter());
        mutations += state.entities.take_touched().len() + state.appearance_changed.len();
        state.appearance_changed.clear();
    }
    assert_eq!(
        mutations, 0,
        "idle clocks must not take row_mut or publish appearance"
    );
    let record = state.entities.row(slot).projectile.as_deref().unwrap();
    assert_eq!(
        record, &saved,
        "the persistent anchor stays immutable between transitions"
    );
    assert_eq!(record.ground_age(state.entities.row(slot).age), 100);
    // Ordinary arrows retain their normal lifetime; a nearby player still
    // receives exactly one real item through the existing inventory owner.
    let p = state.entities.slot_of(player).unwrap();
    state.entities.set_pos(p, state.entities.pos(slot));
    run(&mut state, &mut Vec::new(), [id].into_iter());
    assert!(state.entities.slot_of(id).is_none());
    assert_eq!(
        state.players[&player]
            .inventory
            .slots
            .iter()
            .map(|s| s.count.max(0))
            .sum::<i32>(),
        1
    );
}

/// A battle-scale volley: low arrows through a crowd, high arrows over it and
/// descending ones into it, a dropped item in a low lane and a live hook that
/// is not among the shots. The frozen snapshot keeps every body some shot's
/// query box reaches, leaves the crowd under the high arrows out, and the pass
/// matches the exhaustive oracle (which also offers the item).
#[test]
fn volley_snapshot_keeps_reached_bodies_only_and_matches_exhaustive_oracle() {
    let simulate = |indexed| {
        let (mut state, player) = scene();
        for x in (-60..=60).step_by(3) {
            for z in (-24..=36).step_by(4) {
                mob(
                    &mut state,
                    "minecraft:cow",
                    [f64::from(x) + 0.2, 201.0, f64::from(z) + 0.1],
                );
            }
        }
        let diamond = item(&state, "minecraft:diamond");
        crate::tick::spawn_item_entity(&mut state, [-30.8, 201.0, 0.1], diamond);
        let owner = state
            .entities
            .row(state.entities.slot_of(player).unwrap())
            .uuid;
        let rod = item(&state, "minecraft:fishing_rod");
        let hook = ProjectileState::new(ProjectileKind::FishingHook, Some(owner), rod);
        super::super::launch(&mut state, hook, [80.0, 230.0, 40.0], [0.0; 3]).unwrap();
        let mut ids = Vec::new();
        for i in 0..24 {
            let x = -62.0 + f64::from(i) * 5.0;
            let z = -24.0 + f64::from(i % 16) * 4.0 + 0.1;
            ids.push(arrow(&mut state, player, [x, 201.7, z], [2.5, 0.0, 0.0], 0));
            ids.push(arrow(&mut state, player, [x, 230.0, z], [3.0, 0.5, 0.0], 0));
        }
        for i in 0..16 {
            let from = [-50.0 + f64::from(i) * 7.0, 204.0, f64::from(i) * 3.0 - 20.0];
            ids.push(arrow(&mut state, player, from, [1.5, -2.5, 1.0], 0));
        }
        let names = identities(&state);
        let mut intents = Vec::new();
        if indexed {
            let mut every = Vec::new();
            every_body(&state, &mut every);
            let boxes: Vec<_> = ids
                .iter()
                .filter_map(|&id| shot_query(&state, id))
                .map(|query| query_box(query.start, query.end, query.radius))
                .collect();
            run(&mut state, &mut intents, ids.into_iter());
            let frozen: Vec<_> = state
                .projectiles
                .bodies
                .iter()
                .map(|body| body.id)
                .collect();
            for body in every.iter().filter(|body| !body.item) {
                let reached = boxes.iter().any(|(low, high)| {
                    (0..3).all(|axis| body.max[axis] >= low[axis] && body.min[axis] <= high[axis])
                });
                assert!(
                    !reached || frozen.contains(&body.id),
                    "{:?} meets a query box but left the snapshot",
                    body.id
                );
            }
            assert!(
                frozen.len() * 2 < every.len(),
                "the crowd outside the low lanes stays out: {} of {}",
                frozen.len(),
                every.len()
            );
        } else {
            exhaustive(&mut state, &mut intents, &ids);
        }
        assert!(!intents.is_empty(), "fixture must actually hit a body");
        digest(&state, &intents, &names)
    };
    assert_eq!(simulate(true), simulate(false));
}

/// The overlay-first world read `first_solid` used for every cell.
fn first_solid_by_world_reads(
    state: &TickState,
    from: [f64; 3],
    step: [f64; 3],
    radius: f64,
) -> Result<Option<(f64, crate::boundary::BlockPos)>, ()> {
    let end: [f64; 3] = std::array::from_fn(|axis| from[axis] + step[axis]);
    let min: [i32; 3] =
        std::array::from_fn(|axis| (from[axis].min(end[axis]) - radius).floor() as i32);
    let max: [i32; 3] =
        std::array::from_fn(|axis| (from[axis].max(end[axis]) + radius).floor() as i32);
    let below = i32::from(
        state
            .registry
            .max_collision_box_y
            .div_ceil(16)
            .saturating_sub(1),
    );
    let mut first: Option<(f64, crate::boundary::BlockPos)> = None;
    for y in min[1].saturating_sub(below)..=max[1] {
        for z in min[2]..=max[2] {
            for x in min[0]..=max[0] {
                let block = state.world.block_at(x, y, z).ok_or(())?;
                let boxes = state
                    .registry
                    .collision_boxes
                    .get(block as usize)
                    .ok_or(())?;
                for collision_box in boxes {
                    let local = collision_box.as_blocks();
                    let offset = [f64::from(x), f64::from(y), f64::from(z)];
                    let lower = std::array::from_fn(|axis| offset[axis] + local[axis]);
                    let upper = std::array::from_fn(|axis| offset[axis] + local[axis + 3]);
                    if let Some(t) = swept_sphere_enters(from, step, lower, upper, radius)
                        && first.is_none_or(|(old, _)| t < old)
                    {
                        first = Some((t, crate::boundary::BlockPos { x, y, z }));
                    }
                }
            }
        }
    }
    Ok(first)
}

/// Direct column reads give the same hits, ties and refusals (unloaded
/// columns, heights outside the world) as world reads, both while block
/// writes are pending in the overlay and after they fold into the columns.
#[test]
fn first_solid_column_reads_match_world_reads_before_and_after_folding() {
    let (mut state, _) = scene();
    let shape = |state: &TickState, keep: fn(&[f64; 6]) -> bool| {
        state
            .registry
            .collision_boxes
            .iter()
            .position(|boxes| {
                !boxes.is_empty() && boxes.iter().all(|shape| keep(&shape.as_blocks()))
            })
            .unwrap() as u32
    };
    let tall = shape(&state, |shape| shape[4] > 1.0);
    let low = shape(&state, |shape| shape[4] <= 0.5 && shape[3] == 1.0);
    let stone = state.registry.states.stone;
    let air = state.registry.states.air;
    // A mixed cluster across the x = -16 and z = 0 column seams.
    for x in -18_i32..=-14 {
        for y in 200..=202 {
            for z in -2..=1 {
                let block = [stone, tall, low, air][(x + y + z).rem_euclid(4) as usize];
                state.world.set_block(x, y, z, block);
            }
        }
    }
    let mut segments = vec![
        // Into an unloaded column, and across the world's top and bottom.
        ([1000.0, 201.0, 0.0], [4.0, 0.0, 0.0], 0.3),
        ([0.5, 318.0, 0.5], [0.0, 4.0, 0.0], 0.0),
        ([0.5, -62.0, 0.5], [0.0, -4.0, 0.0], 0.3),
    ];
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = |range: f64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 * range - range
    };
    for _ in 0..400 {
        let from = [-16.0 + next(3.0), 201.5 + next(2.0), next(3.0)];
        let step = [next(4.0), next(3.0), next(4.0)];
        let radius = [0.0, 0.3, 0.5][segments.len() % 3];
        segments.push((from, step, radius));
    }
    let compare = |state: &TickState| {
        let mut hits = 0;
        for &(from, step, radius) in &segments {
            let expected = first_solid_by_world_reads(state, from, step, radius);
            hits += usize::from(matches!(expected, Ok(Some(_))));
            assert_eq!(
                first_solid(state, from, step, radius),
                expected,
                "from={from:?} step={step:?} radius={radius}"
            );
            assert_start_cell(state, from, step, radius);
        }
        for refused in &segments[..3] {
            assert_eq!(first_solid(state, refused.0, refused.1, refused.2), Err(()));
        }
        hits
    };
    assert!(!state.world.overlay_is_empty());
    let pending = compare(&state);
    state.world.fold_overlay();
    assert!(state.world.overlay_is_empty());
    assert_eq!(compare(&state), pending);
    assert!(pending > 20, "fixture must hit placed shapes: {pending}");
}

/// An accepted overlay-free sweep reports the start cell's world state;
/// with writes pending in the overlay it reports none and the caller reads.
fn assert_start_cell(state: &TickState, from: [f64; 3], step: [f64; 3], radius: f64) {
    let Ok(swept) = sweep(state, from, step, radius) else {
        return;
    };
    let cell = from.map(|value| value.floor() as i32);
    let expected = state
        .world
        .overlay_is_empty()
        .then(|| state.world.block_at(cell[0], cell[1], cell[2]).unwrap());
    assert_eq!(swept.start, expected, "from={from:?} step={step:?}");
}

/// Uniform air, uniform water (no collision shape), uniform stone and a mixed
/// section stacked in every column: a sweep reads cells one by one only in
/// sections that can hold a collision shape, yet its hits, ties, refusals and
/// start cell still match world reads.
#[test]
fn sweep_reads_cells_only_in_sections_that_can_hold_collision_shapes() {
    struct Layers(mc_world::Chunk);
    impl mc_world::ChunkGenerator for Layers {
        fn generate(&self, pos: mc_world::ChunkPos) -> mc_world::Chunk {
            let mut chunk = self.0.clone();
            chunk.pos = pos;
            chunk
        }
    }
    let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let air = mc_world::BlockStateId(registry.states.air);
    let stone = registry.states.stone;
    let water = registry.states.water;
    assert!(registry.collision_boxes[water as usize].is_empty());
    assert!(!registry.collision_boxes[stone as usize].is_empty());
    let mut template = mc_world::Chunk::empty(
        mc_world::ChunkPos { x: 0, z: 0 },
        air,
        Identifier::parse("minecraft:plains").unwrap(),
    );
    // Sections from y = -64: stone 0..=15, water 16..=31, mixed 32..=47.
    template.sections[4] = mc_world::ChunkSection::filled(mc_world::BlockStateId(stone), air);
    template.sections[5] = mc_world::ChunkSection::filled(mc_world::BlockStateId(water), air);
    template.set_block(3, 40, 3, mc_world::BlockStateId(stone));
    template.status = "minecraft:full".into();
    let world = crate::world::ChunkWorld::new(
        42,
        Arc::new(Layers(template)),
        &registry,
        air,
        [8, 64, 8],
        mc_world::OVERWORLD_GEOMETRY,
    );
    let mut state = TickState::new(registry, world, 2, 512);
    let chunks: Vec<_> = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| crate::boundary::ChunkPos { x, z }))
        .collect();
    state
        .world
        .ensure_blocking(&chunks.iter().map(|&chunk| (chunk, 0)).collect());
    assert!(state.world.overlay_is_empty());
    let below = i32::from(
        state
            .registry
            .max_collision_box_y
            .div_ceil(16)
            .saturating_sub(1),
    );
    let rows = (below + 1) as usize;
    let check = |from: [f64; 3], step: [f64; 3], radius: f64| {
        take_swept_cells();
        let hit = sweep(&state, from, step, radius).map(|swept| swept.hit);
        let read = take_swept_cells();
        assert_eq!(
            hit,
            first_solid_by_world_reads(&state, from, step, radius),
            "from={from:?} step={step:?} radius={radius}"
        );
        assert_start_cell(&state, from, step, radius);
        (hit, read)
    };
    // Inside uniform air and uniform water: nothing is read cell by cell.
    assert_eq!(
        check([4.5, 100.5, 4.5], [2.0, 1.0, -2.0], 0.3),
        (Ok(None), 0)
    );
    assert_eq!(check([4.5, 24.5, 4.5], [1.0, 1.0, 1.0], 0.3), (Ok(None), 0));
    // From water down into uniform stone, which holds shapes: only the stone
    // rows are read, and the hit is the stone's top face.
    let (hit, read) = check([4.5, 17.5, 4.5], [0.0, -3.0, 0.0], 0.0);
    assert_eq!(
        hit.unwrap().unwrap().1,
        crate::boundary::BlockPos { x: 4, y: 15, z: 4 }
    );
    assert_eq!(read, rows + 1);
    // Down from the mixed section into water: only the mixed rows are read.
    assert_eq!(
        check([4.5, 33.0, 4.5], [0.0, -2.0, 0.0], 0.0),
        (Ok(None), 2)
    );
    // The mixed section across a column seam, then onto its one stone.
    assert_eq!(
        check([14.5, 40.5, 3.5], [3.0, 0.0, 0.0], 0.0),
        (Ok(None), 4 * rows)
    );
    let (hit, read) = check([1.5, 40.5, 3.5], [3.0, 0.0, 0.0], 0.0);
    assert_eq!(
        hit.unwrap().unwrap().1,
        crate::boundary::BlockPos { x: 3, y: 40, z: 3 }
    );
    assert_eq!(read, 4 * rows);
    // Skipped air still refuses an unloaded column and the world's ends.
    assert_eq!(check([1000.5, 100.5, 4.5], [1.0, 0.0, 0.0], 0.3).0, Err(()));
    assert_eq!(check([4.5, 318.0, 4.5], [0.0, 4.0, 0.0], 0.0).0, Err(()));
    assert_eq!(check([4.5, -62.0, 4.5], [0.0, -4.0, 0.0], 0.3).0, Err(()));
    // Random sweeps through every layer and across the column seams.
    let mut seed = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = |range: f64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 * range - range
    };
    let mut hits = 0;
    for index in 0..600 {
        let from = [next(14.0), 24.0 + next(28.0), next(14.0)];
        let step = [next(4.0), next(4.0), next(4.0)];
        hits += usize::from(matches!(
            check(from, step, [0.0, 0.3, 0.5][index % 3]).0,
            Ok(Some(_))
        ));
    }
    assert!(hits > 100, "fixture must hit stone: {hits}");
}

/// An arrow's water drag reads its start cell: from the sweep while no write
/// is pending, from the overlay-first world read while one is.
#[test]
fn arrow_drag_reads_start_cell_from_sweep_or_pending_write() {
    for (water, pending) in [(false, false), (true, false), (true, true), (false, true)] {
        let (mut state, player) = scene();
        state.world.fold_overlay();
        // Water in the start cell, or a stone write away from the flight.
        let (x, block) = if water {
            (-41, state.registry.states.water)
        } else {
            (-60, state.registry.states.stone)
        };
        state.world.set_block(x, 230, 20, block);
        if !pending {
            state.world.fold_overlay();
        }
        assert_eq!(state.world.overlay_is_empty(), !pending);
        let id = arrow(&mut state, player, [-40.5, 230.5, 20.5], [0.5, 0.0, 0.0], 0);
        exhaustive(&mut state, &mut Vec::new(), &[id]);
        let velocity = state
            .entities
            .vel(state.entities.slot_of(id).unwrap())
            .map(|value| value / 20.0);
        let drag = if water { 0.6 } else { 0.99 };
        assert!(
            (velocity[0] - 0.5 * drag).abs() < 1e-9,
            "water={water} pending={pending} velocity={velocity:?}"
        );
    }
}
