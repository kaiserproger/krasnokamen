//! Directory datapacks. Compilation is startup/worker work, never tick filesystem I/O.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::boundary::{ChangeSet, ConnectionId, EntityId, PlayerChange};
use crate::tick::TickState;

const COMMAND_BUDGET: usize = crate::command_context::LIMIT;
const MAX_RESOURCES: usize = 16384;
const MAX_BYTES: usize = 16 * 1024 * 1024;
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

impl Selection {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.enabled.len().saturating_add(self.disabled.len()) > MAX_RESOURCES {
            return Err("Datapack selection limit exceeded".into());
        }
        let mut seen = BTreeSet::new();
        for id in self.enabled.iter().chain(&self.disabled) {
            if id.is_empty() || id.len() > 1024 || !seen.insert(id) {
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
    /// Pack descriptions from `pack.mcmeta` (`file/<name>` and `vanilla`),
    /// preserved so listing can present them through chat hover components.
    pub descriptions: BTreeMap<String, mc_nbt::Tag>,
    pub(crate) functions: BTreeMap<String, Arc<[String]>>,
    pub(crate) tags: BTreeMap<String, Arc<[String]>>,
    pub(crate) predicates: BTreeMap<String, serde_json::Value>,
    resources: BTreeMap<String, String>,
}
impl Default for PackSet {
    fn default() -> Self {
        Self {
            selection: Selection::default(),
            descriptions: BTreeMap::from([(
                "vanilla".into(),
                mc_nbt::Tag::String("Vanilla built-in content".into()),
            )]),
            functions: BTreeMap::new(),
            tags: BTreeMap::new(),
            predicates: BTreeMap::new(),
            resources: BTreeMap::new(),
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

fn read_text(path: &Path, remaining: &mut usize) -> Result<String, String> {
    use std::io::Read;
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
    let mut text = String::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take((*remaining + 1) as u64)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    *remaining = remaining
        .checked_sub(text.len())
        .ok_or("Datapack byte limit exceeded")?;
    Ok(text)
}

struct DiscoveredPack {
    path: PathBuf,
    description: mc_nbt::Tag,
    features: Vec<String>,
}

fn discover(
    root: &Path,
    remaining: &mut usize,
) -> Result<BTreeMap<String, DiscoveredPack>, String> {
    let mut packs = BTreeMap::new();
    if !root.exists() {
        return Ok(packs);
    }
    if std::fs::symlink_metadata(root)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Datapack root cannot be a symlink".into());
    }
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        if packs.len() >= MAX_RESOURCES - 1 {
            return Err("Datapack resource limit exceeded".into());
        }
        let pack = entry.map_err(|e| e.to_string())?;
        if !pack.file_type().map_err(|e| e.to_string())?.is_dir() {
            return Err(format!(
                "Only directory datapacks are supported: {}",
                pack.path().display()
            ));
        }
        let id = format!(
            "file/{}",
            pack.file_name()
                .into_string()
                .map_err(|_| "Non-UTF8 datapack name")?
        );
        let raw = read_text(&pack.path().join("pack.mcmeta"), remaining)?;
        let metadata: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("{id}: {e}"))?;
        let pack_meta = metadata
            .get("pack")
            .filter(|value| value.is_object())
            .ok_or_else(|| format!("{id}: pack.mcmeta requires a pack object"))?;
        let description = pack_meta
            .get("description")
            .map(|value| {
                crate::command_text::parse_component(&value.to_string())
                    .map_err(|error| format!("{id}: {}", error.message()))
            })
            .transpose()?
            .unwrap_or_else(|| mc_nbt::Tag::String(String::new()));
        let features = metadata
            .get("features")
            .map(|features| {
                features
                    .get("enabled")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| format!("{id}: invalid features.enabled"))?
                    .iter()
                    .map(|feature| {
                        feature
                            .as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| format!("{id}: feature identifier must be a string"))
                    })
                    .collect::<Result<Vec<_>, String>>()
            })
            .transpose()?
            .unwrap_or_default();
        packs.insert(
            id,
            DiscoveredPack {
                path: pack.path(),
                description,
                features,
            },
        );
    }
    Ok(packs)
}

