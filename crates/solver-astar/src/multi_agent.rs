//! Multi-agent coordination over a shared reservation table (Issue #26).
//!
//! Extends the single-agent [`super::AstarSolver`] with the ability to plan
//! for several agents acting on the **same** map / day. The agents move in
//! lock-step — one step is one tick of the day — so the shared state we
//! must defend against is two agents occupying the same `(coord, time)`
//! cell. We do that with a [`ReservationTable`] keyed by
//! `(HexCoord, u32)` (where the `u32` is the step index, zero-based).
//!
//! Strategy: **prioritised sequential planning**. We plan agents in the
//! order supplied by the caller, each one running A* over a *time-
//! augmented* state space. After an agent's path is found, every
//! `(coord, t)` the agent visits (including its starting cell at `t = 0`)
//! is recorded in the table; the next agent's A* treats those cells at
//! those times as impassable. This is the textbook "space-time A* with
//! reservation table" pattern described in the canonical multi-agent
//! path-finding literature.
//!
//! Why sequential rather than joint-action search (like
//! `solver-mcts::multi_agent`)? A* is optimal under a fixed objective;
//! enumerating joint actions would explode the search space and lose the
//! f-cost optimality. Per-agent reservations give us conflict-free plans
//! with provably optimal single-agent sub-paths.
//!
//! When an agent cannot reach its goal after every previously-planned
//! agent's cells are blocked, the resulting plan records the failure in
//! the per-agent [`SolverResult`] without short-circuiting the rest of
//! the roster — the caller decides what to do with partial plans.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::cost::{BaseStepCost, CostModel, CostPolicy, DefaultCostModel};
use crate::hex::{Direction, HexCoord};
use crate::solver::{AstarSolver, SolverStats};
use crate::stub::{Action, Path, SolveContext, SolverFailure, SolverResult};

/// Pluggable heuristic for the multi-agent search.
///
/// Mirrors the standard A* `h(s, g) -> lower_bound_on_cost(s, g)`
/// signature. The default — supplied via [`DEFAULT_HEURISTIC`] — is
/// the hex-distance function used internally by the single-agent
/// solver; it is admissible on this grid.
///
/// Function pointer (rather than a boxed trait object) keeps the
/// planner cheap to copy and lets the user pick from a fixed menu at
/// the call site.
pub type HeuristicFn = fn(HexCoord, HexCoord) -> u32;

/// Stable identifier for an agent inside the multi-agent planner.
///
/// Mirrors the `AgentId(pub u8)` shape used by sibling solver crates
/// (`solver-greedy`, `solver-ilp-cross`, `solver-mcts`). The crate-local
/// definition keeps this module independent until the canonical
/// `domain::AgentId` lands via issue #13.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct AgentId(pub u8);

impl AgentId {
    /// Construct an agent id from a raw byte.
    #[must_use]
    pub const fn new(v: u8) -> Self {
        Self(v)
    }

    /// Raw byte value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl core::fmt::Display for AgentId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Agent({})", self.0)
    }
}

/// Default admissible heuristic used by [`MultiAgentAStar`]: hex-grid
/// distance (cube-coordinate metric divided by two).
///
/// Provided as a function pointer so callers can plug it into
/// [`MultiAgentAStar::with_heuristic`].
#[must_use]
pub fn hex_distance(a: HexCoord, b: HexCoord) -> u32 {
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

/// The crate-wide default heuristic. Equal to [`hex_distance`].
pub const DEFAULT_HEURISTIC: HeuristicFn = hex_distance;

/// Space-time reservation table.
///
/// Each entry records which agent occupies `(coord, time)`. A second
/// agent's A* will treat that cell at that time as impassable.
#[derive(Debug, Default, Clone)]
pub struct ReservationTable {
    /// `(coord, time) -> reserving agent`.
    reserved: HashMap<(HexCoord, u32), AgentId>,
}

impl ReservationTable {
    /// Build an empty reservation table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of `(coord, time)` pairs currently reserved.
    #[must_use]
    pub fn len(&self) -> usize {
        self.reserved.len()
    }

    /// `true` when the table holds no reservations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.reserved.is_empty()
    }

    /// Whether `(coord, time)` is **not** claimed by any other agent.
    ///
    /// Returning `true` does not imply the planner will actually enter
    /// the cell — this is the gate used during neighbour expansion.
    #[must_use]
    pub fn is_available(&self, coord: HexCoord, time: u32) -> bool {
        !self.reserved.contains_key(&(coord, time))
    }

    /// Atomically check availability and claim `(coord, time)` for
    /// `agent_id`.
    ///
    /// Returns `true` when the reservation was made (or already owned
    /// by `agent_id` — re-reserving your own slot is a no-op). Returns
    /// `false` when a *different* agent has already reserved that
    /// `(coord, time)` slot.
    pub fn reserve(&mut self, coord: HexCoord, time: u32, agent_id: AgentId) -> bool {
        match self.reserved.get(&(coord, time)) {
            None => {
                self.reserved.insert((coord, time), agent_id);
                true
            }
            Some(existing) if *existing == agent_id => true,
            Some(_) => false,
        }
    }

    /// Lookup the reserving agent for `(coord, time)`, if any.
    #[must_use]
    pub fn reserver(&self, coord: HexCoord, time: u32) -> Option<AgentId> {
        self.reserved.get(&(coord, time)).copied()
    }
}

