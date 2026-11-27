//! Autonomous village adoption regressions: once-only identity, restart
//! persistence through native settlement rows, villager binding, player-bell
//! refusal, and chunk-load order independence.
//!
//! The shared fixture mirrors the runtime contract Builders' E2E relies on: a
//! bell block, generated-villager markers with real provenance claims, loaded
//! chunks listed in `changes.chunks_loaded`, generated villagers spawned via
//! `generated::load` BEFORE `autonomy::tick`.
use super::*;

/// The generated-piece certification tests live beside these: this file is only
/// compiled under `cfg(test)`, so a reversal of the production hunks in
/// `autonomy.rs`/`buildings.rs` cannot take the test module with it.
#[path = "generated_certification_tests.rs"]
mod generated_certification_tests;
#[path = "natural_mine_queue_tests.rs"]
mod natural_mine_queue_tests;
use crate::boundary::{ChangeSet, ChunkPos};
use crate::registry::CoreRegistry;
use crate::settlements::buildings::BuildingKind;
use crate::tick::TickState;
use std::sync::Arc;

const BELL: [i32; 3] = [8, 201, 8];

/// Extra blocks written inside a chunk: (position, block, properties).
type Furniture = Vec<([i32; 3], &'static str, Vec<(&'static str, &'static str)>)>;

struct VillageGenerator {
    terrain: mc_worldgen::TerrainGenerator,
    registry: Arc<CoreRegistry>,
    bells: Vec<[i32; 3]>,
    villagers: Vec<(String, [f64; 3], String)>,
    logs: Vec<[i32; 2]>,
    furniture: Furniture,
}

impl VillageGenerator {
    fn block(&self, name: &str, props: &[(&str, &str)]) -> mc_world::BlockStateId {
        let name = mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap();
        let default = self.registry.blocks.block(&name).unwrap().default;
        let mut values = self
            .registry
            .blocks
            .by_id(default)
            .unwrap()
            .properties
            .clone();
        for (key, value) in props {
            if let Some((_, current)) = values.iter_mut().find(|(property, _)| property == key) {
                *current = (*value).into();
            }
        }
        self.registry
            .blocks
            .by_name_and_props(&name, &values)
            .unwrap()
    }
}

impl mc_world::ChunkGenerator for VillageGenerator {
    fn generate(&self, position: mc_world::ChunkPos) -> mc_world::Chunk {
        let mut chunk = mc_world::ChunkGenerator::generate(&self.terrain, position);
        let air = mc_world::BlockStateId(self.registry.states.air);
        let stone = mc_world::BlockStateId(self.registry.states.stone);
        let soil = self.block("grass_block", &[]);
        let log = self.block("oak_log", &[]);
        let leaves = self.block("oak_leaves", &[("persistent", "false")]);
        for x in 0..16_u8 {
            for z in 0..16_u8 {
                let wx = position.x * 16 + i32::from(x);
                let wz = position.z * 16 + i32::from(z);
                for y in 190..201 {
                    chunk.set_block(x, y, z, stone);
                }
                chunk.set_block(x, 201, z, soil);
                for y in 202..225 {
                    chunk.set_block(x, y, z, air);
                }
                if self.logs.iter().any(|[lx, lz]| *lx == wx && *lz == wz) {
                    for y in 202..206 {
                        chunk.set_block(x, y, z, log);
                    }
                    chunk.set_block(x, 206, z, leaves);
                }
            }
        }
        for bell in &self.bells {
            if bell[0].div_euclid(16) == position.x && bell[2].div_euclid(16) == position.z {
                let bell_id = self.block("bell", &[]);
                chunk.set_block(
                    bell[0].rem_euclid(16) as u8,
                    bell[1],
                    bell[2].rem_euclid(16) as u8,
                    bell_id,
                );
            }
        }
        for (at, name, props) in &self.furniture {
            if at[0].div_euclid(16) == position.x && at[2].div_euclid(16) == position.z {
                let id = self.block(name, props);
                chunk.set_block(
                    at[0].rem_euclid(16) as u8,
                    at[1],
                    at[2].rem_euclid(16) as u8,
                    id,
                );
            }
        }
        let markers: Vec<mc_world::GeneratedEntityMarker> = self
            .villagers
            .iter()
            .enumerate()
            .filter(|(_, (_, pos, _))| {
                (pos[0].floor() as i32).div_euclid(16) == position.x
                    && (pos[2].floor() as i32).div_euclid(16) == position.z
            })
            .map(
                |(index, (claim, pos, profession))| mc_world::GeneratedEntityMarker {
                    claim: claim.clone().replace("#I", &format!("#{index}")),
                    entity_type: "minecraft:villager".into(),
                    position: *pos,
                    persistent: false,
                    villager_kind: "minecraft:plains".into(),
                    profession: format!("minecraft:{profession}"),
                    level: 1,
                    age: 0,
                    yaw: 0.0,
                    pitch: 0.0,
                },
            )
            .collect();
        if !markers.is_empty() {
            chunk.set_generated_entities(&markers);
        }
        chunk
    }

    fn surface_height(&self, _x: i32, _z: i32) -> Option<i32> {
        Some(201)
    }

    fn village_sites_in_region(
        &self,
        min: (i32, i32),
        max: (i32, i32),
    ) -> Vec<mc_world::GeneratedVillageSite> {
        if self.villagers.is_empty() || min.0 > 0 || min.1 > 0 || max.0 < 0 || max.1 < 0 {
            return Vec::new();
        }
        let mut pieces = vec![mc_world::GeneratedVillagePiece {
            template: Some("minecraft:village/plains/town_centers/plains_meeting_point_1".into()),
            position: mc_world::BlockPos { x: 7, y: 201, z: 7 },
            rotation: 0,
            entity_owner: Some("minecraft:village/plains/town_centers".into()),
            bounds: Some((
                mc_world::BlockPos { x: 7, y: 201, z: 7 },
                mc_world::BlockPos { x: 9, y: 204, z: 9 },
            )),
        }];
        let mut seen = BTreeSet::new();
        for (claim, _, _) in &self.villagers {
            let (template, tail) = claim.split_once('@').unwrap();
            let (origin, _) = tail.split_once('#').unwrap();
            if !seen.insert((template, origin)) {
                continue;
            }
            let mut coordinates = origin.split(':').map(|value| value.parse::<i32>().unwrap());
            let at = mc_world::BlockPos {
                x: coordinates.next().unwrap(),
                y: coordinates.next().unwrap(),
                z: coordinates.next().unwrap(),
            };
            pieces.push(mc_world::GeneratedVillagePiece {
                template: Some(if template.ends_with("/houses") {
                    format!("{template}/plains_house_1")
                } else {
                    template.into()
                }),
                position: at,
                rotation: 0,
                entity_owner: Some(template.into()),
                bounds: Some((
                    at,
                    mc_world::BlockPos {
                        x: at.x + 15,
                        y: at.y + 4,
                        z: at.z + 15,
                    },
                )),
            });
        }
        let max = pieces.iter().filter_map(|piece| piece.bounds).fold(
            mc_world::BlockPos {
                x: 48,
                y: 224,
                z: 48,
            },
            |known, (_, next)| mc_world::BlockPos {
                x: known.x.max(next.x),
                y: known.y.max(next.y),
                z: known.z.max(next.z),
            },
        );
        vec![mc_world::GeneratedVillageSite {
            start_chunk: (0, 0),
            structure: mc_data::Identifier::parse("minecraft:village_plains").expect("static id"),
            min: mc_world::BlockPos {
                x: -32,
                y: 188,
                z: -32,
            },
            max,
            pieces,
        }]
    }
}

fn generated_claim(owner: &str, origin: [i32; 3]) -> String {
    format!("{owner}@{}:{}:{}##I", origin[0], origin[1], origin[2])
}

fn fixture(
    bells: Vec<[i32; 3]>,
    villagers: Vec<(String, [f64; 3], String)>,
    logs: Vec<[i32; 2]>,
) -> (TickState, Vec<ChunkPos>) {
    fixture_furnished(bells, villagers, logs, Vec::new())
}

fn fixture_furnished(
    bells: Vec<[i32; 3]>,
    villagers: Vec<(String, [f64; 3], String)>,
    logs: Vec<[i32; 2]>,
    furniture: Furniture,
) -> (TickState, Vec<ChunkPos>) {
    let registry = Arc::new(CoreRegistry::load(None).unwrap());
    let generator = Arc::new(VillageGenerator {
        terrain: mc_worldgen::TerrainGenerator::new(42, Arc::clone(&registry.blocks)),
        registry: Arc::clone(&registry),
        bells,
        villagers,
        logs,
        furniture,
    });
    let mut world = crate::world::ChunkWorld::new(
        42,
        generator,
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [8, 201, 8],
        mc_world::OVERWORLD_GEOMETRY,
    );
    let chunks: Vec<ChunkPos> = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| ChunkPos { x, z }))
        .collect();
    world.ensure_blocking(&chunks.iter().map(|chunk| (*chunk, 0)).collect());
    let mut state = TickState::new(registry, world, 2, 512);
    state
        .tickets
        .insert("autonomy-test".into(), (ChunkPos { x: 0, z: 0 }, 4));
    state.refresh_active_chunks(0);
    (state, chunks)
}

/// Adopt one bell: spawn generated villagers, list loaded chunks, tick.
fn adopt(state: &mut TickState, chunks: &[ChunkPos]) -> ChangeSet {
    let mut changes = ChangeSet::default();
    crate::villager::generated::load(state, chunks, &mut changes);
    changes.chunks_loaded.extend_from_slice(chunks);
    tick(state, &mut changes);
    changes
}

