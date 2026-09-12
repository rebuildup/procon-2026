//! A* implementation and the (currently local) [`Solver`] trait.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::cost::{BaseFuelCost, CostModel, CostPolicy, DefaultCostModel};
use crate::hex::{CellKind, Direction, HexCoord};
use crate::map::HexMap;
use crate::stub::{Action, Path, SolveContext, SolverFailure, SolverResult};

/// Strategy contract. Issue #57 will publish a canonical version in
/// `crates/engine`; the local trait here is the minimum the A* solver
/// needs.
pub trait Solver {
    /// Human-readable identifier used in logs and benchmarks.
    fn name(&self) -> &'static str;

    /// Run the solver. Always returns a [`SolverResult`]; never panics on
    /// unsolvable input — that becomes `SolverFailure::Unreachable` or
    /// `SolverFailure::OutOfFuel`.
    fn solve(&self, ctx: &SolveContext) -> SolverResult;
}

/// Diagnostic counters returned by [`AstarSolver`] alongside a path.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SolverStats {
    /// Cells popped from the open set.
    pub expanded: usize,
    /// Cells pushed onto the open set (including reopens).
    pub generated: usize,
}

/// Configuration for [`AstarSolver`]. The [`CostPolicy`] is the pluggable
/// knob — see issue #25 acceptance criterion that policy must be
/// configurable for "variety 重視" vs "fuel 温存" strategies.
#[derive(Debug, Clone, Copy, Default)]
pub struct AstarSolverConfig {
    /// Policy knobs (fuel scaling, hard fuel constraint).
    pub policy: CostPolicy,
}

/// A* solver. Single agent, single goal.
#[derive(Debug, Clone, Copy)]
pub struct AstarSolver {
    config: AstarSolverConfig,
}

impl Default for AstarSolver {
    fn default() -> Self {
        Self::new(AstarSolverConfig::default())
    }
}

impl AstarSolver {
    /// Build a solver from a configuration.
    #[must_use]
    pub const fn new(config: AstarSolverConfig) -> Self {
        Self { config }
    }

    /// Build a solver with the default cost model and the given policy.
    #[must_use]
    pub const fn with_policy(policy: CostPolicy) -> Self {
        Self::new(AstarSolverConfig { policy })
    }

    /// Borrow the active policy.
    #[must_use]
    pub const fn policy(&self) -> &CostPolicy {
        &self.config.policy
    }

    /// Internal state for the search.
    fn search(&self, ctx: &SolveContext) -> (SolverResult, SolverStats) {
        let model = DefaultCostModel;
        let (open, best_g, came_from, goal_state, stats) = self.run_search(ctx, model);
        build_result(ctx, open, best_g, &came_from, goal_state, stats)
    }

    /// Pure A* loop. Returns the bookkeeping maps so the caller can format a
    /// result. Split out so the type-checker doesn't see `search` as a 100+
    /// line monster.
    #[allow(clippy::type_complexity)]
    fn run_search(
        &self,
        ctx: &SolveContext,
        model: DefaultCostModel,
    ) -> (
        BinaryHeap<OpenEntry>,
        HashMap<(HexCoord, u32), u32>,
        HashMap<(HexCoord, u32), (HexCoord, u32)>,
        Option<(HexCoord, u32)>,
        SolverStats,
    ) {
        let mut stats = SolverStats::default();

        let start_h = hex_distance(ctx.start, ctx.goal);
        let start_f = start_h;
        let mut open: BinaryHeap<OpenEntry> = BinaryHeap::new();
        open.push(OpenEntry {
            f: start_f,
            g: 0,
            h: start_h,
            coord: ctx.start,
            fuel: ctx.initial_fuel,
        });
        stats.generated += 1;

        let mut best_g: HashMap<(HexCoord, u32), u32> = HashMap::new();
        best_g.insert((ctx.start, ctx.initial_fuel), 0);

        let mut came_from: HashMap<(HexCoord, u32), (HexCoord, u32)> = HashMap::new();
        let mut goal_state: Option<(HexCoord, u32)> = None;

        while let Some(entry) = open.pop() {
            // Lazy deletion: drop entries superseded by a strictly better
            // g for the same (coord, fuel) pair.
            if best_g.get(&(entry.coord, entry.fuel)).is_some_and(|&known| known < entry.g) {
                continue;
            }

            if entry.coord == ctx.goal {
                // The heuristic is admissible and the edge costs are
                // monotonic, so the first goal pop is optimal.
                goal_state = Some((entry.coord, entry.fuel));
                stats.expanded += 1;
                break;
            }
            stats.expanded += 1;

            if entry.f > ctx.max_steps {
                continue;
            }

            self.expand(entry, ctx, model, &mut open, &mut best_g, &mut came_from, &mut stats);
        }

        (open, best_g, came_from, goal_state, stats)
    }

