use super::*;
use crate::test_support::{fixture_store, read_manifest, write_manifest};
use mc_core::dimensions::Dimension;
use mc_core::journal::JournalWriter;
use mc_core::native_checkpoint::record;
use mc_core::settlements::households::{Household, HouseholdOrigin};
use mc_core::settlements::persistence::Row;
use mc_core::settlements::{Registry, SettlementId};
use mc_nbt::{ListTag, Tag, tag_type};
use mc_world::native::{NativeCheckpoint, NativeEntitySnapshot};
use mc_world::{BlockStateId, Chunk, ChunkPos};

const SEED: i64 = 42;

pub(crate) fn fields(
    registry: &mc_core::registry::CoreRegistry,
    id: &str,
    n: u128,
    wire: i32,
) -> Vec<(String, Tag)> {
    mc_data::entity_nbt::entity_to_fields(
        &registry.items,
        &mc_data::entity_nbt::VanillaEntity {
            id: id.into(),
            uuid: uuid::Uuid::from_u128(n),
            wire_id: wire,
            pos: [1.5, 80.0, 1.5],
            ..Default::default()
        },
    )
    .unwrap()
}

pub(crate) fn member(
    mut fields: Vec<(String, Tag)>,
    settlement: i64,
    household: Option<&str>,
) -> Vec<(String, Tag)> {
    let mut runtime = vec![("settlement".to_string(), Tag::Long(settlement))];
    if let Some(household) = household {
        runtime.push(("household".into(), Tag::String(household.into())));
    }
    fields.push(("krasnokamen:VillagerRuntime".into(), Tag::Compound(runtime)));
    fields
}

fn settlement_row(id: u64) -> Vec<u8> {
    let mut settlements = Registry::default();
    let mut sequence = id - 1;
    let created = settlements
        .create(
            &mut sequence,
            "Falcon".into(),
            "operator".into(),
            [0, 80, 0],
            32,
        )
        .unwrap();
    assert_eq!(created, SettlementId(id));
    let mut value = settlements.get(created).unwrap().clone();
    let household = Household {
        id: format!("{id}-1"),
        revision: 1,
        origin: HouseholdOrigin::Adoption,
        home: None,
        needs: None,
        policy: None,
    };
    value.household_sequence = 1;
    value.households.insert(household.id.clone(), household);
    serde_json::to_vec(&Row {
        dimension: Dimension::Overworld,
        value,
    })
    .unwrap()
}

/// One persisted transit record (ADR 0023) holding `members`, from settlement
/// 7 to 8, with the household record `leave` removed (if any).
pub(crate) fn transit_row(members: &[u128], household: Option<&str>) -> Vec<u8> {
    let member = |n: u128| {
        serde_json::json!({
            "uuid": uuid::Uuid::from_u128(n).to_string(), "type_id": 0,
            "pos": [16.5, 80.0, 0.5], "vel": [0.0, 0.0, 0.0], "yaw": 0.0, "pitch": 0.0,
            "ground": true, "health": 20.0, "alive": true, "age": 0, "breed_cooldown": 0,
            "pickup_delay": 0, "item": null, "villager": {},
            "appearance": {
                "item": null, "equipment": [], "baby": false, "pose": 0, "shared_flags": 0,
                "health": 20.0, "custom_name": null, "extra": []
            },
            "tick": 1
        })
    };
    serde_json::to_vec(&serde_json::json!({
        "dimension": Dimension::Overworld,
        "value": {
            "source": 7, "destination": 8, "destination_anchor": [400, 80, 0],
            "departed": 1, "arrives": 3_841, "bearing": 90.0, "exit": [16, 80, 0],
            "status": "travelling",
            "household": household.map(|id| serde_json::json!({
                "id": id, "revision": 2, "origin": "adoption",
                "home": null, "needs": null, "policy": null
            })),
            "members": members.iter().map(|&n| member(n)).collect::<Vec<_>>()
        }
    }))
    .unwrap()
}