fn settle_natural_proofs(state: &mut TickState, kinds: &[BuildingKind]) {
    let id = state.settlements.records().next().expect("adopted").id;
    for _ in 0..512 {
        if kinds.iter().all(|kind| has_kind(state, id, *kind)) {
            return;
        }
        state.tick += 20;
        state.tick_budget.reset(state.tick);
        tick(state, &mut ChangeSet::default());
    }
    panic!(
        "reachable natural zones did not certify: {:?}",
        state.settlements.get(id)
    );
}

/// A roofed hut inside a claim piece's bounds, the shape a plains house
/// template has: solid floor, walls, roof, a door and one bed pair.
fn hut_blocks() -> Furniture {
    let (x0, x1, y0, y1, z0, z1) = (0, 15, 201, 205, 0, 15);
    let mut out = Vec::new();
    for x in x0..=x1 {
        for z in z0..=z1 {
            out.push(([x, y1, z], "stone", Vec::new()));
            if x == x0 || x == x1 || z == z0 || z == z1 {
                for y in y0 + 1..y1 {
                    out.push(([x, y, z], "stone", Vec::new()));
                }
            }
        }
    }
    out.push(([x0, y0 + 1, 7], "oak_door", vec![("half", "lower")]));
    out.push(([x0, y0 + 2, 7], "oak_door", vec![("half", "upper")]));
    // A bed pair as placement writes it: the foot sits behind the head's facing.
    out.push((
        [7, y0 + 1, 6],
        "red_bed",
        vec![("part", "head"), ("facing", "east")],
    ));
    out.push((
        [6, y0 + 1, 6],
        "red_bed",
        vec![("part", "foot"), ("facing", "east")],
    ));
    out
}

/// Seed-24 gate 20261124T212153: the adopted village owns beds but reports
/// `beds=0` because a generated piece that holds beds *and* its own workstation
/// classified as the workshop only, and its beds were lost. A plains library is
/// exactly that shape (lectern beside beds).
#[test]
fn an_adopted_house_piece_that_holds_a_workstation_certifies_both() {
    let claim = generated_claim("minecraft:village/plains/houses", [0, 201, 0]);
    let mut furniture = hut_blocks();
    furniture.push(([3, 202, 3], "smoker", Vec::new()));
    let (mut state, chunks) = fixture_furnished(
        vec![BELL],
        vec![(claim, [2.5, 202.0, 2.5], "butcher".into())],
        vec![],
        furniture,
    );
    adopt(&mut state, &chunks);
    let id = state.settlements.records().next().expect("adopted").id;
    let butcher = BuildingKind::Workshop(crate::villager::VillagerProfession::Butcher);
    for _ in 0..512 {
        if has_kind(&state, id, BuildingKind::House) && has_kind(&state, id, butcher) {
            break;
        }
        state.tick += 20;
        state.tick_budget.reset(state.tick);
        tick(&mut state, &mut ChangeSet::default());
    }
    if !has_kind(&state, id, BuildingKind::House)
        && let Err(err) = super::super::buildings::certify(
            &mut state,
            id,
            BuildingKind::House,
            ScriptWorldBounds::new([0, 201, 0], [15, 205, 15]).unwrap(),
        )
    {
        panic!(
            "the piece's house does not certify: {err:?}; nothing else certified: {:?}",
            state.settlements.get(id).unwrap().buildings
        );
    }
    assert!(
        has_kind(&state, id, BuildingKind::House),
        "the piece's beds certify a House: {:?}",
        state.settlements.get(id).unwrap().buildings
    );
    assert!(
        has_kind(&state, id, butcher),
        "the same piece keeps its workstation: {:?}",
        state.settlements.get(id).unwrap().buildings
    );
}
/// The pair is the generated caller's alone: on a house-only piece an owner, an
/// operator or construction certifying a workshop on the house's bounds is refused,
/// and even the generated caller only adds the House+Workshop pair.
#[test]
fn the_pair_rule_is_only_for_the_generated_caller() {
    let claim = generated_claim("minecraft:village/plains/houses", [0, 201, 0]);
    let (mut state, chunks) = fixture_furnished(
        vec![BELL],
        vec![(claim, [2.5, 202.0, 2.5], "none".into())],
        vec![],
        hut_blocks(),
    );
    adopt(&mut state, &chunks);
    let id = state.settlements.records().next().expect("adopted").id;
    let house = state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .values()
        .find(|building| building.kind == BuildingKind::House)
        .expect("the piece's House")
        .bounds;
    let mason = BuildingKind::Workshop(crate::villager::VillagerProfession::Mason);
    let error = super::super::buildings::certify(&mut state, id, mason, house)
        .expect_err("a strict caller may not add a workshop on a house's bounds");
    assert!(
        format!("{error:?}").contains("overlaps another certificate"),
        "{error:?}"
    );
    assert!(
        super::super::buildings::certify_generated(&mut state, id, mason, house).is_err(),
        "the generated caller still refuses a workshop with no workstation in the piece"
    );
}

/// On a piece that holds both, the generated caller re-certifies the existing
/// pair, and still refuses a third kind or a workshop off the piece's bounds.
#[test]
fn a_generated_pair_is_reused_and_nothing_else_shares_its_bounds() {
    let claim = generated_claim("minecraft:village/plains/houses", [0, 201, 0]);
    let mut furniture = hut_blocks();
    furniture.push(([10, 202, 10], "stonecutter", Vec::new()));
    let (mut state, chunks) = fixture_furnished(
        vec![BELL],
        vec![(claim, [2.5, 202.0, 2.5], "none".into())],
        vec![],
        furniture,
    );
    adopt(&mut state, &chunks);
    let id = state.settlements.records().next().expect("adopted").id;
    let buildings = state.settlements.get(id).unwrap().buildings.clone();
    let house = buildings
        .values()
        .find(|building| building.kind == BuildingKind::House)
        .expect("the piece's House")
        .bounds;
    let mason = BuildingKind::Workshop(crate::villager::VillagerProfession::Mason);
    let workshop = buildings
        .values()
        .find(|building| building.kind == mason)
        .expect("the piece's workshop");
    assert_eq!(
        super::super::buildings::certify_generated(&mut state, id, mason, house)
            .expect("the same workshop"),
        workshop.id,
        "a re-certification returns the existing record instead of a third one"
    );
    let armorer = BuildingKind::Workshop(crate::villager::VillagerProfession::Armorer);
    assert!(
        super::super::buildings::certify_generated(&mut state, id, armorer, house).is_err(),
        "a third certificate may not share the pair's bounds"
    );
    assert!(
        super::super::buildings::certify_generated(&mut state, id, BuildingKind::Storage, house)
            .is_err(),
        "a Storage may not share the pair's bounds"
    );
    let shifted = ScriptWorldBounds::new(
        [house.min[0] + 4, house.min[1], house.min[2] + 4],
        [house.max[0] + 4, house.max[1], house.max[2] + 4],
    )
    .unwrap();
    let error = super::super::buildings::certify_generated(&mut state, id, armorer, shifted)
        .expect_err("a workshop off the piece's bounds may not overlap it");
    assert!(
        format!("{error:?}").contains("overlaps another certificate"),
        "{error:?}"
    );
}

/// A piece that holds beds and no job site is a House, and one that holds only a
/// job site is still a workshop.
#[test]
fn piece_kinds_separates_the_house_the_workstation_and_neither() {
    let claim = generated_claim("minecraft:village/plains/houses", [0, 201, 0]);
    let mut furniture = hut_blocks();
    furniture.push(([3, 202, 3], "smoker", Vec::new()));
    let (state, _chunks) = fixture_furnished(
        vec![BELL],
        vec![(claim, [2.5, 202.0, 2.5], "butcher".into())],
        vec![],
        furniture,
    );
    let piece = mc_world::GeneratedVillagePiece {
        template: Some("minecraft:village/plains/houses/plains_house_1".into()),
        position: mc_world::BlockPos { x: 0, y: 201, z: 0 },
        rotation: 0,
        entity_owner: Some("minecraft:village/plains/houses".into()),
        bounds: Some((
            mc_world::BlockPos { x: 0, y: 201, z: 0 },
            mc_world::BlockPos {
                x: 15,
                y: 205,
                z: 15,
            },
        )),
    };
    let bounds = ScriptWorldBounds::new([0, 201, 0], [15, 205, 15]).unwrap();
    let kinds = super::piece_kinds(&piece, &state, &bounds).expect("readable piece");
    assert!(
        kinds.contains(&BuildingKind::House)
            && kinds.contains(&BuildingKind::Workshop(
                crate::villager::VillagerProfession::Butcher
            )),
        "a house that holds its own workstation certifies both: {kinds:?}"
    );
}

#[test]
fn a_street_whose_bounds_graze_a_bed_is_not_a_house_site() {
    let claim = generated_claim("minecraft:village/plains/streets", [0, 201, 0]);
    let (state, _chunks) = fixture_furnished(
        vec![BELL],
        vec![(claim, [2.5, 202.0, 2.5], "butcher".into())],
        vec![],
        hut_blocks(),
    );
    let piece = mc_world::GeneratedVillagePiece {
        template: Some("minecraft:village/plains/streets/straight_02".into()),
        position: mc_world::BlockPos { x: 0, y: 201, z: 0 },
        rotation: 0,
        entity_owner: Some("minecraft:village/plains/streets".into()),
        bounds: Some((
            mc_world::BlockPos { x: 0, y: 201, z: 0 },
            mc_world::BlockPos {
                x: 15,
                y: 205,
                z: 15,
            },
        )),
    };
    let bounds = ScriptWorldBounds::new([0, 201, 0], [15, 205, 15]).unwrap();
    let kinds = super::piece_kinds(&piece, &state, &bounds).expect("readable piece");
    assert!(
        kinds.is_empty(),
        "a street piece must certify no building, even with a bed in its bounds: {kinds:?}"
    );
}

