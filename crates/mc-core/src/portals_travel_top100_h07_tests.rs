//! H07 attribution and exhaustive geometry oracles. These are not capacity gates.
use super::*;
use crate::contact_hazards;
use crate::intent::{Intent, IntentKind};
use crate::survival::DamageKind;
use std::sync::{Arc, LazyLock};

static REGISTRY: LazyLock<Arc<crate::registry::CoreRegistry>> =
    LazyLock::new(|| Arc::new(crate::registry::CoreRegistry::load(None).unwrap()));

struct Terrain {
    geometry: mc_world::ChunkGeometry,
    fill: Option<([i32; 3], u32)>,
}

impl mc_world::ChunkGenerator for Terrain {
    fn generate(&self, pos: mc_world::ChunkPos) -> mc_world::Chunk {
        let mut chunk = mc_world::Chunk::empty_with_geometry(
            pos,
            mc_world::BlockStateId(REGISTRY.states.air),
            mc_data::Identifier::parse("minecraft:plains").unwrap(),
            self.geometry,
        );
        if let Some(([x, y, z], block)) = self.fill
            && pos.x == x.div_euclid(16)
            && pos.z == z.div_euclid(16)
        {
            chunk
                .set_block(
                    x.rem_euclid(16) as u8,
                    y,
                    z.rem_euclid(16) as u8,
                    mc_world::BlockStateId(block),
                )
                .expect("fixture cell inside dimension");
        }
        chunk
    }
}

fn local(dimension: Dimension, fill: Option<([i32; 3], u32)>) -> TickState {
    let registry = Arc::clone(&REGISTRY);
    let world = crate::world::ChunkWorld::new(
        42,
        Arc::new(Terrain {
            geometry: dimension.geometry(),
            fill,
        }),
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [-1, 200, -1],
        dimension.geometry(),
    );
    let mut state = TickState::new(registry, world, 20, 4096);
    state.dimension = dimension;
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    state
}

fn block(name: &str) -> u32 {
    REGISTRY
        .blocks
        .block(&mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap())
        .unwrap()
        .default
        .0
}

fn load(state: &mut TickState, center: mc_world::ChunkPos, radius: i32) {
    let want = crate::world::chunks_in_radius(center, radius)
        .into_iter()
        .map(|pos| (pos, 0))
        .collect();
    state.world.ensure_blocking(&want);
    state
        .world
        .set_active_chunks(want.keys().copied().collect(), state.tick);
}

fn worlds(fill: Option<([i32; 3], u32)>) -> TickState {
    let mut state = local(Dimension::Overworld, fill);
    for dimension in [Dimension::Nether, Dimension::End] {
        state
            .other_dimensions
            .insert(dimension, Box::new(local(dimension, None)));
    }
    state
}

fn cow(state: &mut TickState, pos: [f64; 3]) -> EntityId {
    let id = state.spawn_mob(state.registry.cow_type_id, pos, 10.0);
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.row_mut(slot).uuid = uuid::Uuid::from_u128(id.0 as u128);
    id
}

fn contact(state: &mut TickState, exhaustive: bool) -> Vec<Intent> {
    let mut intents = Vec::new();
    if exhaustive {
        // Unlike the older all-serial reference, force fresh geometry on EVERY
        // pass: an invalidation defect must not be shared by both sides.
        state.contact_hazards_scratch.negative.fill(None);
        contact_hazards::run_all_serial(state, &mut intents);
    } else {
        contact_hazards::run_with(state, &mut intents, usize::MAX);
    }
    intents
}

fn portals(state: &mut TickState, exhaustive: bool) -> BTreeMap<Dimension, ChangeSet> {
    if exhaustive {
        state.portal_contact_stamps.fill(None);
        for world in state.other_dimensions.values_mut() {
            world.portal_contact_stamps.fill(None);
        }
    }
    let mut changes = BTreeMap::new();
    finish_with(state, &mut changes, (!exhaustive).then_some(usize::MAX));
    changes
}

fn hit(intents: &[Intent], kind: DamageKind) -> bool {
    intents.iter().any(|intent| matches!(intent.kind, IntentKind::DamageEntity { kind: actual, .. } if actual == kind))
}

