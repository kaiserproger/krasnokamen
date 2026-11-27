//! Tab-completion suggestions.

use super::*;

/// Vanilla caps one suggestion response at 256 entries.
const MAX_SUGGESTIONS: usize = 256;

/// Push `candidate` when it matches what the player typed. A namespaced
/// candidate also matches a bare path (`minecraft:stone` for `st`).
fn push_suggestion(suggestions: &mut Vec<CommandSuggestionEntry>, prefix: &str, candidate: &str) {
    if suggestions.len() >= MAX_SUGGESTIONS {
        return;
    }
    let matches = candidate.starts_with(prefix)
        || (!prefix.contains(':')
            && candidate
                .strip_prefix("minecraft:")
                .is_some_and(|path| path.starts_with(prefix)));
    if matches {
        suggestions.push(CommandSuggestionEntry {
            text: candidate.to_owned(),
            tooltip_nbt: None,
        });
    }
}

fn push_each<'a>(
    suggestions: &mut Vec<CommandSuggestionEntry>,
    prefix: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) {
    for candidate in candidates {
        push_suggestion(suggestions, prefix, candidate);
    }
}

/// `/item` and `/loot replace` slot arguments: block containers, or the
/// player's equipment, hotbar and inventory slots.
fn push_loot_replace_slots(
    words: &[&str],
    prefix: &str,
    out: &mut Vec<CommandSuggestionEntry>,
) -> bool {
    let root = words.first().copied();
    if !matches!(root, Some("item" | "loot")) || words.get(1).copied() != Some("replace") {
        return false;
    }
    let kind = words.get(2).copied();
    let destination_slot =
        (kind == Some("entity") && words.len() == 4) || (kind == Some("block") && words.len() == 6);
    let from = words.iter().position(|word| *word == "from");
    let source_kind = from.and_then(|at| words.get(at + 1).copied());
    let source_slot = from.zip(source_kind).is_some_and(|(at, source)| {
        let source_len = if source == "entity" { 2 } else { 4 };
        words.len() == at + 1 + source_len
    });
    if !destination_slot && !source_slot {
        return false;
    }
    let slot_owner = if source_slot { source_kind } else { kind };
    if slot_owner == Some("block") {
        for index in 0..54 {
            push_suggestion(out, prefix, &format!("container.{index}"));
        }
    } else {
        push_each(
            out,
            prefix,
            [
                "weapon.mainhand",
                "weapon.offhand",
                "armor.head",
                "armor.chest",
                "armor.legs",
                "armor.feet",
            ],
        );
        for index in 0..9 {
            push_suggestion(out, prefix, &format!("hotbar.{index}"));
        }
        for index in 0..27 {
            push_suggestion(out, prefix, &format!("inventory.{index}"));
        }
    }
    true
}

fn push_settlement_actions(
    root: &str,
    second: Option<&str>,
    argument_index: usize,
    prefix: &str,
    out: &mut Vec<CommandSuggestionEntry>,
) -> bool {
    if root != "settlement" {
        return false;
    }
    if argument_index == 1 {
        for &(action, _) in SETTLEMENT_HELP {
            push_suggestion(out, prefix, action);
        }
    } else if argument_index == 3
        && let Some(domain @ ("production" | "commerce" | "payroll" | "caravan")) = second
    {
        push_suggestion(out, prefix, "status");
        for &(_, action, _) in SETTLEMENT_ECONOMY.iter().filter(|entry| entry.0 == domain) {
            push_suggestion(out, prefix, action);
        }
    }
    true
}

fn push_effect_arguments(
    root: &str,
    second: Option<&str>,
    argument_index: usize,
    prefix: &str,
    out: &mut Vec<CommandSuggestionEntry>,
) -> bool {
    if root != "effect" {
        return false;
    }
    match (second, argument_index) {
        (_, 1) => {
            push_each(out, prefix, ["give", "clear"]);
            true
        }
        (Some("give" | "clear"), 3) => {
            for id in mc_core::effects::EffectId::ALL {
                push_suggestion(
                    out,
                    prefix,
                    mc_data::mob_effects_26_1_2::name_for_id(id as u32)
                        .expect("built-in mob-effect identifier"),
                );
            }
            true
        }
        (Some("give"), 4) => {
            push_suggestion(out, prefix, "infinite");
            true
        }
        (Some("give"), 6) => {
            push_each(out, prefix, ["false", "true"]);
            true
        }
        _ => false,
    }
}

