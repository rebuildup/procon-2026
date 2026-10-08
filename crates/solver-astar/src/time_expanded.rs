//! Time-expanded A* search over a `(step, position, fuel)` graph.
//!
//! Spec anchor: Issue #27 — time-expanded A* variant. Each node
//! represents an agent at a position at a given step. Edges connect
//! nodes whose positions are reachable in one step and whose fuel (when
//! [`TimeExpandedAStar::fuel_aware`] is enabled) permits the move. The
//! search runs in a 3D state space:
//!
//! - **step** — the time index (0..=`max_steps`),
//! - **position** — the hex cell the agent occupies,
//! - **fuel** — the fuel remaining after the move (only tracked when
//!   `fuel_aware` is enabled).
//!
//! Differences from the canonical [`crate::AstarSolver`]:
//!
//! - Costs are tracked as `f64` so weighted step costs from the active
//!   [`crate::CostPolicy`] can be summed without saturating overflow.
//! - The open set keys on `(position, step, fuel)` rather than
//!   `(position, fuel)`, which makes the search strictly monotonic in
//!   `step`. Re-expansion across time is permitted but only strictly
//!   improving paths reach the goal.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::Solver;
use crate::cost::{BaseFuelCost, CostModel, CostPolicy, DefaultCostModel};
use crate::hex::{CellKind, Direction, HexCoord};
use crate::map::HexMap;
use crate::stub::{Action, Path, SolveContext, SolverFailure, SolverResult};

/// A single node in the time-expanded search graph.
///
/// Identifies an agent's state at a given step. `g_cost` is the
/// accumulated weighted step cost to reach this node; `h_cost` is the
/// heuristic estimate from this node to the goal. `parent` records the
/// predecessor's `(position, step)` so callers can recover the path of
/// positions in chronological order. The parent's fuel is recoverable
/// from the chain because each transition's fuel cost is deterministic
/// given the destination cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeExpandedNode {
    /// Cell the agent occupies at this step.
    pub position: HexCoord,
    /// Time index (0-based; `0` is the starting step).
    pub step: u32,
    /// Accumulated weighted step cost from the start to this node.
    pub g_cost: f64,
    /// Heuristic estimate from this node to the goal.
    pub h_cost: f64,
    /// Predecessor's `(position, step)`. `None` only for the root node.
    pub parent: Option<(HexCoord, u32)>,
}

impl TimeExpandedNode {
    /// f = g + h.
    #[must_use]
    pub fn f_cost(&self) -> f64 {
        self.g_cost + self.h_cost
    }

    /// Build the root node at `position` with the given heuristic.
    #[must_use]
    pub fn root(position: HexCoord, h_cost: f64) -> Self {
        Self { position, step: 0, g_cost: 0.0, h_cost, parent: None }
    }
}

/// Configuration for the time-expanded A* search.
///
/// `max_steps` caps the time dimension of the search graph. The solver
/// refuses to expand any node whose `step` equals or exceeds
/// `max_steps`. When `fuel_aware` is `true`, edges that would exceed
/// the available fuel are dropped; otherwise the search collapses to a
/// pure `(step, position)` graph and the fuel counter is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeExpandedAStar {
    /// Hard upper bound on the number of time steps.
    pub max_steps: u32,
    /// Whether to enforce fuel constraints on transitions.
    pub fuel_aware: bool,
}

impl TimeExpandedAStar {
    /// Build a configuration.
    #[must_use]
    pub const fn new(max_steps: u32, fuel_aware: bool) -> Self {
        Self { max_steps, fuel_aware }
    }

    /// Maximum step budget.
    #[must_use]
    pub const fn max_steps(self) -> u32 {
        self.max_steps
    }

    /// Whether fuel is tracked.
    pub const fn fuel_aware(self) -> bool {
        self.fuel_aware
    }