pub(crate) struct Fixture {
    _dir: tempfile::TempDir,
    pub(crate) world: PathBuf,
    pub(crate) config: CoreConfig,
    pub(crate) tick: u64,
}

pub(crate) type EntityFixture = (u32, i32, i32, u128, Vec<(String, Tag)>);
/// One migration transit row: (record UUID, member UUIDs, household).
pub(crate) type TransitFixture<'a> = (u128, &'a [u128], Option<&'a str>);

/// Stopped native world: `entities` as (dimension, chunk x, chunk z, UUID, payload),
/// optional persisted settlement 7, one baseline overworld chunk.
fn world(entities: Vec<EntityFixture>, settlement: bool) -> Fixture {
    world_with(entities, settlement, &[])
}

/// [`world`] plus migration transit rows.
pub(crate) fn world_with(
    entities: Vec<EntityFixture>,
    settlement: bool,
    transits: &[TransitFixture],
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let world = dir.path().join("world");
    let config = crate::test_support::config(world.clone(), SEED);
    let registry = crate::test_support::registry().unwrap();
    let store = fixture_store(&config).unwrap();
    write_manifest(&config, &store, &read_manifest(&store).unwrap()).unwrap();
    let chunk = Chunk::empty(
        ChunkPos { x: 0, z: 0 },
        BlockStateId(registry.states.air),
        mc_data::Identifier::parse("minecraft:plains").unwrap(),
    );
    store
        .fill_generated(0, &[chunk], &registry.blocks, &registry.items)
        .unwrap();
    let mut image = store.read_checkpoint().unwrap();
    let tick = image.tick;
    if settlement {
        image
            .globals
            .insert("settlement/7".into(), record(tick, settlement_row(7)));
        image.globals.insert(
            "settlement_sequence".into(),
            record(tick, serde_json::to_vec(&7u64).unwrap()),
        );
    }
    for &(id, members, household) in transits {
        image.globals.insert(
            format!("transit/{}", uuid::Uuid::from_u128(id)),
            record(tick, transit_row(members, household)),
        );
    }
    let snapshots: Vec<_> = entities
        .into_iter()
        .map(|(dimension, x, z, uuid, payload)| NativeEntitySnapshot {
            identity: *uuid::Uuid::from_u128(uuid).as_bytes(),
            dimension,
            position: ChunkPos { x, z },
            generated_slot: None,
            journal_lsn: tick,
            payload: Some(Tag::Compound(payload)),
        })
        .collect();
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick,
                previous_tick: tick,
                chunks: &[],
                entities: &snapshots,
                players: &[],
                globals: &image.globals,
            },
            &registry.blocks,
            &registry.items,
        )
        .unwrap();
    drop(store);
    Fixture {
        _dir: dir,
        world,
        config,
        tick,
    }
}

fn populated() -> Fixture {
    let registry = crate::test_support::registry().unwrap();
    // The rider is the household's baby: the census counts it a member, not an adult.
    let mut rider = fields(&registry, "minecraft:villager", 0x30, 30);
    rider.retain(|(name, _)| name != "Age");
    rider.push(("Age".into(), Tag::Int(-24_000)));
    let cow_rider = Tag::List(ListTag {
        element_type: tag_type::COMPOUND,
        elements: vec![Tag::Compound(member(rider, 7, Some("7-1")))],
    });
    let mut cow = fields(&registry, "minecraft:cow", 0x20, 20);
    cow.push(("Passengers".into(), cow_rider));
    world(
        vec![
            (
                0,
                0,
                0,
                0x10,
                member(
                    fields(&registry, "minecraft:villager", 0x10, 10),
                    7,
                    Some("7-1"),
                ),
            ),
            (
                0,
                0,
                0,
                0x11,
                member(fields(&registry, "minecraft:villager", 0x11, 11), 7, None),
            ),
            (0, 0, 0, 0x20, cow),
            (
                1,
                5,
                5,
                0x12,
                member(fields(&registry, "minecraft:villager", 0x12, 12), 99, None),
            ),
        ],
        true,
    )
}

