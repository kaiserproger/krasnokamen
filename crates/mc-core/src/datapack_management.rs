//! `/datapack`: list, enable, disable and create.
//!
//! Pinned 26.x `DataPackCommand`: `list` reloads the pack repository first
//! (DATAPACK-LIST-LIVE-DISCOVERY), entries are chat links with a hover of
//! title, newline and `pack.mcmeta` description (DATAPACK-LIST-HOVER),
//! `enable`/`disable` reject already-set and unknown packs with the vanilla
//! translation keys, and `create` (permission 4) writes an empty directory
//! pack. No tick-thread filesystem I/O: every disk read and write, including
//! the fresh listing scan and the pack creation, runs on the datapack worker
//! and its result is delivered on a later tick ([`listed`], [`created`],
//! [`activated`]).
use super::{Action, Failure, Job, ListScope, Listing, Selection};
use crate::boundary::ConnectionId;
use crate::command_execution::CommandResult;
use crate::command_feedback::{text, translated};
use crate::tick::TickState;
use mc_nbt::{ListTag, Tag, tag_type};

/// `Brigadier StringArgumentType.escapeIfRequired`.
fn escape_if_required(name: &str) -> String {
    let plain = |ch: char| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '+');
    if name.chars().all(plain) {
        return name.to_owned();
    }
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

fn translated_failure(key: &'static str, name: &str) -> Failure {
    Failure::Translated {
        key,
        args: vec![name.to_owned()],
    }
}

fn unknown(name: &str) -> Failure {
    translated_failure("commands.datapack.unknown", name)
}

pub(crate) fn command(
    state: &mut TickState,
    connection: ConnectionId,
    text: &str,
) -> CommandResult {
    use CommandResult::{Failed, Unavailable};
    if crate::command_context::exhausted(state) {
        return Unavailable;
    }
    let words: Vec<&str> = crate::commands::command_words(text)
        .map(|words| words.into_iter().collect())
        .unwrap_or_default();
    match dispatch(state, connection, words.as_slice()) {
        Ok(result) => result,
        Err(failure) => {
            failure.send(state, connection);
            Failed
        }
    }
}

fn permission(state: &TickState) -> u8 {
    state
        .command_context
        .current
        .map_or(0, |context| context.permission)
}

fn dispatch(
    state: &mut TickState,
    connection: ConnectionId,
    words: &[&str],
) -> Result<CommandResult, Failure> {
    let usage = || {
        Failure::Text(
            "Supported: datapack list [available|enabled]; datapack enable <pack> [first|last|before|after <pack>]; datapack disable <pack>; datapack create <id> <description>"
                .into(),
        )
    };
    match words {
        ["datapack", "list"] => list(state, connection, ListScope::All),
        ["datapack", "list", "available"] => list(state, connection, ListScope::Available),
        ["datapack", "list", "enabled"] => list(state, connection, ListScope::Enabled),
        // The selection changes when the worker answers, so a command checked against
        // it meanwhile would report "not enabled" for a pack the first one is enabling.
        ["datapack", "enable" | "disable", ..] if state.datapacks.reload.is_some() => {
            Err("Datapack reload already in progress".into())
        }
        ["datapack", "enable", name, order @ ..] => enable(state, connection, name, order),
        ["datapack", "disable", name] => disable(state, connection, name),
        ["datapack", "create", id, description] => {
            if permission(state) < 4 {
                return Err(Failure::Text(
                    crate::commands::CommandError::Unknown.message(),
                ));
            }
            create(state, connection, id, description)
        }
        _ => Err(usage()),
    }
}

/// Title and description of one pack for the hover: `vanilla` is the built-in
/// pack, every other id is a world datapack named by its folder or archive.
fn title(id: &str) -> Tag {
    if id == "vanilla" {
        translated("dataPack.vanilla.name", Vec::new())
    } else {
        text(id.strip_prefix("file/").unwrap_or(id))
    }
}

