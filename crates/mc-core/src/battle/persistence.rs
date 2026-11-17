//! Battle decision history shares native entity custody's checkpoint/WAL fence.
//!
//! The checkpoint row and the WAL `Battle` record carry the full image. Within
//! a WAL tail, later ticks carry `BattlePatch`: the image header and only the
//! list elements that changed since the previous battle record (`capture`).
//! The first battle record after a checkpoint capture or recovery is always
//! full, so a patch never builds on a checkpoint row (`fold_patch`).
use super::{BattleState, EnemyTarget, ground::Escape};
use crate::{
    CoreError,
    boundary::EntityId,
    dimensions::Dimension,
    journal::{JournalEntry, Replay},
    tick::TickState,
};
use mc_world::native::NativeDomainRecord;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

type Column = (EntityId, Option<EnemyTarget>, Option<Escape>);

#[cfg(test)]
pub(crate) struct EncodingFault {
    pub store: std::sync::Weak<mc_world::native::NativeStore>,
    pub observed: std::sync::mpsc::Sender<u64>,
}

#[cfg(test)]
pub(crate) static ENCODING_FAULT: parking_lot::Mutex<Option<EncodingFault>> =
    parking_lot::const_mutex(None);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Image<B, C, D> {
    version: u32,
    battle: B,
    columns: C,
    removed: D,
}

pub(super) fn error(message: impl std::fmt::Display) -> CoreError {
    CoreError::Storage(format!("battle recovery: {message}"))
}

pub(crate) fn row_key(dimension: Dimension) -> String {
    format!(
        "battle/{}",
        crate::native_checkpoint::dimension_id(dimension)
    )
}

/// The battle pass precedes native damage merge. Reconcile only decisions the
/// next ordinary pass discards, using actual native identity/death evidence.
/// For staging references an absent body is not proof of death: an unknown
/// staged ID must still fail replay. Pending retaliation is the exception: a
/// blow whose attacker the image does not carry (no row and no held player's
/// identity) is one the next pass discards (`Retaliation::reconcile_capture`).
fn reconcile_native_capture(state: &mut TickState) {
    let entities = &state.entities;
    let battle = &mut state.hostile.battle;
    let survives = |id: &EntityId| {
        entities
            .slot_of_any(*id)
            .is_none_or(|slot| entities.body(slot).alive)
    };
    battle.ground_pending.retain(survives);
    battle.ground_refused.retain(survives);
    battle
        .retaliation
        .reconcile_capture(entities, &battle.live, &state.connections);
}

/// Test-only encoding fault, armed per native store: every image encoding of
/// that world (full image, patch or checkpoint row) fails once.
fn injected_fault(state: &TickState) -> Result<(), CoreError> {
    #[cfg(test)]
    {
        let fault = {
            let mut armed = ENCODING_FAULT.lock();
            if armed.as_ref().is_some_and(|fault| {
                fault.store.upgrade().is_some_and(|store| {
                    state
                        .world
                        .chunk_source()
                        .native
                        .as_ref()
                        .is_some_and(|actual| Arc::ptr_eq(actual, &store))
                })
            }) {
                armed.take()
            } else {
                None
            }
        };
        if let Some(fault) = fault {
            let _ = fault.observed.send(state.tick);
            return Err(error("injected battle image encoding failure"));
        }
    }
    #[cfg(not(test))]
    let _ = state;
    Ok(())
}

fn columns(battle: &BattleState) -> Vec<Column> {
    battle
        .columns
        .ids
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, battle.columns.targets[i], battle.columns.escape[i]))
        .collect()
}

/// A death merged after the battle pass is already a native tombstone.
fn removed(state: &TickState) -> Vec<EntityId> {
    state
        .hostile
        .battle
        .formation
        .members
        .iter()
        .filter_map(|m| {
            state
                .entities
                .slot_of(m.id)
                .filter(|&slot| state.entities.row(slot).health > 0.0)
                .is_none()
                .then_some(m.id)
        })
        .collect()
}

pub(crate) fn encode(state: &TickState) -> Result<String, CoreError> {
    injected_fault(state)?;
    serde_json::to_string(&Image {
        version: 1,
        battle: &state.hostile.battle,
        columns: columns(&state.hostile.battle),
        removed: removed(state),
    })
    .map_err(error)
}

