// Bounded cardinal A* over already received, walkable terrain only.
// A path is a local movement plan, never a server-arrival or pickup receipt.
const FARMER_PATH_NODE_CAP: usize = 8192;
const FARMER_PATH_RADIUS: i32 = 96;
const FARMER_REPLAN_CAP: u32 = 64;
// Allow modest farmer movement without discarding a useful route. This is
// planning hysteresis, not the stricter one-block arrival/drop predicate.
const FARMER_GOAL_HYSTERESIS: f64 = 4.0;

#[derive(Clone, serde::Serialize)]
struct FarmerSearchStats {
    start: [f64; 3],
    target_last_received: [f64; 3],
    target_column: Option<FarmerColumnEvidence>,
    expanded: usize,
    discovered: usize,
    frontier: usize,
    unknown_edges: u64,
    blocked_edges: u64,
    height_edges: u64,
    radius_edges: u64,
    best_cell: [i32; 2],
    best_ground: f64,
    best_heuristic: u32,
}

#[derive(Clone, serde::Serialize)]
struct FarmerColumnEvidence {
    feet_y: i16,
    top_state: i32,
    support_height: u16,
    walkable: bool,
}

struct FarmerPath {
    route: Vec<[i32; 3]>,
    stats: FarmerSearchStats,
}

#[derive(Clone)]
struct FarmerSearchFailure {
    reason: &'static str,
    stats: FarmerSearchStats,
}
impl std::fmt::Debug for FarmerSearchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FarmerSearchFailure").field("reason", &self.reason).finish()
    }
}
impl std::fmt::Display for FarmerSearchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (expanded={}, discovered={}, frontier={}, best={:?})",
            self.reason, self.stats.expanded, self.stats.discovered, self.stats.frontier, self.stats.best_cell)
    }
}
impl std::error::Error for FarmerSearchFailure {}

#[derive(Clone, Copy)]
struct FarmerSearchNode {
    parent: Option<(i32, i32)>,
    cost: u32,
    ground: f64,
}

fn farmer_heuristic(x: i32, z: i32, y: f64, target: [f64; 3]) -> u32 {
    let dx = ((f64::from(x) + 0.5 - target[0]).abs() - 1.0).max(0.0).ceil() as u32;
    let dz = ((f64::from(z) + 0.5 - target[2]).abs() - 1.0).max(0.0).ceil() as u32;
    let dy = (((y - target[1]).abs() - 1.0).max(0.0) / 1.01).ceil() as u32;
    dx.saturating_add(dz).max(dy)
}

impl Bot {
    fn farmer_missing_footprint(&self, x: f64, z: f64) -> bool {
        ((x - 0.3).floor() as i32..=(x + 0.3).floor() as i32).any(|bx|
            ((z - 0.3).floor() as i32..=(z + 0.3).floor() as i32)
                .any(|bz| self.column(bx,bz).is_none()))
    }