/// One agent's plan slot in the combined multi-agent output.
#[derive(Debug, Clone)]
pub struct AgentPlan {
    /// Agent this plan belongs to.
    pub agent: AgentId,
    /// Outcome of the per-agent A* run.
    pub result: SolverResult,
}

impl AgentPlan {
    /// Borrow the agent's path, if any.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.result.path.as_ref()
    }

    /// `true` when the planner produced a non-empty path for this agent.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.result.path.is_some()
    }
}

/// Combined output of a [`MultiAgentAStar::plan`] call.
///
/// One entry per requested agent, in the same order as the input
/// roster. Partial failures are preserved; callers inspect each
/// [`AgentPlan`] individually.
#[derive(Debug, Clone, Default)]
pub struct MultiAgentPlan {
    /// Per-agent plan slots.
    pub plans: Vec<AgentPlan>,
}

impl MultiAgentPlan {
    /// `true` when every agent reached its goal.
    #[must_use]
    pub fn all_succeeded(&self) -> bool {
        self.plans.iter().all(AgentPlan::succeeded)
    }

    /// Total cells expanded across every per-agent A* run.
    #[must_use]
    pub fn total_expanded(&self) -> usize {
        self.plans.iter().map(|p| p.result.expanded).sum()
    }

    /// Aggregate solver stats (expanded counts summed).
    #[must_use]
    pub fn stats(&self) -> SolverStats {
        SolverStats { expanded: self.total_expanded(), generated: 0 }
    }
}

/// Multi-agent A* planner.
///
/// Plans each agent in roster order. After every successful agent, the
/// cells it occupied at every visited step are written into the shared
/// [`ReservationTable`]; subsequent agents avoid those slots.
#[derive(Debug, Clone)]
pub struct MultiAgentAStar {
    /// Roster of agents the planner is responsible for.
    pub agents: Vec<AgentId>,
    /// Heuristic function used for all per-agent A* runs.
    pub heuristic: HeuristicFn,
    /// Policy knobs forwarded to every per-agent A* call.
    pub policy: CostPolicy,
}

impl Default for MultiAgentAStar {
    fn default() -> Self {
        Self { agents: Vec::new(), heuristic: DEFAULT_HEURISTIC, policy: CostPolicy::default() }
    }
}

impl MultiAgentAStar {
    /// Build a planner with explicit agent roster and the default
    /// heuristic.
    #[must_use]
    pub fn new(agents: Vec<AgentId>) -> Self {
        Self { agents, heuristic: DEFAULT_HEURISTIC, policy: CostPolicy::default() }
    }

    /// Build a planner with an explicit heuristic.
    #[must_use]
    pub fn with_heuristic(mut self, h: HeuristicFn) -> Self {
        self.heuristic = h;
        self
    }

    /// Build a planner with an explicit cost policy.
    #[must_use]
    pub fn with_policy(mut self, p: CostPolicy) -> Self {
        self.policy = p;
        self
    }

