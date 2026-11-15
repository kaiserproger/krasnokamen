use super::*;
use mc_protocol::packets::play::ClientboundSetCamera;

fn scene(target: Option<i32>, bound: Option<i32>, x: f64) -> TickOutput {
    let chunk = chunk_at((x.floor() as i32).div_euclid(16), 0);
    let mut rows = vec![entity(1, [x, 64.0, 8.0])];
    if let Some(id) = target {
        rows.push(entity(id, [x, 64.0, 8.0]));
    }
    let mut out = output(vec![chunk], rows, ChangeSet::default());
    Arc::get_mut(&mut out.snapshot).unwrap().cameras = Arc::new(
        bound
            .map(|id| (EntityId(1), EntityId(id)))
            .into_iter()
            .collect(),
    );
    out
}

fn cameras(packets: &[PlannedPacket]) -> Vec<i32> {
    packets
        .iter()
        .filter(|p| p.id == ClientboundSetCamera::ID)
        .map(|p| {
            ClientboundSetCamera::decode(&mut body_of(p))
                .unwrap()
                .entity_id
        })
        .collect()
}

#[test]
fn camera_binding_follows_pairing_and_release_precedes_target_removal() {
    let cache = ChunkPacketCache::new();
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    let first = plan_viewer(&scene(Some(2), Some(2), 8.0), &mut viewer, 1, &cache);
    assert_eq!(cameras(&first), [2]);
    let add = first.iter().position(|p| p.id == AddEntity::ID).unwrap();
    let camera = first
        .iter()
        .position(|p| p.id == ClientboundSetCamera::ID)
        .unwrap();
    assert!(add < camera);
    assert!(!first[camera].reliable, "camera shares the spawn FIFO");
    let moved = plan_viewer(&scene(Some(2), Some(2), 40.0), &mut viewer, 1, &cache);
    assert_eq!(viewer.position, [40.0, 64.0, 8.0]);
    assert_eq!(viewer.center_sent, Some((2, 0)));
    // Moving between held chunks can remove/re-pair a body; every bind is still ordered.
    assert!(cameras(&moved).iter().all(|id| [1, 2].contains(id)));
    let died = plan_viewer(&scene(None, None, 40.0), &mut viewer, 1, &cache);
    assert_eq!(cameras(&died), [1]);
    assert!(
        died.iter()
            .position(|p| p.id == ClientboundSetCamera::ID)
            .unwrap()
            < died
                .iter()
                .position(|p| p.id == RemoveEntities::ID)
                .unwrap()
    );
    for id in [2, 3] {
        let rebound = plan_viewer(&scene(Some(id), Some(id), 40.0), &mut viewer, 1, &cache);
        assert!(cameras(&rebound).contains(&id));
        let add = rebound.iter().position(|p| p.id == AddEntity::ID).unwrap();
        let camera = rebound
            .iter()
            .rposition(|p| p.id == ClientboundSetCamera::ID)
            .unwrap();
        assert!(add < camera, "respawn/reconnect id={id}");
    }
}

#[test]
fn camera_never_binds_untracked_target_and_debt_survives_full_or_lost_queue() {
    let cache = ChunkPacketCache::new();
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    let missing = plan_viewer(&scene(None, Some(2), 8.0), &mut viewer, 1, &cache);
    assert!(cameras(&missing).is_empty());
    let available = scene(Some(2), Some(2), 8.0);
    viewer.world_credit = 0;
    assert!(cameras(&plan_viewer(&available, &mut viewer, 1, &cache)).is_empty());
    viewer.world_credit = usize::MAX;
    assert_eq!(
        cameras(&plan_viewer(&available, &mut viewer, 1, &cache)),
        [2]
    );
    viewer.world_frame_unsent();
    let repair = plan_viewer(&available, &mut viewer, 1, &cache);
    assert_eq!(cameras(&repair), [1, 2]);
    let add = repair.iter().position(|p| p.id == AddEntity::ID).unwrap();
    let bind = repair
        .iter()
        .rposition(|p| p.id == ClientboundSetCamera::ID)
        .unwrap();
    assert!(add < bind);
    viewer.world_credit = 0;
    let stopped = scene(None, None, 8.0);
    let blocked = plan_viewer(&stopped, &mut viewer, 1, &cache);
    assert!(cameras(&blocked).is_empty());
    assert!(!blocked.iter().any(|p| p.id == RemoveEntities::ID));
    viewer.world_credit = usize::MAX;
    let released = plan_viewer(&stopped, &mut viewer, 1, &cache);
    assert_eq!(cameras(&released), [1]);
    assert!(
        released
            .iter()
            .position(|p| p.id == ClientboundSetCamera::ID)
            .unwrap()
            < released
                .iter()
                .position(|p| p.id == RemoveEntities::ID)
                .unwrap()
    );
}

#[test]
fn camera_releases_before_respawn_forgets_entities() {
    let cache = ChunkPacketCache::new();
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    plan_viewer(&scene(Some(2), Some(2), 8.0), &mut viewer, 1, &cache);
    viewer.forget_all = true;
    let packets = plan_viewer(&scene(Some(2), None, 8.0), &mut viewer, 1, &cache);
    assert_eq!(cameras(&packets), [1]);
    assert!(
        packets
            .iter()
            .position(|p| p.id == ClientboundSetCamera::ID)
            .unwrap()
            < packets
                .iter()
                .position(|p| p.id == RemoveEntities::ID)
                .unwrap()
    );
}

#[test]
fn camera_profile_replacement_releases_removes_pairs_then_rebinds_on_world_fifo() {
    let cache = ChunkPacketCache::new();
    let mut viewer = viewer_at(1, [8.0, 64.0, 8.0]);
    let out = scene(Some(2), Some(2), 8.0);
    plan_viewer(&out, &mut viewer, 1, &cache);
    let roster = PlayerChange::Roster {
        players: Arc::from([mc_core::dimensions::RosterPlayer {
            connection: ConnectionId(2),
            entity: EntityId(2),
            uuid: uuid::Uuid::from_u128(2),
            name: "PovRed".into(),
            game_mode: 0,
        }]),
        removed: Arc::from([]),
    };
    let reliable = plan_player_changes(&[roster], &mut viewer);
    assert!(
        !reliable
            .iter()
            .any(|packet| packet.id == RemoveEntities::ID)
    );
    viewer.world_credit = 0;
    let blocked = plan_viewer(&out, &mut viewer, 1, &cache);
    assert!(!blocked.iter().any(|packet| packet.id == RemoveEntities::ID));
    viewer.world_credit = usize::MAX;
    let repaired = plan_viewer(&out, &mut viewer, 1, &cache);
    assert_eq!(cameras(&repaired), [1, 2]);
    let release = repaired
        .iter()
        .position(|p| p.id == ClientboundSetCamera::ID)
        .unwrap();
    let remove = repaired
        .iter()
        .position(|p| p.id == RemoveEntities::ID)
        .unwrap();
    let add = repaired.iter().position(|p| p.id == AddEntity::ID).unwrap();
    let bind = repaired
        .iter()
        .rposition(|p| p.id == ClientboundSetCamera::ID)
        .unwrap();
    assert!(release < remove && remove < add && add < bind);
    assert!(
        [release, remove, add, bind]
            .into_iter()
            .all(|i| !repaired[i].reliable)
    );
}
