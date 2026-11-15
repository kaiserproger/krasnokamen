//! Selected binary resources and their bounded, private generator input image.
use super::{BTreeMap, BTreeSet, Filter, PackSet, Path, PathBuf, Selection};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[path = "datapacks_worldgen_dimensions.rs"]
mod dimensions;

const MAX_CACHE_FILES: usize = 131_072;
const MAX_CACHE_BYTES: usize = 256 * 1024 * 1024;
const STAGE: &str = ".krasnokamen-worldgen";
const OWNER: &str = ".krasnokamen-worldgen.owner";
const IDENTITY: &str = "KrasnokamenWorldgenInputs";
type Resources = BTreeMap<String, Arc<[u8]>>;

#[derive(serde::Serialize, serde::Deserialize)]
struct Identity {
    digest: String,
    sources: BTreeMap<String, String>,
    structure_owners: BTreeMap<String, String>,
}

pub(crate) struct Adoption {
    pub(crate) changed: bool,
    pub(crate) catalog: Arc<StructureCatalog>,
}

/// Saved starts remain opaque durable data when their pack is absent.
#[derive(Debug)]
pub(crate) struct StructureCatalog {
    active: BTreeSet<mc_data::Identifier>,
    /// Selected generation differs from vanilla and must survive a pack change.
    pub(crate) durable_baselines: bool,
    owners: BTreeMap<String, String>,
    warned: parking_lot::Mutex<BTreeSet<mc_data::Identifier>>,
}

