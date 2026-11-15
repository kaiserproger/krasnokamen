//! Mutation boundary for container signals. Read access cannot escape a mutable
//! map reference: every mutable inventory borrow, insertion/removal and viewer
//! transition records its position. Conservative mutable borrows are cheap;
//! signal recomputation still compares the resulting level before waking cells.

use std::collections::{BTreeMap, HashMap};
use std::ops::Deref;

use crate::boundary::{ChunkPos, EntityId};
use crate::tick::BlockEntityState;

type Cell = (i32, i32, i32);

#[derive(Debug, Default)]
struct DirtyPositions(Vec<Cell>);

impl DirtyPositions {
    fn mark_dirty(&mut self, cell: Cell) {
        self.0.push(cell);
    }

    fn drain_into(&mut self, cells: &mut Vec<Cell>) {
        cells.append(&mut self.0);
    }
}

/// Native container custody, including replay and chunk hydration. There is
/// deliberately no DerefMut/IndexMut or mutable iterator bypass.
#[derive(Debug, Default)]
pub struct BlockEntities {
    records: BTreeMap<Cell, BlockEntityState>,
    dirty: DirtyPositions,
    /// Journal consumer of the same custody marks (drained by the WAL writer).
    journal: Vec<Cell>,
    /// Independent 001-f consumer; hydration and catch-up never enter it.
    external: Vec<Cell>,
}

impl BlockEntities {
    pub fn iter(&self) -> impl Iterator<Item = (&Cell, &BlockEntityState)> {
        self.records.iter()
    }

    pub fn get_mut(&mut self, cell: &Cell) -> Option<&mut BlockEntityState> {
        let record = self.records.get_mut(cell)?;
        self.dirty.mark_dirty(*cell);
        self.journal.push(*cell);
        self.external.push(*cell);
        Some(record)
    }

    pub fn insert(&mut self, cell: Cell, record: BlockEntityState) -> Option<BlockEntityState> {
        self.touch(cell);
        self.records.insert(cell, record)
    }

    pub fn remove(&mut self, cell: &Cell) -> Option<BlockEntityState> {
        let record = self.records.remove(cell)?;
        self.touch(*cell);
        Some(record)
    }

    pub fn entry(
        &mut self,
        cell: Cell,
    ) -> std::collections::btree_map::Entry<'_, Cell, BlockEntityState> {
        self.touch(cell);
        self.records.entry(cell)
    }

    /// Hydration must not acquire mutable live custody on an occupied entry.
    pub(crate) fn hydrate(&mut self, cell: Cell, record: impl FnOnce() -> BlockEntityState) {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.records.entry(cell) {
            self.dirty.mark_dirty(cell);
            self.journal.push(cell);
            entry.insert(record());
        }
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&Cell, &BlockEntityState) -> bool) {
        self.records.retain(|cell, record| {
            if keep(cell, record) {
                true
            } else {
                self.dirty.mark_dirty(*cell);
                self.external.push(*cell);
                self.journal.push(*cell);
                false
            }
        });
    }

    /// Non-slot comparator metadata (crafter disabled slots) shares this fence.
    pub(crate) fn mark_dirty(&mut self, cell: Cell) {
        self.touch(cell);
    }

    /// Residency also affects a double-chest half just across a chunk border.
    /// Only the transitioning column and its one-block halo are invalidated.
    pub(crate) fn mark_chunk_dirty(&mut self, chunk: ChunkPos) {
        let x = chunk.x.saturating_mul(16);
        let z = chunk.z.saturating_mul(16);
        let zs = z.saturating_sub(1)..=z.saturating_add(16);
        for (&cell, _) in self.records.range(
            (x.saturating_sub(1), i32::MIN, i32::MIN)..=(x.saturating_add(16), i32::MAX, i32::MAX),
        ) {
            if zs.contains(&cell.2) {
                self.dirty.mark_dirty(cell);
            }
        }
    }

    pub(crate) fn drain_dirty_into(&mut self, cells: &mut Vec<Cell>) {
        self.dirty.drain_into(cells);
    }

    /// Cells whose record may differ from its last journaled image, sorted and
    /// deduplicated. Every live record is either journaled or listed here.
    pub(crate) fn take_journal_touched(&mut self) -> Vec<Cell> {
        let mut cells = std::mem::take(&mut self.journal);
        cells.sort_unstable();
        cells.dedup();
        cells
    }

    pub(crate) fn take_external_writes(&mut self) -> Vec<Cell> {
        std::mem::take(&mut self.external)
    }

    pub(crate) fn get_mut_catch_up(&mut self, cell: &Cell) -> Option<&mut BlockEntityState> {
        let record = self.records.get_mut(cell)?;
        self.dirty.mark_dirty(*cell);
        self.journal.push(*cell);
        Some(record)
    }

    fn touch(&mut self, cell: Cell) {
        self.dirty.mark_dirty(cell);
        self.journal.push(cell);
        self.external.push(cell);
    }
}

impl Deref for BlockEntities {
    type Target = BTreeMap<Cell, BlockEntityState>;

    fn deref(&self) -> &Self::Target {
        &self.records
    }
}

impl<'a> IntoIterator for &'a BlockEntities {
    type Item = (&'a Cell, &'a BlockEntityState);
    type IntoIter = std::collections::btree_map::Iter<'a, Cell, BlockEntityState>;

