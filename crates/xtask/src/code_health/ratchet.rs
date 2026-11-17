//! Coherence ratchets for `mc-core` production code, measured on the syntax
//! tree (`core_ast`) and the module graph (`module_graph`), never on text.
//!
//! Each ceiling is the value measured when it was last set. A measurement
//! above its ceiling fails code-health. A measurement below it prints
//! `ceiling can drop to N`: lower the constant in the same change. Raising a
//! ceiling is an explicit edit of the constant with a reason comment next to
//! it; there is no flag or environment override. Anti-pattern crate totals
//! survive file splits; the hot-file ceilings deliberately reward extraction.

use std::path::Path;

use super::Finding;
use super::core_ast::{self, CoreFacts, Counter};
use super::module_graph;

#[derive(Clone, Copy, Debug)]
pub(super) struct Ceilings {
    /// Modules in the largest strongly connected component of the top-level
    /// module graph: no module may join the cycle.
    pub(super) cycle_modules: usize,
    pub(super) vanilla_id_literals: usize,
    pub(super) path_equality: usize,
    pub(super) panics: usize,
    pub(super) merge_block_write_pushes: usize,
    pub(super) construction_field_accesses: usize,
    /// Paths relative to `crates/mc-core/src`.
    pub(super) hot_files: &'static [(&'static str, usize)],
}

pub(super) const CEILINGS: Ceilings = Ceilings {
    // Original ceilings measured on ffd52010, 2026-11-07.
    // Config owns block-log policy, 2026-11-09: config, debt and carrier
    // blocks leave the cycle without moving runtime state.
    // Settled-block orchestration is a tick child: 185 modules, 2026-11-09.
    // Books is a pure encoding leaf: 184 modules, 2026-11-09.
    // Merge ec3a463a (LedgerMobsB01) adds the sniffer, parrot and strider tick
    // modules, all of which drive TickState: 187 modules, 2026-11-12.
    cycle_modules: 187,
    // Offline conversion/tools moved above core, 2026-11-08:
    // canonical code-health measured 2617 literals and 114 path comparisons.
    // Canonical ITEM_REGISTRY cutover, 2026-11-11: measured 2598, no spare headroom.
    // Merge ec3a463a (LedgerMobsB01) adds 29 mob-behaviour ids (armadillo,
    // blueprint, loot, frog, mounts, parrot, sleep, sniffer, strider): 2646, 2026-11-12.
    // Q04 climate variants, 2026-11-11: +1 (2647). The cow, pig, chicken and
    // zombie_nautilus identifiers live once as blueprint::VARIANT_* constants
    // that key both the registry mapping and the capture path.
    // CivSimLanding merge of main 71d1c127, 2026-11-12: the ITEM_REGISTRY
    // cutover and main's mob ids measure 2628 together, no spare headroom.
    // Kelp-wash fix a5a8a865 and villager doors b293281d, 2026-11-13: the
    // `washed_away_by_fluids` and `mob_interactable_doors` block tags resolve
    // once at registry load (the remedy this ratchet asks for): 2631.
    // Drowned joins the `hunts_villagers` capability table next to zombie,
    // zombie_villager and husk (vanilla Drowned targets AbstractVillager),
    // 2026-11-13: 2632.
    // R6 G1 (CrowdPush, 2026-11-18): the player and falling-block body sizes are
    // contract lookups by vanilla name, so `blueprint::PLAYER_BODY` and
    // `FALLING_BLOCK_BODY` each name their type once: 2634. These are the
    // registry keys the sizes are resolved from, not inline ids at a use site.
    vanilla_id_literals: 2_634,
    // Measured 113 on 87c33206, 2026-11-13.
    path_equality: 113,
    panics: 5,
    merge_block_write_pushes: 17,
    // D9 baseline on b58021e61, 2026-11-17; no spare headroom.
    construction_field_accesses: 31,
    hot_files: &[
        ("tick/entity_phase.rs", 734),
        ("settlements/construction.rs", 1_076),
        ("tick.rs", 834),
        ("entities.rs", 924),
    ],
};

/// The cycle's production lines on ffd52010: the reference of a diagnostic
/// trend, never a ceiling (every feature in a cycle module adds lines).
const CYCLE_LINES_REFERENCE: (usize, &str) = (188_451, "ffd52010");

