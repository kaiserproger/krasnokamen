//! Reliable wire projection regressions over real entity revisions.
use super::*;
use crate::plan::{ChunkPacketCache, TickPlanIndex, plan_viewer};
use mc_core::blueprint::MovementClass;
use mc_core::boundary::{ChangeSet, EntityAppearance, PlayerChange, TickOutput};
use mc_core::entities::{ColdFields, EntityKind, EntitySlot, EntityStore};
use mc_domain::{ConnectionId, EntityId};
use mc_protocol::frame::{Compression, try_decode_frame};
use mc_protocol::packets::Packet;

struct World {
    store: EntityStore,
    players: Arc<Vec<(String, u16)>>,
    names: Arc<Vec<String>>,
    dimension: Dimension,
    tick: u64,
}

impl World {
    fn new() -> Self {
        Self {
            store: EntityStore::default(),
            players: Arc::default(),
            names: Arc::default(),
            dimension: Dimension::Overworld,
            tick: 0,
        }
    }

    fn spawn(&mut self, id: i32, faction: u16) -> EntitySlot {
        let husk = mc_data::entity_types::entity_type_contract_26_1_2_by_name("minecraft:husk")
            .expect("husk contract")
            .protocol_id;
        let slot = self.store.spawn(
            EntityId(id),
            Uuid::from_u128(id as u128),
            EntityKind::Mob { type_id: husk },
            ColdFields {
                pos: [f64::from(id % 64), 64.0, f64::from(id / 64)],
                vel: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                ground: true,
                health: 20.0,
                breed_cooldown: 0,
                pickup_delay: 0,
                item: None,
                appearance: Arc::new(EntityAppearance::default()),
                movement: MovementClass::Walker,
                half_width: 0.3,
                height: 1.95,
                speed: 0.23,
                attack_damage: 3.0,
                sight_range: 35.0,
                attack_reach: 1.0,
            },
        );
        self.store.set_faction(slot, faction);
        slot
    }

    fn remove(&mut self, id: i32) {
        let slot = self
            .store
            .slot_of_uuid(Uuid::from_u128(id as u128))
            .expect("spawned");
        self.store.remove(slot);
    }

    fn output(&mut self) -> TickOutput {
        self.tick += 1;
        TickOutput {
            tick: self.tick,
            snapshot: Arc::new(WorldSnapshot {
                tick: self.tick,
                dimension: self.dimension,
                day_time_rate: 1.0,
                entities: self.store.views(),
                player_factions: Arc::clone(&self.players),
                team_names: Arc::clone(&self.names),
                ..Default::default()
            }),
            changes: Arc::new(ChangeSet::default()),
            times: Default::default(),
        }
    }
}

fn entry(id: i32) -> String {
    Uuid::from_u128(id as u128).to_string()
}
fn viewer() -> ViewerState {
    ViewerState::new(ConnectionId(7), EntityId(-7), [0.0, 64.0, 0.0])
}

/// Decode every frame, including merged scoreboard batches. A projection
/// removal of individual members (4) is forbidden on every exercised path.
fn teams(packets: &[PlannedPacket]) -> Vec<ClientboundSetPlayerTeam> {
    let mut teams = Vec::new();
    for packet in packets {
        let mut framed = bytes::BytesMut::from(&packet.data[..]);
        while let Some(frame) = try_decode_frame(&mut framed, Compression::Threshold(256)).unwrap()
        {
            if frame.id != ClientboundSetPlayerTeam::ID {
                continue;
            }
            assert!(packet.reliable);
            let team = ClientboundSetPlayerTeam::decode(&mut frame.body.clone()).unwrap();
            assert!(matches!(team.method, CREATE | REMOVE | ADD), "{team:?}");
            teams.push(team);
        }
    }
    teams
}

fn plan(
    output: &TickOutput,
    previous: Option<&EntityViews>,
    viewer: &mut ViewerState,
) -> Vec<ClientboundSetPlayerTeam> {
    let index = TickPlanIndex::with_previous(output, previous);
    let mut out = Vec::new();
    plan_teams(&output.snapshot, index.faction_delta(), viewer, &mut out);
    teams(&out)
}