    /// Expand one open entry into its neighbours.
    #[allow(clippy::too_many_arguments)]
    fn expand(
        &self,
        entry: OpenEntry,
        ctx: &SolveContext,
        model: DefaultCostModel,
        open: &mut BinaryHeap<OpenEntry>,
        best_g: &mut HashMap<(HexCoord, u32), u32>,
        came_from: &mut HashMap<(HexCoord, u32), (HexCoord, u32)>,
        stats: &mut SolverStats,
    ) {
        for neighbor in ctx
            .map
            .neighbors(entry.coord, |cell| model.allows(cell, entry.fuel, &self.config.policy))
        {
            let Some(cell) = ctx.map.get(neighbor) else { continue };
            let Some(fuel_cost) = model.fuel_cost(cell) else { continue };
            let Some(next_fuel) = entry.fuel.checked_sub(fuel_cost) else { continue };
            let Some(step_weight) = model.weighted_cost(cell, entry.fuel, &self.config.policy)
            else {
                continue;
            };
            let g_after = entry.g.saturating_add(step_weight);
            let h_after = hex_distance(neighbor, ctx.goal);
            let f_after = g_after.saturating_add(h_after);
            if f_after > ctx.max_steps {
                continue;
            }
            let state_key = (neighbor, next_fuel);
            if best_g.get(&state_key).is_some_and(|&known| known <= g_after) {
                continue;
            }
            best_g.insert(state_key, g_after);
            came_from.insert(state_key, (entry.coord, entry.fuel));
            open.push(OpenEntry {
                f: f_after,
                g: g_after,
                h: h_after,
                coord: neighbor,
                fuel: next_fuel,
            });
            stats.generated += 1;
        }
    }
}

impl Solver for AstarSolver {
    fn name(&self) -> &'static str {
        "astar"
    }

    fn solve(&self, ctx: &SolveContext) -> SolverResult {
        let (result, _stats) = self.search(ctx);
        result
    }
}

/// Reconstruct the path / failure from the bookkeeping maps returned by
/// [`AstarSolver::run_search`].
#[allow(clippy::type_complexity)]
fn build_result(
    ctx: &SolveContext,
    _open: BinaryHeap<OpenEntry>,
    _best_g: HashMap<(HexCoord, u32), u32>,
    came_from: &HashMap<(HexCoord, u32), (HexCoord, u32)>,
    goal_state: Option<(HexCoord, u32)>,
    stats: SolverStats,
) -> (SolverResult, SolverStats) {
    let Some(state) = goal_state else {
        let min_cost = hex_distance(ctx.start, ctx.goal);
        let min_fuel = estimate_min_fuel(&ctx.map, ctx.start, ctx.goal);
        return (
            SolverResult {
                path: None,
                expanded: stats.expanded,
                failure: Some(diagnose(ctx, min_cost, min_fuel)),
            },
            stats,
        );
    };

    let mut reverse_actions: Vec<Action> = Vec::new();
    let mut cur = state;
    while cur != (ctx.start, ctx.initial_fuel) {
        let Some(prev) = came_from.get(&cur).copied() else { break };
        reverse_actions.push(action_between(prev.0, cur.0));
        cur = prev;
    }
    reverse_actions.reverse();

    (
        SolverResult {
            path: Some(Path { steps: reverse_actions }),
            expanded: stats.expanded,
            failure: None,
        },
        stats,
    )
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Open-set entry. The heap is a min-heap on `f`, with `g` and `h` kept for
/// tie-breaking and lazy-deletion checks.
#[derive(Debug, Clone, Copy)]
struct OpenEntry {
    /// f = g + h.
    f: u32,
    /// Weighted cost so far.
    g: u32,
    /// Heuristic estimate to the goal (kept for tie-breaking only).
    h: u32,
    coord: HexCoord,
    fuel: u32,
}

impl PartialEq for OpenEntry {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f
            && self.g == other.g
            && self.h == other.h
            && self.coord == other.coord
            && self.fuel == other.fuel
    }
}
impl Eq for OpenEntry {}
impl Ord for OpenEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap: invert f so smaller f sorts larger in the natural order.
        // (BinaryHeap is a max-heap.)
        other
            .f
            .cmp(&self.f)
            .then_with(|| other.g.cmp(&self.g))
            .then_with(|| self.h.cmp(&other.h))
            .then_with(|| self.coord.cmp(&other.coord))
            .then_with(|| self.fuel.cmp(&other.fuel))
    }
}
impl PartialOrd for OpenEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Hex grid distance in steps. Each step has weight >= 1, so the distance
/// is an admissible lower bound on the search cost.
///
/// Uses Red Blob Games's "even-r" cube-coordinates formula so the heuristic
/// matches the actual neighbour set.
#[must_use]
fn hex_distance(a: HexCoord, b: HexCoord) -> u32 {
    let (ac, ar) = (a.col, a.row);
    let (bc, br) = (b.col, b.row);
    // Offset -> cube for even-r.
    let ax = ac - ar.div_euclid(2);
    let ay = -ax - ar;
    let bx = bc - br.div_euclid(2);
    let by = -bx - br;
    let dx = ax - bx;
    let dy = ay - by;
    let dz = -dx - dy;
    (dx.unsigned_abs() + dy.unsigned_abs() + dz.unsigned_abs()) / 2
}

