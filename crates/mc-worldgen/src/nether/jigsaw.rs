//! Nether attachment for the same validated jigsaw plans used by the overworld.
use super::moog_jigsaw::NetherMoogQuery;
use super::*;
use crate::vanilla_features::{BlockSemantics, CacheTags};
use crate::village::VillageDecor;
use crate::village::closure::load_jigsaw_closure;
use crate::village::height_query::HeightQuery;
use crate::village::plan_source::VillagePlanSource;
use mc_data::village_data::VillageDataLoader;
use mc_world::chunk::GeneratedStructureSite;
use std::path::Path;

impl NetherGenerator {
    /// Discover Moog Nether sets in an input image, retaining their exact identifiers.
    /// Serving attaches selected, dimension-validated sources with `with_jigsaw_source`.
    pub fn load_jigsaw_worldgen(
        &mut self,
        root: &Path,
        tags: Arc<CacheTags>,
    ) -> Result<(), String> {
        let loader = VillageDataLoader::new(root.join("data/minecraft/worldgen"));
        let codec =
            Identifier::parse("moogs_structures:moogs_structures_generic_nether_jigsaw_structure")
                .expect("static structure codec");
        let sets = loader
            .structure_sets_of_type(&codec)
            .map_err(|e| e.to_string())?;
        let mut sources = Vec::with_capacity(sets.len());
        for set in sets {
            let set_id = set.id.clone();
            let mut closure = load_jigsaw_closure(root, &set_id, set, &self.registry)
                .map_err(|e| e.to_string())?;
            if closure
                .structures
                .iter()
                .any(|s| !mc_data::worldgen_structure_types::is_moog_nether(&s.spec.structure_type))
            {
                return Err(format!(
                    "Structure set {set_id} mixes Nether/overworld Moog codecs"
                ));
            }
            closure.dimension_bounds = (NETHER_MIN_Y, NETHER_CEILING_Y + 1);
            let closure = Arc::new(closure);
            let decor = Arc::new(
                VillageDecor::compile(
                    root,
                    &closure,
                    &BlockSemantics::new(&self.registry, tags.as_ref()),
                )
                .map_err(|e| e.to_string())?,
            );
            sources.push(Arc::new(
                VillagePlanSource::new(
                    closure,
                    decor,
                    self.seed,
                    self.registry.clone(),
                    tags.clone(),
                    tags.clone(),
                )
                .map_err(|e| e.to_string())?,
            ));
        }
        self.jigsaw_sources = sources;
        Ok(())
    }

    /// Attach one selected source whose biome set has been validated for this dimension.
    #[must_use]
    pub fn with_jigsaw_source(mut self, source: Arc<VillagePlanSource>) -> Self {
        self.jigsaw_sources.push(source);
        self
    }

    pub(super) fn jigsaw_sites(
        &self,
        min: (i32, i32),
        max: (i32, i32),
    ) -> Vec<GeneratedStructureSite> {
        let query = NetherMoogQuery { generator: self };
        let biome = |x, y, z| query.biome_at(x, y, z);
        let mut sites = Vec::new();
        for source in &self.jigsaw_sources {
            sites.extend(source.structure_sites_in_region(min, max, &query, &biome));
        }
        sites
    }

    pub(super) fn write_jigsaws(&self, chunk: &mut Chunk) {
        let query = NetherMoogQuery { generator: self };
        let biome = |x, y, z| query.biome_at(x, y, z);
        let surface = |x, z| query.world_surface(x, z);
        let mut touched = [false; 256];
        for source in &self.jigsaw_sources {
            if let Some(plans) = source.plans_for_chunk(chunk.pos.x, chunk.pos.z, &query, &biome) {
                source
                    .write_plans(chunk, &plans, &mut touched, &surface, self.air)
                    .expect("Nether jigsaw source validated at construction");
            }
        }
    }
}