    /// Run the prioritised sequential planner.
    ///
    /// `tasks[i]` is the [`SolveContext`] for `agents[i]`. The two
    /// vectors must have the same length; otherwise the function
    /// returns an empty plan.
    pub fn plan(&self, tasks: &[SolveContext]) -> MultiAgentPlan {
        if tasks.len() != self.agents.len() {
            return MultiAgentPlan::default();
        }

        let mut table = ReservationTable::new();
        let mut plans: Vec<AgentPlan> = Vec::with_capacity(self.agents.len());

        for (agent, ctx) in self.agents.iter().copied().zip(tasks.iter()) {
            let result = self.plan_one(agent, ctx, &table);
            let succeeded = result.path.is_some();
            let path_for_reservation = result.path.clone();
            plans.push(AgentPlan { agent, result });

            // Only commit reservations on success — a failed agent
            // shouldn't permanently block cells it never actually
            // occupied. The `positions()` helper returns one entry per
            // step *and* the start, so we walk both.
            if succeeded {
                if let Some(path) = path_for_reservation.as_ref() {
                    let positions = path.positions(ctx.start);
                    for (t, coord) in positions.iter().enumerate() {
                        // `t` is the *arrival* time at `coord`; for the
                        // start cell that is 0.
                        let t_u32 = u32::try_from(t).unwrap_or(u32::MAX);
                        let _ = table.reserve(*coord, t_u32, agent);
                    }
                }
            }
        }

        MultiAgentPlan { plans }
    }

    /// Run the A* for a single agent, treating every `(coord, time)`
    /// pair in `table` as impassable for the *other* agent slots. The
    /// agent's own future reservations do not yet exist when this is
    /// called — they are committed by [`Self::plan`] after the fact.
    fn plan_one(
        &self,
        agent: AgentId,
        ctx: &SolveContext,
        table: &ReservationTable,
    ) -> SolverResult {
        let model = DefaultCostModel;
        let solver = AstarSolver::new(crate::solver::AstarSolverConfig { policy: self.policy });
        // Reuse the single-agent solver for the trivial empty-table
        // case so we exercise exactly the same code path the issue
        // acceptance criterion points at.
        if table.is_empty() {
            return solver.solve(ctx);
        }

        // Space-time state key: `(coord, time, fuel)`. `time` here is
        // the number of ticks elapsed since the start, so the start
        // cell lives at `t = 0`. Fuel tracking is preserved so the
        // search remains semantically equivalent to the single-agent
        // A* when the table is empty (defensive — should be a no-op in
        // practice because the empty-table branch above short-
        // circuits).
        let mut open: BinaryHeap<MultiOpenEntry> = BinaryHeap::new();
        let mut best_g: HashMap<(HexCoord, u32, u32), u32> = HashMap::new();
        let mut came_from: HashMap<(HexCoord, u32, u32), (HexCoord, u32, u32)> = HashMap::new();
        let mut goal_state: Option<(HexCoord, u32, u32)> = None;
        let mut expanded_count: usize = 0;
        let mut generated_count: usize = 0;

        let start_h = (self.heuristic)(ctx.start, ctx.goal);
        let start_f = start_h;
        open.push(MultiOpenEntry {
            f: start_f,
            g: 0,
            steps: 0,
            h: start_h,
            coord: ctx.start,
            time: 0,
            fuel: ctx.initial_fuel,
        });
        generated_count += 1;
        best_g.insert((ctx.start, 0, ctx.initial_fuel), 0);

        while let Some(entry) = open.pop() {
            if best_g
                .get(&(entry.coord, entry.time, entry.fuel))
                .is_some_and(|&known| known < entry.g)
            {
                continue;
            }

            if entry.coord == ctx.goal {
                goal_state = Some((entry.coord, entry.time, entry.fuel));
                expanded_count += 1;
                break;
            }
            expanded_count += 1;

            // Budget check in unweighted steps: `f` also carries the fuel
            // scaling, which orders the heap rather than measuring the day.
            if entry.steps.saturating_add(entry.h) > ctx.max_steps {
                continue;
            }

            for neighbor in
                ctx.map.neighbors(entry.coord, |cell| model.allows(cell, entry.fuel, &self.policy))
            {
                let Some(cell) = ctx.map.get(neighbor) else { continue };
                let Some(fuel_cost) = model.fuel_cost(cell) else { continue };
                let Some(next_fuel) = entry.fuel.checked_sub(fuel_cost) else { continue };
                let Some(step_weight) = model.weighted_cost(cell, entry.fuel, &self.policy) else {
                    continue;
                };

                let next_time = entry.time + 1;

                // Reject neighbours whose (coord, time) is already
                // claimed by another agent.
                if let Some(holder) = table.reserver(neighbor, next_time) {
                    if holder != agent {
                        continue;
                    }
                }

                let g_after = entry.g.saturating_add(step_weight);
                let h_after = (self.heuristic)(neighbor, ctx.goal);
                let f_after = g_after.saturating_add(h_after);
                // Budget in official units, billing the cell the move
                // departs — see the identical note in `crate::solver`.
                let origin_cost =
                    ctx.map.get(entry.coord).and_then(BaseStepCost::for_cell).map_or(0, |c| c.0);
                let steps_after = entry.steps.saturating_add(origin_cost);
                if steps_after.saturating_add(h_after) > ctx.max_steps {
                    continue;
                }
                let state_key = (neighbor, next_time, next_fuel);
                if best_g.get(&state_key).is_some_and(|&known| known <= g_after) {
                    continue;
                }
                best_g.insert(state_key, g_after);
                came_from.insert(state_key, (entry.coord, entry.time, entry.fuel));
                open.push(MultiOpenEntry {
                    f: f_after,
                    g: g_after,
                    steps: steps_after,
                    h: h_after,
                    coord: neighbor,
                    time: next_time,
                    fuel: next_fuel,
                });
                generated_count += 1;
            }
        }

        // Reconstruct path from goal back to start. We never generate
        // Wait actions in this loop, so all transitions are hex moves.
        let path = goal_state.map(|state| {
            let mut reverse_actions: Vec<Action> = Vec::new();
            let mut cur = state;
            while cur != (ctx.start, 0, ctx.initial_fuel) {
                let Some(prev) = came_from.get(&cur).copied() else { break };
                reverse_actions.push(action_between_hex(prev.0, cur.0));
                cur = prev;
            }
            reverse_actions.reverse();
            Path { steps: reverse_actions }
        });

        let failure = if path.is_none() {
            Some(diagnose_multi(ctx, (self.heuristic)(ctx.start, ctx.goal)))
        } else {
            None
        };

        let _ = generated_count;
        SolverResult { path, expanded: expanded_count, failure }
    }
}

