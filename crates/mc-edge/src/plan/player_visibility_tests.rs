use super::*;
use mc_core::dimensions::RosterPlayer;
use mc_protocol::packets::play::{ClientboundSetCamera, PlayerInfoUpdate};

fn roster(recipient_mode: u8, subject_mode: u8) -> Arc<[RosterPlayer]> {
    [
        (1, recipient_mode),
        (2, subject_mode),
        (4, 0),
        (6, 0),
        (7, 0),
    ]
    .into_iter()
    .map(|(id, game_mode)| RosterPlayer {
        connection: ConnectionId(id as u64),
        entity: EntityId(id),
        uuid: uuid::Uuid::from_u128(id as u128),
        name: format!("Player{id}"),
        game_mode,
    })
    .collect()
}

fn scene() -> TickOutput {
    let mut rows: Vec<_> = (1..=7)
        .map(|id| {
            let mut row = entity(id, [8.0, 64.0, 8.0]);
            row.type_id = player_entity_type_id();
            row
        })
        .collect();
    rows[2].type_id = mc_data::entity_types::entity_type_contract_26_1_2_by_name("minecraft:cow")
        .unwrap()
        .protocol_id;
    rows[4].type_id = rows[2].type_id;
    // Invisibility alone must not suppress ordinary players or mobs.
    for row in &mut rows[1..5] {
        Arc::make_mut(&mut row.appearance).shared_flags = 0x20;
    }
    rows[5].position[0] = 73.0; // Held chunk, but beyond this viewer's 64-block range.
    rows[6].position[0] = 24.0; // Nearby, but chunk (1, 0) is not held.
    let mut out = output(vec![], rows, ChangeSet::default());
    Arc::get_mut(&mut out.snapshot).unwrap().player_roster = roster(0, 0);
    out
}

fn viewer() -> ViewerState {
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    viewer.loaded_chunks.insert((0, 0), 0);
    viewer.loaded_chunks.insert((4, 0), 0);
    viewer
}

fn plan(out: &TickOutput, viewer: &mut ViewerState, frames: usize) -> Vec<PlannedPacket> {
    let index = TickPlanIndex::with_previous(out, viewer.previous_entities());
    let mut packets = Vec::new();
    plan_entities(
        &out.snapshot,
        viewer,
        4,
        &mut PlanScratch::default(),
        &index,
        &mut packets,
        &mut WorldCredit::frames(frames),
    );
    packets
}

fn adds(packets: &[PlannedPacket]) -> Vec<i32> {
    packets
        .iter()
        .filter(|p| p.id == AddEntity::ID)
        .map(|p| AddEntity::decode(&mut body_of(p)).unwrap().entity_id)
        .collect()
}

fn removes(packets: &[PlannedPacket]) -> Vec<i32> {
    packets
        .iter()
        .filter(|p| p.id == RemoveEntities::ID)
        .flat_map(|p| {
            assert!(!p.reliable, "tracking removals retain the world FIFO");
            RemoveEntities::decode(&mut body_of(p)).unwrap().entity_ids
        })
        .collect()
}

#[test]
fn spectator_index_matches_the_roster_scan_and_is_built_once_per_publish() {
    // The row's fix: the recipient/subject spectator answers come from the
    // publish-wide set built once, not from two roster scans per visible
    // player per viewer. This compares the indexed answer with the roster
    // scan for every (entity, recipient) pair of the mixed-mode fixture.
    let scan =
        |snapshot: &mc_core::boundary::WorldSnapshot, entity: &EntityView, recipient: EntityId| {
            if entity.id == recipient || entity.type_id != player_entity_type_id() {
                return true;
            }
            let spectator_viewer = snapshot
                .player_roster
                .iter()
                .any(|player| player.entity == recipient && player.game_mode == 3);
            if spectator_viewer {
                snapshot
                    .cameras
                    .get(&entity.id)
                    .is_none_or(|target| *target == entity.id)
            } else {
                !snapshot
                    .player_roster
                    .iter()
                    .any(|player| player.entity == entity.id && player.game_mode == 3)
            }
        };
    let before = tick_index::SPECTATOR_INDEX_BUILDS.load(std::sync::atomic::Ordering::Relaxed);
    let scans_before = entity_interest::ROSTER_SCANS.load(std::sync::atomic::Ordering::Relaxed);
    for (recipient_mode, subject_mode) in [(0, 0), (3, 0), (0, 3), (3, 3)] {
        let mut out = scene();
        {
            let snapshot = Arc::get_mut(&mut out.snapshot).unwrap();
            snapshot.player_roster = roster(recipient_mode, subject_mode);
            Arc::make_mut(&mut snapshot.cameras).insert(EntityId(2), EntityId(3));
        }
        let index = TickPlanIndex::with_previous(&out, None);
        let snapshot = &*out.snapshot;
        for entity in snapshot.entities.iter() {
            for &recipient in &[EntityId(1), EntityId(2), EntityId(4)] {
                assert_eq!(
                    entity_interest::broadcast_to(snapshot, entity, recipient, &index.spectators),
                    scan(snapshot, entity, recipient),
                    "recipient mode {recipient_mode}, subject mode {subject_mode}: {} -> {}",
                    entity.id.0,
                    recipient.0
                );
            }
        }
    }
    let builds =
        tick_index::SPECTATOR_INDEX_BUILDS.load(std::sync::atomic::Ordering::Relaxed) - before;
    assert_eq!(
        builds, 4,
        "one spectator index per publish, not one scan per viewer and entity"
    );
    assert_eq!(
        entity_interest::ROSTER_SCANS.load(std::sync::atomic::Ordering::Relaxed),
        scans_before,
        "broadcast_to answers from the publish set, not from a roster scan"
    );
}

