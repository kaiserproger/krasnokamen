//! Datapacks: directories and `.zip` archives. Compilation and every other
//! filesystem access is startup/worker work, never tick filesystem I/O.
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mc_nbt::{ListTag, Tag, tag_type};

use crate::boundary::{ChangeSet, ConnectionId, PlayerChange};
use crate::command_feedback::{text, translated};
use crate::tick::TickState;
use mc_domain::EntityId;

#[path = "datapacks_worldgen.rs"]
pub(crate) mod worldgen;

const COMMAND_BUDGET: usize = crate::command_context::LIMIT;
const MAX_RESOURCES: usize = 16384;
/// Whether `relative` (below `data/`) is `<namespace>/<directory>/...` with a
/// directory vanilla does not load.
fn is_ignored_directory(relative: &str) -> bool {
    let mut parts = relative.splitn(3, '/');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(_), Some(kind), Some(_)) if !VANILLA_DATA_DIRECTORIES.contains(&kind)
    )
}
/// Bytes one load may read and expand across every selected pack: raw files plus
/// the decompressed size of gzip structure NBT. The full converted MVS structure
/// pack is 4,110,276 B on disk and 48,122,266 B decompressed (measured 2026-11-17),
/// so 128 MiB leaves about 2.7x headroom for a larger pack while still bounding the
/// memory a hostile pack can make the server allocate.
const MAX_BYTES: usize = 128 * 1024 * 1024;
/// Largest datapack `.zip` (its central directory is read whole into memory).
const MAX_ARCHIVE_BYTES: u64 = 16 * 1024 * 1024;
/// Directories below `data/<namespace>/` that vanilla 26.3 loads: the first path
/// segment of every data-driven registry in `RegistryDataLoader` (`WORLD_REGISTRIES`,
/// `DIMENSION_REGISTRIES`, `RELOADABLE_REGISTRIES`; each registry's directory is its
/// key path, read from the 26.3 merged client+server jar), plus the three non-registry
/// loaders `function` (`ServerFunctionLibrary`), `tags` and `structure` (templates,
/// `StructureTemplateManager`). Vanilla ignores every other directory (mod
/// configuration such as `moogs_structures/`); this server ignores them too, but
/// refuses the vanilla ones it does not implement, so no semantics are dropped silently.
const VANILLA_DATA_DIRECTORIES: &[&str] = &[
    "advancement",
    "banner_pattern",
    "block_transformer",
    "cat_sound_variant",
    "cat_variant",
    "chat_type",
    "chicken_sound_variant",
    "chicken_variant",
    "context_float_provider",
    "context_int_provider",
    "cow_sound_variant",
    "cow_variant",
    "damage_type",
    "decorated_pot_pattern",
    "dialog",
    "dimension",
    "dimension_type",
    "enchantment",
    "enchantment_provider",
    "frog_variant",
    "function",
    "instrument",
    "item_modifier",
    "jukebox_song",
    "loot_table",
    "painting_variant",
    "pig_sound_variant",
    "pig_variant",
    "predicate",
    "recipe",
    "slot_source",
    "structure",
    "sulfur_cube_archetype",
    "tags",
    "test_environment",
    "test_instance",
    "timeline",
    "trade_set",
    "trial_spawner",
    "trim_material",
    "trim_pattern",
    "villager_trade",
    "wolf_sound_variant",
    "wolf_variant",
    "world_clock",
    "worldgen",
    "zombie_nautilus_variant",
];
/// Feature flags enabled for this world and advertised during configuration.
pub const ENABLED_FEATURES: &[&str] = &["minecraft:vanilla"];

/// Ordered native authority for the enabled and explicitly disabled pack ids.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Selection {
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            enabled: vec!["vanilla".into()],
            disabled: Vec::new(),
        }
    }
}

/// The builtin pack or one direct entry of the datapack directory.
fn valid_pack_id(id: &str) -> bool {
    id.len() <= 1024
        && (id == "vanilla"
            || id.strip_prefix("file/").is_some_and(|name| {
                !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
            }))
}

impl Selection {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.enabled.len().saturating_add(self.disabled.len()) > MAX_RESOURCES {
            return Err("Datapack selection limit exceeded".into());
        }
        let mut seen = BTreeSet::new();
        for id in self.enabled.iter().chain(&self.disabled) {
            if !valid_pack_id(id) || !seen.insert(id) {
                return Err(format!("Invalid or duplicate datapack selection id: {id}"));
            }
        }
        if !self.enabled.iter().any(|id| id == "vanilla") {
            return Err("Disabling builtin vanilla content is unsupported".into());
        }
        Ok(())
    }
}
#[derive(Clone)]
pub(crate) struct PackSet {
    pub selection: Selection,
    /// Pack descriptions from `pack.mcmeta` (`file/<name>` and `vanilla`) as
    /// text components, preserved so listing can show them on hover.
    pub descriptions: BTreeMap<String, Tag>,
    pub(crate) functions: BTreeMap<String, Arc<[String]>>,
    pub(crate) tags: BTreeMap<String, Arc<[String]>>,
    /// Loot predicates by `namespace:path`, checked at load to hold only
    /// conditions the selector `predicate=` option can evaluate.
    pub(crate) predicates: BTreeMap<String, serde_json::Value>,
    resources: BTreeMap<String, String>,
    /// Ordered resource layers, including the builtin vanilla position.
    worldgen_layers: Vec<worldgen::Layer>,
    worldgen_base: Option<PathBuf>,
    worldgen_identity: Option<String>,
    worldgen_structures: Arc<[mc_data::Identifier]>,
    structure_catalog: Option<Arc<worldgen::StructureCatalog>>,
    dropped: Vec<String>,
    /// The pack that provided each recipe or loot resource, to name it in errors.
    origins: BTreeMap<String, String>,
    /// Pack entries of the folder that could not be read (id and why), not selected.
    failed: BTreeMap<String, String>,
    /// Entries of the folder that are no pack (a README, a stray file).
    not_packs: Vec<String>,
    /// Files of selected packs below a directory vanilla does not load (`pack: path`).
    ignored: Vec<String>,
    /// New packs this load left out because they cannot load, with the reason.
    skipped: Vec<String>,
    /// The `pack.mcmeta` filters of every enabled pack: they hide the files of
    /// the packs below, the builtin `vanilla` pack included.
    filters: Vec<Filter>,
}

/// One entry of a `pack.mcmeta` `filter.block` list: the files whose namespace
/// and path (below `data/<namespace>/`) match are hidden from lower packs.
#[derive(Clone)]
struct Filter {
    namespace: Option<regex::Regex>,
    path: Option<regex::Regex>,
}
impl PartialEq for Filter {
    fn eq(&self, other: &Self) -> bool {
        let pattern =
            |filter: &Option<regex::Regex>| filter.as_ref().map(|r| r.as_str().to_owned());
        pattern(&self.namespace) == pattern(&other.namespace)
            && pattern(&self.path) == pattern(&other.path)
    }
}
impl Filter {
    fn parse(metadata: &serde_json::Value) -> Result<Vec<Self>, String> {
        let Some(filter) = metadata.get("filter") else {
            return Ok(Vec::new());
        };
        let compile =
            |entry: &serde_json::Value, key: &str| -> Result<Option<regex::Regex>, String> {
                match entry.get(key) {
                    None => Ok(None),
                    Some(serde_json::Value::String(pattern)) => regex::Regex::new(pattern)
                        .map(Some)
                        .map_err(|e| format!("filter {key} is not a valid pattern: {e}")),
                    Some(_) => Err(format!("filter {key} must be a string")),
                }
            };
        filter
            .get("block")
            .and_then(serde_json::Value::as_array)
            .ok_or("filter requires a block list")?
            .iter()
            .map(|entry| {
                Ok(Self {
                    namespace: compile(entry, "namespace")?,
                    path: compile(entry, "path")?,
                })
            })
            .collect()
    }

    fn hides(&self, namespace: &str, path: &str) -> bool {
        self.namespace
            .as_ref()
            .is_none_or(|r| r.is_match(namespace))
            && self.path.as_ref().is_none_or(|r| r.is_match(path))
    }
}
impl Default for PackSet {
    fn default() -> Self {
        Self {
            selection: Selection::default(),
            descriptions: BTreeMap::from([("vanilla".into(), vanilla_description())]),
            functions: BTreeMap::new(),
            tags: BTreeMap::new(),
            predicates: BTreeMap::new(),
            resources: BTreeMap::new(),
            worldgen_layers: Vec::new(),
            worldgen_base: None,
            worldgen_identity: None,
            worldgen_structures: Arc::from([]),
            structure_catalog: None,
            dropped: Vec::new(),
            origins: BTreeMap::new(),
            failed: BTreeMap::new(),
            not_packs: Vec::new(),
            ignored: Vec::new(),
            skipped: Vec::new(),
            filters: Vec::new(),
        }
    }
}

fn vanilla_description() -> Tag {
    translated("dataPack.vanilla.description", Vec::new())
}