    /// Run the search and return the bookkeeping needed to reconstruct
    /// the path. Exposed for the test suite; production callers should
    /// use [`TimeExpandedSolver`] which wraps this in a
    /// [`SolverResult`].
    pub fn search(self, ctx: &SolveContext) -> TimeExpandedResult {
        let model = DefaultCostModel;
        let policy = CostPolicy::default();

        let mut open: BinaryHeap<OpenEntry> = BinaryHeap::new();
        let mut best_g: HashMap<(HexCoord, u32, u32), f64> = HashMap::new();
        let mut came_from: HashMap<(HexCoord, u32, u32), (HexCoord, u32, u32)> = HashMap::new();
        let mut goal_state: Option<(HexCoord, u32, u32)> = None;
        let mut expanded: usize = 0;
        let mut generated: usize = 0;

        let start_h = hex_distance(ctx.start, ctx.goal);
        open.push(OpenEntry::new(ctx.start, 0, 0.0, f64::from(start_h), ctx.initial_fuel));
        best_g.insert((ctx.start, 0, ctx.initial_fuel), 0.0);
        generated += 1;

        while let Some(entry) = open.pop() {
            let key = (entry.coord, entry.step, entry.fuel);
            if let Some(&known) = best_g.get(&key) {
                if known < entry.g {
                    // Strictly worse entry — skip.
                    continue;
                }
            }

            if entry.coord == ctx.goal {
                goal_state = Some(key);
                expanded += 1;
                break;
            }
            expanded += 1;

            if entry.step >= self.max_steps {
                continue;
            }

            self.expand(
                entry,
                key,
                ctx,
                model,
                &policy,
                &mut open,
                &mut best_g,
                &mut came_from,
                &mut generated,
            );
        }

        TimeExpandedResult { goal_state, came_from, expanded, generated }
    }

    #[allow(clippy::too_many_arguments)]
    fn expand(
        self,
        entry: OpenEntry,
        key: (HexCoord, u32, u32),
        ctx: &SolveContext,
        model: DefaultCostModel,
        policy: &CostPolicy,
        open: &mut BinaryHeap<OpenEntry>,
        best_g: &mut HashMap<(HexCoord, u32, u32), f64>,
        came_from: &mut HashMap<(HexCoord, u32, u32), (HexCoord, u32, u32)>,
        generated: &mut usize,
    ) {
        for neighbor in ctx.map.neighbors(entry.coord, |cell| {
            if self.fuel_aware {
                model.allows(cell, entry.fuel, policy)
            } else {
                cell.is_passable()
            }
        }) {
            let Some(cell) = ctx.map.get(neighbor) else { continue };

            let (g_after, h_after, next_fuel) = if self.fuel_aware {
                let Some(fuel_cost) = model.fuel_cost(cell) else { continue };
                let Some(next_fuel) = entry.fuel.checked_sub(fuel_cost) else { continue };
                let step_weight = model.weighted_cost(cell, entry.fuel, policy).unwrap_or(u32::MAX);
                let g_after = entry.g + f64::from(step_weight);
                let h_after = f64::from(hex_distance(neighbor, ctx.goal));
                (g_after, h_after, next_fuel)
            } else {
                // Time-expanded but fuel-blind: every move costs 1 step.
                let g_after = entry.g + 1.0;
                let h_after = f64::from(hex_distance(neighbor, ctx.goal));
                (g_after, h_after, entry.fuel)
            };

            let next_step = entry.step + 1;
            let new_key = (neighbor, next_step, next_fuel);
            if let Some(&known) = best_g.get(&new_key) {
                if known <= g_after {
                    continue;
                }
            }
            best_g.insert(new_key, g_after);
            came_from.insert(new_key, key);
            open.push(OpenEntry::new(neighbor, next_step, g_after, h_after, next_fuel));
            *generated += 1;
        }
    }
}

/// Result of a time-expanded A* run.
///
/// Exposed so callers (the test suite, mostly) can inspect the search
/// graph. The solver itself goes through [`TimeExpandedSolver`] which
/// wraps this into a [`SolverResult`].
#[derive(Debug)]
pub struct TimeExpandedResult {
    /// `(position, step, fuel)` of the popped goal state, if any.
    pub goal_state: Option<(HexCoord, u32, u32)>,
    /// `state -> predecessor state` map for path reconstruction.
    pub came_from: HashMap<(HexCoord, u32, u32), (HexCoord, u32, u32)>,
    /// Cells popped from the open set.
    pub expanded: usize,
    /// Cells pushed onto the open set.
    pub generated: usize,
}