/// Index-aligned image lists a patch carries element by element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum List {
    Members,
    Regiments,
    Fighters,
    Ids,
    Columns,
    Removed,
}

impl List {
    const ALL: [Self; 6] = [
        Self::Members,
        Self::Regiments,
        Self::Fighters,
        Self::Ids,
        Self::Columns,
        Self::Removed,
    ];

    /// JSON pointer of the list inside the full image.
    fn pointer(self) -> &'static str {
        match self {
            Self::Members => "/battle/formation/members",
            Self::Regiments => "/battle/formation/regiments",
            Self::Fighters => "/battle/combat/fighters",
            Self::Ids => "/battle/combat/ids",
            Self::Columns => "/columns",
            Self::Removed => "/removed",
        }
    }
}

/// Above every list capacity `validate_structure` accepts: a corrupt length
/// fails closed before allocation.
const MAX_LIST_LEN: usize = 40_000;

/// Changed parts of the image since the previous battle record of the tail.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    /// The image without its lists (each list empty), when it changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    header: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    lists: Vec<ListPatch>,
}

/// A list's new length and its changed elements as absolute values.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListPatch {
    list: List,
    len: usize,
    set: Vec<(usize, Value)>,
}

impl Patch {
    fn is_empty(&self) -> bool {
        self.header.is_none() && self.lists.is_empty()
    }

    /// The one place a patch reaches an image; a missing list, a length above
    /// capacity or an index past the new length refuses, and an element left
    /// unset after growth stays `null`, which `decode` refuses.
    fn apply(self, image: &mut Value) -> Result<(), String> {
        if let Some(mut header) = self.header {
            for list in List::ALL {
                let slot = header
                    .pointer_mut(list.pointer())
                    .filter(|slot| slot.is_array())
                    .ok_or_else(|| format!("battle patch header lacks {list:?}"))?;
                *slot = image
                    .pointer_mut(list.pointer())
                    .map(Value::take)
                    .ok_or_else(|| format!("battle image lacks {list:?}"))?;
            }
            *image = header;
        }
        for ListPatch { list, len, set } in self.lists {
            if len > MAX_LIST_LEN {
                return Err(format!(
                    "battle patch {list:?} length {len} exceeds capacity"
                ));
            }
            let elements = image
                .pointer_mut(list.pointer())
                .and_then(Value::as_array_mut)
                .ok_or_else(|| format!("battle image lacks {list:?}"))?;
            elements.resize(len, Value::Null);
            for (index, element) in set {
                *elements.get_mut(index).ok_or_else(|| {
                    format!("battle patch {list:?} index {index} past length {len}")
                })? = element;
            }
        }
        Ok(())
    }
}

/// WAL baseline of one dimension's battle image (`BattleState::journaled`).
#[derive(Debug)]
pub(crate) struct Journaled {
    /// JSON of the last captured header and list elements.
    parts: Parts,
    /// No battle record since the last checkpoint capture or recovery: the
    /// next change journals a full image, never a patch on a checkpoint row.
    full_due: bool,
}

impl Journaled {
    /// No image journaled in this WAL tail yet (fresh, or after recovery): the
    /// first record is the whole image, even when that tick resets the battle.
    pub(crate) fn full_due() -> Self {
        Self {
            parts: Parts::default(),
            full_due: true,
        }
    }
}

#[derive(Debug, Default)]
struct Parts {
    header: String,
    lists: [Vec<String>; List::ALL.len()],
}

