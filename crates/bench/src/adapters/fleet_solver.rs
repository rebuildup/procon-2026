//! Fleet planning: patrol routes as a team orienteering problem, supplies as
//! interceptors of the finished patrol routes (Issue #371).
//!
//! `coverage` decides one move at a time (nearest unclaimed series) and lets
//! each Supply chase one paired Patrol. That caps the useful Patrol count at
//! the Supply count: an unpaired Patrol starves, so patrol-heavy kinds lose
//! daily variety. This candidate plans the whole day instead:
//!
//! 1. Shortest-path matrices between every spot and every agent start, under
//!    today's terrain, for three leg metrics (fastest / balanced / fuel-lean).
//! 2. Each Patrol owns an ordered spot list. A simulated-annealing search
//!    inserts, removes, relocates, swaps and reverses spots across patrols,
//!    scored by new series, today's series, today's balls and a greedy
//!    estimate of tomorrow from the day's end positions and fuel.
//! 3. Each Supply owns an ordered list of patrols to refuel. It intercepts
//!    the first one on that patrol's timed route (or meets it where it
//!    stalls), tops it up, moves on, and escorts the last one.
//!
//! The day model mirrors the official tick clock (a move from terrain `x`
//! lands `step_cost(x)` ticks later; every reflection picks and refuels), but
//! the returned plan is never trusted from the model alone: it is replayed
//! through [`engine::MatchSim`], and a plan with an illegal or fuel-short
//! action is discarded for the `coverage` plan.
#![allow(
    clippy::many_single_char_names,
    reason = "graph code: m = metric, u/v = nodes, p/q = patrols, t/f = time/fuel"
)]
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

use crate::adapters::coverage_solver::{CoverageCandidate, mix};
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use engine::MatchSim;
use engine::model::{
    Action, AgentActions, AgentKind, Direction, MatchState, Solver, SolverContext, Terrain,
};
use engine::rules::fuel_cost;

pub struct FleetFactory;
const PARAMS: &[ParamSpec] = &[
    ParamSpec { key: "budget", required: true, doc: "official (required)" },
    ParamSpec {
        key: "iters",
        required: false,
        doc: "OFFLINE deterministic search length per day (default 20000)",
    },
    ParamSpec {
        key: "budget_ms",
        required: false,
        doc: "LIVE wall-clock cap per day plan, 1..59000; overrides iters",
    },
    ParamSpec {
        key: "adapt",
        required: false,
        doc: "on (default) | off: with budget_ms, stop small boards early \
              (response time is the fourth ranking axis)",
    },
    ParamSpec { key: "seed", required: false, doc: "search seed (default 42)" },
    ParamSpec {
        key: "chains",
        required: false,
        doc: "OFFLINE search chains, 1..8 (default 1); with 2+ the day plan is chosen \
              by planning tomorrow from each chain's end. Live uses the spare cores",
    },
];

fn parse<T: std::str::FromStr>(
    spec: &CandidateSpec,
    key: &str,
    default: T,
) -> Result<T, CandidateError> {
    match spec.param(key) {
        None => Ok(default),
        Some(s) => s.parse().map_err(|_| CandidateError::Build(format!("invalid {key}: {s:?}"))),
    }
}

impl CandidateFactory for FleetFactory {
    fn algorithm(&self) -> &'static str {
        "fleet"
    }
    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }
    fn describe(&self) -> &'static str {
        "Team-orienteering patrol routes with supply interception"
    }
    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        if spec.param("budget") != Some("official") {
            return Err(CandidateError::Build("fleet requires budget=official".into()));
        }
        let iters = parse(spec, "iters", 20_000u32)?;
        let budget_ms = parse(spec, "budget_ms", 0u32)?;
        if budget_ms > 59_000 {
            return Err(CandidateError::Build(format!(
                "budget_ms must be 0..=59000, got {budget_ms}"
            )));
        }
        let seed = parse(spec, "seed", 42u64)?;
        let adapt = match spec.param("adapt") {
            None | Some("on") => true,
            Some("off") => false,
            Some(other) => {
                return Err(CandidateError::Build(format!("unknown adapt mode {other:?}")));
            }
        };
        let chains = parse(spec, "chains", 1u32)?;
        if !(1..=8).contains(&chains) {
            return Err(CandidateError::Build(format!("chains must be 1..=8, got {chains}")));
        }
        Ok(Box::new(FleetCandidate { iters, budget_ms, seed, chains, adapt }))
    }
}

pub struct FleetCandidate {
    pub iters: u32,
    pub budget_ms: u32,
    pub seed: u64,
    /// Offline chain count; live runs one chain per spare core (at most 4).
    pub chains: u32,
    /// Live: shrink the window on boards whose search converges sooner.
    pub adapt: bool,
}

/// Starting temperatures of the chains, in score units (one ball = 100).
/// The cool schedule collects the most balls on roomy days; the hot ones
/// keep more series on days too short to reach everything (measured on the
/// hard corpus), so the chains differ and tomorrow's replay picks.
const CHAIN_T0: [f64; 4] = [40.0, 150.0, 400.0, 40.0];

const NM: usize = 3;
const FAST: usize = 0;
const BAL: usize = 1;
const NONE: u32 = u32::MAX;

const W_NEW: i64 = 1_000_000;
const W_SERIES: i64 = 10_000;
const W_BALL: i64 = 100;
/// Tomorrow is an estimate, so it is worth a little less than today.
const TOMORROW_NUM: i64 = 8;
const TOMORROW_DEN: i64 = 10;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix(self.0)
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n.max(1) as u64).expect("bounded")
    }
    fn unit(&mut self) -> f64 {
        #[allow(clippy::cast_precision_loss, reason = "53-bit mantissa sample")]
        let x = (self.next() >> 11) as f64 / (1u64 << 53) as f64;
        x
    }
}

struct PatrolInfo {
    /// Fuel the patrol may burn today (its tank, or a paced share of it
    /// when no Supply can ever refill it).
    budget: u32,
    agent: usize,
    start: usize,
    fuel: u32,
    cap: u32,
}

struct SupplyInfo {
    agent: usize,
    start: usize,
}

/// One day's static world: nodes are the spots (first) then the agent starts.
struct World {
    time_cost: Vec<u32>,
    fuel_cost: Vec<u32>,
    neighbors: Vec<[u32; 6]>,
    nodes: Vec<usize>,
    spots: usize,
    series: Vec<u32>,
    stock: Vec<u32>,
    /// `spot_at[cell]` = spot node on that cell.
    spot_at: Vec<u32>,
    nn: usize,
    time: [Vec<u32>; NM],
    fuel: [Vec<u32>; NM],
    /// `prev[m][u][cell]` = (previous cell, direction index) on u's tree.
    prev: [Vec<Vec<(u32, u8)>>; NM],
    /// Spots strictly inside the `u -> v` path, with their time offset.
    via: [Vec<Vec<(u16, u32)>>; NM],
    steps: u32,
    tomorrow: Option<u32>,
    /// `(today's steps, later days' steps)` when tanks are paced.
    pace: Option<(u64, u64)>,
    patrols: Vec<PatrolInfo>,
    supplies: Vec<SupplyInfo>,
    new_series: Vec<bool>,
    series_slots: usize,
}