/// An unreadable column is not a verdict: classification returns `None`, so the
/// caller keeps the piece queued instead of memoising "no building" forever.
#[test]
fn a_piece_over_an_unreadable_column_is_classified_later_not_never() {
    let (state, _chunks) = fixture(vec![BELL], vec![], vec![]);
    let piece = mc_world::GeneratedVillagePiece {
        template: Some("minecraft:village/plains/houses/plains_house_1".into()),
        position: mc_world::BlockPos {
            x: 900,
            y: 201,
            z: 900,
        },
        rotation: 0,
        entity_owner: Some("minecraft:village/plains/houses".into()),
        bounds: Some((
            mc_world::BlockPos {
                x: 900,
                y: 201,
                z: 900,
            },
            mc_world::BlockPos {
                x: 915,
                y: 205,
                z: 915,
            },
        )),
    };
    let bounds = ScriptWorldBounds::new([900, 201, 900], [915, 205, 915]).unwrap();
    assert!(
        super::piece_kinds(&piece, &state, &bounds).is_none(),
        "an unloaded column must retry, not classify"
    );
}

fn villagers_of(state: &TickState) -> Vec<(mc_domain::EntityId, Option<SettlementId>)> {
    state
        .entities
        .slots()
        .filter_map(|slot| {
            state
                .entities
                .row(slot)
                .villager
                .as_ref()
                .map(|v| (state.entities.body(slot).id, v.settlement))
        })
        .collect()
}

#[test]
fn generated_village_adopts_once_and_binds_residents() {
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![
            (
                generated_claim(
                    "minecraft:village/plains/houses/plains_house_1",
                    [0, 201, 0],
                ),
                [6.5, 202.0, 6.5],
                "farmer".into(),
            ),
            (
                generated_claim(
                    "minecraft:village/plains/houses/plains_house_1",
                    [0, 201, 0],
                ),
                [9.5, 202.0, 9.5],
                "none".into(),
            ),
        ],
        vec![],
    );
    let changes = adopt(&mut state, &chunks);
    assert!(!changes.blocks.is_empty() || state.settlements.records().count() == 1);
    let record = state
        .settlements
        .records()
        .find(|record| record.anchor == BELL)
        .expect("bell anchor adopted");
    assert_eq!(record.owner, None);
    assert_eq!(record.id.0, state.settlement_sequence);
    assert!(
        record
            .village_source
            .as_deref()
            .is_some_and(|source| source.starts_with("village:minecraft:overworld:"))
    );
    let storage = record.bootstrap_storage.expect("founding chest");
    assert!(is_chest_block(&state, storage));
    // Both nearby villagers bound to the exact native entities.
    let residents = villagers_of(&state);
    assert_eq!(residents.len(), 2);
    assert!(
        residents
            .iter()
            .all(|(_, settlement)| *settlement == Some(record.id))
    );
    // Adopted marker committed so removal is never silently re-adopted.
    assert!(
        state
            .generated_entity_claims
            .contains(&adopted_marker(record.village_source.as_deref().unwrap()))
    );
    // Second tick with the same chunks: no duplicate record, no re-bind churn.
    let before = state.settlements.records().count();
    adopt(&mut state, &chunks);
    assert_eq!(state.settlements.records().count(), before);
    assert_eq!(villagers_of(&state).len(), 2);
}

#[test]
fn player_bell_without_generated_provenance_never_adopts() {
    // A hand-placed bell on chunks with no generated markers or sites: the
    // fixture generator authors no markers, and `block_at` bell with no
    // worldgen site cover still yields a source — but adoption requires a
    // generated villager claim on the same tick. Here neither exists.
    let (mut state, chunks) = fixture(vec![[24, 201, 24]], vec![], vec![]);
    let bell_id = {
        let name = mc_data::Identifier::parse("minecraft:bell").unwrap();
        state.registry.blocks.block(&name).unwrap().default.0
    };
    state.world.set_block(24, 202, 24, bell_id);
    state.world.fold_overlay();
    let mut changes = ChangeSet::default();
    changes.chunks_loaded.extend_from_slice(&chunks);
    tick(&mut state, &mut changes);
    // The hand bell has no generated provenance: no village adopted.
    assert_eq!(state.settlements.records().count(), 0);
}

#[test]
fn owned_villagers_are_never_reassigned() {
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![
            (
                generated_claim(
                    "minecraft:village/plains/houses/plains_house_1",
                    [0, 201, 0],
                ),
                [6.5, 202.0, 6.5],
                "farmer".into(),
            ),
            (
                generated_claim(
                    "minecraft:village/plains/houses/plains_house_2",
                    [16, 201, 16],
                ),
                [7.5, 202.0, 7.5],
                "none".into(),
            ),
        ],
        vec![],
    );
    // A player town claims the second villager first.
    let town = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "town".into(),
            "owner".into(),
            [7, 201, 7],
            48,
        )
        .unwrap();
    let mut changes = ChangeSet::default();
    crate::villager::generated::load(&mut state, &chunks, &mut changes);
    // Bind the second spawned villager to the player town manually.
    let spawned: Vec<_> = changes.entities_spawned.clone();
    assert_eq!(spawned.len(), 2);
    crate::settlements::bind(&mut state, town, spawned[1]).unwrap();
    changes.chunks_loaded.extend_from_slice(&chunks);
    tick(&mut state, &mut changes);
    let village = state
        .settlements
        .records()
        .find(|record| record.village_source.is_some())
        .expect("village adopted")
        .id;
    assert_ne!(village, town);
    let second = state.entities.slot_of(spawned[1]).unwrap();
    assert_eq!(
        state
            .entities
            .row(second)
            .villager
            .as_ref()
            .unwrap()
            .settlement,
        Some(town),
        "adoption never steals an owned resident"
    );
    let first = state.entities.slot_of(spawned[0]).unwrap();
    assert_eq!(
        state
            .entities
            .row(first)
            .villager
            .as_ref()
            .unwrap()
            .settlement,
        Some(village)
    );
}

#[test]
fn row_restore_keeps_identity_and_rebinds_fresh_villagers() {
    let (state, _) = autonomously_adopted_state();
    let record = state
        .settlements
        .records()
        .find(|r| r.anchor == BELL)
        .expect("adopted")
        .clone();
    let source = record.village_source.clone().unwrap();
    // Native row round-trip: per-record checkpoint row -> fresh registry.
    let rows = crate::settlements::persistence::checkpoint_rows(&state, 11).unwrap();
    let mut reopened = fixture(vec![BELL], vec![], vec![]).0;
    crate::settlements::persistence::hydrate_dimension(
        &mut reopened,
        &rows,
        &crate::journal::Replay::default(),
        record.id.0,
    )
    .unwrap();
    let restored = reopened.settlements.get(record.id).expect("row restored");
    assert_eq!(restored.village_source, Some(source.clone()));
    assert_eq!(restored.anchor, BELL);
    assert_eq!(restored.bootstrap_storage, record.bootstrap_storage);
    // Re-adopting the same source returns the identical id (no duplicate).
    let again = reopened
        .settlements
        .adopt_village(&mut reopened.settlement_sequence, &source, BELL)
        .unwrap();
    assert_eq!(again, record.id);
}

#[test]
fn generated_chunk_load_order_does_not_change_identity() {
    let villagers = vec![(
        generated_claim(
            "minecraft:village/plains/houses/plains_house_1",
            [0, 201, 0],
        ),
        [6.5, 202.0, 6.5],
        "farmer".into(),
    )];
    // Forward order.
    let (mut first, chunks) = fixture(vec![BELL], villagers.clone(), vec![]);
    adopt(&mut first, &chunks);
    let first_id = first
        .settlements
        .records()
        .find(|r| r.anchor == BELL)
        .expect("adopted")
        .id;
    // Reverse order: same bell, same source, same id.
    let (mut second, _) = fixture(vec![BELL], villagers, vec![]);
    let mut reversed = chunks.clone();
    reversed.reverse();
    adopt(&mut second, &reversed);
    let second_id = second
        .settlements
        .records()
        .find(|r| r.anchor == BELL)
        .expect("adopted")
        .id;
    assert_eq!(first_id, second_id);
    assert_eq!(
        first.settlements.get(first_id).unwrap().village_source,
        second.settlements.get(second_id).unwrap().village_source
    );
}

#[test]
fn natural_mine_candidate_rejects_water_beside_its_first_cut() {
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![(
            generated_claim(
                "minecraft:village/plains/houses/plains_house_1",
                [0, 201, 0],
            ),
            [6.5, 202.0, 6.5],
            "none".into(),
        )],
        vec![],
    );
    adopt(&mut state, &chunks);
    let id = state.settlements.records().next().unwrap().id;
    let site = state
        .world
        .village_sites_in_region((0, 0), (0, 0))
        .remove(0);
    let mut candidates = VecDeque::new();
    natural_zones(&mut state, id, BELL, &site, 0, &mut candidates);
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.kind == BuildingKind::Mine)
    );
    candidates.clear();
    let water = state
        .registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:water").unwrap())
        .unwrap()
        .default
        .0;
    state.world.set_block(19, 201, 7, water);
    natural_zones(&mut state, id, BELL, &site, 0, &mut candidates);
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.kind != BuildingKind::Mine),
        "a flooded first cut must not become a mine job"
    );
}

