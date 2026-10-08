//! `procon ops warmup` — measure cold-start / warm / 1-day-smoke timing for
//! the contest solver stack.
//!
//! Why this exists: the contest rules cap each day at ~60 s wall-clock. If a
//! solver's first decision on day N takes 30 s because nothing has been
//! pre-computed, we have already lost that day. `run_warmup` exists so
//! operators can `procon warmup` ahead of the match, verify p90 stays below
//! the budget, and confirm no solver regresses silently.
//!
//! The mini solvers in [`fixtures`] are deliberately tiny local stand-ins
//! for the eventual production crates. They share the timing surface but
//! keep this command's build cost zero until #22-#44 land.
//!
//! Three passes per solver, each producing an independent sample stream:
//!
//! 1. **cold**: instantiate a fresh solver, time `prepare()` + one
//!    `decide()`. Catches first-touch / page-fault regressions.
//! 2. **warm**: same as cold, but the solver instance is reused across
//!    samples so caches stay hot. Catches per-call algorithmic regressions
//!    that the cold pass would mask.
//! 3. **smoke**: run `day_steps` decisions end-to-end on the setting,
//!    starting from a known position, walking each agent across the grid.
//!    Catches latency that only shows up at scale (e.g. quadratic
//!    bookkeeping that grows with `day_steps`).

use std::cmp::Ordering;
use std::time::Instant;

use domain::id::PlayerId;
use serde::Serialize;

use crate::fixtures::{
    Action, MiniSolver, SAMPLES_PER_PASS, SolveOutcome, Tile, WARMUP_ITERATIONS, WarmupSetting,
    micros,
};

/// Public, serialisable summary returned by [`run_warmup`].
///
/// `solver_runs[i]` aligns with `solver_names[i]`. Times are reported in
/// microseconds so they stay readable in both human and JSON output.
#[derive(Debug, Clone, Serialize)]
pub struct WarmupReport {
    pub setting: SettingSummary,
    pub solver_runs: Vec<SolverRun>,
    /// `passes` describing what each sample bucket measured.
    pub passes: Vec<&'static str>,
    /// `samples_per_pass` used to compute the percentiles.
    pub samples_per_pass: usize,
}

/// Compact, JSON-friendly summary of the setting under test.
#[derive(Debug, Clone, Serialize)]
pub struct SettingSummary {
    pub match_id: String,
    pub width: usize,
    pub height: usize,
    pub agents: usize,
    pub spots: usize,
    pub day_steps: u32,
}

impl From<&WarmupSetting> for SettingSummary {
    fn from(s: &WarmupSetting) -> Self {
        Self {
            match_id: s.match_id.to_string(),
            width: s.width,
            height: s.height,
            agents: s.agents.len(),
            spots: s.spots.len(),
            day_steps: s.day_steps,
        }
    }
}

/// Timing summary for one solver across all three passes.
#[derive(Debug, Clone, Serialize)]
pub struct SolverRun {
    pub name: String,
    pub cold: PassTimings,
    pub warm: PassTimings,
    pub smoke: PassTimings,
}

/// p50 / p90 / max plus a sample count, all in microseconds.
#[derive(Debug, Clone, Serialize)]
pub struct PassTimings {
    pub p50_us: u64,
    pub p90_us: u64,
    pub max_us: u64,
    pub samples: usize,
}

impl PassTimings {
    fn from_samples(samples: &[u64]) -> Self {
        assert!(!samples.is_empty(), "warmup pass produced no samples");
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let p50 = percentile(&sorted, 0.50);
        let p90 = percentile(&sorted, 0.90);
        let max = *sorted.last().expect("non-empty");
        Self { p50_us: p50, p90_us: p90, max_us: max, samples: sorted.len() }
    }
}

/// Linearly-interpolated percentile. `p` is in `[0.0, 1.0]`.
///
/// We accept `f64` precision loss deliberately: timing data is noisy and
/// the centi-microsecond differences that a lossless conversion would
/// preserve are well below the noise floor of `Instant::now()`.
fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.len() == 1 {
        return sorted[0];
    }
    let len = sorted.len();
    #[allow(clippy::cast_precision_loss)]
    let rank = p.clamp(0.0, 1.0) * (len as f64 - 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lo = rank.floor() as usize;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let hi = rank.ceil() as usize;
    match lo.cmp(&hi) {
        Ordering::Equal => sorted[lo],
        Ordering::Less => {
            #[allow(clippy::cast_precision_loss)]
            let weight = rank - lo as f64;
            #[allow(clippy::cast_precision_loss)]
            let lo_v = sorted[lo] as f64;
            #[allow(clippy::cast_precision_loss)]
            let hi_v = sorted[hi] as f64;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let blended = (lo_v + (hi_v - lo_v) * weight).round() as u64;
            blended
        }
        Ordering::Greater => sorted[hi],
    }
}