fn push_tick_arguments(
    root: &str,
    second: Option<&str>,
    argument_index: usize,
    prefix: &str,
    out: &mut Vec<CommandSuggestionEntry>,
) -> bool {
    if root != "tick" {
        return false;
    }
    match (second, argument_index) {
        (_, 1) => push_each(
            out,
            prefix,
            ["query", "rate", "step", "sprint", "unfreeze", "freeze"],
        ),
        (Some("rate"), 2) => push_suggestion(out, prefix, "20"),
        // Vanilla merges the `stop` literal with the time argument's
        // `1t`/`1s` suggestions (TickCommand.java:35-44).
        (Some("step"), 2) => push_each(out, prefix, ["stop", "1t", "1s"]),
        // TickCommand.java:45-53: `stop` plus `60s`/`1d`/`3d`.
        (Some("sprint"), 2) => push_each(out, prefix, ["stop", "60s", "1d", "3d"]),
        _ => {}
    }
    true
}

fn push_bossbar_arguments(
    root: &str,
    second: Option<&str>,
    argument_index: usize,
    words: &[&str],
    prefix: &str,
    out: &mut Vec<CommandSuggestionEntry>,
) -> bool {
    if root != "bossbar" {
        return false;
    }
    match (second, argument_index) {
        (_, 1) => {
            push_each(out, prefix, ["add", "remove", "list", "set", "get"]);
            true
        }
        (Some("set"), 3) => {
            push_each(
                out,
                prefix,
                [
                    "name", "color", "style", "value", "max", "visible", "players",
                ],
            );
            true
        }
        // Words are root,set,id,field: the field is the fourth word.
        (Some("set"), 4) => match words.get(3).copied() {
            Some("color") => {
                push_each(
                    out,
                    prefix,
                    ["pink", "blue", "red", "green", "yellow", "purple", "white"],
                );
                true
            }
            Some("style") => {
                push_each(
                    out,
                    prefix,
                    [
                        "progress",
                        "notched_6",
                        "notched_10",
                        "notched_12",
                        "notched_20",
                    ],
                );
                true
            }
            Some("visible") => {
                push_each(out, prefix, ["false", "true"]);
                true
            }
            _ => false,
        },
        (Some("get"), 3) => {
            push_each(out, prefix, ["value", "max", "visible", "players"]);
            true
        }
        _ => false,
    }
}

fn push_locate_arguments(
    root: &str,
    second: Option<&str>,
    argument_index: usize,
    structure_ids: &[String],
    prefix: &str,
    out: &mut Vec<CommandSuggestionEntry>,
) -> bool {
    if root != "locate" {
        return false;
    }
    match (second, argument_index) {
        (_, 1) => push_each(out, prefix, ["structure", "biome", "poi"]),
        (Some("structure"), 2) => {
            push_each(out, prefix, structure_ids.iter().map(String::as_str));
        }
        _ => {}
    }
    true
}

fn push_title_arguments(
    root: &str,
    second: Option<&str>,
    argument_index: usize,
    player_names: &[String],
    prefix: &str,
    out: &mut Vec<CommandSuggestionEntry>,
) -> bool {
    if root != "title" {
        return false;
    }
    match (second, argument_index) {
        (_, 1) => {
            push_each(out, prefix, ["@a", "@p", "@r", "@s"]);
            push_each(out, prefix, player_names.iter().map(String::as_str));
        }
        (_, 2) => push_each(
            out,
            prefix,
            ["clear", "reset", "title", "subtitle", "actionbar", "times"],
        ),
        _ => {}
    }
    true
}