fn summary(packets: &[ClientboundSetPlayerTeam]) -> Vec<(String, u8, Vec<String>)> {
    packets
        .iter()
        .map(|packet| {
            let mut entries = packet.players.clone();
            entries.sort();
            (packet.name.clone(), packet.method, entries)
        })
        .collect()
}
fn sorted(entries: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut entries: Vec<_> = entries.into_iter().collect();
    entries.sort();
    entries
}

#[test]
fn join_creates_each_team_before_its_adds_with_explicit_parameters() {
    let mut world = World::new();
    for id in 1..=3 {
        world.spawn(id, 1);
    }
    world.spawn(4, 2);
    world.spawn(5, 0);
    world.players = Arc::new(vec![("PovRed".into(), 1)]);
    let packets = plan(&world.output(), None, &mut viewer());
    assert_eq!(
        summary(&packets),
        [
            ("sf 1".into(), CREATE, vec![]),
            (
                "sf 1".into(),
                ADD,
                sorted([entry(1), entry(2), entry(3), "PovRed".into()])
            ),
            ("sf 2".into(), CREATE, vec![]),
            ("sf 2".into(), ADD, vec![entry(4)]),
        ]
    );
    for (packet, color, display) in [(&packets[0], 12, "Red"), (&packets[2], 9, "Blue")] {
        let p = packet.parameters.as_ref().unwrap();
        assert_eq!(p.display_name, mc_nbt::Tag::String(display.into()));
        assert_eq!(
            (p.color, p.nametag_visibility, p.collision_rule, p.options),
            (color, 1, 0, 0b01),
            "nametag never, collision always, friendly fire only"
        );
        assert_eq!(p.prefix, mc_nbt::Tag::String(String::new()));
        assert_eq!(p.suffix, mc_nbt::Tag::String(String::new()));
    }
}

#[test]
fn adds_are_chunked_at_4096_entries() {
    let mut world = World::new();
    for id in 1..=9000 {
        world.spawn(id, 1);
    }
    let packets = plan(&world.output(), None, &mut viewer());
    assert_eq!((packets[0].method, packets[0].players.len()), (CREATE, 0));
    let counts: Vec<_> = packets[1..]
        .iter()
        .map(|p| {
            assert_eq!((p.name.as_str(), p.method), ("sf 1", ADD));
            p.players.len()
        })
        .collect();
    assert_eq!(counts, [4096, 4096, 808]);
}

#[test]
fn continuous_chain_sends_only_additions_and_never_method_four() {
    let mut world = World::new();
    let slots: Vec<_> = (1..=4).map(|id| world.spawn(id, 1)).collect();
    world.spawn(5, 2);
    let first = world.output();
    let (mut a, mut b) = (viewer(), viewer());
    plan(&first, None, &mut a);
    plan(&first, None, &mut b);
    world.store.set_faction(slots[0], 0);
    world.store.set_faction(slots[1], 2);
    world.spawn(6, 1);
    let next = world.output();
    assert!(
        TickPlanIndex::with_previous(&next, Some(&first.snapshot.entities))
            .faction_delta()
            .is_some()
    );
    let expected = [
        ("sf 1".into(), ADD, vec![entry(6)]),
        ("sf 2".into(), ADD, vec![entry(2)]),
    ];
    for viewer in [&mut a, &mut b] {
        assert_eq!(
            summary(&plan(&next, Some(&first.snapshot.entities), viewer)),
            expected
        );
        assert_eq!(viewer.teams.mobs.get(&Uuid::from_u128(1)), Some(&1));
        assert_eq!(viewer.teams.mobs.get(&Uuid::from_u128(2)), Some(&2));
    }
    let quiet = world.output();
    assert!(plan(&quiet, Some(&next.snapshot.entities), &mut a).is_empty());
}