/// Run the three-pass timing harness for every name in `solver_names`.
///
/// `solver_names` is matched against the registry in [`mini_solvers`].
/// Unknown names are reported as a [`WarmupError::UnknownSolver`] so the
/// caller surfaces the typo before any timing data is emitted.
pub fn run_warmup(
    setting: &WarmupSetting,
    solver_names: &[&str],
) -> Result<WarmupReport, WarmupError> {
    let mut solver_runs = Vec::with_capacity(solver_names.len());
    for &name in solver_names {
        let solver = mini_solvers(name).ok_or(WarmupError::UnknownSolver(name.to_owned()))?;
        solver_runs.push(time_solver(setting, solver));
    }
    Ok(WarmupReport {
        setting: SettingSummary::from(setting),
        solver_runs,
        passes: vec!["cold", "warm", "smoke"],
        samples_per_pass: SAMPLES_PER_PASS,
    })
}

/// Errors returned by [`run_warmup`]. Intentionally narrow — anything else
/// would mean the warmup harness itself regressed, not the solver under
/// test.
#[derive(Debug, thiserror::Error)]
pub enum WarmupError {
    #[error("unknown solver name: {0} (known: greedy, astar, loop, mcts)")]
    UnknownSolver(String),
}

fn time_solver(setting: &WarmupSetting, mut solver: Box<dyn MiniSolver>) -> SolverRun {
    let name = solver.name().to_owned();

    // --- cold pass -------------------------------------------------------
    // Brand-new solver each sample. Catches first-touch / cache miss.
    let mut cold_samples = Vec::with_capacity(SAMPLES_PER_PASS);
    for _ in 0..SAMPLES_PER_PASS {
        let mut fresh = mini_solvers(&name).expect("known solver");
        let start = Instant::now();
        fresh.prepare(setting);
        let _ = fresh.decide(setting, &setting.agents[0], (0, 0), setting.spots[0]);
        cold_samples.push(micros(start.elapsed()));
    }

    // --- warm pass -------------------------------------------------------
    // Same instance reused, plus WARMUP_ITERATIONS discards so caches settle.
    let mut warm_samples = Vec::with_capacity(SAMPLES_PER_PASS);
    for _ in 0..WARMUP_ITERATIONS {
        solver.prepare(setting);
        let _ = solver.decide(setting, &setting.agents[0], (0, 0), setting.spots[0]);
    }
    for _ in 0..SAMPLES_PER_PASS {
        let start = Instant::now();
        let _ = solver.decide(setting, &setting.agents[0], (0, 0), setting.spots[0]);
        warm_samples.push(micros(start.elapsed()));
    }

    // --- smoke pass ------------------------------------------------------
    // End-to-end: walk `day_steps` decisions per agent, summing wall-clock.
    let mut smoke_samples = Vec::with_capacity(SAMPLES_PER_PASS);
    for sample in 0..SAMPLES_PER_PASS {
        let mut s = mini_solvers(&name).expect("known solver");
        s.prepare(setting);
        let start = Instant::now();
        for step in 0..setting.day_steps {
            for (agent_idx, agent) in setting.agents.iter().enumerate() {
                let from = pseudo_pos(agent_idx, step, setting);
                let to = setting.spots[(step as usize + agent_idx + sample) % setting.spots.len()];
                let _ = s.decide(setting, agent, from, to);
            }
        }
        smoke_samples.push(micros(start.elapsed()));
    }

    SolverRun {
        name,
        cold: PassTimings::from_samples(&cold_samples),
        warm: PassTimings::from_samples(&warm_samples),
        smoke: PassTimings::from_samples(&smoke_samples),
    }
}

/// Deterministic per-agent start position so smoke pass is reproducible.
fn pseudo_pos(agent_idx: usize, step: u32, setting: &WarmupSetting) -> (usize, usize) {
    let span = setting.width.min(setting.height);
    let row = (agent_idx * 2) % span;
    let col = ((step as usize) + agent_idx) % setting.width;
    (row, col)
}

