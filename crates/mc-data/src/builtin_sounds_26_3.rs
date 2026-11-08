//! Built-in sound registry protocol order for the canonical 26.3 profile.
//! Facts projected from the versioned vanilla registry report; names are the
//! identity, and numeric IDs belong only to this protocol profile.
use std::collections::HashMap;
use std::sync::LazyLock;

/// Canonical name; gameplay need not initialize the wire-ID catalogue to use it.
pub const CHORUS_FRUIT_TELEPORT: &str = "minecraft:item.chorus_fruit.teleport";

static NAMES: LazyLock<Vec<String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/builtin_sounds_26_3.json"))
        .expect("embedded 26.3 sound index")
});
static IDS: LazyLock<HashMap<&'static str, u32>> = LazyLock::new(|| {
    NAMES
        .iter()
        .enumerate()
        .map(|(id, name)| (name.as_str(), id as u32))
        .collect()
});

pub fn names() -> &'static [String] {
    &NAMES
}

pub fn name(id: u32) -> Option<&'static str> {
    names().get(id as usize).map(String::as_str)
}

pub fn id(name: &str) -> Option<u32> {
    IDS.get(name).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_sound_index_is_dense_and_bidirectional() {
        assert!(!names().is_empty());
        for (raw, value) in names().iter().enumerate() {
            assert_eq!(name(raw as u32), Some(value.as_str()));
            assert_eq!(id(value), Some(raw as u32));
        }
        assert_eq!(name(names().len() as u32), None);
        assert_eq!(id("minecraft:not_a_registered_sound"), None);
    }
}