/// Single-agent wrapper that consults an optional [`ReservationTable`].
///
/// When `table` is `None`, the solver delegates directly to
/// [`AstarSolver`] — preserving the "single-agent falls through to base
/// A* unchanged" acceptance criterion of Issue #26.
#[derive(Debug, Clone, Copy)]
pub struct MultiAgentSolver<'a> {
    /// Agent the solver plans for.
    pub agent: AgentId,
    /// Shared reservation table to consult during expansion.
    pub table: Option<&'a ReservationTable>,
    /// Cost policy forwarded to A*.
    pub policy: CostPolicy,
}

impl Default for MultiAgentSolver<'_> {
    fn default() -> Self {
        Self { agent: AgentId(0), table: None, policy: CostPolicy::default() }
    }
}

impl<'a> MultiAgentSolver<'a> {
    /// Build a single-agent solver with no reservation table (falls
    /// through to the base A*).
    #[must_use]
    pub fn new(agent: AgentId) -> Self {
        Self { agent, table: None, policy: CostPolicy::default() }
    }

    /// Borrow a custom reservation table.
    #[must_use]
    pub fn with_table(mut self, table: &'a ReservationTable) -> Self {
        self.table = Some(table);
        self
    }

    /// Override the cost policy.
    #[must_use]
    pub fn with_policy(mut self, policy: CostPolicy) -> Self {
        self.policy = policy;
        self
    }
}

