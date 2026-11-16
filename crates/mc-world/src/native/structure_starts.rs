use super::{NativeCheckpointReader, NativeError, codec, db};
use crate::{ChunkPos, chunk::GeneratedStructureSite};
use mc_nbt::Tag;

impl NativeCheckpointReader {
    /// Read one candidate's coverage and starts from the pinned snapshot,
    /// without scanning unrelated chunks or expanding block palettes.
    /// `None` is uncovered; `Some([])` authoritatively suppresses generation.
    pub fn structure_starts_at(
        &self,
        dimension: u32,
        pos: ChunkPos,
        skip_referenced: bool,
    ) -> Result<Option<Vec<GeneratedStructureSite>>, NativeError> {
        let key = super::chunk_key(dimension, pos);
        let metadata = db(self.tx.open_table(super::METADATA))?;
        let imported = db(self.tx.open_table(super::IMPORTED))?;
        let bytes = match db(metadata.get(key.as_slice()))? {
            Some(bytes) => Some(bytes),
            None => db(imported.get(key.as_slice()))?,
        };
        if let Some(bytes) = bytes {
            let root = codec::decode(bytes.value())?;
            let Tag::Compound(fields) = root else {
                return Err(NativeError::Invalid("structure metadata root"));
            };
            return crate::structure_starts::read(&fields, pos, skip_referenced)
                .map(Some)
                .map_err(NativeError::Source);
        }
        if db(db(self.tx.open_table(super::APPLIED))?.get(key.as_slice()))?.is_some() {
            return Ok(Some(Vec::new()));
        }
        let overlays = db(self.tx.open_table(super::OVERLAYS))?;
        let covered = db(overlays.range(key.as_slice()..))?
            .next()
            .transpose()
            .map_err(|error| NativeError::Database(error.into()))?
            .is_some_and(|(stored, _)| stored.value().starts_with(&key));
        Ok(covered.then(Vec::new))
    }
}