fn push_tellraw_arguments(
    root: &str,
    argument_index: usize,
    player_names: &[String],
    prefix: &str,
    out: &mut Vec<CommandSuggestionEntry>,
) -> bool {
    if root != "tellraw" {
        return false;
    }
    if argument_index == 1 {
        push_each(out, prefix, ["@a", "@p", "@r", "@s"]);
        push_each(out, prefix, player_names.iter().map(String::as_str));
    }
    true
}

fn push_damage_arguments(
    root: &str,
    argument_index: usize,
    words: &[&str],
    player_names: &[String],
    prefix: &str,
    out: &mut Vec<CommandSuggestionEntry>,
) -> bool {
    if root != "damage" {
        return false;
    }
    match argument_index {
        1 => {
            push_each(out, prefix, ["@p", "@r", "@s", "@e[limit=1]"]);
            push_each(out, prefix, player_names.iter().map(String::as_str));
        }
        3 => {
            for id in 0.. {
                let Some(name) = mc_data::required_registry_entry("damage_type", id) else {
                    break;
                };
                if mc_core::survival::DamageKind::command_type(name.as_str()).is_some() {
                    push_suggestion(out, prefix, name.as_str());
                }
            }
        }
        4 => push_each(out, prefix, ["at", "by"]),
        5 | 7 if words.get(4).copied() == Some("by") => {
            push_each(out, prefix, ["@p", "@r", "@s", "@e[limit=1]"]);
            push_each(out, prefix, player_names.iter().map(String::as_str));
        }
        6 if words.get(4).copied() == Some("by") => push_suggestion(out, prefix, "from"),
        _ => {}
    }
    true
}

fn push_sound_or_particle_names(
    root: &str,
    argument_index: usize,
    prefix: &str,
    out: &mut Vec<CommandSuggestionEntry>,
) -> bool {
    if argument_index != 1 {
        return false;
    }
    let names: &[&str] = match root {
        "playsound" => &[
            "minecraft:block.bell.use",
            "minecraft:entity.player.levelup",
            "minecraft:entity.experience_orb.pickup",
        ],
        "particle" => &[
            "minecraft:flame",
            "minecraft:smoke",
            "minecraft:heart",
            "minecraft:cloud",
            "minecraft:sweep_attack",
        ],
        _ => return false,
    };
    push_each(out, prefix, names.iter().copied());
    true
}

fn push_tag_actions(
    root: &str,
    argument_index: usize,
    prefix: &str,
    out: &mut Vec<CommandSuggestionEntry>,
) -> bool {
    if root != "tag" || argument_index != 2 {
        return false;
    }
    push_each(out, prefix, ["add", "remove", "list"]);
    true
}

/// Registry arguments that complete a typed id list.
#[allow(clippy::too_many_arguments)]
fn push_registry_ids(
    root: &str,
    second: Option<&str>,
    argument_index: usize,
    item_ids: &[String],
    entity_ids: &[String],
    block_ids: &[String],
    prefix: &str,
    out: &mut Vec<CommandSuggestionEntry>,
) -> bool {
    let ids: &[String] = match (root, second, argument_index) {
        ("give" | "clear", _, 2) | ("debug", Some("give"), 2) => item_ids,
        ("summon", _, 1) => entity_ids,
        ("setblock", _, 4) | ("fill", _, 7) | ("fill", _, 9) => block_ids,
        _ => return false,
    };
    push_each(out, prefix, ids.iter().map(String::as_str));
    true
}

/// Argument positions that complete player selectors and names.
fn selector_position(root: &str, second: Option<&str>, argument_index: usize) -> bool {
    let named_target = matches!(
        root,
        "msg"
            | "tell"
            | "w"
            | "give"
            | "clear"
            | "tag"
            | "kill"
            | "kick"
            | "ban"
            | "mute"
            | "unmute"
            | "warn"
            | "pardon"
            | "op"
            | "deop"
            | "spawnpoint"
            | "enchant"
    ) && argument_index == 1;
    named_target
        || (matches!(root, "tp" | "teleport") && (1..=2).contains(&argument_index))
        || (root == "playsound" && argument_index == 3)
        || (root == "particle" && argument_index == 11)
        || (root == "gamemode" && argument_index == 2)
        || (matches!(root, "xp" | "experience") && argument_index == 2)
        || (root == "advancement" && argument_index == 2)
        || (root == "team" && second == Some("leave") && argument_index == 2)
        || (root == "team" && second == Some("join") && argument_index == 3)
        || (root == "scoreboard" && second == Some("players") && argument_index == 3)
        || (root == "effect" && matches!(second, Some("give" | "clear")) && argument_index == 2)
}