    // Reuse is only a movement plan, never evidence of arrival. Recheck the
    // complete remaining route against the current received terrain.
    fn farmer_retained_route(&self, route: &[[i32; 3]], waypoint: usize,
        target: [f64; 3]) -> std::result::Result<(), &'static str> {
        let remaining = route.get(waypoint..).ok_or("previous waypoint missing")?;
        if remaining.len() < 2 { return Err("previous route has no remaining edge"); }
        let first = remaining[0];
        if (self.x - (f64::from(first[0]) + 0.5)).abs() >= 0.001
            || (self.z - (f64::from(first[2]) + 0.5)).abs() >= 0.001
            || (self.y - f64::from(first[1])).abs() >= 0.1 {
            return Err("actor is not centered on previous route");
        }
        let mut ground = self.ground_y(self.x,self.z).ok_or("previous route support blocked or unknown")?;
        if (ground - self.y).abs() > 0.1 { return Err("previous route support changed"); }
        let start_heuristic = farmer_heuristic(first[0],first[2],ground,target);
        for edge in remaining.windows(2) {
            let [from,to] = [edge[0],edge[1]];
            if (i64::from(from[0]) - i64::from(to[0])).abs()
                + (i64::from(from[2]) - i64::from(to[2])).abs() != 1 {
                return Err("previous route has a noncardinal edge");
            }
            let next = self.ground_y(f64::from(to[0]) + 0.5,f64::from(to[2]) + 0.5)
                .ok_or("previous route support blocked or unknown")?;
            if (next - f64::from(to[1])).abs() >= 0.1 || (next.ceil() - next) >= 0.1
                || (next - ground).abs() > 1.01 {
                return Err("previous route height changed");
            }
            let edge_ground = self.ground_y((f64::from(from[0]) + f64::from(to[0])) * 0.5 + 0.5,
                (f64::from(from[2]) + f64::from(to[2])) * 0.5 + 0.5)
                .ok_or("previous route edge blocked or unknown")?;
            if (edge_ground - ground).abs() > 1.01 || (edge_ground - next).abs() > 1.01 {
                return Err("previous route edge height changed");
            }
            ground = next;
        }
        let last = remaining[remaining.len() - 1];
        let start_distance = (self.x - target[0]).powi(2) + (self.y - target[1]).powi(2)
            + (self.z - target[2]).powi(2);
        let end_distance = (f64::from(last[0]) + 0.5 - target[0]).powi(2)
            + (ground - target[1]).powi(2) + (f64::from(last[2]) + 0.5 - target[2]).powi(2);
        if farmer_heuristic(last[0],last[2],ground,target) >= start_heuristic
            || end_distance >= start_distance {
            return Err("previous route does not progress toward current farmer position");
        }
        Ok(())
    }

    fn farmer_route_hysteresis(&self, route: &[[i32; 3]], waypoint: usize,
        target: [f64; 3]) -> std::result::Result<(), &'static str> {
        self.farmer_retained_route(route, waypoint, target)?;
        let last = *route.last().ok_or("previous route is empty")?;
        let ground = self.ground_y(f64::from(last[0]) + 0.5, f64::from(last[2]) + 0.5)
            .ok_or("previous route endpoint blocked or unknown")?;
        if (f64::from(last[0]) + 0.5 - target[0]).abs() > FARMER_GOAL_HYSTERESIS
            || (ground - target[1]).abs() > FARMER_GOAL_HYSTERESIS
            || (f64::from(last[2]) + 0.5 - target[2]).abs() > FARMER_GOAL_HYSTERESIS {
            return Err("farmer materially displaced from route endpoint");
        }
        Ok(())
    }

    fn farmer_path(&self, target: [f64; 3]) -> Result<FarmerPath> {
        self.pursuit_path(target, None)
    }

    fn source_approach_path(&self, point: [i32; 3]) -> Result<FarmerPath> {
        self.pursuit_path([f64::from(point[0]) + 0.5, f64::from(point[1]),
            f64::from(point[2]) + 0.5], Some(point))
    }

    fn pursuit_path(&self, target: [f64; 3], exact: Option<[i32; 3]>) -> Result<FarmerPath> {
        ensure!(target.iter().all(|coordinate| coordinate.is_finite()), "nonfinite farmer position");
        let start = (self.x.floor() as i32, self.z.floor() as i32);
        let start_y = self.ground_y(f64::from(start.0) + 0.5, f64::from(start.1) + 0.5)
            .context("farmer pursuit starting support unknown or blocked")?;
        ensure!((start_y - self.y).abs() <= 0.1, "farmer pursuit actor is not on observed support");
        ensure!((target[0] - self.x).abs() <= f64::from(FARMER_PATH_RADIUS)
            && (target[2] - self.z).abs() <= f64::from(FARMER_PATH_RADIUS), "farmer outside pursuit radius");
        let initial_h = farmer_heuristic(start.0, start.1, start_y, target);
        let target_column = self.column(target[0].floor() as i32, target[2].floor() as i32).map(|column|
            FarmerColumnEvidence { feet_y:column.feet_y, top_state:column.top,
                support_height:self.facts.get(column.top).support_height, walkable:column.walkable });
        let mut stats = FarmerSearchStats { start: [self.x,self.y,self.z], target_last_received: target, target_column,
            expanded:0, discovered:1, frontier:1, unknown_edges:0, blocked_edges:0,
            height_edges:0, radius_edges:0, best_cell:[start.0,start.1], best_ground:start_y,
            best_heuristic:initial_h };
        let mut nodes = HashMap::new();
        let mut frontier = std::collections::BinaryHeap::new();
        nodes.insert(start, FarmerSearchNode { parent:None, cost:0, ground:start_y });
        frontier.push(std::cmp::Reverse((initial_h, initial_h, 0_u32, start.0, start.1)));
        let mut capped = false;
        let mut goal = None;
        while let Some(std::cmp::Reverse((_, heuristic, cost, x, z))) = frontier.pop() {
            let Some(node) = nodes.get(&(x,z)).copied() else { continue; };
            if node.cost != cost { continue; }
            if stats.expanded == FARMER_PATH_NODE_CAP { capped = true; break; }
            stats.expanded += 1;
            if heuristic < stats.best_heuristic {
                stats.best_heuristic = heuristic;
                stats.best_cell = [x,z];
                stats.best_ground = node.ground;
            }
            let reached = if let Some(point) = exact {
                x == point[0] && z == point[2] && (node.ground - f64::from(point[1])).abs() < 0.1
            } else {
                (f64::from(x) + 0.5 - target[0]).abs() <= 1.0
                    && (f64::from(z) + 0.5 - target[2]).abs() <= 1.0
                    && (node.ground - target[1]).abs() <= 1.0
            };
            if reached {
                goal = Some((x,z));
                break;
            }
            for (dx,dz) in [(1,0),(0,1),(-1,0),(0,-1)] {
                let Some(nx) = x.checked_add(dx) else { continue; };
                let Some(nz) = z.checked_add(dz) else { continue; };
                if (i64::from(nx) - i64::from(start.0)).abs() > i64::from(FARMER_PATH_RADIUS)
                    || (i64::from(nz) - i64::from(start.1)).abs() > i64::from(FARMER_PATH_RADIUS) {
                    stats.radius_edges += 1;
                    continue;
                }
                let next_cost = cost + 1;
                if nodes.get(&(nx,nz)).is_some_and(|old| old.cost <= next_cost) { continue; }
                let px = f64::from(nx) + 0.5;
                let pz = f64::from(nz) + 0.5;
                let Some(next_y) = self.ground_y(px,pz) else {
                    if self.farmer_missing_footprint(px,pz) { stats.unknown_edges += 1; }
                    else { stats.blocked_edges += 1; }
                    continue;
                };
                if (next_y.ceil() - next_y) >= 0.1 || (next_y - node.ground).abs() > 1.01 {
                    stats.height_edges += 1;
                    continue;
                }
                let edge_x = (f64::from(x) + f64::from(nx)) * 0.5 + 0.5;
                let edge_z = (f64::from(z) + f64::from(nz)) * 0.5 + 0.5;
                let Some(edge_y) = self.ground_y(edge_x,edge_z) else {
                    if self.farmer_missing_footprint(edge_x,edge_z) { stats.unknown_edges += 1; }
                    else { stats.blocked_edges += 1; }
                    continue;
                };
                if (edge_y - node.ground).abs() > 1.01 || (edge_y - next_y).abs() > 1.01 {
                    stats.height_edges += 1;
                    continue;
                }
                if !nodes.contains_key(&(nx,nz)) && nodes.len() == FARMER_PATH_NODE_CAP {
                    capped = true;
                    continue;
                }
                let next_h = farmer_heuristic(nx,nz,next_y,target);
                nodes.insert((nx,nz), FarmerSearchNode { parent:Some((x,z)), cost:next_cost, ground:next_y });
                frontier.push(std::cmp::Reverse((next_cost.saturating_add(next_h),next_h,next_cost,nx,nz)));
            }
        }
        stats.discovered = nodes.len();
        stats.frontier = frontier.len();
        let Some(mut cursor) = goal else {
            return Err(FarmerSearchFailure { reason: if capped { "farmer pursuit node cap reached" }
                else { "farmer unreachable through received walkable terrain" }, stats }.into());
        };
        let mut route = Vec::new();
        loop {
            let column = self.column(cursor.0,cursor.1).context("pursuit route column disappeared")?;
            route.push([cursor.0, i32::from(column.feet_y), cursor.1]);
            match nodes.get(&cursor).and_then(|node| node.parent) {
                Some(previous) => cursor = previous,
                None => break,
            }
        }
        route.reverse();
        Ok(FarmerPath { route, stats })
    }
}
