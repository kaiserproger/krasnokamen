//! The production engine's persistent compilation cache.
//!
//! Startup recompiles every plugin's component unless Wasmtime's builtin cache
//! is attached: the builtin keys each entry by content hash plus the engine
//! revision and every compile setting, so unchanged components survive a
//! restart while changed bytes or a changed compiler can never reuse stale
//! code. Corrupt entries fall back to recompiling; the builtin never serves
//! bytes it cannot decode.
//!
//! These tests prove the mechanism the production [`engine`] relies on, with
//! the same builtin API and the real fixture, against an isolated cache
//! directory per test: the second engine instance hits the first one's entry,
//! different bytes and different compiler settings compile fresh without
//! aliasing or poisoning each other, and a corrupted directory still compiles.
//! The last test proves the shipped [`engine`] itself caches: it respawns this
//! binary with an isolated cache home, compiles the fixture on two fresh
//! production engines, and asserts the second engine reused the first one's
//! entries instead of recompiling.

mod fixture;

use std::path::{Path, PathBuf};

use fixture::{component_bytes, repo_root};
use mc_plugin_host::{CompiledPlugin, PluginLimits, engine};
use wasmtime::component::Component;
use wasmtime::{Cache, CacheConfig, Config, Engine, OptLevel};
use wit_component::{ComponentEncoder, StringEncoding, dummy_module, embed_component_metadata};
use wit_parser::{ManglingAndAbi, Resolve};

/// An engine with the builtin cache pointed at `dir`, the same attachment the
/// production engine uses, plus the cache handle so the test can read the
/// builtin's own hit/miss counters.
fn cached_engine(dir: &Path) -> (Engine, Cache) {
    let mut cache_config = CacheConfig::new();
    cache_config.with_directory(dir);
    let cache = Cache::new(cache_config).expect("a temp dir takes a compilation cache");
    let mut config = Config::new();
    config.wasm_component_model(true).cache(Some(cache.clone()));
    let engine = Engine::new(&config).expect("an engine with a cache builds");
    (engine, cache)
}

/// A valid component of the same contract with different bytes: the dummy
/// module for the real WIT, encoded the way a published package is encoded.
fn dummy_component_bytes() -> Vec<u8> {
    let wit: PathBuf = repo_root().join("crates/mc-script/wit");
    let mut resolve = Resolve::new();
    let (package, _) = resolve.push_path(&wit).expect("the WIT parses");
    let world = resolve
        .select_world(&[package], Some("plugin"))
        .expect("the WIT declares the `plugin` world");
    let mut module = dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8)
        .expect("the dummy module takes the world's component types");
    ComponentEncoder::default()
        .module(&module)
        .expect("the dummy module carries its component types")
        .validate(true)
        .encode()
        .expect("the dummy module encodes as a component")
}

/// Overwrite every file under `root` with garbage, returning how many files
/// were corrupted. Whatever the cache layout, the next engine must recompile
/// instead of serving or choking on these entries.
fn corrupt_files(root: &Path) -> usize {
    let mut corrupted = 0;
    for entry in std::fs::read_dir(root).expect("the cache directory reads") {
        let path = entry.expect("a cache entry").path();
        if path.is_dir() {
            corrupted += corrupt_files(&path);
        } else if path.is_file() {
            std::fs::write(&path, b"this is not a cached module").expect("a cache file corrupts");
            corrupted += 1;
        }
    }
    corrupted
}

#[test]
fn same_bytes_are_served_from_cache_on_the_next_engine_instance() {
    let dir = tempfile::tempdir().expect("a cache root");
    let bytes = component_bytes();

    let (first, first_cache) = cached_engine(dir.path());
    Component::new(&first, &bytes).expect("the fixture compiles");
    assert!(
        first_cache.cache_misses() >= 1,
        "the first compile populates the cache"
    );
    drop(first);
    drop(first_cache);

    let (second, second_cache) = cached_engine(dir.path());
    Component::new(&second, &bytes).expect("the same bytes compile on a fresh engine");
    assert!(
        second_cache.cache_hits() >= 1,
        "the second engine hits the first engine's cache entry instead of re-running Cranelift"
    );
}