    fn into_iter(self) -> Self::IntoIter {
        self.records.iter()
    }
}

#[derive(Clone, Copy, Debug)]
struct ContainerOpener {
    cell: Cell,
    until: Option<u64>,
}

/// Menu and native openers share counts, invalidation and animation ownership.
#[derive(Debug, Default)]
pub struct OpenContainers {
    viewers: HashMap<EntityId, ContainerOpener>,
    counts: HashMap<Cell, usize>,
    native_count: usize,
    dirty: DirtyPositions,
    animation_dirty: Vec<Cell>,
}

impl OpenContainers {
    pub fn insert(&mut self, viewer: EntityId, cell: Cell) -> Option<Cell> {
        self.set(viewer, cell, None)
    }

    pub(crate) fn insert_native(&mut self, viewer: EntityId, cell: Cell, until: u64) {
        self.set(viewer, cell, Some(until));
    }

    fn set(&mut self, viewer: EntityId, cell: Cell, until: Option<u64>) -> Option<Cell> {
        let previous = self.viewers.insert(viewer, ContainerOpener { cell, until });
        if previous.is_some_and(|open| open.until.is_some()) {
            self.native_count -= 1;
        }
        self.native_count += usize::from(until.is_some());
        let previous_cell = previous.map(|open| open.cell);
        if previous_cell != Some(cell) {
            if let Some(previous) = previous_cell {
                Self::release_cell(
                    &mut self.counts,
                    &mut self.dirty,
                    &mut self.animation_dirty,
                    previous,
                );
            }
            *self.counts.entry(cell).or_default() += 1;
            self.dirty.mark_dirty(cell);
            self.animation_dirty.push(cell);
        }
        previous_cell
    }

    pub fn remove(&mut self, viewer: &EntityId) -> Option<Cell> {
        let open = self.viewers.remove(viewer)?;
        self.native_count -= usize::from(open.until.is_some());
        Self::release_cell(
            &mut self.counts,
            &mut self.dirty,
            &mut self.animation_dirty,
            open.cell,
        );
        Some(open.cell)
    }

    fn release_cell(
        counts: &mut HashMap<Cell, usize>,
        dirty: &mut DirtyPositions,
        animation_dirty: &mut Vec<Cell>,
        cell: Cell,
    ) {
        let count = counts.get_mut(&cell).expect("registered container opener");
        *count -= 1;
        if *count == 0 {
            counts.remove(&cell);
        }
        dirty.mark_dirty(cell);
        animation_dirty.push(cell);
    }

    pub fn clear(&mut self) {
        for &cell in self.counts.keys() {
            self.dirty.mark_dirty(cell);
            self.animation_dirty.push(cell);
        }
        self.viewers.clear();
        self.counts.clear();
        self.native_count = 0;
    }

    pub fn menu_at(&self, viewer: &EntityId) -> Option<&Cell> {
        self.viewers
            .get(viewer)
            .filter(|open| open.until.is_none())
            .map(|open| &open.cell)
    }

    pub fn menu_viewers(&self) -> impl Iterator<Item = (&EntityId, &Cell)> {
        self.viewers
            .iter()
            .filter(|(_, open)| open.until.is_none())
            .map(|(id, open)| (id, &open.cell))
    }

    pub(crate) fn viewers(&self) -> impl Iterator<Item = (&EntityId, &Cell)> {
        self.viewers.iter().map(|(id, open)| (id, &open.cell))
    }

    pub fn contains(&self, viewer: &EntityId) -> bool {
        self.viewers.contains_key(viewer)
    }

    pub fn is_empty(&self) -> bool {
        self.viewers.is_empty()
    }

    pub(crate) fn native_open(&self, viewer: EntityId) -> bool {
        self.viewers
            .get(&viewer)
            .is_some_and(|open| open.until.is_some())
    }

    pub(crate) fn count_at(&self, cell: Cell, partner: Option<Cell>) -> usize {
        self.counts.get(&cell).copied().unwrap_or(0)
            + partner
                .and_then(|cell| self.counts.get(&cell))
                .copied()
                .unwrap_or(0)
    }

    pub(crate) fn retain_native(&mut self, mut keep: impl FnMut(EntityId, Cell, u64) -> bool) {
        if self.native_count == 0 {
            return;
        }
        let Self {
            viewers,
            counts,
            native_count,
            dirty,
            animation_dirty,
        } = self;
        viewers.retain(|&id, open| {
            if let Some(until) = open.until
                && !keep(id, open.cell, until)
            {
                *native_count -= 1;
                Self::release_cell(counts, dirty, animation_dirty, open.cell);
                return false;
            }
            true
        });
    }

    pub(crate) fn drain_dirty_into(&mut self, cells: &mut Vec<Cell>) {
        self.dirty.drain_into(cells);
    }

    pub(crate) fn take_animation_dirty(&mut self) -> Vec<Cell> {
        std::mem::take(&mut self.animation_dirty)
    }

    pub(crate) fn recycle_animation_dirty(&mut self, mut cells: Vec<Cell>) {
        cells.clear();
        debug_assert!(self.animation_dirty.is_empty());
        self.animation_dirty = cells;
    }
}