/// `pack_format` of the data pack version 26.3 (`version.json`
/// `pack_version.data_major`), written into the packs `/datapack create` makes.
const DATA_PACK_FORMAT: u32 = 121;

/// `FileUtil.isValidPathSegment`: not `.`/`..` and only `[-._a-z0-9]`.
pub(crate) fn valid_pack_folder(name: &str) -> bool {
    name != "."
        && name != ".."
        && !name.is_empty()
        && name
            .chars()
            .all(|ch| matches!(ch, '-' | '.' | '_' | 'a'..='z' | '0'..='9'))
}

/// `FileUtil.isPathPartPortable`: not a trailing-dot name or a reserved
/// Windows device name (`CON`, `COM1`, `LPT1`, ...) with or without extension.
pub(crate) fn portable_pack_folder(name: &str) -> bool {
    if name.ends_with('.') {
        return false;
    }
    let device = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    !(matches!(
        device.as_str(),
        "COM" | "CLOCK$" | "CON" | "PRN" | "AUX" | "NUL"
    ) || (device.len() == 4
        && (device.starts_with("COM") || device.starts_with("LPT"))
        && device.as_bytes()[3].is_ascii_digit()
        && device.as_bytes()[3] != b'0'))
}

/// Why a worker job or command was refused: a vanilla translation key with
/// its text arguments, or a free-text message.
#[derive(Debug)]
pub(crate) enum Failure {
    Translated {
        key: &'static str,
        args: Vec<String>,
    },
    Text(String),
    /// A refused reload or activation with the pack ids its folder holds: the old
    /// packs stay installed, but vanilla `getPack` answers from that repository.
    Refused(Box<Failure>, BTreeSet<String>),
}
impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::Text(message)
    }
}
impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self::Text(message.to_owned())
    }
}
impl Failure {
    pub(crate) fn send(self, state: &mut TickState, connection: ConnectionId) {
        match self {
            Self::Translated { key, args } => crate::command_execution::feedback(
                state,
                connection,
                key,
                args.into_iter().map(text).collect(),
            ),
            Self::Text(message) => feedback(state, connection, message),
            Self::Refused(failure, _) => failure.send(state, connection),
        }
    }
    fn into_text(self) -> String {
        match self {
            Self::Translated { key, args } => format!("{key} {}", args.join(" ")),
            Self::Text(message) => message,
            Self::Refused(failure, _) => failure.into_text(),
        }
    }
}

fn identifier(value: &str) -> Result<String, String> {
    let value = if value.contains(':') {
        value.to_owned()
    } else {
        format!("minecraft:{value}")
    };
    mc_data::Identifier::parse(value)
        .map(|id| id.to_string())
        .map_err(|e| e.to_string())
}

fn files(
    root: &Path,
    out: &mut Vec<PathBuf>,
    depth: usize,
    visited: &mut usize,
) -> Result<(), String> {
    if depth > 32 {
        return Err("Datapack directory nesting limit exceeded".into());
    }
    if !root.exists() {
        return Ok(());
    }
    if std::fs::symlink_metadata(root)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err(format!(
            "Datapack symlinks are unsupported: {}",
            root.display()
        ));
    }
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        *visited += 1;
        if *visited > MAX_RESOURCES {
            return Err("Datapack resource limit exceeded".into());
        }
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err(format!(
                "Datapack symlinks are unsupported: {}",
                entry.path().display()
            ));
        }
        if kind.is_dir() {
            files(&entry.path(), out, depth + 1, visited)?;
        } else if kind.is_file() {
            out.push(entry.path());
        }
        if out.len() > MAX_RESOURCES {
            return Err("Datapack resource limit exceeded".into());
        }
    }
    out.sort();
    Ok(())
}

/// Read a bounded binary resource; UTF-8 is required only by text consumers.
fn limited_bytes(reader: impl Read, remaining: &mut usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take((*remaining + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    *remaining = remaining
        .checked_sub(bytes.len())
        .ok_or("Datapack byte limit exceeded")?;
    Ok(bytes)
}

fn limited(reader: impl Read, remaining: &mut usize) -> Result<String, String> {
    String::from_utf8(limited_bytes(reader, remaining)?).map_err(|e| e.to_string())
}

fn read_text(path: &Path, remaining: &mut usize) -> Result<String, String> {
    if !std::fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_file()
    {
        return Err(format!(
            "Datapack resource must be a regular file: {}",
            path.display()
        ));
    }
    limited(
        std::fs::File::open(path).map_err(|e| e.to_string())?,
        remaining,
    )
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "Datapack archive must be a regular file: {}",
            path.display()
        ));
    }
    // Opening reads the whole central directory into memory, whatever
    // the entries are: bound it by the file, then by the entry count.
    if metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(format!("Datapack archive too large: {}", path.display()));
    }
    let archive = zip::ZipArchive::new(std::fs::File::open(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if archive.len() > MAX_RESOURCES {
        return Err(format!(
            "Datapack archive has too many entries: {}",
            path.display()
        ));
    }
    Ok(archive)
}

/// Where a pack's files live.
enum Source {
    Dir(PathBuf),
    Zip(PathBuf),
}

/// One discovered pack with its parsed `pack.mcmeta`.
struct Scanned {
    source: Source,
    metadata: serde_json::Value,
}

impl Source {
    /// Every binary resource below the pack's `data/`.
    fn data_files(
        &self,
        remaining: &mut usize,
        visited: &mut usize,
    ) -> Result<Vec<(String, Vec<u8>)>, String> {
        match self {
            Self::Dir(path) => {
                let data = path.join("data");
                let mut paths = Vec::new();
                files(&data, &mut paths, 0, visited)?;
                paths
                    .into_iter()
                    .map(|path| {
                        let relative = mc_data::resource_path::slash_relative(&data, &path)
                            .ok_or("Non-UTF8 datapack path")?;
                        let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                        Ok((relative, limited_bytes(file, remaining)?))
                    })
                    .collect()
            }
            Self::Zip(path) => {
                let mut archive = open_zip(path)?;
                let mut out = Vec::new();
                for index in 0..archive.len() {
                    let mut file = archive
                        .by_index(index)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                    let name = file.name().to_owned();
                    let Some(relative) = name.strip_prefix("data/") else {
                        continue;
                    };
                    if file.is_dir() || relative.is_empty() {
                        continue;
                    }
                    if file.is_symlink() {
                        return Err(format!("Datapack symlinks are unsupported: {name}"));
                    }
                    if relative.contains(['\\', '\0'])
                        || relative
                            .split('/')
                            .any(|part| part.is_empty() || part == "." || part == "..")
                    {
                        return Err(format!("Unsafe datapack archive path: {name}"));
                    }
                    *visited += 1;
                    if *visited > MAX_RESOURCES {
                        return Err("Datapack resource limit exceeded".into());
                    }
                    out.push((relative.to_owned(), limited_bytes(&mut file, remaining)?));
                }
                out.sort_by(|a, b| a.0.cmp(&b.0));
                if out.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                    return Err("Duplicate datapack archive resource path".into());
                }
                Ok(out)
            }
        }
    }
}

/// What one pass over the pack folder found.
#[derive(Default)]
struct Scan {
    /// Every directory and `.zip` pack with a valid `pack.mcmeta`.
    packs: BTreeMap<String, Scanned>,
    /// Entries that look like packs but cannot be read, by pack id, with why.
    failed: BTreeMap<String, String>,
    /// Entries that are no pack at all (a README, a stray file): ignored.
    not_packs: Vec<String>,
}

/// Read one pack's `pack.mcmeta` and check it names a `pack` object.
fn scan_pack(
    path: PathBuf,
    name: &str,
    is_dir: bool,
    remaining: &mut usize,
) -> Result<Scanned, String> {
    let (source, raw_meta) = if is_dir {
        let meta = read_text(&path.join("pack.mcmeta"), remaining)
            .map_err(|e| format!("pack.mcmeta: {e}"))?;
        (Source::Dir(path), meta)
    } else {
        let mut archive = open_zip(&path)?;
        let meta = {
            let file = archive
                .by_name("pack.mcmeta")
                .map_err(|e| format!("{name}: pack.mcmeta: {e}"))?;
            limited(file, remaining)?
        };
        (Source::Zip(path), meta)
    };
    let metadata: serde_json::Value =
        serde_json::from_str(&raw_meta).map_err(|e| format!("pack.mcmeta: {e}"))?;
    if !metadata
        .get("pack")
        .is_some_and(serde_json::Value::is_object)
    {
        return Err("pack.mcmeta requires a pack object".into());
    }
    Ok(Scanned { source, metadata })
}