#[test]
fn natural_lumber_and_mine_zones_certify_after_deferred_route_proofs() {
    // Grove west of the bell (x in [-15,-1] band style, local columns) and a
    // stone band below the surface for the quarry box.
    let logs: Vec<[i32; 2]> = (-14..=-9)
        .flat_map(|x| (4..=7).map(move |z| [x, z]))
        .collect();
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![(
            generated_claim(
                "minecraft:village/plains/houses/plains_house_1",
                [0, 201, 0],
            ),
            [6.5, 202.0, 6.5],
            "none".into(),
        )],
        logs,
    );
    adopt(&mut state, &chunks);
    assert!(
        state
            .settlements
            .records()
            .next()
            .unwrap()
            .buildings
            .values()
            .all(|building| !building.natural_zone),
        "chunk adoption must only queue candidates, never run route proof"
    );
    let before = state
        .tick_budget
        .remaining(crate::tick_budget::Kind::PathNode);
    // Even another adoption call in the same tick cannot prove queued routes.
    tick(&mut state, &mut ChangeSet::default());
    assert_eq!(
        state
            .tick_budget
            .remaining(crate::tick_budget::Kind::PathNode),
        before
    );
    state.tick += 20;
    state.tick_budget.reset(state.tick);
    state
        .tick_budget
        .exhaust_for_test(crate::tick_budget::Kind::PathNode);
    tick(&mut state, &mut ChangeSet::default());
    assert!(
        state
            .settlements
            .records()
            .next()
            .unwrap()
            .buildings
            .values()
            .all(|building| !building.natural_zone),
        "budget refusal must defer the candidate, not install a job"
    );
    settle_natural_proofs(&mut state, &[BuildingKind::Mine, BuildingKind::LumberCamp]);
    let record = state
        .settlements
        .records()
        .find(|r| r.anchor == BELL)
        .expect("adopted");
    // Zone boxes are opportunistic: grove/quarry must genuinely pass. The
    // fixture plants both; at least the lumber grove must certify.
    let kinds: Vec<BuildingKind> = record
        .buildings
        .values()
        .map(|building| building.kind)
        .collect();
    assert!(
        kinds.contains(&BuildingKind::LumberCamp),
        "grove certifies a natural lumber camp, got {kinds:?}"
    );
    let mine = record
        .buildings
        .values()
        .find(|building| building.kind == BuildingKind::Mine)
        .expect("stone zone certifies a natural mine");
    assert_eq!(
        mine.work_points[0][0], mine.bounds.min[0],
        "the west-edge entrance leaves the staircase enough horizontal run to reach stone"
    );
    assert_eq!(mine.bounds.max[0] - mine.bounds.min[0] + 1, 15);
    assert_eq!(mine.bounds.max[2] - mine.bounds.min[2] + 1, 15);
    assert_eq!(
        mine.work_points[0][2],
        mine.bounds.min[2] + 1,
        "the entrance leaves enough branch rows to fund storage and a house"
    );
    let existing = record.buildings.clone();
    let id = record.id;
    let [x, y, z] = mine.work_points[0];
    state.world.set_block(x, y, z, state.registry.states.stone);
    state.tick += 20;
    state.tick_budget.reset(state.tick);
    tick(&mut state, &mut ChangeSet::default());
    assert_eq!(
        state.settlements.get(id).unwrap().buildings,
        existing,
        "route proof gates new adoption only; existing certificates are not revoked"
    );
}

#[test]
fn natural_lumber_camp_certifies_the_grove_not_one_tree_column() {
    // CIV-6: the gate's phase log (20261124T020538) found the natural camp
    // certified from a 1x8x1 column, so a lumberjack's zone was one trunk and
    // the town never harvested wood. The grove west of the bell spans several
    // columns; certification must cover the grove, not the probed column.
    let logs: Vec<[i32; 2]> = (-14..=-9)
        .flat_map(|x| (4..=7).map(move |z| [x, z]))
        .collect();
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![(
            generated_claim(
                "minecraft:village/plains/houses/plains_house_1",
                [0, 201, 0],
            ),
            [6.5, 202.0, 6.5],
            "none".into(),
        )],
        logs,
    );
    adopt(&mut state, &chunks);
    settle_natural_proofs(&mut state, &[BuildingKind::LumberCamp]);
    let camp = state
        .settlements
        .records()
        .next()
        .unwrap()
        .buildings
        .values()
        .find(|building| building.kind == BuildingKind::LumberCamp)
        .expect("the grove certifies a natural lumber camp");
    assert!(camp.natural_zone);
    let width = camp.bounds.max[0] - camp.bounds.min[0] + 1;
    let depth = camp.bounds.max[2] - camp.bounds.min[2] + 1;
    assert!(
        width > 2 && depth > 2,
        "a natural camp's zone is the grove, not one tree column: {width}x{depth} at {:?}..{:?}",
        camp.bounds.min,
        camp.bounds.max
    );
    // The grove is a bounded square around the proved trunk, not a log census:
    // the zone must still cover every column of the fixture stand (x -14..=-9,
    // z 4..=7) and must stop before the generated house piece, whose bounds
    // start at x = 0.
    assert!(
        camp.bounds.min[0] <= -14 && camp.bounds.max[0] >= -9,
        "the zone covers the whole stand, not one trunk: x {:?}..{:?}",
        camp.bounds.min[0],
        camp.bounds.max[0]
    );
    assert!(
        camp.bounds.min[2] <= 4 && camp.bounds.max[2] >= 7,
        "the zone covers the whole stand, not one trunk: z {:?}..{:?}",
        camp.bounds.min[2],
        camp.bounds.max[2]
    );
    assert!(
        camp.bounds.max[0] < 0,
        "the generated house piece stays outside the zone: x {:?}..{:?}",
        camp.bounds.min[0],
        camp.bounds.max[0]
    );
}

#[test]
fn natural_grove_ignores_structure_logs_and_finds_an_unblocked_side() {
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![(
            generated_claim(
                "minecraft:village/plains/houses/plains_house_1",
                [0, 201, 0],
            ),
            [-2.5, 202.0, 6.5],
            "none".into(),
        )],
        vec![[-8, 4], [-4, 4], [4, 4]],
    );
    state.world.set_block(-8, 206, 4, state.registry.states.air);
    for y in 202..=207 {
        state.world.set_block(-5, y, 4, state.registry.states.stone);
    }
    adopt(&mut state, &chunks);
    settle_natural_proofs(&mut state, &[BuildingKind::LumberCamp]);
    let camp = state
        .settlements
        .records()
        .next()
        .unwrap()
        .buildings
        .values()
        .find(|building| building.kind == BuildingKind::LumberCamp)
        .unwrap();
    assert!(camp.bounds.min[0] <= -4 && camp.bounds.max[0] >= -4);
    assert!(
        camp.bounds.max[0] < 0,
        "generated house must remain protected"
    );
    assert_ne!(camp.work_points[0], [-5, 202, 4]);
}

#[test]
#[ignore = "requires the local vanilla 26.1.2 content cache"]
fn seed_100_lumberjack_deposits_real_logs() {
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    let mut config = crate::config::CoreConfig::new(std::path::PathBuf::new(), 100);
    config.vanilla_data_dir = Some(data.clone());
    let registry = Arc::new(CoreRegistry::load(Some(&data)).unwrap());
    let terrain = crate::core::overworld_terrain(&config, &registry).unwrap();
    let world = crate::world::ChunkWorld::new(
        100,
        Arc::new(terrain),
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [128, 70, 0],
        mc_world::OVERWORLD_GEOMETRY,
    );
    let mut state = TickState::new(registry, world, 4, 512);
    let chunks: Vec<_> = (2..=11)
        .flat_map(|x| (-4..=4).map(move |z| ChunkPos { x, z }))
        .collect();
    state
        .world
        .ensure_blocking(&chunks.iter().map(|p| (*p, 0)).collect());
    for (index, chunk) in chunks.iter().filter(|p| p.x >= 3).enumerate() {
        state
            .tickets
            .insert(format!("seed100-{index}"), (*chunk, 0));
    }
    state.refresh_active_chunks(0);
    let mut changes = ChangeSet::default();
    crate::villager::generated::load(&mut state, &chunks, &mut changes);
    changes.chunks_loaded.extend(chunks);
    tick(&mut state, &mut changes);
    let id = state.settlements.records().next().unwrap().id;
    for _ in 0..12_000 {
        crate::tick::run_tick(&mut state, &[]);
        let record = state.settlements.get(id).unwrap();
        if let Some([x, y, z]) = record.bootstrap_storage
            && let Some(chest) = state.block_entities.get(&(x, y, z))
            && let Some(stack) = chest.slots.iter().find(|stack| {
                stack.count > 0
                    && state
                        .registry
                        .items
                        .name_of(stack.item_id)
                        .is_some_and(|name| name.path().ends_with("_log"))
            })
        {
            let camp = record
                .buildings
                .values()
                .find(|b| b.kind == BuildingKind::LumberCamp)
                .unwrap();
            let [camp_x, _, camp_z] = camp.work_points[0];
            assert!(
                state
                    .world
                    .is_active(camp_x.div_euclid(16), camp_z.div_euclid(16))
            );
            eprintln!(
                "tick={} camp={camp:?} deposited {:?} in [{x}, {y}, {z}]",
                state.tick, stack
            );
            return;
        }
        if state.tick.is_multiple_of(2000) {
            for slot in state.entities.slots() {
                if let Some(v) = state.entities.row(slot).villager.as_ref()
                    && v.data.profession == crate::villager::VillagerProfession::Lumberjack
                {
                    eprintln!(
                        "tick={} pos={:?} job={:?} activity={:?} navigation={:?} progress={:?} inventory={:?}",
                        state.tick,
                        state.entities.pos(slot),
                        v.job_site,
                        v.activity,
                        v.navigation_target,
                        v.lumber_progress,
                        v.inventory
                    );
                }
            }
        }
    }
    panic!(
        "seed 100: no logs deposited; buildings={:?}",
        state.settlements.get(id).unwrap().buildings
    );
}