fn cell_index(width: usize, col: i32, row: i32) -> usize {
    usize::try_from(row).expect("in-bounds row") * width
        + usize::try_from(col).expect("in-bounds col")
}

impl World {
    #[allow(clippy::too_many_lines, reason = "one-shot precomputation of the day's matrices")]
    fn new(state: &MatchState, steps: u32, later: &[u32]) -> Self {
        let tomorrow = later.first().copied();
        let map = &state.setting.map;
        let width = usize::from(map.size.width);
        let cells = map.cells();
        let time_cost: Vec<u32> = cells
            .iter()
            .map(|c| engine::wire_plan::terrain_step_cost(c.terrain).unwrap_or(NONE))
            .collect();
        let fuel: Vec<u32> = cells.iter().map(|c| fuel_cost(c.terrain)).collect();
        let neighbors: Vec<[u32; 6]> = cells
            .iter()
            .map(|c| {
                let mut out = [NONE; 6];
                for (d, dir) in Direction::ALL.iter().enumerate() {
                    let to = dir.step(c.coord);
                    if map.get(to).is_some_and(|t| t.terrain != Terrain::Pond) {
                        out[d] = u32::try_from(cell_index(width, to.col, to.row)).expect("cell");
                    }
                }
                out
            })
            .collect();
        let mut nodes = Vec::new();
        let mut series = Vec::new();
        let mut stock = Vec::new();
        let mut spot_at = vec![NONE; cells.len()];
        for (i, c) in cells.iter().enumerate() {
            if let Some(spot) = &c.spot {
                spot_at[i] = u32::try_from(nodes.len()).expect("node");
                nodes.push(i);
                series.push(spot.series);
                stock.push(spot.max_stock);
            }
        }
        let spots = nodes.len();
        let mut patrols = Vec::new();
        let mut supplies = Vec::new();
        for (a, agent) in state.agents.iter().enumerate() {
            let start = nodes.len();
            nodes.push(cell_index(width, agent.pos.col, agent.pos.row));
            if agent.kind == AgentKind::Patrol {
                patrols.push(PatrolInfo {
                    budget: agent.fuel,
                    agent: a,
                    start,
                    fuel: agent.fuel,
                    cap: agent.fuel_cap,
                });
            } else {
                supplies.push(SupplyInfo { agent: a, start });
            }
        }
        let nn = nodes.len();
        // Without a Supply the tanks must last the match: pace them by each
        // day's share of the remaining steps.
        let later_steps: u64 = later.iter().map(|&x| u64::from(x)).sum();
        let pace =
            (supplies.is_empty() && later_steps > 0).then_some((u64::from(steps), later_steps));
        if let Some((today, rest)) = pace {
            for patrol in &mut patrols {
                patrol.budget = paced(patrol.fuel, today, today + rest);
            }
        }
        let series_slots = series.iter().copied().max().map_or(0, |m| m as usize + 1);
        let mut new_series = vec![true; series_slots];
        for &s in &state.series_taken {
            if let Some(slot) = new_series.get_mut(s as usize) {
                *slot = false;
            }
        }
        let mut world = Self {
            time_cost,
            fuel_cost: fuel,
            neighbors,
            nodes,
            spots,
            series,
            stock,
            spot_at,
            nn,
            time: std::array::from_fn(|_| vec![NONE; nn * nn]),
            fuel: std::array::from_fn(|_| vec![NONE; nn * nn]),
            prev: std::array::from_fn(|_| Vec::with_capacity(nn)),
            via: std::array::from_fn(|_| vec![Vec::new(); nn * nn]),
            steps,
            tomorrow,
            pace,
            patrols,
            supplies,
            new_series,
            series_slots,
        };
        for m in 0..NM {
            for u in 0..nn {
                world.tree(m, u);
            }
        }
        world
    }

    /// Dijkstra from node `u` under metric `m`; fills the matrices and vias.
    fn tree(&mut self, m: usize, u: usize) {
        let n = self.time_cost.len();
        let mut key = vec![u64::MAX; n];
        let mut time = vec![NONE; n];
        let mut fuel = vec![NONE; n];
        let mut prev = vec![(NONE, 0u8); n];
        let source = self.nodes[u];
        key[source] = 0;
        time[source] = 0;
        fuel[source] = 0;
        let mut heap = BinaryHeap::from([Reverse((0u64, source))]);
        while let Some(Reverse((k, i))) = heap.pop() {
            if k != key[i] {
                continue;
            }
            let (t, f) = (u64::from(self.time_cost[i]), u64::from(self.fuel_cost[i]));
            let edge = match m {
                FAST => t * 1024 + f,
                BAL => (t + f) * 1024 + t,
                _ => (f * 3 + t) * 1024 + t,
            };
            for d in 0..6 {
                let j = self.neighbors[i][d];
                if j == NONE {
                    continue;
                }
                let j = j as usize;
                if k + edge < key[j] {
                    key[j] = k + edge;
                    time[j] = time[i] + self.time_cost[i];
                    fuel[j] = fuel[i] + self.fuel_cost[i];
                    prev[j] = (u32::try_from(i).expect("cell"), u8::try_from(d).expect("dir"));
                    heap.push(Reverse((key[j], j)));
                }
            }
        }
        for v in 0..self.nn {
            let target = self.nodes[v];
            self.time[m][u * self.nn + v] = time[target];
            self.fuel[m][u * self.nn + v] = fuel[target];
            if time[target] == NONE || target == source {
                continue;
            }
            let mut via = Vec::new();
            let mut cell = prev[target].0 as usize;
            while cell != source {
                if self.spot_at[cell] != NONE {
                    via.push((u16::try_from(self.spot_at[cell]).expect("node"), time[cell]));
                }
                cell = prev[cell].0 as usize;
            }
            via.reverse();
            self.via[m][u * self.nn + v] = via;
        }
        self.prev[m].push(prev);
    }

    fn t(&self, m: usize, u: usize, v: usize) -> u32 {
        self.time[m][u * self.nn + v]
    }
    fn f(&self, m: usize, u: usize, v: usize) -> u32 {
        self.fuel[m][u * self.nn + v]
    }