impl StructureCatalog {
    pub(crate) fn observe(&self, chunk: &mc_world::Chunk) -> Result<(), String> {
        for site in mc_world::structure_starts::read(&chunk.extras, chunk.pos, false)? {
            if !self.active.contains(&site.structure)
                && self.warned.lock().insert(site.structure.clone())
            {
                let pack = self
                    .owners
                    .get(site.structure.as_str())
                    .map(String::as_str)
                    .unwrap_or("unknown source pack; restore the structure's namespace");
                tracing::warn!(structure = %site.structure, likely_missing_pack = pack,
                    "saved structure start is inactive; retained unchanged until its pack returns");
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct Layer {
    id: String,
    filters: Vec<Filter>,
    pub(super) resources: Resources,
}
impl Layer {
    pub(super) fn new(id: String, filters: Vec<Filter>) -> Self {
        Self {
            id,
            filters,
            resources: BTreeMap::new(),
        }
    }
    pub(super) fn vanilla() -> Self {
        Self::new("vanilla".into(), Vec::new())
    }
}

pub(super) fn validate_path(path: &str) -> Result<(), String> {
    if path.len() > 1024
        || path.contains(['\\', '\0'])
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(format!("Unsafe datapack resource path: {path}"));
    }
    let (namespace, resource) = path.split_once('/').ok_or("Resource must be namespaced")?;
    mc_data::Identifier::parse(format!("{namespace}:{resource}"))
        .map_err(|e| format!("Invalid datapack resource path {path}: {e}"))?;
    Ok(())
}

pub(super) fn is_resource(path: &str) -> bool {
    path.split_once('/').is_some_and(|(_, tail)| {
        tail.starts_with("worldgen/")
            || tail.starts_with("structure/")
            || tail.starts_with("tags/worldgen/")
            || tail.starts_with("dimension/")
            || tail.starts_with("dimension_type/")
            || tail.starts_with("msl_pieces_spawn_counts/")
            || tail.starts_with("msl_pieces_spawn_counts_additions/")
    })
}

pub(super) fn validate_resource(path: &str, bytes: &[u8]) -> Result<usize, String> {
    let (_, tail) = path.split_once('/').ok_or("Missing namespace")?;
    if tail.starts_with("structure/") && tail.ends_with(".nbt") {
        let decoded;
        let mut input = if bytes.starts_with(&[0x1f, 0x8b]) {
            let mut budget = super::MAX_BYTES;
            decoded = super::limited_bytes(flate2::read::GzDecoder::new(bytes), &mut budget)?;
            decoded.as_slice()
        } else {
            bytes
        };
        let expanded_bytes = input.len();
        let (_, tag) =
            mc_nbt::read_named(&mut input).map_err(|e| format!("Invalid structure NBT: {e}"))?;
        if !input.is_empty() || !matches!(tag, mc_nbt::Tag::Compound(_)) {
            return Err("Structure NBT must be one complete named compound".into());
        }
        return Ok(expanded_bytes);
    }
    let supported = [
        "worldgen/structure_set/",
        "worldgen/structure/",
        "worldgen/template_pool/",
        "worldgen/processor_list/",
        "tags/worldgen/biome/",
    ];
    if !tail.ends_with(".json") || !supported.iter().any(|prefix| tail.starts_with(prefix)) {
        return Err(format!("Unsupported datapack worldgen resource: {path}"));
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid worldgen JSON: {e}"))?;
    if !value.is_object() {
        return Err("Worldgen resource must be a JSON object".into());
    }
    if tail.starts_with("worldgen/structure/") {
        let name = value["type"].as_str().ok_or("Structure requires type")?;
        let kind = mc_data::Identifier::parse(name).map_err(|e| e.to_string())?;
        if !mc_worldgen::village::supports_structure_type(&kind) {
            return Err(format!("Unsupported datapack structure type: {kind}"));
        }
    }
    if tail.starts_with("tags/") {
        tag_values(&value)?;
    }
    Ok(bytes.len())
}

fn collect(
    root: &Path,
    relative: &str,
    depth: usize,
    out: &mut Resources,
    remaining: &mut usize,
    visited: &mut usize,
) -> Result<(), String> {
    if depth > 32 {
        return Err(format!(
            "Worldgen cache nesting limit at {}",
            root.display()
        ));
    }
    let meta = std::fs::symlink_metadata(root).map_err(|e| format!("{}: {e}", root.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!("Worldgen cache symlink: {}", root.display()));
    }
    *visited += 1;
    if *visited > MAX_CACHE_FILES {
        return Err("Worldgen cache file limit exceeded".into());
    }
    if meta.is_dir() {
        for entry in std::fs::read_dir(root).map_err(|e| format!("{}: {e}", root.display()))? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "Non-UTF8 worldgen cache path")?;
            let path = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            collect(&entry.path(), &path, depth + 1, out, remaining, visited)?;
        }
    } else if meta.is_file() {
        let file = std::fs::File::open(root).map_err(|e| format!("{}: {e}", root.display()))?;
        let bytes = super::limited_bytes(file, remaining)
            .map_err(|e| format!("{}: {e}", root.display()))?;
        insert(out, relative.to_owned(), bytes.into())?;
    } else {
        return Err(format!(
            "Worldgen cache input is not a regular file: {}",
            root.display()
        ));
    }
    Ok(())
}

fn baseline(root: Option<&Path>) -> Result<Resources, String> {
    let mut out = BTreeMap::new();
    if let Some(root) = root {
        let mut remaining = MAX_CACHE_BYTES;
        let mut visited = 0;
        for (name, entries) in mc_data::biome_convention_tags::default_biome_tags() {
            let (namespace, path) = name
                .split_once(':')
                .ok_or("Invalid authored biome convention id")?;
            let values: Vec<_> = entries
                .iter()
                .map(|(id, required)| serde_json::json!({"id":id,"required":required}))
                .collect();
            let bytes = serde_json::to_vec(&serde_json::json!({"values":values}))
                .map_err(|e| e.to_string())?;
            remaining = remaining
                .checked_sub(bytes.len())
                .ok_or("Worldgen cache byte limit exceeded")?;
            visited += 1;
            insert(
                &mut out,
                format!("data/{namespace}/tags/worldgen/biome/{path}.json"),
                bytes.into(),
            )?;
        }
        // Generators read data and registry reports, never assets or neighbouring version caches.
        for name in ["data", "reports"] {
            collect(
                &root.join(name),
                name,
                0,
                &mut out,
                &mut remaining,
                &mut visited,
            )?;
        }
    }
    Ok(out)
}

fn tag_values(value: &serde_json::Value) -> Result<Vec<(String, bool)>, String> {
    if value.get("replace").is_some_and(|v| !v.is_boolean()) {
        return Err("Biome tag replace must be boolean".into());
    }
    value["values"]
        .as_array()
        .ok_or("Biome tag requires values")?
        .iter()
        .map(|entry| {
            let name = entry
                .as_str()
                .or_else(|| entry["id"].as_str())
                .ok_or("Invalid biome tag entry")?;
            let nested = name.starts_with('#');
            let name = super::identifier(name.strip_prefix('#').unwrap_or(name))?;
            let required = match entry.get("required") {
                None => true,
                Some(value) => value
                    .as_bool()
                    .ok_or("Biome tag required must be boolean")?,
            };
            Ok((if nested { format!("#{name}") } else { name }, required))
        })
        .collect()
}

fn insert(out: &mut Resources, path: String, bytes: Arc<[u8]>) -> Result<(), String> {
    if path.contains("/tags/worldgen/biome/") {
        let mut value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("{path}: {e}"))?;
        tag_values(&value).map_err(|e| format!("{path}: {e}"))?;
        if value.get("replace").and_then(serde_json::Value::as_bool) != Some(true)
            && let Some(previous) = out.get(&path)
        {
            let previous: serde_json::Value =
                serde_json::from_slice(previous).map_err(|e| format!("{path}: {e}"))?;
            let mut merged = previous["values"]
                .as_array()
                .ok_or("Lower biome tag requires values")?
                .clone();
            merged.extend(
                value["values"]
                    .as_array()
                    .ok_or("Biome tag requires values")?
                    .iter()
                    .cloned(),
            );
            value["values"] = serde_json::Value::Array(merged);
        }
        out.insert(
            path,
            serde_json::to_vec(&value)
                .map_err(|e| e.to_string())?
                .into(),
        );
    } else {
        out.insert(path, bytes);
    }
    Ok(())
}

impl PackSet {
    pub(crate) fn bind_structure_catalog(&mut self, catalog: Arc<StructureCatalog>) {
        self.structure_catalog = Some(catalog);
    }

