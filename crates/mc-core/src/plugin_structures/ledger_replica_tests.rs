//! A cold WAL replica of the structure ledger for tests: a checkpoint image of
//! the plugin state plus every journaled `Plugins` delta, folded from their
//! serialized bytes exactly as recovery folds them. `check` compares it with
//! the live decoded cache, which holds every in-memory mutation, so a write
//! that missed its mark (present in the cache, absent from the WAL) shows up
//! even though the live domain and the replica would agree with each other.

use std::collections::BTreeMap;

use super::{Ledger, META, OWNER, SCHEMA, STRUCTURE, SURVEY, WORKSITE};
use crate::tick::TickState;

pub(crate) struct Replica {
    plugins: crate::plugin::PluginState,
    tick: u64,
}

/// One journaled boundary: the structure-ledger keys it carried (`true` for a
/// post-image, `false` for a removal) and the WAL bytes of its entries.
pub(crate) struct Boundary {
    pub(crate) keys: BTreeMap<String, bool>,
    pub(crate) bytes: Vec<u8>,
}

impl Replica {
    /// Seals what `state` has pending, then starts from the bytes a
    /// checkpoint stores for its plugin state.
    pub(crate) fn capture(state: &mut TickState) -> Self {
        let mut sealed = Vec::new();
        crate::plugin::journal(state, &mut sealed);
        let bytes = serde_json::to_vec(&state.plugins).unwrap();
        Self {
            plugins: serde_json::from_slice(&bytes).unwrap(),
            tick: state.tick,
        }
    }

    /// The replica's plugin state, as recovery would hand it to a new tick owner.
    pub(crate) fn plugins(&self) -> crate::plugin::PluginState {
        self.plugins.clone()
    }

    /// Journals one tick boundary of `state` and folds the batch bytes.
    pub(crate) fn step(&mut self, state: &mut TickState) -> Boundary {
        let mut entries = Vec::new();
        crate::plugin::journal(state, &mut entries);
        let bytes = serde_json::to_vec(&entries).unwrap();
        let mut keys = BTreeMap::new();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for entry in value.as_array().unwrap() {
            let Some(domain) = entry
                .pointer("/Plugins/value/domains")
                .and_then(|domains| domains.get(OWNER))
                .and_then(serde_json::Value::as_object)
            else {
                continue;
            };
            for (key, image) in domain {
                keys.insert(key.clone(), image.get("Present").is_some());
            }
        }
        let entries: Vec<crate::journal::JournalEntry> = serde_json::from_slice(&bytes).unwrap();
        self.tick += 1;
        let replay = crate::journal::Replay {
            plugins: std::mem::take(&mut self.plugins),
            ..Default::default()
        };
        self.plugins =
            crate::journal::replay_batches(replay, &[(self.tick, self.tick, entries)]).plugins;
        Boundary { keys, bytes }
    }

    /// The replica holds exactly the live cache's records and meta, and decodes
    /// cold on its own.
    pub(crate) fn check(&self, state: &TickState) -> Result<(), String> {
        let stored: BTreeMap<String, serde_json::Value> = self
            .plugins
            .domain_entries(OWNER)
            .map(|(key, value)| (key.to_owned(), value.clone()))
            .collect();
        let warm = state.plugins.structure_cache.decoded.get().is_some();
        if !warm && state.plugins.domain_entries(OWNER).next().is_some() {
            return Err(
                "the live ledger cache is cold: a check would only reread the domain".into(),
            );
        }
        let live = super::super::ledger(state).map_err(|error| format!("live: {error:?}"))?;
        let mut expected = BTreeMap::new();
        if !stored.is_empty() || live.revision != 0 {
            expected.insert(
                META.to_owned(),
                serde_json::json!({"schema": SCHEMA, "revision": live.revision}),
            );
        }
        let mut add = |prefix: &str, id: &str, value: serde_json::Value| {
            expected.insert(format!("{prefix}{id}"), value);
        };
        for (id, record) in live.structures() {
            add(STRUCTURE, id, serde_json::to_value(record).unwrap());
        }
        for (token, survey) in live.surveys() {
            add(SURVEY, token, serde_json::to_value(survey).unwrap());
        }
        for (id, worksite) in live.worksites() {
            add(WORKSITE, id, serde_json::to_value(worksite).unwrap());
        }
        if stored != expected {
            let differing: Vec<_> = expected
                .keys()
                .chain(stored.keys())
                .filter(|key| expected.get(*key) != stored.get(*key))
                .collect();
            return Err(format!(
                "the WAL replica differs from the live cache at {differing:?}"
            ));
        }
        Ledger::decode(self.plugins.domain_entries(OWNER))
            .map(|_| ())
            .map_err(|error| format!("the replica does not decode: {error:?}"))
    }

    pub(crate) fn assert_matches(&self, state: &TickState) {
        if let Err(error) = self.check(state) {
            panic!("{error}");
        }
    }
}