impl TimeExpandedResult {
    /// Reconstruct the `(position, step)` path from start to goal.
    ///
    /// Returns `None` if the search did not reach the goal. The first
    /// entry is the start; the last is the goal state.
    #[must_use]
    pub fn reconstruct_path(&self) -> Option<Vec<(HexCoord, u32)>> {
        let goal = self.goal_state?;
        let mut path = vec![(goal.0, goal.1)];
        let mut cur = goal;
        // Walk until we find the root (no predecessor).
        let mut safety = 0u32;
        while let Some(&prev) = self.came_from.get(&cur) {
            path.push((prev.0, prev.1));
            if prev.1 == 0 {
                break;
            }
            cur = prev;
            safety += 1;
            if safety > u32::try_from(self.came_from.len()).unwrap_or(u32::MAX) + 2 {
                break;
            }
        }
        path.reverse();
        Some(path)
    }
}

/// `Solver`-compatible wrapper around [`TimeExpandedAStar`].
///
/// Implements [`crate::Solver`] so it can be plugged into the same
/// coordinator dispatch as [`crate::AstarSolver`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeExpandedSolver {
    config: TimeExpandedAStar,
}

impl TimeExpandedSolver {
    /// Build a solver with the given configuration.
    #[must_use]
    pub const fn new(max_steps: u32, fuel_aware: bool) -> Self {
        Self { config: TimeExpandedAStar::new(max_steps, fuel_aware) }
    }

    /// Borrow the underlying [`TimeExpandedAStar`].
    #[must_use]
    pub const fn config(&self) -> &TimeExpandedAStar {
        &self.config
    }

    /// Run the search and return the raw result.
    #[must_use]
    pub fn run(self, ctx: &SolveContext) -> TimeExpandedResult {
        self.config.search(ctx)
    }
}

impl Solver for TimeExpandedSolver {
    fn name(&self) -> &'static str {
        "time-expanded-astar"
    }

    fn solve(&self, ctx: &SolveContext) -> SolverResult {
        let result = self.config.search(ctx);
        let path = result.goal_state.map(|goal_state| {
            let positions = reconstruct_positions(&result, goal_state);
            let mut steps = Vec::with_capacity(positions.len().saturating_sub(1));
            for window in positions.windows(2) {
                let from = window[0];
                let to = window[1];
                let action = direction_between(from, to).map_or(Action::Wait, Action::Move);
                steps.push(action);
            }
            Path { steps }
        });

        let failure = if path.is_none() {
            let min_cost = hex_distance(ctx.start, ctx.goal);
            Some(diagnose_failure(ctx, min_cost))
        } else {
            None
        };

        SolverResult { path, expanded: result.expanded, failure }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Open-set entry. The heap is a min-heap on `f`, with `g` and `h` kept
/// for tie-breaking and lazy-deletion checks.
#[derive(Debug, Clone, Copy)]
struct OpenEntry {
    /// f = g + h.
    f: f64,
    /// Weighted cost so far.
    g: f64,
    /// Heuristic estimate to the goal.
    h: f64,
    coord: HexCoord,
    step: u32,
    fuel: u32,
}

impl OpenEntry {
    fn new(coord: HexCoord, step: u32, g: f64, h: f64, fuel: u32) -> Self {
        Self { f: g + h, g, h, coord, step, fuel }
    }
}

impl PartialEq for OpenEntry {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f
            && self.g == other.g
            && self.h == other.h
            && self.coord == other.coord
            && self.step == other.step
            && self.fuel == other.fuel
    }
}
impl Eq for OpenEntry {}
impl Ord for OpenEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap: invert f so smaller f sorts larger in the natural
        // order. `BinaryHeap` is a max-heap.
        //
        // NaN handling: push NaN-laden entries to the bottom so a
        // poisoned state never blocks progress.
        match other.f.partial_cmp(&self.f) {
            Some(Ordering::Equal) | None => {}
            Some(o) => return o,
        }
        match other.g.partial_cmp(&self.g) {
            Some(Ordering::Equal) | None => {}
            Some(o) => return o,
        }
        // Tie-break by lower h, then lower coord, then lower step,
        // then lower fuel. All four are deterministic and `Ord`.
        self.h
            .partial_cmp(&other.h)
            .unwrap_or(Ordering::Equal)
            .then_with(|| self.coord.cmp(&other.coord))
            .then_with(|| self.step.cmp(&other.step))
            .then_with(|| self.fuel.cmp(&other.fuel))
    }
}
impl PartialOrd for OpenEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Hex grid distance in steps. Mirrors the private copy in
/// `crate::solver` so this module stays self-contained.
#[must_use]
fn hex_distance(a: HexCoord, b: HexCoord) -> u32 {
    let (ac, ar) = (a.col, a.row);
    let (bc, br) = (b.col, b.row);
    let ax = ac - ar.div_euclid(2);
    let ay = -ax - ar;
    let bx = bc - br.div_euclid(2);
    let by = -bx - br;
    let dx = ax - bx;
    let dy = ay - by;
    let dz = -dx - dy;
    (dx.unsigned_abs() + dy.unsigned_abs() + dz.unsigned_abs()) / 2
}