impl PackSet {
    pub(crate) fn load(root: &Path, mut selection: Selection) -> Result<Self, String> {
        selection.validate()?;
        let mut remaining = MAX_BYTES;
        let packs = discover(root, &mut remaining)?;
        let mut descriptions = Self::default().descriptions;
        for (id, pack) in &packs {
            if !selection.disabled.contains(id) {
                for feature in &pack.features {
                    if !ENABLED_FEATURES.contains(&feature.as_str()) {
                        return Err(format!("{id}: world-disabled feature {feature}"));
                    }
                }
            }
            descriptions.insert(id.clone(), pack.description.clone());
        }
        selection
            .enabled
            .retain(|id| id == "vanilla" || packs.contains_key(id));
        selection.disabled.retain(|id| packs.contains_key(id));
        for id in packs.keys() {
            if !selection.enabled.contains(id) && !selection.disabled.contains(id) {
                selection.enabled.push(id.clone());
            }
        }
        selection.validate()?;
        let mut result = Self {
            selection,
            descriptions,
            ..Self::default()
        };
        let mut visited = 0;
        let mut raw_tags = BTreeMap::<String, Vec<(String, bool)>>::new();
        for id in &result.selection.enabled {
            if id == "vanilla" {
                continue;
            }
            let data = packs[id].path.join("data");
            let mut paths = Vec::new();
            files(&data, &mut paths, 0, &mut visited)?;
            for path in paths {
                let relative = mc_data::resource_path::slash_relative(&data, &path)
                    .ok_or("Non-UTF8 datapack path")?;
                let (namespace, tail) = relative
                    .split_once('/')
                    .ok_or("Resource must be namespaced")?;
                let (kind, resource) = tail.split_once('/').ok_or("Resource must have a type")?;
                if matches!(kind, "worldgen" | "dimension" | "dimension_type") {
                    return Err(format!("Worldgen datapacks are not supported: {relative}"));
                }
                let text = read_text(&path, &mut remaining)?;
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
                    if lines.iter().any(|line| {
                        line.len() > 32767 || line.starts_with('/') || line.starts_with('$')
                    }) {
                        return Err(format!(
                            "Unsupported function line in {id} (macros and leading slashes are unsupported)"
                        ));
                    }
                    result.functions.insert(id, lines.into());
                } else if kind == "tags"
                    && resource.starts_with("function/")
                    && resource.ends_with(".json")
                {
                    let id = identifier(&format!(
                        "{namespace}:{}",
                        resource
                            .strip_prefix("function/")
                            .expect("function tag directory checked")
                            .strip_suffix(".json")
                            .expect("tag extension checked")
                    ))?;
                    let tag: serde_json::Value =
                        serde_json::from_str(&text).map_err(|e| e.to_string())?;
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
                } else if kind == "predicate" && resource.ends_with(".json") {
                    let id = identifier(&format!(
                        "{namespace}:{}",
                        resource.strip_suffix(".json").expect("predicate extension")
                    ))?;
                    let value: serde_json::Value =
                        serde_json::from_str(&text).map_err(|error| format!("{id}: {error}"))?;
                    let conditions = if value.is_array() {
                        value
                    } else {
                        serde_json::Value::Array(vec![value])
                    };
                    let evaluation = mc_data::advancements::evaluate_conditions(
                        &conditions,
                        &serde_json::Value::Null,
                    );
                    if !evaluation.unsupported.is_empty() {
                        return Err(format!(
                            "Unsupported predicate {id}: {:?}",
                            evaluation.unsupported
                        ));
                    }
                    result.predicates.insert(id, conditions);
                } else if matches!(kind, "recipe" | "loot_table") && resource.ends_with(".json") {
                    result.resources.insert(relative, text);
                } else {
                    return Err(format!("Unsupported datapack resource: {relative}"));
                }
            }
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

    pub(crate) fn prepare_content(
        &self,
        base: &Arc<crate::registry::CoreRegistry>,
    ) -> Result<Arc<crate::registry::CoreRegistry>, String> {
        if self.resources.is_empty() {
            return Ok(Arc::clone(base));
        }
        let mut registry = base.prepare_content();
        self.install_content(&mut registry)?;
        Ok(Arc::new(registry))
    }

    pub(crate) fn install_content(
        &self,
        registry: &mut crate::registry::CoreRegistry,
    ) -> Result<(), String> {
        for (path, text) in &self.resources {
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
                if let Some(existing) = registry.recipes.iter_mut().find(|old| old.id == recipe.id)
                {
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
                    "Unsupported datapack loot domain: {id}; currently block loot tables only"
                ));
            }
        }
        registry.refresh_iron_smelting();
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct Runtime {
    packs: Arc<PackSet>,
    discovered: Option<Arc<BTreeMap<String, mc_nbt::Tag>>>,
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
    pub(crate) origin: Option<[f64; 3]>,
}
struct Reload {
    receiver: std::sync::mpsc::Receiver<Result<WorkerResult, String>>,
    connection: ConnectionId,
    /// Whether this worker run backs an enable/disable activation (report
    /// loaded-pack counts) or a plain `/reload` (report reloaded functions).
    activation: bool,
    completion: Option<crate::command_context::CompletionId>,
}
enum WorkerResult {
    Reload {
        packs: PackSet,
        registry: Option<Arc<crate::registry::CoreRegistry>>,
    },
    DebugReport(crate::command_debug::Report),
    List {
        descriptions: BTreeMap<String, mc_nbt::Tag>,
        filter: Option<String>,
    },
    Created {
        descriptions: BTreeMap<String, mc_nbt::Tag>,
        name: String,
    },
}

struct Frame {
    lines: Arc<[String]>,
    next: usize,
    started: bool,
    trace_name: Option<String>,
    context: crate::command_context::Context,
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
        self.discovered = None;
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
    fn catalog(&self) -> &BTreeMap<String, mc_nbt::Tag> {
        self.discovered
            .as_deref()
            .unwrap_or(&self.packs.descriptions)
    }
    /// Queue a worker-thread activation for an enable/disable selection:
    /// the worker recompiles and classifies the outcome on completion while
    /// the old packs and selection stay authoritative. Tick-thread callers
    /// never touch the filesystem.
    pub(crate) fn request_activation(
        &mut self,
        selection: Selection,
        connection: ConnectionId,
        completion: crate::command_context::CompletionId,
    ) -> Result<(), String> {
        selection.validate()?;
        self.request_reload(selection, connection, true, Some(completion))
    }
    fn request_reload(
        &mut self,
        selection: Selection,
        connection: ConnectionId,
        activation: bool,
        completion: Option<crate::command_context::CompletionId>,
    ) -> Result<(), String> {
        let base = Arc::clone(
            self.base_registry
                .as_ref()
                .ok_or("Datapack runtime is not installed")?,
        );
        let current = Arc::clone(&self.packs);
        self.request_work(connection, activation, completion, move |root| {
            let packs = PackSet::load(root, selection)?;
            let registry = if packs.resources == current.resources {
                None
            } else {
                Some(packs.prepare_content(&base)?)
            };
            Ok(WorkerResult::Reload { packs, registry })
        })
    }

