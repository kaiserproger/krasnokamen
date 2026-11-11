//! Two raw clients of different releases on one server: each must receive the
//! other's player-list entry and player entity, whichever joined first.
use super::*;
use mc_protocol::packets::play::{
    AddEntity, PlayerInfoActions, PlayerInfoRemove, PlayerInfoUpdate, RemoveEntities,
};
use std::sync::atomic::{AtomicBool, Ordering};

/// What one client has been sent about another player, applied like the
/// vanilla client: a player entity added before its list entry is discarded
/// ("Server attempted to add player prior to sending player info").
#[derive(Default, Debug)]
struct Seen {
    listed: bool,
    entity: Option<i32>,
}

/// Plays `viewer` until it holds both the player-list entry and a player
/// entity for `subject`.
async fn sees(
    viewer: &mut Client,
    subject: &str,
    player_type: i32,
    mine: &AtomicBool,
    other: &AtomicBool,
) {
    let uuid = crate::crypto::offline_uuid(subject);
    let mut seen = Seen::default();
    tokio::time::timeout(
        Duration::from_secs(150),
        viewer.play_until(|client, frame| {
            let mut body = frame.body.as_ref();
            if frame.id == client.id("play", true, "minecraft:player_info_update") {
                let update = PlayerInfoUpdate::decode(&mut body).unwrap();
                seen.listed |= update.actions.contains(PlayerInfoActions::ADD_PLAYER)
                    && update.entries.iter().any(|entry| entry.profile_id == uuid);
            } else if frame.id == client.id("play", true, "minecraft:player_info_remove") {
                if PlayerInfoRemove::decode(&mut body)
                    .unwrap()
                    .profile_ids
                    .contains(&uuid)
                {
                    seen.listed = false;
                    seen.entity = None;
                }
            } else if frame.id == client.id("play", true, "minecraft:add_entity") {
                let add = AddEntity::decode(&mut body).unwrap();
                if add.uuid == uuid && add.entity_type_id == player_type && seen.listed {
                    seen.entity = Some(add.entity_id);
                }
            } else if frame.id == client.id("play", true, "minecraft:remove_entities") {
                let removed = RemoveEntities::decode(&mut body).unwrap();
                if seen
                    .entity
                    .is_some_and(|id| removed.entity_ids.contains(&id))
                {
                    seen.entity = None;
                }
            }
            if seen.listed && seen.entity.is_some() {
                mine.store(true, Ordering::SeqCst);
            }
            // Keep answering keep-alives until the other viewer is done too.
            (mine.load(Ordering::SeqCst) && other.load(Ordering::SeqCst)).then_some(())
        }),
    )
    .await
    .ok();
    assert!(
        mine.load(Ordering::SeqCst),
        "{} client never saw {subject} ({seen:?})",
        viewer.profile.version.release
    );
}

/// `first` joins, settles in Play, then `second` joins; both must see each other.
async fn mutual_visibility(first: i32, second: i32) {
    let dir = tempfile::tempdir().unwrap();
    let server = server(dir.path());
    let first_profile = peer_profile(&server, first);
    let second_profile = peer_profile(&server, second);
    let player_type = |protocol| {
        let registries: serde_json::Value = serde_json::from_slice(
            &std::fs::read(peer_root(&server, protocol).join("reports/registries.json")).unwrap(),
        )
        .unwrap();
        registries["minecraft:entity_type"]["entries"]["minecraft:player"]["protocol_id"]
            .as_i64()
            .unwrap() as i32
    };
    let (first_player, second_player) = (player_type(first), player_type(second));
    let hub = Arc::clone(&server.hub);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let serve = tokio::spawn(crate::server::serve(listener, server));
    tokio::time::timeout(Duration::from_secs(600), async {
        let mut early = Client::join(address, "Early", first_profile, true).await;
        early
            .play_until(|client, frame| {
                (frame.id == client.id("play", true, "minecraft:player_position")).then_some(())
            })
            .await;
        let mut late = Client::join(address, "Late", second_profile, true).await;
        // Both are read until both are satisfied so each keeps answering
        // keep-alives: an unread client is kicked after 30 s and its player-list
        // entry removed, which would look like a visibility failure.
        let (early_done, late_done) = (AtomicBool::new(false), AtomicBool::new(false));
        tokio::join!(
            sees(&mut late, "Early", second_player, &late_done, &early_done),
            sees(&mut early, "Late", first_player, &early_done, &late_done)
        );
    })
    .await
    .expect("mixed-version join within the deadline");
    hub.shutdown();
    serve.abort();
}

macro_rules! visibility {
    ($name:ident, $first:literal, $second:literal) => {
        #[ignore = "requires imported real caches and step snapshots"]
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn $name() {
            mutual_visibility($first, $second).await;
        }
    };
}

visibility!(protocol_775_then_776_see_each_other, 775, 776);
visibility!(protocol_776_then_775_see_each_other, 776, 775);
visibility!(protocol_775_then_777_see_each_other, 775, 777);
visibility!(protocol_777_then_775_see_each_other, 777, 775);
visibility!(protocol_776_then_777_see_each_other, 776, 777);
visibility!(protocol_777_then_776_see_each_other, 777, 776);
