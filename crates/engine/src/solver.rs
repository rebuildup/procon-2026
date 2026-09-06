//! Shared solver contract.
//!
//! Every `solver-*` crate implements exactly one thing — [`Solver`] — and the
//! coordinator drives all of them through `dyn Solver` without knowing which
//! strategy sits behind the pointer. That keeps strategy selection (and the
//! "run several strategies, keep the best answer" pattern) a coordinator
//! concern instead of leaking into each solver.
//!
//! # Contest framing
//!
//! Spec: 募集要項「試合の進行」 — a match is played in **days**, and a team
//! submits **one answer per day** covering that day's steps for all of its
//! agents. Q&A Q3 gives each day a wall-clock budget (`daySeconds`), and Q&A Q6
//! splits a step into a 反映フェーズ / アクションフェーズ pair where an answer
//! that contains any invalid action is rejected *as a whole*.
//!
//! Three consequences are baked into the types here:
//!
//! - [`SolverContext`] carries the authoritative state snapshot, the remaining
//!   budget for the current day, and the caller's own team / agent identity.
//! - [`SolverDecision`] carries exactly one day's worth of [`Action`]s.
//! - A solver cannot fail. `decide` returns a decision, never a `Result`,
//!   because "no answer" loses the day outright; a solver that runs out of
//!   time must degrade to a valid fallback such as [`SolverDecision::wait_all`].

use std::time::Duration;

use domain::PlayerId;
use protocol::MatchSnapshot;
use serde::{Deserialize, Serialize};

/// Team identity as seen by a solver.
///
/// The contest model has no dedicated team type yet: a team *is* a player of
/// the match, so this aliases [`domain::PlayerId`]. Callers should spell the
/// alias rather than `PlayerId` so that introducing a real `TeamId` newtype
/// later is a one-line change here instead of an edit in every solver.
pub type TeamId = PlayerId;

/// Zero-based index of an agent inside a team.
///
/// Spec: 募集要項 — a team fields 3..=8 agents, so a byte is sufficient.
pub type AgentId = u8;

/// Movement direction on the hex grid.
///
/// Spec: 募集要項「移動と燃料」 — six neighbours under the even-row
/// right-offset layout fixed by Q&A Q1.
///
/// **Stub.** The canonical `Direction` / [`Action`] pair lands in `domain`
/// with issue #53; this module then re-exports those types and deletes the
/// local definitions. Field and variant shapes are chosen to match #53 so the
/// swap does not churn solver code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// `col + 1`, same row.
    East,
    /// One row up, to the right.
    NorthEast,
    /// One row up, to the left.
    NorthWest,
    /// `col - 1`, same row.
    West,
    /// One row down, to the left.
    SouthWest,
    /// One row down, to the right.
    SouthEast,
}

impl Direction {
    /// Every direction, in a stable order. Useful for exhaustive neighbour
    /// scans in greedy solvers and for property tests.
    pub const ALL: [Self; 6] = [
        Self::East,
        Self::NorthEast,
        Self::NorthWest,
        Self::West,
        Self::SouthWest,
        Self::SouthEast,
    ];
}

/// What a single agent does in a single step.
///
/// **Stub** — see [`Direction`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionKind {
    /// Stay put. Also the only always-legal action, which makes it the safe
    /// fallback when a solver runs out of budget.
    Wait,
    /// Move one cell towards the given direction.
    Move(Direction),
}

/// One agent's action for one step.
///
/// **Stub** — see [`Direction`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Action {
    /// Which of the team's agents acts.
    pub agent_id: AgentId,
    /// What it does.
    pub kind: ActionKind,
}

impl Action {
    /// Action that keeps `agent_id` where it is.
    #[must_use]
    pub const fn wait(agent_id: AgentId) -> Self {
        Self { agent_id, kind: ActionKind::Wait }
    }

    /// Action that moves `agent_id` one cell towards `direction`.
    #[must_use]
    pub const fn step(agent_id: AgentId, direction: Direction) -> Self {
        Self { agent_id, kind: ActionKind::Move(direction) }
    }

    /// Whether this action consumes no movement (and therefore no fuel).
    #[must_use]
    pub const fn is_wait(self) -> bool {
        matches!(self.kind, ActionKind::Wait)
    }
}

