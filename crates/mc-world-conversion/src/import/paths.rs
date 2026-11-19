//! Source path classification, NBT/region/POI validation and the tree walk.

use super::*;

fn exact_nbt(bytes: &[u8], path: &Path) -> Result<Tag, CoreError> {
    let mut input = bytes;
    let (_, tag) = mc_nbt::read_named(&mut input)
        .map_err(|error| fail(format!("{}: {error}", path.display())))?;
    if !input.is_empty() {
        return Err(fail(format!("{}: trailing NBT bytes", path.display())));
    }
    Ok(tag)
}

fn region_position(path: &Path) -> Result<(i32, i32), CoreError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| fail("invalid region filename"))?;
    let parts: Vec<_> = name.split('.').collect();
    if parts.len() != 4 || parts[0] != "r" || parts[3] != "mca" {
        return Err(fail(format!(
            "{}: unsupported region filename",
            path.display()
        )));
    }
    let x: i32 = parts[1].parse().map_err(fail)?;
    let z: i32 = parts[2].parse().map_err(fail)?;
    if name != format!("r.{x}.{z}.mca") {
        return Err(fail("noncanonical region filename"));
    }
    Ok((x, z))
}

pub(super) fn validate_poi_path(path: &Path) -> Result<(), CoreError> {
    if !matches!(
        path.parent().and_then(Path::to_str),
        Some(
            "dimensions/minecraft/overworld/poi"
                | "dimensions/minecraft/the_nether/poi"
                | "dimensions/minecraft/the_end/poi"
        )
    ) {
        return Err(fail(format!("{}: unsupported POI path", path.display())));
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| fail("invalid POI filename"))?;
    let mut parts = name.split('.');
    let prefix = parts.next().unwrap_or_default();
    let x: i32 = parts
        .next()
        .ok_or_else(|| fail("missing POI X"))?
        .parse()
        .map_err(fail)?;
    let z: i32 = parts
        .next()
        .ok_or_else(|| fail("missing POI Z"))?
        .parse()
        .map_err(fail)?;
    let extension = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || !matches!((prefix, extension), ("r", "mca") | ("c", "mcc"))
        || name != format!("{prefix}.{x}.{z}.{extension}")
    {
        return Err(fail(format!(
            "{}: unsupported POI filename",
            path.display()
        )));
    }
    Ok(())
}

pub(super) fn visit(
    path: &Path,
    mut visitor: impl FnMut(ChunkPos, Tag, &[u8]) -> Result<(), CoreError>,
) -> Result<(), CoreError> {
    let (rx, rz) = region_position(path)?;
    let mut result = Ok(());
    mc_world::anvil::visit_region(path, |payload| {
        if result.is_err() {
            return;
        }
        result = (|| {
            let pos = ChunkPos {
                x: rx
                    .checked_mul(32)
                    .and_then(|x| x.checked_add(i32::from(payload.local_x)))
                    .ok_or_else(|| fail("region X overflow"))?,
                z: rz
                    .checked_mul(32)
                    .and_then(|z| z.checked_add(i32::from(payload.local_z)))
                    .ok_or_else(|| fail("region Z overflow"))?,
            };
            let tag = exact_nbt(&payload.uncompressed_nbt, path)?;
            visitor(pos, tag, &payload.uncompressed_nbt)
        })();
    })
    .map_err(fail)?;
    result
}

fn legacy_root(dimension: Dimension) -> &'static Path {
    Path::new(match dimension {
        Dimension::Overworld => "",
        Dimension::Nether => "DIM-1",
        Dimension::End => "DIM1",
    })
}

/// Layout admission is read-only: mappings never rename or write source files.
pub(super) fn dimension_root(
    paths: &[PathBuf],
    dimension: Dimension,
) -> Result<PathBuf, CoreError> {
    let modern = dimension.directory(Path::new(""));
    let legacy = legacy_root(dimension);
    let active = |root: &Path, old: bool| {
        paths.iter().any(|path| {
            ["region", "entities", "poi", "data"]
                .iter()
                .filter(|domain| !(old && dimension == Dimension::Overworld && **domain == "data"))
                .any(|domain| path.starts_with(root.join(domain)))
        })
    };
    match (active(&modern, false), active(legacy, true)) {
        (true, true) => Err(fail(format!(
            "{}: ambiguous old/new dimension locations {} and {}",
            dimension.name(),
            legacy.display(),
            modern.display()
        ))),
        (_, true) => Ok(legacy.into()),
        _ => Ok(modern),
    }
}

pub(super) fn player_path(path: &Path) -> bool {
    matches!(path.parent(), Some(parent) if parent == Path::new("players/data") || parent == Path::new("playerdata"))
        && path.extension().and_then(|value| value.to_str()) == Some("dat")
}

pub(super) fn typed_saved_data(path: &Path) -> bool {
    path == Path::new("data/minecraft/scoreboard.dat")
        || (matches!(
            path.parent().and_then(Path::to_str),
            Some(
                "dimensions/minecraft/overworld/data/minecraft"
                    | "dimensions/minecraft/the_nether/data/minecraft"
                    | "dimensions/minecraft/the_end/data/minecraft"
            )
        ) && matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("raids.dat" | "ender_dragon_fight.dat" | "world_border.dat" | "chunk_tickets.dat")
        ))
}

/// Retired vanilla stores have no native owner or executed DFU path. Preserve
/// their entire file bytes without allocating a historical feature tree merely
/// to invent an effective source version. Unknown saved-data names stay refused.
pub(crate) fn retired_saved_data(path: &Path) -> bool {
    matches!(
        path.to_str(),
        Some(
            "data/Mineshaft.dat"
                | "data/Monument.dat"
                | "data/Stronghold.dat"
                | "data/Temple.dat"
                | "data/Village.dat"
                | "data/villages.dat"
                | "data/villages_end.dat"
                | "data/villages_nether.dat"
        )
    )
}

pub(super) fn opaque_saved_data(path: &Path) -> bool {
    let Some(name) = path.to_str() else {
        return false;
    };
    let maps = name.strip_prefix("data/minecraft/maps/");
    let map = maps.and_then(|name| name.strip_suffix(".dat"));
    let numbered_map =
        map.is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()));
    let command_storage = name
        .strip_prefix("data/")
        .and_then(|name| name.strip_suffix("/command_storage.dat"))
        .is_some_and(|namespace| {
            !namespace.is_empty()
                && namespace.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'.' | b'-')
                })
        });
    numbered_map
        || command_storage
        || matches!(
            name,
            "data/minecraft/maps/last_id.dat"
                | "data/minecraft/custom_boss_events.dat"
                | "data/minecraft/random_sequences.dat"
                | "data/minecraft/scheduled_events.dat"
                | "data/minecraft/stopwatches.dat"
        )
}
