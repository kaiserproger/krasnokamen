//! Raids SAVED_DATA payload upgrade.

use super::*;

/// Port of vanilla 26.1.2 `RaidRenamesDataFix` (registered at DataVersion
/// 4309 for `SAVED_DATA_RAIDS`; javap in
/// `.analysis/codex-logs/constantinople/aux-2586-raid-javap.txt`).
/// Renames 1.16.5 PascalCase keys to the snake_case codec fields and folds
/// inline `CX/CY/CZ` into `center`. Missing keys are left alone (DFU
/// `renameField` is a no-op when absent); a non-empty `Raids` list whose
/// entries lack every known shape fails closed downstream in `raids::from_nbt`.
#[cfg(test)]
pub(crate) fn upgrade_raids_payload_for_tests(
    data: &mut Vec<(String, Tag)>,
    version: i32,
    steps: &mut Vec<super::super::UpgradeFixReceipt>,
) {
    upgrade_raids_payload(data, version, steps);
}

pub(super) fn upgrade_raids_payload(
    data: &mut Vec<(String, Tag)>,
    version: i32,
    steps: &mut Vec<super::super::UpgradeFixReceipt>,
) {
    if version >= 4309 {
        return;
    }
    let mut renamed = 0_u32;
    let rename = |fields: &mut Vec<(String, Tag)>, old: &str, new: &str, counter: &mut u32| {
        if let Some(index) = fields.iter().position(|(name, _)| name == old)
            && !fields.iter().any(|(name, _)| name == new)
        {
            fields[index].0 = new.into();
            *counter += 1;
        }
    };
    rename(data, "Raids", "raids", &mut renamed);
    rename(data, "Tick", "tick", &mut renamed);
    rename(data, "NextAvailableID", "next_id", &mut renamed);
    if let Some((_, Tag::List(list))) = data.iter_mut().find(|(name, _)| name == "raids") {
        for entry in &mut list.elements {
            let Tag::Compound(fields) = entry else {
                continue;
            };
            // Vanilla `fixInlineBlockPos(CX, CY, CZ -> center)`: only when all
            // three parts read as numbers; otherwise the entry stays for the
            // typed reader to reject.
            let center = ["CX", "CY", "CZ"].iter().all(|key| {
                fields.iter().any(|(name, value)| {
                    name == key
                        && matches!(
                            value,
                            Tag::Byte(_)
                                | Tag::Short(_)
                                | Tag::Int(_)
                                | Tag::Long(_)
                                | Tag::Float(_)
                                | Tag::Double(_)
                        )
                })
            });
            if center && !fields.iter().any(|(name, _)| name == "center") {
                let mut coords = Vec::new();
                for key in ["CX", "CY", "CZ"] {
                    let index = fields
                        .iter()
                        .position(|(name, _)| name == key)
                        .expect("checked");
                    let (_, value) = fields.remove(index);
                    coords.push(match value {
                        Tag::Byte(v) => i32::from(v),
                        Tag::Short(v) => i32::from(v),
                        Tag::Int(v) => v,
                        Tag::Long(v) => v as i32,
                        Tag::Float(v) => v as i32,
                        Tag::Double(v) => v as i32,
                        _ => 0,
                    });
                }
                fields.push(("center".into(), Tag::IntArray(coords)));
                renamed += 1;
            }
            for (old, new) in [
                ("Id", "id"),
                ("Started", "started"),
                ("Active", "active"),
                ("TicksActive", "ticks_active"),
                ("BadOmenLevel", "raid_omen_level"),
                ("GroupsSpawned", "groups_spawned"),
                ("PreRaidTicks", "cooldown_ticks"),
                ("PostRaidTicks", "post_raid_ticks"),
                ("TotalHealth", "total_health"),
                ("NumGroups", "group_count"),
                ("Status", "status"),
                ("HeroesOfTheVillage", "heroes_of_the_village"),
            ] {
                rename(fields, old, new, &mut renamed);
            }
        }
    }
    if renamed > 0 {
        steps.push(super::super::UpgradeFixReceipt {
            version: 4309,
            fix: "RaidRenamesDataFix".into(),
        });
    }
}
