//! Persisted layout of the structure ledger: one `@native-structures` domain
//! key per survey, structure and worksite, plus the `ledger` meta key with the
//! schema and the global revision. A write publishes only the records it
//! touched, so a tick journals those records and the meta key, never the
//! world's whole construction history.
//!
//! The maps are private to this module: every mutable path marks its record
//! key, and `publish` writes exactly the marked keys. The meta key keeps the
//! name the single-image ledger used, in a shape that image's decoder refuses
//! (it lacks `surveys`, `structures` and `worksites`), so an older binary
//! never reads a split ledger as empty.

use std::collections::{BTreeMap, BTreeSet};

use mc_script::ScriptOperationFailure as Failure;
use serde::{Deserialize, Serialize};

use super::{OWNER, Structure, Survey, Worksite};

/// The meta key: `{"schema": 2, "revision": N}`.
pub(super) const META: &str = "ledger";
const SCHEMA: u32 = 2;
const STRUCTURE: &str = "structure/";
const SURVEY: &str = "survey/";
const WORKSITE: &str = "worksite/";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Meta {
    schema: u32,
    revision: u64,
}

#[derive(Clone, Default)]
pub(super) struct Ledger {
    pub(super) revision: u64,
    surveys: BTreeMap<String, Survey>,
    structures: BTreeMap<String, Structure>,
    worksites: BTreeMap<String, Worksite>,
    /// Record keys written since the last publish; never persisted.
    touched: BTreeSet<String>,
}

pub(super) fn structure_key(id: &str) -> String {
    format!("{STRUCTURE}{id}")
}

/// The single-image ledger written before the split carries its maps in the
/// meta key; this binary refuses it with its own diagnostic.
pub(super) fn legacy(meta: Option<&serde_json::Value>) -> bool {
    meta.and_then(|value| value.get("structures")).is_some()
}

impl Ledger {
    pub(super) fn surveys(&self) -> &BTreeMap<String, Survey> {
        &self.surveys
    }

    pub(super) fn structures(&self) -> &BTreeMap<String, Structure> {
        &self.structures
    }

    pub(super) fn worksites(&self) -> &BTreeMap<String, Worksite> {
        &self.worksites
    }

    pub(super) fn insert_survey(&mut self, token: String, survey: Survey) {
        self.touched.insert(format!("{SURVEY}{token}"));
        self.surveys.insert(token, survey);
    }

    /// Drops the surveys `keep` rejects; each becomes a removed key.
    pub(super) fn retain_surveys(&mut self, keep: impl Fn(&Survey) -> bool) {
        let touched = &mut self.touched;
        self.surveys.retain(|token, survey| {
            let kept = keep(survey);
            if !kept {
                touched.insert(format!("{SURVEY}{token}"));
            }
            kept
        });
    }

    pub(super) fn insert_structure(&mut self, id: String, record: Structure) {
        self.touched.insert(structure_key(&id));
        self.structures.insert(id, record);
    }

    /// Removes a record; unless it is inserted again, publish removes its key.
    pub(super) fn take_structure(&mut self, id: &str) -> Option<Structure> {
        let record = self.structures.remove(id)?;
        self.touched.insert(structure_key(id));
        Some(record)
    }

    pub(super) fn structure_mut(&mut self, id: &str) -> Option<&mut Structure> {
        let record = self.structures.get_mut(id)?;
        self.touched.insert(structure_key(id));
        Some(record)
    }

    /// Every record `select` accepts, each marked as written.
    pub(super) fn structures_mut_where(
        &mut self,
        select: impl Fn(&Structure) -> bool,
    ) -> impl Iterator<Item = &mut Structure> {
        let touched = &mut self.touched;
        self.structures
            .iter_mut()
            .filter(move |(_, record)| select(record))
            .map(move |(id, record)| {
                touched.insert(structure_key(id));
                record
            })
    }

    pub(super) fn insert_worksite(&mut self, id: String, worksite: Worksite) {
        self.touched.insert(format!("{WORKSITE}{id}"));
        self.worksites.insert(id, worksite);
    }

    /// Writes every touched record key (removed when its record is gone) and
    /// the meta key, then forgets the marks. All of them land in the current
    /// tick's plugin delta, which the WAL publishes as one batch.
    pub(super) fn publish(&mut self, plugins: &mut crate::plugin::PluginState) {
        for key in std::mem::take(&mut self.touched) {
            let value = if let Some(id) = key.strip_prefix(STRUCTURE) {
                self.structures.get(id).map(serde_json::to_value)
            } else if let Some(token) = key.strip_prefix(SURVEY) {
                self.surveys.get(token).map(serde_json::to_value)
            } else if let Some(id) = key.strip_prefix(WORKSITE) {
                self.worksites.get(id).map(serde_json::to_value)
            } else {
                unreachable!("ledger marks only record keys")
            };
            match value {
                Some(value) => plugins.set_domain(
                    OWNER,
                    &key,
                    value.expect("native structure record serialization"),
                ),
                None => plugins.delete_domain(OWNER, &key),
            }
        }
        let meta = Meta {
            schema: SCHEMA,
            revision: self.revision,
        };
        plugins.set_domain(
            OWNER,
            META,
            serde_json::to_value(meta).expect("native structure meta serialization"),
        );
    }

    /// Assembles the ledger from every key of the owner. No key at all is a
    /// fresh world; records without the meta key, another schema, an unknown
    /// key or an unknown record field refuse.
    pub(super) fn decode<'a>(
        entries: impl Iterator<Item = (&'a str, &'a serde_json::Value)>,
    ) -> Result<Self, Failure> {
        let malformed = |_| Failure::RuntimeUnavailable;
        let mut ledger = Self::default();
        let mut meta = false;
        let mut records = false;
        for (key, value) in entries {
            if key == META {
                let Meta { schema, revision } = Meta::deserialize(value).map_err(malformed)?;
                if schema != SCHEMA {
                    return Err(Failure::RuntimeUnavailable);
                }
                ledger.revision = revision;
                meta = true;
                continue;
            }
            records = true;
            if let Some(id) = key.strip_prefix(STRUCTURE) {
                let record = Structure::deserialize(value).map_err(malformed)?;
                ledger.structures.insert(id.to_owned(), record);
            } else if let Some(token) = key.strip_prefix(SURVEY) {
                let survey = Survey::deserialize(value).map_err(malformed)?;
                ledger.surveys.insert(token.to_owned(), survey);
            } else if let Some(id) = key.strip_prefix(WORKSITE) {
                let worksite = Worksite::deserialize(value).map_err(malformed)?;
                ledger.worksites.insert(id.to_owned(), worksite);
            } else {
                return Err(Failure::RuntimeUnavailable);
            }
        }
        if records && !meta {
            return Err(Failure::RuntimeUnavailable);
        }
        #[cfg(test)]
        if meta {
            super::count_ledger(|stats| stats.decodes += 1);
        }
        Ok(ledger)
    }
}

#[cfg(test)]
impl Ledger {
    /// A record mutation that skips its write mark: only for proving that the
    /// WAL replica check catches a missed mark.
    pub(super) fn structure_mut_unmarked(&mut self, id: &str) -> Option<&mut Structure> {
        self.structures.get_mut(id)
    }
}

#[cfg(test)]
#[path = "ledger_replica_tests.rs"]
pub(crate) mod replica;