/// Everything a solver is allowed to look at when planning a day.
///
/// This is the *whole* input contract: a solver must be a pure function of
/// this struct. No clock, no environment, no shared mutable state — otherwise
/// replays (`crate::replay`) stop reproducing and comparing two strategies on
/// the same day becomes meaningless.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolverContext {
    /// Authoritative state snapshot the plan must be based on.
    pub match_state: MatchSnapshot,
    /// Wall-clock budget left for answering the current day, in milliseconds.
    ///
    /// Spec: Q&A Q3 (`daySeconds`). The coordinator owns the clock and passes
    /// the remainder down; solvers must treat this as a hard deadline and keep
    /// a valid partial answer available at all times.
    pub remaining_time_ms: u64,
    /// The team this solver plays for.
    pub my_team_id: TeamId,
    /// Ids of the agents this solver controls, in submission order.
    ///
    /// A decision must cover exactly these agents; actions for anyone else are
    /// rejected with the rest of the answer (Q&A Q6).
    pub my_agent_ids: Vec<AgentId>,
    /// Zero-based index of the day being planned.
    pub day: u32,
    /// Number of steps in the day being planned.
    pub day_steps: u32,
}

impl SolverContext {
    /// Assemble a context. Prefer this over struct literals so added fields do
    /// not break every call site at once.
    #[must_use]
    pub fn new(
        match_state: MatchSnapshot,
        my_team_id: TeamId,
        my_agent_ids: Vec<AgentId>,
        day: u32,
        day_steps: u32,
        remaining_time_ms: u64,
    ) -> Self {
        Self { match_state, remaining_time_ms, my_team_id, my_agent_ids, day, day_steps }
    }

    /// Remaining budget as a [`Duration`].
    #[must_use]
    pub const fn remaining_time(&self) -> Duration {
        Duration::from_millis(self.remaining_time_ms)
    }

    /// Whether the budget is exhausted. A solver seeing `true` should return a
    /// fallback immediately rather than start a search.
    #[must_use]
    pub const fn is_out_of_time(&self) -> bool {
        self.remaining_time_ms == 0
    }

    /// How many actions a well-formed decision for this day contains:
    /// one per controlled agent per step.
    #[must_use]
    pub fn expected_action_count(&self) -> usize {
        self.my_agent_ids.len() * self.day_steps as usize
    }
}

/// One day's worth of actions, as produced by a [`Solver`].
///
/// `actions` is **step-major**: the first `my_agent_ids.len()` entries are step
/// 0 (one action per controlled agent, in `SolverContext::my_agent_ids` order),
/// the next block is step 1, and so on for `day_steps` blocks. The flat layout
/// keeps the type cheap to build and to serialise; [`SolverDecision::steps`]
/// recovers the per-step grouping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolverDecision {
    /// The planned actions, step-major.
    pub actions: Vec<Action>,
    /// Free-form solver telemetry (search depth, score estimate, seed, …).
    ///
    /// Carried through to replay metadata and the frontend inspector. It never
    /// affects the submitted answer, so solvers may put anything JSON here.
    pub metadata: serde_json::Value,
}

impl SolverDecision {
    /// Wrap a plan with empty metadata.
    #[must_use]
    pub fn new(actions: Vec<Action>) -> Self {
        Self { actions, metadata: serde_json::Value::Null }
    }

    /// Attach telemetry to a plan.
    #[must_use]
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// The always-legal fallback: every controlled agent waits every step.
    ///
    /// Solvers should return this (or a better partial plan) instead of an
    /// empty decision when the budget runs out, because a malformed answer is
    /// rejected as a whole (Q&A Q6).
    #[must_use]
    pub fn wait_all(ctx: &SolverContext) -> Self {
        let mut actions = Vec::with_capacity(ctx.expected_action_count());
        for _ in 0..ctx.day_steps {
            actions.extend(ctx.my_agent_ids.iter().copied().map(Action::wait));
        }
        Self::new(actions)
    }

    /// Number of planned actions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// Whether nothing was planned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Iterate the plan one step at a time, in blocks of `agent_count`
    /// (normally `SolverContext::my_agent_ids.len()`).
    ///
    /// `agent_count` is clamped to at least one, so a zero cannot panic.
    pub fn steps(&self, agent_count: usize) -> std::slice::Chunks<'_, Action> {
        self.actions.chunks(agent_count.max(1))
    }

    /// Cheap shape check against the context that produced it: one action per
    /// controlled agent per step, and no action for a foreign agent.
    ///
    /// This is *not* rule validation — legality of each move (bounds, ponds,
    /// fuel) belongs to `domain` (issue #53). It only catches solvers that
    /// return a differently-shaped plan than the day asked for.
    #[must_use]
    pub fn is_well_shaped(&self, ctx: &SolverContext) -> bool {
        self.actions.len() == ctx.expected_action_count()
            && self.actions.iter().all(|a| ctx.my_agent_ids.contains(&a.agent_id))
    }
}

