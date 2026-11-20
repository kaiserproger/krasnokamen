//! Durable custody census over checkpoint rows plus the published WAL tail.
use std::sync::Arc;

use mc_core::dimensions::Dimension;
use mc_core::journal::{JournalEntry, JournalWriter, ReplayEntity};

use crate::census_tests::{
    EntityFixture, Fixture, TransitFixture, fields, member, transit_row, world_with,
};
use crate::custody_census;
use crate::recovery_roundtrip_tests::entity_entry;

const LEAD: u128 = 0x10;
const LONE: u128 = 0x11;

fn uuid(n: u128) -> String {
    uuid::Uuid::from_u128(n).to_string()
}

/// Villager `LEAD` (wire 10, household 7-1) and lone resident `LONE`
/// (wire 11) of persisted settlement 7, plus `transits` rows.
fn town(transits: &[TransitFixture]) -> Fixture {
    let registry = crate::test_support::registry().unwrap();
    let villagers: Vec<EntityFixture> = vec![
        (
            0,
            0,
            0,
            LEAD,
            member(
                fields(&registry, "minecraft:villager", LEAD, 10),
                7,
                Some("7-1"),
            ),
        ),
        (
            0,
            0,
            0,
            LONE,
            member(fields(&registry, "minecraft:villager", LONE, 11), 7, None),
        ),
    ];
    world_with(villagers, true, transits)
}

/// The WAL image of a record holding `members` (the checkpoint row's value).
fn image(members: &[u128], household: Option<&str>) -> Arc<String> {
    let row: serde_json::Value = serde_json::from_slice(&transit_row(members, household)).unwrap();
    Arc::new(row["value"].to_string())
}

fn transit(id: u128, record: Option<Arc<String>>, tick: u64) -> JournalEntry {
    JournalEntry::Transit {
        id: uuid::Uuid::from_u128(id),
        record,
        tick,
    }
}

fn append(fixture: &Fixture, tick: u64, entries: &[JournalEntry]) {
    let mut writer = JournalWriter::open(
        &fixture.world,
        1,
        mc_core::journal::test_wal_identity(&fixture.world),
    )
    .unwrap();
    writer.append(tick, tick, entries).unwrap();
    writer.flush().unwrap();
}

fn stored(fixture: &Fixture, wire: i32) -> ReplayEntity {
    let world = mc_core::native_checkpoint::recover_offline(&fixture.config).unwrap();
    world.dimensions[&Dimension::Overworld].entities[&wire].clone()
}

#[test]
fn custody_census_follows_a_departure_and_an_arrival_through_the_wal() {
    let fixture = town(&[]);
    let lone = stored(&fixture, 11);
    let tick = fixture.tick;

    // Departure tick: the body leaves and the record appears in one batch.
    append(
        &fixture,
        tick + 1,
        &[
            JournalEntry::RemoveEntity {
                id: 11,
                tick: tick + 1,
            },
            transit(LONE, Some(image(&[LONE], None)), tick + 1),
        ],
    );
    let receipt = custody_census(&fixture.config);
    assert_eq!(receipt.status, "PASS", "{:?}", receipt.error);
    assert_eq!(receipt.checkpoint_tick, Some(tick));
    assert_eq!(receipt.durable_tick, Some(tick + 1));
    assert!(!receipt.villagers.contains_key(&uuid(LONE)));
    assert_eq!(
        receipt.villagers[&uuid(LEAD)].household.as_deref(),
        Some("7-1")
    );
    let record = &receipt.transits[&uuid(LONE)];
    assert_eq!(record.members, [uuid(LONE)]);
    assert_eq!((record.source, record.destination), (7, 8));

    // Arrival tick: the record is deleted and the same UUID re-enters.
    let mut arrived = lone;
    arrived.tick = tick + 2;
    if let Some(villager) = arrived.villager.as_mut() {
        villager.settlement = Some(mc_core::settlements::SettlementId(8));
    }
    append(
        &fixture,
        tick + 2,
        &[transit(LONE, None, tick + 2), entity_entry(99, arrived)],
    );
    let receipt = custody_census(&fixture.config);
    assert_eq!(receipt.status, "PASS", "{:?}", receipt.error);
    assert_eq!(receipt.durable_tick, Some(tick + 2));
    assert!(receipt.transits.is_empty());
    let back = &receipt.villagers[&uuid(LONE)];
    assert_eq!((back.wire_id, back.settlement), (99, Some(8)));
}

#[test]
fn custody_census_reads_a_checkpointed_transit_row() {
    let fixture = town(&[(0x40, &[0x40, 0x41], Some("7-2"))]);
    let receipt = custody_census(&fixture.config);
    assert_eq!(receipt.status, "PASS", "{:?}", receipt.error);
    let record = &receipt.transits[&uuid(0x40)];
    assert_eq!(record.members, [uuid(0x40), uuid(0x41)]);
    assert_eq!(record.household.as_deref(), Some("7-2"));
    assert_eq!(receipt.villagers.len(), 2);
}