fn read(path: &Path) -> CensusReceipt {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn census_accepts_global_starter_chest_grant_rows() {
    let fixture = world(vec![], true);
    let registry = crate::test_support::registry().unwrap();
    let store = fixture_store(&fixture.config).unwrap();
    let mut image = store.read_checkpoint().unwrap();
    let player = uuid::Uuid::from_u128(0x123);
    image.globals.insert(
        format!("settlement_chest/{player}"),
        record(image.tick, serde_json::to_vec(&(player, 2_u64)).unwrap()),
    );
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick: image.tick,
                previous_tick: image.tick,
                chunks: &[],
                entities: &[],
                players: &[],
                globals: &image.globals,
            },
            &registry.blocks,
            &registry.items,
        )
        .unwrap();
    drop(store);
    let out = tempfile::tempdir().unwrap();
    let receipt =
        census_native(&fixture.config, &fixture.world, &out.path().join("c.json")).unwrap();
    assert_eq!(receipt.status, "PASS");
    assert!(receipt.read_only);
    assert_eq!(receipt.counts["settlements"], 1);
}

#[test]
fn census_counts_transit_records_and_members_once() {
    let registry = crate::test_support::registry().unwrap();
    let villager = (
        0,
        0,
        0,
        0x10,
        member(
            fields(&registry, "minecraft:villager", 0x10, 10),
            7,
            Some("7-1"),
        ),
    );
    let fixture = world_with(
        vec![villager],
        true,
        &[(0x40, &[0x40, 0x41], Some("7-2")), (0x50, &[0x50], None)],
    );
    let out = tempfile::tempdir().unwrap();
    let receipt =
        census_native(&fixture.config, &fixture.world, &out.path().join("c.json")).unwrap();
    assert_eq!(receipt.status, "PASS");
    assert_eq!(receipt.counts["transit_records"], 2);
    assert_eq!(receipt.counts["transit_members"], 3);
    assert_eq!(receipt.counts["villagers"], 1);
}

/// Negative controls of the exactly-once custody invariant: a member that is
/// also a stored entity, a member of two records, a household held twice.
#[test]
fn census_refuses_transit_custody_held_twice() {
    let registry = crate::test_support::registry().unwrap();
    let villager = || {
        (
            0,
            0,
            0,
            0x10,
            member(
                fields(&registry, "minecraft:villager", 0x10, 10),
                7,
                Some("7-1"),
            ),
        )
    };
    let cases: [(&[TransitFixture], &str); 3] = [
        (
            &[(0x10, &[0x10], None)],
            "UUID 00000000-0000-0000-0000-000000000010 of transit record",
        ),
        (
            &[(0x40, &[0x40, 0x41], None), (0x41, &[0x41], None)],
            "UUID 00000000-0000-0000-0000-000000000041 of transit record",
        ),
        (
            &[(0x40, &[0x40], Some("7-1"))],
            "household 7-1 is both in transit record",
        ),
    ];
    for (transits, detail) in cases {
        let fixture = world_with(vec![villager()], true, transits);
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("c.json");
        let error = census_native(&fixture.config, &fixture.world, &path)
            .unwrap_err()
            .to_string();
        assert!(error.contains(detail), "missing {detail:?} in {error}");
        assert_eq!(read(&path).status, "FAIL");
    }
}