fn scan(root: &Path, remaining: &mut usize) -> Result<Scan, String> {
    let mut found = Scan::default();
    if !root.exists() {
        return Ok(found);
    }
    if std::fs::symlink_metadata(root)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Datapack root cannot be a symlink".into());
    }
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        if found.packs.len() + found.failed.len() >= MAX_RESOURCES - 1 {
            return Err("Datapack resource limit exceeded".into());
        }
        let pack = entry.map_err(|e| e.to_string())?;
        let kind = pack.file_type().map_err(|e| e.to_string())?;
        let path = pack.path();
        let Ok(name) = pack.file_name().into_string() else {
            found
                .not_packs
                .push(pack.file_name().to_string_lossy().into_owned());
            continue;
        };
        if !kind.is_dir() && !(kind.is_file() && name.ends_with(".zip")) {
            found.not_packs.push(name);
            continue;
        }
        let id = format!("file/{name}");
        match scan_pack(path, &name, kind.is_dir(), remaining) {
            Ok(scanned) => {
                found.packs.insert(id, scanned);
            }
            Err(error) => {
                found.failed.insert(id, error);
            }
        }
    }
    Ok(found)
}

/// A JSON text component as a network component tag.
fn component(value: &serde_json::Value) -> Tag {
    use serde_json::Value;
    match value {
        Value::String(value) => Tag::String(value.clone()),
        Value::Bool(flag) => Tag::Byte(i8::from(*flag)),
        Value::Number(number) => number
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .map_or_else(|| Tag::Double(number.as_f64().unwrap_or(0.0)), Tag::Int),
        Value::Null => Tag::String(String::new()),
        Value::Array(items) => Tag::List(ListTag {
            element_type: tag_type::COMPOUND,
            elements: items
                .iter()
                .map(|item| match component(item) {
                    Tag::Compound(fields) => Tag::Compound(fields),
                    Tag::String(value) => text(value),
                    _ => text(item),
                })
                .collect(),
        }),
        Value::Object(fields) => Tag::Compound(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), component(value)))
                .collect(),
        ),
    }
}

fn description(metadata: &serde_json::Value) -> Tag {
    metadata
        .get("pack")
        .and_then(|pack| pack.get("description"))
        .map_or_else(|| Tag::String(String::new()), component)
}

/// Whether every feature `pack.mcmeta` requires is enabled in this world.
fn features_enabled(metadata: &serde_json::Value) -> bool {
    metadata.get("features").is_none_or(|features| {
        features
            .get("enabled")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|required| {
                required.iter().all(|feature| {
                    feature
                        .as_str()
                        .is_some_and(|feature| ENABLED_FEATURES.contains(&feature))
                })
            })
    })
}

