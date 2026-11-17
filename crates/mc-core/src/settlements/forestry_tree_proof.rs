//! Selection-only whole-tree evidence. Never scanned by the per-cell executor.
//!
//! N5: a leaf is foreign only when a log it could belong to belongs to *no*
//! non-ambiguous registered component. Touching registered trees are therefore
//! one grove: the leaves between their trunks are permitted, and a contributing
//! tree whose own log stands within [`RANGE`] of a certified cell is felled
//! whole instead of being left mutilated. Ownership and permission are still
//! measured from *this* root's own logs only, so a union never widens one tree's
//! crown claim to a neighbour's.
use super::*;

pub(super) const RANGE: i32 = 6;
const LOG: u8 = 1;
const OWNED: u8 = 2;
const UNKNOWN: u8 = 4;
/// A log of some non-ambiguous registered component inside this box.
const UNION: u8 = 8;

/// One component walk per registered root per scan, shared by every proof and
/// by the whole-tree contributor checks, so a grove costs one walk per tree.
/// Only roots whose halo crosses a proof box are ever walked.
#[derive(Default)]
pub(super) struct Grove {
    components: HashMap<[i32; 3], Option<Vec<[i32; 3]>>>,
}

impl Grove {
    /// `None` when the component is ambiguous (nothing may be owned) or when
    /// the root is not a tree log at all.
    pub(super) fn component(&mut self, state: &TickState, root: [i32; 3]) -> Option<&[[i32; 3]]> {
        self.components
            .entry(root)
            .or_insert_with(|| component(state, root));
        self.components
            .get(&root)
            .expect("memoized component")
            .as_deref()
            .filter(|logs| !logs.is_empty())
    }
}

pub(super) struct Proof {
    min: [i32; 3],
    size: [usize; 3],
    distance: Vec<u8>,
    logs: Vec<u8>,
    /// Registered roots (this one included) with at least one component log
    /// inside this box: the only trees a certified cell can touch.
    pub(super) contributors: Vec<[i32; 3]>,
}

