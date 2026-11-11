//! Find and measure a natural battlefield in the production overworld generator.
//!
//! `coarse` ranks windows of many seeds from the router's cave-free LOD surface
//! (`TerrainGenerator::lod_column`, an 8-block lattice). `detail` builds the
//! serving generator (vanilla router, villages and jigsaw structures as
//! `mc_core::core::overworld_terrain` does without plugins), generates every
//! chunk of the field plus its margin and measures the real columns: ground
//! height, neighbour steps, water, trees and non-natural blocks.
//!
//! ```text
//! battle_site_scan coarse <content> <out.jsonl> <seed-from> <seed-to> <half-extent> <threads>
//! battle_site_scan detail <content> <out-dir> <seed> <cx> <cz> <width> <depth> <margin> <threads>
//! ```

use std::collections::BTreeMap;
use std::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use mc_data::Identifier;
use mc_world::{BlockRegistry, ChunkGenerator, ChunkPos};
use mc_worldgen::TerrainGenerator;
use mc_worldgen::vanilla_features::{BlockSemantics, CacheTags};
use mc_worldgen::village::plan_source::VillagePlanSource;
use mc_worldgen::village::{VillageDecor, load_village_closure};

const WIDTH: i32 = 420;
const DEPTH: i32 = 340;
const STEP: i32 = 8;
const SEA_LEVEL: i32 = 63;

type Result<T, E = Box<dyn Error + Send + Sync>> = std::result::Result<T, E>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args.first().map(String::as_str) {
        Some("coarse") if args.len() == 7 => coarse(&args[1..]),
        Some("detail") if args.len() == 10 => detail(&args[1..]),
        _ => Err("usage: see the module documentation".into()),
    };
    if let Err(error) = outcome {
        eprintln!("battle_site_scan: {error}");
        std::process::exit(1);
    }
}

fn blocks() -> Result<Arc<BlockRegistry>> {
    Ok(Arc::new(BlockRegistry::from_report(
        &mc_data::blocks::solaris_required_blocks_report(),
    )?))
}

fn good_biome(path: &str) -> bool {
    matches!(
        path,
        "plains" | "sunflower_plains" | "meadow" | "savanna" | "savanna_plateau"
    )
}

fn wet_biome(path: &str) -> bool {
    path.contains("river")
        || path.contains("ocean")
        || path.contains("swamp")
        || path.contains("beach")
}

struct Window {
    seed: i64,
    tier: &'static str,
    cx: i32,
    cz: i32,
    range: i32,
    robust: i32,
    rough: f64,
    max_step8: i32,
    bad: usize,
    wet: usize,
    origin: (usize, usize),
    biomes: BTreeMap<String, usize>,
}

fn coarse(args: &[String]) -> Result<()> {
    let root = PathBuf::from(&args[0]);
    let out = PathBuf::from(&args[1]);
    let (from, to): (i64, i64) = (args[2].parse()?, args[3].parse()?);
    let half: i32 = args[4].parse()?;
    let threads: usize = args[5].parse()?;
    let blocks = blocks()?;
    let next = AtomicUsize::new(0);
    let seeds: Vec<i64> = (from..=to).collect();
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&seed) = seeds.get(index) else { break };
                    let started = std::time::Instant::now();
                    match scan_seed(&root, &blocks, seed, half) {
                        Ok(windows) => {
                            eprintln!(
                                "seed {seed}: {} windows in {:.1}s",
                                windows.len(),
                                started.elapsed().as_secs_f64()
                            );
                            results.lock().unwrap().extend(windows);
                        }
                        Err(error) => eprintln!("seed {seed}: {error}"),
                    }
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| score(a).total_cmp(&score(b)));
    let mut file = std::fs::File::create(&out)?;
    for w in &results {
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "seed": w.seed, "tier": w.tier, "cx": w.cx, "cz": w.cz, "range8": w.range,
                "p5_p95_range8": w.robust,
                "mean_abs_step8": (w.rough * 100.0).round() / 100.0, "max_step8": w.max_step8,
                "bad_biome_samples": w.bad, "wet_samples": w.wet, "score": score(w),
                "biomes": w.biomes,
            })
        )?;
    }
    println!("{} windows ranked into {}", results.len(), out.display());
    Ok(())
}

fn score(w: &Window) -> f64 {
    f64::from(w.range) + f64::from(w.robust) + 2.0 * w.rough + 50.0 * w.wet as f64
}

