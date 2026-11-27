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
fn domain_boundary_resolves_children_aliases_and_explicit_reexports() {
    let fixture = Fixture::new(
        "domain-paths",
        &[(
            "lib.rs",
            r#"
        mod villager {
            pub mod brain { pub fn tick() {} }
            mod persistence { fn f() { super::brain::tick(); } }
            fn f() { brain::tick(); }
        }
        mod bridge { pub use crate::villager::brain as ai; }
        mod host {
            use crate::bridge::ai as resident_ai;
            fn f() { resident_ai::tick(); }
        }
    "#,
        )],
    );
    let facts = fixture.facts();
    assert_eq!(facts.domain_refs.len(), 5);
    assert!(
        facts
            .domain_refs
            .iter()
            .all(|reference| reference.target.starts_with("villager::brain"))
    );
}

#[test]
fn domain_boundary_ast_negative_control_is_independent_of_scc() {
    let ceiling = crate::code_health::domain_boundary::CEILING;
    let calls = "crate::villager::brain::tick();".repeat(ceiling);
    let host = format!(
        "fn f() {{ {calls} }}\n\
         #[cfg(test)] fn excluded() {{ crate::villager::brain::tick(); }}\n"
    );
    let fixture = Fixture::new(
        "domain-boundary",
        &[
            (
                "lib.rs",
                "mod host; mod villager { pub mod brain { pub fn tick() {} } } mod settlements { pub struct Registry; }",
            ),
            ("host.rs", &host),
        ],
    );
    let facts = fixture.facts();
    assert_eq!(facts.domain_refs.len(), ceiling);
    assert_eq!(largest_component(&facts.edges()).len(), 1);
    let mut findings = Vec::new();
    crate::code_health::domain_boundary::check(&fixture.0, &facts.domain_refs, &mut findings);
    assert!(findings.is_empty());
    // This additional import is a named dependency, even without a call. No
    // runtime compilation or unrelated SCC growth is needed to catch it.
    fs::write(
        fixture.0.join("crates/mc-core/src/host.rs"),
        format!("{host}\nuse crate::settlements::Registry as NewOwner;\n"),
    )
    .unwrap();
    let changed = fixture.facts();
    assert_eq!(changed.domain_refs.len(), ceiling + 1);
    assert_eq!(largest_component(&changed.edges()).len(), 1);
    crate::code_health::domain_boundary::check(&fixture.0, &changed.domain_refs, &mut findings);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].message.contains("D65 inbound domain paths"));
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
        construction_field_accesses: 0,
        hot_files: &[],
        split_crates: &[],
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
fn split_crate_code_keeps_counting_toward_the_ceilings() {
    let fixture = Fixture::new(
        "split-crate",
        &[
            ("lib.rs", "mod a;\n"),
            ("a.rs", "pub fn f() -> &'static str {\n    \"minecraft:stone\"\n}\n"),
        ],
    );
    let split = fixture.0.join("crates/mc-split/src");
    fs::create_dir_all(&split).unwrap();
    fs::write(split.join("lib.rs"), "mod b;\n").unwrap();
    fs::write(
        split.join("b.rs"),
        "pub fn g() -> &'static str {\n    \"minecraft:dirt\"\n}\n",
    )
    .unwrap();
    let ceilings = Ceilings {
        cycle_modules: 1,
        vanilla_id_literals: 2,
        path_equality: 0,
        panics: 0,
        merge_block_write_pushes: 0,
        construction_field_accesses: 0,
        hot_files: &[],
        split_crates: &["mc-split"],
    };
    let mut findings = Vec::new();
    check(&fixture.0, &ceilings, &mut findings);
    assert!(findings.is_empty(), "{findings:?}");

    // Negative control: a literal added in the split crate fails mc-core's ceiling.
    fs::write(
        split.join("b.rs"),
        "pub fn g() -> [&'static str; 2] {\n    [\"minecraft:dirt\", \"minecraft:sand\"]\n}\n",
    )
    .unwrap();
    check(&fixture.0, &ceilings, &mut findings);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0]
            .message
            .contains("string literals: 3 is above its ceiling 2"),
        "{}",
        findings[0].message
    );

    // A listed crate that does not exist fails closed.
    let mut findings = Vec::new();
    let missing = Ceilings {
        split_crates: &["mc-gone"],
        ..ceilings
    };
    check(&fixture.0, &missing, &mut findings);
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("cannot read split crate mc-gone")),
        "{findings:?}"
    );
}