#[test]
fn spectator_tracking_wire_matches_vanilla_recipient_and_subject_matrix() {
    for (recipient_mode, recipient_bound) in [(0, false), (3, false), (3, true)] {
        for (subject_mode, subject_bound) in [(0, false), (3, false), (3, true)] {
            let mut out = scene();
            let snapshot = Arc::get_mut(&mut out.snapshot).unwrap();
            snapshot.player_roster = roster(recipient_mode, subject_mode);
            if recipient_bound {
                Arc::make_mut(&mut snapshot.cameras).insert(EntityId(1), EntityId(3));
            }
            if subject_bound {
                Arc::make_mut(&mut snapshot.cameras).insert(EntityId(2), EntityId(3));
            }
            let mut viewer = viewer();
            let tab = plan_player_changes(
                &[PlayerChange::Roster {
                    players: Arc::clone(&snapshot.player_roster),
                    removed: Arc::from([]),
                }],
                &mut viewer,
            );
            assert!(
                tab.iter().any(|p| p.id == PlayerInfoUpdate::ID),
                "tab roster is not body visibility"
            );
            let packets = plan(&out, &mut viewer, 128);
            let added = adds(&packets);
            let expected = if recipient_mode == 3 {
                !subject_bound
            } else {
                subject_mode != 3
            };
            assert_eq!(
                added.contains(&2),
                expected,
                "recipient=({recipient_mode},{recipient_bound}) subject=({subject_mode},{subject_bound})"
            );
            assert!(!added.contains(&1), "self is never paired");
            assert!(
                packets
                    .iter()
                    .filter(|p| p.id == ClientboundSetEntityData::ID)
                    .any(|p| ClientboundSetEntityData::decode(&mut body_of(p))
                        .unwrap()
                        .entity_id
                        == 1),
                "self metadata survives any spectator camera state"
            );
            for id in [3, 4, 5] {
                assert!(
                    added.contains(&id),
                    "ordinary/invisible subject {id} remains trackable"
                );
            }
            assert!(!added.contains(&6), "range gate remains authoritative");
            assert!(!added.contains(&7), "unheld chunk remains authoritative");
        }
    }
}

#[test]
fn spectator_tracking_reconciles_mode_and_camera_changes_without_entity_revision() {
    let mut out = scene();
    let revision = out.snapshot.entities.revision();
    let mut viewer = viewer();
    assert!(adds(&plan(&out, &mut viewer, 128)).contains(&2));
    assert!(plan(&out, &mut viewer, 128).is_empty()); // Seed retained interest.

    Arc::get_mut(&mut out.snapshot).unwrap().player_roster = roster(0, 3);
    assert!(plan(&out, &mut viewer, 0).is_empty());
    assert!(
        viewer.tracked_entities.contains_key(&EntityId(2)),
        "zero-credit removal stays owed"
    );
    assert_eq!(removes(&plan(&out, &mut viewer, 128)), [2]);
    Arc::get_mut(&mut out.snapshot).unwrap().player_roster = roster(3, 3);
    assert!(plan(&out, &mut viewer, 0).is_empty());
    assert!(adds(&plan(&out, &mut viewer, 128)).contains(&2));

    // Recipient camera binding does not hide a free-camera subject.
    Arc::make_mut(&mut Arc::get_mut(&mut out.snapshot).unwrap().cameras)
        .insert(EntityId(1), EntityId(3));
    assert!(removes(&plan(&out, &mut viewer, 128)).is_empty());
    viewer.camera = Some(EntityId(2));
    Arc::make_mut(&mut Arc::get_mut(&mut out.snapshot).unwrap().cameras)
        .insert(EntityId(2), EntityId(3));
    let packets = plan(&out, &mut viewer, 128);
    assert_eq!(removes(&packets), [2]);
    let release = packets
        .iter()
        .position(|p| p.id == ClientboundSetCamera::ID)
        .unwrap();
    let remove = packets
        .iter()
        .position(|p| p.id == RemoveEntities::ID)
        .unwrap();
    assert!(
        release < remove,
        "lost target eligibility releases camera before removal"
    );
    assert_eq!(
        ClientboundSetCamera::decode(&mut body_of(&packets[release]))
            .unwrap()
            .entity_id,
        1
    );

    Arc::make_mut(&mut Arc::get_mut(&mut out.snapshot).unwrap().cameras)
        .insert(EntityId(2), EntityId(4));
    assert!(
        !adds(&plan(&out, &mut viewer, 128)).contains(&2),
        "rebind stays hidden"
    );
    Arc::make_mut(&mut Arc::get_mut(&mut out.snapshot).unwrap().cameras).remove(&EntityId(2));
    assert!(
        adds(&plan(&out, &mut viewer, 128)).contains(&2),
        "unbind re-pairs stationary subject"
    );
    Arc::get_mut(&mut out.snapshot).unwrap().player_roster = roster(0, 3);
    assert_eq!(
        removes(&plan(&out, &mut viewer, 128)),
        [2],
        "viewer mode exit removes free spectator"
    );
    Arc::get_mut(&mut out.snapshot).unwrap().player_roster = roster(0, 0);
    assert!(
        adds(&plan(&out, &mut viewer, 128)).contains(&2),
        "subject mode exit re-pairs"
    );
    assert_eq!(out.snapshot.entities.revision(), revision);
}