fn contact_transcript(exhaustive: bool) -> Vec<String> {
    let mut state = local(Dimension::Overworld, None);
    load(&mut state, mc_world::ChunkPos { x: -1, z: -1 }, 0);
    let id = cow(&mut state, [-0.5, 200.0, -0.5]);
    let slot = state.entities.slot_of(id).unwrap();
    let mut transcript = Vec::new();
    for event in 0..10 {
        state.tick = 20 + event * 20;
        match event {
            2 => contact_hazards::ignite(&mut state, id, 41),
            3 => {
                state.world.set_block(-1, 200, -1, block("water"));
            }
            4 => {
                state
                    .world
                    .set_block(-1, 200, -1, state.registry.states.air);
                state.world.fold_overlay();
            }
            5 => {
                contact_hazards::ignite(&mut state, id, 41);
                state.weather = (crate::commands::Weather::Rain, 1000);
            }
            6 => {
                state
                    .world
                    .set_block(-1, 203, -1, state.registry.states.stone);
                state.world.fold_overlay();
                contact_hazards::ignite(&mut state, id, 41);
            }
            7 => {
                state.world.set_block(-1, 200, -1, block("cactus"));
            }
            8 => {
                state
                    .world
                    .set_block(-1, 200, -1, state.registry.states.air);
                state.world.fold_overlay();
            }
            9 => {
                state.world.set_block(-1, 200, -1, block("lava"));
                state.world.fold_overlay();
            }
            _ => {}
        }
        state.appearance_changed.clear();
        let intents = contact(&mut state, exhaustive);
        let body = state.entities.body(slot);
        match event {
            2 | 6 => {
                assert!(hit(&intents, DamageKind::OnFire));
                assert_eq!(body.burn_ticks, 40);
            }
            3 | 5 => assert_eq!(body.burn_ticks, 0),
            7 => assert!(hit(&intents, DamageKind::Cactus)),
            9 => assert!(hit(&intents, DamageKind::Lava)),
            _ => {}
        }
        transcript.push(format!(
            "{intents:?} {} {} {:?}",
            body.burn_ticks,
            state.entities.row(slot).appearance.shared_flags,
            state.appearance_changed
        ));
    }
    transcript
}

#[test]
fn h07_contact_exhaustive_oracle_edit_ignition_water_rain_negative_coordinates() {
    assert_eq!(contact_transcript(false), contact_transcript(true));
}

#[test]
fn h07_quiet_cost_fence_counts_cauldron_tail_separately() {
    let mut state = local(Dimension::Overworld, None);
    load(&mut state, mc_world::ChunkPos { x: -1, z: -1 }, 0);
    for _ in 0..128 {
        cow(&mut state, [-0.5, 200.0, -0.5]);
    }
    contact(&mut state, false);
    portals(&mut state, false);
    contact_hazards::take_work_counts();
    take_work_counts();
    assert!(contact(&mut state, false).is_empty());
    assert!(portals(&mut state, false).is_empty());
    let contact = contact_hazards::take_work_counts();
    let portal = take_work_counts();
    assert_eq!((contact, portal), ([128, 0, 128, 0, 128], [128, 0, 0]));
    println!("H07 quiet contact={contact:?} portals={portal:?}; cauldron tail is NOT zero-read");
}