/// The contract every `solver-*` crate implements.
///
/// Object-safe by construction: the coordinator stores solvers as
/// `Box<dyn Solver>` and picks between them at runtime, so no method may be
/// generic or take `self` by value.
///
/// `Send + Sync` is required because the coordinator may race several solvers
/// against the same day on different threads and keep the best answer.
///
/// # Example
///
/// The minimum a `solver-*` skeleton needs — a name and a day plan:
///
/// ```
/// use engine::solver::{Solver, SolverContext, SolverDecision};
///
/// struct WaitSolver;
///
/// impl Solver for WaitSolver {
///     fn name(&self) -> &'static str {
///         "wait"
///     }
///
///     fn decide(&self, ctx: &SolverContext) -> SolverDecision {
///         // Legal but useless: everyone stands still all day.
///         SolverDecision::wait_all(ctx)
///     }
/// }
///
/// // The coordinator only ever sees `dyn Solver`.
/// let solvers: Vec<Box<dyn Solver>> = vec![Box::new(WaitSolver)];
/// assert_eq!(solvers[0].name(), "wait");
/// ```
pub trait Solver: Send + Sync {
    /// Stable, human-readable identifier — used in logs, replay metadata, and
    /// to report which strategy produced the submitted answer.
    ///
    /// `&'static str` on purpose: a solver's identity is a compile-time
    /// constant, and the coordinator keeps names around after dropping the
    /// solver that produced an answer. Per-run parameters (beam width, seed,
    /// …) belong in [`SolverDecision::metadata`], not in the name.
    fn name(&self) -> &'static str;

    /// One-time setup before the first day of a match (precomputing shortest
    /// paths, clustering spots, seeding an RNG, …).
    ///
    /// Defaults to doing nothing, so stateless solvers can ignore it. The
    /// coordinator calls this at most once per match, before any `decide`.
    fn init(&mut self, ctx: &SolverContext) {
        let _ = ctx;
    }

    /// Plan one day.
    ///
    /// Must return within `ctx.remaining_time()` and must be deterministic for
    /// a given `ctx` — see [`SolverContext`]. Taking `&self` keeps the method
    /// callable from several threads at once; solvers needing per-day scratch
    /// state should allocate it locally or use interior mutability.
    fn decide(&self, ctx: &SolverContext) -> SolverDecision;
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::state::{GameState, Score};
    use domain::{MatchId, Outcome};

    /// Compile-time proof that the trait is object-safe (`dyn`-compatible).
    const _ASSERT_OBJECT_SAFE: Option<&dyn Solver> = None;

    fn ctx(day_steps: u32, agents: &[AgentId]) -> SolverContext {
        let match_id = MatchId::new("match-test").expect("valid id");
        let team = TeamId::new("player-a").expect("valid id");
        let snapshot = MatchSnapshot {
            match_id: match_id.clone(),
            state: GameState { match_id, turn: 0, scores: vec![(team.clone(), Score(0))] },
            outcome: Outcome::Pending,
            server_time_unix_ms: 0,
        };
        SolverContext::new(snapshot, team, agents.to_vec(), 0, day_steps, 60_000)
    }

    /// Mock solver: returns a fixed, well-shaped plan regardless of state.
    struct FixedSolver {
        name: &'static str,
        direction: Direction,
        init_calls: u32,
    }

    impl FixedSolver {
        fn new(name: &'static str, direction: Direction) -> Self {
            Self { name, direction, init_calls: 0 }
        }
    }

    impl Solver for FixedSolver {
        fn name(&self) -> &'static str {
            self.name
        }

        fn init(&mut self, _ctx: &SolverContext) {
            self.init_calls += 1;
        }

