//! Selection-only whole-tree evidence. Never scanned by the per-cell executor.
use super::*;

const RANGE: i32 = 6;
const LOG: u8 = 1;
const OWNED: u8 = 2;
const UNKNOWN: u8 = 4;

pub(super) struct Proof {
    min: [i32; 3],
    size: [usize; 3],
    distance: Vec<u8>,
    logs: Vec<u8>,
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
    /// leaves, the second detects every foreign log that could support them.
    pub(super) fn new(state: &TickState, root: [i32; 3]) -> Self {
        #[cfg(test)]
        BUILDS.with(|count| count.set(count.get() + 1));
        let Some(component) = component(state, root).filter(|logs| !logs.is_empty()) else {
            return Self {
                min: root,
                size: [0; 3],
                distance: Vec::new(),
                logs: Vec::new(),
            };
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
        for at in component {
            cells[proof.index(at).expect("component inside proof")] |= OWNED;
        }
        let mut queue = VecDeque::new();
        // Two nibbles hold distances to foreign and owned logs respectively.
        // Multi-source fields are built once, not a 13³ scan for every leaf.
        for shift in [0, 4] {
            for (index, &cell) in cells.iter().enumerate() {
                let source = if shift == 0 {
                    cell & UNKNOWN != 0 || cell & (LOG | OWNED) == LOG
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
        // ForkingTrunkPlacer advances Y and one horizontal axis together.
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
                    if tree(state, block) && !seen[index(next)] {
                        seen[index(next)] = true;
                        logs.push(next);
                    }
                }
            }
        }
    }
    Some(logs)
}
