//! Saved structure starts checked against the structures the selected packs provide.

use std::collections::{BTreeMap, BTreeSet};

use mc_data::Identifier;

/// Saved starts remain opaque durable data when their pack is absent.
#[derive(Debug)]
pub struct StructureCatalog {
    pub active: BTreeSet<Identifier>,
    pub owners: BTreeMap<String, String>,
    pub warned: parking_lot::Mutex<BTreeSet<Identifier>>,
}

impl StructureCatalog {
    /// Catalog of the selected packs; nothing is warned yet.
    #[must_use]
    pub fn new(active: BTreeSet<Identifier>, owners: BTreeMap<String, String>) -> Self {
        Self {
            active,
            owners,
            warned: parking_lot::Mutex::new(BTreeSet::new()),
        }
    }

    pub fn observe(&self, chunk: &mc_world::Chunk) -> Result<(), String> {
        for site in mc_world::structure_starts::read(&chunk.extras, chunk.pos, false)? {
            if !self.active.contains(&site.structure)
                && self.warned.lock().insert(site.structure.clone())
            {
                let pack = self
                    .owners
                    .get(site.structure.as_str())
                    .map(String::as_str)
                    .unwrap_or("unknown source pack; restore the structure's namespace");
                tracing::warn!(structure = %site.structure, likely_missing_pack = pack,
                    "saved structure start is inactive; retained unchanged until its pack returns");
            }
        }
        Ok(())
    }
}