fn scan_seed(
    root: &Path,
    blocks: &Arc<BlockRegistry>,
    seed: i64,
    half: i32,
) -> Result<Vec<Window>> {
    let mut generator = TerrainGenerator::new(seed, Arc::clone(blocks));
    generator.load_vanilla_worldgen(root)?;
    let n = (2 * half / STEP + 1) as usize;
    let mut height = vec![0i32; n * n];
    let mut biome = vec![0u16; n * n];
    let mut names: Vec<String> = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let x = -half + i as i32 * STEP;
            let z = -half + j as i32 * STEP;
            let (y, id) = generator.lod_column(x, z);
            height[j * n + i] = y;
            let path = id.path().to_owned();
            let code = match names.iter().position(|name| *name == path) {
                Some(code) => code,
                None => {
                    names.push(path);
                    names.len() - 1
                }
            };
            biome[j * n + i] = code as u16;
        }
    }
    let good: Vec<bool> = names.iter().map(|name| good_biome(name)).collect();
    let wet: Vec<bool> = names.iter().map(|name| wet_biome(name)).collect();
    let (wn, dn) = ((WIDTH / STEP + 1) as usize, (DEPTH / STEP + 1) as usize);
    let mut windows = Vec::new();
    let stride = 4usize;
    let mut j0 = 0;
    while j0 + dn <= n {
        let mut i0 = 0;
        while i0 + wn <= n {
            let (mut lo, mut hi, mut bad, mut wets, mut sum, mut pairs, mut max_step) =
                (i32::MAX, i32::MIN, 0usize, 0usize, 0i64, 0i64, 0i32);
            let mut histogram = [0u32; 400];
            for j in j0..j0 + dn {
                for i in i0..i0 + wn {
                    let y = height[j * n + i];
                    let b = biome[j * n + i] as usize;
                    lo = lo.min(y);
                    hi = hi.max(y);
                    histogram[(y + 64).clamp(0, 399) as usize] += 1;
                    if !good[b] {
                        bad += 1;
                    }
                    if wet[b] || y < SEA_LEVEL {
                        wets += 1;
                    }
                    if i + 1 < i0 + wn {
                        let d = (height[j * n + i + 1] - y).abs();
                        sum += i64::from(d);
                        pairs += 1;
                        max_step = max_step.max(d);
                    }
                    if j + 1 < j0 + dn {
                        let d = (height[(j + 1) * n + i] - y).abs();
                        sum += i64::from(d);
                        pairs += 1;
                        max_step = max_step.max(d);
                    }
                }
            }
            if wets == 0 {
                let cells = (wn * dn) as u32;
                let quantile = |q: u32| {
                    let mut seen = 0;
                    for (index, count) in histogram.iter().enumerate() {
                        seen += count;
                        if seen * 100 >= cells * q {
                            return index as i32;
                        }
                    }
                    399
                };
                windows.push(Window {
                    seed,
                    tier: if bad * 10 <= wn * dn { "open" } else { "any" },
                    cx: -half + ((i0 + wn / 2) as i32) * STEP,
                    cz: -half + ((j0 + dn / 2) as i32) * STEP,
                    range: hi - lo,
                    robust: quantile(95) - quantile(5),
                    rough: sum as f64 / pairs as f64,
                    max_step8: max_step,
                    bad,
                    wet: wets,
                    origin: (i0, j0),
                    biomes: BTreeMap::new(),
                });
            }
            i0 += stride;
        }
        j0 += stride;
    }
    windows.sort_by(|a, b| score(a).total_cmp(&score(b)));
    let mut kept: Vec<Window> = Vec::new();
    for tier in ["open", "any"] {
        let mut taken = 0;
        for w in windows.iter().filter(|w| tier == "any" || w.tier == "open") {
            if kept
                .iter()
                .all(|k| (k.cx - w.cx).abs() > WIDTH || (k.cz - w.cz).abs() > DEPTH)
            {
                let (i0, j0) = w.origin;
                let mut counts = BTreeMap::new();
                for j in j0..j0 + dn {
                    for i in i0..i0 + wn {
                        *counts
                            .entry(names[biome[j * n + i] as usize].clone())
                            .or_insert(0) += 1;
                    }
                }
                kept.push(Window {
                    biomes: counts,
                    ..*w
                });
                taken += 1;
                if taken == 3 {
                    break;
                }
            }
        }
    }
    Ok(kept)
}