impl Ceilings {
    fn counter(&self, counter: Counter) -> usize {
        match counter {
            Counter::VanillaIdLiteral => self.vanilla_id_literals,
            Counter::PathEquality => self.path_equality,
            Counter::Panic => self.panics,
            Counter::MergeBlockWritePush => self.merge_block_write_pushes,
            Counter::ConstructionFieldAccess => self.construction_field_accesses,
        }
    }
}

/// (name, why it is an anti-pattern, known false positives).
fn describe(counter: Counter) -> (&'static str, &'static str, &'static str) {
    match counter {
        Counter::VanillaIdLiteral => (
            "`\"minecraft:..\"` string literals",
            "inline vanilla id; resolve a typed kind or tag once at load",
            "data and registry tables, protocol/NBT/Anvil schema keys and wire \
             registry names hold vanilla ids legitimately; the module breakdown \
             shows where they concentrate",
        ),
        Counter::PathEquality => (
            "`.path()` compared with `==`/`!=`",
            "stringly-typed block/item identity; compare a typed kind",
            "the method is matched by name without its receiver type, so `path()` \
             on a file path or URL type counts too",
        ),
        Counter::Panic => (
            "`panic!` invocations",
            "crash path; persisted or external data must return an error",
            "a panic on a truly impossible internal state is legitimate",
        ),
        Counter::MergeBlockWritePush => (
            "`merge_block_writes.push(..)` calls",
            "hand-rolled block write; use `TickState::write_block`",
            "the push inside `TickState::write_block` itself counts, and so does \
             any receiver named `merge_block_writes` whatever its type",
        ),
        Counter::ConstructionFieldAccess => (
            "`.project`/`.pending_order` accesses outside construction",
            "construction state escapes its owner; use the construction interface",
            "field names are a structural proxy for SettlementRecord, without \
             receiver type resolution; unrelated same-named fields count too",
        ),
    }
}

/// What the syntax tree cannot show; these stay outside every number above.
const NOT_SEEN: &[&str] = &[
    "module references by type inference or method resolution: an edge exists only \
     where code names the module (`crate::`, `super::`, a root re-export)",
    "vanilla ids without the `minecraft:` prefix or built at runtime; byte strings",
    "a `.path()` right operand of `==`/`!=`, construction field accesses, and \
     items/statements inside a macro body that is not comma-separated expressions \
     (`macro_rules!` templates, `vec![x; n]`, tracing fields); such bodies are read \
     token by token only for vanilla ids, panics and merge-block-write pushes",
    "construction field destructuring and fields generated by macro expansion",
];

struct Limit {
    name: &'static str,
    measured: usize,
    ceiling: usize,
    fix: &'static str,
}

/// Measures `root`'s `mc-core`, prints the report and pushes a finding for
/// every ratchet above its ceiling.
pub(super) fn check(root: &Path, ceilings: &Ceilings, findings: &mut Vec<Finding>) {
    let src = root.join("crates/mc-core/src");
    let facts = match core_ast::collect(&src) {
        Ok(facts) => facts,
        Err(err) => {
            findings.push(Finding {
                path: src,
                line: 0,
                message: format!("cannot read mc-core structure: {err}"),
            });
            return;
        }
    };
    println!("mc-core coherence ratchets (syn, production code; above a ceiling fails):");
    let total_lines: usize =
        facts.modules.iter().map(|m| m.lines).sum::<usize>() + facts.root.lines;
    println!(
        "  {} top-level modules, {total_lines} production lines (lib.rs {})",
        facts.modules.len(),
        facts.root.lines
    );

    let edges = facts.edges();
    let cycle = module_graph::largest_component(&edges);
    let cycle_lines: usize = cycle.iter().map(|&m| facts.modules[m].lines).sum();
    println!(
        "  largest module cycle: {} modules (ceiling {}); {cycle_lines} production lines \
         (diagnostic trend, {} on {})",
        cycle.len(),
        ceilings.cycle_modules,
        CYCLE_LINES_REFERENCE.0,
        CYCLE_LINES_REFERENCE.1
    );
    if cycle.len() > 1 {
        report_cycle(&facts, &edges, &cycle);
    }
    let outside: Vec<&str> = (0..facts.modules.len())
        .filter(|module| cycle.binary_search(module).is_err())
        .map(|module| facts.modules[module].name.as_str())
        .collect();
    println!("    outside it ({}): {}", outside.len(), outside.join(", "));

    let mut limits = vec![Limit {
        name: "largest module cycle (modules)",
        measured: cycle.len(),
        ceiling: ceilings.cycle_modules,
        fix: "a new module reference pulled modules into the cycle; remove it (see the \
              decoupling hints)",
    }];
    for counter in Counter::ALL {
        let (name, why, false_positives) = describe(counter);
        let measured = facts.total(counter);
        println!(
            "  {name}: {measured} (ceiling {}) - {why}",
            ceilings.counter(counter)
        );
        println!("    by module: {}", breakdown(&facts, counter));
        println!("    false positives: {false_positives}");
        limits.push(Limit {
            name,
            measured,
            ceiling: ceilings.counter(counter),
            fix: why,
        });
    }
    for &(path, ceiling) in ceilings.hot_files {
        let measured = facts.files.get(&src.join(path)).copied().unwrap_or(0);
        println!("  hot file {path}: {measured} production items/statements (ceiling {ceiling})");
        limits.push(Limit {
            name: path,
            measured,
            ceiling,
            fix: "hot-file growth; extract code into its owning module",
        });
    }
    println!("  not seen by these counts (no structural form):");
    for line in NOT_SEEN {
        println!("    - {line}");
    }

    for limit in limits {
        if limit.measured > limit.ceiling {
            findings.push(Finding {
                path: src.clone(),
                line: 0,
                message: format!(
                    "ratchet {}: {} is above its ceiling {} (+{}): {}, or raise the \
                     ceiling in crates/xtask/src/code_health/ratchet.rs with a reason comment",
                    limit.name,
                    limit.measured,
                    limit.ceiling,
                    limit.measured - limit.ceiling,
                    limit.fix
                ),
            });
        } else if limit.measured < limit.ceiling {
            println!(
                "  {}: ceiling can drop to {} (now {}); lower it in \
                 crates/xtask/src/code_health/ratchet.rs",
                limit.name, limit.measured, limit.ceiling
            );
        }
    }
    println!();
}