#[test]
fn natural_mine_requires_a_route_from_entrance_to_stock() {
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![(
            generated_claim(
                "minecraft:village/plains/houses/plains_house_1",
                [0, 201, 0],
            ),
            [-7.5, 202.0, 0.5],
            "none".into(),
        )],
        vec![],
    );
    adopt(&mut state, &chunks);
    let id = state.settlements.records().next().unwrap().id;
    for z in -16..=31 {
        for y in 202..=210 {
            state.world.set_block(-1, y, z, state.registry.states.stone);
        }
    }
    let residents = state
        .entities
        .slots()
        .map(|slot| state.entities.row(slot).uuid)
        .collect();
    let mut candidates = VecDeque::from([NaturalCandidate {
        kind: BuildingKind::Mine,
        bounds: ScriptWorldBounds::new([-14, 188, -10], [-2, 203, 4]).unwrap(),
        entrance: [-14, 202, -9],
        queued_at: 0,
        stock_proved: false,
        after_resident: None,
    }]);
    for _ in 0..8 {
        state.tick += 20;
        state.tick_budget.reset(state.tick);
        prove_natural(&mut state, id, None, &residents, &mut candidates);
        if candidates.is_empty() {
            break;
        }
    }
    assert!(
        candidates.is_empty(),
        "unreachable stock must reject the candidate"
    );
    assert!(
        !has_kind(&state, id, BuildingKind::Mine),
        "a worker must be able to bring extracted materials to stock"
    );
}

#[test]
fn loaded_but_unsimulated_natural_worksite_is_not_certified() {
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![(
            generated_claim(
                "minecraft:village/plains/houses/plains_house_1",
                [0, 201, 0],
            ),
            [6.5, 202.0, 6.5],
            "none".into(),
        )],
        vec![],
    );
    state
        .world
        .ensure_blocking(&(1..=5).map(|x| (ChunkPos { x, z: 0 }, 0)).collect());
    adopt(&mut state, &chunks);
    let id = state.settlements.records().next().unwrap().id;
    state.settlements.get_mut(id).unwrap().radius = 136;
    let residents = state
        .entities
        .slots()
        .map(|slot| state.entities.row(slot).uuid)
        .collect();
    let mut candidates = VecDeque::from([NaturalCandidate {
        kind: BuildingKind::Mine,
        bounds: ScriptWorldBounds::new([80, 188, 0], [94, 203, 14]).unwrap(),
        entrance: [80, 202, 1],
        queued_at: 0,
        stock_proved: false,
        after_resident: None,
    }]);
    assert!(state.world.has_chunk(5, 0));
    assert!(!state.world.is_active(5, 0));
    state.tick = 20;
    state.tick_budget.reset(state.tick);
    prove_natural(&mut state, id, None, &residents, &mut candidates);
    // A resident-but-unsimulated entrance is retried, never certified: the zone
    // stays queued while the chunk may still become simulated (the player walks
    // over the village) and is dropped once its age passes the bound - one bound
    // for the whole probe batch, not a per-candidate counter.
    assert!(!has_kind(&state, id, BuildingKind::Mine));
    assert_eq!(candidates.len(), 1, "the worksite waits for its chunk");
    state.tick = NATURAL_ZONE_ENTRANCE_HOLD_TICKS - 20;
    state.tick_budget.reset(state.tick);
    prove_natural(&mut state, id, None, &residents, &mut candidates);
    assert_eq!(candidates.len(), 1, "still inside the bound");
    assert!(!has_kind(&state, id, BuildingKind::Mine));
    state.tick = NATURAL_ZONE_ENTRANCE_HOLD_TICKS;
    state.tick_budget.reset(state.tick);
    prove_natural(&mut state, id, None, &residents, &mut candidates);
    assert!(candidates.is_empty(), "the age bound expires it");
    assert!(!has_kind(&state, id, BuildingKind::Mine));
}

#[test]
fn natural_quarry_skips_an_isolated_plateau_for_a_reachable_work_point() {
    let start = [6.5, 202.0, 6.5];
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![(
            generated_claim(
                "minecraft:village/plains/houses/plains_house_1",
                [0, 201, 0],
            ),
            start,
            "none".into(),
        )],
        vec![],
    );
    // The first quarry probe is a valid stone deposit with an open entrance,
    // but its nine-block cliff cannot be climbed by an ordinary villager.
    // Flat stone remains available at later probes outside the house pieces.
    for x in 17..=31 {
        for z in 6..=20 {
            for y in 202..=210 {
                state.world.set_block(x, y, z, state.registry.states.stone);
            }
        }
    }
    adopt(&mut state, &chunks);
    settle_natural_proofs(&mut state, &[BuildingKind::Mine]);
    state.tick += 1;
    state.tick_budget.reset(state.tick);
    let mine = state
        .settlements
        .records()
        .next()
        .unwrap()
        .buildings
        .values()
        .find(|building| building.kind == BuildingKind::Mine)
        .expect("nearby accessible stone must still supply a quarry");
    let [x, y, z] = mine.work_points[0];
    let mut cache = crate::settlements::allocator::PoiCache::default();
    let mut path = Vec::new();
    assert!(
        crate::settlements::allocator::find_path(
            &mut state,
            &mut cache,
            start,
            crate::boundary::BlockPos { x, y, z },
            0.3,
            1.95,
            &mut path,
        ),
        "adoption certified an unreachable quarry entrance at {x},{y},{z}; path={path:?}"
    );
}

#[test]
fn treeless_village_leaves_queue_after_bounded_natural_zone_sweeps() {
    let villagers = vec![(
        generated_claim(
            "minecraft:village/plains/houses/plains_house_1",
            [0, 201, 0],
        ),
        [0.5, 202.0, 0.5],
        "none".into(),
    )];
    let (mut state, chunks) = fixture(vec![BELL], villagers, Vec::new());
    adopt(&mut state, &chunks);
    let radius = state.settlements.records().next().expect("adopted").radius;
    let passes = zone_probe_limit(radius).div_ceil(ZONE_PROBES_PER_PASS);
    for _ in 0..=passes * 4 + 8 {
        state.tick += 20;
        state.tick_budget.reset(state.tick);
        tick(&mut state, &mut ChangeSet::default());
    }

    assert!(
        state.autonomy_queue.villages.is_empty(),
        "treeless village must not keep probing after two full search cycles"
    );
}

#[test]
fn dense_village_finds_natural_zones_beyond_generated_footprints() {
    let villagers = [-24, -8, 8, 24]
        .into_iter()
        .flat_map(|x| {
            [-24, -8, 8, 24].into_iter().map(move |z| {
                (
                    generated_claim(
                        "minecraft:village/plains/houses/plains_house_1",
                        [x, 201, z],
                    ),
                    [f64::from(x) + 2.5, 202.0, f64::from(z) + 2.5],
                    "none".into(),
                )
            })
        })
        .collect();
    let logs = (-54..=-51)
        .flat_map(|x| (20..=23).map(move |z| [x, z]))
        .collect();
    let (mut state, chunks) = fixture(vec![BELL], villagers, logs);
    // Load the intervening streets as well as the resource chunks: an
    // isolated loaded island is not an ordinary worker-accessible zone.
    let outside = (-4..=4)
        .flat_map(|x| (-1..=2).map(move |z| (ChunkPos { x, z }, 0)))
        .collect();
    state.world.ensure_blocking(&outside);
    state
        .tickets
        .insert("dense-test".into(), (ChunkPos { x: 0, z: 0 }, 5));
    state.refresh_active_chunks(state.tick);
    adopt(&mut state, &chunks);
    let id = state.settlements.records().next().unwrap().id;
    state.settlements.get_mut(id).unwrap().radius = 136;
    let site = state
        .world
        .village_sites_in_region((0, 0), (0, 0))
        .remove(0);
    let residents = state
        .entities
        .slots()
        .map(|slot| state.entities.row(slot).uuid)
        .collect();
    let mut candidates = VecDeque::new();
    for cursor in 0..1024 {
        natural_zones(&mut state, id, BELL, &site, cursor, &mut candidates);
        while !candidates.is_empty() {
            state.tick += 20;
            state.tick_budget.reset(state.tick);
            prove_natural(&mut state, id, None, &residents, &mut candidates);
        }
        if has_kind(&state, id, BuildingKind::Mine)
            && has_kind(&state, id, BuildingKind::LumberCamp)
        {
            break;
        }
    }
    let record = state.settlements.get(id).unwrap();
    for kind in [BuildingKind::Mine, BuildingKind::LumberCamp] {
        let zone = record
            .buildings
            .values()
            .find(|building| building.kind == kind)
            .unwrap_or_else(|| {
                panic!("{kind:?} must find loaded terrain outside the dense village")
            });
        assert!(
            site.pieces
                .iter()
                .filter_map(|piece| piece.bounds)
                .all(|(min, max)| {
                    let lo = [min.x, min.y, min.z];
                    let hi = [max.x, max.y, max.z];
                    ![0, 2].into_iter().all(|axis| {
                        lo[axis] <= zone.bounds.max[axis] && hi[axis] >= zone.bounds.min[axis]
                    })
                })
        );
        assert!(
            zone.bounds.min[0] < BELL[0] - 16 || zone.bounds.max[0] > BELL[0] + 32,
            "search must leave the original bell-neighbourhood probes"
        );
    }
}

#[test]
fn natural_quarry_uses_local_entrance_height_instead_of_bell_height() {
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![(
            generated_claim(
                "minecraft:village/plains/houses/plains_house_1",
                [0, 201, 0],
            ),
            [6.5, 202.0, 6.5],
            "none".into(),
        )],
        vec![],
    );
    for x in 17..=31 {
        for z in 6..=20 {
            for y in 202..=210 {
                state.world.set_block(x, y, z, state.registry.states.stone);
            }
        }
    }
    // A real ascending approach preserves the high local entrance case;
    // the separate cliff regression intentionally has no such route.
    for step in 0..=8 {
        for z in 6..=8 {
            for y in 202..=202 + step {
                state
                    .world
                    .set_block(8 + step, y, z, state.registry.states.stone);
            }
        }
    }
    adopt(&mut state, &chunks);
    settle_natural_proofs(&mut state, &[BuildingKind::Mine]);
    let mine = state
        .settlements
        .records()
        .next()
        .unwrap()
        .buildings
        .values()
        .find(|building| building.kind == BuildingKind::Mine)
        .expect("loaded stone terrain above the bell still supplies a natural quarry");
    assert_eq!(mine.work_points[0][1], 211);
    assert!(mine.bounds.min[1] < 211 && mine.bounds.max[1] >= 212);
}