// ---------------------------------------------------------------------
// Mini solvers
// ---------------------------------------------------------------------
//
// Each solver is a deliberately small implementation that:
//   - mirrors the eventual solver's API surface (so swapping in the real
//     crate is a one-line change in `mini_solvers()`),
//   - returns the correct shape of [`SolveOutcome`] (so consumers can wire
//     metrics against it today),
//   - exposes a known cost profile so `cold` vs `warm` percentiles diverge
//     in a predictable way (so regressions are obvious).
//
// We do NOT benchmark algorithmic quality here — the goal is timing
// surface, not solution quality.

/// Greedy: always head toward `to` along Manhattan until the next tile would
/// be `Blocked`, then pick the cheapest neighbour.
struct GreedySolver;

impl MiniSolver for GreedySolver {
    fn name(&self) -> &'static str {
        "greedy"
    }

    fn decide(
        &mut self,
        setting: &WarmupSetting,
        agent_id: &PlayerId,
        from: (usize, usize),
        to: (usize, usize),
    ) -> SolveOutcome {
        let mut row = from.0;
        let mut col = from.1;
        let mut actions = Vec::with_capacity(setting.day_steps as usize);
        let mut explored = 0usize;
        for step in 0..setting.day_steps as usize {
            let to_r = i64::try_from(to.0).expect("grid fits in i64");
            let to_c = i64::try_from(to.1).expect("grid fits in i64");
            let r = i64::try_from(row).expect("grid fits in i64");
            let c = i64::try_from(col).expect("grid fits in i64");
            let dr = (to_r - r).signum();
            let dc = (to_c - c).signum();
            let nr = r + dr;
            let nc = c + dc;
            let (nr_u, nc_u) = match (usize::try_from(nr).ok(), usize::try_from(nc).ok()) {
                (Some(a), Some(b)) => (a, b),
                _ => (row, col),
            };
            explored += 1;
            if nr_u < setting.height
                && nc_u < setting.width
                && setting.tile(nr_u, nc_u) != Tile::Blocked
            {
                row = nr_u;
                col = nc_u;
            }
            actions.push(Action {
                agent: agent_id.clone(),
                #[allow(clippy::cast_possible_truncation)]
                step: step as u32,
                dr: dr as i8,
                dc: dc as i8,
            });
            if (row, col) == to {
                break;
            }
        }
        SolveOutcome { actions, explored_nodes: explored }
    }
}

/// A* over the grid, with a tiny caching layer so warm passes actually
/// benefit from pre-computation. The cache is keyed by `(from, to)` and
/// stores the computed path.
#[allow(clippy::type_complexity)] // cache signature mirrors the future production cache shape
struct AstarSolver {
    cache: std::collections::HashMap<(usize, usize, usize, usize), Vec<(usize, usize)>>,
}

impl AstarSolver {
    fn new() -> Self {
        Self { cache: std::collections::HashMap::new() }
    }
}

impl MiniSolver for AstarSolver {
    fn name(&self) -> &'static str {
        "astar"
    }

    fn prepare(&mut self, setting: &WarmupSetting) {
        // Pre-compute the cross-ridge pair so the warm pass shows a real win
        // against cold. Without this `prepare()` is a no-op and A* would
        // appear identical to greedy in warmup numbers.
        for &(sr, sc) in &setting.spots {
            for &(tr, tc) in &setting.spots {
                let path = astar(setting, (sr, sc), (tr, tc));
                self.cache.insert((sr, sc, tr, tc), path);
            }
        }
    }

    fn decide(
        &mut self,
        setting: &WarmupSetting,
        agent_id: &PlayerId,
        from: (usize, usize),
        to: (usize, usize),
    ) -> SolveOutcome {
        let path = self
            .cache
            .entry((from.0, from.1, to.0, to.1))
            .or_insert_with(|| astar(setting, from, to))
            .clone();
        // The dr/dc values are intentionally tiny (a single grid step); the
        // cast to i8 is bounded by the grid span which always fits.
        #[allow(clippy::cast_possible_truncation)]
        let actions: Vec<Action> = path
            .iter()
            .enumerate()
            .map(|(step, &(r, c))| Action {
                agent: agent_id.clone(),
                #[allow(clippy::cast_possible_truncation)]
                step: step as u32,
                dr: r as i8 - from.0 as i8,
                dc: c as i8 - from.1 as i8,
            })
            .collect();
        SolveOutcome { actions, explored_nodes: path.len() }
    }
}