/// The serving overworld generator without plugins
/// (`mc_core::core::overworld_terrain`).
fn serving_generator(
    root: &Path,
    blocks: &Arc<BlockRegistry>,
    seed: i64,
) -> Result<TerrainGenerator> {
    let mut terrain = TerrainGenerator::new(seed, Arc::clone(blocks));
    terrain.load_vanilla_worldgen(root)?;
    let data = Arc::new(mc_data::load(root)?);
    let tags = Arc::new(mc_data::tags::load(root, &data)?);
    let tags = Arc::new(CacheTags::new(Arc::clone(blocks), data, tags));
    for (index, set) in [
        "villages",
        "ancient_cities",
        "trail_ruins",
        "trial_chambers",
    ]
    .into_iter()
    .enumerate()
    {
        let closure = Arc::new(load_village_closure(
            root,
            &Identifier::parse(format!("minecraft:{set}"))?,
            blocks,
        )?);
        let decor = Arc::new(VillageDecor::compile(
            root,
            &closure,
            &BlockSemantics::new(blocks, tags.as_ref()),
        )?);
        let source = Arc::new(VillagePlanSource::new(
            closure,
            decor,
            seed,
            Arc::clone(blocks),
            tags.clone(),
            tags.clone(),
        )?);
        terrain = if index == 0 {
            terrain.with_village_plans(source)
        } else {
            terrain.with_jigsaw_source(source)
        };
    }
    Ok(terrain)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Air,
    Plant,
    Tree,
    Fluid,
    Ground,
    Other,
}

fn kind(path: &str) -> Kind {
    if matches!(path, "air" | "cave_air" | "void_air") {
        return Kind::Air;
    }
    if matches!(
        path,
        "water"
            | "lava"
            | "bubble_column"
            | "seagrass"
            | "tall_seagrass"
            | "kelp"
            | "kelp_plant"
            | "ice"
    ) {
        return Kind::Fluid;
    }
    if path.ends_with("_log")
        || path.ends_with("_wood")
        || path.ends_with("_leaves")
        || path == "bee_nest"
        || path.ends_with("mushroom_block")
        || path == "mushroom_stem"
        || path == "mangrove_roots"
        || path == "muddy_mangrove_roots"
    {
        return Kind::Tree;
    }
    if path.ends_with("_sapling")
        || path.ends_with("_mushroom")
        || path.ends_with("grass")
        || path.ends_with("fern")
        || path.ends_with("_bush")
        || path == "bush"
        || matches!(
            path,
            "dandelion"
                | "poppy"
                | "blue_orchid"
                | "allium"
                | "azure_bluet"
                | "red_tulip"
                | "orange_tulip"
                | "white_tulip"
                | "pink_tulip"
                | "oxeye_daisy"
                | "cornflower"
                | "lily_of_the_valley"
                | "sunflower"
                | "lilac"
                | "rose_bush"
                | "peony"
                | "torchflower"
                | "pink_petals"
                | "wildflowers"
                | "leaf_litter"
                | "snow"
                | "moss_carpet"
                | "vine"
                | "sugar_cane"
                | "pumpkin"
                | "melon"
                | "cactus"
                | "cactus_flower"
                | "open_eyeblossom"
                | "closed_eyeblossom"
                | "pitcher_plant"
                | "short_dry_grass"
                | "tall_dry_grass"
                | "glow_lichen"
                | "hanging_roots"
        )
    {
        return Kind::Plant;
    }
    if matches!(
        path,
        "grass_block"
            | "dirt"
            | "coarse_dirt"
            | "podzol"
            | "rooted_dirt"
            | "mud"
            | "stone"
            | "andesite"
            | "diorite"
            | "granite"
            | "gravel"
            | "sand"
            | "red_sand"
            | "sandstone"
            | "red_sandstone"
            | "clay"
            | "moss_block"
            | "snow_block"
            | "packed_ice"
            | "calcite"
            | "tuff"
            | "deepslate"
            | "dripstone_block"
            | "mycelium"
            | "powder_snow"
    ) || path.ends_with("terracotta")
        || path.ends_with("_ore")
    {
        return Kind::Ground;
    }
    Kind::Other
}

const F_WATER: u8 = 1;
const F_LAVA: u8 = 2;
const F_TREE: u8 = 4;
const F_PLANT: u8 = 8;
const F_OTHER: u8 = 16;

