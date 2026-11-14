//! `/datapack` management: enable/disable ordering, listing with hover,
//! atomic activation.
//!
//! Vanilla boundary (Main-approved): `enable`/`disable` swap compiled
//! functions/tags and supported recipe/block-loot resources live via the
//! `/reload` worker. Worldgen/dimensions/other registries/ZIP refuse atomically
//! as unsupported (never a false restart promise). Listing discovers the
//! current filesystem catalog on
//! the worker; creation writes pack metadata there before completing its
//! native command result. No tick-thread filesystem I/O.
use super::{Selection, feedback};
use crate::tick::TickState;

fn quoted(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    out.push('"');
    for ch in name.chars() {
        if matches!(ch, '"' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

fn unquote(word: &str) -> Option<String> {
    let body = word.strip_prefix('"')?.strip_suffix('"')?;
    if body.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            out.push(chars.next()?);
        } else if ch == '"' {
            return None;
        } else {
            out.push(ch);
        }
    }
    Some(out)
}

fn pack_name(word: &str) -> Option<String> {
    if word.starts_with('"') {
        return unquote(word);
    }
    if word.is_empty() || word.len() > 1024 || word.contains([' ', '\t', '\n', '[', ']', '{', '}'])
    {
        return None;
    }
    Some(word.to_owned())
}

/// Parse a pack reference: `available|enabled <name>`, or a bare/quotable name.
fn reference(words: &[&str]) -> Option<String> {
    match words {
        ["available", name] | ["enabled", name] => pack_name(name),
        [name] => pack_name(name),
        _ => None,
    }
}

pub(crate) fn command(
    state: &mut TickState,
    connection: crate::boundary::ConnectionId,
    text: &str,
) -> crate::command_execution::CommandResult {
    use crate::command_execution::CommandResult::Failed;
    let words: Vec<&str> = crate::commands::command_words(text)
        .map(|words| words.into_iter().collect())
        .unwrap_or_default();
    let result = dispatch(state, connection, words.as_slice());
    match result {
        Ok(result) => result,
        Err(message) => {
            feedback(state, connection, message);
            Failed
        }
    }
}

fn dispatch(
    state: &mut TickState,
    connection: crate::boundary::ConnectionId,
    words: &[&str],
) -> Result<crate::command_execution::CommandResult, String> {
    use crate::command_execution::CommandResult::Deferred;
    match words {
        ["datapack", "list"] | ["datapack", "list", "available" | "enabled"] => {
            let filter = words.get(2).map(|word| (*word).to_owned());
            let completion = state.command_context.completion_id()?;
            state.datapacks.request_work(connection, false, Some(completion), move |root| {
                Ok(super::WorkerResult::List { descriptions: discover_descriptions(root)?, filter })
            })?;
            Ok(Deferred(completion))
        }
        ["datapack", "enable", rest @ ..] => enable(state, connection, rest).map(Deferred),
        ["datapack", "disable", rest @ ..] => disable(state, connection, rest).map(Deferred),
        ["datapack", "create", name, description] => create(state, connection, name, description),
        _ => Err(
            "Supported: datapack list [available|enabled]; datapack enable <pack> [first|last|before|after <pack>]; datapack disable <pack>".into(),
        ),
    }
}

/// One listed pack as a hoverable chat component: the quoted name shows the
/// `pack.mcmeta` description on hover (DATAPACK-LIST-HOVER).
fn hover_entry(name: &str, description: &mc_nbt::Tag) -> mc_nbt::Tag {
    let fields = vec![
        ("text".to_owned(), mc_nbt::Tag::String(quoted(name))),
        (
            "hover_event".to_owned(),
            mc_nbt::Tag::Compound(vec![
                ("action".to_owned(), mc_nbt::Tag::String("show_text".to_owned())),
                ("value".to_owned(), description.clone()),
            ]),
        ),
    ];
    mc_nbt::Tag::Compound(fields)
}

pub(super) fn report_list(
    state: &mut TickState,
    connection: crate::boundary::ConnectionId,
    filter: Option<&str>,
) -> i32 {
    let descriptions = state.datapacks.catalog();
    let enabled = &state.datapacks.selection().enabled;
    let mut count = 0;
    let messages = ["enabled", "available"].into_iter()
        .filter(|kind| filter.is_none_or(|filter| filter == *kind))
        .map(|kind| {
            let names: Vec<String> = if kind == "enabled" {
                enabled.iter().filter(|id| descriptions.contains_key(*id)).cloned().collect()
            } else {
                descriptions.keys().filter(|id| !enabled.contains(*id)).cloned().collect()
            };
            count += names.len() as i32;
            list_feedback(kind, &names, descriptions)
        }).collect::<Vec<_>>();
    for (key, args) in messages {
        crate::command_execution::feedback(state, connection, &key, args);
    }
    count
}

fn discover_descriptions(root: &std::path::Path) -> Result<std::collections::BTreeMap<String, mc_nbt::Tag>, String> {
    let mut descriptions = super::PackSet::default().descriptions;
    let mut remaining = super::MAX_BYTES;
    for (id, pack) in super::discover(root, &mut remaining)? {
        if pack.features.iter().all(|feature| super::ENABLED_FEATURES.contains(&feature.as_str())) {
            descriptions.insert(id, pack.description);
        }
    }
    Ok(descriptions)
}

fn create(state: &mut TickState, connection: crate::boundary::ConnectionId, name: &str, description: &str) -> Result<crate::command_execution::CommandResult, String> {
    if state.command_context.current.is_none_or(|context| context.permission < 4) {
        return Err(crate::commands::CommandError::Unknown.message());
    }
    let name = pack_name(name).ok_or("Invalid new pack name")?;
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if name.is_empty() || name.len() > 255 || name.ends_with(['.', ' '])
        || name.chars().any(|ch| ch.is_control() || "<>:\"/\\|?*".contains(ch))
        || matches!(stem.as_str(), "" | "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return Err(format!("Invalid new pack name '{name}'"));
    }
    let description = crate::command_text::parse_component(description).map_err(|error| error.message())?;
    let context = state.command_context.current.expect("owner permission checked");
    let description = crate::command_text::resolve_component(state, context.executor, &description)?;
    let encoded_description = component_json(&description)?;
    let completion = state.command_context.completion_id()?;
    state.datapacks.request_work(connection, false, Some(completion), move |root| {
        let mut descriptions = discover_descriptions(root)?;
        create_pack(root, &name, encoded_description)?;
        descriptions.insert(format!("file/{name}"), description);
        Ok(super::WorkerResult::Created { descriptions, name })
    })?;
    Ok(crate::command_execution::CommandResult::Deferred(completion))
}

fn component_json(tag: &mc_nbt::Tag) -> Result<serde_json::Value, String> {
    use mc_nbt::Tag;
    Ok(match tag {
        Tag::String(value) => value.clone().into(),
        Tag::Compound(fields) => serde_json::Value::Object(fields.iter().map(|(key, value)| {
            let encoded = if matches!(key.as_str(), "bold" | "italic" | "underlined" | "strikethrough" | "obfuscated" | "interpret")
                && let Tag::Byte(value) = value
            {
                serde_json::Value::Bool(*value != 0)
            } else {
                component_json(value)?
            };
            Ok((key.clone(), encoded))
        }).collect::<Result<_, String>>()?),
        Tag::List(list) => serde_json::Value::Array(list.elements.iter().map(component_json).collect::<Result<_, _>>()?),
        Tag::Byte(value) => (*value).into(),
        Tag::Short(value) => (*value).into(),
        Tag::Int(value) => (*value).into(),
        Tag::Long(value) => (*value).into(),
        Tag::Float(value) => serde_json::Number::from_f64(f64::from(*value)).ok_or("Non-finite component number")?.into(),
        Tag::Double(value) => serde_json::Number::from_f64(*value).ok_or("Non-finite component number")?.into(),
        _ => return Err("Unsupported array in text component".into()),
    })
}

fn create_pack(root: &std::path::Path, name: &str, description: serde_json::Value) -> Result<(), String> {
    use std::io::Write;
    // Canonical 26.3 version.json: data_major=121, data_minor=0.
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "pack": { "description": description, "min_format": [121, 0], "max_format": [121, 0] }
    })).map_err(|error| format!("Encoding datapack '{name}': {error}"))?;
    std::fs::create_dir_all(root).map_err(|error| format!("Creating datapack directory: {error}"))?;
    if std::fs::symlink_metadata(root).map_err(|error| error.to_string())?.file_type().is_symlink() {
        return Err("Datapack root cannot be a symlink".into());
    }
    let directory = root.join(name);
    std::fs::create_dir(&directory).map_err(|error| format!("Creating datapack '{name}': {error}"))?;
    let metadata = directory.join("pack.mcmeta");
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&metadata)?;
        file.write_all(&bytes)?;
        file.sync_all()
    })();
    if let Err(error) = result {
        let cleanup = std::fs::remove_file(&metadata).and_then(|()| std::fs::remove_dir(&directory));
        return Err(format!("Creating datapack '{name}': {error}; cleanup: {cleanup:?}"));
    }
    Ok(())
}