#[test]
fn hot_file_counts_ignore_layout_and_test_code() {
    let fixture = Fixture::new(
        "hot-file-filter",
        &[
            ("lib.rs", "mod a; mod b; mod ignored_tests;"),
            (
                "a.rs",
                r#"
                // let fake = "statements in a comment";
                #[doc = "fn fake() {}"]
                pub fn f() {
                    #[cfg(test)] let ignored = 1;
                    #[cfg(test)] assert!(false);
                    let keep = 1;
                }
                #[cfg(test)] fn test_only() { panic!("ignored"); }
                #[test] fn another_test() { panic!("ignored"); }
                #[cfg(test)] mod tests { fn hidden() {} }
                "#,
            ),
            (
                "b.rs",
                r#"
                struct S;
                impl S {
                    fn production() {}
                    #[cfg(test)] fn hidden() {}
                }
                trait T {
                    fn production();
                    #[cfg(test)] fn hidden();
                }
                unsafe extern "C" {
                    fn production();
                    #[cfg(test)] fn hidden();
                }
                "#,
            ),
            ("ignored_tests.rs", "fn hidden() { let ignored = 1; }"),
        ],
    );
    let facts = fixture.facts();
    let src = fixture.0.join("crates/mc-core/src");
    assert_eq!(facts.files[&src.join("lib.rs")], 2);
    assert_eq!(facts.files[&src.join("a.rs")], 2);
    assert_eq!(facts.files[&src.join("b.rs")], 7);
    assert!(!facts.files.contains_key(&src.join("ignored_tests.rs")));

    fs::write(src.join("a.rs"), "\n\npub fn f() { let keep = 1; }\n").unwrap();
    assert_eq!(fixture.facts().files[&src.join("a.rs")], 2);
}

#[test]
fn hot_file_growth_above_its_measured_ceiling_fails() {
    let fixture = Fixture::new(
        "hot-file-growth",
        &[("lib.rs", "mod a;"), ("a.rs", "fn f() { let first = 1; }")],
    );
    let ceilings = Ceilings {
        hot_files: &[("a.rs", 2)],
        split_crates: &[],
        ..super::CEILINGS
    };
    let mut findings = Vec::new();
    check(&fixture.0, &ceilings, &mut findings);
    assert!(findings.is_empty(), "{findings:?}");

    // Negative control: one new production statement, not a line-count change.
    fs::write(
        fixture.0.join("crates/mc-core/src/a.rs"),
        "fn f() { let first = 1; let second = 2; }",
    )
    .unwrap();
    check(&fixture.0, &ceilings, &mut findings);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0]
            .message
            .contains("a.rs: 3 is above its ceiling 2")
    );
}

#[test]
fn construction_accesses_count_only_outside_the_owner_and_children() {
    let fixture = Fixture::new(
        "construction-access",
        &[
            (
                "lib.rs",
                "mod settlements; mod other; fn f() { show!(r.projects); }",
            ),
            (
                "settlements.rs",
                "mod construction; #[path = \"settlements/construction_extra.rs\"] \
                 mod legacy; mod registry;",
            ),
            (
                "settlements/construction.rs",
                "mod child; fn f() { r.projects; r.orders; }",
            ),
            (
                "settlements/construction/child.rs",
                "fn f() { r.projects; }",
            ),
            (
                "settlements/construction_extra.rs",
                "#[path = \"legacy_child.rs\"] mod child; fn f() { r.projects; }",
            ),
            ("settlements/legacy_child.rs", "fn f() { r.orders; }"),
            (
                "settlements/registry.rs",
                r#"
                fn f() { r.projects; r.orders; }
                #[cfg(test)] fn hidden() { r.projects; }
                const TEXT: &str = "r.projects r.orders";
                "#,
            ),
            ("other.rs", "mod construction;"),
            ("other/construction.rs", "fn f() { r.projects; }"),
        ],
    );
    assert_eq!(fixture.facts().total(Counter::ConstructionFieldAccess), 4);
    let measured = Ceilings {
        construction_field_accesses: 4,
        hot_files: &[],
        split_crates: &[],
        ..super::CEILINGS
    };
    let mut findings = Vec::new();
    check(&fixture.0, &measured, &mut findings);
    assert!(findings.is_empty(), "{findings:?}");
    check(
        &fixture.0,
        &Ceilings {
            construction_field_accesses: 3,
            ..measured
        },
        &mut findings,
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        findings[0]
            .message
            .contains("accesses outside construction")
    );

    let inline = Fixture::new(
        "construction-inline",
        &[(
            "lib.rs",
            "mod settlements { mod construction { mod child { fn f() { r.projects; } } } \
             mod registry { fn f() { r.orders; } } }",
        )],
    );
    assert_eq!(inline.facts().total(Counter::ConstructionFieldAccess), 1);
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