    /// Directions of the `u -> v` path under metric `m`, with origin cells.
    fn path(&self, m: usize, u: usize, v: usize) -> Vec<(usize, Direction)> {
        let prev = &self.prev[m][u];
        let source = self.nodes[u];
        let mut cell = self.nodes[v];
        let mut out = Vec::new();
        while cell != source {
            let (p, d) = prev[cell];
            if p == NONE {
                return Vec::new();
            }
            out.push((p as usize, Direction::ALL[usize::from(d)]));
            cell = p as usize;
        }
        out.reverse();
        out
    }
}

#[derive(Clone)]
struct Solution {
    routes: Vec<Vec<u16>>,
    metric: Vec<u8>,
    /// Per supply: patrols to refuel in order; the last one is escorted.
    serve: Vec<Vec<u8>>,
}

#[derive(Clone, Copy)]
struct Visit {
    node: usize,
    arr: u32,
    dep: u32,
    /// Route index the patrol travels to after this visit.
    next: usize,
}

#[derive(Clone, Default)]
struct Timeline {
    visits: Vec<Visit>,
    /// Leg cut by the end of the day: `(target node, arrival tick)`.
    pending: Option<(usize, u32)>,
    fuel: u32,
    escorted: bool,
    stalled: bool,
    /// Index of the next route entry to travel to.
    next: usize,
}

#[derive(Clone, Default)]
struct SupplyLine {
    /// `(node, depart tick)`: go there on the fastest path, leave at `dep`.
    stops: Vec<(usize, u32)>,
    /// Escort this patrol from the last stop's departure tick.
    escort: Option<usize>,
}

struct Evaluation {
    score: i64,
    /// Today's modelled `(series, balls)`.
    today: (i64, i64),
    /// Series the greedy estimate reaches tomorrow (0 on the last day).
    tomorrow_series: i64,
    lines: Vec<Timeline>,
    supplies: Vec<SupplyLine>,
}

/// A patrol's day-end state, as tomorrow's greedy estimate needs it.
#[derive(Clone, Copy)]
struct Start {
    node: usize,
    offset: u32,
    fuel: u32,
    escorted: bool,
}

impl World {
    /// Advance patrol `p` along its route from the timeline's last visit.
    fn advance(&self, sol: &Solution, p: usize, line: &mut Timeline) {
        let m = usize::from(sol.metric[p]);
        let route = &sol.routes[p];
        line.stalled = false;
        line.pending = None;
        while line.next < route.len() {
            let last = *line.visits.last().expect("timeline starts with the start visit");
            let to = usize::from(route[line.next]);
            let (t, f) = (self.t(m, last.node, to), self.f(m, last.node, to));
            if t == NONE {
                line.next += 1;
                continue;
            }
            if !line.escorted && line.fuel < f {
                line.stalled = true;
                return;
            }
            if !line.escorted {
                line.fuel -= f;
            }
            if last.dep + t > self.steps {
                line.pending = Some((to, last.dep + t));
                return;
            }
            line.next += 1;
            line.visits.push(Visit {
                node: to,
                arr: last.dep + t,
                dep: last.dep + t,
                next: line.next,
            });
        }
    }

    /// Where and when a supply at `(node, tick)` can first share a cell with
    /// patrol timeline `line`: `(visit index, meeting tick)`.
    fn intercept(
        &self,
        line: &Timeline,
        node: usize,
        tick: u32,
        escort: bool,
    ) -> Option<(usize, u32)> {
        for (k, visit) in line.visits.iter().enumerate() {
            let reach = self.t(FAST, node, visit.node);
            if reach == NONE {
                continue;
            }
            let arrive = tick + reach;
            if k == 0 {
                // No reflection at tick 0: a touch needs the patrol to still
                // be standing there at tick 1.
                let lingers = self.time_cost[self.nodes[visit.node]] >= 2 || visit.dep >= 1;
                if arrive == 0 && (escort || lingers) {
                    return Some((0, 0));
                }
                continue;
            }
            if arrive <= visit.arr {
                return Some((k, visit.arr));
            }
        }
        // No moving intercept: meet where the patrol ends up standing.
        if line.pending.is_some() {
            return None;
        }
        let k = line.visits.len() - 1;
        let last = line.visits[k];
        let reach = self.t(FAST, node, last.node);
        if reach == NONE {
            return None;
        }
        let meet = (tick + reach).max(last.arr).max(1);
        (meet <= self.steps).then_some((k, meet))
    }

