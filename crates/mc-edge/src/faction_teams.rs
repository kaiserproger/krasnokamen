//! Battle factions on the wire: one vanilla scoreboard team per faction,
//! planned per connection from the [`WorldSnapshot`]. Mobs join by UUID
//! string (vanilla `Entity.getScoreboardName`), live players by name
//! ([`WorldSnapshot::player_factions`]). The core stays the only authority:
//! these teams exist only on the client, are never persisted and never change
//! gameplay; the core's `/team` teams remain the only teams the server knows.
//!
//! Vanilla client behaviour (javap 26.1.2 and 26.3,
//! `.analysis/codex-logs/faction-teams/javap.txt`):
//! - an add to an unknown team is dropped, so a team is created first;
//! - an add moves an entry out of its other team;
//! - removing an entry from a team that does not hold it throws, which
//!   disconnects. The projection never sends method 4: entries stay until
//!   their team is removed, or an add moves them. The core's entry-free rule
//!   rejects faction admission for `/team` entries and rejects `/team join`
//!   for active faction entries; [`forget_for_core_team`] tracks core adds;
//! - the scoreboard outlives respawns and dimension changes;
//! - a team member's nameplate follows the team's nametag visibility before
//!   the hidden-HUD check, so these teams use `never`.
//!
//! Team name `sf <faction>`: no `/team` command can create it. Vanilla
//! `team add` takes a Brigadier word (`[0-9A-Za-z_.+-]`), and the Krasnokamen
//! tokenizer (`mc_core::commands::command_words`) keeps whitespace only
//! inside quotes or brackets, which stay part of the word. A core team of
//! exactly that name (an imported world, a plugin) blocks its faction: it is
//! not projected, and the connection logs one error naming the team.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use mc_core::boundary::{EntityView, EntityViews, WorldSnapshot};
use mc_core::dimensions::Dimension;
use mc_protocol::packets::play::{ClientboundSetPlayerTeam, TeamParameters};
use rustc_hash::FxHashMap;
use uuid::Uuid;

use crate::plan::{PlannedPacket, ViewerState, push_reliable};

/// `sf <faction>`.
const TEAM_PREFIX: &str = "sf ";
/// Entries per add packet: 4096 UUID strings are about 150 KB, far inside the
/// 2 MiB frame and the 65,536 entries the 776 translation accepts.
pub(crate) const MEMBERS_PER_PACKET: usize = 4096;
pub(crate) const CREATE: u8 = 0;
pub(crate) const REMOVE: u8 = 1;
pub(crate) const ADD: u8 = 3;
/// 26.3 `TeamColor` ids (the `ChatFormatting` order): red 12, blue 9; other
/// factions get no colour (`ChatFormatting.RESET`).
const RED: i32 = 12;
const BLUE: i32 = 9;
const NO_COLOR: i32 = 21;
/// `Team$Visibility.NEVER` and `Team$CollisionRule.ALWAYS` ids.
const NAMETAG_NEVER: i32 = 1;
const COLLISION_ALWAYS: i32 = 0;
/// Friendly fire as vanilla (bit 0), allies' invisibility not seen through
/// (bit 1 clear).
const OPTIONS: u8 = 0b01;

pub(crate) fn team_name(faction: u16) -> String {
    format!("{TEAM_PREFIX}{faction}")
}

/// Faction membership change of one continuous entity revision step: built
/// once per publish in the shared change loop
/// ([`crate::plan::TickPlanIndex::with_previous`]) and read by every viewer
/// whose memberships were reconciled at `base`.
#[derive(Debug)]
pub(crate) struct FactionDelta {
    base: u64,
    /// `(faction, uuid)` of the rows that entered a nonzero faction,
    /// ascending faction.
    added: Vec<(u16, Uuid)>,
    /// Factions that lost a row (it left the snapshot or changed faction),
    /// ascending.
    lost: Vec<u16>,
    /// The `lost` factions that still have a row, found on first use.
    still_held: OnceLock<Vec<u16>>,
}

impl FactionDelta {
    pub(crate) fn new(base: u64) -> Self {
        Self {
            base,
            added: Vec::new(),
            lost: Vec::new(),
            still_held: OnceLock::new(),
        }
    }

    /// One changed row: its image in the base and in the new revision.
    pub(crate) fn observe(&mut self, old: Option<&EntityView>, new: Option<&EntityView>) {
        let old_faction = old.map_or(0, |row| row.faction);
        let new_faction = new.map_or(0, |row| row.faction);
        if old_faction == new_faction {
            return;
        }
        if old_faction != 0 {
            self.lost.push(old_faction);
        }
        if let Some(row) = new.filter(|_| new_faction != 0) {
            self.added.push((new_faction, row.uuid));
        }
    }

    pub(crate) fn finish(mut self) -> Self {
        self.added.sort_by_key(|(faction, _)| *faction);
        self.lost.sort_unstable();
        self.lost.dedup();
        self
    }

    fn still_held(&self, entities: &EntityViews) -> &[u16] {
        self.still_held.get_or_init(|| held(entities, &self.lost))
    }
}