/// Negative controls: each custody violation at the durable boundary is a FAIL.
#[test]
fn custody_census_refuses_a_uuid_held_twice() {
    let cases: [(Vec<TransitFixture>, Vec<JournalEntry>, &str); 6] = [
        // Body kept while its record was published (body + record).
        (
            vec![],
            vec![transit(LONE, Some(image(&[LONE], None)), 0)],
            "member 00000000-0000-0000-0000-000000000011 of transit record",
        ),
        // A checkpointed record whose member is still a stored body.
        (
            vec![(LONE, &[LONE], None)],
            vec![],
            "is also a world entity",
        ),
        // One member in two records (row plus WAL image).
        (
            vec![(0x40, &[0x40, 0x41], None)],
            vec![transit(0x41, Some(image(&[0x41], None)), 0)],
            "member 00000000-0000-0000-0000-000000000041 is held by transit records",
        ),
        // The departed household still belongs to its settlement.
        (
            vec![(0x40, &[0x40], Some("7-1"))],
            vec![],
            "household 7-1 is in transit record",
        ),
        // A record not keyed by its lowest member.
        (
            vec![(0x41, &[0x40, 0x41], None)],
            vec![],
            "is not keyed by its lowest member",
        ),
        // One household in two records with disjoint members.
        (
            vec![(0x40, &[0x40], Some("7-2")), (0x41, &[0x41], Some("7-2"))],
            vec![],
            "household 7-2 is in transit records 00000000-0000-0000-0000-000000000040 and \
             00000000-0000-0000-0000-000000000041",
        ),
    ];
    for (rows, wal, detail) in cases {
        let fixture = town(&rows);
        if !wal.is_empty() {
            let tick = fixture.tick + 1;
            let wal: Vec<_> = wal
                .into_iter()
                .map(|entry| match entry {
                    JournalEntry::Transit { id, record, .. } => {
                        JournalEntry::Transit { id, record, tick }
                    }
                    entry => entry,
                })
                .collect();
            append(&fixture, tick, &wal);
        }
        let receipt = custody_census(&fixture.config);
        assert_eq!(receipt.status, "FAIL", "{detail}");
        let error = receipt.error.unwrap_or_default();
        assert!(error.contains(detail), "missing {detail:?} in {error}");
    }
}

/// A body that re-enters under a new wire id while the old body survives:
/// the recovery fold the census reads refuses it, so the census fails.
#[test]
fn custody_census_refuses_an_entity_uuid_stored_twice() {
    let fixture = town(&[]);
    let mut copy = stored(&fixture, 11);
    copy.tick = fixture.tick + 1;
    append(&fixture, fixture.tick + 1, &[entity_entry(99, copy)]);
    let receipt = custody_census(&fixture.config);
    assert_eq!(receipt.status, "FAIL");
    let error = receipt.error.unwrap_or_default();
    assert!(
        error.contains(
            "duplicate entity identity WireId 99 UUID 00000000-0000-0000-0000-000000000011"
        ),
        "{error}"
    );
}

/// Transit images fold per dimension, as recovery hydrates them: a Nether
/// image of an Overworld row's id is a second record (double custody), and a
/// Nether tombstone leaves the Overworld record in place.
#[test]
fn custody_census_keeps_dimensions_apart() {
    let nether = |entry: JournalEntry| JournalEntry::Dimension {
        dimension: Dimension::Nether,
        entries: vec![entry],
    };
    let fixture = town(&[(0x40, &[0x40, 0x41], None)]);
    let tick = fixture.tick + 1;
    append(
        &fixture,
        tick,
        &[nether(transit(
            0x40,
            Some(image(&[0x40, 0x41], None)),
            tick,
        ))],
    );
    let receipt = custody_census(&fixture.config);
    assert_eq!(receipt.status, "FAIL");
    let error = receipt.error.unwrap_or_default();
    assert!(
        error.contains(
            "member 00000000-0000-0000-0000-000000000040 is held by transit records \
             00000000-0000-0000-0000-000000000040 (minecraft:overworld) and \
             00000000-0000-0000-0000-000000000040 (minecraft:the_nether)"
        ),
        "{error}"
    );

    let fixture = town(&[(0x40, &[0x40, 0x41], None)]);
    let tick = fixture.tick + 1;
    append(&fixture, tick, &[nether(transit(0x40, None, tick))]);
    let receipt = custody_census(&fixture.config);
    assert_eq!(receipt.status, "PASS", "{:?}", receipt.error);
    assert_eq!(
        receipt.transits[&uuid(0x40)].dimension,
        "minecraft:overworld"
    );
}