/// Serialize the image part by part into `parts`; the patch holds the parts
/// that differ from what `parts` held. Allocates only for changed parts.
fn capture(state: &mut TickState, parts: &mut Parts) -> Result<Patch, CoreError> {
    injected_fault(state)?;
    let removed = removed(state);
    let battle = &mut state.hostile.battle;
    let columns = columns(battle);
    let lists = (
        std::mem::take(&mut battle.formation.members),
        std::mem::take(&mut battle.formation.regiments),
        std::mem::take(&mut battle.combat.fighters),
        std::mem::take(&mut battle.combat.ids),
    );
    let header = serde_json::to_string(&Image {
        version: 1,
        battle: &*battle,
        columns: &[] as &[Column],
        removed: &[] as &[EntityId],
    });
    (
        battle.formation.members,
        battle.formation.regiments,
        battle.combat.fighters,
        battle.combat.ids,
    ) = lists;
    let header = header.map_err(error)?;
    let mut patch = Patch::default();
    if parts.header != header {
        patch.header = Some(serde_json::from_str(&header).map_err(error)?);
        parts.header = header;
    }
    let mut buffer = Vec::new();
    let [
        members,
        regiments,
        fighters,
        ids,
        column_parts,
        removed_parts,
    ] = &mut parts.lists;
    let battle = &state.hostile.battle;
    let formation = &battle.formation;
    let combat = &battle.combat;
    let buffer = &mut buffer;
    let out = &mut patch;
    diff_list(List::Members, &formation.members, members, buffer, out)?;
    diff_list(
        List::Regiments,
        &formation.regiments,
        regiments,
        buffer,
        out,
    )?;
    diff_list(List::Fighters, &combat.fighters, fighters, buffer, out)?;
    diff_list(List::Ids, &combat.ids, ids, buffer, out)?;
    diff_list(List::Columns, &columns, column_parts, buffer, out)?;
    diff_list(List::Removed, &removed, removed_parts, buffer, out)?;
    Ok(patch)
}

/// Compare each element's JSON with `old` and record the changed elements and
/// a changed length, keeping `old` equal to the current elements.
fn diff_list<T: Serialize>(
    list: List,
    elements: &[T],
    old: &mut Vec<String>,
    buffer: &mut Vec<u8>,
    patch: &mut Patch,
) -> Result<(), CoreError> {
    let previous_len = old.len();
    let mut set = Vec::new();
    for (index, element) in elements.iter().enumerate() {
        buffer.clear();
        serde_json::to_writer(&mut *buffer, element).map_err(error)?;
        if old
            .get(index)
            .is_some_and(|old| old.as_bytes() == buffer.as_slice())
        {
            continue;
        }
        set.push((index, serde_json::from_slice(buffer).map_err(error)?));
        let json = String::from_utf8(buffer.clone()).map_err(error)?;
        match old.get_mut(index) {
            Some(slot) => *slot = json,
            None => old.push(json),
        }
    }
    old.truncate(elements.len());
    if !set.is_empty() || previous_len != elements.len() {
        patch.lists.push(ListPatch {
            list,
            len: elements.len(),
            set,
        });
    }
    Ok(())
}

pub(crate) fn decode(bytes: &str) -> Result<BattleState, CoreError> {
    let Image {
        version,
        mut battle,
        columns,
        removed,
    }: Image<BattleState, Vec<Column>, Vec<EntityId>> =
        serde_json::from_str(bytes).map_err(error)?;
    if version != 1 {
        return Err(error(format!("unsupported version {version}")));
    }
    validate_structure(&battle, &columns, &removed)?;
    let removed: rustc_hash::FxHashSet<_> = removed.into_iter().collect();
    for member in &mut battle.formation.members {
        member.alive = !removed.contains(&member.id);
    }
    battle.formation.restore_index();
    battle.columns.restore(columns);
    Ok(battle)
}