    #[allow(clippy::too_many_lines, reason = "one pass: timelines, supply meets, picks, score")]
    fn evaluate(&self, sol: &Solution) -> Evaluation {
        let np = self.patrols.len();
        let mut lines: Vec<Timeline> = (0..np)
            .map(|p| Timeline {
                visits: vec![Visit { node: self.patrols[p].start, arr: 0, dep: 0, next: 0 }],
                fuel: self.patrols[p].budget,
                ..Timeline::default()
            })
            .collect();
        for (p, line) in lines.iter_mut().enumerate() {
            self.advance(sol, p, line);
        }
        let mut supplies = Vec::with_capacity(self.supplies.len());
        for (s, info) in self.supplies.iter().enumerate() {
            let (mut node, mut tick) = (info.start, 0u32);
            let mut out = SupplyLine::default();
            let list = &sol.serve[s];
            for (i, &q) in list.iter().enumerate() {
                let q = usize::from(q);
                let escort = i + 1 == list.len();
                let Some((k, meet)) = self.intercept(&lines[q], node, tick, escort) else {
                    continue;
                };
                let line = &mut lines[q];
                // Undo what the patrol did after the meeting; replay it with
                // the refuelled tank.
                line.visits.truncate(k + 1);
                line.visits[k].dep = meet;
                line.next = line.visits[k].next;
                line.fuel = self.patrols[q].cap;
                line.escorted = escort;
                self.advance(sol, q, line);
                node = line.visits[k].node;
                tick = meet;
                out.stops.push((node, meet));
                if escort {
                    out.escort = Some(q);
                }
            }
            supplies.push(out);
        }

        // Picks: every reflection a patrol stands on a stocked spot.
        let mut events: Vec<(u32, usize, usize)> = Vec::new();
        for (p, line) in lines.iter().enumerate() {
            let m = usize::from(sol.metric[p]);
            let start_cell = self.nodes[line.visits[0].node];
            if self.spot_at[start_cell] != NONE
                && (self.time_cost[start_cell] >= 2
                    || line.visits[0].dep >= 1
                    || line.visits.len() == 1)
            {
                events.push((1, p, self.spot_at[start_cell] as usize));
            }
            for w in line.visits.windows(2) {
                for &(node, off) in &self.via[m][w[0].node * self.nn + w[1].node] {
                    events.push((w[0].dep + off, p, usize::from(node)));
                }
                if w[1].node < self.spots {
                    events.push((w[1].arr.max(1), p, w[1].node));
                }
            }
            if let Some((to, _)) = line.pending {
                let last = line.visits.last().expect("non-empty timeline");
                for &(node, off) in &self.via[m][last.node * self.nn + to] {
                    if last.dep + off <= self.steps {
                        events.push((last.dep + off, p, usize::from(node)));
                    }
                }
            }
        }
        events.sort_unstable();
        let mut stock = self.stock.clone();
        let mut picked = vec![false; np * self.spots];
        let mut covered = vec![false; self.series_slots];
        let (mut series, mut fresh, mut balls) = (0i64, 0i64, 0i64);
        for &(_, p, node) in &events {
            if stock[node] == 0 || picked[p * self.spots + node] {
                continue;
            }
            stock[node] -= 1;
            picked[p * self.spots + node] = true;
            balls += 1;
            let s = self.series[node] as usize;
            if !covered[s] {
                covered[s] = true;
                series += 1;
                if self.new_series[s] {
                    fresh += 1;
                }
            }
        }
        let mut score = fresh * W_NEW + series * W_SERIES + balls * W_BALL;
        let mut tomorrow_series = 0;
        // Idle time is worth a little: prefer the shorter of two equal days.
        for line in &lines {
            let end =
                line.pending.map_or(line.visits.last().expect("non-empty").arr, |_| self.steps);
            score -= i64::from(end) * 20 / i64::from(self.steps.max(1));
        }
        if let Some(tomorrow) = self.tomorrow {
            let starts: Vec<Start> = lines
                .iter()
                .zip(&self.patrols)
                .map(|(line, patrol)| {
                    let (node, offset) = line.pending.map_or_else(
                        || (line.visits.last().expect("non-empty").node, 0),
                        |(to, at)| (to, at - self.steps),
                    );
                    let fuel = match self.pace {
                        // Paced: tomorrow's share of what is really left.
                        Some((_, rest)) => {
                            let left = patrol.fuel - (patrol.budget - line.fuel.min(patrol.budget));
                            paced(left, u64::from(tomorrow), rest)
                        }
                        None => line.fuel,
                    };
                    Start { node, offset, fuel, escorted: line.escorted }
                })
                .collect();
            let (d, b) = self.greedy(&starts, tomorrow, None);
            // Tomorrow's series are worth nearly as much as today's; its
            // balls are not counted (the greedy estimate is too coarse and
            // trading today's sure balls for it measured worse).
            let _ = b;
            tomorrow_series = d;
            score += d * W_SERIES * TOMORROW_NUM / TOMORROW_DEN;
            for (p, start) in starts.iter().enumerate() {
                let full = i64::from(self.patrols[p].cap.min(tomorrow));
                let have = if start.escorted { full } else { i64::from(start.fuel).min(full) };
                score += 150 * have / full.max(1);
            }
        }
        Evaluation { score, today: (series, balls), tomorrow_series, lines, supplies }
    }

    /// Nearest-useful-spot greedy from `starts` over a `steps`-tick day:
    /// `(series, balls)`. With `routes`, also records each patrol's order.
    fn greedy(
        &self,
        starts: &[Start],
        steps: u32,
        mut routes: Option<&mut Vec<Vec<u16>>>,
    ) -> (i64, i64) {
        let np = starts.len();
        let mut at: Vec<usize> = starts.iter().map(|s| s.node).collect();
        let mut tick: Vec<u32> = starts.iter().map(|s| s.offset).collect();
        let mut fuel: Vec<u32> = starts.iter().map(|s| s.fuel).collect();
        let mut done = vec![false; np];
        let mut stock = self.stock.clone();
        let mut picked = vec![false; np * self.spots];
        let mut covered = vec![false; self.series_slots];
        let (mut series, mut balls) = (0i64, 0i64);
        let pick = |p: usize,
                    node: usize,
                    stock: &mut Vec<u32>,
                    picked: &mut Vec<bool>,
                    covered: &mut Vec<bool>,
                    series: &mut i64,
                    balls: &mut i64| {
            if stock[node] == 0 || picked[p * self.spots + node] {
                return;
            }
            stock[node] -= 1;
            picked[p * self.spots + node] = true;
            *balls += 1;
            let s = self.series[node] as usize;
            if !covered[s] {
                covered[s] = true;
                *series += 1;
            }
        };
        for p in 0..np {
            let cell = self.nodes[at[p]];
            if tick[p] == 0 && self.spot_at[cell] != NONE {
                pick(
                    p,
                    self.spot_at[cell] as usize,
                    &mut stock,
                    &mut picked,
                    &mut covered,
                    &mut series,
                    &mut balls,
                );
            }
        }
        while let Some(p) = (0..np).filter(|&p| !done[p]).min_by_key(|&p| (tick[p], p)) {
            let m = if starts[p].escorted { FAST } else { BAL };
            let mut best: Option<(bool, u32, usize)> = None;
            for node in 0..self.spots {
                if stock[node] == 0 || picked[p * self.spots + node] {
                    continue;
                }
                let t = self.t(m, at[p], node);
                if t == NONE || tick[p] + t > steps {
                    continue;
                }
                if !starts[p].escorted && self.f(m, at[p], node) > fuel[p] {
                    continue;
                }
                let key = (covered[self.series[node] as usize], t, node);
                if best.is_none_or(|b| key < b) {
                    best = Some(key);
                }
            }
            let Some((_, t, node)) = best else {
                done[p] = true;
                continue;
            };
            if !starts[p].escorted {
                fuel[p] -= self.f(m, at[p], node);
            }
            tick[p] += t;
            at[p] = node;
            pick(p, node, &mut stock, &mut picked, &mut covered, &mut series, &mut balls);
            if let Some(routes) = routes.as_deref_mut() {
                routes[p].push(u16::try_from(node).expect("node"));
            }
        }
        (series, balls)
    }

    fn initial(&self) -> Solution {
        let np = self.patrols.len();
        // Supplies: neediest patrol first, each supply takes the patrols
        // nearest to it; the last of its list is escorted.
        let mut serve = vec![Vec::new(); self.supplies.len()];
        if !self.supplies.is_empty() {
            let mut order: Vec<usize> = (0..np).collect();
            order.sort_by_key(|&p| (self.patrols[p].fuel, p));
            let per = np.div_ceil(self.supplies.len());
            for p in order {
                let best =
                    (0..self.supplies.len()).filter(|&s| serve[s].len() < per).min_by_key(|&s| {
                        (self.t(FAST, self.supplies[s].start, self.patrols[p].start), s)
                    });
                if let Some(s) = best {
                    serve[s].push(u8::try_from(p).expect("patrol"));
                }
            }
        }
        let escorted: Vec<bool> = (0..np)
            .map(|p| serve.iter().any(|l| l.last().is_some_and(|&q| usize::from(q) == p)))
            .collect();
        let starts: Vec<Start> = (0..np)
            .map(|p| Start {
                node: self.patrols[p].start,
                offset: 0,
                fuel: self.patrols[p].budget,
                escorted: escorted[p],
            })
            .collect();
        let mut routes = vec![Vec::new(); np];
        self.greedy(&starts, self.steps, Some(&mut routes));
        let metric = escorted.iter().map(|&e| u8::from(!e)).collect();
        Solution { routes, metric, serve }
    }

