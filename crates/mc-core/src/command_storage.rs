//! World-global command storage, retained as vanilla NBT compounds.
use crate::command_context::Context;
use crate::tick::TickState;
use mc_nbt::Tag;
use std::collections::BTreeMap;

pub(crate) type Storage = BTreeMap<String, Tag>;

pub(crate) fn to_tag(storage: &Storage) -> Tag {
    Tag::Compound(
        storage
            .iter()
            .map(|(name, tag)| (name.clone(), tag.clone()))
            .collect(),
    )
}
pub(crate) fn from_tag(tag: Tag) -> Result<Storage, String> {
    let Tag::Compound(fields) = tag else {
        return Err("Command storage must be a compound".into());
    };
    let mut result = Storage::new();
    for (name, value) in fields {
        if !matches!(value, Tag::Compound(_))
            || mc_data::Identifier::parse(&name).is_err()
            || result.insert(name, value).is_some()
        {
            return Err("Invalid command storage entry".into());
        }
    }
    Ok(result)
}
pub(crate) fn encode(storage: &Storage) -> Vec<u8> {
    let mut bytes = Vec::new();
    mc_nbt::write_named(&mut bytes, "", &to_tag(storage)).expect("valid storage NBT");
    bytes
}
pub(crate) fn decode(bytes: &[u8]) -> Result<Storage, String> {
    let mut input = bytes;
    let (_, tag) = mc_nbt::read_named(&mut input).map_err(|e| e.to_string())?;
    if !input.is_empty() {
        return Err("Trailing command storage bytes".into());
    }
    from_tag(tag)
}
fn source(state: &TickState, context: Context, input: &mut &str) -> Result<Tag, String> {
    let root = crate::command_data::Accessor::parse(state, context, input)?.read(state)?;
    if input.trim().is_empty() {
        Ok(root)
    } else {
        let path = crate::command_nbt_path::argument(input)?;
        Ok(crate::command_nbt_path::get(&root, path)?.into_owned())
    }
}

/// Prepare one NBT edit without touching the accessor's authoritative owner.
/// Native entity/block accessors hydrate and validate this candidate before commit.
pub(crate) fn prepare(
    state: &TickState,
    context: Context,
    previous: &Tag,
    action: &str,
    mut input: &str,
) -> Result<(i32, Tag), String> {
    let word = crate::command_nbt_path::argument;
    let mut root = previous.clone();
    let count = match action {
        "merge" => {
            let patch = crate::command_snbt::parse(input.trim()).map_err(str::to_owned)?;
            let (Tag::Compound(fields), Tag::Compound(patch)) = (&mut root, patch) else {
                return Err("Expected compound".into());
            };
            fn merge(fields: &mut Vec<(String, Tag)>, patch: Vec<(String, Tag)>) {
                for (key, value) in patch {
                    if let Some((_, old)) = fields.iter_mut().find(|(name, _)| *name == key) {
                        if let (Tag::Compound(a), Tag::Compound(b)) = (&mut *old, &value) {
                            merge(a, b.clone());
                        } else {
                            *old = value;
                        }
                    } else {
                        fields.push((key, value));
                    }
                }
            }
            merge(fields, patch);
            i32::from(root != *previous)
        }
        "remove" => {
            let path = word(&mut input)?;
            if !input.trim().is_empty() {
                return Err("Trailing arguments".into());
            }
            crate::command_nbt_path::modify(&mut root, path, "remove", None, 0)?
        }
        "modify" => {
            let path = word(&mut input)?;
            let operation = word(&mut input)?;
            if !matches!(operation, "set" | "merge" | "append" | "prepend" | "insert") {
                return Err("Invalid data operation".into());
            }
            let index = if operation == "insert" {
                word(&mut input)?
                    .parse()
                    .map_err(|_| "Invalid list index")?
            } else {
                0
            };
            let value = match word(&mut input)? {
                "value" => {
                    let (value, consumed) = crate::command_snbt::parse_prefix(input.trim_start())
                        .map_err(str::to_owned)?;
                    input = &input.trim_start()[consumed..];
                    value
                }
                "from" => source(state, context, &mut input)?,
                _ => return Err("Expected value or from".into()),
            };
            if !input.trim().is_empty() {
                return Err("Trailing arguments".into());
            }
            crate::command_nbt_path::modify(&mut root, path, operation, Some(&value), index)?
        }
        _ => return Err("Expected data get, merge, modify or remove".into()),
    };
    if count == 0 || root == *previous {
        return Err("Data did not change".into());
    }
    Ok((count, root))
}
