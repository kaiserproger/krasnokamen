//! D65's inbound-path ratchet, separate from the top-level SCC ceiling.
//! Resolves qualified paths through module children, imports and explicit
//! re-exports. This measures named paths, not method resolution.
use super::Finding;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

// D65 before D63: 116 -> 113. Main-approved batch B adds seven starter-chest
// host references: rebased S1 is 123; S3 cuts four (including offline grants).
// The six remaining QW journal/dimension hooks move in D65 S2/S6; lower then.
// This rebaselines named paths only; the independent SCC ceiling stays 186.
pub(super) const CEILING: usize = 119;

#[derive(Debug)]
pub(super) struct Reference {
    pub(super) file: PathBuf,
    pub(super) caller: String,
    pub(super) target: String,
    pub(super) allowance: Option<&'static str>,
}

fn domain(path: &[String]) -> bool {
    match path.first().map(String::as_str) {
        Some("settlements") => path.get(1).is_none_or(|part| part != "inventory"),
        Some("plugin_structures" | "plugin_settlements") => true,
        Some("command_execution") => path.get(1).is_some_and(|part| part == "settlement"),
        Some("villager") => path.get(1).is_some_and(|part| {
            matches!(
                part.as_str(),
                "brain"
                    | "building"
                    | "doors"
                    | "harvest"
                    | "lumber"
                    | "mining"
                    | "return_home"
                    | "shepherd"
                    | "smelting"
                    | "soldier"
            )
        }),
        _ => false,
    }
}

#[derive(Default)]
pub(super) struct Index {
    children: BTreeMap<Vec<String>, BTreeSet<String>>,
    imports: BTreeMap<Vec<String>, BTreeMap<String, Vec<String>>>,
    pending: Vec<Pending>,
}

struct Pending {
    scope: Vec<String>,
    names: Vec<String>,
    file: PathBuf,
    field: Option<String>,
}

impl Index {
    pub(super) fn module(&mut self, scope: &[String], name: &str) {
        self.children
            .entry(scope.to_vec())
            .or_default()
            .insert(name.to_owned());
    }

    pub(super) fn import(&mut self, scope: &[String], names: &[String], alias: &str) {
        let mut path = names.to_vec();
        let alias = if alias == "self" {
            path.pop();
            let Some(name) = path.last() else { return };
            name.clone()
        } else {
            alias.to_owned()
        };
        self.imports
            .entry(scope.to_vec())
            .or_default()
            .insert(alias, path);
    }

    pub(super) fn record(
        &mut self,
        scope: &[String],
        names: &[String],
        file: &Path,
        field: Option<&str>,
    ) {
        if !domain(scope) {
            self.pending.push(Pending {
                scope: scope.to_vec(),
                names: names.to_vec(),
                file: file.to_owned(),
                field: field.map(str::to_owned),
            });
        }
    }

    fn absolute(&self, scope: &[String], names: &[String], depth: usize) -> Option<Vec<String>> {
        if depth > 32 {
            return None;
        } // a cyclic import is not a resolved path
        let mut path = scope.to_vec();
        let mut index = 0;
        match names.first()?.as_str() {
            "crate" => {
                path.clear();
                index = 1;
            }
            "self" => index = 1,
            "super" => {
                while names.get(index).is_some_and(|part| part == "super") {
                    path.pop()?;
                    index += 1;
                }
            }
            first
                if self
                    .children
                    .get(scope)
                    .is_some_and(|children| children.contains(first)) => {}
            first => {
                let imported = self.imports.get(scope)?.get(first)?;
                let mut path = self.absolute(scope, imported, depth + 1)?;
                path.extend_from_slice(&names[1..]);
                return self.canonical(path, depth + 1);
            }
        }
        path.extend_from_slice(&names[index..]);
        if path.last().is_some_and(|part| part == "self") {
            path.pop();
        }
        self.canonical(path, depth + 1)
    }

    fn canonical(&self, path: Vec<String>, depth: usize) -> Option<Vec<String>> {
        if depth > 32 {
            return None;
        }
        for index in 0..path.len() {
            let parent = &path[..index];
            if self
                .children
                .get(parent)
                .is_some_and(|children| children.contains(&path[index]))
            {
                continue;
            }
            if let Some(imported) = self
                .imports
                .get(parent)
                .and_then(|imports| imports.get(&path[index]))
            {
                let mut target = self.absolute(parent, imported, depth + 1)?;
                target.extend_from_slice(&path[index + 1..]);
                return self.canonical(target, depth + 1);
            }
        }
        Some(path)
    }