    fn mutate(&self, sol: &mut Solution, rng: &mut Rng) {
        let np = sol.routes.len();
        let spot = |rng: &mut Rng| u16::try_from(rng.below(self.spots)).expect("node");
        match rng.below(10) {
            0 => {
                let p = rng.below(np);
                let at = rng.below(sol.routes[p].len() + 1);
                sol.routes[p].insert(at, spot(rng));
            }
            1 => {
                // Cheapest insertion of a spot the patrol does not visit yet.
                let p = rng.below(np);
                let node = spot(rng);
                if !sol.routes[p].contains(&node) {
                    let at = self.cheapest_slot(sol, p, usize::from(node));
                    sol.routes[p].insert(at, node);
                }
            }
            2 => {
                let p = rng.below(np);
                if !sol.routes[p].is_empty() {
                    let at = rng.below(sol.routes[p].len());
                    sol.routes[p].remove(at);
                }
            }
            3 | 4 => {
                let (p, q) = (rng.below(np), rng.below(np));
                if !sol.routes[p].is_empty() {
                    let from = rng.below(sol.routes[p].len());
                    let node = sol.routes[p].remove(from);
                    let at = if rng.below(2) == 0 {
                        rng.below(sol.routes[q].len() + 1)
                    } else {
                        self.cheapest_slot(sol, q, usize::from(node))
                    };
                    sol.routes[q].insert(at, node);
                }
            }
            5 => {
                let (p, q) = (rng.below(np), rng.below(np));
                if !sol.routes[p].is_empty() && !sol.routes[q].is_empty() {
                    let (i, j) = (rng.below(sol.routes[p].len()), rng.below(sol.routes[q].len()));
                    let (a, b) = (sol.routes[p][i], sol.routes[q][j]);
                    sol.routes[p][i] = b;
                    sol.routes[q][j] = a;
                }
            }
            6 => {
                let p = rng.below(np);
                let len = sol.routes[p].len();
                if len >= 2 {
                    let (i, j) = (rng.below(len), rng.below(len));
                    sol.routes[p][i.min(j)..=i.max(j)].reverse();
                }
            }
            7 => {
                let p = rng.below(np);
                if !sol.routes[p].is_empty() {
                    let at = rng.below(sol.routes[p].len());
                    sol.routes[p][at] = spot(rng);
                }
            }
            8 => {
                let p = rng.below(np);
                sol.metric[p] = u8::try_from(rng.below(NM)).expect("metric");
            }
            _ => {
                if sol.serve.is_empty() {
                    return;
                }
                let s = rng.below(sol.serve.len());
                match rng.below(3) {
                    0 if sol.serve[s].len() >= 2 => {
                        let len = sol.serve[s].len();
                        sol.serve[s].swap(rng.below(len), rng.below(len));
                    }
                    1 if !sol.serve[s].is_empty() => {
                        let from = rng.below(sol.serve[s].len());
                        let q = sol.serve[s].remove(from);
                        let to = rng.below(sol.serve.len());
                        let at = rng.below(sol.serve[to].len() + 1);
                        sol.serve[to].insert(at, q);
                    }
                    _ => {
                        let q = u8::try_from(rng.below(np)).expect("patrol");
                        for list in &mut sol.serve {
                            list.retain(|&x| x != q);
                        }
                        if rng.below(4) > 0 {
                            let at = rng.below(sol.serve[s].len() + 1);
                            sol.serve[s].insert(at, q);
                        }
                    }
                }
            }
        }
    }

    /// Route position where inserting `node` adds the least travel time.
    fn cheapest_slot(&self, sol: &Solution, p: usize, node: usize) -> usize {
        let m = usize::from(sol.metric[p]);
        let route = &sol.routes[p];
        let leg = |a: usize, b: usize| u64::from(self.t(m, a, b));
        (0..=route.len())
            .min_by_key(|&at| {
                let before =
                    if at == 0 { self.patrols[p].start } else { usize::from(route[at - 1]) };
                match route.get(at) {
                    Some(&after) => {
                        let after = usize::from(after);
                        (leg(before, node) + leg(node, after)).saturating_sub(leg(before, after))
                    }
                    None => leg(before, node),
                }
            })
            .unwrap_or(0)
    }

    /// Drop the route tail the day never reaches (keeps mutations useful).
    fn trim(sol: &mut Solution, eval: &Evaluation) {
        for (p, line) in eval.lines.iter().enumerate() {
            let keep = line.next + usize::from(line.pending.is_some() || line.stalled);
            sol.routes[p].truncate(keep);
        }
    }

    fn search(
        &self,
        iters: u32,
        deadline: Option<Instant>,
        seed: u64,
        t0: f64,
        start: Option<Solution>,
    ) -> (Solution, Evaluation) {
        let mut rng = Rng(seed);
        let mut current = start.unwrap_or_else(|| self.initial());
        let mut current_eval = self.evaluate(&current);
        let mut best = current.clone();
        let mut best_score = current_eval.score;
        if self.spots == 0 {
            return (best, current_eval);
        }
        let t1 = 1.0f64;
        let started = Instant::now();
        let mut live_progress = 0.0f64;
        let mut i = 0u32;
        loop {
            let progress = match deadline {
                None => {
                    if i >= iters {
                        break;
                    }
                    f64::from(i) / f64::from(iters.max(1))
                }
                Some(end) => {
                    // Live: cool along the wall-clock window, so the schedule
                    // completes whatever the machine's speed.
                    if i % 256 == 0 {
                        let now = Instant::now();
                        if now >= end {
                            break;
                        }
                        let window = end.duration_since(started).as_secs_f64().max(1e-3);
                        live_progress = now.duration_since(started).as_secs_f64() / window;
                    }
                    live_progress
                }
            };
            i += 1;
            let temperature = t0 * (t1 / t0).powf(progress);
            let mut next = current.clone();
            self.mutate(&mut next, &mut rng);
            let eval = self.evaluate(&next);
            #[allow(clippy::cast_precision_loss, reason = "score deltas are small")]
            let delta = (eval.score - current_eval.score) as f64;
            if delta >= 0.0 || rng.unit() < (delta / temperature).exp() {
                Self::trim(&mut next, &eval);
                current = next;
                current_eval = eval;
                if current_eval.score > best_score {
                    best_score = current_eval.score;
                    best = current.clone();
                }
            }
        }
        let eval = self.evaluate(&best);
        (best, eval)
    }