#[cfg(test)]
thread_local! {
    pub(super) static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Proof {
    fn index(&self, at: [i32; 3]) -> Option<usize> {
        let local = std::array::from_fn::<_, 3, _>(|axis| at[axis] - self.min[axis]);
        local
            .iter()
            .enumerate()
            .all(|(axis, &v)| v >= 0 && (v as usize) < self.size[axis])
            .then(|| {
                (local[1] as usize * self.size[0] + local[0] as usize) * self.size[2]
                    + local[2] as usize
            })
    }

    fn position(&self, index: usize) -> [i32; 3] {
        [
            self.min[0] + (index / self.size[2] % self.size[0]) as i32,
            self.min[1] + (index / (self.size[0] * self.size[2])) as i32,
            self.min[2] + (index % self.size[2]) as i32,
        ]
    }

    fn max(&self) -> [i32; 3] {
        std::array::from_fn(|axis| self.min[axis] + self.size[axis] as i32 - 1)
    }

    /// Further than [`RANGE`] from every log that belongs to no registered
    /// component, and within [`RANGE`] of one of this root's own logs. Every
    /// permitted cell keeps a full [`RANGE`] of read environment inside the box,
    /// because the box reaches `2 * RANGE` above the connected log top.
    pub(super) fn permits(&self, at: [i32; 3]) -> bool {
        self.index(at).is_some_and(|index| {
            self.distance[index] & 15 > RANGE as u8 && self.distance[index] >> 4 <= RANGE as u8
        })
    }

    pub(super) fn owns_log(&self, at: [i32; 3]) -> bool {
        self.index(at)
            .is_some_and(|index| self.logs.get(index).is_some_and(|cell| cell & OWNED != 0))
    }

    pub(super) fn positions(&self, root: [i32; 3]) -> impl Iterator<Item = [i32; 3]> {
        let radius = PLANTED_CROWN_RADIUS as i32;
        let top = (root[1] + 64).min(self.min[1] + self.size[1] as i32 - 1);
        (root[1]..=top).flat_map(move |y| {
            (-radius..=radius).flat_map(move |dx| {
                (-radius..=radius).map(move |dz| [root[0] + dx, y, root[2] + dz])
            })
        })
    }

    /// Once per cached root: own only non-stripped tree logs inside the halo.
    /// Cap at the connected log top + 2*RANGE: the first range reaches owned
    /// leaves, the second detects every log that could support them.
    pub(super) fn new(state: &TickState, root: [i32; 3], grove: &mut Grove) -> Self {
        #[cfg(test)]
        BUILDS.with(|count| count.set(count.get() + 1));
        let Some(component) = grove.component(state, root).map(<[[i32; 3]]>::to_vec) else {
            return Self::empty(root);
        };
        let top = component
            .iter()
            .map(|at| at[1])
            .max()
            .expect("nonempty component");
        let radius = PLANTED_CROWN_RADIUS as i32;
        let min = [
            root[0] - radius - RANGE,
            (root[1] - RANGE).max(state.world.geometry().min_y()),
            root[2] - radius - RANGE,
        ];
        let max = [
            root[0] + radius + RANGE,
            (top + 2 * RANGE)
                .min(root[1] + 64 + RANGE)
                .min(state.world.geometry().max_y() - 1),
            root[2] + radius + RANGE,
        ];
        let size = std::array::from_fn(|axis| (max[axis] - min[axis] + 1) as usize);
        let len = size.iter().product();
        let mut proof = Self {
            min,
            size,
            distance: vec![0x77; len],
            logs: Vec::new(),
            contributors: Vec::new(),
        };
        let mut cells = vec![0; len];
        for (index, cell) in cells.iter_mut().enumerate() {
            let [x, y, z] = proof.position(index);
            let Some(id) = state.world.physics_block_at(x, y, z) else {
                *cell = UNKNOWN;
                continue;
            };
            let Some(&log) = state.registry.tree_logs.get(id as usize) else {
                *cell = UNKNOWN;
                continue;
            };
            if log {
                *cell |= LOG;
            }
        }
        for at in &component {
            cells[proof.index(*at).expect("component inside proof")] |= OWNED;
        }
        // A log of another non-ambiguous registered component stops being a
        // foreign source, so leaves between the two trunks are permitted.
        // Ambiguous neighbours stay foreign: nothing is assumed about them.
        let roots: Vec<[i32; 3]> = state
            .settlements
            .records()
            .flat_map(|record| record.planted_trees.iter().copied())
            .collect();
        let [min_x, min_y, min_z] = proof.min;
        let [max_x, max_y, max_z] = proof.max();
        for other in roots {
            let crown = PLANTED_CROWN_RADIUS as i32;
            if other[0] + crown < min_x
                || other[0] - crown > max_x
                || other[2] + crown < min_z
                || other[2] - crown > max_z
                || other[1] > max_y
                || other[1] + 64 < min_y
            {
                continue;
            }
            let Some(logs) = grove.component(state, other).map(<[[i32; 3]]>::to_vec) else {
                continue;
            };
            let mut marked = false;
            for at in logs {
                if let Some(index) = proof.index(at) {
                    if cells[index] & LOG != 0 {
                        cells[index] |= UNION;
                        marked = true;
                    }
                }
            }
            if marked {
                proof.contributors.push(other);
            }
        }
        let mut queue = VecDeque::new();
        // Two nibbles hold distances to foreign and own logs respectively.
        // Multi-source fields are built once, not a 13³ scan for every leaf.
        for shift in [0, 4] {
            for (index, &cell) in cells.iter().enumerate() {
                let source = if shift == 0 {
                    cell & UNKNOWN != 0 || (cell & LOG != 0 && cell & UNION == 0)
                } else {
                    cell & OWNED != 0
                };
                if source {
                    proof.distance[index] &= !(15 << shift);
                    queue.push_back(index);
                }
            }
            proof.spread(&mut queue, shift);
        }
        proof.logs = cells;
        proof
    }

    fn empty(root: [i32; 3]) -> Self {
        Self {
            min: root,
            size: [0; 3],
            distance: Vec::new(),
            logs: Vec::new(),
            contributors: Vec::new(),
        }
    }

    fn spread(&mut self, queue: &mut VecDeque<usize>, shift: u8) {
        while let Some(index) = queue.pop_front() {
            let distance = ((self.distance[index] >> shift) & 15) + 1;
            if distance > RANGE as u8 {
                continue;
            }
            let at = self.position(index);
            for axis in 0..3 {
                for step in [-1, 1] {
                    let mut next = at;
                    next[axis] += step;
                    let Some(neighbor) = self.index(next) else {
                        continue;
                    };
                    if (self.distance[neighbor] >> shift) & 15 > distance {
                        self.distance[neighbor] =
                            (self.distance[neighbor] & !(15 << shift)) | (distance << shift);
                        queue.push_back(neighbor);
                    }
                }
            }
        }
    }
}

/// Discover the connected component before allocating the shorter distance box.
/// A fixed halo bitmap avoids repeated reads and per-neighbour allocations.
fn component(state: &TickState, root: [i32; 3]) -> Option<Vec<[i32; 3]>> {
    const WIDTH: usize = PLANTED_CROWN_RADIUS as usize * 2 + 1;
    const HEIGHT: usize = 65;
    let index = |at: [i32; 3]| {
        ((at[1] - root[1]) as usize * WIDTH
            + (at[0] - root[0] + PLANTED_CROWN_RADIUS as i32) as usize)
            * WIDTH
            + (at[2] - root[2] + PLANTED_CROWN_RADIUS as i32) as usize
    };
    let block = state.world.physics_block_at(root[0], root[1], root[2])?;
    if !state.registry.tree_logs.get(block as usize).copied()? || !tree(state, block) {
        return Some(Vec::new());
    }
    let mut seen = [false; WIDTH * WIDTH * HEIGHT];
    let mut logs = vec![root];
    seen[index(root)] = true;
    let mut cursor = 0;
    while cursor < logs.len() {
        let at = logs[cursor];
        cursor += 1;
        let below = state.world.physics_block_at(at[0], at[1] - 1, at[2])?;
        if (at[0], at[2]) != (root[0], root[2])
            && (state.registry.tree_soil.get(below as usize).copied()?
                || (state.registry.materials.classify(below).is_solid() && !tree(state, below)))
        {
            return None;
        }
        // ForkingTrunkPlacer advances Y and one horizontal axis together. A
        // second horizontal axis in the same step is not a shape any sapling of
        // this tree grows (mc-world plant_rules), so it stays foreign.
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                for dz in -1i32..=1 {
                    if dx.abs() + dz.abs() > 1 || (dy == 0 && dx == 0 && dz == 0) {
                        continue;
                    }
                    let next = [at[0] + dx, at[1] + dy, at[2] + dz];
                    let block = state.world.physics_block_at(next[0], next[1], next[2])?;
                    if !state.registry.tree_logs.get(block as usize).copied()? {
                        continue;
                    }
                    // A log beyond the ownership envelope is evidence of a
                    // larger/foreign component, never a silent truncation.
                    if !in_halo(root, next) {
                        return None;
                    }
                    if !tree(state, block) || seen[index(next)] {
                        continue;
                    }
                    // N5c: a retained player edit is never part of a tree, so
                    // an attributable player log keeps the whole component.
                    if state.block_log.player_touched(next) {
                        return None;
                    }
                    seen[index(next)] = true;
                    logs.push(next);
                }
            }
        }
    }
    Some(logs)
}
