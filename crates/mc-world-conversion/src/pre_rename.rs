//! Field names of an Anvil export written before the Krasnokamen rename.
//!
//! A 1.0.0-rc.5 (or older) server writes its own NBT fields under the old name:
//! `solaris:WireId`, `solaris:PortalTime`, `SolarisUnlockedRecipes`. The rename
//! (`tools/dev/rebrand.py`) changed every such name to the Krasnokamen spelling
//! and nothing else, so import renames the fields back to what this build reads
//! instead of keeping them as unread extra data. One field also changed shape
//! since: the scalar rider `solaris:Rider` (a player UUID) is now the ordered
//! list `krasnokamen:Riders`.
//!
//! Only field names are renamed. String values in the old `solaris:` namespace
//! are left as they are and reported: some are hash domains that the rename
//! kept on purpose, the rest named native-only state the export does not carry.

use super::fail;
use mc_core::CoreError;
use mc_nbt::{ListTag, Tag, tag_type};
use serde::{Deserialize, Serialize};

/// What the renaming did across one import, for the receipt.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct PreRename {
    /// Fields renamed to their Krasnokamen name.
    pub keys: u64,
    /// String values left in the old `solaris:` namespace.
    pub values: u64,
    /// Record and field of the first such value.
    pub first_value: Option<String>,
}

impl PreRename {
    /// Rename the pre-rename fields of `tag`. `record` names the source record
    /// (a path in the source world) for errors and the receipt; it is only
    /// formatted when one of them needs it.
    pub(crate) fn rename(
        &mut self,
        tag: &mut Tag,
        record: &dyn Fn() -> String,
    ) -> Result<(), CoreError> {
        self.walk(tag, record, "")
    }

    fn walk(
        &mut self,
        tag: &mut Tag,
        record: &dyn Fn() -> String,
        field: &str,
    ) -> Result<(), CoreError> {
        match tag {
            Tag::Compound(fields) => {
                let mut renamed = false;
                for (key, value) in fields.iter_mut() {
                    if let Some(name) = current_name(key) {
                        if name == "krasnokamen:Rider" {
                            *value = riders(std::mem::replace(value, Tag::Byte(0)), record)?;
                            *key = "krasnokamen:Riders".into();
                        } else {
                            *key = name;
                        }
                        self.keys += 1;
                        renamed = true;
                    }
                    self.walk(value, record, key)?;
                }
                if renamed {
                    let mut names: Vec<&str> = fields.iter().map(|(key, _)| key.as_str()).collect();
                    names.sort_unstable();
                    if let Some(pair) = names.windows(2).find(|pair| pair[0] == pair[1]) {
                        return Err(fail(format!(
                            "{}: field {} is present under both its pre-rename and its current name",
                            record(),
                            pair[0]
                        )));
                    }
                }
            }
            Tag::List(list) => {
                for element in &mut list.elements {
                    self.walk(element, record, field)?;
                }
            }
            Tag::String(value) if value.starts_with("solaris:") => {
                self.values += 1;
                if self.first_value.is_none() {
                    self.first_value = Some(format!("{} field {field}", record()));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// The current name of a pre-rename field, or `None` for any other field.
fn current_name(key: &str) -> Option<String> {
    (key.contains("solaris") || key.contains("Solaris") || key.contains("SOLARIS")).then(|| {
        key.replace("solaris", "krasnokamen")
            .replace("Solaris", "Krasnokamen")
            .replace("SOLARIS", "KRASNOKAMEN")
    })
}

/// RC5's single player rider as the current ordered rider list.
fn riders(rider: Tag, record: &dyn Fn() -> String) -> Result<Tag, CoreError> {
    if !matches!(&rider, Tag::IntArray(uuid) if uuid.len() == 4) {
        return Err(fail(format!(
            "{}: pre-rename solaris:Rider is not a UUID",
            record()
        )));
    }
    Ok(Tag::List(ListTag {
        element_type: tag_type::INT_ARRAY,
        elements: vec![rider],
    }))
}

#[cfg(test)]
#[path = "pre_rename_tests.rs"]
mod tests;