fn astar(setting: &WarmupSetting, from: (usize, usize), to: (usize, usize)) -> Vec<(usize, usize)> {
    use std::collections::BinaryHeap;

    #[derive(Debug, Eq, PartialEq)]
    struct Node {
        cost: u32,
        row: usize,
        col: usize,
    }
    impl Ord for Node {
        fn cmp(&self, other: &Self) -> Ordering {
            other.cost.cmp(&self.cost) // min-heap
        }
    }
    impl PartialOrd for Node {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    let heuristic = |(r, c): (usize, usize)| -> u32 {
        let dr = u32::try_from(r.abs_diff(to.0)).expect("grid fits in u32");
        let dc = u32::try_from(c.abs_diff(to.1)).expect("grid fits in u32");
        // Saturating add — `u32` max is well above any realistic grid span.
        dr.saturating_add(dc)
    };

    let mut heap = BinaryHeap::new();
    let mut came_from: std::collections::HashMap<(usize, usize), (usize, usize)> =
        std::collections::HashMap::new();
    let mut g_score: std::collections::HashMap<(usize, usize), u32> =
        std::collections::HashMap::new();
    g_score.insert(from, 0);
    heap.push(Node { cost: heuristic(from), row: from.0, col: from.1 });
    let mut explored = 0usize;

    while let Some(node) = heap.pop() {
        if (node.row, node.col) == to {
            let mut path = vec![to];
            let mut cur = to;
            while let Some(&prev) = came_from.get(&cur) {
                path.push(prev);
                cur = prev;
            }
            path.reverse();
            // First element is `from`; skip it so callers see only moves.
            path.remove(0);
            // Touch `explored` so the warm-vs-cold comparison cannot be
            // optimised away.
            explored += path.len();
            let _ = explored;
            return path;
        }
        explored += 1;
        let (r, c) = (node.row, node.col);
        let cur_g = *g_score.get(&(r, c)).unwrap_or(&u32::MAX);
        for (dr, dc) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
            // All grid coordinates fit comfortably in `i32`; this conversion
            // is sound for any realistic contest map (32x32 / 64x64).
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let nr_signed = (r as i32) + dr;
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let nc_signed = (c as i32) + dc;
            if nr_signed < 0 || nc_signed < 0 {
                continue;
            }
            #[allow(clippy::cast_sign_loss)]
            let (nr, nc) = (nr_signed as usize, nc_signed as usize);
            if nr >= setting.height || nc >= setting.width {
                continue;
            }
            let tile = setting.tile(nr, nc);
            if tile == Tile::Blocked || tile.cost() == u32::MAX {
                continue;
            }
            let tentative = cur_g.saturating_add(tile.cost());
            let key = (nr, nc);
            if tentative < *g_score.get(&key).unwrap_or(&u32::MAX) {
                g_score.insert(key, tentative);
                came_from.insert(key, (r, c));
                let f = tentative.saturating_add(heuristic(key));
                heap.push(Node { cost: f, row: nr, col: nc });
            }
        }
    }
    Vec::new()
}

/// 2-opt loop: produce a `decide` decision by greedily picking the next
/// spot to visit, then 2-opt-swap the visit order across one pass.
struct LoopSolver {
    route: Vec<usize>,
}

impl LoopSolver {
    fn new() -> Self {
        Self { route: Vec::new() }
    }
}

impl MiniSolver for LoopSolver {
    fn name(&self) -> &'static str {
        "loop"
    }

    fn prepare(&mut self, setting: &WarmupSetting) {
        // Initial route: visit spots in index order.
        self.route = (0..setting.spots.len()).collect();
        // One 2-opt pass: swap any pair that improves total Manhattan length.
        let mut improved = true;
        let mut iterations = 0;
        while improved && iterations < 8 {
            improved = false;
            iterations += 1;
            for i in 0..self.route.len() {
                for j in (i + 1)..self.route.len() {
                    let before = route_cost(&self.route, setting);
                    self.route.swap(i, j);
                    let after = route_cost(&self.route, setting);
                    if after >= before {
                        self.route.swap(i, j);
                    } else {
                        improved = true;
                    }
                }
            }
        }
    }

    fn decide(
        &mut self,
        setting: &WarmupSetting,
        agent_id: &PlayerId,
        from: (usize, usize),
        _to: (usize, usize),
    ) -> SolveOutcome {
        let actions: Vec<Action> = self
            .route
            .iter()
            .enumerate()
            .map(|(step, &spot_idx)| {
                let (sr, sc) = setting.spots[spot_idx];
                let from_r = i64::try_from(from.0).expect("grid fits in i64");
                let from_c = i64::try_from(from.1).expect("grid fits in i64");
                let s_r = i64::try_from(sr).expect("grid fits in i64");
                let s_c = i64::try_from(sc).expect("grid fits in i64");
                let dr = (s_r - from_r).signum();
                let dc = (s_c - from_c).signum();
                Action {
                    agent: agent_id.clone(),
                    #[allow(clippy::cast_possible_truncation)]
                    step: step as u32,
                    dr: dr as i8,
                    dc: dc as i8,
                }
            })
            .collect();
        SolveOutcome { actions, explored_nodes: self.route.len() }
    }
}