fn push_selector_arguments(
    root: &str,
    player_names: &[String],
    prefix: &str,
    out: &mut Vec<CommandSuggestionEntry>,
) {
    push_each(out, prefix, ["@a", "@p", "@r", "@s"]);
    if matches!(
        root,
        "kill" | "tp" | "teleport" | "enchant" | "team" | "scoreboard" | "effect"
    ) {
        push_each(out, prefix, ["@e", "@n"]);
    }
    push_each(out, prefix, player_names.iter().map(String::as_str));
}

/// Cheap completion for roots, player selectors/names and registry arguments.
/// Brigadier string ranges are UTF-16 code-unit offsets, including any slash.
#[allow(clippy::too_many_arguments)]
pub(crate) fn suggestions(
    request_id: i32,
    command: &str,
    op_level: u8,
    player_names: &[String],
    item_ids: &[String],
    block_ids: &[String],
    entity_ids: &[String],
    structure_ids: &[String],
    plugins: &[(String, u8)],
) -> ClientboundCommandSuggestions {
    let start = command
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(usize::from(command.starts_with('/')), |(index, c)| {
            index + c.len_utf8()
        });
    let prefix = &command[start..];
    let preceding = command[..start].trim_start_matches('/');
    let words: Vec<&str> = preceding.split_whitespace().collect();
    let root = words.first().copied();
    let second = words.get(1).copied();
    let argument_index = words.len();
    let mut result = ClientboundCommandSuggestions {
        id: request_id,
        start: command[..start].encode_utf16().count() as i32,
        length: prefix.encode_utf16().count() as i32,
        suggestions: Vec::new(),
    };
    let Some(root) = root else {
        for &(name, level) in ROOTS {
            if op_level >= level && !plugins.iter().any(|(root, _)| root == name) {
                push_suggestion(&mut result.suggestions, prefix, name);
            }
        }
        for (name, level) in plugins {
            if op_level >= *level {
                push_suggestion(&mut result.suggestions, prefix, name);
            }
        }
        return result;
    };
    if plugins.iter().any(|(name, _)| name == root) {
        return result;
    }
    if !ROOTS
        .iter()
        .any(|&(name, level)| name == root && op_level >= level)
    {
        return result;
    }
    let out = &mut result.suggestions;
    if push_loot_replace_slots(&words, prefix, out)
        || push_settlement_actions(root, second, argument_index, prefix, out)
        || push_effect_arguments(root, second, argument_index, prefix, out)
        || push_tick_arguments(root, second, argument_index, prefix, out)
        || push_bossbar_arguments(root, second, argument_index, &words, prefix, out)
        || push_locate_arguments(root, second, argument_index, structure_ids, prefix, out)
        || push_title_arguments(root, second, argument_index, player_names, prefix, out)
        || push_tellraw_arguments(root, argument_index, player_names, prefix, out)
        || push_damage_arguments(root, argument_index, &words, player_names, prefix, out)
        || push_sound_or_particle_names(root, argument_index, prefix, out)
        || push_tag_actions(root, argument_index, prefix, out)
    {
        return result;
    }
    if push_registry_ids(
        root,
        second,
        argument_index,
        item_ids,
        entity_ids,
        block_ids,
        prefix,
        out,
    ) {
        return result;
    }
    if selector_position(root, second, argument_index) {
        push_selector_arguments(root, player_names, prefix, out);
    } else if root == "time" && second == Some("set") && argument_index == 2 {
        push_each(
            out,
            prefix,
            [
                "minecraft:day",
                "minecraft:noon",
                "minecraft:night",
                "minecraft:midnight",
            ],
        );
    }
    result
}