/// Which of `factions` (ascending) still have a row; stops once all are found.
fn held(entities: &EntityViews, factions: &[u16]) -> Vec<u16> {
    let mut found = Vec::new();
    if factions.is_empty() {
        return found;
    }
    for row in entities {
        if row.faction != 0
            && factions.binary_search(&row.faction).is_ok()
            && !found.contains(&row.faction)
        {
            found.push(row.faction);
            if found.len() == factions.len() {
                break;
            }
        }
    }
    found.sort_unstable();
    found
}

/// What one client holds: the projected teams and their entries exactly as
/// sent. Respawns keep it: the client scoreboard outlives them; a dimension
/// change removes the teams and syncs again.
#[derive(Debug, Default)]
pub(crate) struct TeamBaseline {
    /// Dimension the client's teams were planned from.
    dimension: Option<Dimension>,
    /// Entity revision `mobs` was last reconciled with; `None`: never.
    revision: Option<u64>,
    /// Mob entries (UUID) the client holds in a projected team.
    mobs: FxHashMap<Uuid, u16>,
    /// The player list `players` was last reconciled with.
    player_list: Arc<Vec<(String, u16)>>,
    /// Players the client holds in a projected team.
    players: BTreeMap<String, u16>,
    /// Teams created on the client, ascending faction.
    created: Vec<u16>,
    /// Core team names the `blocked` factions were derived from.
    names: Arc<Vec<String>>,
    /// Factions whose team name a core team holds, ascending.
    blocked: Vec<u16>,
}

impl TeamBaseline {
    /// The client no longer holds `faction`'s team (removed, or owned by a
    /// core team of that name), nor any entry in it.
    fn forget(&mut self, faction: u16) {
        self.created.retain(|&created| created != faction);
        self.mobs.retain(|_, held| *held != faction);
        self.players.retain(|_, held| *held != faction);
    }
}

/// Plan this viewer's team packets from one snapshot of its dimension. All
/// are reliable, like the core's scoreboard packets: a membership the client
/// misses is never repaired.
///
/// Entries stay in their last projected team after their faction ends. Only
/// an add moves them, or removal of the whole team takes them along. Never
/// send method 4, including before a core `/team` add. A team leaves (method
/// 1) when no snapshot row and no player of its faction remains.
pub(crate) fn plan_teams(
    snapshot: &WorldSnapshot,
    delta: Option<&FactionDelta>,
    viewer: &mut ViewerState,
    out: &mut Vec<PlannedPacket>,
) {
    let connection = viewer.connection;
    let teams = &mut viewer.teams;
    if teams
        .dimension
        .is_some_and(|held| held != snapshot.dimension)
    {
        // The new dimension's snapshot does not describe these teams.
        for &faction in &teams.created {
            push_reliable(out, connection, &team(faction, REMOVE, Vec::new()));
        }
        teams.created.clear();
        teams.mobs.clear();
        teams.players.clear();
        teams.revision = None;
        teams.player_list = Arc::default();
    }
    teams.dimension = Some(snapshot.dimension);
    if !Arc::ptr_eq(&teams.names, &snapshot.team_names) {
        let blocked = blocked(&snapshot.team_names);
        for &faction in &blocked {
            if teams.blocked.contains(&faction) {
                continue;
            }
            tracing::error!(
                ?connection,
                "battle faction {faction} is not sent as a team: a scoreboard team named {:?} exists; rename or remove it",
                team_name(faction)
            );
            // The core's team owns that name on the client now.
            teams.forget(faction);
        }
        if teams.blocked.iter().any(|f| !blocked.contains(f)) {
            // A freed faction's members never went out: sync everything.
            teams.revision = None;
            teams.player_list = Arc::default();
        }
        teams.blocked = blocked;
        teams.names = Arc::clone(&snapshot.team_names);
    }

    let entities = &snapshot.entities;
    let mut additions: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    let mut candidates: Vec<u16> = Vec::new();
    let mut shared = None;
    let mut present = None;
    match teams.revision {
        Some(revision) if revision == entities.revision() => {}
        Some(revision) if delta.is_some_and(|delta| delta.base == revision) => {
            let delta = delta.expect("matched base");
            for (faction, uuid) in &delta.added {
                if !teams.blocked.contains(faction)
                    && teams.mobs.insert(*uuid, *faction) != Some(*faction)
                {
                    additions
                        .entry(*faction)
                        .or_default()
                        .push(uuid.to_string());
                }
            }
            candidates.extend_from_slice(&delta.lost);
            shared = Some(delta);
        }
        _ => {
            present = Some(reconcile_mobs(
                entities,
                &teams.blocked,
                &mut teams.mobs,
                &mut additions,
            ));
            candidates.extend_from_slice(&teams.created);
        }
    }
    teams.revision = Some(entities.revision());
    if !Arc::ptr_eq(&teams.player_list, &snapshot.player_factions) {
        let list = &snapshot.player_factions;
        // A core add may already have moved a stale projected entry.
        candidates.extend(teams.player_list.iter().map(|(_, faction)| *faction));
        candidates.extend(teams.players.values().copied());
        for (name, faction) in list.iter() {
            if *faction == 0
                || teams.players.get(name) == Some(faction)
                || teams.blocked.contains(faction)
            {
                continue;
            }
            additions.entry(*faction).or_default().push(name.clone());
            teams.players.insert(name.clone(), *faction);
        }
        teams.player_list = Arc::clone(list);
    }

    // Removal by data: no row and no player of the faction is left.
    candidates.sort_unstable();
    candidates.dedup();
    candidates.retain(|faction| teams.created.binary_search(faction).is_ok());
    let mut gone = Vec::new();
    for faction in candidates {
        let rows = match (&present, shared) {
            (Some(present), _) => present.binary_search(&faction).is_ok(),
            (None, Some(delta)) if delta.lost.binary_search(&faction).is_ok() => {
                delta.still_held(entities).binary_search(&faction).is_ok()
            }
            _ => !held(entities, &[faction]).is_empty(),
        };
        if !rows
            && !snapshot
                .player_factions
                .iter()
                .any(|(_, held)| *held == faction)
        {
            gone.push(faction);
        }
    }
    for faction in gone {
        push_reliable(out, connection, &team(faction, REMOVE, Vec::new()));
        teams.forget(faction);
    }
    for (faction, members) in additions {
        if let Err(at) = teams.created.binary_search(&faction) {
            push_reliable(out, connection, &team(faction, CREATE, Vec::new()));
            teams.created.insert(at, faction);
        }
        push_members(out, connection, faction, members);
    }
}

