//! Battle decision history shares native entity custody's checkpoint/WAL fence.
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

pub(crate) fn encode(state: &TickState) -> Result<String, CoreError> {
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
    let battle = &state.hostile.battle;
    let columns: Vec<Column> = battle
        .columns
        .ids
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, battle.columns.targets[i], battle.columns.escape[i]))
        .collect();
    // A death merged after the battle pass is already a native tombstone.
    let removed: Vec<_> = battle
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
        .collect();
    serde_json::to_string(&Image {
        version: 1,
        battle,
        columns,
        removed,
    })
    .map_err(error)
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
            || (spec.facing[0].hypot(spec.facing[1]) - 1.0).abs() > 1e-6
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
    let Some(image) = &replay.battle else {
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
    let battle = decode(image)?;
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

pub(crate) fn checkpoint_rows(
    state: &TickState,
    tick: u64,
) -> Result<BTreeMap<String, NativeDomainRecord>, CoreError> {
    std::iter::once(state)
        .chain(state.other_dimensions.values().map(|state| &**state))
        .map(|state| {
            Ok((
                row_key(state.dimension),
                crate::native_checkpoint::record(tick, encode(state)?.into_bytes()),
            ))
        })
        .collect()
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
    let image = encode(state)?;
    if state
        .hostile
        .battle
        .journaled
        .as_ref()
        .is_some_and(|old| **old == image)
    {
        return Ok(());
    }
    let image = Arc::new(image);
    entries.push(JournalEntry::Battle {
        value: Arc::clone(&image),
    });
    state.hostile.battle.journaled = Some(image);
    Ok(())
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