#[test]
fn chain_break_reconciles_against_what_went_out() {
    let mut world = World::new();
    let slots: Vec<_> = (1..=4).map(|id| world.spawn(id, 1)).collect();
    let mut viewer = viewer();
    let first = world.output();
    plan(&first, None, &mut viewer);
    world.store.set_faction(slots[0], 2);
    world.store.set_faction(slots[2], 0);
    let skipped = world.output();
    world.store.set_faction(slots[1], 2);
    world.spawn(5, 1);
    world.remove(4);
    let latest = world.output();
    assert_eq!(
        summary(&plan(
            &latest,
            Some(&skipped.snapshot.entities),
            &mut viewer
        )),
        [
            ("sf 1".into(), ADD, vec![entry(5)]),
            ("sf 2".into(), CREATE, vec![]),
            ("sf 2".into(), ADD, sorted([entry(1), entry(2)])),
        ]
    );
    assert_eq!(viewer.teams.mobs.get(&Uuid::from_u128(3)), Some(&1));
    assert_eq!(viewer.teams.mobs.get(&Uuid::from_u128(4)), Some(&1));
}

#[test]
fn respawn_resends_nothing() {
    let mut world = World::new();
    let slot = world.spawn(1, 1);
    world.players = Arc::new(vec![("PovRed".into(), 1)]);
    let cache = ChunkPacketCache::new();
    let mut viewer = viewer();
    assert_eq!(
        teams(&plan_viewer(&world.output(), &mut viewer, 4, &cache)).len(),
        2
    );
    crate::plan::plan_player_changes(
        &[PlayerChange::Respawned {
            teleport_id: 1,
            dimension: Dimension::Overworld,
            position: [0.0, 64.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            game_mode: 0,
        }],
        &mut viewer,
    );
    world.store.set_pos(slot, [3.5, 64.0, 3.5]);
    for _ in 0..2 {
        assert!(teams(&plan_viewer(&world.output(), &mut viewer, 4, &cache)).is_empty());
    }
}

#[test]
fn teams_are_removed_by_current_data_not_stale_entries() {
    let mut world = World::new();
    world.spawn(1, 1);
    world.spawn(2, 1);
    world.players = Arc::new(vec![("PovRed".into(), 1)]);
    let mut viewer = viewer();
    let first = world.output();
    plan(&first, None, &mut viewer);
    world.remove(1);
    let second = world.output();
    assert!(plan(&second, Some(&first.snapshot.entities), &mut viewer).is_empty());
    world.remove(2);
    let third = world.output();
    assert!(
        plan(&third, Some(&second.snapshot.entities), &mut viewer).is_empty(),
        "holder keeps team"
    );
    world.players = Arc::default();
    let ended = world.output();
    assert_eq!(
        summary(&plan(&ended, Some(&third.snapshot.entities), &mut viewer)),
        [("sf 1".into(), REMOVE, vec![])]
    );
    assert!(viewer.teams.mobs.is_empty() && viewer.teams.players.is_empty());
    world.spawn(3, 1);
    assert_eq!(
        summary(&plan(&world.output(), None, &mut viewer)),
        [
            ("sf 1".into(), CREATE, vec![]),
            ("sf 1".into(), ADD, vec![entry(3)]),
        ]
    );
}

#[test]
fn reset_with_surviving_faction_never_removes_entries() {
    let mut world = World::new();
    world.spawn(1, 1);
    world.spawn(9, 1);
    world.players = Arc::new(vec![("PovRed".into(), 1)]);
    let mut viewer = viewer();
    let first = world.output();
    plan(&first, None, &mut viewer);
    world.remove(1);
    world.players = Arc::default();
    let reset = world.output();
    assert!(plan(&reset, Some(&first.snapshot.entities), &mut viewer).is_empty());
    assert_eq!(viewer.teams.players.get("PovRed"), Some(&1));
    world.players = Arc::new(vec![("PovRed".into(), 1)]);
    let again = world.output();
    assert!(plan(&again, Some(&reset.snapshot.entities), &mut viewer).is_empty());
    world.players = Arc::new(vec![("PovRed".into(), 2)]);
    let moved = world.output();
    assert_eq!(
        summary(&plan(&moved, Some(&again.snapshot.entities), &mut viewer)),
        [
            ("sf 2".into(), CREATE, vec![]),
            ("sf 2".into(), ADD, vec!["PovRed".into()]),
        ]
    );
}

fn route_join(viewer: &mut ViewerState, members: Vec<String>) -> Vec<ClientboundSetPlayerTeam> {
    let join = PlayerChange::Scoreboard {
        events: vec![mc_core::scoreboard::ScoreboardEvent::Team {
            name: "allies".into(),
            method: ADD,
            team: None,
            members,
        }]
        .into(),
    };
    teams(&crate::plan::plan_player_changes(&[join], viewer))
}

#[test]
fn core_join_moves_released_entries_without_projected_removals() {
    let mut world = World::new();
    world.spawn(0xab, 1);
    world.spawn(2, 1);
    world.players = Arc::new(vec![("PovRed".into(), 1)]);
    let mut viewer = viewer();
    let first = world.output();
    plan(&first, None, &mut viewer);
    let spelled = entry(0xab).to_uppercase();
    let members = vec!["PovRed".into(), spelled, "Other".into()];
    assert_eq!(
        summary(&route_join(&mut viewer, members.clone())),
        [("allies".into(), ADD, sorted(members))]
    );
    assert!(!viewer.teams.players.contains_key("PovRed"));
    assert!(!viewer.teams.mobs.contains_key(&Uuid::from_u128(0xab)));
    world.players = Arc::default();
    world.remove(0xab);
    let ended = world.output();
    assert!(plan(&ended, Some(&first.snapshot.entities), &mut viewer).is_empty());
    world.remove(2);
    let gone = world.output();
    assert_eq!(
        summary(&plan(&gone, Some(&ended.snapshot.entities), &mut viewer)),
        [("sf 1".into(), REMOVE, vec![])]
    );
}

#[test]
fn released_holder_only_team_is_removed_when_hold_ends() {
    let mut world = World::new();
    world.players = Arc::new(vec![("PovRed".into(), 1)]);
    let mut viewer = viewer();
    let first = world.output();
    assert_eq!(plan(&first, None, &mut viewer).len(), 2);
    route_join(&mut viewer, vec!["PovRed".into()]);
    world.players = Arc::default();
    assert_eq!(
        summary(&plan(
            &world.output(),
            Some(&first.snapshot.entities),
            &mut viewer
        )),
        [("sf 1".into(), REMOVE, vec![])]
    );
}

#[test]
fn blocked_factions_send_neither_adds_nor_member_removals() {
    let mut world = World::new();
    let slot = world.spawn(1, 1);
    world.spawn(2, 1);
    world.names = Arc::new(vec!["sf 2".into()]);
    let mut viewer = viewer();
    let first = world.output();
    plan(&first, None, &mut viewer);
    world.store.set_faction(slot, 2);
    let moved = world.output();
    assert!(plan(&moved, Some(&first.snapshot.entities), &mut viewer).is_empty());
    assert_eq!(viewer.teams.mobs.get(&Uuid::from_u128(1)), Some(&1));
    world.names = Arc::new(vec!["sf 1".into()]);
    let freed = world.output();
    assert_eq!(
        summary(&plan(&freed, Some(&moved.snapshot.entities), &mut viewer)),
        [
            ("sf 2".into(), CREATE, vec![]),
            ("sf 2".into(), ADD, vec![entry(1)]),
        ],
        "core owns sf 1: never remove it; freed sf 2 syncs"
    );
}

#[test]
fn dimension_change_removes_teams_then_syncs_again() {
    let mut world = World::new();
    world.spawn(1, 1);
    let mut viewer = viewer();
    plan(&world.output(), None, &mut viewer);
    let mut nether = World::new();
    nether.dimension = Dimension::Nether;
    nether.spawn(2, 2);
    assert_eq!(
        summary(&plan(&nether.output(), None, &mut viewer)),
        [
            ("sf 1".into(), REMOVE, vec![]),
            ("sf 2".into(), CREATE, vec![]),
            ("sf 2".into(), ADD, vec![entry(2)]),
        ]
    );
}

#[test]
fn restart_has_rows_only_and_ordinary_team_names_do_not_block() {
    let mut world = World::new();
    world.spawn(1, 1);
    world.names = Arc::new(vec!["sf:1".into(), "sf 01".into()]);
    assert_eq!(
        summary(&plan(&world.output(), None, &mut viewer())),
        [
            ("sf 1".into(), CREATE, vec![]),
            ("sf 1".into(), ADD, vec![entry(1)]),
        ]
    );
}