impl Solver for MultiAgentSolver<'_> {
    fn name(&self) -> &'static str {
        "multi-agent-astar"
    }

    fn solve(&self, ctx: &SolveContext) -> SolverResult {
        match self.table {
            None => AstarSolver::new(crate::solver::AstarSolverConfig { policy: self.policy })
                .solve(ctx),
            Some(table) => {
                let planner = MultiAgentAStar {
                    agents: vec![self.agent],
                    heuristic: DEFAULT_HEURISTIC,
                    policy: self.policy,
                };
                planner.plan_one(self.agent, ctx, table)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Open-set entry for the space-time search.
#[derive(Debug, Clone, Copy)]
struct MultiOpenEntry {
    /// f = g + h.
    f: u32,
    /// Weighted cost so far.
    g: u32,
    /// **Unweighted** step cost so far — the unit `ctx.max_steps` is in.
    /// `g`/`f` carry the policy's fuel scaling; see `solver::OpenEntry`.
    steps: u32,
    /// Heuristic estimate to the goal.
    h: u32,
    coord: HexCoord,
    /// Step index since start.
    time: u32,
    fuel: u32,
}

impl PartialEq for MultiOpenEntry {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f
            && self.g == other.g
            && self.steps == other.steps
            && self.h == other.h
            && self.coord == other.coord
            && self.time == other.time
            && self.fuel == other.fuel
    }
}
impl Eq for MultiOpenEntry {}
impl Ord for MultiOpenEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap: invert f so smaller f sorts larger.
        other
            .f
            .cmp(&self.f)
            .then_with(|| other.g.cmp(&self.g))
            .then_with(|| other.steps.cmp(&self.steps))
            .then_with(|| self.h.cmp(&other.h))
            .then_with(|| self.coord.cmp(&other.coord))
            .then_with(|| self.time.cmp(&other.time))
            .then_with(|| self.fuel.cmp(&other.fuel))
    }
}
impl PartialOrd for MultiOpenEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Compute the action that goes from `from` to `to`. Assumes `to` is
/// exactly one neighbour of `from`. Falls back to `Wait` if no
/// neighbour matches, mirroring the single-agent solver's defensive
/// branch.
fn action_between_hex(from: HexCoord, to: HexCoord) -> Action {
    for dir in Direction::ALL {
        if dir.step(from) == to {
            return Action::Move(dir);
        }
    }
    Action::Wait
}

fn diagnose_multi(ctx: &SolveContext, min_cost: u32) -> SolverFailure {
    if min_cost > ctx.max_steps {
        SolverFailure::ExceedsStepBudget { min_cost }
    } else {
        SolverFailure::Unreachable
    }
}

// Re-export the parent-crate solver trait for downstream users.
pub use crate::solver::Solver;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hex::CellKind;
    use crate::map::HexMap;

    fn open_map() -> HexMap {
        HexMap::new(8, 8, vec![CellKind::Plain; 64]).unwrap()
    }

    #[test]
    fn reservation_table_starts_empty() {
        let t = ReservationTable::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        assert!(t.is_available(HexCoord::new(0, 0), 0));
    }

    #[test]
    fn reservation_table_round_trip() {
        let mut t = ReservationTable::new();
        assert!(t.reserve(HexCoord::new(1, 1), 0, AgentId(7)));
        assert_eq!(t.reserver(HexCoord::new(1, 1), 0), Some(AgentId(7)));
        assert!(!t.is_available(HexCoord::new(1, 1), 0));
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn reservation_table_rejects_double_claim() {
        let mut t = ReservationTable::new();
        assert!(t.reserve(HexCoord::new(2, 2), 3, AgentId(1)));
        assert!(!t.reserve(HexCoord::new(2, 2), 3, AgentId(2)));
        assert_eq!(t.reserver(HexCoord::new(2, 2), 3), Some(AgentId(1)));
    }

    #[test]
    fn reservation_table_allows_self_re_reservation() {
        let mut t = ReservationTable::new();
        assert!(t.reserve(HexCoord::new(0, 0), 0, AgentId(5)));
        assert!(t.reserve(HexCoord::new(0, 0), 0, AgentId(5)));
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn multi_agent_plan_handles_mismatched_lengths() {
        let planner = MultiAgentAStar::new(vec![AgentId(0), AgentId(1)]);
        let ctx = SolveContext::new(open_map(), HexCoord::new(0, 0), HexCoord::new(1, 0), 10, 20)
            .expect("valid context");
        let plan = planner.plan(&[ctx]);
        assert!(plan.plans.is_empty(), "mismatched inputs yield an empty plan");
    }

    #[test]
    fn multi_agent_plan_single_agent_matches_base_solver() {
        // Single agent: the multi-agent wrapper must reach the goal and
        // agree with `AstarSolver` on the step count.
        let map = open_map();
        let start = HexCoord::new(0, 4);
        let goal = HexCoord::new(7, 4);
        let ctx = SolveContext::new(map.clone(), start, goal, 100, 50).unwrap();

        let base = AstarSolver::default().solve(&ctx);
        let base_path = base.path.as_ref().expect("base path");

        let planner = MultiAgentAStar::new(vec![AgentId(42)]);
        let plan = planner.plan(std::slice::from_ref(&ctx));
        assert!(plan.all_succeeded());
        assert_eq!(plan.plans.len(), 1);
        let ours = plan.plans[0].path().expect("planner path");
        assert_eq!(ours.steps.len(), base_path.steps.len());
    }
}