#[test]
fn census_counts_entities_passengers_villagers_chunks_and_persisted_settlements() {
    let fixture = populated();
    let before = hashes(&fixture.world).unwrap();
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("census.json");
    let receipt = census_native(&fixture.config, &fixture.world, &path).unwrap();
    assert_eq!(read(&path).counts, receipt.counts);
    assert_eq!(receipt.status, "PASS");
    assert!(receipt.read_only);
    assert_eq!(receipt.checkpoint_tick, Some(fixture.tick));
    assert_eq!(receipt.source_unchanged, Some(true));
    let counts = |key: &str| receipt.counts[key];
    // Three overworld roots + one nether root; the cow carries one passenger.
    assert_eq!(counts("entity_roots"), 4);
    assert_eq!(counts("entities"), 5);
    assert_eq!(counts("villagers"), 4);
    assert_eq!(counts("chunks"), 1);
    assert_eq!(counts("settlements"), 1);
    assert_eq!(counts("households"), 1);
    assert_eq!(counts("settlement_members"), 4);
    assert_eq!(counts("household_members"), 2);
    assert_eq!(counts("settlement_orphan_members"), 1);
    assert_eq!(counts("settlement_sequence"), 7);
    let overworld = &receipt.dimensions["minecraft:overworld"];
    assert_eq!(
        (
            overworld.entity_roots,
            overworld.entities,
            overworld.villagers
        ),
        (3, 4, 3)
    );
    assert_eq!((overworld.chunks, overworld.settlements), (1, 1));
    let nether = &receipt.dimensions["minecraft:the_nether"];
    assert_eq!(
        (nether.entity_roots, nether.entities, nether.villagers),
        (1, 1, 1)
    );
    assert_eq!(
        receipt.dimensions["minecraft:the_end"],
        DimensionCensus::default()
    );
    assert_eq!(
        receipt.settlement_details,
        vec![SettlementCensus {
            id: 7,
            name: "Falcon".into(),
            dimension: "minecraft:overworld".into(),
            anchor: [0, 80, 0],
            radius: 32,
            households: 1,
            members: 3,
            household_members: 2,
            household_details: BTreeMap::from([(
                "7-1".into(),
                HouseholdCensus {
                    origin: HouseholdOrigin::Adoption,
                    members: 2,
                    adults: 1,
                },
            )]),
        }]
    );
    assert_eq!(
        hashes(&fixture.world).unwrap(),
        before,
        "source bytes unchanged"
    );
}

#[test]
fn census_reports_zero_settlements_when_no_core_rows_exist() {
    let registry = crate::test_support::registry().unwrap();
    let fixture = world(
        vec![(
            0,
            0,
            0,
            0x10,
            fields(&registry, "minecraft:villager", 0x10, 10),
        )],
        false,
    );
    let out = tempfile::tempdir().unwrap();
    let receipt =
        census_native(&fixture.config, &fixture.world, &out.path().join("c.json")).unwrap();
    assert_eq!(receipt.counts["villagers"], 1);
    assert_eq!(receipt.counts["settlements"], 0);
    assert_eq!(receipt.counts["settlement_members"], 0);
    assert!(receipt.settlement_details.is_empty());
}

#[test]
fn census_refuses_missing_native_world_without_touching_source() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    fs::create_dir(&source).unwrap();
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("c.json");
    let error = census_native(
        &crate::test_support::config(source.clone(), SEED),
        &source,
        &path,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("world import-anvil"), "{error}");
    let receipt = read(&path);
    assert_eq!(receipt.status, "FAIL");
    assert!(receipt.counts.is_empty());
    assert_eq!(fs::read_dir(&source).unwrap().count(), 0);
}

#[test]
fn census_refuses_journal_tail_instead_of_reporting_stale_counts() {
    let fixture = populated();
    let mut writer = JournalWriter::open(
        &fixture.world,
        1,
        mc_core::journal::test_wal_identity(&fixture.world),
    )
    .unwrap();
    writer
        .append(fixture.tick + 1, fixture.tick + 1, &[])
        .unwrap();
    writer.flush().unwrap();
    drop(writer);
    let before = hashes(&fixture.world).unwrap();
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("c.json");
    let error = census_native(&fixture.config, &fixture.world, &path)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("complete batches after checkpoint tick"),
        "{error}"
    );
    let receipt = read(&path);
    assert_eq!(receipt.status, "FAIL");
    assert!(receipt.counts.is_empty());
    assert_eq!(hashes(&fixture.world).unwrap(), before);
}

