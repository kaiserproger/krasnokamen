//! Real local data remains outside Git; this checks the serving source constructor.
use super::*;
use mc_world::ChunkGenerator;

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let destination = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            std::fs::copy(entry.path(), destination).unwrap();
        }
    }
}

#[test]
#[ignore = "requires local MVS_NETHER_INPUT_ROOT and KRASNOKAMEN_CONTENT_CACHE"]
fn selected_mvs_nether_sets_route_to_the_serving_nether_generator() {
    let input = std::env::var_os("MVS_NETHER_INPUT_ROOT")
        .map(PathBuf::from)
        .unwrap();
    let cache = std::env::var_os("KRASNOKAMEN_CONTENT_CACHE")
        .map(PathBuf::from)
        .unwrap();
    let world = tempfile::tempdir().unwrap();
    let packs = world.path().join("datapacks");
    let pack = packs.join("mvs-nether");
    copy_tree(&input.join("data/mvs"), &pack.join("data/mvs"));
    std::fs::write(
        pack.join("pack.mcmeta"),
        br#"{"pack":{"description":"local MVS Nether evaluation"}}"#,
    )
    .unwrap();
    let mut packs = super::super::PackSet::load(
        &packs,
        super::super::Selection {
            enabled: vec!["vanilla".into(), "file/mvs-nether".into()],
            disabled: Vec::new(),
        },
    )
    .unwrap();
    let mut inputs = packs.prepare_worldgen(world.path(), Some(&cache)).unwrap();
    let registry = crate::registry::CoreRegistry::load(Some(&cache)).unwrap();
    inputs.validate(&registry).unwrap();
    let expected = [
        "mvs:nether_well",
        "mvs:crimson_enchanting_table",
        "mvs:nether_devil",
    ];
    assert_eq!(
        inputs.structure_sets[&crate::dimensions::Dimension::Nether].len(),
        expected.len()
    );
    assert!(
        !inputs
            .structure_sets
            .contains_key(&crate::dimensions::Dimension::Overworld)
    );
    let stage = inputs.root().unwrap().to_owned();
    let mut config = crate::CoreConfig::new(world.path().to_owned(), 1234);
    config.vanilla_data_dir = Some(stage.clone());
    let generator = crate::core::nether_terrain(&config, &registry, Some(&inputs)).unwrap();
    drop(inputs);
    assert!(
        !stage.exists(),
        "compiled sources must not borrow the temporary resource image"
    );
    for name in expected {
        let id = mc_data::Identifier::parse(name).unwrap();
        let targets = std::slice::from_ref(&id);
        let mut found = None;
        'search: for ring in 0..=8 {
            for candidate in generator.structure_candidates_in_ring(targets, (0, 0), ring) {
                if let Some(site) = generator
                    .structure_sites_at(targets, candidate)
                    .into_iter()
                    .next()
                {
                    found = Some(site);
                    break 'search;
                }
            }
        }
        let site = found.unwrap_or_else(|| panic!("serving Nether source did not admit {name}"));
        let chunk = generator.generate(mc_world::ChunkPos {
            x: site.start_chunk.0,
            z: site.start_chunk.1,
        });
        let starts = mc_world::structure_starts::read(&chunk.extras, chunk.pos, false).unwrap();
        assert!(starts.iter().any(|start| start.structure == id));
        assert!((0..128).contains(&site.position.y));
        eprintln!(
            "selected serving Nether source {name}: {:?} {:?}",
            site.start_chunk, site.position
        );
    }
    let mut admitted = 0;
    for path in [
        "fortress",
        "bastion_remnant",
        "nether_fossil",
        "ruined_portal_nether",
    ] {
        let id = mc_data::Identifier::parse(format!("{}:{path}", "minecraft")).unwrap();
        let targets = std::slice::from_ref(&id);
        let mut candidates = 0;
        for ring in 0..=2 {
            for candidate in generator.structure_candidates_in_ring(targets, (0, 0), ring) {
                candidates += 1;
                let point = (candidate.x, candidate.z);
                let expected: Vec<_> = generator
                    .structure_sites_in_region(point, point)
                    .into_iter()
                    .filter(|site| site.structure == id)
                    .map(|site| (site.structure, site.start_chunk, site.position))
                    .collect();
                let actual: Vec<_> = generator
                    .structure_sites_at(targets, candidate)
                    .into_iter()
                    .map(|site| (site.structure, site.start_chunk, site.position))
                    .collect();
                admitted += actual.len();
                assert_eq!(
                    actual, expected,
                    "point lookup changed vanilla Nether {path} admission"
                );
            }
        }
        assert!(
            candidates > 0,
            "vanilla Nether {path} lost its placement grid"
        );
    }
    assert!(
        admitted > 0,
        "fixed-seed vanilla compatibility scope must include admitted starts"
    );
}