#[test]
fn h07_eviction_readmission_and_missing_column_load_invalidate_proofs() {
    let pos = mc_world::ChunkPos { x: -1, z: -1 };
    let mut state = worlds(Some(([-1, 200, -1], block("nether_portal"))));
    let id = cow(&mut state, [-0.5, 200.0, -0.5]);
    let slot = state.entities.slot_of(id).unwrap();
    portals(&mut state, false);
    assert!(state.portal_contact_stamps[slot.0 as usize].is_some());
    load(&mut state, pos, 0);
    portals(&mut state, false);
    assert_eq!(state.entities.row(slot).entity_life.portal.elapsed, 1);
    // A separate quiet resident proves eviction cannot silently preserve a
    // resident revision; neither a load nor an eviction is a block overlay.
    let mut quiet = local(Dimension::Overworld, None);
    load(&mut quiet, pos, 0);
    let id = cow(&mut quiet, [-0.5, 200.0, -0.5]);
    contact(&mut quiet, false);
    portals(&mut quiet, false);
    let slot = quiet.entities.slot_of(id).unwrap();
    assert!(quiet.contact_hazards_scratch.negative[slot.0 as usize].is_some());
    quiet
        .world
        .set_active_chunks(Default::default(), quiet.tick);
    // In-memory world: project metadata and simulate the storage ACK, as in
    // tick_tests' eviction fixture. There is no native writer in this world.
    quiet.world.capture_dirty(&Default::default());
    let written = [(pos, quiet.world.storage_revision(pos))];
    quiet.world.mark_flushed(&written);
    assert_eq!(quiet.world.evict_outside(&Default::default()), vec![pos]);
    contact(&mut quiet, false);
    assert!(quiet.contact_hazards_scratch.negative[slot.0 as usize].is_none());
    let mut cache = crate::world::SectionMatchCache::default();
    cache.reset(&quiet.world);
    assert!(
        !quiet_proof(
            &quiet.entities,
            &quiet.world,
            &cache,
            slot,
            &mut quiet.portal_contact_stamps[slot.0 as usize]
        )
        .0
    );
    load(&mut quiet, pos, 0);
    contact_hazards::take_work_counts();
    contact(&mut quiet, false);
    assert_eq!(contact_hazards::take_work_counts()[1], 1);
    assert!(quiet.contact_hazards_scratch.negative[slot.0 as usize].is_some());
}

fn travel_transcript(exhaustive: bool) -> Vec<String> {
    let mut state = worlds(None);
    load(&mut state, mc_world::ChunkPos { x: -1, z: -1 }, 0);
    load(
        world_mut(&mut state, Dimension::End),
        mc_world::ChunkPos { x: 6, z: 0 },
        1,
    );
    let id = cow(&mut state, [-0.5, 200.0, -0.5]);
    let mut transcript = Vec::new();
    for event in 0..6 {
        if event == 2 {
            state.world.set_block(-1, 200, -1, block("end_portal"));
        }
        let changes = portals(&mut state, exhaustive);
        let (dimension, slot) = travel::locate(&state, id).unwrap();
        let world = travel::world(&state, dimension).unwrap();
        let timer = &world.entities.row(slot).entity_life.portal;
        if event >= 2 {
            assert_eq!(dimension, Dimension::End);
            assert_eq!(timer.cooldown, 302 - event);
        }
        let identities = std::iter::once(&state)
            .chain(state.other_dimensions.values().map(|world| &**world))
            .filter(|world| world.entities.slot_of_any(id).is_some())
            .count();
        assert_eq!(identities, 1);
        transcript.push(format!(
            "{dimension:?} {:?} {} {} {:?}",
            world.entities.pos(slot),
            timer.cooldown,
            timer.elapsed,
            changes
        ));
    }
    transcript
}

#[test]
fn h07_portal_edit_transfer_cooldown_exhaustive_oracle() {
    assert_eq!(travel_transcript(false), travel_transcript(true));
}

#[test]
fn h07_contact_loaded_hazard_and_negative_chunk_seam_match_exhaustive() {
    for exhaustive in [false, true] {
        let mut state = local(
            Dimension::Overworld,
            Some(([-17, 200, -1], block("cactus"))),
        );
        let id = cow(&mut state, [-16.02, 200.0, -0.5]);
        let slot = state.entities.slot_of(id).unwrap();
        contact_hazards::ignite(&mut state, id, 41);
        assert!(contact(&mut state, exhaustive).is_empty());
        assert_eq!(state.entities.body(slot).burn_ticks, 41, "inactive clock");
        load(&mut state, mc_world::ChunkPos { x: -2, z: -1 }, 1);
        assert!(hit(&contact(&mut state, exhaustive), DamageKind::Cactus));
        state
            .world
            .set_block(-17, 200, -1, state.registry.states.air);
        state.world.fold_overlay();
        assert!(!hit(&contact(&mut state, exhaustive), DamageKind::Cactus));
        state.world.set_block(-16, 200, -1, block("cactus"));
        assert!(hit(&contact(&mut state, exhaustive), DamageKind::Cactus));
    }
}