        fn decide(&self, ctx: &SolverContext) -> SolverDecision {
            if ctx.is_out_of_time() {
                return SolverDecision::wait_all(ctx);
            }
            let mut actions = Vec::with_capacity(ctx.expected_action_count());
            for _ in 0..ctx.day_steps {
                actions.extend(ctx.my_agent_ids.iter().map(|id| Action::step(*id, self.direction)));
            }
            SolverDecision::new(actions)
                .with_metadata(serde_json::json!({ "solver": self.name, "kind": "fixed" }))
        }
    }

    #[test]
    fn fixed_solver_plans_one_action_per_agent_per_step() {
        let ctx = ctx(3, &[0, 1]);
        let solver = FixedSolver::new("fixed-east", Direction::East);

        let decision = solver.decide(&ctx);

        assert_eq!(decision.len(), 6, "3 steps x 2 agents");
        assert!(decision.is_well_shaped(&ctx));
        assert_eq!(decision.actions[0], Action::step(0, Direction::East));
        assert_eq!(decision.metadata["solver"], "fixed-east");
    }

    #[test]
    fn solver_is_usable_through_dyn_dispatch() {
        let ctx = ctx(2, &[0, 1, 2]);
        let solvers: Vec<Box<dyn Solver>> = vec![
            Box::new(FixedSolver::new("fixed-east", Direction::East)),
            Box::new(FixedSolver::new("fixed-west", Direction::West)),
        ];

        let names: Vec<&str> = solvers.iter().map(|s| s.name()).collect();
        assert_eq!(names, ["fixed-east", "fixed-west"]);

        for solver in &solvers {
            let decision = solver.decide(&ctx);
            assert!(decision.is_well_shaped(&ctx), "{} returned a bad shape", solver.name());
            assert_eq!(decision.len(), ctx.expected_action_count());
        }
    }

    #[test]
    fn init_default_is_a_no_op_and_override_runs_once() {
        struct Stateless;
        impl Solver for Stateless {
            fn name(&self) -> &'static str {
                "stateless"
            }
            fn decide(&self, ctx: &SolverContext) -> SolverDecision {
                SolverDecision::wait_all(ctx)
            }
        }

        let ctx = ctx(1, &[0]);
        // Default `init` must be callable through `dyn` without any impl.
        let mut stateless: Box<dyn Solver> = Box::new(Stateless);
        stateless.init(&ctx);
        assert_eq!(stateless.decide(&ctx).len(), 1);

        let mut fixed = FixedSolver::new("fixed", Direction::East);
        fixed.init(&ctx);
        assert_eq!(fixed.init_calls, 1);
    }

    #[test]
    fn out_of_time_falls_back_to_waiting() {
        let mut ctx = ctx(4, &[0, 1]);
        ctx.remaining_time_ms = 0;
        assert!(ctx.is_out_of_time());

        let decision = FixedSolver::new("fixed", Direction::East).decide(&ctx);

        assert!(decision.is_well_shaped(&ctx));
        assert!(decision.actions.iter().copied().all(Action::is_wait));
    }

    #[test]
    fn wait_all_is_step_major_and_covers_every_agent() {
        let ctx = ctx(3, &[2, 5]);
        let decision = SolverDecision::wait_all(&ctx);

        let steps: Vec<Vec<AgentId>> = decision
            .steps(ctx.my_agent_ids.len())
            .map(|step| step.iter().map(|a| a.agent_id).collect())
            .collect();
        assert_eq!(steps, vec![vec![2, 5], vec![2, 5], vec![2, 5]]);
    }

    #[test]
    fn wrong_shape_is_rejected() {
        let ctx = ctx(2, &[0, 1]);

        assert!(!SolverDecision::new(vec![]).is_well_shaped(&ctx), "empty plan");
        assert!(!SolverDecision::new(vec![Action::wait(0); 3]).is_well_shaped(&ctx), "short plan");
        let foreign = vec![Action::wait(0), Action::wait(9), Action::wait(0), Action::wait(1)];
        assert!(!SolverDecision::new(foreign).is_well_shaped(&ctx), "foreign agent id");
    }

    #[test]
    fn remaining_time_maps_to_duration() {
        let ctx = ctx(1, &[0]);
        assert_eq!(ctx.remaining_time(), Duration::from_secs(60));
    }

    #[test]
    fn decision_round_trips_through_json() {
        let ctx = ctx(2, &[0, 1]);
        let decision = FixedSolver::new("fixed", Direction::SouthWest).decide(&ctx);

        let json = serde_json::to_string(&decision).expect("serialise");
        let back: SolverDecision = serde_json::from_str(&json).expect("deserialise");

        assert_eq!(decision, back);
    }

    #[test]
    fn context_round_trips_through_json() {
        let ctx = ctx(5, &[0, 1, 2]);
        let json = serde_json::to_string(&ctx).expect("serialise");
        let back: SolverContext = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(ctx, back);
    }
}