    /// Per-agent sequential answer lists for an evaluated solution.
    fn emit(&self, sol: &Solution, eval: &Evaluation, agents: usize) -> Vec<Vec<Action>> {
        let mut lists: Vec<Vec<Action>> = vec![Vec::new(); agents];
        // Patrol action lists with the tick each action starts at.
        let mut timed: Vec<Vec<(u32, Option<Direction>)>> = Vec::new();
        for (p, line) in eval.lines.iter().enumerate() {
            let m = usize::from(sol.metric[p]);
            let mut out = Vec::new();
            let mut tick = 0u32;
            let mut legs: Vec<(usize, usize)> =
                line.visits.windows(2).map(|w| (w[0].node, w[1].node)).collect();
            if let Some((to, _)) = line.pending {
                legs.push((line.visits.last().expect("non-empty").node, to));
            }
            for (k, &(from, to)) in legs.iter().enumerate() {
                while tick < line.visits[k].dep {
                    out.push((tick, None));
                    tick += 1;
                }
                for (cell, dir) in self.path(m, from, to) {
                    if tick + self.time_cost[cell] > self.steps {
                        break;
                    }
                    out.push((tick, Some(dir)));
                    tick += self.time_cost[cell];
                }
            }
            timed.push(out);
        }
        for (p, actions) in timed.iter().enumerate() {
            let agent = self.patrols[p].agent;
            lists[agent] = actions.iter().map(|&(_, dir)| action(agent, dir)).collect();
        }
        for (s, line) in eval.supplies.iter().enumerate() {
            let agent = self.supplies[s].agent;
            let mut node = self.supplies[s].start;
            let mut tick = 0u32;
            let mut out = Vec::new();
            for &(stop, dep) in &line.stops {
                for (cell, dir) in self.path(FAST, node, stop) {
                    out.push(action(agent, Some(dir)));
                    tick += self.time_cost[cell];
                }
                while tick < dep {
                    out.push(action(agent, None));
                    tick += 1;
                }
                node = stop;
            }
            if let Some(q) = line.escort {
                out.extend(
                    timed[q]
                        .iter()
                        .filter(|&&(at, _)| at >= tick)
                        .map(|&(_, dir)| action(agent, dir)),
                );
            }
            lists[agent] = out;
        }
        lists
    }
}

/// Share of `fuel` for a day of `day` steps out of `total` remaining steps,
/// with headroom so a day that needs a little more is not cut short.
fn paced(fuel: u32, day: u64, total: u64) -> u32 {
    let share = u64::from(fuel) * day * 130 / (total.max(1) * 100);
    u32::try_from(share).unwrap_or(u32::MAX).min(fuel)
}

fn action(agent: usize, dir: Option<Direction>) -> Action {
    let id = u8::try_from(agent).expect("agent id");
    dir.map_or_else(|| Action::wait(id), |d| Action::step(id, d))
}

/// One day replayed through the official simulator.
struct Replay {
    /// No illegal, fuel-short or step-mismatched action.
    clean: bool,
    variety: usize,
    series_today: usize,
    balls_today: u32,
    /// Predicted next day-start `(wire cell index, fuel)` per agent.
    ends: Vec<(usize, u32)>,
    /// The state the next day starts from.
    state: MatchState,
}

fn replay(ctx: &SolverContext<'_>, plan: &AgentActions) -> Replay {
    let mut state = ctx.state.clone();
    let before = (state.illegal_action_count, state.rejected_response_count);
    let held: u32 = state.agents.iter().map(engine::Agent::total_inventory).sum();
    let sim = MatchSim::default();
    for step in plan.actions.chunks(ctx.agent_ids.len().max(1)) {
        sim.step(&mut state, &AgentActions::new(step.to_vec()));
    }
    let width = usize::from(state.setting.map.size.width);
    let total: u32 = state.agents.iter().map(engine::Agent::total_inventory).sum();
    Replay {
        clean: (state.illegal_action_count, state.rejected_response_count) == before,
        variety: state.series_taken.len(),
        series_today: state
            .series_acquired_by_day
            .get(&ctx.day)
            .map_or(0, std::collections::BTreeSet::len),
        balls_today: total.saturating_sub(held),
        ends: state
            .agents
            .iter()
            .map(|a| (cell_index(width, a.pos.col, a.pos.row), a.fuel))
            .collect(),
        state,
    }
}

/// One chain's finished day: its plan, the official replay of it, and what
/// the same planner then makes of tomorrow.
struct Candidate {
    plan: AgentActions,
    check: Replay,
    model: (i64, i64),
    /// Tomorrow's `(series, balls)` from the plan's end state.
    tomorrow: (i64, i64),
}

impl Candidate {
    /// Clean first; then variety, series and balls over today and tomorrow.
    fn key(&self) -> (bool, usize, i64, i64) {
        (
            self.check.clean,
            self.check.variety,
            i64::try_from(self.check.series_today).unwrap_or(i64::MAX) + self.tomorrow.0,
            i64::from(self.check.balls_today) + self.tomorrow.1,
        )
    }
}

impl FleetCandidate {
    fn seed(&self, ctx: &SolverContext<'_>, k: usize) -> u64 {
        self.seed ^ (u64::from(ctx.day) << 32) ^ ((k as u64) << 48)
    }

    /// Turn an evaluated solution into the day's answer and replay it.
    fn finish(
        ctx: &SolverContext<'_>,
        world: &World,
        sol: &Solution,
        eval: &Evaluation,
    ) -> (AgentActions, Replay) {
        let n = ctx.state.agents.len();
        let lists = world.emit(sol, eval, n);
        let steps = ctx.day_steps as usize;
        let mut output = Vec::with_capacity(n * steps);
        for step in 0..steps {
            for (i, list) in lists.iter().enumerate() {
                output.push(list.get(step).copied().unwrap_or_else(|| action(i, None)));
            }
        }
        let plan =
            crate::adapters::walk::BudgetMode::Official.apply(ctx, AgentActions::new(output));
        let check = replay(ctx, &plan);
        (plan, check)
    }
}

/// Run `job` for every chain: on threads when live, in turn when offline (so
/// an `iters` run is reproducible).
fn per_chain<T: Send>(chains: usize, threaded: bool, job: impl Fn(usize) -> T + Sync) -> Vec<T> {
    if threaded && chains > 1 {
        std::thread::scope(|scope| {
            let job = &job;
            let handles: Vec<_> = (0..chains).map(|k| scope.spawn(move || job(k))).collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        })
    } else {
        (0..chains).map(job).collect()
    }
}