fn list_feedback(
    kind: &str,
    names: &[String],
    descriptions: &std::collections::BTreeMap<String, mc_nbt::Tag>,
) -> (String, Vec<mc_nbt::Tag>) {
    use mc_nbt::{ListTag, Tag, tag_type};
    if names.is_empty() {
        return (format!("commands.datapack.list.{kind}.none"), Vec::new());
    }
    let mut entries = Vec::with_capacity(names.len() * 2 - 1);
    for name in names {
        if !entries.is_empty() {
            entries.push(Tag::Compound(vec![("text".into(), Tag::String(", ".into()))]));
        }
        entries.push(hover_entry(
            name,
            descriptions.get(name).unwrap_or(&Tag::String(String::new())),
        ));
    }
    (
        format!("commands.datapack.list.{kind}.success"),
        vec![
            Tag::String(names.len().to_string()),
            Tag::List(ListTag { element_type: tag_type::COMPOUND, elements: entries }),
        ],
    )
}

fn move_enabled(selection: &mut Selection, id: &str, rest: &[&str]) -> Result<(), String> {
    let position = selection
        .enabled
        .iter()
        .position(|entry| entry == id)
        .ok_or_else(|| format!("Datapack {id} is not enabled"))?;
    match rest {
        [] | ["last"] => {
            let entry = selection.enabled.remove(position);
            selection.enabled.push(entry);
        }
        ["first"] => {
            let entry = selection.enabled.remove(position);
            selection.enabled.insert(0, entry);
        }
        ["before", other] | ["after", other] => {
            let other = pack_name(other).ok_or_else(|| format!("Unknown datapack {other}"))?;
            let entry = selection.enabled.remove(position);
            let at = selection
                .enabled
                .iter()
                .position(|entry| entry == &other)
                .ok_or_else(|| format!("Unknown datapack {other}"))?;
            let at = if rest[0] == "after" { at + 1 } else { at };
            selection
                .enabled
                .insert(at.min(selection.enabled.len()), entry);
        }
        _ => {
            return Err(
                "Supported: datapack enable <pack> [first|last|before|after <pack>]".into(),
            );
        }
    }
    Ok(())
}

