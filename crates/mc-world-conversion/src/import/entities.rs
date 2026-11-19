//! Entity NBT admission: body bytes, UUID admission and duplicate drops.

use super::*;

use super::paths::visit;

// Named NBT requires a compound root. Move the body into that wrapper and back
// rather than cloning entity payloads solely for the cross-storage comparison.
pub(super) fn entity_body_bytes(list: &mut Tag) -> Result<Vec<u8>, CoreError> {
    let wrapper = Tag::Compound(vec![(
        "Entities".into(),
        std::mem::replace(list, Tag::Compound(Vec::new())),
    )]);
    let mut bytes = Vec::new();
    let result = mc_nbt::write_named(&mut bytes, "", &wrapper);
    let Tag::Compound(mut fields) = wrapper else {
        unreachable!("comparison wrapper is a compound")
    };
    *list = fields.pop().expect("single comparison body").1;
    result.map_err(fail)?;
    Ok(bytes)
}

// Decode only identity here: discarded later bodies may contain unsupported
// payloads. The normal typed entity decoder validates every retained body.
pub(super) fn admit_entity_uuids(
    list: &mut Tag,
    source: &Path,
    chunk: [i32; 2],
    uuids: &mut BTreeSet<uuid::Uuid>,
    drops: &mut Vec<super::super::DuplicateUuidDrop>,
) -> Result<(), CoreError> {
    let Tag::List(list) = list else {
        return Err(fail(format!("{}: invalid Entities list", source.display())));
    };
    let mut retained = 0;
    for entity_index in 0..list.elements.len() {
        let invalid = |reason| {
            if let Tag::Compound(fields) = &list.elements[entity_index] {
                return fail(mc_world::anvil::entity_regions::record_error(
                    source,
                    chunk,
                    entity_index,
                    fields,
                    reason,
                ));
            }
            fail(format!(
                "{} chunk ({}, {}) entity-index {entity_index}: {reason}",
                source.display(),
                chunk[0],
                chunk[1]
            ))
        };
        let uuid = match field(&list.elements[entity_index], "UUID")
            .map_err(|error| invalid(error.to_string()))?
        {
            Some(Tag::IntArray(words)) if words.len() == 4 => {
                let bits = words
                    .iter()
                    .fold(0_u128, |bits, word| (bits << 32) | u128::from(*word as u32));
                uuid::Uuid::from_u128(bits)
            }
            _ => return Err(invalid("UUID: expected four-int UUID".into())),
        };
        if uuid.is_nil() {
            return Err(invalid("nil entity UUID".into()));
        }
        if uuids.insert(uuid) {
            // Swapping into earlier dropped slots leaves later original indices
            // untouched and retains admitted bodies in their source order.
            if retained != entity_index {
                list.elements.swap(retained, entity_index);
            }
            retained += 1;
        } else {
            drops.push(super::super::DuplicateUuidDrop {
                source: source.to_owned(),
                chunk,
                entity_index,
                uuid: uuid.to_string(),
                staged_nbt: PathBuf::new(),
            });
        }
    }
    list.elements.truncate(retained);
    Ok(())
}

pub(super) fn stage_duplicate_uuid_drops(
    source: &Path,
    stage: &Path,
    drops: &mut [super::super::DuplicateUuidDrop],
) -> Result<(), CoreError> {
    for (index, drop) in drops.iter_mut().enumerate() {
        drop.staged_nbt =
            PathBuf::from("duplicate-uuid-dropped").join(format!("{index}-{}.nbt", drop.uuid));
    }
    let mut sources: BTreeMap<_, BTreeMap<_, Vec<_>>> = BTreeMap::new();
    for drop in drops.iter() {
        sources
            .entry(drop.source.as_path())
            .or_default()
            .entry(drop.chunk)
            .or_default()
            .push(drop);
    }
    // Revisit only files with losses; never retain every original entity NBT
    // during the upgrading/typed-admission pass.
    for (relative, mut chunks) in sources {
        visit(&source.join(relative), |pos, tag, _| {
            let Some(records) = chunks.remove(&[pos.x, pos.z]) else {
                return Ok(());
            };
            let legacy = field(&tag, "Level")?
                .map(|level| field(level, "Entities"))
                .transpose()?
                .flatten();
            let lists = [legacy, field(&tag, "entities")?, field(&tag, "Entities")?];
            let mut lists = lists.into_iter().flatten();
            let Some(Tag::List(list)) = lists.next() else {
                return Err(fail(format!(
                    "{} chunk ({}, {}): missing original dropped-entity list",
                    relative.display(),
                    pos.x,
                    pos.z
                )));
            };
            if lists.next().is_some() {
                return Err(fail(format!(
                    "{} chunk ({}, {}): ambiguous original dropped-entity list",
                    relative.display(),
                    pos.x,
                    pos.z
                )));
            }
            for drop in records {
                let original = list.elements.get(drop.entity_index).ok_or_else(|| {
                    fail(format!(
                        "{} chunk ({}, {}) entity-index {} UUID {}: original dropped entity missing",
                        relative.display(), pos.x, pos.z, drop.entity_index, drop.uuid
                    ))
                })?;
                super::super::write_nbt(&stage.join(&drop.staged_nbt), original)?;
            }
            Ok(())
        })?;
        if !chunks.is_empty() {
            return Err(fail(format!(
                "{}: original dropped-entity chunks missing: {:?}",
                relative.display(),
                chunks.keys().collect::<Vec<_>>()
            )));
        }
    }
    Ok(())
}