/// `Pack.getChatLink(enabled)`: a bracketed, coloured, shift-clickable name
/// whose hover shows the title, a line break and the description.
pub(super) fn chat_link(id: &str, enabled: bool, description: &Tag) -> Tag {
    let name = if id == "vanilla" {
        text(id)
    } else {
        // `PackSource.WORLD.decorate`: "<id> (world)" in gray.
        let Tag::Compound(mut fields) = translated(
            "pack.nameAndSource",
            vec![text(id), translated("pack.source.world", Vec::new())],
        ) else {
            unreachable!("translated builds a compound");
        };
        fields.push(("color".into(), Tag::String("gray".into())));
        Tag::Compound(fields)
    };
    let description = match description {
        Tag::String(value) => text(value),
        other => other.clone(),
    };
    let Tag::Compound(mut fields) = translated("chat.square_brackets", vec![name]) else {
        unreachable!("translated builds a compound");
    };
    fields.push((
        "color".into(),
        Tag::String(if enabled { "green" } else { "red" }.into()),
    ));
    fields.push(("insertion".into(), Tag::String(escape_if_required(id))));
    fields.push((
        "hover_event".into(),
        Tag::Compound(vec![
            ("action".into(), Tag::String("show_text".into())),
            (
                "value".into(),
                Tag::Compound(vec![
                    ("text".into(), Tag::String(String::new())),
                    (
                        "extra".into(),
                        Tag::List(ListTag {
                            element_type: tag_type::COMPOUND,
                            elements: vec![title(id), text("\n"), description],
                        }),
                    ),
                ]),
            ),
        ]),
    ));
    Tag::Compound(fields)
}

/// `ComponentUtils.formatList`: entries joined by a gray comma.
fn format_list(entries: Vec<Tag>) -> Tag {
    if entries.len() == 1 {
        return entries.into_iter().next().expect("one entry");
    }
    let mut extra = Vec::with_capacity(entries.len() * 2);
    for (index, entry) in entries.into_iter().enumerate() {
        if index > 0 {
            extra.push(Tag::Compound(vec![
                ("text".into(), Tag::String(", ".into())),
                ("color".into(), Tag::String("gray".into())),
            ]));
        }
        extra.push(entry);
    }
    Tag::Compound(vec![
        ("text".into(), Tag::String(String::new())),
        (
            "extra".into(),
            Tag::List(ListTag {
                element_type: tag_type::COMPOUND,
                elements: extra,
            }),
        ),
    ])
}

fn present_group(
    state: &mut TickState,
    connection: ConnectionId,
    entries: &[(String, Tag)],
    enabled: bool,
) {
    let (none, success) = if enabled {
        (
            "commands.datapack.list.enabled.none",
            "commands.datapack.list.enabled.success",
        )
    } else {
        (
            "commands.datapack.list.available.none",
            "commands.datapack.list.available.success",
        )
    };
    if entries.is_empty() {
        crate::command_execution::success_feedback(state, connection, none, Vec::new());
        return;
    }
    let links = entries
        .iter()
        .map(|(id, description)| chat_link(id, enabled, description))
        .collect::<Vec<_>>();
    crate::command_execution::success_feedback(
        state,
        connection,
        success,
        vec![text(entries.len()), format_list(links)],
    );
}

/// Show a listing in vanilla order: enabled packs, then available ones.
fn present(state: &mut TickState, connection: ConnectionId, scope: ListScope, listing: &Listing) {
    if scope != ListScope::Available {
        present_group(state, connection, &listing.enabled, true);
    }
    if scope != ListScope::Enabled {
        present_group(state, connection, &listing.available, false);
    }
}

fn cached_listing(state: &TickState) -> Listing {
    let packs = state.datapacks.packs();
    let describe = |id: &String| {
        (
            id.clone(),
            packs
                .descriptions
                .get(id)
                .cloned()
                .unwrap_or_else(|| Tag::String(String::new())),
        )
    };
    Listing {
        enabled: packs.selection.enabled.iter().map(describe).collect(),
        available: packs.selection.disabled.iter().map(describe).collect(),
    }
}