#[test]
fn census_refuses_entity_payload_uuid_that_differs_from_its_record_key() {
    let registry = crate::test_support::registry().unwrap();
    let fixture = world(
        vec![(
            0,
            0,
            0,
            0x10,
            fields(&registry, "minecraft:villager", 0x99, 10),
        )],
        false,
    );
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("c.json");
    let error = census_native(&fixture.config, &fixture.world, &path)
        .unwrap_err()
        .to_string();
    for detail in [
        "differs from native entity record key",
        "chunk (0, 0)",
        "minecraft:villager",
    ] {
        assert!(error.contains(detail), "missing {detail:?} in {error}");
    }
    assert_eq!(read(&path).status, "FAIL");
}

#[test]
fn census_receipt_inside_source_is_refused_without_writing() {
    let fixture = populated();
    let path = fixture.world.join("census.json");
    assert!(census_native(&fixture.config, &fixture.world, &path).is_err());
    assert!(!path.exists());
}

/// Books R4-P1 / advisor R2: a census promises a byte-identical source, so a
/// store ahead of its marker (the commit-to-marker crash window) is accepted
/// without rewriting the marker, and a disagreeing pair is refused.
#[test]
fn census_keeps_a_store_ahead_of_its_marker_byte_identical_and_refuses_a_bad_pair() {
    let fixture = populated();
    let marker = mc_world::store_marker::read(&fixture.world)
        .unwrap()
        .expect("fixture marker");
    mc_world::store_marker::write(
        &fixture.world,
        marker.store_uuid,
        mc_world::store_marker::ROOT_BRANCH,
        marker.checkpoint_tick.saturating_sub(1),
    )
    .unwrap();
    let ahead = mc_world::store_marker::read(&fixture.world)
        .unwrap()
        .unwrap();
    let before = hashes(&fixture.world).unwrap();
    let out = tempfile::tempdir().unwrap();
    let receipt = census_native(
        &fixture.config,
        &fixture.world,
        &out.path().join("ahead.json"),
    )
    .unwrap();
    assert_eq!(receipt.status, "PASS");
    assert_eq!(receipt.source_unchanged, Some(true));
    assert_eq!(
        hashes(&fixture.world).unwrap(),
        before,
        "a read-only census must not rewrite the source, marker included"
    );
    assert_eq!(
        mc_world::store_marker::read(&fixture.world).unwrap(),
        Some(ahead),
        "the marker stays as it was"
    );

    // Missing, foreign and behind markers are refused before any count.
    for (marker_write, expected) in [
        (None, "is missing but this store has checkpoint"),
        (Some([0x33; 16]), "belong to different worlds"),
    ] {
        let fixture = world(vec![], false);
        let ours = mc_world::store_marker::read(&fixture.world)
            .unwrap()
            .unwrap();
        match marker_write {
            None => std::fs::remove_file(mc_world::store_marker::path(&fixture.world)).unwrap(),
            Some(uuid) => mc_world::store_marker::write(
                &fixture.world,
                uuid,
                mc_world::store_marker::ROOT_BRANCH,
                ours.checkpoint_tick,
            )
            .unwrap(),
        }
        let out = tempfile::tempdir().unwrap();
        let error = census_native(
            &fixture.config,
            &fixture.world,
            &out.path().join("bad.json"),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains(expected), "{expected:?} in {error}");
    }

    let fixture = world(vec![], false);
    let ours = mc_world::store_marker::read(&fixture.world)
        .unwrap()
        .unwrap();
    mc_world::store_marker::write(
        &fixture.world,
        ours.store_uuid,
        mc_world::store_marker::ROOT_BRANCH,
        ours.checkpoint_tick + 1,
    )
    .unwrap();
    let out = tempfile::tempdir().unwrap();
    let error = census_native(
        &fixture.config,
        &fixture.world,
        &out.path().join("behind.json"),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("behind checkpoint marker"), "{error}");
}