    fn worldgen_resources(&self) -> Result<Resources, String> {
        self.overlay_resources(baseline(self.worldgen_base.as_deref())?)
    }

    fn overlay_resources(&self, base: Resources) -> Result<Resources, String> {
        let mut out = BTreeMap::new();
        // No worldgen overlay: retain the owned baseline without cloning its tree.
        if self
            .worldgen_layers
            .iter()
            .all(|layer| layer.resources.is_empty() && layer.filters.is_empty())
        {
            return Ok(base);
        }
        for layer in &self.worldgen_layers {
            out.retain(|path: &String, _| {
                path.strip_prefix("data/")
                    .and_then(|p| p.split_once('/'))
                    .is_none_or(|(n, p)| !layer.filters.iter().any(|f| f.hides(n, p)))
            });
            if layer.id == "vanilla" {
                for (path, bytes) in &base {
                    insert(&mut out, path.clone(), Arc::clone(bytes))?;
                }
            } else {
                for (path, bytes) in &layer.resources {
                    insert(&mut out, format!("data/{path}"), Arc::clone(bytes))?;
                }
            }
        }
        Ok(out)
    }

    pub(super) fn worldgen_fingerprint(&self) -> Result<String, String> {
        Ok(fingerprint(&self.worldgen_resources()?))
    }