impl Solver for FleetCandidate {
    fn name(&self) -> &'static str {
        "fleet"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let started = Instant::now();
        let fallback =
            || CoverageCandidate { park_risk: false, variant: 0, fuel_aware: false }.decide(ctx);
        let state = ctx.state;
        // Agent ids index the answer lists; anything else is not this
        // candidate's contract.
        if state.agents.iter().enumerate().any(|(i, a)| usize::from(a.id) != i)
            || !state.agents.iter().any(|a| a.kind == AgentKind::Patrol)
        {
            return fallback();
        }
        let later: Vec<u32> =
            (ctx.day + 1..state.setting.num_days).map(|d| state.setting.day_steps_for(d)).collect();
        let world = World::new(state, ctx.day_steps, &later);
        let window = (self.budget_ms > 0).then(|| {
            // Balls stop improving once the search has converged, and from
            // there a longer answer only costs the fourth ranking axis. The
            // converged budget grew with (patrols x spots)^2 on the corpus:
            // 16x16 boards were flat from 0.25 s, 24x24 from about 2 s, and
            // 32x32 still gained at 4 s.
            let scale = (world.patrols.len() * world.spots) as u64;
            let converged = (scale * scale / 3).max(400);
            let cap = u64::from(self.budget_ms);
            let budget = Duration::from_millis(if self.adapt { cap.min(converged) } else { cap });
            if ctx.remaining_ms == u32::MAX {
                budget
            } else {
                // The coordinator already holds back its submission margin
                // (`solver_budget_ms`), so only the replay check and encoding
                // are left to cover. A quarter second of search is kept even
                // when it reports no time: the greedy start alone measured
                // far weaker than a short search.
                budget.min(Duration::from_millis(u64::from(
                    ctx.remaining_ms.saturating_sub(300).max(250),
                )))
            }
        });
        // Live: one chain per spare core. Offline: `chains`.
        let live = window.is_some();
        let chains = if live {
            std::thread::available_parallelism().map_or(1, usize::from).clamp(1, 4)
        } else {
            self.chains as usize
        };
        // Several chains get a second phase on 30% of the window (offline: a
        // third of `iters`); one chain spends everything on the day plan.
        let second = chains > 1;
        let plan_until = window.map(|w| started + if second { w * 7 / 10 } else { w });
        let until = window.map(|w| started + w);
        let first = per_chain(chains, live, |k| {
            world.search(self.iters, plan_until, self.seed(ctx, k), CHAIN_T0[k % 4], None)
        });
        // Is tomorrow's variety at stake? If every chain covers every series
        // today and the mere greedy estimate covers them all tomorrow, the
        // chains differ only in balls: polish them. Otherwise let the same
        // planner play tomorrow from each chain's end and compare.
        let all_series =
            i64::try_from(world.series.iter().collect::<std::collections::BTreeSet<_>>().len())
                .unwrap_or(i64::MAX);
        let settled = later.is_empty()
            || first
                .iter()
                .all(|(_, e)| e.today.0 == all_series && e.tomorrow_series == all_series);
        let candidates: Vec<Candidate> = per_chain(first.len(), live, |k| {
            let (sol, eval) = &first[k];
            if !second {
                let (plan, check) = Self::finish(ctx, &world, sol, eval);
                return Candidate { plan, check, model: eval.today, tomorrow: (0, 0) };
            }
            let seed = self.seed(ctx, k) ^ 0x5bd1_e995;
            if settled {
                let (sol, eval) = world.search(self.iters / 3, until, seed, 5.0, Some(sol.clone()));
                let (plan, check) = Self::finish(ctx, &world, &sol, &eval);
                return Candidate { plan, check, model: eval.today, tomorrow: (0, 0) };
            }
            let (plan, check) = Self::finish(ctx, &world, sol, eval);
            let tomorrow = match later.split_first() {
                Some((&steps, rest)) if check.clean => {
                    let next = World::new(&check.state, steps, rest);
                    next.search(self.iters / 3, until, seed, CHAIN_T0[0], None).1.today
                }
                _ => (0, 0),
            };
            Candidate { plan, check, model: eval.today, tomorrow }
        });
        // Ties keep the first chain (`max_by_key` returns the last maximum).
        let Some(best) = candidates.into_iter().rev().max_by_key(Candidate::key) else {
            return fallback();
        };
        // Evidence line for live runs: what the model promised, what the
        // official replay delivers, and where the server should report the
        // agents tomorrow. Game state only.
        tracing::info!(
            day = ctx.day,
            clean = best.check.clean,
            chains,
            model_series = best.model.0,
            model_balls = best.model.1,
            replay_series = best.check.series_today,
            replay_balls = best.check.balls_today,
            tomorrow_series = best.tomorrow.0,
            tomorrow_balls = best.tomorrow.1,
            predicted_ends = ?best.check.ends,
            "fleet day plan"
        );
        if best.check.clean { best.plan } else { fallback() }
    }
}

/// Outcome of one pre-match composition trial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindsTrial {
    pub kinds: Vec<AgentKind>,
    /// `(variety, daily variety sum, balls)` of the simulated match.
    pub score: (u32, u32, u32),
}

/// Search length of a composition trial: two whole-match simulations of an
/// 8-agent, 10-day board still end within a couple of seconds.
const KINDS_TRIAL_ITERS: u32 = 8_000;

/// Choose the `POST /agent` composition by simulating the match (Issue #371).
///
/// Two trials play the whole match offline with a short `fleet` search: one
/// Supply (slot 0) and no Supply at all. The all-Patrol team is chosen only
/// when it loses nothing on variety and daily variety and gains at least 5%
/// balls — it fields one more Patrol, which wins short, fuel-rich matches by
/// about 20% balls, but nothing can refuel it if a day drifts from the
/// simulation. Which slot holds the Supply is not searched: short trials
/// could not rank the slots reliably (measured: choosing by trial lost daily
/// variety on tight boards).
///
/// A trial still running at `window` is ignored; without both trials the
/// answer is one Supply in slot 0, the composition the offline evidence was
/// measured with.
#[must_use]
pub fn choose_kinds(
    wire: &protocol::match_config::MatchSetting,
    team: &domain::PlayerId,
    window: Duration,
) -> (Vec<AgentKind>, Vec<KindsTrial>) {
    let n = wire.agents.len();
    let one_supply = |slot: usize| -> Vec<AgentKind> {
        (0..n).map(|i| if i == slot { AgentKind::Supply } else { AgentKind::Patrol }).collect()
    };
    let fallback = if n >= 2 { one_supply(0) } else { vec![AgentKind::Patrol; n] };
    let Ok(config) = engine::from_wire::sim_config_from_wire(wire) else {
        return (fallback, Vec::new());
    };
    let mut compositions: Vec<Vec<AgentKind>> = vec![vec![AgentKind::Patrol; n]];
    if n >= 2 {
        compositions.push(one_supply(0));
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    for kinds in compositions {
        let Ok(setting) = engine::from_wire::protocol_setting_to_engine_with_kinds(
            wire,
            team.clone(),
            Some(&kinds),
        ) else {
            continue;
        };
        let sender = sender.clone();
        std::thread::spawn(move || {
            let solver = FleetCandidate {
                iters: KINDS_TRIAL_ITERS,
                budget_ms: 0,
                seed: 42,
                chains: 1,
                adapt: true,
            };
            if let Ok(result) = MatchSim::new(config).run(setting, &solver) {
                if result.rejected_response_count == 0 {
                    let score = (result.variety, result.daily_variety_sum, result.total_balls);
                    // A closed channel means the window is over.
                    let _ = sender.send(KindsTrial { kinds, score });
                }
            }
        });
    }
    drop(sender);
    let deadline = Instant::now() + window;
    let mut trials = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match receiver.recv_timeout(left) {
            Ok(trial) => trials.push(trial),
            Err(_) => break,
        }
    }
    // Report order: the one-Supply trial first.
    trials.sort_by_key(|t| !t.kinds.contains(&AgentKind::Supply));
    let supplied = trials.iter().find(|t| t.kinds.contains(&AgentKind::Supply));
    let all_patrol = trials.iter().find(|t| !t.kinds.contains(&AgentKind::Supply));
    let chosen = match (supplied, all_patrol) {
        (Some(one), Some(zero)) => {
            let keeps_variety = (zero.score.0, zero.score.1) >= (one.score.0, one.score.1);
            let gains = u64::from(zero.score.2) * 100 >= u64::from(one.score.2) * 105;
            if keeps_variety && gains { zero.kinds.clone() } else { one.kinds.clone() }
        }
        _ => fallback,
    };
    (chosen, trials)
}