/// A quarry protects the ground it can cut: `mining::operational_bounds` lowers
/// the floor 48 blocks under the certificate, so only a piece **wholly above the
/// quarry top** is out of its way (a piece at `entrance+2` still gets the soil
/// under it combed). A piece wholly below the box is still inside the dug volume
/// and refuses the mine, exactly as the old X/Z rule did; the old rule's bug was
/// that it refused *every* village piece by X/Z alone (HANDOFF-42/44/45).
#[test]
fn natural_quarry_never_undermines_a_generated_piece_at_another_height() {
    let (mut state, id) = autonomously_adopted_state();
    let record = state.settlements.get_mut(id).unwrap();
    record.radius = 136;
    record
        .buildings
        .retain(|_, building| building.kind != BuildingKind::Mine);
    let mut site = state
        .world
        .village_sites_in_region((0, 0), (0, 0))
        .remove(0);
    let piece =
        |min: mc_world::BlockPos, max: mc_world::BlockPos| mc_world::GeneratedVillagePiece {
            template: Some("minecraft:village/plains/houses/plains_house_1".into()),
            position: min,
            rotation: 0,
            entity_owner: Some("minecraft:village/plains/houses".into()),
            bounds: Some((min, max)),
        };
    let bounds = ScriptWorldBounds::new([17, 188, 6], [31, 203, 20]).unwrap();
    // 1. A house far above the quarry: not in its volume, so not in its way.
    site.pieces.push(piece(
        mc_world::BlockPos {
            x: 17,
            y: 250,
            z: 6,
        },
        mc_world::BlockPos {
            x: 31,
            y: 260,
            z: 20,
        },
    ));
    site.max.y = 260;
    let mut candidates = VecDeque::new();
    queue_natural(
        &site,
        BuildingKind::Mine,
        bounds,
        [17, 202, 7],
        state.tick,
        &mut candidates,
    );
    assert!(
        !candidates.is_empty(),
        "a piece 47 blocks above the quarry is out of its volume"
    );
    assert!(
        !has_kind(&state, id, BuildingKind::Mine),
        "queueing is not certification"
    );
    // 2. A piece wholly below the certificate box: the quarry digs 48 blocks under
    //    it, so this piece's natural() blocks are inside the dug volume. The old
    //    X/Z rule protected it; so does this one.
    site.pieces.pop();
    site.pieces.push(piece(
        mc_world::BlockPos {
            x: 17,
            y: 150,
            z: 6,
        },
        mc_world::BlockPos {
            x: 31,
            y: 160,
            z: 20,
        },
    ));
    let mut candidates = VecDeque::new();
    queue_natural(
        &site,
        BuildingKind::Mine,
        bounds,
        [17, 202, 7],
        state.tick,
        &mut candidates,
    );
    assert!(
        candidates.is_empty(),
        "a piece in the dug volume below the box is not even queued"
    );
    // 3. A piece whose volume the quarry would cut at its own level.
    site.pieces.pop();
    site.pieces.push(piece(
        mc_world::BlockPos {
            x: 17,
            y: 195,
            z: 6,
        },
        mc_world::BlockPos {
            x: 31,
            y: 205,
            z: 20,
        },
    ));
    let mut candidates = VecDeque::new();
    queue_natural(
        &site,
        BuildingKind::Mine,
        bounds,
        [17, 202, 7],
        state.tick,
        &mut candidates,
    );
    assert!(
        candidates.is_empty(),
        "a piece standing inside the certificate box is not even queued"
    );
    // 4. The boundary is the quarry top: a piece whose floor *is* the top is still
    //    reachable (rows cut `entrance-1..entrance`), one block higher is not.
    site.pieces.pop();
    site.pieces.push(piece(
        mc_world::BlockPos {
            x: 17,
            y: 203,
            z: 6,
        },
        mc_world::BlockPos {
            x: 31,
            y: 210,
            z: 20,
        },
    ));
    let mut candidates = VecDeque::new();
    queue_natural(
        &site,
        BuildingKind::Mine,
        bounds,
        [17, 202, 7],
        state.tick,
        &mut candidates,
    );
    assert!(
        candidates.is_empty(),
        "a piece whose floor is the quarry top is still in reach"
    );
    site.pieces.pop();
    site.pieces.push(piece(
        mc_world::BlockPos {
            x: 17,
            y: 204,
            z: 6,
        },
        mc_world::BlockPos {
            x: 31,
            y: 210,
            z: 20,
        },
    ));
    let mut candidates = VecDeque::new();
    queue_natural(
        &site,
        BuildingKind::Mine,
        bounds,
        [17, 202, 7],
        state.tick,
        &mut candidates,
    );
    assert!(
        !candidates.is_empty(),
        "a piece wholly above the quarry top is out of reach"
    );
}

/// Reusable founding helper: adopt one bell with the exact runtime contract
/// (generated load first, then autonomy tick) and return the adopted record.
/// Used by the row-restore test below and exposed for Builders' E2E.
pub(crate) fn autonomously_adopted_state() -> (TickState, SettlementId) {
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![(
            generated_claim(
                "minecraft:village/plains/houses/plains_house_1",
                [0, 201, 0],
            ),
            [6.5, 202.0, 6.5],
            "farmer".into(),
        )],
        vec![],
    );
    adopt(&mut state, &chunks);
    let id = state
        .settlements
        .records()
        .find(|record| record.anchor == BELL)
        .expect("fixture village adopts")
        .id;
    (state, id)
}

#[test]
fn adopted_founding_helper_matches_inline_adoption() {
    let (state, id) = autonomously_adopted_state();
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.anchor, BELL);
    assert!(record.village_source.is_some());
    assert!(record.bootstrap_storage.is_some());
}

#[test]
fn a_player_bell_beside_generated_residents_does_not_found_another_village() {
    let (mut state, chunks) = fixture(
        vec![[4, 201, 4], BELL],
        vec![(
            generated_claim(
                "minecraft:village/plains/houses/plains_house_1",
                [0, 201, 0],
            ),
            [6.5, 202.0, 6.5],
            "none".into(),
        )],
        vec![],
    );
    adopt(&mut state, &chunks);
    assert_eq!(state.settlements.records().count(), 1);
    assert_eq!(state.settlements.records().next().unwrap().anchor, BELL);
}

#[test]
fn restart_initial_scan_does_not_require_fresh_generated_claims_or_loaded_events() {
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![(
            generated_claim(
                "minecraft:village/plains/houses/plains_house_1",
                [0, 201, 0],
            ),
            [6.5, 202.0, 6.5],
            "none".into(),
        )],
        vec![],
    );
    let mut changes = ChangeSet::default();
    crate::villager::generated::load(&mut state, &chunks, &mut changes);
    state.new_generated_entity_claims.clear();
    assert!(state.generated_entity_initial_scan);
    tick(&mut state, &mut ChangeSet::default());
    assert_eq!(state.settlements.records().count(), 1);
    assert!(villagers_of(&state).iter().all(|(_, id)| id.is_some()));
}

#[test]
fn removing_a_claimed_generated_village_retires_its_source_across_queue_rebuilds() {
    let (mut state, id) = autonomously_adopted_state();
    state.settlements.claim(id, "owner".into(), BELL).unwrap();
    assert!(super::super::remove(&mut state, id));
    state.autonomy_queue = AutonomyQueue::default();
    tick(&mut state, &mut ChangeSet::default());
    assert_eq!(state.settlements.records().count(), 0);
}

pub(super) fn native_state(config: &crate::CoreConfig) -> TickState {
    let registry = Arc::new(CoreRegistry::load(None).unwrap());
    let generator = Arc::new(VillageGenerator {
        terrain: mc_worldgen::TerrainGenerator::new(42, Arc::clone(&registry.blocks)),
        registry: Arc::clone(&registry),
        bells: vec![BELL],
        villagers: vec![(
            generated_claim("minecraft:village/plains/houses", [0, 201, 0]),
            [6.5, 202.0, 6.5],
            "none".into(),
        )],
        logs: vec![],
        furniture: Vec::new(),
    });
    let mut world = crate::world::ChunkWorld::new_native(
        42,
        crate::native_checkpoint::tests::fixture_store(config).unwrap(),
        crate::native_checkpoint::dimension_id(crate::dimensions::Dimension::Overworld),
        generator,
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [8, 201, 8],
        mc_world::OVERWORLD_GEOMETRY,
    );
    let chunk = ChunkPos { x: 0, z: 0 };
    world.ensure_blocking(&[(chunk, 0)].into_iter().collect());
    TickState::new(registry, world, 2, 512)
}

fn native_village(config: &crate::CoreConfig) -> (TickState, SettlementId) {
    let mut state = native_state(config);
    let chunk = ChunkPos { x: 0, z: 0 };
    adopt(&mut state, &[chunk]);
    let id = state.settlements.records().next().unwrap().id;
    (state, id)
}