fn validate_structure(
    battle: &BattleState,
    columns: &[Column],
    removed: &[EntityId],
) -> Result<(), CoreError> {
    let formation = &battle.formation;
    if formation.members.len() > 20_000 || formation.regiments.len() > 128 || columns.len() > 40_000
    {
        return Err(error("capacity exceeded"));
    }
    if battle.running && formation.members.is_empty() {
        return Err(error("running without a roster"));
    }
    let mut ids = rustc_hash::FxHashSet::default();
    let mut regiment_ids = rustc_hash::FxHashSet::default();
    let mut dead = rustc_hash::FxHashSet::default();
    if removed.iter().any(|id| !dead.insert(*id)) {
        return Err(error("duplicate removed roster ids"));
    }
    let mut cursor = 0;
    for (ri, regiment) in formation.regiments.iter().enumerate() {
        let spec = regiment.spec;
        if !regiment_ids.insert(spec.id)
            || !super::battle_row(spec.faction)
            || spec.files == 0
            || regiment.start != cursor
            || regiment.len == 0
            || regiment.len > 1000
            || regiment
                .start
                .checked_add(regiment.len)
                .is_none_or(|end| end > formation.members.len())
            || usize::from(spec.files) > regiment.len
            || regiment.ranks != regiment.len.div_ceil(usize::from(spec.files)) as u16
            || regiment.sections.len() != regiment.len.div_ceil(super::formation::SECTION_SIZE)
            || regiment.evacuating.len() != usize::from(spec.files)
            || regiment
                .enemy_regiment
                .is_some_and(|i| i >= formation.regiments.len() || i == ri)
            || !(1.0..=2.0).contains(&spec.frontage)
            || !(spec.frontage..=2.0).contains(&regiment.interval)
            || (mc_physics::mth::length2(spec.facing[0], spec.facing[1]) - 1.0).abs() > 1e-6
        {
            return Err(error(format!("invalid regiment {}", spec.id)));
        }
        let mut places = rustc_hash::FxHashSet::default();
        for (offset, member) in formation.members[cursor..cursor + regiment.len]
            .iter()
            .enumerate()
        {
            if member.id.0 <= 0
                || !ids.insert(member.id)
                || member.regiment != ri
                || member.file >= usize::from(spec.files)
                || member.original_rank != (offset / usize::from(spec.files)) as u16
                || member.rank > regiment.ranks.saturating_mul(2).saturating_add(1)
                || (!dead.contains(&member.id) && !places.insert((member.file, member.rank)))
            {
                return Err(error(format!("invalid roster member {}", member.id.0)));
            }
        }
        cursor += regiment.len;
    }
    if cursor != formation.members.len() {
        return Err(error("unowned roster suffix"));
    }
    if removed.iter().any(|id| !ids.contains(id)) {
        return Err(error("invalid removed roster ids"));
    }
    let mut column_ids = rustc_hash::FxHashSet::default();
    if columns
        .iter()
        .any(|(id, _, _)| id.0 <= 0 || !column_ids.insert(*id))
    {
        return Err(error("duplicate execution identity"));
    }
    // Before the first running tick the execution columns and fighters are empty.
    if !columns.is_empty()
        && (columns.len() < formation.members.len()
            || columns
                .iter()
                .zip(&formation.members)
                .any(|(c, m)| c.0 != m.id))
    {
        return Err(error(
            "execution columns do not begin with the stable roster",
        ));
    }
    battle.combat.validate_history(columns)?;
    battle.live.validate_history(formation)?;
    battle
        .retaliation
        .validate_history(formation.members.len())?;
    Ok(())
}

pub(crate) fn validate_replay(
    replay: &Replay,
    registry: &crate::registry::CoreRegistry,
) -> Result<(), CoreError> {
    if let Some(fault) = &replay.battle_fault {
        return Err(error(fault));
    }
    let Some(image) = image(replay) else {
        if replay.tickets.contains_key(super::staging::OWNER)
            || replay
                .entities
                .values()
                .any(|entity| super::staging::owned(&entity.state.opaque_entity_fields))
        {
            return Err(error("staging ticket has no durable battle history"));
        }
        return Ok(());
    };
    let battle = decode(&image)?;
    for member in &battle.formation.members {
        match replay.entities.get(&member.id.0) {
            Some(entity) if entity.alive && entity.health > 0.0 => {
                let side = battle.formation.regiments[member.regiment].spec.faction;
                if !member.alive || entity.state.faction != side {
                    return Err(error(format!("member {} life/side mismatch", member.id.0)));
                }
            }
            _ if !member.alive => {}
            _ => return Err(error(format!("missing live member {}", member.id.0))),
        }
    }
    for &id in battle.ground_pending.iter().chain(&battle.ground_refused) {
        if !replay.entities.contains_key(&id.0) {
            return Err(error(format!("missing grounded body {}", id.0)));
        }
    }
    // A removed roster member is a known historical target, never a new body.
    let mut identities: rustc_hash::FxHashSet<_> =
        replay.entities.keys().map(|&id| EntityId(id)).collect();
    identities.extend(battle.formation.members.iter().map(|member| member.id));
    identities.extend(battle.columns.ids.iter().copied());
    identities.extend(battle.live.historical_players());
    battle.live.validate_items(registry)?;
    let known = |id: EntityId| identities.contains(&id);
    for (_, target, _) in battle
        .columns
        .ids
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, battle.columns.targets[i], battle.columns.escape[i]))
    {
        if let Some(target) = target
            && !known(target.id)
        {
            return Err(error(format!("unknown target {}", target.id.0)));
        }
    }
    battle.combat.validate_references(&known, registry)?;
    battle.retaliation.validate_references(&known)?;
    Ok(())
}