    /// Freeze the exact selected resource image before spawn and replay reads.
    pub(crate) fn prepare_worldgen(
        &mut self,
        world: &Path,
        base: Option<&Path>,
    ) -> Result<Inputs, String> {
        Stage::cleanup(world)?;
        self.worldgen_base = base.map(Path::to_path_buf);
        let base_resources = baseline(base)?;
        let base_identity = fingerprint(&base_resources);
        let resources = self.overlay_resources(base_resources)?;
        let identity = fingerprint(&resources);
        self.worldgen_identity = Some(identity.clone());
        let biome_tags = Arc::new(BiomeTags::load(&resources)?);
        let validation: Vec<_> = self
            .worldgen_layers
            .iter()
            .flat_map(|layer| layer.resources.iter())
            .filter(|(path, bytes)| {
                resources
                    .get(&format!("data/{path}"))
                    .is_some_and(|effective| Arc::ptr_eq(effective, bytes))
            })
            .map(|(path, _)| path.clone())
            .collect();
        let mut structure_sets = Vec::new();
        for layer in &self.worldgen_layers {
            for (path, bytes) in &layer.resources {
                if let Some((namespace, resource)) = path.split_once('/')
                    && let Some(name) = resource
                        .strip_prefix("worldgen/structure_set/")
                        .and_then(|p| p.strip_suffix(".json"))
                    && resources
                        .get(&format!("data/{path}"))
                        .is_some_and(|effective| Arc::ptr_eq(effective, bytes))
                {
                    let id = mc_data::Identifier::parse(format!("{namespace}:{name}"))
                        .map_err(|e| e.to_string())?;
                    if !structure_sets.contains(&id) {
                        structure_sets.push(id);
                    }
                }
            }
        }
        structure_sets.sort_by(|a, b| (a.path(), a.namespace()).cmp(&(b.path(), b.namespace())));
        let mut sources = BTreeMap::from([("vanilla cache".into(), base_identity.clone())]);
        for layer in &self.worldgen_layers {
            if layer.id != "vanilla" && (!layer.resources.is_empty() || !layer.filters.is_empty()) {
                let mut hash = Sha256::new();
                for (path, bytes) in &layer.resources {
                    hash.update(path.as_bytes());
                    hash.update((bytes.len() as u64).to_be_bytes());
                    hash.update(bytes);
                }
                for filter in &layer.filters {
                    for pattern in [&filter.namespace, &filter.path] {
                        hash.update(pattern.as_ref().map_or("", |p| p.as_str()).as_bytes());
                        hash.update([0]);
                    }
                }
                sources.insert(layer.id.clone(), format!("{:x}", hash.finalize()));
            }
        }
        let mut structures = Vec::new();
        let mut catalog = BTreeSet::new();
        let mut structure_owners = BTreeMap::new();
        for path in resources.keys() {
            if let Some(path) = path.strip_prefix("data/")
                && let Some((namespace, tail)) = path.split_once('/')
                && let Some(name) = tail
                    .strip_prefix("worldgen/structure/")
                    .and_then(|p| p.strip_suffix(".json"))
            {
                let id = mc_data::Identifier::parse(format!("{namespace}:{name}"))
                    .map_err(|e| e.to_string())?;
                catalog.insert(id.clone());
                if validation.contains(&path.to_owned()) {
                    structures.push(id);
                    if let Some(layer) = self
                        .worldgen_layers
                        .iter()
                        .rev()
                        .find(|l| l.resources.contains_key(path))
                    {
                        structure_owners.insert(format!("{namespace}:{name}"), layer.id.clone());
                    }
                }
            }
        }
        self.worldgen_structures = structures.into();
        if base.is_none() && self.worldgen_layers.iter().any(|l| !l.resources.is_empty()) {
            return Err("Datapack worldgen requires the vanilla gameplay cache".into());
        }
        let stage = if base.is_some() && identity != base_identity {
            Some(Stage::create(world, &resources)?)
        } else {
            None
        };
        Ok(Inputs {
            stage,
            base_root: base.map(Path::to_path_buf),
            identity,
            sources,
            biome_tags,
            unrouted_structure_sets: structure_sets,
            structure_sets: BTreeMap::new(),
            validation,
            catalog,
            structure_owners,
        })
    }
}

fn fingerprint(resources: &Resources) -> String {
    let mut hash = Sha256::new();
    hash.update(b"Krasnokamen selected worldgen inputs v1\0");
    for (path, bytes) in resources {
        // Gameplay reloads do not change immutable terrain. Registry reports do.
        if !path.starts_with("reports/") && !path.strip_prefix("data/").is_some_and(is_resource) {
            continue;
        }
        hash.update((path.len() as u64).to_be_bytes());
        hash.update(path.as_bytes());
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}

pub(crate) struct Inputs {
    stage: Option<Stage>,
    base_root: Option<PathBuf>,
    pub(crate) identity: String,
    pub(crate) biome_tags: Arc<BiomeTags>,
    unrouted_structure_sets: Vec<mc_data::Identifier>,
    pub(crate) structure_sets: BTreeMap<crate::dimensions::Dimension, Vec<mc_data::Identifier>>,
    sources: BTreeMap<String, String>,
    validation: Vec<String>,
    catalog: BTreeSet<mc_data::Identifier>,
    structure_owners: BTreeMap<String, String>,
}
impl Inputs {
    pub(crate) fn root(&self) -> Option<&Path> {
        self.stage
            .as_ref()
            .map(|s| s.0.as_path())
            .or(self.base_root.as_deref())
    }

    pub(crate) fn validate(
        &mut self,
        registry: &crate::registry::CoreRegistry,
    ) -> Result<(), String> {
        let selected_sets = std::mem::take(&mut self.unrouted_structure_sets);
        let Some(root) = self.root() else {
            return Ok(());
        };
        let loader =
            mc_data::village_data::VillageDataLoader::new(root.join("data/minecraft/worldgen"));
        let mut structure_dimensions = BTreeMap::new();
        for path in &self.validation {
            let (namespace, tail) = path.split_once('/').ok_or("Missing resource namespace")?;
            let mut validate = || -> Result<(), String> {
                if tail.starts_with("structure/") {
                    mc_worldgen::structures::StructureTemplate::from_nbt_file(
                        root.join("data").join(path),
                        &registry.blocks,
                    )
                    .map_err(|e| e.to_string())?;
                } else if let Some(tail) = tail.strip_prefix("worldgen/") {
                    let (kind, name) = tail
                        .split_once('/')
                        .ok_or("Missing worldgen resource kind")?;
                    let name = name
                        .strip_suffix(".json")
                        .ok_or("Worldgen resource must be JSON")?;
                    let id = mc_data::Identifier::parse(format!("{namespace}:{name}"))
                        .map_err(|e| e.to_string())?;
                    match kind {
                        "structure_set" => {
                            loader.load_structure_set(&id).map_err(|e| e.to_string())?;
                        }
                        "structure" => {
                            let structure =
                                loader.load_structure(&id).map_err(|e| e.to_string())?;
                            structure_dimensions
                                .insert(id, self.biome_tags.structure_dimension(&structure)?);
                        }
                        "template_pool" => {
                            loader.load_template_pool(&id).map_err(|e| e.to_string())?;
                        }
                        "processor_list" => {
                            loader.load_processor_list(&id).map_err(|e| e.to_string())?;
                        }
                        _ => return Err(format!("Unsupported worldgen resource {path}")),
                    }
                }
                Ok(())
            };
            validate().map_err(|e| format!("Selected datapack resource {path}: {e}"))?;
        }
        if !selected_sets.is_empty() {
            self.structure_sets = dimensions::route_sets(
                root,
                &self.biome_tags,
                selected_sets,
                structure_dimensions,
            )?;
        }
        Ok(())
    }
    /// Diagnostic identity: startup changes are admitted, but invalidate the terrain epoch.
    pub(crate) fn stage_identity(
        &self,
        globals: &mut BTreeMap<String, mc_world::native::NativeDomainRecord>,
    ) -> Result<Adoption, crate::CoreError> {
        let saved = crate::terrain::read_data_field(globals, IDENTITY)
            .map_err(crate::CoreError::Storage)?
            .map(|saved| {
                serde_json::from_str::<Identity>(&saved).map_err(|e| {
                    crate::CoreError::Storage(format!("datapack worldgen identity: {e}"))
                })
            })
            .transpose()?;
        let changed = saved
            .as_ref()
            .is_none_or(|saved| saved.digest != self.identity);
        let mut owners = saved
            .as_ref()
            .map(|s| s.structure_owners.clone())
            .unwrap_or_default();
        owners.extend(self.structure_owners.clone());
        if let Some(saved) = &saved
            && saved.digest != self.identity
        {
            let changed_sources: BTreeSet<_> = saved
                .sources
                .keys()
                .chain(self.sources.keys())
                .filter(|key| saved.sources.get(*key) != self.sources.get(*key))
                .cloned()
                .collect();
            tracing::warn!(old = %saved.digest, new = %self.identity, changed = ?changed_sources,
                "datapack worldgen inputs changed at startup; existing chunks retained, new chunks use selected inputs");
            crate::terrain::stage_epoch(globals, &crate::terrain::mint_epoch())?;
        }
        let value = serde_json::to_string(&Identity {
            digest: self.identity.clone(),
            sources: self.sources.clone(),
            structure_owners: owners.clone(),
        })
        .map_err(|e| crate::CoreError::Storage(e.to_string()))?;
        if value.len() > 32_767 {
            return Err(crate::CoreError::Storage(
                "datapack worldgen identity exceeds native metadata string limit".into(),
            ));
        }
        crate::terrain::stage_data_field(globals, IDENTITY, &value)?;
        Ok(Adoption {
            changed,
            catalog: Arc::new(StructureCatalog {
                durable_baselines: self.stage.is_some(),
                active: self.catalog.clone(),
                owners,
                warned: parking_lot::Mutex::new(BTreeSet::new()),
            }),
        })
    }
}

struct Stage(PathBuf);
impl Stage {
    fn cleanup(world: &Path) -> Result<(), String> {
        let root = world.join(STAGE);
        let metadata = match std::fs::symlink_metadata(&root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("{}: {error}", root.display())),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(format!(
                "Refusing unowned worldgen staging path {}",
                root.display()
            ));
        }
        let marker = world.join(OWNER);
        let owned = std::fs::symlink_metadata(&marker)
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink() && m.len() == 0);
        if !owned {
            return std::fs::remove_dir(&root).map_err(|e| {
                format!(
                    "Refusing unowned worldgen staging path {}: {e}",
                    root.display()
                )
            });
        }
        for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_name() != "data" && entry.file_name() != "reports" {
                return Err(format!(
                    "Refusing unexpected worldgen staging entry {}",
                    entry.path().display()
                ));
            }
        }
        std::fs::remove_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))
    }

    fn create(world: &Path, resources: &Resources) -> Result<Self, String> {
        Self::cleanup(world)?;
        let root = world.join(STAGE);
        // The sibling marker survives partial cleanup. Persist its ownership
        // before creating any disposable files, so every crash point is recoverable.
        let marker = world.join(OWNER);
        match std::fs::symlink_metadata(&marker) {
            Ok(m) if !m.is_file() || m.file_type().is_symlink() || m.len() != 0 => {
                return Err(format!(
                    "Refusing unowned worldgen staging marker {}",
                    marker.display()
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", marker.display())),
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&marker)
            .and_then(|file| file.sync_all())
            .and_then(|()| mc_world::atomic_file::sync_parent_dir(&marker))
            .map_err(|e| format!("{}: {e}", marker.display()))?;
        std::fs::create_dir(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        let stage = Self(root);
        for (path, bytes) in resources {
            let path = stage.0.join(path);
            std::fs::create_dir_all(path.parent().ok_or("Worldgen resource missing parent")?)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Ok(stage)
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            tracing::warn!(path = %self.0.display(), %error, "worldgen staging cleanup failed");
        }
    }
}

/// One semantic authority for the effective namespaced biome-tag image.
#[derive(Default)]
pub(crate) struct BiomeTags(BTreeMap<mc_data::Identifier, BTreeSet<mc_data::Identifier>>);
impl BiomeTags {
    fn load(resources: &Resources) -> Result<Self, String> {
        let mut raw = BTreeMap::new();
        let mut biomes = BTreeSet::new();
        for path in resources.keys() {
            if let Some(name) = path
                .strip_prefix("reports/registry_network_nbt/minecraft/worldgen/biome/")
                .and_then(|p| p.strip_suffix(".nbt"))
            {
                let (namespace, entry) = name
                    .split_once('/')
                    .ok_or("Biome wire payload missing entry namespace")?;
                biomes.insert(format!("{namespace}:{entry}"));
            }
            if let Some(path) = path.strip_prefix("data/")
                && let Some((namespace, tail)) = path.split_once('/')
                && let Some(name) = tail
                    .strip_prefix("worldgen/biome/")
                    .and_then(|p| p.strip_suffix(".json"))
            {
                biomes.insert(format!("{namespace}:{name}"));
            }
        }
        for (path, bytes) in resources {
            if let Some(path) = path.strip_prefix("data/")
                && let Some((namespace, tail)) = path.split_once('/')
                && let Some(name) = tail
                    .strip_prefix("tags/worldgen/biome/")
                    .and_then(|p| p.strip_suffix(".json"))
            {
                let value = serde_json::from_slice(bytes).map_err(|e| format!("{path}: {e}"))?;
                raw.insert(
                    format!("{namespace}:{name}"),
                    tag_values(&value).map_err(|e| format!("{path}: {e}"))?,
                );
            }
        }
        fn expand(
            id: &str,
            raw: &BTreeMap<String, Vec<(String, bool)>>,
            biomes: &BTreeSet<String>,
            visiting: &mut BTreeSet<String>,
            work: &mut usize,
            out: &mut BTreeSet<mc_data::Identifier>,
        ) -> Result<(), String> {
            *work += 1;
            if *work > super::MAX_RESOURCES || visiting.len() >= 32 || !visiting.insert(id.into()) {
                return Err(format!("Cyclic or excessive biome tag expansion: {id}"));
            }
            for (value, required) in &raw[id] {
                if let Some(nested) = value.strip_prefix('#') {
                    if raw.contains_key(nested) {
                        expand(nested, raw, biomes, visiting, work, out)?;
                    } else if *required {
                        return Err(format!("Biome tag {id}: missing tag {nested}"));
                    }
                } else if biomes.contains(value) {
                    out.insert(
                        mc_data::Identifier::parse(value.clone()).map_err(|e| e.to_string())?,
                    );
                } else if *required {
                    return Err(format!("Biome tag {id}: unknown biome {value}"));
                }
            }
            visiting.remove(id);
            Ok(())
        }
        let mut result = BTreeMap::new();
        for id in raw.keys() {
            let mut values = BTreeSet::new();
            expand(id, &raw, &biomes, &mut BTreeSet::new(), &mut 0, &mut values)?;
            result.insert(
                mc_data::Identifier::parse(id.clone()).map_err(|e| e.to_string())?,
                values,
            );
        }
        Ok(Self(result))
    }
}
impl mc_worldgen::vanilla_features::BiomeTagIndex for BiomeTags {
    fn biome_in_tag(&self, tag: &mc_data::Identifier, biome: &mc_data::Identifier) -> bool {
        self.0.get(tag).is_some_and(|values| values.contains(biome))
    }
}

/// Resolve the saved checkpoint + WAL choice before any generator is constructed.
pub(crate) fn selection(
    replays: &BTreeMap<crate::dimensions::Dimension, crate::journal::Replay>,
) -> Result<Selection, crate::CoreError> {
    let selected = replays
        .get(&crate::dimensions::Dimension::Overworld)
        .and_then(|r| r.datapack_selection.clone())
        .unwrap_or_default();
    selected.validate().map_err(crate::CoreError::Storage)?;
    Ok(selected)
}

#[cfg(test)]
#[path = "datapacks_worldgen_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "datapacks_worldgen_nether_tests.rs"]
mod nether_tests;