fn counted(listing: &Listing, scope: ListScope) -> i32 {
    let count = match scope {
        ListScope::Enabled => listing.enabled.len(),
        ListScope::Available => listing.available.len(),
        ListScope::All => listing.enabled.len() + listing.available.len(),
    };
    count as i32
}

/// Vanilla reloads the pack repository before listing, so a pack added to or
/// removed from the folder is listed at once. The scan runs on the worker and
/// the listing is shown when it finishes; the numeric result is the cached
/// snapshot's count because the command's result is fixed when it returns.
/// While another worker job runs the cached snapshot is listed instead.
fn list(
    state: &mut TickState,
    connection: ConnectionId,
    scope: ListScope,
) -> Result<CommandResult, Failure> {
    let cached = cached_listing(state);
    let result = counted(&cached, scope);
    if state
        .datapacks
        .request_job(Job::List(scope), connection)
        .is_err()
    {
        present(state, connection, scope, &cached);
    }
    Ok(CommandResult::Success(result))
}

/// A tag's JSON text, for the `pack.mcmeta` description of a created pack.
fn json(tag: &Tag) -> serde_json::Value {
    use serde_json::{Number, Value};
    match tag {
        Tag::String(value) => Value::String(value.clone()),
        Tag::Byte(value) => Value::from(*value),
        Tag::Short(value) => Value::from(*value),
        Tag::Int(value) => Value::from(*value),
        Tag::Long(value) => Value::from(*value),
        Tag::Float(value) => Number::from_f64(f64::from(*value)).map_or(Value::Null, Value::Number),
        Tag::Double(value) => Number::from_f64(*value).map_or(Value::Null, Value::Number),
        Tag::Compound(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), json(value)))
                .collect(),
        ),
        Tag::List(list) => Value::Array(list.elements.iter().map(json).collect()),
        Tag::ByteArray(_) | Tag::IntArray(_) | Tag::LongArray(_) => Value::Null,
    }
}

/// `/datapack create <id> <description>`: validated here, written by the worker.
fn create(
    state: &mut TickState,
    connection: ConnectionId,
    id: &str,
    description: &str,
) -> Result<CommandResult, Failure> {
    let id = pack_name(id).ok_or_else(|| Failure::Text("Invalid pack name".into()))?;
    if !super::valid_pack_folder(&id) {
        return Err(translated_failure(
            "commands.datapack.create.invalid_name",
            &id,
        ));
    }
    if !super::portable_pack_folder(&id) {
        return Err(translated_failure(
            "commands.datapack.create.invalid_full_name",
            &id,
        ));
    }
    let description = crate::command_text::parse_component(description)
        .map_err(|error| Failure::Text(error.message()))?;
    state.datapacks.request_job(
        Job::Create {
            id,
            description: json(&description).to_string(),
        },
        connection,
    )?;
    Ok(CommandResult::Completed)
}

fn move_enabled(selection: &mut Selection, id: &str, rest: &[&str]) -> Result<(), Failure> {
    let position = selection
        .enabled
        .iter()
        .position(|entry| entry == id)
        .expect("pack was just enabled");
    let entry = selection.enabled.remove(position);
    // `vanilla` stays first: ordering before it is unsupported.
    let floor = usize::from(
        selection
            .enabled
            .first()
            .is_some_and(|first| first == "vanilla"),
    );
    let at = match rest {
        [] | ["last"] => selection.enabled.len(),
        ["first"] => floor,
        [placement @ ("before" | "after"), other] => {
            let other = pack_name(other).ok_or_else(|| unknown(other))?;
            let at = selection
                .enabled
                .iter()
                .position(|entry| entry == &other)
                .ok_or_else(|| unknown(&other))?;
            if *placement == "after" {
                at + 1
            } else {
                at.max(floor)
            }
        }
        _ => {
            return Err(Failure::Text(
                "Supported: datapack enable <pack> [first|last|before|after <pack>]".into(),
            ));
        }
    };
    selection
        .enabled
        .insert(at.min(selection.enabled.len()), entry);
    Ok(())
}