impl PackSet {
    /// Load the packs on disk. A pack found on disk that the selection has
    /// never seen is enabled, like a pack newly dropped into a world's folder.
    pub(crate) fn load(root: &Path, selection: Selection) -> Result<Self, String> {
        Self::load_with(root, selection, true)
    }
    /// Like [`Self::load`], but a pack the selection has not seen stays out of
    /// it: `/datapack enable` changes the named pack only.
    fn load_known(root: &Path, selection: Selection) -> Result<Self, String> {
        Self::load_with(root, selection, false)
    }
    /// The packs a start, an offline export or a `/reload` runs. A pack newly found
    /// on disk is enabled, but one that cannot be read or that `admit` refuses is
    /// skipped with a warning naming it and the reason, so a new pack can stop neither
    /// a start nor a reload. `admit` decides what the caller can run: at a start, the
    /// content installs and the worldgen inputs prepare, validate and pass the replay
    /// ancestry fence; on `/reload`, the content installs and the immutable running
    /// generators stay unchanged. A skipped pack stays unselected, so it is no worldgen
    /// input, and a restart offers it again. This deliberately differs from vanilla,
    /// which enables a new pack and refuses to start when it fails (offering
    /// `--safeMode`); Krasnokamen has no safe mode. A pack the saved selection holds
    /// that no longer loads still refuses, naming pack and file: its bytes are the
    /// world's selected generation inputs.
    ///
    /// Trials run one at a time and only the last admitted state stays alive (a
    /// world has one worldgen staging directory). Returns it with the skip warnings.
    pub(crate) fn open<T, E: std::fmt::Display>(
        root: &Path,
        selection: Selection,
        mut admit: impl FnMut(Result<Self, String>) -> Result<T, E>,
    ) -> Result<(T, Vec<String>), E> {
        let first = match admit(Self::load(root, selection.clone())) {
            Ok(admitted) => return Ok((admitted, Vec::new())),
            Err(error) => error,
        };
        let mut remaining = MAX_BYTES;
        let Ok(found) = scan(root, &mut remaining) else {
            return Err(first);
        };
        let failed = found.failed;
        let new: Vec<String> = found
            .packs
            .into_keys()
            .chain(failed.keys().cloned())
            .filter(|id| !selection.enabled.contains(id) && !selection.disabled.contains(id))
            .collect();
        if new.is_empty() {
            return Err(first);
        }
        let mut accepted = selection;
        let mut admitted = Some(admit(Self::load_known(root, accepted.clone()))?);
        let mut skipped = Vec::new();
        for id in new {
            if let Some(error) = failed.get(&id) {
                skipped.push(format!(
                    "Skipping new datapack {id}, it is not enabled: {id}: {error}"
                ));
                continue;
            }
            let mut candidate = accepted.clone();
            candidate.enabled.push(id.clone());
            admitted = None;
            match admit(Self::load_known(root, candidate.clone())) {
                Ok(state) => {
                    accepted = candidate;
                    admitted = Some(state);
                }
                Err(error) => skipped.push(format!(
                    "Skipping new datapack {id}, it is not enabled: {error}"
                )),
            }
        }
        let admitted = match admitted {
            Some(state) => state,
            None => admit(Self::load_known(root, accepted))?,
        };
        Ok((admitted, skipped))
    }
    fn load_with(root: &Path, mut selection: Selection, discover: bool) -> Result<Self, String> {
        selection.validate()?;
        let mut remaining = MAX_BYTES;
        let scanned = scan(root, &mut remaining)?;
        // A pack the selection holds that cannot be read stops the load, as does one new to
        // the selection when the load would enable it; a refusal names the pack and the file.
        for (id, error) in &scanned.failed {
            if selection.enabled.contains(id) || (discover && !selection.disabled.contains(id)) {
                return Err(format!("{id}: {error}"));
            }
        }
        let (packs, failed, not_packs) = (scanned.packs, scanned.failed, scanned.not_packs);
        let mut descriptions = BTreeMap::from([("vanilla".to_owned(), vanilla_description())]);
        for (id, pack) in &packs {
            if (discover || selection.enabled.contains(id))
                && !selection.disabled.contains(id)
                && let Some(features) = pack.metadata.get("features")
            {
                let required = features
                    .get("enabled")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| format!("{id}: invalid features.enabled"))?;
                for feature in required {
                    let feature = feature
                        .as_str()
                        .ok_or_else(|| format!("{id}: feature identifier must be a string"))?;
                    if !ENABLED_FEATURES.contains(&feature) {
                        return Err(format!("{id}: world-disabled feature {feature}"));
                    }
                }
            }
            descriptions.insert(id.clone(), description(&pack.metadata));
        }
        let dropped: Vec<String> = selection
            .enabled
            .iter()
            .filter(|id| *id != "vanilla" && !packs.contains_key(*id))
            .cloned()
            .collect();
        selection
            .enabled
            .retain(|id| id == "vanilla" || packs.contains_key(id));
        selection.disabled.retain(|id| packs.contains_key(id));
        for id in packs.keys() {
            if discover && !selection.enabled.contains(id) && !selection.disabled.contains(id) {
                selection.enabled.push(id.clone());
            }
        }
        selection.validate()?;
        let mut result = Self {
            selection,
            descriptions,
            dropped,
            failed,
            not_packs,
            ..Self::default()
        };
        let mut visited = 0;
        let mut raw_tags = BTreeMap::<String, Vec<(String, bool)>>::new();
        let enabled = result.selection.enabled.clone();
        for id in &enabled {
            if id == "vanilla" {
                result.worldgen_layers.push(worldgen::Layer::vanilla());
                continue;
            }
            let pack = &packs[id];
            // A pack's `filter` hides matching files of every pack below it.
            let filters =
                Filter::parse(&pack.metadata).map_err(|e| format!("{id}: pack.mcmeta: {e}"))?;
            result.hide(&filters, &mut raw_tags);
            result.filters.extend(filters.iter().cloned());
            let mut layer = worldgen::Layer::new(id.clone(), filters);
            let data = pack
                .source
                .data_files(&mut remaining, &mut visited)
                .map_err(|e| format!("{id}: {e}"))?;
            // Worldgen this server cannot generate refuses the whole pack, naming
            // every such entry (a bounded list) instead of loading a partial pack.
            let mut unsupported = Vec::new();
            for (relative, bytes) in data {
                let add = || -> Result<(), String> {
                    worldgen::validate_path(&relative)?;
                    if worldgen::is_resource(&relative) {
                        let expanded = match worldgen::validate_resource(&relative, &bytes) {
                            Ok(expanded) => expanded,
                            Err(worldgen::ResourceError::Unsupported(entry)) => {
                                unsupported.push(entry);
                                return Ok(());
                            }
                            Err(worldgen::ResourceError::Invalid(error)) => return Err(error),
                        };
                        remaining = remaining
                            .checked_sub(expanded.saturating_sub(bytes.len()))
                            .ok_or("Datapack expanded byte limit exceeded")?;
                        layer.resources.insert(relative.clone(), bytes.into());
                    } else if is_ignored_directory(&relative) {
                        // Not decoded: vanilla never reads such a file, whatever its bytes.
                        result.ignored.push(format!("{id}: {relative}"));
                    } else {
                        let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
                        result.add_file(&mut raw_tags, id, &relative, text)?;
                    }
                    Ok(())
                };
                add().map_err(|e| format!("{id}: {relative}: {e}"))?;
            }
            if !unsupported.is_empty() {
                const NAMED: usize = 8;
                let mut named = unsupported[..unsupported.len().min(NAMED)].join(", ");
                if unsupported.len() > NAMED {
                    named.push_str(&format!(" and {} more", unsupported.len() - NAMED));
                }
                return Err(format!(
                    "{id}: unsupported worldgen this server cannot generate: {named}"
                ));
            }
            result.worldgen_layers.push(layer);
        }
        fn expand(
            id: &str,
            raw: &BTreeMap<String, Vec<(String, bool)>>,
            functions: &BTreeMap<String, Arc<[String]>>,
            visiting: &mut BTreeSet<String>,
            output: &mut Vec<String>,
            work: &mut usize,
        ) -> Result<(), String> {
            *work += 1;
            if visiting.len() >= 32 || *work > MAX_RESOURCES {
                return Err("Function tag expansion limit exceeded".into());
            }
            if !visiting.insert(id.to_owned()) {
                return Err(format!("Cyclic function tag: {id}"));
            }
            for (name, required) in raw
                .get(id)
                .ok_or_else(|| format!("Unknown function tag {id}"))?
            {
                let nested = name.strip_prefix('#');
                let name = identifier(nested.unwrap_or(name))?;
                if nested.is_some() {
                    if raw.contains_key(&name) {
                        expand(&name, raw, functions, visiting, output, work)?;
                    } else if *required {
                        return Err(format!("Unknown function tag {name}"));
                    }
                } else if functions.contains_key(&name) {
                    if !output.contains(&name) {
                        output.push(name);
                    }
                } else if *required {
                    return Err(format!("Unknown function {name}"));
                }
            }
            visiting.remove(id);
            Ok(())
        }
        for id in raw_tags.keys() {
            let mut output = Vec::new();
            expand(
                id,
                &raw_tags,
                &result.functions,
                &mut BTreeSet::new(),
                &mut output,
                &mut 0,
            )?;
            result.tags.insert(id.clone(), output.into());
        }
        Ok(result)
    }

    /// Remove what `filters` hide from the content the packs below have added.
    fn hide(&mut self, filters: &[Filter], raw_tags: &mut BTreeMap<String, Vec<(String, bool)>>) {
        if filters.is_empty() {
            return;
        }
        let hidden = |namespace: &str, path: &str| filters.iter().any(|f| f.hides(namespace, path));
        let split = |id: &str| {
            id.split_once(':')
                .map(|(n, p)| (n.to_owned(), p.to_owned()))
        };
        self.functions.retain(|id, _| {
            split(id).is_none_or(|(n, p)| !hidden(&n, &format!("function/{p}.mcfunction")))
        });
        raw_tags.retain(|id, _| {
            split(id).is_none_or(|(n, p)| !hidden(&n, &format!("tags/function/{p}.json")))
        });
        self.predicates.retain(|id, _| {
            split(id).is_none_or(|(n, p)| !hidden(&n, &format!("predicate/{p}.json")))
        });
        self.resources.retain(|path, _| {
            path.split_once('/')
                .is_none_or(|(n, rest)| !hidden(n, rest))
        });
    }

    /// Add one file of the pack `id` (`relative` is the path below `data/`).
    fn add_file(
        &mut self,
        raw_tags: &mut BTreeMap<String, Vec<(String, bool)>>,
        pack: &str,
        relative: &str,
        text: String,
    ) -> Result<(), String> {
        let (namespace, tail) = relative
            .split_once('/')
            .ok_or("Resource must be namespaced")?;
        let (kind, resource) = tail.split_once('/').ok_or("Resource must have a type")?;
        if kind == "function" && resource.ends_with(".mcfunction") {
            let id = identifier(&format!(
                "{namespace}:{}",
                resource
                    .strip_suffix(".mcfunction")
                    .expect("function extension checked")
            ))?;
            let lines = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if lines
                .iter()
                .any(|line| line.len() > 32767 || line.starts_with('/') || line.starts_with('$'))
            {
                return Err(format!(
                    "Unsupported function line in {id} (macros and leading slashes are unsupported)"
                ));
            }
            self.functions.insert(id, lines.into());
        } else if kind == "tags" && resource.starts_with("function/") && resource.ends_with(".json")
        {
            let id = identifier(&format!(
                "{namespace}:{}",
                resource
                    .strip_prefix("function/")
                    .expect("function tag directory checked")
                    .strip_suffix(".json")
                    .expect("tag extension checked")
            ))?;
            let tag: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let values = raw_tags.entry(id).or_default();
            if tag.get("replace").and_then(serde_json::Value::as_bool) == Some(true) {
                values.clear();
            }
            for value in tag
                .get("values")
                .and_then(serde_json::Value::as_array)
                .ok_or("Function tag requires values")?
            {
                let name = value
                    .as_str()
                    .or_else(|| value.get("id").and_then(serde_json::Value::as_str))
                    .ok_or("Invalid function tag entry")?;
                let required = value
                    .get("required")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true);
                values.push((name.to_owned(), required));
            }
        } else if matches!(kind, "recipe" | "loot_table") && resource.ends_with(".json") {
            self.resources.insert(relative.to_owned(), text);
            self.origins.insert(relative.to_owned(), pack.to_owned());
        } else if kind == "predicate" && resource.ends_with(".json") {
            let id = identifier(&format!(
                "{namespace}:{}",
                resource
                    .strip_suffix(".json")
                    .expect("predicate extension checked")
            ))?;
            let predicate: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let unsupported = mc_data::advancements::predicate_unsupported(&predicate);
            if !unsupported.is_empty() {
                return Err(format!(
                    "Unsupported predicate {id}: {}",
                    unsupported.into_iter().collect::<Vec<_>>().join(", ")
                ));
            }
            self.predicates.insert(id, predicate);
        } else {
            return Err("Unsupported datapack resource".into());
        }
        Ok(())
    }

    /// Fresh listing for `/datapack list`: the selected packs still on disk in
    /// priority order, then every other compatible pack found in the folder.
    pub(crate) fn listing(root: &Path, selection: &Selection) -> Result<Listing, String> {
        let mut remaining = MAX_BYTES;
        let scanned = scan(root, &mut remaining)?;
        let (packs, failed) = (scanned.packs, scanned.failed);
        let describe = |id: &String| {
            let description = if id == "vanilla" {
                vanilla_description()
            } else if let Some(error) = failed.get(id) {
                // A selected pack that no longer loads is still running; its hover says why.
                Tag::String(error.clone())
            } else {
                packs.get(id).map_or_else(
                    || Tag::String(String::new()),
                    |pack| description(&pack.metadata),
                )
            };
            (id.clone(), description)
        };
        Ok(Listing {
            enabled: selection
                .enabled
                .iter()
                .filter(|id| {
                    *id == "vanilla" || packs.contains_key(*id) || failed.contains_key(*id)
                })
                .map(describe)
                .collect(),
            available: packs
                .iter()
                .filter(|(id, pack)| {
                    !selection.enabled.contains(id) && features_enabled(&pack.metadata)
                })
                .map(|(id, _)| describe(id))
                .collect(),
        })
    }

    /// Whether the last scan found a pack (or archive that failed to read) by this id,
    /// selected or not: the repository "has" it, as vanilla's `getPack` means it.
    pub(crate) fn has_pack(&self, id: &str) -> bool {
        self.descriptions.contains_key(id) || self.failed.contains_key(id)
    }

    /// Whether installing `self` instead of `current` changes the registry:
    /// any difference in recipes, loot or predicates.
    pub(crate) fn changes_registry(&self, current: &Self) -> bool {
        self.resources != current.resources
            || self.predicates != current.predicates
            || self.filters != current.filters
    }

    /// One operator-facing warning per condition this load found that `previous`
    /// did not already have: a selected pack gone from disk and a non-pack entry in
    /// the folder. A new pack this load skipped is always stated.
    pub(crate) fn warnings(&self, previous: Option<&Self>) -> Vec<String> {
        let mut warnings = Vec::new();
        for id in &self.dropped {
            if previous.is_none_or(|previous| !previous.dropped.contains(id)) {
                warnings.push(format!("Datapack {id} is no longer on disk; saved starts are retained but inactive until its resources return"));
            }
        }
        // A skipped new pack is stated by every start and `/reload` that finds it still
        // broken (only those loads skip packs), so it is never silent the second time.
        warnings.extend(self.skipped.iter().cloned());
        for name in &self.not_packs {
            if previous.is_none_or(|previous| !previous.not_packs.contains(name)) {
                warnings.push(format!(
                    "Found non-pack entry '{name}' in the datapack folder, ignoring it"
                ));
            }
        }
        for path in &self.ignored {
            if previous.is_none_or(|previous| !previous.ignored.contains(path)) {
                warnings.push(format!(
                    "Ignoring datapack file {path}: its directory is not a vanilla data resource type"
                ));
            }
        }
        warnings
    }

    pub(crate) fn install_content(
        &self,
        registry: &mut crate::registry::CoreRegistry,
    ) -> Result<(), String> {
        registry.datapack_structures = Arc::clone(&self.worldgen_structures);
        registry
            .structure_catalog
            .clone_from(&self.structure_catalog);
        // The builtin vanilla pack is below every pack: their filters hide its files too.
        if !self.filters.is_empty() {
            let hidden = |id: &mc_data::Identifier, directory: &str| {
                let path = format!("{directory}/{}.json", id.path());
                self.filters.iter().any(|f| f.hides(id.namespace(), &path))
            };
            registry
                .recipes
                .retain(|recipe| !hidden(&recipe.id, "recipe"));
            registry.loot.remove_blocks(|block| {
                let path = block.path();
                self.filters
                    .iter()
                    .any(|f| f.hides(block.namespace(), &format!("loot_table/blocks/{path}.json")))
            });
        }
        let mut containers = BTreeMap::new();
        for (path, text) in &self.resources {
            let named = |error: String| match self.origins.get(path) {
                Some(pack) => format!("{pack}: {path}: {error}"),
                None => format!("{path}: {error}"),
            };
            // Container tables may have any namespace/path, not only chests/*.
            let (namespace, tail) = path
                .split_once('/')
                .ok_or_else(|| named("Missing namespace".into()))?;
            if let Some(resource) = tail.strip_prefix("loot_table/") {
                let value: serde_json::Value =
                    serde_json::from_str(text).map_err(|error| named(error.to_string()))?;
                if value
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|kind| {
                        matches!(kind.split_once(':'), Some(("minecraft", "chest" | "empty")))
                    })
                {
                    let id = mc_data::Identifier::parse(format!(
                        "{namespace}:{}",
                        resource
                            .strip_suffix(".json")
                            .ok_or_else(|| named("Resource must be JSON".into()))?
                    ))
                    .map_err(|error| named(error.to_string()))?;
                    containers.insert(id, text.clone());
                    continue;
                }
            }
            self.install_resource(registry, path, text).map_err(named)?;
        }
        if !containers.is_empty() || !self.filters.is_empty() {
            registry.chest_loot = Arc::new(
                registry
                    .chest_loot
                    .with_datapacks(
                        containers,
                        |id| {
                            self.filters.iter().any(|filter| {
                                filter.hides(
                                    id.namespace(),
                                    &format!("loot_table/{}.json", id.path()),
                                )
                            })
                        },
                        &registry.items,
                    )
                    .map_err(|error| {
                        let mc_data::loot::chest_26_1_2::ChestLootError::Unsupported { id, .. } =
                            &error
                        else {
                            return error.to_string();
                        };
                        let path = format!("{}/loot_table/{}.json", id.namespace(), id.path());
                        match self.origins.get(&path) {
                            Some(pack) => format!("{pack}: {path}: {error}"),
                            None => error.to_string(),
                        }
                    })?,
            );
        }
        registry.predicates = self.predicates.clone();
        registry.refresh_recipe_indexes();
        Ok(())
    }

    fn install_resource(
        &self,
        registry: &mut crate::registry::CoreRegistry,
        path: &str,
        text: &str,
    ) -> Result<(), String> {
        let (namespace, tail) = path.split_once('/').ok_or("Missing namespace")?;
        let (kind, resource) = tail.split_once('/').ok_or("Missing resource kind")?;
        let id = mc_data::Identifier::parse(format!(
            "{namespace}:{}",
            resource
                .strip_suffix(".json")
                .ok_or("Resource must be JSON")?
        ))
        .map_err(|e| e.to_string())?;
        if kind == "recipe" {
            let recipe = mc_data::recipes::parse_datapack_recipe(id, text)?;
            if registry.items.id_of(&recipe.result.item).is_none() {
                return Err(format!("Unknown recipe output {}", recipe.result.item));
            }
            if let Some(existing) = registry.recipes.iter_mut().find(|old| old.id == recipe.id) {
                *existing = recipe;
            } else {
                registry.recipes.push(recipe);
            }
        } else if let Some(block) = id.path().strip_prefix("blocks/") {
            let block = mc_data::Identifier::parse(format!("{namespace}:{block}"))
                .map_err(|e| e.to_string())?;
            registry.loot.override_datapack_block(block, text)?;
        } else {
            return Err(format!(
                "Unsupported datapack loot domain: {id}; supported: block and container tables"
            ));
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct Runtime {
    packs: Arc<PackSet>,
    /// Pack ids the folder held when the last reload or activation was refused:
    /// the old packs stay installed, but vanilla `getPack` answers from the disk.
    scanned: Option<BTreeSet<String>>,
    root: Option<PathBuf>,
    /// Unmodified startup content (including plugin contributions), used only
    /// to prepare replacement catalogs. TickState.registry remains authoritative.
    base_registry: Option<Arc<crate::registry::CoreRegistry>>,
    reload: Option<Reload>,
    load_pending: bool,
    pending: Vec<Frame>,
    pub(crate) running: bool,
    scheduled: BTreeMap<(u64, String), ()>,
    dirty: bool,
    selection_dirty: bool,
    /// Traced lines awaiting the end of the functions they called:
    /// `(depth, command, result)`, innermost last.
    trace_returns: Vec<(usize, String, i32)>,
    pub(crate) origin: Option<[f64; 3]>,
}

/// What the datapack worker does for one command.
#[derive(Clone, Debug)]
pub(crate) enum Job {
    /// `/reload`: recompile the selected packs.
    Reload,
    /// `/datapack enable|disable`: recompile with a changed selection.
    Activation {
        action: Action,
        id: String,
    },
    /// `/datapack list`: scan the pack folder for a fresh listing.
    List(ListScope),
    /// `/datapack create`: write an empty directory pack; `description` is
    /// the JSON text of its description component.
    Create {
        id: String,
        description: String,
    },
    DebugReport(Arc<crate::command_debug::Report>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Enable,
    Disable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListScope {
    All,
    Enabled,
    Available,
}
/// Datapack warnings the operator's chat shows per load; the server log has all of them.
const CHAT_WARNINGS: usize = 3;
/// A fresh pack listing: `(pack id, description component)` in vanilla order.
pub(crate) struct Listing {
    pub(crate) enabled: Vec<(String, Tag)>,
    pub(crate) available: Vec<(String, Tag)>,
}
/// A finished worker job.
enum Completed {
    Packs(Box<PackSet>, Option<Arc<crate::registry::CoreRegistry>>),
    Listing(Listing),
    Created,
    DebugReport(Arc<crate::command_debug::Report>),
}
struct Reload {
    receiver: std::sync::mpsc::Receiver<Result<Completed, Failure>>,
    connection: ConnectionId,
    job: Job,
    completion: Option<crate::command_context::CompletionId>,
}

struct Frame {
    /// Function id, for the `/debug function` trace.
    id: String,
    lines: Arc<[String]>,
    next: usize,
    started: bool,
    context: crate::command_context::Context,
}

/// Create the empty directory pack `root/<id>` with its `data/` folder and a
/// `pack.mcmeta` holding `description`, like vanilla `createPack`.
fn create_pack(root: &Path, id: &str, description: &str) -> Result<(), Failure> {
    let failed = |key: &'static str| Failure::Translated {
        key,
        args: vec![id.to_owned()],
    };
    let path = root.join(id);
    if std::fs::symlink_metadata(&path).is_ok() {
        return Err(failed("commands.datapack.create.already_exists"));
    }
    let description: serde_json::Value = serde_json::from_str(description)
        .map_err(|_| failed("commands.datapack.create.metadata_encode_failure"))?;
    let metadata = serde_json::json!({
        "pack": {
            "description": description,
            "min_format": DATA_PACK_FORMAT,
            "max_format": DATA_PACK_FORMAT,
        }
    });
    let written = std::fs::create_dir_all(root)
        .and_then(|()| std::fs::create_dir(&path))
        .and_then(|()| std::fs::create_dir(path.join("data")))
        .and_then(|()| std::fs::write(path.join("pack.mcmeta"), metadata.to_string()));
    if let Err(error) = written {
        tracing::warn!(pack = %path.display(), %error, "datapack create failed");
        let _ = std::fs::remove_dir_all(&path);
        return Err(failed("commands.datapack.create.io_failure"));
    }
    Ok(())
}

/// The identity of the immutable generators the running world serves.
fn running_identity(current: &PackSet) -> Result<String, String> {
    match &current.worldgen_identity {
        Some(identity) => Ok(identity.clone()),
        None => current.worldgen_fingerprint(),
    }
}

fn prepare_packs(
    mut packs: PackSet,
    current: &PackSet,
    running: &str,
    base: Option<&crate::registry::CoreRegistry>,
) -> Result<(PackSet, Option<Arc<crate::registry::CoreRegistry>>), String> {
    packs.worldgen_base.clone_from(&current.worldgen_base);
    let identity = packs.worldgen_fingerprint()?;
    if identity != running {
        return Err("Datapack worldgen inputs changed; immutable generators cannot be reloaded; restart the server to apply the selected inputs".into());
    }
    packs.worldgen_identity = Some(identity);
    packs.worldgen_structures = Arc::clone(&current.worldgen_structures);
    packs
        .structure_catalog
        .clone_from(&current.structure_catalog);
    let registry = if packs.changes_registry(current) {
        let mut registry = base
            .ok_or("No base registry for the reload")?
            .prepare_content();
        packs.install_content(&mut registry)?;
        Some(Arc::new(registry))
    } else {
        None
    };
    Ok((packs, registry))
}

/// The worker body: all filesystem access of a job. A refused reload or activation
/// also reports the pack ids on disk, so `/datapack disable` agrees with `/datapack list`.
fn work(
    root: &Path,
    selection: Selection,
    job: &Job,
    base: Option<&crate::registry::CoreRegistry>,
    current: &PackSet,
) -> Result<Completed, Failure> {
    match run_job(root, selection, job, base, current) {
        Err(failure) if matches!(job, Job::Reload | Job::Activation { .. }) => {
            let mut remaining = MAX_BYTES;
            Err(match scan(root, &mut remaining) {
                Ok(found) => Failure::Refused(
                    Box::new(failure),
                    found
                        .packs
                        .into_keys()
                        .chain(found.failed.into_keys())
                        .collect(),
                ),
                // A folder that cannot be scanned leaves the old repository standing.
                Err(_) => failure,
            })
        }
        result => result,
    }
}

fn run_job(
    root: &Path,
    selection: Selection,
    job: &Job,
    base: Option<&crate::registry::CoreRegistry>,
    current: &PackSet,
) -> Result<Completed, Failure> {
    match job {
        // Like a start: a new pack that cannot load, or that would change the running
        // immutable generators, is skipped with a named warning (a restart offers it
        // again) and the rest applies; a pack the selection holds that cannot load or
        // whose worldgen inputs changed refuses the reload.
        Job::Reload => {
            let base = base.ok_or("No base registry for the reload")?;
            let running = running_identity(current)?;
            let ((mut packs, registry), skipped) = PackSet::open(root, selection, |loaded| {
                prepare_packs(loaded?, current, &running, Some(base))
            })?;
            packs.skipped = skipped;
            Ok(Completed::Packs(Box::new(packs), registry))
        }
        Job::Activation { action, id } => {
            let packs = PackSet::load_known(root, selection)?;
            if *action == Action::Enable
                && let Some(error) = packs.failed.get(id)
            {
                return Err(Failure::Text(format!("{id}: {error}")));
            }
            // A pack that is not on disk cannot be enabled.
            if *action == Action::Enable && !packs.selection.enabled.contains(id) {
                return Err(Failure::Translated {
                    key: "commands.datapack.unknown",
                    args: vec![id.clone()],
                });
            }
            let (packs, registry) =
                prepare_packs(packs, current, &running_identity(current)?, base)?;
            Ok(Completed::Packs(Box::new(packs), registry))
        }
        Job::List(_) => Ok(Completed::Listing(PackSet::listing(root, &selection)?)),
        Job::Create { id, description } => {
            create_pack(root, id, description).map(|()| Completed::Created)
        }
        Job::DebugReport(report) => {
            report.write(root).map_err(Failure::Text)?;
            Ok(Completed::DebugReport(Arc::clone(report)))
        }
    }
}

impl Runtime {
    #[cfg(test)]
    pub(crate) fn new(
        packs: PackSet,
        root: PathBuf,
        base: Arc<crate::registry::CoreRegistry>,
    ) -> Self {
        Self {
            packs: Arc::new(packs),
            root: Some(root),
            base_registry: Some(base),
            load_pending: true,
            ..Self::default()
        }
    }
    pub(crate) fn install(
        &mut self,
        packs: PackSet,
        root: PathBuf,
        base: Arc<crate::registry::CoreRegistry>,
    ) {
        self.selection_dirty |= self.packs.selection != packs.selection;
        self.packs = Arc::new(packs);
        self.root = Some(root);
        self.base_registry = Some(base);
        self.load_pending = true;
    }

    pub(crate) fn pending_work(&self) -> bool {
        self.reload.is_some()
    }
    pub(crate) fn selection(&self) -> &Selection {
        &self.packs.selection
    }
    pub(crate) fn packs(&self) -> &PackSet {
        &self.packs
    }
    /// Whether the repository has this pack, selected or not (vanilla `getPack`).
    pub(crate) fn has_pack(&self, id: &str) -> bool {
        self.scanned
            .as_ref()
            .map_or_else(|| self.packs.has_pack(id), |ids| ids.contains(id))
    }
    /// Queue a worker-thread job for the current selection; the old packs and
    /// selection stay authoritative until it reports back. Tick-thread
    /// callers never touch the filesystem.
    pub(crate) fn request_job(
        &mut self,
        job: Job,
        connection: ConnectionId,
        completion: crate::command_context::CompletionId,
    ) -> Result<(), Failure> {
        let selection = self.selection().clone();
        self.spawn(job, selection, connection, Some(completion))
    }
    /// Queue an enable/disable activation of `selection`.
    pub(crate) fn request_activation(
        &mut self,
        connection: ConnectionId,
        selection: Selection,
        action: Action,
        id: String,
        completion: crate::command_context::CompletionId,
    ) -> Result<(), Failure> {
        selection.validate()?;
        self.spawn(
            Job::Activation { action, id },
            selection,
            connection,
            Some(completion),
        )
    }
    fn request_reload(
        &mut self,
        connection: ConnectionId,
        completion: crate::command_context::CompletionId,
    ) -> Result<(), String> {
        let selection = self.selection().clone();
        self.spawn(Job::Reload, selection, connection, Some(completion))
            .map_err(Failure::into_text)
    }
    fn spawn(
        &mut self,
        job: Job,
        selection: Selection,
        connection: ConnectionId,
        completion: Option<crate::command_context::CompletionId>,
    ) -> Result<(), Failure> {
        if self.reload.is_some() {
            return Err("Datapack reload already in progress".into());
        }
        let root = self
            .root
            .clone()
            .ok_or("No datapack directory configured")?;
        let base = self.base_registry.clone();
        let current = Arc::clone(&self.packs);
        let (tx, receiver) = std::sync::mpsc::sync_channel(1);
        let running = job.clone();
        std::thread::Builder::new()
            .name("datapack-worker".into())
            .spawn(move || {
                let _ = tx.send(work(&root, selection, &running, base.as_deref(), &current));
            })
            .map_err(|error| format!("Starting datapack worker: {error}"))?;
        self.reload = Some(Reload {
            receiver,
            connection,
            job,
            completion,
        });
        Ok(())
    }
    pub(crate) fn request_debug_report(
        &mut self,
        connection: ConnectionId,
        completion: crate::command_context::CompletionId,
        report: crate::command_debug::Report,
    ) -> Result<(), String> {
        self.spawn(
            Job::DebugReport(Arc::new(report)),
            self.selection().clone(),
            connection,
            Some(completion),
        )
        .map_err(Failure::into_text)
    }
    pub(crate) fn image(&self) -> Vec<(u64, String)> {
        self.scheduled.keys().cloned().collect()
    }
    pub(crate) fn restore(&mut self, image: Vec<(u64, String)>) -> Result<(), crate::CoreError> {
        if image.len() > MAX_RESOURCES {
            return Err(crate::CoreError::Storage(
                "Scheduled function limit exceeded".into(),
            ));
        }
        let mut scheduled = BTreeMap::new();
        for (due, name) in image {
            let raw = name.strip_prefix('#').unwrap_or(&name);
            if identifier(raw).map_err(crate::CoreError::Storage)? != raw {
                return Err(crate::CoreError::Storage(
                    "Noncanonical scheduled function identifier".into(),
                ));
            }
            if scheduled.insert((due, name), ()).is_some() {
                return Err(crate::CoreError::Storage(
                    "Duplicate scheduled function".into(),
                ));
            }
        }
        self.scheduled = scheduled;
        self.dirty = false;
        Ok(())
    }
    pub(crate) fn restore_selection(
        &mut self,
        selection: Selection,
    ) -> Result<(), crate::CoreError> {
        selection.validate().map_err(crate::CoreError::Storage)?;
        self.packs = std::sync::Arc::new(PackSet {
            selection,
            ..PackSet::default()
        });
        self.selection_dirty = false;
        Ok(())
    }
    pub(crate) fn journal(&mut self, entries: &mut Vec<crate::journal::JournalEntry>) {
        if std::mem::take(&mut self.dirty) {
            entries.push(crate::journal::JournalEntry::ScheduledFunctions {
                value: self.image(),
            });
        }
        if std::mem::take(&mut self.selection_dirty) {
            entries.push(crate::journal::JournalEntry::DatapackSelection {
                value: self.selection().clone(),
            });
        }
    }
}

fn feedback(state: &mut TickState, connection: ConnectionId, message: impl Into<String>) {
    state.push_player(
        connection,
        PlayerChange::Chat {
            message: message.into(),
        },
    );
}

pub(crate) fn enqueue(
    state: &mut TickState,
    name: &str,
    connection: ConnectionId,
    executor: EntityId,
) -> Result<(), String> {
    let context = crate::command_context::Context::source(state, connection, executor, 2);
    let tag = name.strip_prefix('#');
    let id = identifier(tag.unwrap_or(name))?;
    let runtime = &mut state.datapacks;
    if runtime.pending.len() >= COMMAND_BUDGET {
        return Err("Function stack budget exceeded".into());
    }
    if tag.is_some() {
        let names = runtime
            .packs
            .tags
            .get(&id)
            .ok_or_else(|| format!("Unknown function tag {id}"))?;
        if runtime.pending.len() + names.len() > COMMAND_BUDGET {
            return Err("Function stack budget exceeded".into());
        }
        for name in names.iter().rev() {
            runtime.pending.push(Frame {
                id: name.clone(),
                lines: runtime.packs.functions[name].clone(),
                next: 0,
                started: false,
                context,
            });
        }
    } else {
        let lines = runtime
            .packs
            .functions
            .get(&id)
            .ok_or_else(|| format!("Unknown function {id}"))?
            .clone();
        runtime.pending.push(Frame {
            id,
            lines,
            next: 0,
            started: false,
            context,
        });
    }
    Ok(())
}
pub(crate) fn execute_block(state: &mut TickState, text: &str, changes: &mut ChangeSet) {
    let connection = crate::command_console::CONNECTION;
    let executor = crate::command_console::EXECUTOR;
    let text = text.trim_start_matches('/');
    let context = crate::command_context::Context::source(state, connection, executor, 2);
    crate::command_context::submit(state, text, context, changes);
}

pub(crate) fn drain(state: &mut TickState, changes: &mut ChangeSet) {
    if state.datapacks.running {
        return;
    }
    drain_until(state, changes, 0);
}

pub(crate) fn pending_depth(state: &TickState) -> usize {
    state.datapacks.pending.len()
}

/// A traced invocation shares the ordinary interpreter and budget, but must
/// finish its own frames before returning to the caller's pending function.
pub(crate) fn drain_until(state: &mut TickState, changes: &mut ChangeSet, stop_depth: usize) {
    crate::command_context::begin_sequence(state);
    let running = std::mem::replace(&mut state.datapacks.running, true);
    while state.datapacks.pending.len() > stop_depth {
        let mut frame = state
            .datapacks
            .pending
            .pop()
            .expect("pending function frame");
        if !frame.started {
            if let Err(error) = crate::command_context::charge(state) {
                state.datapacks.pending.clear();
                feedback(state, frame.context.connection, error);
                break;
            }
            frame.started = true;
            // The frames below this one are its callers: a call is traced at
            // the caller's depth, its lines one level deeper.
            let depth = state.datapacks.pending.len();
            if let Some(trace) = &mut state.debug_trace {
                trace.on_call(depth, &frame.id, frame.lines.len());
            }
        }
        if frame.next == frame.lines.len() {
            release_returns(state);
            continue;
        }
        if crate::command_context::exhausted(state) {
            crate::command_debug::trace_budget_exhausted(state);
            state.datapacks.pending.clear();
            feedback(
                state,
                frame.context.connection,
                "Function command sequence budget exceeded",
            );
            break;
        }
        let lines = frame.lines.clone();
        let index = frame.next;
        let context = frame.context;
        frame.next += 1;
        state.datapacks.pending.push(frame);
        if state.debug_trace.is_some() {
            trace_line(state, &lines[index], context, changes);
        } else {
            crate::command_context::submit(state, &lines[index], context, changes);
        }
        if state.command_context.waiting() {
            state.command_context.suspend_functions(stop_depth);
            break;
        }
    }
    release_returns(state);
    state.datapacks.running = running;
}

pub(crate) fn cancel_pending(state: &mut TickState) {
    state.datapacks.pending.clear();
    state.datapacks.trace_returns.clear();
    state.debug_trace = None;
}

/// Print the results of traced lines whose called functions have finished: a
/// line at depth `d` is complete once no frame deeper than `d` remains.
fn release_returns(state: &mut TickState) {
    while state
        .datapacks
        .trace_returns
        .last()
        .is_some_and(|(depth, ..)| state.datapacks.pending.len() <= *depth)
    {
        let (depth, line, result) = state.datapacks.trace_returns.pop().expect("checked above");
        if let Some(trace) = &mut state.debug_trace {
            trace.on_return(depth, &line, result);
        }
    }
}

/// Run one function line for `/debug function`, recording the command and its
/// result like `ExecuteCommand` does. A line that called functions reports its
/// result after theirs, as vanilla's callback does.
fn trace_line(
    state: &mut TickState,
    line: &str,
    context: crate::command_context::Context,
    changes: &mut ChangeSet,
) {
    let depth = state.datapacks.pending.len();
    if let Some(trace) = &mut state.debug_trace {
        trace.on_command(depth, line);
    }
    state.command_context.last_result = 0;
    crate::command_context::run(state, line, context, changes);
    let result = state.command_context.last_result;
    if state.datapacks.pending.len() > depth {
        state
            .datapacks
            .trace_returns
            .push((depth, line.to_owned(), result));
    } else if let Some(trace) = &mut state.debug_trace {
        if state.command_context.waiting() {
            trace.awaiting = Some((depth, line.to_owned()));
        } else {
            trace.on_return(depth, line, result);
        }
    }
}

pub(crate) fn command(
    state: &mut TickState,
    connection: ConnectionId,
    executor: EntityId,
    text: &str,
    changes: &mut ChangeSet,
) -> crate::command_execution::CommandResult {
    use crate::command_execution::CommandResult::{Failed, Unavailable};
    let Some(root) = text.split_whitespace().next() else {
        return Unavailable;
    };
    if root == "data" {
        return crate::command_data::command(state, connection, executor, text);
    }
    if root == "datapack" {
        return management::command(state, connection, text);
    }
    if !matches!(root, "function" | "schedule" | "reload") {
        return Unavailable;
    }
    let words = text.split_whitespace().collect::<Vec<_>>();
    let result = match words.as_slice() {
        ["function", name] => enqueue(state, name, connection, executor),
        ["schedule", "clear", name] => {
            let tag = name.starts_with('#');
            identifier(name.strip_prefix('#').unwrap_or(name)).map(|name| {
                let name = if tag { format!("#{name}") } else { name };
                state.datapacks.scheduled.retain(|(_, scheduled), _| scheduled != &name);
                state.datapacks.dirty = true;
            })
        }
        ["schedule", "function", name, time] | ["schedule", "function", name, time, "replace" | "append"] => {
            (|| {
                let multiplier = if time.ends_with('d') { 24000.0 } else if time.ends_with('s') { 20.0 } else { 1.0 };
                let number = time.strip_suffix(['d', 's', 't']).unwrap_or(time);
                let delay = number.parse::<f64>().map_err(|_| "Invalid schedule time")? * multiplier;
                if !delay.is_finite() || delay < 1.0 || delay > i32::MAX as f64 { return Err("Schedule delay must be 1..2147483647 ticks".into()); }
                let id = identifier(name.strip_prefix('#').unwrap_or(name))?;
                let exists = if name.starts_with('#') { state.datapacks.packs.tags.contains_key(&id) } else { state.datapacks.packs.functions.contains_key(&id) };
                if !exists { return Err(format!("Unknown function {name}")); }
                let id = if name.starts_with('#') { format!("#{id}") } else { id };
                if words.last() != Some(&"append") { state.datapacks.scheduled.retain(|(_, name), _| name != &id); }
                if state.datapacks.scheduled.len() >= MAX_RESOURCES { return Err("Scheduled function limit exceeded".into()); }
                let due = crate::command_context::execution_tick(state).saturating_add(delay.round() as u64);
                state.datapacks.scheduled.insert((due, id), ());
                state.datapacks.dirty = true;
                Ok(())
            })()
        }
        ["reload"] => {
            let queued = (|| {
                let completion = state.command_context.completion_id()?;
                if state.datapacks.base_registry.is_none() {
                    state.datapacks.base_registry = Some(Arc::clone(&state.registry));
                }
                state.datapacks.request_reload(connection, completion)?;
                Ok::<_, String>(completion)
            })();
            return match queued {
                Ok(id) => crate::command_execution::CommandResult::Deferred(id),
                Err(error) => {
                    feedback(state, connection, error);
                    Failed
                }
            };
        }
        _ => Err("Supported: function <id|#tag>; schedule function <id> <time> [append|replace]; schedule clear <id>; reload; datapack list".into()),
    };
    let outcome = if let Err(error) = result {
        feedback(state, connection, error);
        Failed
    } else {
        crate::command_execution::CommandResult::Completed
    };
    drain(state, changes);
    outcome
}

/// Publish the registry prepared by the worker in every world.
fn install_registry(state: &mut TickState, registry: Arc<crate::registry::CoreRegistry>) {
    if state.dimension_globals.active {
        state
            .dimension_globals
            .broadcasts
            .push(crate::dimensions::globals::Broadcast::Registry(Arc::clone(
                &registry,
            )));
    }
    for world in state.other_dimensions.values_mut() {
        world.registry = Arc::clone(&registry);
    }
    adopt_registry(state, registry);
}

/// Use a reloaded registry in this world and tell its clients.
pub(crate) fn adopt_registry(state: &mut TickState, registry: Arc<crate::registry::CoreRegistry>) {
    state.registry = Arc::clone(&registry);
    if let Some(published) = &state.published_registry {
        *published
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::clone(&registry);
    }
    let reload = crate::boundary::RegistryReload::new(registry);
    let connections: Vec<ConnectionId> = state.connections.all_connections().collect();
    for connection in connections {
        state.push_player(
            connection,
            PlayerChange::RecipesReloaded {
                reload: reload.clone(),
            },
        );
        if let Some(entity) = state.connections.entity_of(connection) {
            let changes = state
                .players
                .get_mut(&entity)
                .map(|player| player.reload_recipes(&state.registry.item_rules()))
                .unwrap_or_default();
            for change in changes {
                state.push_player(connection, change);
            }
            crate::advancements::sync_join(state, entity);
        }
    }
}

/// Show a finished worker job's outcome and, for recompiled packs, install them.
fn finish(
    state: &mut TickState,
    reload: Reload,
    result: Result<Completed, Failure>,
) -> (bool, i32) {
    let connection = reload.connection;
    let deliver = reload
        .completion
        .is_none_or(|id| crate::command_context::completion_context(state, id).is_some());
    let completed = match result {
        Err(Failure::Refused(failure, repository)) => {
            state.datapacks.scanned = Some(repository);
            Err(*failure)
        }
        other => other,
    };
    let completed = match completed {
        Ok(completed) => completed,
        Err(failure) => {
            if deliver {
                if matches!(reload.job, Job::DebugReport(_)) {
                    tracing::warn!(error = %failure.into_text(), "debug trace write failed");
                    crate::command_execution::feedback(
                        state,
                        connection,
                        "commands.debug.function.traceFailed",
                        Vec::new(),
                    );
                } else if let (Job::Reload | Job::Activation { .. }, Failure::Text(text)) =
                    (&reload.job, &failure)
                {
                    // The worker refused before anything was installed.
                    feedback(
                        state,
                        connection,
                        format!("{text}; old packs and selection retained"),
                    );
                } else {
                    failure.send(state, connection);
                }
            }
            return (false, 0);
        }
    };
    match (&reload.job, completed) {
        (Job::Reload | Job::Activation { .. }, Completed::Packs(packs, registry)) => {
            if let Some(registry) = registry {
                install_registry(state, registry);
            }
            let count = packs.selection.enabled.len() as i32;
            state.datapacks.selection_dirty |= state.datapacks.packs.selection != packs.selection;
            let warnings = packs.warnings(Some(&state.datapacks.packs));
            state.datapacks.packs = Arc::new(*packs);
            state.datapacks.scanned = None;
            state.datapacks.load_pending = true;
            if deliver {
                match &reload.job {
                    Job::Activation { action, id } => {
                        management::activated(state, connection, *action, id)
                    }
                    _ => crate::command_execution::success_feedback(
                        state,
                        connection,
                        "commands.reload.success",
                        Vec::new(),
                    ),
                }
            }
            for warning in &warnings {
                tracing::warn!("{warning}");
            }
            if deliver {
                let hidden = warnings.len().saturating_sub(CHAT_WARNINGS);
                for warning in warnings.into_iter().take(CHAT_WARNINGS) {
                    feedback(state, connection, warning);
                }
                if hidden > 0 {
                    feedback(
                        state,
                        connection,
                        format!("{hidden} more datapack warning(s), see the server log"),
                    );
                }
            }
            (true, count)
        }
        (Job::List(scope), Completed::Listing(listing)) => {
            if deliver {
                management::listed(state, connection, *scope, &listing);
            }
            (true, management::counted(&listing, *scope))
        }
        (Job::Create { id, .. }, Completed::Created) => {
            if deliver {
                management::created(state, connection, id);
            }
            (true, 1)
        }
        (Job::DebugReport(_), Completed::DebugReport(report)) => {
            if deliver {
                if report.success {
                    crate::command_execution::success_feedback(
                        state,
                        connection,
                        report.feedback,
                        report.arguments.clone(),
                    );
                } else {
                    crate::command_execution::feedback(
                        state,
                        connection,
                        report.feedback,
                        report.arguments.clone(),
                    );
                }
            }
            (report.success, report.result)
        }
        _ => unreachable!("a worker answers its own job"),
    }
}

pub(crate) fn poll_worker(state: &mut TickState) {
    if state.dimension_globals.active && !state.command_context.shared {
        return;
    }
    if let Some(polled) = state
        .datapacks
        .reload
        .as_ref()
        .map(|reload| reload.receiver.try_recv())
    {
        match polled {
            Ok(result) => {
                let reload = state.datapacks.reload.take().expect("polled worker job");
                let completion = reload.completion;
                let previous = state.command_context.current;
                state.command_context.current = completion
                    .and_then(|id| crate::command_context::completion_context(state, id))
                    .or(previous);
                let (success, value) = finish(state, reload, result);
                if let Some(id) = completion {
                    crate::command_context::complete(state, id, success, value);
                }
                state.command_context.current = previous;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let reload = state.datapacks.reload.take().expect("polled worker job");
                if let Some(id) = reload.completion {
                    crate::command_context::complete(state, id, false, 0);
                }
                feedback(
                    state,
                    reload.connection,
                    "Datapack worker disconnected; old packs and selection retained",
                );
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
}

pub(crate) fn tick(state: &mut TickState, changes: &mut ChangeSet) {
    poll_worker(state);
    crate::command_context::resume(state, changes);
    if state.command_context.waiting() {
        return;
    }
    if std::mem::take(&mut state.datapacks.load_pending)
        && state.datapacks.packs.tags.contains_key("minecraft:load")
    {
        execute_block(state, "function #minecraft:load", changes);
    }
    for _ in 0..COMMAND_BUDGET {
        if !state
            .datapacks
            .scheduled
            .first_key_value()
            .is_some_and(|((due, _), _)| *due <= state.tick)
        {
            break;
        }
        let ((_, name), ()) = state.datapacks.scheduled.pop_first().expect("due entry");
        state.datapacks.dirty = true;
        execute_block(state, &format!("function {name}"), changes);
    }
    if state.datapacks.packs.tags.contains_key("minecraft:tick") {
        execute_block(state, "function #minecraft:tick", changes);
    }
}

/// Wait for the running worker job and trace writers (blocking on their
/// channels, no polling), then deliver their results on a tick of the
/// datapack runtime: the test stand-in for "a later tick".
#[cfg(test)]
pub(crate) fn settle(state: &mut TickState) {
    fn answered<T>(value: T) -> std::sync::mpsc::Receiver<T> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        tx.send(value).expect("fresh channel");
        rx
    }
    if let Some(reload) = state.datapacks.reload.take() {
        let result = reload.receiver.recv().expect("worker answers");
        state.datapacks.reload = Some(Reload {
            receiver: answered(result),
            ..reload
        });
    }
    tick(state, &mut ChangeSet::default());
}

#[cfg(test)]
#[path = "datapacks_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "datapack_commands_tests.rs"]
mod command_tests;

#[path = "datapack_management.rs"]
mod management;

#[cfg(test)]
mod iron_smelting_tests {
    use super::PackSet;
    use crate::registry::CoreRegistry;

    #[test]
    fn recipe_install_refreshes_iron_smelting_fact() {
        let mut registry = CoreRegistry::load(None).unwrap();
        registry.recipes.clear();
        registry.refresh_recipe_indexes();
        assert!(registry.iron_smelting.is_none());

        let mut packs = PackSet::default();
        packs.resources.insert(
            "test/recipe/iron.json".into(),
            r#"{"type":"minecraft:smelting","ingredient":"minecraft:raw_iron","result":{"id":"minecraft:iron_ingot"},"cookingtime":37}"#.into(),
        );
        packs.install_content(&mut registry).unwrap();
        assert_eq!(registry.iron_smelting.unwrap().cook_ticks, 37);

        packs.resources.insert(
            "test/recipe/iron.json".into(),
            r#"{"type":"minecraft:smelting","ingredient":"minecraft:raw_iron","result":{"id":"minecraft:iron_ingot"},"cookingtime":53}"#.into(),
        );
        packs.install_content(&mut registry).unwrap();
        assert_eq!(registry.iron_smelting.unwrap().cook_ticks, 53);
    }
}

#[cfg(test)]
#[path = "datapacks_container_tests.rs"]
mod container_tests;