pub(crate) fn restore(state: &mut TickState) -> Result<(), CoreError> {
    let high_water = state
        .hostile
        .battle
        .formation
        .members
        .iter()
        .map(|m| m.id.0)
        .chain(state.hostile.battle.columns.ids.iter().map(|id| id.0))
        .chain(
            state
                .hostile
                .battle
                .live
                .historical_players()
                .map(|id| id.0),
        )
        .max()
        .unwrap_or(0);
    state.entities.set_next_wire_id(high_water);
    for member in &mut state.hostile.battle.formation.members {
        member.slot = state.entities.slot_of(member.id);
        if let Some(slot) = member.slot {
            member.pos = state.entities.pos(slot);
        }
    }
    state.hostile.battle.live.restore_connections();
    Ok(())
}

/// Checkpoint rows of every dimension. The capture also makes each journaled
/// battle's next record a full image: WAL batches after this tick then never
/// patch the checkpoint row.
pub(crate) fn checkpoint_rows(
    state: &mut TickState,
    tick: u64,
) -> Result<BTreeMap<String, NativeDomainRecord>, CoreError> {
    prepare_capture(state);
    for child in state.other_dimensions.values_mut() {
        prepare_capture(child);
    }
    std::iter::once(&*state)
        .chain(state.other_dimensions.values().map(|state| &**state))
        .map(|state| {
            Ok((
                row_key(state.dimension),
                crate::native_checkpoint::record(tick, encode(state)?.into_bytes()),
            ))
        })
        .collect()
}

fn prepare_capture(state: &mut TickState) {
    reconcile_native_capture(state);
    if let Some(journaled) = &mut state.hostile.battle.journaled {
        journaled.full_due = true;
    }
}

pub(crate) fn checkpoint_image(
    globals: &BTreeMap<String, NativeDomainRecord>,
    dimension: Dimension,
    tick: u64,
) -> Result<Option<Arc<String>>, CoreError> {
    for key in globals.keys().filter(|key| key.starts_with("battle/")) {
        if !matches!(key.as_str(), "battle/0" | "battle/1" | "battle/2") {
            return Err(error(format!("unsupported checkpoint row {key}")));
        }
    }
    let key = row_key(dimension);
    let Some(record) = globals.get(&key) else {
        return Ok(None);
    };
    if record.version != 1 || record.captured_tick != tick {
        return Err(error(format!("invalid checkpoint row {key}")));
    }
    let image = String::from_utf8(record.payload.clone()).map_err(error)?;
    decode(&image)?;
    Ok(Some(Arc::new(image)))
}

/// One battle record per changed tick: the whole image when one is due,
/// otherwise a patch of the changed parts. An unchanged image journals nothing.
pub(crate) fn journal_entries(
    state: &mut TickState,
    entries: &mut Vec<JournalEntry>,
) -> Result<(), CoreError> {
    if state.hostile.battle.formation.members.is_empty()
        && state.hostile.battle.live.is_empty()
        && state.hostile.battle.journaled.is_none()
    {
        return Ok(());
    }
    reconcile_native_capture(state);
    let mut journaled = state
        .hostile
        .battle
        .journaled
        .take()
        .unwrap_or_else(Journaled::full_due);
    let record = capture(state, &mut journaled.parts).and_then(|patch| {
        if patch.is_empty() {
            Ok(None)
        } else if journaled.full_due {
            Ok(Some(JournalEntry::Battle {
                value: Arc::new(encode(state)?),
            }))
        } else {
            Ok(Some(JournalEntry::BattlePatch {
                value: serde_json::to_value(patch).map_err(error)?,
            }))
        }
    });
    let result = match record {
        Ok(record) => {
            if let Some(record) = record {
                entries.push(record);
                journaled.full_due = false;
            }
            Ok(())
        }
        // The captured parts may now hold unjournaled state: start over.
        Err(error) => {
            journaled = Journaled::full_due();
            Err(error)
        }
    };
    state.hostile.battle.journaled = Some(journaled);
    result
}

