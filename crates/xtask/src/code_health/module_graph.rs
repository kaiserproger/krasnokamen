//! Strongly connected components of a module dependency graph.
//!
//! `edges[n]` lists the modules node `n` refers to. Node indices are stable,
//! so every result here is deterministic for a given graph.

/// Tarjan's algorithm. Each component lists its nodes in ascending order.
pub(super) fn strongly_connected(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct Walk<'a> {
        edges: &'a [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        components: Vec<Vec<usize>>,
    }

    fn connect(walk: &mut Walk<'_>, node: usize) {
        walk.index[node] = Some(walk.next);
        walk.low[node] = walk.next;
        walk.next += 1;
        walk.stack.push(node);
        walk.on_stack[node] = true;
        let edges = walk.edges;
        for &next in &edges[node] {
            match walk.index[next] {
                None => {
                    connect(walk, next);
                    walk.low[node] = walk.low[node].min(walk.low[next]);
                }
                Some(index) if walk.on_stack[next] => {
                    walk.low[node] = walk.low[node].min(index);
                }
                Some(_) => {}
            }
        }
        if walk.index[node] == Some(walk.low[node]) {
            let mut component = Vec::new();
            while let Some(member) = walk.stack.pop() {
                walk.on_stack[member] = false;
                component.push(member);
                if member == node {
                    break;
                }
            }
            component.sort_unstable();
            walk.components.push(component);
        }
    }

    let count = edges.len();
    let mut walk = Walk {
        edges,
        index: vec![None; count],
        low: vec![0; count],
        on_stack: vec![false; count],
        stack: Vec::new(),
        next: 0,
        components: Vec::new(),
    };
    for node in 0..count {
        if walk.index[node].is_none() {
            connect(&mut walk, node);
        }
    }
    walk.components
}

/// The largest component; ties go to the one with the smallest node index.
pub(super) fn largest_component(edges: &[Vec<usize>]) -> Vec<usize> {
    let mut components = strongly_connected(edges);
    components.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    components.into_iter().next().unwrap_or_default()
}

/// Edges from and to `node` that stay inside `members`.
pub(super) struct Degree {
    pub(super) node: usize,
    pub(super) incoming: Vec<usize>,
    pub(super) outgoing: Vec<usize>,
}

pub(super) fn degrees(edges: &[Vec<usize>], members: &[usize]) -> Vec<Degree> {
    let mut inside = vec![false; edges.len()];
    for &member in members {
        inside[member] = true;
    }
    let mut degrees: Vec<Degree> = members
        .iter()
        .map(|&node| Degree {
            node,
            incoming: Vec::new(),
            outgoing: edges[node]
                .iter()
                .copied()
                .filter(|&next| inside[next] && next != node)
                .collect(),
        })
        .collect();
    let pairs: Vec<(usize, usize)> = degrees
        .iter()
        .flat_map(|degree| degree.outgoing.iter().map(|&to| (degree.node, to)))
        .collect();
    for (from, to) in pairs {
        if let Ok(target) = members.binary_search(&to) {
            degrees[target].incoming.push(from);
        }
    }
    degrees
}

/// What it takes to move one module out of a cycle: cut every edge on its
/// cheaper side (all edges into it, or all edges out of it, inside the
/// cycle). `leaving` counts the modules that are no longer in the largest
/// remaining cycle afterwards, the module itself included.
pub(super) struct Hint {
    pub(super) node: usize,
    /// `(from, to)` edges to cut.
    pub(super) cut: Vec<(usize, usize)>,
    pub(super) leaving: usize,
}

/// Decoupling hints for the cycle `members` (ascending node indices), the
/// best first: most modules leaving per cut edge, then fewest cut edges.
pub(super) fn decoupling_hints(edges: &[Vec<usize>], members: &[usize]) -> Vec<Hint> {
    let mut hints: Vec<Hint> = degrees(edges, members)
        .into_iter()
        .map(|degree| {
            let node = degree.node;
            let cut = if degree.incoming.len() < degree.outgoing.len() {
                degree.incoming.iter().map(|&from| (from, node)).collect()
            } else {
                degree.outgoing.iter().map(|&to| (node, to)).collect()
            };
            let rest: Vec<usize> = members.iter().copied().filter(|&m| m != node).collect();
            // One module alone is no cycle (self-references are not edges).
            let remaining = match largest_component(&induced(edges, &rest)).len() {
                1 => 0,
                size => size,
            };
            Hint {
                node,
                cut,
                leaving: members.len() - remaining,
            }
        })
        .collect();
    hints.sort_by(|a, b| {
        (b.leaving * a.cut.len())
            .cmp(&(a.leaving * b.cut.len()))
            .then_with(|| a.cut.len().cmp(&b.cut.len()))
            .then_with(|| a.node.cmp(&b.node))
    });
    hints
}

/// The subgraph on `nodes` (ascending), renumbered `0..nodes.len()`.
fn induced(edges: &[Vec<usize>], nodes: &[usize]) -> Vec<Vec<usize>> {
    nodes
        .iter()
        .map(|&node| {
            edges[node]
                .iter()
                .filter_map(|next| nodes.binary_search(next).ok())
                .collect()
        })
        .collect()
}