#[test]
fn changed_bytes_and_changed_compiler_settings_do_not_reuse_stale_code() {
    let dir = tempfile::tempdir().expect("a cache root");
    let bytes = component_bytes();
    let other = dummy_component_bytes();
    assert_ne!(bytes, other, "the two components differ");

    let (engine, cache) = cached_engine(dir.path());
    Component::new(&engine, &bytes).expect("the fixture compiles");
    let hits = cache.cache_hits();
    let misses = cache.cache_misses();

    Component::new(&engine, &other).expect("different bytes compile");
    assert_eq!(
        cache.cache_hits(),
        hits,
        "different bytes must not hit the fixture's cache entry"
    );
    assert!(
        cache.cache_misses() > misses,
        "different bytes compile fresh instead of reusing cached code"
    );

    Component::new(&engine, &bytes).expect("the fixture still compiles");
    assert!(
        cache.cache_hits() > hits,
        "the fixture's own entry survived the other component"
    );

    // A changed compiler setting is a different cache key, never a stale hit.
    let mut retuned = Config::new();
    retuned
        .wasm_component_model(true)
        .cranelift_opt_level(OptLevel::None)
        .cache(Some(cache.clone()));
    let retuned_engine = Engine::new(&retuned).expect("a retuned engine builds");
    let hits_before_retune = cache.cache_hits();
    Component::new(&retuned_engine, &bytes).expect("the retuned compile succeeds");
    assert_eq!(
        cache.cache_hits(),
        hits_before_retune,
        "a changed opt level must not hit the old entry"
    );
}

#[test]
fn a_corrupted_cache_falls_back_to_recompiling() {
    let dir = tempfile::tempdir().expect("a cache root");
    let bytes = component_bytes();

    let (engine, cache) = cached_engine(dir.path());
    Component::new(&engine, &bytes).expect("the fixture compiles");
    assert!(
        cache.cache_misses() >= 1,
        "the first compile populates the cache"
    );
    drop(engine);
    drop(cache);

    let corrupted = corrupt_files(dir.path());
    assert!(
        corrupted >= 1,
        "the first compile left cache entries to corrupt"
    );

    let (fresh, _) = cached_engine(dir.path());
    Component::new(&fresh, &bytes)
        .expect("corrupt entries recompile instead of poisoning the engine");
}

/// The `ENGINE_CACHE_CHILD` entry point: compile the fixture once with the
/// shipped [`engine`] and record how many cache entries that produced.
///
/// This runs in a subprocess, never in the test process itself: the cache home
/// moves with the child (via `XDG_CACHE_HOME` when set, or `HOME` otherwise),
/// so the child's engines share one isolated directory without touching the
/// parent process's environment. The count goes to `ENGINE_CACHE_OUT` because
/// the test harness captures a passing test's stdout.
fn engine_cache_child(cache_home: &Path, out: &Path) {
    let before = cache_entry_paths(cache_home);
    let limits = PluginLimits::default();
    let bytes = component_bytes();
    let production = engine(&limits).expect("the production engine builds");
    let compiled = CompiledPlugin::compile(&production, &bytes, &limits, "0.9.0")
        .expect("the fixture compiles on a production engine");
    assert_eq!(compiled.api_version(), "0.9.0");
    let after = cache_entry_paths(cache_home);
    let mut added: Vec<String> = after
        .difference(&before)
        .map(|path| path.display().to_string())
        .collect();
    added.sort();
    // First line: the count the parent asserts on. Following lines: exactly which
    // entries appeared, so a race names the file instead of only the number.
    let mut report = added.len().to_string();
    for path in added {
        report.push('\n');
        report.push_str(&path);
    }
    std::fs::write(out, report).expect("the child reports its new entries");
}

/// Cache entry files below `cache_home`, as paths relative to it, wherever the
/// platform puts them: the Wasmtime cache lives under its own directory, and
/// the engine revision directory nests deeper still.
fn cache_entry_paths(cache_home: &Path) -> std::collections::BTreeSet<PathBuf> {
    fn walk(root: &Path, prefix: &Path, out: &mut std::collections::BTreeSet<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let relative = prefix.join(entry.file_name());
            if path.is_dir() {
                walk(&path, &relative, out);
            } else if path.is_file() && is_module_entry(&relative) {
                out.insert(relative);
            }
        }
    }
    let mut out = std::collections::BTreeSet::new();
    walk(cache_home, Path::new(""), &mut out);
    out
}