/// Forget entries moved by a core team add. Vanilla performs the move;
/// the projection only updates its baseline, without an entry-removal packet.
pub(crate) fn forget_for_core_team(
    event: &mc_core::scoreboard::ScoreboardEvent,
    viewer: &mut ViewerState,
) {
    let mc_core::scoreboard::ScoreboardEvent::Team {
        method: 0 | 3,
        members,
        ..
    } = event
    else {
        return;
    };
    let teams = &mut viewer.teams;
    if teams.mobs.is_empty() && teams.players.is_empty() {
        return;
    }
    for member in members {
        // UUID spelling variants share a baseline key; player names stay exact.
        match Uuid::try_parse(member) {
            Ok(uuid) => {
                teams.mobs.remove(&uuid);
            }
            Err(_) => {
                teams.players.remove(member);
            }
        }
    }
}

/// Add `members` in chunks of [`MEMBERS_PER_PACKET`].
fn push_members(
    out: &mut Vec<PlannedPacket>,
    connection: mc_core::boundary::ConnectionId,
    faction: u16,
    mut members: Vec<String>,
) {
    while !members.is_empty() {
        let rest = members.split_off(members.len().min(MEMBERS_PER_PACKET));
        let chunk = std::mem::replace(&mut members, rest);
        push_reliable(out, connection, &team(faction, ADD, chunk));
    }
}

/// Add current mobs missing from the baseline, retaining stale entries until
/// their whole team leaves. Returns factions with a current row, ascending.
fn reconcile_mobs(
    entities: &EntityViews,
    blocked: &[u16],
    mobs: &mut FxHashMap<Uuid, u16>,
    additions: &mut BTreeMap<u16, Vec<String>>,
) -> Vec<u16> {
    let mut present = Vec::new();
    for row in entities {
        if row.faction == 0 {
            continue;
        }
        if let Err(at) = present.binary_search(&row.faction) {
            present.insert(at, row.faction);
        }
        if blocked.contains(&row.faction) {
            continue;
        }
        if mobs.insert(row.uuid, row.faction) != Some(row.faction) {
            additions
                .entry(row.faction)
                .or_default()
                .push(row.uuid.to_string());
        }
    }
    present
}

/// Factions whose exact team name a core team holds, ascending.
fn blocked(names: &[String]) -> Vec<u16> {
    let mut blocked: Vec<u16> = names
        .iter()
        .filter_map(|name| {
            let faction = name.strip_prefix(TEAM_PREFIX)?.parse::<u16>().ok()?;
            (faction != 0 && team_name(faction) == *name).then_some(faction)
        })
        .collect();
    blocked.sort_unstable();
    blocked
}

/// One projection packet. Parameters are explicit, never vanilla defaults:
/// a holder's nameplate must stay out of hidden-HUD frames.
pub(crate) fn team(faction: u16, method: u8, players: Vec<String>) -> ClientboundSetPlayerTeam {
    let (display, color) = match faction {
        1 => ("Red".to_owned(), RED),
        2 => ("Blue".to_owned(), BLUE),
        _ => (format!("Faction {faction}"), NO_COLOR),
    };
    ClientboundSetPlayerTeam {
        name: team_name(faction),
        method,
        parameters: (method == CREATE).then(|| TeamParameters {
            display_name: mc_nbt::Tag::String(display),
            options: OPTIONS,
            nametag_visibility: NAMETAG_NEVER,
            collision_rule: COLLISION_ALWAYS,
            color,
            prefix: mc_nbt::Tag::String(String::new()),
            suffix: mc_nbt::Tag::String(String::new()),
        }),
        players,
    }
}

#[cfg(test)]
#[path = "faction_teams_tests.rs"]
mod tests;
