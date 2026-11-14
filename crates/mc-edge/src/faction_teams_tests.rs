//! Raw-protocol tests for [`super::plan_teams`] over a real `EntityStore`,
//! whose `views()` keeps the revision chain (`base_revision`, `changed`) the
//! shared delta reads.

use super::*;
use crate::plan::{ChunkPacketCache, PlannedPacket, TickPlanIndex, ViewerState, plan_viewer};
use mc_core::blueprint::MovementClass;
use mc_core::boundary::{
    ChangeSet, ConnectionId, EntityAppearance, EntityId, PlayerChange, TickOutput,
};
use mc_core::entities::{ColdFields, EntityKind, EntitySlot, EntityStore};
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
            uuid::Uuid::from_u128(id as u128),
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

    /// Remove by id: the store swap-removes, so earlier slots move.
    fn remove(&mut self, id: i32) {
        let slot = self
            .store
            .slot_of_uuid(uuid::Uuid::from_u128(id as u128))
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
    uuid::Uuid::from_u128(id as u128).to_string()
}

fn viewer() -> ViewerState {
    ViewerState::new(ConnectionId(7), EntityId(-7), [0.0, 64.0, 0.0])
}

/// The decoded team packets among `packets`; each is reliable and is a
/// create, a team removal, an add or an entry removal. (The player-change
/// path merges a scoreboard batch into one packet of several frames, so
/// every frame is decoded.)
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
            assert!(
                matches!(team.method, CREATE | REMOVE | ADD | REMOVE_MEMBERS),
                "{team:?}"
            );
            teams.push(team);
        }
    }
    teams
}

/// The vanilla client's team bookkeeping (javap 26.1.2 and 26.3,
/// `handleSetPlayerTeamPacket`): a create of an existing team merges, an add
/// moves the entry out of its other team, an unknown team is ignored, and
/// removing an entry the team does not hold throws (a disconnect).
#[derive(Default)]
struct Client {
    teams: BTreeMap<String, std::collections::BTreeSet<String>>,
}

