use std::fs;
use std::path::PathBuf;

use super::{Ceilings, breakdown, check};
use crate::code_health::core_ast::{self, Counter};
use crate::code_health::module_graph::{decoupling_hints, largest_component, strongly_connected};

/// A throwaway workspace with `crates/mc-core/src/<path>` files.
struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let root =
            std::env::temp_dir().join(format!("xtask-ratchet-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (path, body) in files {
            let path = root.join("crates/mc-core/src").join(path);
            fs::create_dir_all(path.parent().expect("fixture file has a parent"))
                .expect("create fixture dir");
            fs::write(&path, body).expect("write fixture file");
        }
        Fixture(root)
    }

    fn facts(&self) -> core_ast::CoreFacts {
        core_ast::collect(&self.0.join("crates/mc-core/src")).expect("fixture parses")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn tarjan_finds_every_component() {
    // 0 -> 1 -> 2 -> 0 is a cycle, 2 -> 3, 3 <-> 4 is a second one, 5 is alone.
    let edges = vec![vec![1], vec![2], vec![0, 3], vec![4], vec![3], vec![]];
    let mut components = strongly_connected(&edges);
    components.sort();
    assert_eq!(components, vec![vec![0, 1, 2], vec![3, 4], vec![5]]);
    assert_eq!(largest_component(&edges), vec![0, 1, 2]);
    assert_eq!(largest_component(&[vec![], vec![]]), vec![0]);
}

#[test]
fn decoupling_hints_rank_modules_by_modules_leaving_per_cut_edge() {
    // Ring 0 -> 1 -> 2 -> 0 plus 3 hanging on it (2 -> 3 -> 0). Cutting the
    // one edge 0 -> 1 breaks every cycle; moving 3 out frees only itself.
    let edges = vec![vec![1], vec![2], vec![0, 3], vec![0]];
    let cycle = largest_component(&edges);
    assert_eq!(cycle, vec![0, 1, 2, 3]);
    let hints = decoupling_hints(&edges, &cycle);
    assert_eq!(hints[0].node, 0);
    assert_eq!(hints[0].cut, vec![(0, 1)]);
    assert_eq!(hints[0].leaving, 4);
    let last = hints.last().expect("one hint per module");
    assert_eq!((last.node, last.leaving), (3, 1));
}

#[test]
fn test_code_is_not_production() {
    let a = "\
pub fn f() -> &'static str {
    crate::b::G
}
#[cfg(any(test, feature = \"bench\"))]
pub fn bench() {
    panic!(\"bench builds keep this\");
}
#[cfg(test)]
mod tests {
    fn t() {
        let _ = \"minecraft:test_only\";
        panic!(\"test\");
    }
}
#[cfg(all(test, feature = \"x\"))]
fn only_tests() {
    panic!(\"test\");
}
#[test]
fn bare_test() {
    panic!(\"test\");
}
pub struct S;
impl S {
    #[cfg(test)]
    fn helper() {
        panic!(\"test\");
    }
}
";
    let b = "\
pub const G: &str = \"minecraft:stone\";
#[cfg(test)]
fn uses_a() {
    let _ = crate::a::f();
}
";
    let test_file = "pub fn x() { panic!(\"x\"); let _ = \"minecraft:a\"; crate::a::f(); }\n";
    let fixture = Fixture::new(
        "tests",
        &[
            (
                "lib.rs",
                "mod a;\nmod b;\n#[cfg(test)]\nmod a_tests;\nmod helpers_tests;\n",
            ),
            ("a.rs", a),
            ("b.rs", b),
            ("a_tests.rs", test_file),
            ("helpers_tests.rs", test_file),
        ],
    );
    let facts = fixture.facts();
    let names: Vec<&str> = facts.modules.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(facts.total(Counter::VanillaIdLiteral), 1);
    assert_eq!(
        facts.total(Counter::Panic),
        1,
        "only the cfg(any(test, ..)) fn"
    );
    assert_eq!(facts.edges(), vec![vec![1], vec![]], "b -> a is test-only");
    assert_eq!((facts.modules[0].lines, facts.modules[1].lines), (10, 1));
    assert_eq!(facts.root.lines, 2);
}

#[test]
fn comments_and_attributes_are_not_counted() {
    let a = "\
// let x = \"minecraft:line_comment\";
/// \"minecraft:doc_comment\"
/* \"minecraft:block_comment\" */
#[doc = \"minecraft:doc_attribute\"]
pub fn f() -> &'static str {
    \"minecraft:real\" // \"minecraft:trailing\"
}
";
    let fixture = Fixture::new("comments", &[("lib.rs", "mod a;\n"), ("a.rs", a)]);
    assert_eq!(fixture.facts().total(Counter::VanillaIdLiteral), 1);
}

#[test]
fn macro_bodies_are_counted() {
    let a = "\
macro_rules! template {
    () => {
        \"minecraft:in_template\"
    };
}
pub fn g(id: &str, block: &Block, state: &mut State) -> usize {
    let s = format!(\"{}\", \"minecraft:fmt\");
    let v = vec![String::from(\"minecraft:vec\"); 2];
    assert!(matches!(id, \"minecraft:a\" | \"minecraft:b\"), \"{}\", \"minecraft:c\");
    log!(target: \"minecraft:t\", x = %id);
    assert!(block.path() == \"x\" && \"y\" != block.path());
    log!(target: \"t\", state.merge_block_writes.push((1, 2)), block.path() != \"z\", panic!(\"t\"));
    state.merge_block_writes.push((3, 4));
    let _ = Some(1).unwrap_or_else(|| panic!(\"in closure\"));
    s.len() + v.len()
}
";
    let fixture = Fixture::new("macros", &[("lib.rs", "mod a;\n"), ("a.rs", a)]);
    let facts = fixture.facts();
    // template, fmt, vec, a, b, c (a macro inside a macro), t.
    assert_eq!(facts.total(Counter::VanillaIdLiteral), 7);
    assert_eq!(facts.total(Counter::PathEquality), 3);
    assert_eq!(facts.total(Counter::MergeBlockWritePush), 2);
    assert_eq!(facts.total(Counter::Panic), 2);
}

#[test]
fn module_references_resolve_through_super_path_attributes_and_root_reexports() {
    let fixture = Fixture::new(
        "resolve",
        &[
            (
                "lib.rs",
                "mod a;\nmod b;\nmod c;\npub use b::Thing as Alias;\n",
            ),
            (
                "a.rs",
                "mod inner;\n#[path = \"a_extra.rs\"]\nmod extra;\npub fn f() {}\n",
            ),
            (
                "a/inner.rs",
                "use super::super::b::Thing;\npub fn g(_: Thing) {}\n",
            ),
            ("a_extra.rs", "pub fn h() { crate::c::k(); }\n"),
            (
                "b.rs",
                "pub struct Thing;\npub fn uses() { super::c::k(); }\n",
            ),
            (
                "c.rs",
                "pub fn k() { let _: Option<crate::Alias> = None; self::local(); }\nfn local() {}\n",
            ),
        ],
    );
    let facts = fixture.facts();
    assert_eq!(facts.edges(), vec![vec![1, 2], vec![2], vec![1]]);
    assert_eq!(largest_component(&facts.edges()), vec![1, 2]);
    assert_eq!(facts.modules[0].lines, 4 + 2 + 1, "a owns its #[path] file");
}

#[test]
fn a_ratchet_above_its_ceiling_fails() {
    let fixture = Fixture::new(
        "gate",
        &[
            ("lib.rs", "mod a;\nmod b;\n"),
            (
                "a.rs",
                "pub fn f() -> &'static str {\n    crate::b::g();\n    \"minecraft:stone\"\n}\n",
            ),
            (
                "b.rs",
                "pub fn g() {\n    if false {\n        crate::a::f();\n    }\n    panic!(\"x\");\n}\n",
            ),
        ],
    );
    let facts = fixture.facts();
    assert_eq!(breakdown(&facts, Counter::VanillaIdLiteral), "a 1");
    let measured = Ceilings {
        cycle_modules: 2,
        vanilla_id_literals: 1,
        path_equality: 0,
        panics: 1,
        merge_block_write_pushes: 0,
    };
    let mut findings = Vec::new();
    check(&fixture.0, &measured, &mut findings);
    assert!(findings.is_empty(), "{findings:?}");

    // Negative control: each ceiling one below its measurement fails alone.
    let lowered = [
        (
            "largest module cycle (modules)",
            Ceilings {
                cycle_modules: 1,
                ..measured
            },
        ),
        (
            "string literals",
            Ceilings {
                vanilla_id_literals: 0,
                ..measured
            },
        ),
        (
            "`panic!` invocations",
            Ceilings {
                panics: 0,
                ..measured
            },
        ),
    ];
    for (name, ceilings) in lowered {
        let mut findings = Vec::new();
        check(&fixture.0, &ceilings, &mut findings);
        assert_eq!(findings.len(), 1, "{name}: {findings:?}");
        assert!(
            findings[0].message.contains(name) && findings[0].message.contains("above its ceiling"),
            "{name}: {}",
            findings[0].message
        );
    }
}

#[test]
fn an_unparsable_file_fails_closed() {
    let fixture = Fixture::new(
        "broken",
        &[("lib.rs", "mod a;\nmod missing;\n"), ("a.rs", "pub fn (\n")],
    );
    let mut findings = Vec::new();
    check(&fixture.0, &super::CEILINGS, &mut findings);
    assert_eq!(findings.len(), 1);
    assert!(
        findings[0].message.contains("parse failed"),
        "{}",
        findings[0].message
    );
    assert!(findings[0].message.contains("no file for `mod missing;`"));
}