#[test]
fn h07_portal_cooldown_refreshes_in_contact_then_decays_after_exit() {
    for exhaustive in [false, true] {
        let mut state = worlds(Some(([-1, 200, -1], block("nether_portal"))));
        load(&mut state, mc_world::ChunkPos { x: -1, z: -1 }, 0);
        let id = cow(&mut state, [-0.5, 200.0, -0.5]);
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.row_mut(slot).entity_life.portal.cooldown = 7;
        portals(&mut state, exhaustive);
        assert_eq!(state.entities.row(slot).entity_life.portal.cooldown, 300);
        state.entities.set_pos(slot, [-2.5, 200.0, -0.5]);
        portals(&mut state, exhaustive);
        assert_eq!(state.entities.row(slot).entity_life.portal.cooldown, 299);
        portals(&mut state, exhaustive);
        assert_eq!(state.entities.row(slot).entity_life.portal.cooldown, 298);
        state.entities.set_pos(slot, [-0.5, 200.0, -0.5]);
        portals(&mut state, exhaustive);
        assert_eq!(state.entities.row(slot).entity_life.portal.cooldown, 300);
        assert_eq!(state.entities.row(slot).entity_life.portal.elapsed, 0);
    }
}

#[test]
fn h07_frozen_tick_keeps_contact_burn_and_preserves_portal_baseline() {
    let mut state = worlds(None);
    load(&mut state, mc_world::ChunkPos { x: -1, z: -1 }, 0);
    let id = cow(&mut state, [-0.5, 200.0, -0.5]);
    contact(&mut state, false);
    portals(&mut state, false);
    contact_hazards::ignite(&mut state, id, 41);
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.row_mut(slot).entity_life.portal.cooldown = 7;
    state.tick_control.set_frozen(true);
    crate::tick::run_tick(&mut state, &[]);
    let slot = state.entities.slot_of(id).unwrap();
    assert_eq!(state.entities.body(slot).burn_ticks, 41);
    // Existing portal finish runs outside the simulation gate. This card is
    // behavior-neutral: expose, rather than silently change, that baseline.
    assert_eq!(state.entities.row(slot).entity_life.portal.cooldown, 6);
    println!("H07 frozen baseline burn=41 portal cooldown=6 (from 7)");
}

#[test]
#[ignore = "H07 focused deterministic work counts, not a capacity gate"]
fn h07_focused_work_counts() {
    for scene in ["quiet", "moving", "hazard", "portal"] {
        let mut state = worlds(None);
        load(&mut state, mc_world::ChunkPos { x: -1, z: -1 }, 0);
        let ids: Vec<_> = (0..4096)
            .map(|_| cow(&mut state, [-0.5, 200.0, -0.5]))
            .collect();
        if scene == "hazard" {
            state.world.set_block(-1, 200, -1, block("cactus"));
        }
        if scene == "portal" {
            state.world.set_block(-1, 200, -1, block("nether_portal"));
        }
        state.world.fold_overlay();
        contact(&mut state, false);
        portals(&mut state, false);
        contact_hazards::take_work_counts();
        take_work_counts();
        if scene == "moving" {
            for id in &ids {
                let slot = state.entities.slot_of(*id).unwrap();
                state.entities.set_pos(slot, [-2.5, 200.0, -0.5]);
            }
        }
        let intents = contact(&mut state, false);
        let changes = portals(&mut state, false);
        let contact_counts = contact_hazards::take_work_counts();
        let portal_counts = take_work_counts();
        println!(
            "H07 scene={scene} resident=4096 active=4096 contact={contact_counts:?} portals={portal_counts:?} intents={} changed_dimensions={}",
            intents.len(),
            changes.len()
        );
        if scene == "quiet" {
            assert_eq!(contact_counts, [4096, 0, 4096, 0, 4096]);
            assert_eq!(portal_counts, [4096, 0, 0]);
        }
        if scene == "hazard" {
            assert_eq!(intents.len(), 4096);
            assert!(contact_counts[3] >= 4096);
        }
        if scene == "portal" {
            assert!(portal_counts[2] >= 4096);
        }
    }
}