fn route_cost(route: &[usize], setting: &WarmupSetting) -> u64 {
    let mut total = 0u64;
    for window in route.windows(2) {
        let (a, b) = (window[0], window[1]);
        let (ar, ac) = setting.spots[a];
        let (br, bc) = setting.spots[b];
        let dr = u64::try_from(ar.abs_diff(br)).expect("grid fits in u64");
        let dc = u64::try_from(ac.abs_diff(bc)).expect("grid fits in u64");
        total = total.saturating_add(dr.saturating_add(dc));
    }
    total
}

/// Monte-Carlo Tree Search: budget `rollouts` rollouts per call, score each
/// by random spot ordering. Caches the best action for the current
/// `(from, to)` pair so warm passes are noticeably faster.
struct MctsSolver {
    best: std::collections::HashMap<(usize, usize, usize, usize), Action>,
    rollouts: usize,
}

impl MctsSolver {
    fn new(rollouts: usize) -> Self {
        Self { best: std::collections::HashMap::new(), rollouts }
    }
}

impl MiniSolver for MctsSolver {
    fn name(&self) -> &'static str {
        "mcts"
    }

    fn decide(
        &mut self,
        setting: &WarmupSetting,
        agent_id: &PlayerId,
        from: (usize, usize),
        to: (usize, usize),
    ) -> SolveOutcome {
        let key = (from.0, from.1, to.0, to.1);
        if let Some(cached) = self.best.get(&key) {
            return SolveOutcome { actions: vec![cached.clone()], explored_nodes: 0 };
        }
        let mut best_action = Action { agent: agent_id.clone(), step: 0, dr: 0, dc: 0 };
        let mut best_score = i64::MIN;
        let mut explored = 0usize;
        // Splitmix64-style LCG constants — well-known, used as-is to keep
        // the warmup deterministic across runs.
        #[allow(clippy::unreadable_literal)]
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let step_max = setting.day_steps.max(1) as usize;
        for _ in 0..self.rollouts {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let step = (state >> 33) as usize % step_max;
            let to_r = i64::try_from(to.0).expect("grid fits in i64");
            let to_c = i64::try_from(to.1).expect("grid fits in i64");
            let f_r = i64::try_from(from.0).expect("grid fits in i64");
            let f_c = i64::try_from(from.1).expect("grid fits in i64");
            let dr = (to_r - f_r).signum();
            let dc = (to_c - f_c).signum();
            let step_i = i64::try_from(step).expect("warmup step fits in i64");
            let score = step_i - dr.abs() - dc.abs();
            explored += 1;
            if score > best_score {
                best_score = score;
                best_action = Action {
                    agent: agent_id.clone(),
                    #[allow(clippy::cast_possible_truncation)]
                    step: step as u32,
                    dr: dr as i8,
                    dc: dc as i8,
                };
            }
        }
        self.best.insert(key, best_action.clone());
        SolveOutcome { actions: vec![best_action], explored_nodes: explored }
    }
}

/// Registry mapping solver names to factories. Keep this in one place so
/// adding a new solver is a single edit.
fn mini_solvers(name: &str) -> Option<Box<dyn MiniSolver>> {
    match name {
        "greedy" => Some(Box::new(GreedySolver)),
        "astar" => Some(Box::new(AstarSolver::new())),
        "loop" => Some(Box::new(LoopSolver::new())),
        "mcts" => Some(Box::new(MctsSolver::new(64))),
        _ => None,
    }
}

/// Convenience: list the solver names this build knows about. Useful for
/// `--help` output and for tests that assert the registry stays in sync.
#[must_use]
#[allow(dead_code)] // exercised by tests in this module
pub fn known_solvers() -> &'static [&'static str] {
    &["greedy", "astar", "loop", "mcts"]
}