pub(super) fn native_checkpoint(state: &mut TickState, config: &crate::CoreConfig, tick: u64) {
    use crate::native_checkpoint::{encode_entities, encode_players, record, refresh_metadata};
    use mc_world::native::NativeCheckpoint;
    state.world.fold_overlay();
    state.tick = tick;
    let store = crate::native_checkpoint::tests::fixture_store(config).unwrap();
    let (players, captured, block_entities) = crate::core::checkpoint_data_for(state, tick);
    let entities = encode_entities(
        crate::dimensions::Dimension::Overworld,
        &captured,
        &state.registry.items,
        tick,
    )
    .unwrap();
    let players = encode_players(players.iter(), &state.registry.items, tick).unwrap();
    let capture = state.world.capture_dirty(&block_entities);
    let written = capture.written();
    let mut image = store.read_checkpoint().unwrap();
    assert!(
        !written.is_empty() || image.tick > 0,
        "the native baseline must capture the real founding chest"
    );
    let mut manifest: crate::journal::CheckpointManifest =
        serde_json::from_slice(&image.globals["checkpoint"].payload).unwrap();
    manifest.tick = tick;
    manifest.world_spawn = Some([8, 201, 8]);
    manifest.generated_entity_claims = state.generated_entity_claims.clone();
    for child in manifest.dimensions.values_mut() {
        child.tick = tick;
    }
    image
        .globals
        .extend(crate::settlements::persistence::checkpoint_rows(state, tick).unwrap());
    image.globals.insert(
        "checkpoint".into(),
        record(tick, serde_json::to_vec(&manifest).unwrap()),
    );
    refresh_metadata(&mut image.globals, &manifest).unwrap();
    for record in image.globals.values_mut() {
        record.captured_tick = tick;
    }
    store
        .commit_checkpoint_stream(
            NativeCheckpoint {
                tick,
                previous_tick: image.tick,
                chunks: &[],
                entities: &[],
                players: &players,
                globals: &image.globals,
            },
            |write| {
                capture
                    .visit_native(|snapshot| {
                        write(snapshot)
                            .map_err(|error| crate::config::CoreError::Storage(error.to_string()))
                    })
                    .map_err(|error| mc_world::native::NativeError::Source(error.to_string()))
            },
            entities.iter().map(Ok),
            &state.registry.blocks,
            &state.registry.items,
        )
        .unwrap();
    state.world.mark_flushed(&written);
}

#[test]
fn founded_empty_chest_is_distinct_once_and_survives_native_restart() {
    use crate::commands::Command;
    use crate::settlements::commands::SettlementCommand;
    let directory = tempfile::tempdir().unwrap();
    let config = crate::CoreConfig::new(directory.path().to_owned(), 42);
    let mut state = native_state(&config);
    let (connection, executor) = super::starter_tests::founder(&mut state);
    let records_before = state.settlements.records().count();
    let existing = [9, 202, 8];
    let site = storage_chest_site(&state, executor, BELL).unwrap();
    state
        .world
        .set_block(existing[0], existing[1], existing[2], site.block);
    let stock = mc_data::ItemStack::new(
        state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:emerald").unwrap())
            .unwrap(),
        7,
    );
    state.block_entities.insert(
        (existing[0], existing[1], existing[2]),
        crate::tick::BlockEntityState {
            slots: vec![stock.clone(); 27],
            furnace: None,
        },
    );
    let command = Command::Settlement(SettlementCommand::Create {
        name: "founded".into(),
        radius: 48,
    });
    let mut changes = ChangeSet::default();
    crate::command_execution::execute(&mut state, connection, executor, &command, &mut changes);
    let record = state.settlements.find("founded").unwrap();
    let id = record.id;
    let storage = record.bootstrap_storage.unwrap();
    assert_ne!(storage, existing);
    assert_eq!(changes.blocks.len(), 1);
    assert_eq!(
        changes.blocks[0].position,
        BlockPos {
            x: storage[0],
            y: storage[1],
            z: storage[2]
        }
    );
    assert!(
        state.block_entities[&(storage[0], storage[1], storage[2])]
            .slots
            .iter()
            .all(mc_data::ItemStack::is_empty)
    );
    assert_eq!(
        state.block_entities[&(existing[0], existing[1], existing[2])].slots[0],
        stock
    );
    // A repeated request cannot create a second chest or replace its contents.
    crate::command_execution::execute(&mut state, connection, executor, &command, &mut changes);
    ensure_storage_chest(&mut state, &mut changes, id, BELL, None);
    assert_eq!(changes.blocks.len(), 1);
    assert_eq!(state.settlements.records().count(), records_before + 1);
    native_checkpoint(&mut state, &config, 1);
    drop(state);
    let mut reopened =
        crate::core::recover_dimension_state(&config, crate::dimensions::Dimension::Overworld)
            .unwrap();
    assert_eq!(
        crate::settlements::starter_chest::granted(&reopened, uuid::Uuid::nil()),
        1
    );
    assert_eq!(
        reopened.settlements.get(id).unwrap().bootstrap_storage,
        Some(storage)
    );
    reopened
        .world
        .ensure_blocking(&[(ChunkPos { x: 0, z: 0 }, 0)].into_iter().collect());
    assert!(is_chest_block(&reopened, storage));
    let chunk = reopened.world.chunk(ChunkPos { x: 0, z: 0 }).unwrap();
    assert!(
        chunk.chests[&BlockPos {
            x: storage[0],
            y: storage[1],
            z: storage[2]
        }]
            .slots
            .iter()
            .all(|slot| slot.is_empty())
    );
    let restored_stock = &chunk.chests[&BlockPos {
        x: existing[0],
        y: existing[1],
        z: existing[2],
    }]
        .slots[0];
    assert_eq!(restored_stock.item_id, stock.item_id);
    assert_eq!(restored_stock.count, stock.count);
    let mut after = ChangeSet::default();
    ensure_storage_chest(&mut reopened, &mut after, id, BELL, None);
    assert!(after.blocks.is_empty());
}

#[test]
fn generated_village_membership_claim_and_empty_chest_survive_native_disk_restarts() {
    let directory = tempfile::tempdir().unwrap();
    let config = crate::CoreConfig::new(directory.path().to_owned(), 42);
    let (mut state, id) = native_village(&config);
    let storage = state
        .settlements
        .get(id)
        .unwrap()
        .bootstrap_storage
        .unwrap();
    let residents: Vec<_> = state
        .entities
        .slots()
        .filter_map(|slot| {
            state
                .entities
                .row(slot)
                .villager
                .as_ref()
                .map(|_| state.entities.row(slot).uuid)
        })
        .collect();
    native_checkpoint(&mut state, &config, 1);
    drop(state);
    let mut reopened =
        crate::core::recover_dimension_state(&config, crate::dimensions::Dimension::Overworld)
            .unwrap();
    assert_eq!(reopened.settlements.records().count(), 1);
    assert!(reopened.settlements.get(id).unwrap().owner.is_none());
    assert_eq!(
        reopened.settlements.get(id).unwrap().bootstrap_storage,
        Some(storage)
    );
    for uuid in &residents {
        let slot = reopened.entities.slot_of_uuid(*uuid).unwrap();
        assert_eq!(
            reopened
                .entities
                .row(slot)
                .villager
                .as_ref()
                .unwrap()
                .settlement,
            Some(id)
        );
    }
    reopened.world.ensure_blocking(
        &[(
            ChunkPos {
                x: storage[0].div_euclid(16),
                z: storage[2].div_euclid(16),
            },
            0,
        )]
        .into_iter()
        .collect(),
    );
    assert!(is_chest_block(&reopened, storage));
    let chest = reopened
        .world
        .chunk(ChunkPos {
            x: storage[0].div_euclid(16),
            z: storage[2].div_euclid(16),
        })
        .unwrap()
        .chests
        .get(&mc_world::BlockPos {
            x: storage[0],
            y: storage[1],
            z: storage[2],
        })
        .unwrap();
    assert!(
        chest
            .slots
            .iter()
            .all(|slot| slot == &mc_world::FurnaceSlot::EMPTY)
    );
    assert!(chest.loot_table.is_none());
    reopened
        .settlements
        .claim(id, "alice".into(), BELL)
        .unwrap();
    native_checkpoint(&mut reopened, &config, 2);
    drop(reopened);
    let mut claimed =
        crate::core::recover_dimension_state(&config, crate::dimensions::Dimension::Overworld)
            .unwrap();
    assert_eq!(
        claimed.settlements.get(id).unwrap().owner.as_deref(),
        Some("alice")
    );
    assert!(claimed.settlements.claim(id, "bob".into(), BELL).is_err());
    let source = claimed
        .settlements
        .get(id)
        .unwrap()
        .village_source
        .clone()
        .unwrap();
    assert_eq!(
        claimed
            .settlements
            .adopt_village(&mut claimed.settlement_sequence, &source, BELL),
        Ok(id)
    );
    assert_eq!(claimed.settlements.records().count(), 1);
}

#[test]
fn authoritative_generated_site_is_adopted_even_when_its_inhabitants_died() {
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![(
            generated_claim(
                "minecraft:village/plains/houses/plains_house_1",
                [0, 201, 0],
            ),
            [6.5, 202.0, 6.5],
            "none".into(),
        )],
        vec![],
    );
    let mut changes = ChangeSet::default();
    crate::villager::generated::load(&mut state, &chunks, &mut changes);
    for entity in &changes.entities_spawned {
        let slot = state.entities.slot_of(*entity).unwrap();
        state.entities.body_mut(slot).alive = false;
    }
    changes.chunks_loaded.extend(chunks);
    tick(&mut state, &mut changes);
    assert_eq!(state.settlements.records().count(), 1);
}

#[test]
fn residents_beyond_default_radius_and_chunk_scan_budget_bind_on_later_passes() {
    let (mut state, mut chunks) = fixture(
        vec![BELL],
        vec![
            (
                generated_claim(
                    "minecraft:village/plains/houses/plains_house_1",
                    [0, 201, 0],
                ),
                [6.5, 202.0, 6.5],
                "none".into(),
            ),
            (
                generated_claim(
                    "minecraft:village/plains/houses/plains_house_2",
                    [80, 201, 0],
                ),
                [82.5, 202.0, 6.5],
                "none".into(),
            ),
        ],
        vec![],
    );
    let outer = ChunkPos { x: 5, z: 0 };
    state
        .world
        .ensure_blocking(&[(outer, 0)].into_iter().collect());
    chunks.push(outer);
    adopt(&mut state, &chunks);
    for _ in 0..4 {
        state.tick += 1;
        tick(&mut state, &mut ChangeSet::default());
    }
    assert_eq!(state.settlements.records().count(), 1);
    assert!(state.settlements.records().next().unwrap().radius > 74);
    assert_eq!(villagers_of(&state).len(), 2);
    assert!(villagers_of(&state).iter().all(|(_, id)| id.is_some()));
}