impl Client {
    fn apply(&mut self, packets: &[ClientboundSetPlayerTeam]) {
        for packet in packets {
            let name = &packet.name;
            match packet.method {
                CREATE => {
                    self.teams.entry(name.clone()).or_default();
                    self.add(name, &packet.players);
                }
                REMOVE => {
                    self.teams.remove(name);
                }
                ADD if self.teams.contains_key(name) => self.add(name, &packet.players),
                REMOVE_MEMBERS => {
                    if let Some(team) = self.teams.get_mut(name) {
                        for entry in &packet.players {
                            assert!(
                                team.remove(entry),
                                "client throws: {entry} is not in {name}"
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn add(&mut self, name: &str, entries: &[String]) {
        for entry in entries {
            for team in self.teams.values_mut() {
                team.remove(entry);
            }
            self.teams.get_mut(name).unwrap().insert(entry.clone());
        }
    }

    fn team_of(&self, entry: &str) -> Option<&str> {
        self.teams
            .iter()
            .find(|(_, members)| members.contains(entry))
            .map(|(name, _)| name.as_str())
    }
}

/// Plan the projection alone. `previous` is the snapshot the fanout builds
/// the publish's shared index from (any viewer's last one).
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

/// `(team, method, sorted entries)` of each packet.
fn summary(packets: &[ClientboundSetPlayerTeam]) -> Vec<(String, u8, Vec<String>)> {
    packets
        .iter()
        .map(|team| {
            let mut players = team.players.clone();
            players.sort();
            (team.name.clone(), team.method, players)
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
    world.spawn(5, 2);
    world.spawn(6, 0);
    world.players = Arc::new(vec![("PovRed".to_owned(), 1)]);
    let packets = plan(&world.output(), None, &mut viewer());
    assert_eq!(
        summary(&packets),
        [
            ("sf 1".to_owned(), CREATE, vec![]),
            (
                "sf 1".to_owned(),
                ADD,
                sorted([entry(1), entry(2), entry(3), "PovRed".to_owned()])
            ),
            ("sf 2".to_owned(), CREATE, vec![]),
            ("sf 2".to_owned(), ADD, sorted([entry(4), entry(5)])),
        ]
    );
    let red = packets[0].parameters.as_ref().unwrap();
    assert_eq!(red.display_name, mc_nbt::Tag::String("Red".into()));
    assert_eq!(
        (
            red.color,
            red.nametag_visibility,
            red.collision_rule,
            red.options
        ),
        (12, 1, 0, 0b01),
        "red, nametag never, collision always, friendly fire only"
    );
    assert_eq!(red.prefix, mc_nbt::Tag::String(String::new()));
    assert_eq!(red.suffix, mc_nbt::Tag::String(String::new()));
    let blue = packets[2].parameters.as_ref().unwrap();
    assert_eq!(blue.display_name, mc_nbt::Tag::String("Blue".into()));
    assert_eq!((blue.color, blue.nametag_visibility), (9, 1));
}

#[test]
fn adds_are_chunked_at_4096_entries() {
    let mut world = World::new();
    for id in 1..=9000 {
        world.spawn(id, 1);
    }
    let packets = plan(&world.output(), None, &mut viewer());
    assert_eq!(packets[0].method, CREATE);
    let adds: Vec<usize> = packets[1..]
        .iter()
        .map(|team| {
            assert_eq!((team.name.as_str(), team.method), ("sf 1", ADD));
            team.players.len()
        })
        .collect();
    assert_eq!(adds, [4096, 4096, 808]);
}

/// A continuous revision step reads the one shared delta: only rows whose
/// faction changed go out, an entry whose faction went to zero leaves its
/// team (method 4) and the baseline, one moving to another faction is only
/// added there, and two viewers get the same packets.
#[test]
fn continuous_chain_sends_only_the_shared_delta() {
    let mut world = World::new();
    let slots: Vec<_> = (1..=6)
        .map(|id| world.spawn(id, if id <= 3 { 1 } else { 0 }))
        .collect();
    world.spawn(7, 2);
    let first = world.output();
    let (mut a, mut b) = (viewer(), viewer());
    let mut client = Client::default();
    client.apply(&plan(&first, None, &mut a));
    plan(&first, None, &mut b);
    world.store.set_faction(slots[0], 0);
    world.store.set_faction(slots[1], 2);
    world.store.set_faction(slots[3], 2);
    world.spawn(8, 1);
    let second = world.output();
    let index = TickPlanIndex::with_previous(&second, Some(&first.snapshot.entities));
    assert!(index.faction_delta().is_some(), "continuous step");
    let expected = [
        ("sf 1".to_owned(), REMOVE_MEMBERS, vec![entry(1)]),
        ("sf 1".to_owned(), ADD, vec![entry(8)]),
        ("sf 2".to_owned(), ADD, sorted([entry(2), entry(4)])),
    ];
    let sent = plan(&second, Some(&first.snapshot.entities), &mut a);
    assert_eq!(summary(&sent), expected);
    client.apply(&sent);
    assert_eq!(client.team_of(&entry(1)), None);
    assert_eq!(client.team_of(&entry(2)), Some("sf 2"));
    assert!(!a.teams.mobs.contains_key(&Uuid::from_u128(1)));
    assert_eq!(a.teams.mobs.get(&Uuid::from_u128(2)), Some(&2));
    assert_eq!(
        summary(&plan(&second, Some(&first.snapshot.entities), &mut b)),
        expected
    );
    let quiet = world.output();
    assert!(plan(&quiet, Some(&second.snapshot.entities), &mut a).is_empty());
}

/// A viewer whose last plan was two revisions back reconciles its baseline
/// with the snapshot, not with the publish's shared base: rows that left
/// their faction (set to zero, gone) leave their team, moved and new rows are
/// added.
#[test]
fn chain_break_reconciles_against_what_went_out() {
    let mut world = World::new();
    let slots: Vec<_> = (1..=4).map(|id| world.spawn(id, 1)).collect();
    let mut viewer = viewer();
    let mut client = Client::default();
    let first = world.output();
    client.apply(&plan(&first, None, &mut viewer));
    world.store.set_faction(slots[0], 2);
    world.store.set_faction(slots[2], 0);
    let skipped = world.output();
    world.store.set_faction(slots[1], 2);
    world.spawn(5, 1);
    world.remove(4);
    let latest = world.output();
    let index = TickPlanIndex::with_previous(&latest, Some(&skipped.snapshot.entities));
    assert!(index.faction_delta().is_some());
    let sent = plan(&latest, Some(&skipped.snapshot.entities), &mut viewer);
    assert_eq!(
        summary(&sent),
        [
            (
                "sf 1".to_owned(),
                REMOVE_MEMBERS,
                sorted([entry(3), entry(4)])
            ),
            ("sf 1".to_owned(), ADD, vec![entry(5)]),
            ("sf 2".to_owned(), CREATE, vec![]),
            ("sf 2".to_owned(), ADD, sorted([entry(1), entry(2)])),
        ]
    );
    client.apply(&sent);
    assert_eq!(client.team_of(&entry(3)), None);
    assert_eq!(client.team_of(&entry(1)), Some("sf 2"));
    assert_eq!(viewer.teams.mobs.len(), 3);
}

/// The vanilla client keeps its scoreboard across a respawn, so the full
/// planner sends nothing again after a same-dimension death respawn.
#[test]
fn respawn_resends_nothing() {
    let mut world = World::new();
    let slots: Vec<_> = (1..=3).map(|id| world.spawn(id, 1)).collect();
    world.players = Arc::new(vec![("PovRed".to_owned(), 1)]);
    let cache = ChunkPacketCache::new();
    let mut viewer = viewer();
    let first = world.output();
    let sent = teams(&plan_viewer(&first, &mut viewer, 4, &cache));
    assert_eq!(sent.len(), 2, "{sent:?}");
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
    world.store.set_pos(slots[0], [3.5, 64.0, 3.5]);
    let after = world.output();
    assert!(teams(&plan_viewer(&after, &mut viewer, 4, &cache)).is_empty());
    let again = world.output();
    assert!(teams(&plan_viewer(&again, &mut viewer, 4, &cache)).is_empty());
}

/// A team leaves when the snapshot holds no row and no player of its faction
/// (by data, not by command), and comes back with the faction; until then a
/// gone row's entry leaves the team that stays.
#[test]
fn teams_are_removed_by_data() {
    let mut world = World::new();
    world.spawn(1, 1);
    world.spawn(2, 1);
    world.spawn(3, 2);
    world.players = Arc::new(vec![("PovBlue".to_owned(), 2)]);
    let mut viewer = viewer();
    let mut client = Client::default();
    let mut previous = world.output();
    client.apply(&plan(&previous, None, &mut viewer));
    let mut step = |world: &mut World, viewer: &mut ViewerState| {
        let next = world.output();
        let packets = plan(&next, Some(&previous.snapshot.entities), viewer);
        client.apply(&packets);
        previous = next;
        summary(&packets)
    };
    world.remove(1);
    assert_eq!(
        step(&mut world, &mut viewer),
        [("sf 1".to_owned(), REMOVE_MEMBERS, vec![entry(1)])],
        "a red row is left"
    );
    world.remove(2);
    assert_eq!(
        step(&mut world, &mut viewer),
        [("sf 1".to_owned(), REMOVE, vec![])]
    );
    world.remove(3);
    assert_eq!(
        step(&mut world, &mut viewer),
        [("sf 2".to_owned(), REMOVE_MEMBERS, vec![entry(3)])],
        "PovBlue keeps sf 2"
    );
    world.players = Arc::default();
    assert_eq!(
        step(&mut world, &mut viewer),
        [("sf 2".to_owned(), REMOVE, vec![])]
    );
    world.spawn(4, 1);
    assert_eq!(
        step(&mut world, &mut viewer),
        [
            ("sf 1".to_owned(), CREATE, vec![]),
            ("sf 1".to_owned(), ADD, vec![entry(4)]),
        ]
    );
}

/// Battle reset ends every hold and removes the staged rows but leaves
/// summoned faction rows standing: the holders' and removed rows' entries
/// leave the team that stays (method 4, listed entries only, once), the
/// holder's nameplate is his own again, and a new hold adds the name back.
#[test]
fn a_reset_releases_the_holders_from_a_team_that_stays() {
    let mut world = World::new();
    for id in 1..=3 {
        world.spawn(id, 1);
    }
    world.spawn(9, 1);
    world.players = Arc::new(vec![("PovRed".to_owned(), 1)]);
    let mut viewer = viewer();
    let mut client = Client::default();
    let first = world.output();
    client.apply(&plan(&first, None, &mut viewer));
    for id in 1..=3 {
        world.remove(id);
    }
    world.players = Arc::default();
    let reset = world.output();
    let sent = plan(&reset, Some(&first.snapshot.entities), &mut viewer);
    assert_eq!(
        summary(&sent),
        [(
            "sf 1".to_owned(),
            REMOVE_MEMBERS,
            sorted([entry(1), entry(2), entry(3), "PovRed".to_owned()])
        )],
        "the summoned survivor keeps sf 1"
    );
    client.apply(&sent);
    assert_eq!(client.team_of("PovRed"), None, "nameplate back");
    assert_eq!(client.team_of(&entry(9)), Some("sf 1"));
    assert!(viewer.teams.players.is_empty());
    world.players = Arc::default();
    let quiet = world.output();
    assert!(plan(&quiet, Some(&reset.snapshot.entities), &mut viewer).is_empty());
    world.players = Arc::new(vec![("PovRed".to_owned(), 1)]);
    let again = world.output();
    assert_eq!(
        summary(&plan(&again, Some(&quiet.snapshot.entities), &mut viewer)),
        [("sf 1".to_owned(), ADD, vec!["PovRed".to_owned()])]
    );
}

/// Battle reset of a staged battle removes every row and hold: each team
/// leaves whole (method 1), with no entry removals before it.
#[test]
fn a_reset_of_the_whole_battle_removes_the_teams() {
    let mut world = World::new();
    world.spawn(1, 1);
    world.spawn(2, 2);
    world.players = Arc::new(vec![("PovBlue".to_owned(), 2), ("PovRed".to_owned(), 1)]);
    let mut viewer = viewer();
    let mut client = Client::default();
    let first = world.output();
    client.apply(&plan(&first, None, &mut viewer));
    world.remove(1);
    world.remove(2);
    world.players = Arc::default();
    let reset = world.output();
    let sent = plan(&reset, Some(&first.snapshot.entities), &mut viewer);
    assert_eq!(
        summary(&sent),
        [
            ("sf 1".to_owned(), REMOVE, vec![]),
            ("sf 2".to_owned(), REMOVE, vec![]),
        ]
    );
    client.apply(&sent);
    assert!(client.teams.is_empty());
    assert!(viewer.teams.mobs.is_empty() && viewer.teams.players.is_empty());
    assert!(viewer.teams.created.is_empty());
}

/// After a restart the battle's holds are gone (`BattleState` is not
/// persisted) and every client joins afresh: the teams hold the persisted
/// faction rows only.
#[test]
fn after_a_restart_the_teams_hold_the_rows_only() {
    let mut world = World::new();
    world.spawn(1, 1);
    world.spawn(2, 2);
    assert_eq!(
        summary(&plan(&world.output(), None, &mut viewer())),
        [
            ("sf 1".to_owned(), CREATE, vec![]),
            ("sf 1".to_owned(), ADD, vec![entry(1)]),
            ("sf 2".to_owned(), CREATE, vec![]),
            ("sf 2".to_owned(), ADD, vec![entry(2)]),
        ]
    );
}

/// A `/team join` routed through the player-change path, as the pump does
/// before the world publish.
fn route_join(viewer: &mut ViewerState, members: &[String]) -> Vec<ClientboundSetPlayerTeam> {
    use mc_core::scoreboard::ScoreboardEvent;
    let join = PlayerChange::Scoreboard {
        events: vec![ScoreboardEvent::Team {
            name: "allies".into(),
            method: ADD,
            team: None,
            members: members.to_vec(),
        }]
        .into(),
    };
    teams(&crate::plan::plan_player_changes(&[join], viewer))
}

/// A client already holding the core's empty team `allies`.
fn client_with_allies() -> Client {
    let mut client = Client::default();
    client.apply(&[ClientboundSetPlayerTeam {
        name: "allies".into(),
        method: CREATE,
        parameters: None,
        players: Vec::new(),
    }]);
    client
}

/// The pump routes the core's scoreboard packets before the world publish. A
/// holder whose hold ends in the tick of his `/team join` is released from
/// his faction team ahead of the core's add (which moves him on the client);
/// the publish then removes nothing more. An entry not listed is never
/// removed.
#[test]
fn a_team_join_in_the_tick_a_hold_ends_releases_the_holder_first() {
    let mut world = World::new();
    world.spawn(1, 1);
    world.players = Arc::new(vec![("PovRed".to_owned(), 1)]);
    let mut viewer = viewer();
    let mut client = client_with_allies();
    let first = world.output();
    client.apply(&plan(&first, None, &mut viewer));
    let routed = route_join(&mut viewer, &["PovRed".to_owned(), "Other".to_owned()]);
    assert_eq!(
        summary(&routed),
        [
            ("sf 1".to_owned(), REMOVE_MEMBERS, vec!["PovRed".to_owned()]),
            (
                "allies".to_owned(),
                ADD,
                vec!["Other".to_owned(), "PovRed".to_owned()]
            ),
        ]
    );
    client.apply(&routed);
    world.players = Arc::default();
    let ended = world.output();
    assert!(plan(&ended, Some(&first.snapshot.entities), &mut viewer).is_empty());
    assert_eq!(client.team_of("PovRed"), Some("allies"));
}

/// The same for a mob row that left the snapshot in the tick its UUID joins a
/// `/team` team (the core refuses the join only while the row has a
/// faction): the entry released is the one sent, the lowercase UUID, also
/// when the command spelled it in capitals.
#[test]
fn a_team_join_in_the_tick_a_row_leaves_releases_its_entry_first() {
    let mut world = World::new();
    world.spawn(0xab, 1);
    world.spawn(2, 1);
    let mut viewer = viewer();
    let mut client = client_with_allies();
    let first = world.output();
    client.apply(&plan(&first, None, &mut viewer));
    world.remove(0xab);
    let spelled = entry(0xab).to_uppercase();
    assert_ne!(spelled, entry(0xab));
    let routed = route_join(&mut viewer, std::slice::from_ref(&spelled));
    assert_eq!(
        summary(&routed),
        [
            ("sf 1".to_owned(), REMOVE_MEMBERS, vec![entry(0xab)]),
            ("allies".to_owned(), ADD, vec![spelled.clone()]),
        ]
    );
    client.apply(&routed);
    let gone = world.output();
    let sent = plan(&gone, Some(&first.snapshot.entities), &mut viewer);
    assert!(sent.is_empty(), "{sent:?}");
    assert_eq!(client.team_of(&entry(0xab)), None);
    assert_eq!(client.team_of(&spelled), Some("allies"));
    assert_eq!(client.team_of(&entry(2)), Some("sf 1"));
}

/// A team held only by a holder leaves with his hold, also when his entry
/// was released ahead of a core `/team` add in that tick.
#[test]
fn a_released_holders_team_leaves_with_his_hold() {
    let mut world = World::new();
    world.spawn(1, 1);
    world.spawn(2, 2);
    world.players = Arc::new(vec![("PovRed".to_owned(), 1)]);
    let mut viewer = viewer();
    let mut client = client_with_allies();
    let first = world.output();
    client.apply(&plan(&first, None, &mut viewer));
    world.remove(1);
    let fallen = world.output();
    let sent = plan(&fallen, Some(&first.snapshot.entities), &mut viewer);
    assert_eq!(
        summary(&sent),
        [("sf 1".to_owned(), REMOVE_MEMBERS, vec![entry(1)])]
    );
    client.apply(&sent);
    client.apply(&route_join(&mut viewer, &["PovRed".to_owned()]));
    world.players = Arc::default();
    let ended = world.output();
    let sent = plan(&ended, Some(&fallen.snapshot.entities), &mut viewer);
    assert_eq!(summary(&sent), [("sf 1".to_owned(), REMOVE, vec![])]);
    client.apply(&sent);
    assert_eq!(client.team_of("PovRed"), Some("allies"));
}

/// A row whose faction a core team name blocks leaves the team it was sent
/// in; the blocked faction gets no add.
#[test]
fn a_row_moving_to_a_blocked_faction_leaves_its_team() {
    let mut world = World::new();
    let slots: Vec<_> = (1..=2).map(|id| world.spawn(id, 1)).collect();
    world.names = Arc::new(vec!["sf 2".to_owned()]);
    let mut viewer = viewer();
    let mut client = Client::default();
    let first = world.output();
    client.apply(&plan(&first, None, &mut viewer));
    world.store.set_faction(slots[0], 2);
    let moved = world.output();
    let sent = plan(&moved, Some(&first.snapshot.entities), &mut viewer);
    assert_eq!(
        summary(&sent),
        [("sf 1".to_owned(), REMOVE_MEMBERS, vec![entry(1)])]
    );
    client.apply(&sent);
    assert_eq!(client.team_of(&entry(1)), None);
    assert!(!viewer.teams.mobs.contains_key(&Uuid::from_u128(1)));
}

#[test]
fn dimension_change_removes_the_teams_and_syncs_again() {
    let mut overworld = World::new();
    overworld.spawn(1, 1);
    let mut viewer = viewer();
    plan(&overworld.output(), None, &mut viewer);
    let mut nether = World::new();
    nether.dimension = Dimension::Nether;
    nether.spawn(2, 2);
    assert_eq!(
        summary(&plan(&nether.output(), None, &mut viewer)),
        [
            ("sf 1".to_owned(), REMOVE, vec![]),
            ("sf 2".to_owned(), CREATE, vec![]),
            ("sf 2".to_owned(), ADD, vec![entry(2)]),
        ]
    );
}

/// A `/team` team of exactly a projection name (an imported world or a
/// plugin) owns that name: its faction is not projected, a team the
/// projection had created is left to it, and once the name is free the
/// faction syncs in full.
#[test]
fn a_core_team_with_the_projection_name_blocks_its_faction() {
    let mut world = World::new();
    world.spawn(1, 1);
    world.spawn(2, 1);
    world.spawn(3, 2);
    world.names = Arc::new(vec!["sf 1".to_owned()]);
    let mut viewer = viewer();
    let first = world.output();
    assert_eq!(
        summary(&plan(&first, None, &mut viewer)),
        [
            ("sf 2".to_owned(), CREATE, vec![]),
            ("sf 2".to_owned(), ADD, vec![entry(3)]),
        ]
    );
    world.names = Arc::new(vec!["sf 2".to_owned()]);
    world.remove(3);
    let second = world.output();
    let packets = summary(&plan(&second, Some(&first.snapshot.entities), &mut viewer));
    assert_eq!(
        packets,
        [
            ("sf 1".to_owned(), CREATE, vec![]),
            ("sf 1".to_owned(), ADD, sorted([entry(1), entry(2)])),
        ],
        "sf 1 freed: full sync; sf 2 now the core's: never removed by the projection"
    );
}

/// `/team add sf:1` is an ordinary team: it blocks nothing.
#[test]
fn a_core_team_named_sf_colon_coexists_with_the_projection() {
    let mut world = World::new();
    world.spawn(1, 1);
    world.names = Arc::new(vec!["sf:1".to_owned(), "sf 01".to_owned()]);
    assert_eq!(
        summary(&plan(&world.output(), None, &mut viewer())),
        [
            ("sf 1".to_owned(), CREATE, vec![]),
            ("sf 1".to_owned(), ADD, vec![entry(1)]),
        ]
    );
}
