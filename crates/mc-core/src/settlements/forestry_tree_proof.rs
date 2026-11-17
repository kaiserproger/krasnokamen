//! Selection-only whole-tree evidence. Never scanned by the per-cell executor.
use super::*;

const RANGE: i32 = 6;
const LOG: u8 = 1;
const OWNED: u8 = 2;
const UNKNOWN: u8 = 4;
const SOIL: u8 = 8;

pub(super) struct Proof {
    min: [i32; 3],
    size: [usize; 3],
    distance: Vec<u8>,
    logs: Vec<u8>,
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

    /// At most 21×77×21 cells, once per relevant root per selection call.
    /// Six-face Manhattan distance conservatively includes every vanilla
    /// LeavesBlock propagation path, including paths through persistent leaves.
    pub(super) fn new(state: &TickState, root: [i32; 3]) -> Self {
        let radius = PLANTED_CROWN_RADIUS as i32;
        let min = [
            root[0] - radius - RANGE,
            (root[1] - RANGE).max(state.world.geometry().min_y()),
            root[2] - radius - RANGE,
        ];
        let max = [
            root[0] + radius + RANGE,
            (root[1] + 64 + RANGE).min(state.world.geometry().max_y() - 1),
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
            let (Some(&log), Some(&soil)) = (
                state.registry.tree_logs.get(id as usize),
                state.registry.tree_soil.get(id as usize),
            ) else {
                *cell = UNKNOWN;
                continue;
            };
            if log {
                *cell |= LOG;
            }
            if soil {
                *cell |= SOIL;
            }
        }
        let mut queue = VecDeque::new();
        if proof.connect(&mut cells, root, &mut queue) {
            proof.distance.fill(0);
            return proof;
        }
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

    fn connect(&self, cells: &mut [u8], root: [i32; 3], queue: &mut VecDeque<usize>) -> bool {
        let root_index = self.index(root).expect("root inside its bounded proof");
        if cells[root_index] & LOG != 0 {
            cells[root_index] |= OWNED;
            queue.push_back(root_index);
        }
        let mut ambiguous = false;
        while let Some(index) = queue.pop_front() {
            let at = self.position(index);
            if let Some(below) = self.index([at[0], at[1] - 1, at[2]]) {
                ambiguous |= cells[below] & UNKNOWN != 0;
                if cells[below] & SOIL != 0 {
                    // Two ground-root columns cannot establish a single tree.
                    ambiguous |= (at[0], at[2]) != (root[0], root[2]);
                }
            }
            // ForkingTrunkPlacer advances Y and one horizontal axis together.
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    for dz in -1i32..=1 {
                        if dx.abs() + dz.abs() > 1 || (dy == 0 && dx == 0 && dz == 0) {
                            continue;
                        }
                        let next = [at[0] + dx, at[1] + dy, at[2] + dz];
                        if next[0].abs_diff(root[0]) > PLANTED_CROWN_RADIUS
                            || next[2].abs_diff(root[2]) > PLANTED_CROWN_RADIUS
                            || !(root[1]..=root[1] + 64).contains(&next[1])
                        {
                            continue;
                        }
                        let Some(neighbor) = self.index(next) else {
                            continue;
                        };
                        ambiguous |= cells[neighbor] & UNKNOWN != 0;
                        if cells[neighbor] & (LOG | OWNED) == LOG {
                            cells[neighbor] |= OWNED;
                            queue.push_back(neighbor);
                        }
                    }
                }
            }
        }
        ambiguous
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