fn detail(args: &[String]) -> Result<()> {
    let root = PathBuf::from(&args[0]);
    let out = PathBuf::from(&args[1]);
    let seed: i64 = args[2].parse()?;
    let (cx, cz): (i32, i32) = (args[3].parse()?, args[4].parse()?);
    let (width, depth, margin): (i32, i32, i32) =
        (args[5].parse()?, args[6].parse()?, args[7].parse()?);
    let threads: usize = args[8].parse()?;
    std::fs::create_dir_all(&out)?;
    let blocks = blocks()?;
    let started = std::time::Instant::now();
    let generator = serving_generator(&root, &blocks, seed)?;
    // Field: [cx - width/2, cx + width/2) x [cz - depth/2, cz + depth/2).
    let (fx0, fz0) = (cx - width / 2, cz - depth / 2);
    let (ax0, az0) = (fx0 - margin, fz0 - margin);
    let (aw, ad) = ((width + 2 * margin) as usize, (depth + 2 * margin) as usize);
    let (c0x, c0z) = (ax0.div_euclid(16), az0.div_euclid(16));
    let (c1x, c1z) = (
        (ax0 + aw as i32 - 1).div_euclid(16),
        (az0 + ad as i32 - 1).div_euclid(16),
    );
    let chunks: Vec<ChunkPos> = (c0z..=c1z)
        .flat_map(|z| (c0x..=c1x).map(move |x| ChunkPos { x, z }))
        .collect();
    let ground = std::sync::Mutex::new(vec![i16::MIN; aw * ad]);
    let flags = std::sync::Mutex::new(vec![0u8; aw * ad]);
    let tops = std::sync::Mutex::new(vec![i16::MIN; aw * ad]);
    let others = std::sync::Mutex::new(BTreeMap::<String, usize>::new());
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&pos) = chunks.get(index) else { break };
                    let chunk = generator.generate(pos);
                    let mut local = Vec::with_capacity(256);
                    let mut local_other = Vec::new();
                    for lz in 0..16u8 {
                        for lx in 0..16u8 {
                            let wx = pos.x * 16 + i32::from(lx);
                            let wz = pos.z * 16 + i32::from(lz);
                            let (ix, iz) = (wx - ax0, wz - az0);
                            if ix < 0 || iz < 0 || ix >= aw as i32 || iz >= ad as i32 {
                                continue;
                            }
                            let (mut g, mut top, mut f) = (i16::MIN, i16::MIN, 0u8);
                            for y in (-64..=250).rev() {
                                let Some(id) = chunk.get_block(lx, y, lz) else {
                                    continue;
                                };
                                let state = blocks.by_id(id).expect("generated state");
                                let path = state.block.id.path();
                                let k = kind(path);
                                if k == Kind::Air {
                                    continue;
                                }
                                if top == i16::MIN {
                                    top = y as i16;
                                }
                                match k {
                                    Kind::Plant => f |= F_PLANT,
                                    Kind::Tree => f |= F_TREE,
                                    Kind::Fluid => {
                                        f |= if path == "lava" { F_LAVA } else { F_WATER };
                                        g = y as i16;
                                        break;
                                    }
                                    Kind::Other => {
                                        f |= F_OTHER;
                                        local_other.push(path.to_owned());
                                        g = y as i16;
                                        break;
                                    }
                                    Kind::Ground => {
                                        g = y as i16;
                                        break;
                                    }
                                    Kind::Air => {}
                                }
                            }
                            local.push((iz as usize * aw + ix as usize, g, top, f));
                        }
                    }
                    let (mut ground, mut tops, mut flags) = (
                        ground.lock().unwrap(),
                        tops.lock().unwrap(),
                        flags.lock().unwrap(),
                    );
                    for (i, g, t, f) in local {
                        ground[i] = g;
                        tops[i] = t;
                        flags[i] = f;
                    }
                    let mut others = others.lock().unwrap();
                    for path in local_other {
                        *others.entry(path).or_insert(0) += 1;
                    }
                }
            });
        }
    });
    let ground = ground.into_inner().unwrap();
    let tops = tops.into_inner().unwrap();
    let flags = flags.into_inner().unwrap();
    let generated_seconds = started.elapsed().as_secs_f64();

    let inside = |ix: usize, iz: usize| {
        let (x, z) = (ix as i32 - margin, iz as i32 - margin);
        x >= 0 && z >= 0 && x < width && z < depth
    };
    let mut hist = BTreeMap::<i32, usize>::new();
    let (mut lo, mut hi, mut sum, mut cells) = (i32::MAX, i32::MIN, 0i64, 0usize);
    let (mut water, mut lava, mut tree, mut plant, mut other) = (0, 0, 0, 0, 0);
    let mut heights = Vec::new();
    for iz in 0..ad {
        for ix in 0..aw {
            if !inside(ix, iz) {
                continue;
            }
            let i = iz * aw + ix;
            let g = i32::from(ground[i]);
            heights.push(g);
            lo = lo.min(g);
            hi = hi.max(g);
            sum += i64::from(g);
            cells += 1;
            let f = flags[i];
            water += usize::from(f & F_WATER != 0);
            lava += usize::from(f & F_LAVA != 0);
            tree += usize::from(f & F_TREE != 0);
            plant += usize::from(f & F_PLANT != 0);
            other += usize::from(f & F_OTHER != 0);
            if ix + 1 < aw && inside(ix + 1, iz) {
                *hist
                    .entry((i32::from(ground[i + 1]) - g).abs())
                    .or_insert(0) += 1;
            }
            if iz + 1 < ad && inside(ix, iz + 1) {
                *hist
                    .entry((i32::from(ground[i + aw]) - g).abs())
                    .or_insert(0) += 1;
            }
        }
    }
    heights.sort_unstable();
    let pct = |p: f64| heights[((heights.len() - 1) as f64 * p).round() as usize];
    let pairs: usize = hist.values().sum();
    let over_one: usize = hist.iter().filter(|(d, _)| **d > 1).map(|(_, c)| c).sum();
    // Surroundings: every column of the margin ring.
    let (mut rlo, mut rhi, mut rwater, mut rtree, mut rcells) = (i32::MAX, i32::MIN, 0, 0, 0);
    for iz in 0..ad {
        for ix in 0..aw {
            if inside(ix, iz) {
                continue;
            }
            let i = iz * aw + ix;
            rlo = rlo.min(i32::from(ground[i]));
            rhi = rhi.max(i32::from(tops[i]).max(i32::from(ground[i])));
            rwater += usize::from(flags[i] & F_WATER != 0);
            rtree += usize::from(flags[i] & F_TREE != 0);
            rcells += 1;
        }
    }
    let mut biomes = BTreeMap::<String, usize>::new();
    for iz in (0..depth).step_by(4) {
        for ix in (0..width).step_by(4) {
            let i = (iz + margin) as usize * aw + (ix + margin) as usize;
            let b = generator.noise_biome(fx0 + ix, i32::from(ground[i]), fz0 + iz);
            *biomes.entry(b.path().to_owned()).or_insert(0) += 1;
        }
    }
    let villages: Vec<_> = generator
        .village_sites_in_region((c0x - 8, c0z - 8), (c1x + 8, c1z + 8))
        .into_iter()
        .map(|site| format!("{:?}", site))
        .collect();
    let centre_i = (cz - az0) as usize * aw + (cx - ax0) as usize;
    let stats = serde_json::json!({
        "seed": seed,
        "centre": [cx, i32::from(ground[centre_i]), cz],
        "centre_ground_y": ground[centre_i],
        "field": {"x0": fx0, "z0": fz0, "x1": fx0 + width - 1, "z1": fz0 + depth - 1, "width": width, "depth": depth},
        "area": {"x0": ax0, "z0": az0, "width": aw, "depth": ad, "margin": margin},
        "chunks_generated": chunks.len(),
        "generate_seconds": (generated_seconds * 10.0).round() / 10.0,
        "field_columns": cells,
        "ground_min": lo, "ground_max": hi, "ground_range": hi - lo,
        "ground_p1": pct(0.01), "ground_p5": pct(0.05), "ground_p50": pct(0.5),
        "ground_p95": pct(0.95), "ground_p99": pct(0.99),
        "ground_mean": (sum as f64 / cells as f64 * 100.0).round() / 100.0,
        "step_histogram": hist.iter().map(|(d, c)| (d.to_string(), *c)).collect::<BTreeMap<_, _>>(),
        "neighbour_pairs": pairs,
        "pairs_step_over_1": over_one,
        "pairs_step_over_1_fraction": over_one as f64 / pairs as f64,
        "water_columns": water, "lava_columns": lava, "tree_columns": tree,
        "plant_columns": plant, "non_natural_columns": other,
        "non_natural_blocks": others.into_inner().unwrap(),
        "biomes_step4": biomes,
        "surroundings": {"columns": rcells, "ground_min": rlo, "top_max": rhi, "water_columns": rwater, "tree_columns": rtree},
        "village_and_jigsaw_starts_near": villages,
    });
    std::fs::write(
        out.join("stats.json"),
        serde_json::to_string_pretty(&stats)? + "\n",
    )?;
    let mut bytes = Vec::with_capacity(aw * ad * 5);
    for ((g, t), f) in ground.iter().zip(&tops).zip(&flags) {
        bytes.extend_from_slice(&g.to_le_bytes());
        bytes.extend_from_slice(&t.to_le_bytes());
        bytes.push(*f);
    }
    std::fs::write(out.join("columns.bin"), bytes)?;
    println!("{}", serde_json::to_string(&stats)?);
    Ok(())
}