/// Fold state of one dimension's battle records in the WAL tail (`Replay::battle_tail`).
#[derive(Debug, Default)]
pub(crate) enum Tail {
    /// No battle record folded: `Replay::battle` is the checkpoint row, which
    /// a patch never builds on.
    #[default]
    Checkpoint,
    /// `Replay::battle` is the latest full WAL image.
    Full,
    /// That image with every later patch applied.
    Patched(Value),
}

pub(crate) fn fold_full(replay: &mut Replay, value: &Arc<String>) {
    replay.battle = Some(Arc::clone(value));
    replay.battle_tail = Tail::Full;
}

/// Apply one WAL patch to the replayed image; any refusal becomes the replay's
/// battle fault, which recovery reports before decoding anything.
pub(crate) fn fold_patch(replay: &mut Replay, value: &Value) {
    if replay.battle_fault.is_some() {
        return;
    }
    let folded = Patch::deserialize(value)
        .map_err(|e| format!("malformed battle patch: {e}"))
        .and_then(|patch| {
            let mut image = match std::mem::take(&mut replay.battle_tail) {
                Tail::Checkpoint => {
                    return Err("battle patch without a full image earlier in the WAL tail".into());
                }
                Tail::Full => {
                    serde_json::from_str(replay.battle.as_deref().map_or("", String::as_str))
                        .map_err(|e| format!("malformed battle image: {e}"))?
                }
                Tail::Patched(image) => image,
            };
            patch.apply(&mut image)?;
            Ok(image)
        });
    match folded {
        Ok(image) => replay.battle_tail = Tail::Patched(image),
        Err(fault) => replay.battle_fault = Some(fault),
    }
}

/// The recovered image: the checkpoint row, or the latest full WAL image with
/// its later patches.
pub(crate) fn image(replay: &Replay) -> Option<Arc<String>> {
    match &replay.battle_tail {
        Tail::Patched(image) => Some(Arc::new(image.to_string())),
        Tail::Checkpoint | Tail::Full => replay.battle.clone(),
    }
}

pub(super) mod reach {
    use serde::{Deserialize, Serialize};
    pub fn serialize<S: serde::Serializer>(
        value: &mc_melee_range::MeleeReach,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        [value.min, value.max].serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<mc_melee_range::MeleeReach, D::Error> {
        let [min, max] = <[f64; 2]>::deserialize(deserializer)?;
        if !(0.0..=64.0).contains(&min) || !(min..=64.0).contains(&max) {
            return Err(serde::de::Error::custom("invalid melee reach"));
        }
        Ok(mc_melee_range::MeleeReach { min, max })
    }
}

pub(super) mod distance {
    use serde::{Deserialize, Serialize};
    pub fn serialize<S: serde::Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        value.is_finite().then_some(*value).serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        Ok(Option::<f64>::deserialize(deserializer)?.unwrap_or(f64::INFINITY))
    }
}

pub(super) mod volley_map {
    use super::super::archery::Volley;
    use rustc_hash::FxHashMap;
    use serde::{Deserialize, Serialize};
    pub fn serialize<S: serde::Serializer>(
        value: &FxHashMap<(u16, u32), Volley>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut rows: Vec<_> = value.iter().collect();
        rows.sort_unstable_by_key(|row| row.0);
        rows.serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<FxHashMap<(u16, u32), Volley>, D::Error> {
        let rows = Vec::<((u16, u32), Volley)>::deserialize(deserializer)?;
        let mut result = FxHashMap::default();
        for (key, value) in rows {
            if result.insert(key, value).is_some() {
                return Err(serde::de::Error::custom("duplicate volley"));
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