fn enable(
    state: &mut TickState,
    connection: crate::boundary::ConnectionId,
    rest: &[&str],
) -> Result<crate::command_context::CompletionId, String> {
    let (name, order) = match rest {
        [name, order @ ..] => (
            reference(&[*name]).ok_or_else(|| format!("Unknown datapack {name}"))?,
            order,
        ),
        [] => {
            return Err(
                "Supported: datapack enable <pack> [first|last|before|after <pack>]".into(),
            );
        }
    };
    if name == "vanilla" {
        return Err("Datapack vanilla is already enabled".into());
    }
    let mut selection = state.datapacks.selection().clone();
    if selection.enabled.iter().any(|entry| entry == &name) {
        return Err(format!("Datapack {name} is already enabled"));
    } else {
        if !state.datapacks.catalog().contains_key(&name) {
            return Err(format!("Unknown datapack {name}"));
        }
        selection.disabled.retain(|entry| entry != &name);
        selection.enabled.push(name.clone());
        if !order.is_empty() {
            move_enabled(&mut selection, &name, order)?;
        }
    }
    activate(state, connection, selection)
}

fn disable(
    state: &mut TickState,
    connection: crate::boundary::ConnectionId,
    rest: &[&str],
) -> Result<crate::command_context::CompletionId, String> {
    let name = match rest {
        [name] => reference(&[name]).ok_or_else(|| format!("Unknown datapack {name}"))?,
        _ => return Err("Supported: datapack disable <pack>".into()),
    };
    if name == "vanilla" {
        return Err("Disabling builtin vanilla content is unsupported".into());
    }
    let mut selection = state.datapacks.selection().clone();
    if !selection.enabled.iter().any(|entry| entry == &name) {
        if selection.disabled.iter().any(|entry| entry == &name) {
            return Err(format!("Datapack {name} is already disabled"));
        }
        return Err(format!("Unknown datapack {name}"));
    }
    selection.enabled.retain(|entry| entry != &name);
    selection.disabled.push(name);
    activate(state, connection, selection)
}

/// Atomically activate `selection` through the reload worker: validate, queue
/// a worker recompile, and complete with the loaded-pack count. The worker classifies
/// the outcome on completion: functions/tags and supported recipe/block-loot
/// resources swap live; worldgen/dimensions/other registries remain unsupported.
/// Old packs and selection stay authoritative until successful publication.
fn activate(
    state: &mut TickState,
    connection: crate::boundary::ConnectionId,
    selection: Selection,
) -> Result<crate::command_context::CompletionId, String> {
    selection.validate()?;
    let completion = state.command_context.completion_id()?;
    state.datapacks.request_activation(selection, connection, completion)?;
    Ok(completion)
}

#[cfg(test)]
#[path = "datapack_management_tests.rs"]
mod tests;