/// One Supply slot's pre-match trials: whole matches played with a short
/// `fleet` search while our own road use is counted as every team's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotTrial {
    pub slot: usize,
    /// Matches played to the end inside the window.
    pub matches: u32,
    /// `(variety, daily variety sum, balls)` summed over those matches.
    pub total: (u32, u32, u32),
}

/// Mean daily-variety gap (in series per match, as a fraction `num / den`)
/// below which two slots count as level on the second ranking axis and the
/// balls decide. Short trials scatter by about half a series per match.
const SLOT_VARIETY_TIE: (u32, u32) = (1, 4);

/// Which agent should be the Supply (Issue #428).
///
/// `choose_kinds` always puts the one Supply in slot 0. The starts differ,
/// though: on boards with the round-one frame the first day is `W + H` steps
/// long, and a start that sits next to several series is worth more as a
/// Patrol than as the Supply. This plays every single-Supply composition
/// through whole matches — as many rounds of different seeds as fit in
/// `window` — and returns the slot with the best mean score in the official
/// order. Roads are classified as if every team moved like us
/// (`team_count = 1`): with the other teams counted as absent the roads never
/// jam and the trial overstates what road-side starts are worth.
///
/// Only complete rounds count, so every slot is judged on the same seeds.
/// Without one complete round the answer is `None` (keep slot 0).
#[must_use]
pub fn choose_supply_slot(
    wire: &protocol::match_config::MatchSetting,
    team: &domain::PlayerId,
    window: Duration,
) -> (Option<usize>, Vec<SlotTrial>) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let n = wire.agents.len();
    let Ok(mut config) = engine::from_wire::sim_config_from_wire(wire) else {
        return (None, Vec::new());
    };
    config.team_count = 1;
    let settings: Vec<_> = (0..n)
        .filter_map(|slot| {
            let kinds: Vec<AgentKind> = (0..n)
                .map(|i| if i == slot { AgentKind::Supply } else { AgentKind::Patrol })
                .collect();
            engine::from_wire::protocol_setting_to_engine_with_kinds(
                wire,
                team.clone(),
                Some(&kinds),
            )
            .ok()
        })
        .collect();
    if n < 3 || settings.len() != n || window.is_zero() {
        return (None, Vec::new());
    }
    let settings = Arc::new(settings);
    let next = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = std::sync::mpsc::channel();
    let threads = std::thread::available_parallelism().map_or(1, usize::from).clamp(1, 16);
    for _ in 0..threads {
        let (settings, next, stop, sender) =
            (Arc::clone(&settings), Arc::clone(&next), Arc::clone(&stop), sender.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let job = next.fetch_add(1, Ordering::Relaxed);
                let (round, slot) = (job / settings.len(), job % settings.len());
                let solver = FleetCandidate {
                    iters: KINDS_TRIAL_ITERS,
                    budget_ms: 0,
                    seed: 42 + round as u64,
                    chains: 1,
                    adapt: true,
                };
                let score = MatchSim::new(config)
                    .run(settings[slot].clone(), &solver)
                    .ok()
                    .filter(|r| r.rejected_response_count == 0)
                    .map_or((0, 0, 0), |r| (r.variety, r.daily_variety_sum, r.total_balls));
                if sender.send((round, slot, score)).is_err() {
                    return;
                }
            }
        });
    }
    drop(sender);
    let deadline = Instant::now() + window;
    let mut rounds: Vec<Vec<Option<(u32, u32, u32)>>> = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match receiver.recv_timeout(left) {
            Ok((round, slot, score)) => {
                if rounds.len() <= round {
                    rounds.resize(round + 1, vec![None; n]);
                }
                rounds[round][slot] = Some(score);
            }
            Err(_) => break,
        }
    }
    stop.store(true, Ordering::Relaxed);
    let mut trials: Vec<SlotTrial> =
        (0..n).map(|slot| SlotTrial { slot, matches: 0, total: (0, 0, 0) }).collect();
    for round in rounds.iter().filter(|r| r.iter().all(Option::is_some)) {
        for (trial, score) in trials.iter_mut().zip(round) {
            let (v, d, b) = score.expect("complete round");
            trial.matches += 1;
            trial.total = (trial.total.0 + v, trial.total.1 + d, trial.total.2 + b);
        }
    }
    if trials[0].matches == 0 {
        return (None, Vec::new());
    }
    let chosen = best_slot(&trials);
    (chosen, trials)
}

/// The slot with the best trial total in the official order. Every slot
/// must have played the same number of matches, so totals compare like
/// means. Slots within [`SLOT_VARIETY_TIE`] of the best daily variety are
/// level on that axis; a full tie keeps the lowest slot.
#[must_use]
pub fn best_slot(trials: &[SlotTrial]) -> Option<usize> {
    let played = trials.first()?.matches;
    let best_v = trials.iter().map(|t| t.total.0).max()?;
    let best_d = trials.iter().filter(|t| t.total.0 == best_v).map(|t| t.total.1).max()?;
    let slack = played * SLOT_VARIETY_TIE.0 / SLOT_VARIETY_TIE.1;
    trials
        .iter()
        .filter(|t| t.total.0 == best_v && t.total.1 + slack >= best_d)
        // `max_by_key` returns the last maximum: reversed, the lowest slot.
        .rev()
        .max_by_key(|t| t.total.2)
        .map(|t| t.slot)
}