fn reference(words: &[&str]) -> Option<String> {
    match words {
        ["available" | "enabled", name] => pack_name(name),
        [name] => pack_name(name),
        _ => None,
    }
}

fn enable(
    state: &mut TickState,
    connection: ConnectionId,
    name: &str,
    order: &[&str],
) -> Result<CommandResult, Failure> {
    let name = reference(&[name]).ok_or_else(|| unknown(name))?;
    let mut selection = state.datapacks.selection().clone();
    if selection.enabled.iter().any(|entry| entry == &name) {
        return Err(translated_failure("commands.datapack.enable.failed", &name));
    }
    selection.disabled.retain(|entry| entry != &name);
    selection.enabled.push(name.clone());
    move_enabled(&mut selection, &name, order)?;
    activate(state, connection, selection, Action::Enable, name)
}

fn disable(
    state: &mut TickState,
    connection: ConnectionId,
    name: &str,
) -> Result<CommandResult, Failure> {
    let name = reference(&[name]).ok_or_else(|| unknown(name))?;
    if name == "vanilla" {
        return Err(translated_failure(
            "commands.datapack.disable.failed.feature",
            &name,
        ));
    }
    let mut selection = state.datapacks.selection().clone();
    if !selection.enabled.iter().any(|entry| entry == &name) {
        // Vanilla `DataPackCommand.getPack`: a pack the repository has but that is not
        // selected is "not enabled", whether it was disabled, skipped for failing to load, or
        // never selected; only a pack nobody has is unknown.
        let known = state.datapacks.packs().has_pack(&name);
        return Err(
            if known || selection.disabled.iter().any(|entry| entry == &name) {
                translated_failure("commands.datapack.disable.failed", &name)
            } else {
                unknown(&name)
            },
        );
    }
    selection.enabled.retain(|entry| entry != &name);
    selection.disabled.push(name.clone());
    activate(state, connection, selection, Action::Disable, name)
}

/// Atomically activate `selection` through the reload worker: validate, queue
/// a worker recompile and return the loaded-pack count. The old packs and
/// selection stay authoritative until the worker reports back; the worker also
/// refuses a pack that is not on disk (`commands.datapack.unknown`).
fn activate(
    state: &mut TickState,
    connection: ConnectionId,
    selection: Selection,
    action: Action,
    id: String,
) -> Result<CommandResult, Failure> {
    selection.validate().map_err(Failure::Text)?;
    let count = selection.enabled.len() as i32;
    state
        .datapacks
        .request_activation(connection, selection, action, id)?;
    Ok(CommandResult::Success(count))
}

/// Show a fresh listing from the worker's scan.
pub(super) fn listed(
    state: &mut TickState,
    connection: ConnectionId,
    scope: ListScope,
    listing: &Listing,
) {
    present(state, connection, scope, listing);
}

/// `commands.datapack.create.success`.
pub(super) fn created(state: &mut TickState, connection: ConnectionId, id: &str) {
    crate::command_execution::success_feedback(
        state,
        connection,
        "commands.datapack.create.success",
        vec![text(id)],
    );
}

/// `commands.datapack.modify.enable|disable` once the new selection is live.
pub(super) fn activated(state: &mut TickState, connection: ConnectionId, action: Action, id: &str) {
    let description = state
        .datapacks
        .packs()
        .descriptions
        .get(id)
        .cloned()
        .unwrap_or_else(|| Tag::String(String::new()));
    let link = chat_link(id, action == Action::Enable, &description);
    let key = match action {
        Action::Enable => "commands.datapack.modify.enable",
        Action::Disable => "commands.datapack.modify.disable",
    };
    crate::command_execution::success_feedback(state, connection, key, vec![link]);
}