/// Render [`WarmupReport`] as a human-readable summary table.
///
/// Kept separate from the JSON serialisation path so JSON consumers do not
/// have to parse free-form text. The output is intentionally line-oriented
/// to stay readable in CI logs.
#[must_use]
#[allow(clippy::too_many_lines)] // formatted table; one block reads better than five helpers
pub fn format_report_human(report: &WarmupReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "warmup setting: match={} size={}x{} agents={} spots={} day_steps={}",
        report.setting.match_id,
        report.setting.width,
        report.setting.height,
        report.setting.agents,
        report.setting.spots,
        report.setting.day_steps,
    );
    let _ =
        writeln!(out, "samples_per_pass={} passes={:?}", report.samples_per_pass, report.passes);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{:<8} {:>8} {:>8} {:>8}    {:>8} {:>8} {:>8}    {:>10} {:>10} {:>10}",
        "solver",
        "cold_p50",
        "cold_p90",
        "cold_max",
        "warm_p50",
        "warm_p90",
        "warm_max",
        "smoke_p50",
        "smoke_p90",
        "smoke_max",
    );
    for run in &report.solver_runs {
        let _ = writeln!(
            out,
            "{:<8} {:>7}us {:>7}us {:>7}us    {:>7}us {:>7}us {:>7}us    {:>9}us {:>9}us {:>9}us",
            run.name,
            run.cold.p50_us,
            run.cold.p90_us,
            run.cold.max_us,
            run.warm.p50_us,
            run.warm.p90_us,
            run.warm.max_us,
            run.smoke.p50_us,
            run.smoke.p90_us,
            run.smoke.max_us,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> WarmupSetting {
        crate::fixtures::tiny_setting()
    }

    #[test]
    fn all_four_solvers_register() {
        for name in known_solvers() {
            let solver = mini_solvers(name).unwrap_or_else(|| panic!("missing solver {name}"));
            assert_eq!(solver.name(), *name);
        }
    }

    #[test]
    fn unknown_solver_errors() {
        let err = run_warmup(&tiny(), &["nope"]).unwrap_err();
        assert!(matches!(err, WarmupError::UnknownSolver(_)));
    }

    #[test]
    fn warmup_report_has_all_passers() {
        let report = run_warmup(&tiny(), &["greedy"]).expect("greedy runs");
        assert_eq!(report.solver_runs.len(), 1);
        let run = &report.solver_runs[0];
        assert_eq!(run.name, "greedy");
        assert_eq!(run.cold.samples, SAMPLES_PER_PASS);
        assert_eq!(run.warm.samples, SAMPLES_PER_PASS);
        assert_eq!(run.smoke.samples, SAMPLES_PER_PASS);
        // p50 <= p90 <= max, always.
        assert!(run.cold.p50_us <= run.cold.p90_us);
        assert!(run.cold.p90_us <= run.cold.max_us);
    }

    #[test]
    fn percentile_handles_single_sample() {
        assert_eq!(percentile(&[42], 0.5), 42);
        assert_eq!(percentile(&[42], 0.9), 42);
        assert_eq!(percentile(&[42], 0.0), 42);
    }

    #[test]
    fn percentile_interpolates() {
        let sorted = vec![10, 20, 30, 40, 50];
        assert_eq!(percentile(&sorted, 0.0), 10);
        assert_eq!(percentile(&sorted, 1.0), 50);
        assert_eq!(percentile(&sorted, 0.5), 30);
    }

    #[test]
    fn astar_finds_path_through_ridge() {
        let setting = tiny();
        // From (0, 0) to spot (6, 6) — must cross the mountain ridge.
        let path = astar(&setting, (0, 0), (6, 6));
        assert!(!path.is_empty(), "A* must find a path; mountain ridge is passable");
        // Final position must be the goal.
        assert_eq!(*path.last().expect("non-empty"), (6, 6));
    }

    #[test]
    fn greedy_decides_actions() {
        let setting = tiny();
        let mut solver = GreedySolver;
        let outcome = solver.decide(&setting, &setting.agents[0], (0, 0), (2, 2));
        assert_ne!(outcome.actions.len(), 0);
    }

    #[test]
    fn format_report_human_is_non_empty() {
        let report = run_warmup(&tiny(), &["greedy", "mcts"]).expect("runs");
        let rendered = format_report_human(&report);
        assert!(rendered.contains("greedy"));
        assert!(rendered.contains("mcts"));
        assert!(rendered.contains("cold_p50"));
    }
}