/// Whether `relative` is a compiled module entry, by Wasmtime's own layout.
///
/// `wasmtime-internal-cache` 36.0.15 writes a module as
/// `<cache dir>/modules/<compiler dir>/<hash>` where `hash` is the 32-byte
/// cache key in unpadded base64url — 43 characters, no dot
/// (`lib.rs:257` `cache.directory().join("modules").join(compiler_dir)`,
/// `lib.rs:262-273` `root_path.join(hash)`).
///
/// Everything else in that tree is bookkeeping written by the cache worker
/// thread, asynchronously, after the compile that produced it returned — and it
/// must never be counted as a new entry:
///
/// * `<hash>.stats` — per-entry usage statistics (`worker.rs:273`, `:413`);
/// * `<hash>.wip-atomic-write-<reason>` — the staging file of the atomic
///   write that publishes the file above, renamed into place on success
///   (`lib.rs:327-342`). The *stats* write stages as
///   `<hash>.wip-atomic-write-stats`, which the old `*.stats` filter missed: it
///   was the file this test counted under load (1 of 100, then 3 of 100);
/// * `<path>.wip-<pid>` — cleanup/compression task locks (`worker.rs:837`);
/// * `<cache dir>/.cleanup` — the cleanup-task marker (`worker.rs:425`).
fn is_module_entry(relative: &Path) -> bool {
    let is_modules_child = relative
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .is_some_and(|dir| dir == "modules");
    let Some(name) = relative.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    // 43 base64url characters, no dot: the module hash and nothing else.
    is_modules_child
        && name.len() == 43
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

#[test]
fn the_counter_sees_module_entries_and_not_the_workers_async_writes() {
    // The exact file name this test counted under load
    // (`wasmtime/modules/wasmtime--1794681568745/<hash>.wip-atomic-write-stats`),
    // plus every other file Wasmtime's cache directory can hold.
    let module = Path::new(
        "wasmtime/modules/wasmtime--1794681568745/5P-Dzair9udw094-2sFFLB1geexthDk2VBZvP4Soskg",
    );
    assert!(is_module_entry(module), "a module entry is what the oracle counts");
    for not_an_entry in [
        "wasmtime/modules/wasmtime--1794681568745/5P-Dzair9udw094-2sFFLB1geexthDk2VBZvP4Soskg.wip-atomic-write-stats",
        "wasmtime/modules/wasmtime--1794681568745/5P-Dzair9udw094-2sFFLB1geexthDk2VBZvP4Soskg.stats",
        "wasmtime/modules/wasmtime--1794681568745/5P-Dzair9udw094-2sFFLB1geexthDk2VBZvP4Soskg.wip-4242",
        "wasmtime/.cleanup",
        "wasmtime/modules/wasmtime--1794681568745/not-a-hash",
    ] {
        assert!(
            !is_module_entry(Path::new(not_an_entry)),
            "{not_an_entry} is cache bookkeeping, not an entry"
        );
    }
}

#[test]
fn the_production_engine_reuses_its_cache_across_restarts() {
    if let (Ok(child_home), Ok(child_out)) = (
        std::env::var("ENGINE_CACHE_CHILD"),
        std::env::var("ENGINE_CACHE_OUT"),
    ) {
        engine_cache_child(Path::new(&child_home), Path::new(&child_out));
        return;
    }
    let home = tempfile::tempdir().expect("an isolated cache home");
    let exe = std::env::current_exe().expect("this test binary");
    // `(count, the entry paths that appeared)`: the paths make a race name the
    // file it counted instead of only the number.
    let run_child = |extra_env: Option<(&str, &Path)>| -> (usize, Vec<String>) {
        let out = tempfile::NamedTempFile::new().expect("a child report file");
        let mut command = std::process::Command::new(&exe);
        command
            .arg("--exact")
            .arg("the_production_engine_reuses_its_cache_across_restarts")
            .arg("--nocapture")
            .env("ENGINE_CACHE_CHILD", home.path())
            .env("ENGINE_CACHE_OUT", out.path());
        // `XDG_CACHE_HOME` only reroutes the cache on platforms that honour
        // it; `HOME` is the portable half, covering the macOS/Windows cache
        // directory too.
        command.env("XDG_CACHE_HOME", home.path());
        if let Some((key, value)) = extra_env {
            command.env(key, value);
        }
        assert!(
            command.status().expect("the child runs").success(),
            "a production engine child compiles",
        );
        // `NamedTempFile` unlinks on drop, so read before it goes out of scope.
        let report = std::fs::read_to_string(out.path())
            .expect("the child reports its new entries");
        let mut lines = report.lines();
        let count = lines
            .next()
            .expect("the child reports a count")
            .trim()
            .parse()
            .expect("the child reports a count");
        (count, lines.map(str::to_owned).collect())
    };
    // A fallback `HOME` for platforms without `XDG_CACHE_HOME`: only consulted
    // when the primary run leaves the cache home empty.
    for fallback in [None, Some(("HOME", home.path()))] {
        let (first_new, _) = run_child(fallback);
        if first_new == 0 {
            continue;
        }
        let (second_new, added) = run_child(fallback);
        // Without the cache the current production `engine()` is this test's
        // red: no shared directory is consulted, so the second compile writes
        // as many files as the first. With the cache attached the second
        // engine reuses the first one's entries and writes nothing new.
        assert_eq!(
            second_new, 0,
            "the second production engine reuses the first one's cache instead of recompiling; \
             it counted {added:?}"
        );
        return;
    }
    panic!(
        "the production engine wrote no cache entries: cache not enabled, or neither XDG_CACHE_HOME nor HOME reroutes this platform's cache directory"
    );
}