fn report_cycle(facts: &CoreFacts, edges: &[Vec<usize>], cycle: &[usize]) {
    let name = |module: usize| facts.modules[module].name.as_str();
    let mut degrees = module_graph::degrees(edges, cycle);
    degrees.sort_by(|a, b| {
        (b.incoming.len() + b.outgoing.len())
            .cmp(&(a.incoming.len() + a.outgoing.len()))
            .then_with(|| a.node.cmp(&b.node))
    });
    let top: Vec<String> = degrees
        .iter()
        .take(10)
        .map(|d| format!("{} {}/{}", name(d.node), d.incoming.len(), d.outgoing.len()))
        .collect();
    println!(
        "    most connected inside it (in/out module edges): {}",
        top.join(", ")
    );
    println!("    decoupling hints (cut these edges and the modules leave the cycle):");
    for hint in module_graph::decoupling_hints(edges, cycle).iter().take(8) {
        let refs: usize = hint
            .cut
            .iter()
            .map(|&(from, to)| facts.modules[from].refs.get(&to).copied().unwrap_or(0))
            .sum();
        let cut = if hint.cut.len() <= 3 {
            let listed: Vec<String> = hint
                .cut
                .iter()
                .map(|&(from, to)| format!("{} -> {}", name(from), name(to)))
                .collect();
            listed.join(", ")
        } else if hint.cut.iter().all(|&(_, to)| to == hint.node) {
            format!("every edge into {}", name(hint.node))
        } else {
            format!("every edge out of {}", name(hint.node))
        };
        println!(
            "      {}: cut {} edge(s), {refs} reference(s) ({cut}); {} module(s) leave",
            name(hint.node),
            hint.cut.len(),
            hint.leaving
        );
    }
}

fn breakdown(facts: &CoreFacts, counter: Counter) -> String {
    let mut counts: Vec<(&str, usize)> = facts
        .modules
        .iter()
        .chain([&facts.root])
        .map(|module| (module.name.as_str(), module.count(counter)))
        .filter(|&(_, count)| count > 0)
        .collect();
    if counts.is_empty() {
        return "none".into();
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let shown: Vec<String> = counts
        .iter()
        .take(8)
        .map(|(name, count)| format!("{name} {count}"))
        .collect();
    let rest: usize = counts.iter().skip(8).map(|&(_, count)| count).sum();
    if rest > 0 {
        format!(
            "{}, {} more modules {rest}",
            shown.join(", "),
            counts.len() - 8
        )
    } else {
        shown.join(", ")
    }
}

#[cfg(test)]
#[path = "ratchet_tests.rs"]
mod tests;