/// Lower-bound fuel needed to walk a shortest path from `start` to `goal`,
/// using BFS and the actual cell kinds. Returns `u32::MAX` if no path
/// exists at all.
fn estimate_min_fuel(map: &HexMap, start: HexCoord, goal: HexCoord) -> u32 {
    use std::collections::VecDeque;
    let mut visited = std::collections::HashSet::new();
    let mut queue: VecDeque<(HexCoord, u32)> = VecDeque::new();
    queue.push_back((start, 0));
    visited.insert(start);
    while let Some((coord, fuel)) = queue.pop_front() {
        if coord == goal {
            return fuel;
        }
        for next in map.neighbors(coord, CellKind::is_passable) {
            if !visited.insert(next) {
                continue;
            }
            let Some(cell) = map.get(next) else { continue };
            let Some(extra) = BaseFuelCost::for_cell(cell) else { continue };
            let next_fuel = fuel.saturating_add(extra.0);
            queue.push_back((next, next_fuel));
        }
    }
    u32::MAX
}

fn diagnose(ctx: &SolveContext, min_cost: u32, min_fuel: u32) -> SolverFailure {
    if min_cost > ctx.max_steps {
        SolverFailure::ExceedsStepBudget { min_cost }
    } else if min_fuel > ctx.initial_fuel {
        SolverFailure::OutOfFuel { min_fuel }
    } else {
        SolverFailure::Unreachable
    }
}

/// Compute the action that goes from `from` to `to`. Assumes `to` is exactly
/// one neighbour of `from` (the A* search only ever generates such
/// transitions).
fn action_between(from: HexCoord, to: HexCoord) -> Action {
    for dir in Direction::ALL {
        if dir.step(from) == to {
            return Action::Move(dir);
        }
    }
    // Should be unreachable; if A* generated it, it must be a neighbour.
    Action::Wait
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::HexMap;

    fn open_map() -> HexMap {
        HexMap::new(8, 8, vec![CellKind::Plain; 64]).unwrap()
    }

    #[test]
    fn hex_distance_zero_for_same_cell() {
        let a = HexCoord::new(3, 3);
        assert_eq!(hex_distance(a, a), 0);
    }

    #[test]
    fn hex_distance_matches_neighbour_count() {
        let a = HexCoord::new(4, 4);
        let b = HexCoord::new(6, 4);
        assert_eq!(hex_distance(a, b), 2);
    }

    #[test]
    fn hex_distance_across_rows() {
        let a = HexCoord::new(4, 4);
        let b = HexCoord::new(4, 7);
        // Even-r offset: 3 rows down + 0 column.
        assert_eq!(hex_distance(a, b), 3);
    }

    #[test]
    fn action_between_round_trip() {
        let from = HexCoord::new(4, 4);
        for dir in Direction::ALL {
            let to = dir.step(from);
            assert_eq!(action_between(from, to), Action::Move(dir));
        }
    }

    #[test]
    fn solver_returns_empty_when_already_at_goal() {
        let map = open_map();
        let ctx = SolveContext::new(map, HexCoord::new(3, 3), HexCoord::new(3, 3), 10, 50).unwrap();
        let solver = AstarSolver::default();
        let result = solver.solve(&ctx);
        let path = result.path.expect("path");
        assert!(path.steps.is_empty());
    }
}