/// Reconstruct the position sequence from the goal back to the start.
fn reconstruct_positions(
    result: &TimeExpandedResult,
    goal_state: (HexCoord, u32, u32),
) -> Vec<HexCoord> {
    let mut positions = Vec::new();
    positions.push(goal_state.0);
    let mut cur = goal_state;
    let mut safety = 0u32;
    while let Some(&prev) = result.came_from.get(&cur) {
        positions.push(prev.0);
        if prev.1 == 0 {
            break;
        }
        cur = prev;
        safety += 1;
        if safety > u32::try_from(result.came_from.len()).unwrap_or(u32::MAX) + 2 {
            break;
        }
    }
    positions.reverse();
    positions
}

/// Resolve `(from, to)` to the [`Direction`] that links them, if any.
fn direction_between(from: HexCoord, to: HexCoord) -> Option<Direction> {
    Direction::ALL.iter().copied().find(|d| d.step(from) == to)
}

/// Decide which [`SolverFailure`] to report when no path was found.
///
/// **Unit note (Issue #304):** callers pass `hex_distance` — a *move count*
/// — while `ctx.max_steps` is the official terrain-weighted budget since
/// #302. The two are not the same unit: every move costs at least one
/// official step, so this comparison can only *under*-report an overrun (it
/// fires when even the cheapest geometry cannot fit, and `min_cost` states
/// the distance rather than the official cost of the terrain it crosses).
/// It is diagnostics only — no path selection reads it — and
/// `crate::solver::estimate_min_steps` is the official-unit estimator that
/// fixed the same mismatch in `diagnose` / `diagnose_multi`. Adopting it
/// here would be a behaviour-preserving test/label change; deliberately
/// left out of #304's scope.
fn diagnose_failure(ctx: &SolveContext, min_cost: u32) -> SolverFailure {
    if min_cost > ctx.max_steps {
        SolverFailure::ExceedsStepBudget { min_cost }
    } else {
        // Lower-bound fuel check (BFS over the actual map).
        let min_fuel = estimate_min_fuel(&ctx.map, ctx.start, ctx.goal);
        if min_fuel > ctx.initial_fuel {
            SolverFailure::OutOfFuel { min_fuel }
        } else {
            SolverFailure::Unreachable
        }
    }
}

/// Lower-bound fuel needed to walk a shortest path from `start` to
/// `goal`, using BFS and the actual cell kinds. Returns `u32::MAX` if no
/// path exists at all.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f_cost_is_g_plus_h() {
        let n = TimeExpandedNode {
            position: HexCoord::new(0, 0),
            step: 2,
            g_cost: 3.0,
            h_cost: 4.0,
            parent: None,
        };
        assert!((n.f_cost() - 7.0).abs() < f64::EPSILON);
    }

    #[test]
    fn root_has_zero_g_cost_and_no_parent() {
        let n = TimeExpandedNode::root(HexCoord::new(2, 3), 5.0);
        assert_eq!(n.position, HexCoord::new(2, 3));
        assert_eq!(n.step, 0);
        assert!(n.g_cost.abs() < f64::EPSILON);
        assert!(n.parent.is_none());
    }
}