#[test]
fn resident_bind_budget_retains_every_generated_resident() {
    let residents = (0..80)
        .map(|_| {
            (
                generated_claim(
                    "minecraft:village/plains/houses/plains_house_1",
                    [0, 201, 0],
                ),
                [6.5, 202.0, 6.5],
                "none".into(),
            )
        })
        .collect();
    let (mut state, chunks) = fixture(vec![BELL], residents, vec![]);
    adopt(&mut state, &chunks);
    for _ in 0..4 {
        state.tick += 1;
        tick(&mut state, &mut ChangeSet::default());
    }
    assert_eq!(villagers_of(&state).len(), 80);
    assert!(villagers_of(&state).iter().all(|(_, id)| id.is_some()));
}

#[test]
fn actual_pool_owner_claims_bind_even_when_the_template_id_differs() {
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![(
            generated_claim("minecraft:village/plains/houses", [0, 201, 0]),
            [6.5, 202.0, 6.5],
            "none".into(),
        )],
        vec![],
    );
    adopt(&mut state, &chunks);
    assert_eq!(villagers_of(&state).len(), 1);
    assert!(villagers_of(&state)[0].1.is_some());
}

#[test]
fn an_unloaded_generated_house_does_not_starve_a_loaded_farm_certificate() {
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![
            (
                generated_claim(
                    "minecraft:village/plains/houses/plains_house_1",
                    [80, 201, 0],
                ),
                [82.5, 202.0, 6.5],
                "none".into(),
            ),
            (
                generated_claim(
                    "minecraft:village/plains/houses/plains_large_farm_1",
                    [0, 201, 0],
                ),
                [6.5, 202.0, 6.5],
                "farmer".into(),
            ),
        ],
        vec![],
    );
    let composter = state
        .registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:composter").unwrap())
        .unwrap()
        .default
        .0;
    state.world.set_block(4, 202, 4, composter);
    state.world.fold_overlay();
    adopt(&mut state, &chunks);
    let record = state.settlements.records().next().unwrap();
    assert!(record.buildings.values().any(|building| building.kind
        == BuildingKind::Workshop(crate::villager::VillagerProfession::Farmer,)));
}

#[test]
#[ignore = "requires the local vanilla 26.1.2 content cache"]
fn seed_100_natural_miner_deposits_cobblestone() {
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    let mut config = crate::config::CoreConfig::new(std::path::PathBuf::new(), 100);
    config.vanilla_data_dir = Some(data.clone());
    let registry = Arc::new(CoreRegistry::load(Some(&data)).unwrap());
    let terrain = crate::core::overworld_terrain(&config, &registry).unwrap();
    let world = crate::world::ChunkWorld::new(
        100,
        Arc::new(terrain),
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [128, 70, 0],
        mc_world::OVERWORLD_GEOMETRY,
    );
    let mut state = TickState::new(registry, world, 4, 512);
    let chunks: Vec<_> = (2..=11)
        .flat_map(|x| (-4..=4).map(move |z| ChunkPos { x, z }))
        .collect();
    state
        .world
        .ensure_blocking(&chunks.iter().map(|p| (*p, 0)).collect());
    for (index, chunk) in chunks.iter().filter(|p| p.x >= 3).enumerate() {
        state
            .tickets
            .insert(format!("seed100-{index}"), (*chunk, 0));
    }
    state.refresh_active_chunks(0);
    let mut changes = ChangeSet::default();
    crate::villager::generated::load(&mut state, &chunks, &mut changes);
    changes.chunks_loaded.extend(chunks);
    tick(&mut state, &mut changes);
    let cobble = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:cobblestone").unwrap())
        .unwrap();
    for _ in 0..12_000 {
        crate::tick::run_tick(&mut state, &[]);
        if state.tick.is_multiple_of(1_000) {
            for record in state.settlements.records() {
                for mine in record
                    .buildings
                    .values()
                    .filter(|b| b.kind == BuildingKind::Mine)
                {
                    eprintln!(
                        "tick={} mine={:?} active={}",
                        state.tick,
                        mine.work_points,
                        state.world.is_active(
                            mine.work_points[0][0].div_euclid(16),
                            mine.work_points[0][2].div_euclid(16)
                        )
                    );
                }
            }
            for slot in state.entities.slots() {
                if let Some(v) = state.entities.row(slot).villager.as_ref()
                    && v.data.profession == crate::villager::VillagerProfession::Miner
                {
                    eprintln!(
                        "tick={} pos={:?} progress={:?} inventory={:?}",
                        state.tick,
                        state.entities.pos(slot),
                        v.mining_progress,
                        v.inventory
                    );
                }
            }
        }
        for record in state.settlements.records() {
            if let Some([x, y, z]) = record.bootstrap_storage
                && let Some(chest) = state.block_entities.get(&(x, y, z))
                && chest
                    .slots
                    .iter()
                    .any(|stack| stack.item_id == cobble && stack.count > 0)
            {
                eprintln!("tick={} cobblestone deposited at {x},{y},{z}", state.tick);
                return;
            }
        }
    }
    panic!("seed 100 miner did not deposit cobblestone");
}

#[test]
fn busy_gatherer_can_prove_the_other_natural_source() {
    use crate::villager::VillagerProfession;
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![(
            generated_claim(
                "minecraft:village/plains/houses/plains_house_1",
                [0, 201, 0],
            ),
            [8.5, 202.0, 6.5],
            "none".into(),
        )],
        vec![[10, 8]],
    );
    adopt(&mut state, &chunks);
    let id = state.settlements.records().next().unwrap().id;
    super::super::buildings::certify_zone(
        &mut state,
        id,
        BuildingKind::LumberCamp,
        ScriptWorldBounds::new([10, 201, 8], [11, 207, 8]).unwrap(),
        [11, 202, 8],
    )
    .unwrap();
    let slot = state
        .entities
        .slots()
        .find(|&slot| state.entities.row(slot).villager.is_some())
        .unwrap();
    let uuid = state.entities.row(slot).uuid;
    let v = state.entities.row_mut(slot).villager.as_mut().unwrap();
    v.data.profession = VillagerProfession::Lumberjack;
    v.job_site = Some(BlockPos {
        x: 11,
        y: 202,
        z: 8,
    });
    v.job_site_dimension = state.dimension.name().into();
    let mut candidates = VecDeque::from([NaturalCandidate {
        kind: BuildingKind::Mine,
        bounds: ScriptWorldBounds::new([-14, 188, -10], [-2, 203, 4]).unwrap(),
        entrance: [-14, 202, -9],
        queued_at: 0,
        stock_proved: false,
        after_resident: None,
    }]);
    for _ in 0..8 {
        state.tick += 20;
        state.tick_budget.reset(state.tick);
        prove_natural(
            &mut state,
            id,
            None,
            &[uuid].into_iter().collect(),
            &mut candidates,
        );
    }
    assert!(
        has_kind(&state, id, BuildingKind::Mine),
        "already staffing the grove must not make the only adult disappear from natural mine admission"
    );
}

#[test]
fn review_generated_village_adoption_respects_an_active_expedition_claim() {
    let (mut state, chunks) = fixture(
        vec![BELL],
        vec![(
            generated_claim(
                "minecraft:village/plains/houses/plains_house_1",
                [0, 201, 0],
            ),
            [6.5, 202.0, 6.5],
            "none".into(),
        )],
        vec![],
    );
    let parent = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "parent".into(),
            "owner".into(),
            [-96, 201, 8],
            16,
        )
        .unwrap();
    let mut family = Vec::new();
    for _ in 0..2 {
        let slot = crate::settlements::allocator::tests::villager(&mut state, [-96.5, 201.0, 8.5]);
        let entity = state.entities.body(slot).id;
        crate::settlements::bind(&mut state, parent, entity).unwrap();
        family.push(entity);
    }
    let household = crate::settlements::households::form(
        &mut state,
        parent,
        &family,
        crate::settlements::households::HouseholdOrigin::Migration,
    )
    .unwrap();
    state.settlements.get_mut(parent).unwrap().founding =
        Some(crate::settlements::construction::FoundingExpedition {
            destination: [0, 200, 0],
            household,
        });
    let sequence = state.settlement_sequence;
    adopt(&mut state, &chunks);
    assert_eq!(
        state.settlements.records().count(),
        1,
        "generated bell cannot steal an expedition claim"
    );
    assert_eq!(state.settlement_sequence, sequence);
    crate::settlements::persistence::finish_recovery(&mut state).unwrap();
}

/// The village gate clock must not starve the 20-tick gates: every pending
/// village takes them, whatever the queue length, while the queue keeps
/// stepping only a fixed batch a tick. Eight pending villages with a fixed
/// batch of four used to alternate [0..3] / [4..7], so villages 4..7 never saw
/// `tick % 20 == 0` (SETTLEMENT-AUTONOMY-VILLAGE-ROTATION-STARVES-TICK-GATES).
#[test]
fn every_pending_village_takes_its_gate_clock_turn() {
    for pending in 1..=16usize {
        let mut queue: std::collections::VecDeque<(usize, u64)> =
            (0..pending).map(|village| (village, 0)).collect();
        let mut gated = vec![false; pending];
        for tick in 0..400u64 {
            for _ in 0..super::VILLAGES_PER_TICK.min(queue.len()) {
                let (village, mut next_gate) = queue.pop_front().expect("counted village");
                if super::gate_due(&mut next_gate, tick) {
                    gated[village] = true;
                }
                queue.push_back((village, next_gate));
            }
        }
        assert!(
            gated.iter().all(|seen| *seen),
            "queue of {pending} pending villages starves some village's 20-tick gate"
        );
    }
}