    pub(super) fn finish(self) -> Vec<Reference> {
        self.pending
            .iter()
            .filter_map(|pending| {
                let path = self.absolute(&pending.scope, &pending.names, 0)?;
                resolved(
                    &pending.scope,
                    &path,
                    &pending.file,
                    pending.field.as_deref(),
                )
            })
            .collect()
    }
}

#[cfg(test)]
fn resolve(
    scope: &[String],
    names: &[String],
    file: &Path,
    field: Option<&str>,
) -> Option<Reference> {
    let path = Index::default().absolute(scope, names, 0)?;
    resolved(scope, &path, file, field)
}

fn resolved(
    scope: &[String],
    path: &[String],
    file: &Path,
    field: Option<&str>,
) -> Option<Reference> {
    if domain(scope) || !domain(path) {
        return None;
    }
    let caller = scope.join("::");
    let target = path.join("::");
    // Allowances are typed and attached to the actual field, not a filename
    // wildcard: adding an unrelated call in that file still spends the ceiling.
    let allowance = match (caller.as_str(), field, target.as_str()) {
        ("tick", Some("settlements"), "settlements::registry::Registry") => {
            Some("C2: TickState.settlements")
        }
        ("journal", Some("image"), "settlements::registry::RecordImage") => {
            Some("C1: JournalEntry::Settlement.image")
        }
        ("journal::replay", Some("settlement_tail"), "settlements::persistence::tail::Tail") => {
            Some("C1: Replay settlement tail")
        }
        ("villager", Some("settlement"), "settlements::registry::SettlementId") => {
            Some("C3: VillagerState.settlement")
        }
        ("villager", Some("soldier"), "settlements::army::Soldier") => {
            Some("C3: VillagerState.soldier")
        }
        _ => None,
    };
    Some(Reference {
        file: file.to_owned(),
        caller,
        target,
        allowance,
    })
}

pub(super) fn check(src: &Path, references: &[Reference], findings: &mut Vec<Finding>) {
    println!(
        "  D65 inbound domain paths: {} (ceiling {CEILING}; independent of SCC)",
        references.len()
    );
    let mut by_caller = BTreeMap::<&str, usize>::new();
    for reference in references {
        *by_caller.entry(&reference.caller).or_default() += 1;
        if let Some(until) = reference.allowance {
            println!(
                "    typed allowance until {until}: {} -> {}",
                reference.caller, reference.target
            );
        }
    }
    for (caller, count) in by_caller {
        println!("    {caller}: {count}");
    }
    if references.len() > CEILING {
        findings.push(Finding { path:src.to_owned(), line:0,
            message:format!("D65 inbound domain paths {} exceed ceiling {CEILING}; cut the new coupling, do not raise the SCC ceiling",references.len()) });
        for reference in references {
            println!(
                "    {}: {} -> {}",
                reference
                    .file
                    .strip_prefix(src)
                    .unwrap_or(&reference.file)
                    .display(),
                reference.caller,
                reference.target
            );
        }
    } else if references.len() < CEILING {
        println!("    D65 inbound ceiling can drop to {}", references.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reference(scope: &str, path: &str, field: Option<&str>) -> Option<Reference> {
        let split = |s: &str| s.split("::").map(str::to_owned).collect::<Vec<_>>();
        resolve(&split(scope), &split(path), Path::new("fixture.rs"), field)
    }
    #[test]
    fn domain_boundary_counts_submodules_and_exact_typed_allowances() {
        assert!(reference("villager::persistence", "super::mining::Progress", None).is_some());
        assert!(reference("villager::brain", "crate::settlements::Registry", None).is_none());
        assert!(
            reference(
                "plugin_inventories::native",
                "crate::settlements::SettlementId",
                None
            )
            .is_some()
        );
        assert!(reference("tick", "crate::settlements::inventory::load", None).is_none());
        assert!(reference("tick", "crate::villager::trades::tick", None).is_none());
        let typed = reference(
            "tick",
            "crate::settlements::registry::Registry",
            Some("settlements"),
        )
        .unwrap();
        assert_eq!(typed.allowance, Some("C2: TickState.settlements"));
        assert!(
            reference(
                "tick",
                "crate::settlements::registry::Registry",
                Some("unrelated")
            )
            .unwrap()
            .allowance
            .is_none()
        );
    }
    #[test]
    fn domain_boundary_new_path_trips_its_own_ceiling() {
        let refs = (0..=CEILING)
            .map(|_| reference("tick", "crate::villager::brain::tick", None).unwrap())
            .collect::<Vec<_>>();
        let mut findings = Vec::new();
        check(Path::new("fixture"), &refs, &mut findings);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("exceed ceiling"));
    }
}