    pub(crate) fn request_debug_report(
        &mut self,
        connection: ConnectionId,
        completion: crate::command_context::CompletionId,
        report: crate::command_debug::Report,
    ) -> Result<(), String> {
        self.request_work(connection, false, Some(completion), move |root| {
            report.write(root)?;
            Ok(WorkerResult::DebugReport(report))
        })
    }

    fn request_work(
        &mut self,
        connection: ConnectionId,
        activation: bool,
        completion: Option<crate::command_context::CompletionId>,
        work: impl FnOnce(&Path) -> Result<WorkerResult, String> + Send + 'static,
    ) -> Result<(), String> {
        if self.reload.is_some() {
            return Err("Datapack reload already in progress".into());
        }
        let root = self
            .root
            .clone()
            .ok_or("No datapack directory configured")?;
        let (tx, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("datapack-reload".into())
            .spawn(move || {
                let _ = tx.send(work(&root));
            })
            .map_err(|error| format!("Starting datapack reload: {error}"))?;
        self.reload = Some(Reload {
            receiver,
            connection,
            activation,
            completion,
        });
        Ok(())
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
                lines: runtime.packs.functions[name].clone(),
                next: 0,
                started: false,
                trace_name: state.debug_tracing.then(|| name.clone()),
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
            lines,
            next: 0,
            started: false,
            trace_name: state.debug_tracing.then_some(id),
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
            if let Some(name) = &frame.trace_name {
                crate::command_debug::trace_enter(state, name, frame.lines.len());
            }
        }
        if frame.next == frame.lines.len() {
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
        crate::command_debug::trace_command(state, &lines[index]);
        state.datapacks.pending.push(frame);
        crate::command_context::submit(state, &lines[index], context, changes);
        if state.command_context.waiting() {
            state.command_context.suspend_functions(stop_depth);
            break;
        }
    }
    state.datapacks.running = running;
}

pub(crate) fn cancel_pending(state: &mut TickState) {
    state.datapacks.pending.clear();
    state.debug_tracing = false;
    state.debug_traces.clear();
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
                state.datapacks.request_reload(
                    state.datapacks.selection().clone(), connection, false, Some(completion),
                )?;
                Ok::<_, String>(completion)
            })();
            return match queued {
                Ok(completion) => crate::command_execution::CommandResult::Deferred(completion),
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

fn publish_content(
    state: &mut TickState,
    registry: Arc<crate::registry::CoreRegistry>,
    _changes: &mut ChangeSet,
) {
    fn install(world: &mut TickState, registry: &Arc<crate::registry::CoreRegistry>) {
        world.registry = Arc::clone(registry);
        let mut updates = Vec::with_capacity(world.players.len());
        for player in world.players.values_mut() {
            let changes = player.reload_recipes(&registry.item_rules());
            let book = crate::boundary::PlayerChange::RecipesUnlocked {
                registry: crate::boundary::RecipeSource(Arc::clone(registry)),
                recipes: player.unlocked_recipes.iter().cloned().collect(),
                replace: true,
            };
            updates.push((player.connection, book, changes));
        }
        for (connection, book, changes) in updates {
            world.push_player(connection, book);
            for change in changes {
                world.push_player(connection, change);
            }
        }
    }
    install(state, &registry);
    for world in state.other_dimensions.values_mut() {
        install(world, &registry);
    }
}

pub(crate) fn poll_worker(state: &mut TickState, changes: &mut ChangeSet) {
    if state.dimension_globals.active && !state.command_context.shared {
        return;
    }
    if let Some(reload) = &state.datapacks.reload {
        let connection = reload.connection;
        let activation = reload.activation;
        let completion = reload.completion;
        let event_context =
            completion.and_then(|id| crate::command_context::completion_context(state, id));
        let deliver = completion.is_none() || event_context.is_some();
        let result = reload.receiver.try_recv();
        let previous = state.command_context.current;
        state.command_context.current = event_context.or(previous);
        match result {
            Ok(result) => {
                state.datapacks.reload = None;
                let mut outcome = (false, 0);
                match result {
                    Ok(WorkerResult::Reload { packs, registry }) => {
                        if let Some(registry) = registry {
                            publish_content(state, registry, changes);
                        }
                        state.datapacks.selection_dirty |=
                            state.datapacks.packs.selection != packs.selection;
                        let count = packs.selection.enabled.len();
                        outcome = (true, count as i32);
                        state.datapacks.discovered = None;
                        state.datapacks.packs = Arc::new(packs);
                        state.datapacks.load_pending = true;
                        if deliver && crate::command_execution::accepts_success(state, connection) {
                            feedback(
                                state,
                                connection,
                                if activation {
                                    format!("Datapack selection applied ({count} enabled)")
                                } else {
                                    "Datapack content reloaded".into()
                                },
                            );
                        }
                    }
                    Ok(WorkerResult::List {
                        descriptions,
                        filter,
                    }) => {
                        state.datapacks.discovered = Some(Arc::new(descriptions));
                        let count = if deliver {
                            management::report_list(state, connection, filter.as_deref())
                        } else {
                            0
                        };
                        outcome = (true, count);
                    }
                    Ok(WorkerResult::Created { descriptions, name }) => {
                        state.datapacks.discovered = Some(Arc::new(descriptions));
                        if deliver {
                            crate::command_execution::success_feedback(
                                state,
                                connection,
                                "commands.datapack.create.success",
                                vec![mc_nbt::Tag::String(name)],
                            );
                        }
                        outcome = (true, 1);
                    }
                    Ok(WorkerResult::DebugReport(report)) => {
                        if deliver {
                            if report.success {
                                crate::command_execution::success_feedback(
                                    state,
                                    connection,
                                    report.feedback,
                                    report.arguments,
                                );
                            } else {
                                crate::command_execution::feedback(
                                    state,
                                    connection,
                                    report.feedback,
                                    report.arguments,
                                );
                            }
                        }
                        outcome = (report.success, report.result);
                    }
                    Err(error) => {
                        if deliver {
                            feedback(
                                state,
                                connection,
                                format!("Datapack operation failed: {error}"),
                            );
                        }
                    }
                }
                if let Some(id) = completion {
                    crate::command_context::complete(state, id, outcome.0, outcome.1);
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                state.datapacks.reload = None;
                if let Some(id) = completion {
                    crate::command_context::complete(state, id, false, 0);
                }
                if deliver {
                    feedback(
                        state,
                        connection,
                        "Datapack worker disconnected; operation completion unavailable",
                    );
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        state.command_context.current = previous;
    }
}

pub(crate) fn tick(state: &mut TickState, changes: &mut ChangeSet) {
    poll_worker(state, changes);
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

#[cfg(test)]
#[path = "datapacks_tests.rs"]
mod tests;

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
        registry.refresh_iron_smelting();
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
