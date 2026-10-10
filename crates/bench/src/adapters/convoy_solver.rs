//! Convoy planning: each patrol's day solved exactly on the cell clock, a
//! supply that may join its patrol on any cell at any tick, and day plans
//! compared by playing the remaining days (Issue #405).
//!
//! `fleet` routes a patrol over an ordered spot list and lets a supply touch
//! it only on a visited spot or where it stalls. On the 2026-10-10 round-one
//! board that hides the one day-1 plan worth nine series: the patrol has to
//! be refuelled between two spots without losing a tick. This candidate
//! drops the spot-list model for the day's inner problem:
//!
//! 1. **Patrol day** — a label search over `(cell, collected spots)` keeping
//!    the Pareto front of `(tick, fuel)`. Moves use the official departure
//!    clock and fuel. A patrol whose supply can already be on the cell it
//!    lands on is either *escorted* from there (the supply mirrors its moves,
//!    so the tank no longer binds) or *touched* (topped up once; the supply
//!    leaves for another patrol).
//! 2. **Team day** — a supply touches at most one patrol and then escorts
//!    another. Every patrol's reachable `(spots, end cell, end fuel)` options
//!    are combined by best response under the official order (new series,
//!    series today, balls) plus a small end-state term.
//! 3. **Match** — the day plans of every touch / escort choice (and `fleet`'s
//!    own plan) are replayed through [`engine::MatchSim`], and a beam over
//!    day-end states plays the remaining days under today's road state. The
//!    first-day plan of the best whole-match path is answered.
//!
//! Every answered plan has passed the official replay. When the board is
//! outside what the label search handles (more than 32 spots, or the label
//! cap), the answer is `fleet`'s plan unchanged.
#![allow(
    clippy::many_single_char_names,
    reason = "search code: c = cell, m = mask, t = tick, f = fuel, p = patrol, s = supply"
)]
use std::collections::{BinaryHeap, HashMap};
use std::hash::{BuildHasherDefault, Hasher};
use std::time::{Duration, Instant};

use crate::adapters::coverage_solver::mix;
use crate::adapters::fleet_solver::FleetCandidate;
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use engine::MatchSim;
use engine::model::{
    Action, AgentActions, AgentKind, Direction, MatchState, Solver, SolverContext, Terrain,
    TrafficState,
};
use engine::rules::fuel_cost;

pub struct ConvoyFactory;
const PARAMS: &[ParamSpec] = &[
    ParamSpec { key: "budget", required: true, doc: "official (required)" },
    ParamSpec {
        key: "budget_ms",
        required: false,
        doc: "wall-clock cap per day plan, 1..59000 (default 0: no clock, deterministic)",
    },
    ParamSpec {
        key: "fleet_iters",
        required: false,
        doc: "search length of the embedded fleet plan for today (default 60000)",
    },
    ParamSpec {
        key: "fleet_chains",
        required: false,
        doc: "chains of the embedded fleet plan for today, 1..8 (default 4)",
    },
    ParamSpec { key: "seed", required: false, doc: "seed of the embedded fleet plan (default 42)" },
    ParamSpec {
        key: "fuel_need",
        required: false,
        doc: "fleet's fuel_need for today's embedded plan, 0..=300 (default 0: off)",
    },
    ParamSpec {
        key: "fuel_mix",
        required: false,
        doc: "fleet's fuel_mix for today's embedded plan: off (default) | on",
    },
    ParamSpec {
        key: "depth",
        required: false,
        doc: "days played ahead when comparing day plans (default 8; 0 = today only)",
    },
    ParamSpec {
        key: "forecast",
        required: false,
        doc: "road state assumed for later days: persist (today's) | busy (today's, with \
              every smooth road busy) | both (default: summed over the two)",
    },
    ParamSpec {
        key: "width",
        required: false,
        doc: "day-end states kept per day ahead (default 24)",
    },
    ParamSpec {
        key: "labels",
        required: false,
        doc: "label cap of one patrol-day search (default 1500000)",
    },
    ParamSpec {
        key: "bail",
        required: false,
        doc: "when the patrol-day label search fails, answer the embedded fleet's plan \
              instead of burning the rest of the window on a one-node beam: on | off \
              (default off)",
    },
    ParamSpec {
        key: "cluster",
        required: false,
        doc: "partition the spots into geographic clusters, one per patrol, and count \
              only the patrol's own cluster's spots in its label-search mask: on | off \
              (default off)",
    },
    ParamSpec {
        key: "clusters",
        required: false,
        doc: "cluster count when cluster=on, 1..=8; 0 = auto (min(patrols, 4)) \
              (default 0)",
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

impl CandidateFactory for ConvoyFactory {
    fn algorithm(&self) -> &'static str {
        "convoy"
    }
    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }
    fn describe(&self) -> &'static str {
        "Exact patrol-day label search with mid-leg supply joins and whole-match rollouts"
    }
    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        if spec.param("budget") != Some("official") {
            return Err(CandidateError::Build("convoy requires budget=official".into()));
        }
        let budget_ms = parse(spec, "budget_ms", 0u32)?;
        if budget_ms > 59_000 {
            return Err(CandidateError::Build(format!(
                "budget_ms must be 0..=59000, got {budget_ms}"
            )));
        }
        let seed = parse(spec, "seed", 42u64)?;
        let chains = parse(spec, "fleet_chains", 4u32)?;
        if !(1..=8).contains(&chains) {
            return Err(CandidateError::Build(format!("fleet_chains must be 1..=8, got {chains}")));
        }
        Ok(Box::new(ConvoyCandidate {
            fleet: FleetCandidate {
                iters: parse(spec, "fleet_iters", 60_000u32)?,
                // Live: a quarter of the window, at most four seconds.
                budget_ms: (budget_ms / 4).min(4_000),
                seed,
                chains,
                adapt: false,
                fuel_need: parse(spec, "fuel_need", 0u32)?.min(300),
                fuel_mix: spec.param("fuel_mix") == Some("on"),
                second_pct: 70,
            },
            ahead: FleetCandidate {
                iters: 20_000,
                budget_ms: 0,
                seed,
                chains: 1,
                adapt: true,
                fuel_need: 0,
                fuel_mix: false,
                second_pct: 70,
            },
            budget_ms,
            scenarios: match spec.param("forecast") {
                None | Some("both") => vec![false, true],
                Some("persist") => vec![false],
                Some("busy") => vec![true],
                Some(other) => {
                    return Err(CandidateError::Build(format!("unknown forecast {other:?}")));
                }
            },
            depth: parse(spec, "depth", 8u32)?,
            width: parse(spec, "width", 24usize)?.max(1),
            labels: parse(spec, "labels", 1_500_000usize)?,
            bail: match spec.param("bail") {
                None | Some("off") => false,
                Some("on") => true,
                Some(other) => {
                    return Err(CandidateError::Build(format!("unknown bail mode {other:?}")));
                }
            },
            cluster: match spec.param("cluster") {
                None | Some("off") => false,
                Some("on") => true,
                Some(other) => {
                    return Err(CandidateError::Build(format!("unknown cluster mode {other:?}")));
                }
            },
            clusters: {
                let k = parse(spec, "clusters", 0u32)?;
                if k > 8 {
                    return Err(CandidateError::Build(format!("clusters must be 0..=8, got {k}")));
                }
                if k > 0 && spec.param("cluster") != Some("on") {
                    return Err(CandidateError::Build("clusters requires cluster=on".into()));
                }
                k
            },
        }))
    }
}

pub struct ConvoyCandidate {
    /// `fleet` as it plans today; its plan is the one to beat.
    pub fleet: FleetCandidate,
    /// A cheap `fleet` for the days played ahead.
    pub ahead: FleetCandidate,
    pub budget_ms: u32,
    /// Road forecasts the days ahead are played under: `false` keeps
    /// today's state, `true` also turns every smooth road busy.
    pub scenarios: Vec<bool>,
    pub depth: u32,
    pub width: usize,
    pub labels: usize,
    /// When the label search fails and the pool holds only `fleet`'s plan,
    /// answer it instead of playing the remaining days on a one-node beam
    /// (Issue #432). The beam provably cannot change that answer: with a
    /// single root, `best` is always 0. The saved budget is not handed back
    /// to the fleet — the fleet has already searched with a quarter of the
    /// window, and returning early is what keeps the live response short.
    pub bail: bool,
    /// Partition the board's spots into geographic clusters so each patrol's
    /// label-search mask only carries its own cluster's spots (Issue #432):
    /// the mask dimension drops from 2^(all spots) to 2^(cluster spots).
    pub cluster: bool,
    /// Cluster count when [`Self::cluster`] is on; `0` = auto, resolved per
    /// day as `min(patrols, 4)`.
    pub clusters: u32,
}

const NONE: u32 = u32::MAX;
const ESCORTED: u16 = u16::MAX;
const WAIT: u8 = 6;
const W_NEW: i64 = 1_000_000;
const W_SERIES: i64 = 10_000;
const W_BALL: i64 = 100;
/// End-state terms `(full tank, standing on a spot)` tried for each escort
/// choice; the rollout decides which one the day needed.
const VARIANTS: [(i64, i64); 3] = [(150, 60), (0, 0), (400, 250)];

#[derive(Default)]
struct KeyHasher(u64);
impl Hasher for KeyHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = mix(self.0 ^ u64::from(b));
        }
    }
    fn write_u64(&mut self, x: u64) {
        self.0 = mix(x);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}
type KeyMap<V> = HashMap<u64, V, BuildHasherDefault<KeyHasher>>;

fn cell_index(width: usize, col: i32, row: i32) -> usize {
    usize::try_from(row).expect("in-bounds row") * width
        + usize::try_from(col).expect("in-bounds col")
}

/// Partition the board's spots into geographic clusters: repeated median
/// splits of the largest group on its wider axis (column vs row span, ties
/// toward columns), member spots ordered by coordinate and then spot index.
/// Deterministic, coordinates only, no randomness. Patrol `p` is responsible
/// for cluster `p % groups`; with `k == 0` the count is `min(patrols, 4)`.
fn split_clusters(coords: &[(i32, i32)], patrols: usize, k: u32) -> Clusters {
    let k = match k {
        0 => patrols.min(4),
        k => (k as usize).clamp(1, patrols),
    };
    let mut members: Vec<Vec<usize>> = vec![(0..coords.len()).collect()];
    while members.len() < k {
        let g = (0..members.len())
            .min_by_key(|&i| (std::cmp::Reverse(members[i].len()), i))
            .expect("groups");
        if members[g].len() < 2 {
            break;
        }
        let (mut lo_c, mut hi_c, mut lo_r, mut hi_r) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
        for &s in &members[g] {
            let (c, r) = coords[s];
            lo_c = lo_c.min(c);
            hi_c = hi_c.max(c);
            lo_r = lo_r.min(r);
            hi_r = hi_r.max(r);
        }
        let by_col = hi_c - lo_c >= hi_r - lo_r;
        let mut sorted = members[g].clone();
        sorted.sort_by_key(|&s| if by_col { (coords[s].0, s) } else { (coords[s].1, s) });
        let half = sorted.len() / 2;
        let upper = sorted.split_off(half);
        members[g] = sorted;
        members.push(upper);
    }
    // A spot outside the patrol's cluster gets no bit: passing through it is
    // free, picking it scores balls only (in the replay, never the search).
    let bit: Vec<Vec<u32>> = (0..patrols)
        .map(|p| {
            let cluster = p % members.len();
            let mut row = vec![NONE; coords.len()];
            for (b, &s) in members[cluster].iter().enumerate() {
                row[s] = u32::try_from(b).expect("cluster size");
            }
            row
        })
        .collect();
    Clusters { bit }
}

/// One day's static board.
struct Board {
    time: Vec<u32>,
    fuel: Vec<u32>,
    nbr: Vec<[u32; 6]>,
    /// `spot_at[cell]` = spot index, or `NONE`.
    spot_at: Vec<u32>,
    series: Vec<usize>,
    stock: Vec<u32>,
    series_count: usize,
    new_series: Vec<bool>,
    steps: u32,
    patrols: Vec<Unit>,
    supplies: Vec<Unit>,
    /// `cluster=on`: spot clusters and each patrol's spot→mask-bit tables.
    /// `None` keeps the global spot-index bit of the unpartitioned search.
    cluster: Option<Clusters>,
}

/// Geographic spot clusters (Issue #432). A series whose spots fall in two
/// clusters lands in both clusters' masks: both patrols may pick it, and
/// `tally` charges the stock per visitor as before.
struct Clusters {
    /// `bit[p][s]` = the bit of global spot `s` in patrol `p`'s mask, or
    /// `NONE` when `s` is outside `p`'s cluster (picked balls only).
    bit: Vec<Vec<u32>>,
}

#[derive(Clone, Copy)]
struct Unit {
    agent: usize,
    cell: usize,
    fuel: u32,
    cap: u32,
}

impl Board {
    fn new(state: &MatchState, steps: u32, cluster: Option<u32>) -> Option<Self> {
        let map = &state.setting.map;
        let width = usize::from(map.size.width);
        let cells = map.cells();
        let time = cells
            .iter()
            .map(|c| engine::wire_plan::terrain_step_cost(c.terrain).unwrap_or(NONE))
            .collect();
        let fuel = cells.iter().map(|c| fuel_cost(c.terrain)).collect();
        let nbr = cells
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
        let mut spot_at = vec![NONE; cells.len()];
        let mut ids: Vec<u32> = Vec::new();
        let (mut series, mut stock) = (Vec::new(), Vec::new());
        for (i, c) in cells.iter().enumerate() {
            if let Some(spot) = &c.spot {
                spot_at[i] = u32::try_from(series.len()).expect("spot");
                let slot = ids.iter().position(|&s| s == spot.series).unwrap_or_else(|| {
                    ids.push(spot.series);
                    ids.len() - 1
                });
                series.push(slot);
                stock.push(spot.max_stock);
            }
        }
        if series.len() > 32 || cells.len() > usize::from(u16::MAX) || steps >= u32::from(u16::MAX)
        {
            return None;
        }
        let new_series = ids.iter().map(|s| !state.series_taken.contains(s)).collect();
        let (mut patrols, mut supplies) = (Vec::new(), Vec::new());
        for (agent, a) in state.agents.iter().enumerate() {
            let unit = Unit {
                agent,
                cell: cell_index(width, a.pos.col, a.pos.row),
                fuel: a.fuel,
                cap: a.fuel_cap,
            };
            if a.kind == AgentKind::Patrol {
                patrols.push(unit);
            } else {
                supplies.push(unit);
            }
        }
        // A supply on the same cell tops a patrol up at tick 1 whatever either
        // of them does, as long as leaving the cell takes two ticks or more.
        let time: Vec<u32> = time;
        for patrol in &mut patrols {
            if time[patrol.cell] >= 2 && supplies.iter().any(|s: &Unit| s.cell == patrol.cell) {
                patrol.fuel = patrol.cap;
            }
        }
        let cluster = cluster.filter(|_| !series.is_empty() && !patrols.is_empty()).map(|k| {
            let coords: Vec<(i32, i32)> = spot_at
                .iter()
                .enumerate()
                .filter(|&(_, s)| *s != NONE)
                .map(|(i, _)| {
                    let c = &cells[i].coord;
                    (c.col, c.row)
                })
                .collect();
            split_clusters(&coords, patrols.len(), k)
        });
        Some(Self {
            time,
            fuel,
            nbr,
            spot_at,
            series,
            stock,
            series_count: ids.len(),
            new_series,
            steps,
            patrols,
            supplies,
            cluster,
        })
    }

    /// Earliest arrival tick on every cell from `from`, with the tree.
    fn reach(&self, from: usize) -> (Vec<u32>, Vec<(u32, u8)>) {
        let mut dist = vec![NONE; self.time.len()];
        let mut prev = vec![(NONE, 0u8); self.time.len()];
        let mut heap = BinaryHeap::new();
        dist[from] = 0;
        heap.push(std::cmp::Reverse((0u32, from)));
        while let Some(std::cmp::Reverse((d, u))) = heap.pop() {
            if d > dist[u] || self.time[u] == NONE {
                continue;
            }
            for (dir, &v) in self.nbr[u].iter().enumerate() {
                if v == NONE {
                    continue;
                }
                let nd = d + self.time[u];
                if nd < dist[v as usize] {
                    dist[v as usize] = nd;
                    prev[v as usize] =
                        (u32::try_from(u).expect("cell"), u8::try_from(dir).expect("dir"));
                    heap.push(std::cmp::Reverse((nd, v as usize)));
                }
            }
        }
        (dist, prev)
    }
}

#[derive(Clone, Copy)]
struct Label {
    cell: u16,
    mask: u32,
    tick: u16,
    /// Fuel left, or `ESCORTED`.
    fuel: u16,
    parent: u32,
    /// Direction index, or `WAIT` (until this label's tick).
    act: u8,
    /// Topped up by a supply that then left (touch mode).
    touched: bool,
    dead: bool,
}

/// How a patrol's supply treats it today. The slice is the earliest tick the
/// supply can stand on each cell.
#[derive(Clone, Copy)]
enum Supplied<'a> {
    Tank,
    Escort(&'a [u32]),
    Touch(&'a [u32]),
}

/// One way a patrol can spend the day.
#[derive(Clone, Copy)]
struct Opt {
    mask: u32,
    cell: u16,
    fuel: u16,
    label: u32,
}

/// Every non-dominated day of one patrol.
struct PatrolDay {
    labels: Vec<Label>,
    options: Vec<Opt>,
}

impl Board {
    /// Label search for one patrol. Returns `None` when the label cap is hit.
    #[allow(clippy::too_many_lines, reason = "one label-setting loop: waits, moves, options")]
    fn patrol_day(
        &self,
        unit: Unit,
        mode: Supplied<'_>,
        last_day: bool,
        (cap_labels, deadline): (usize, Option<Instant>),
        spot_bit: &[u32],
    ) -> Option<PatrolDay> {
        let steps = u16::try_from(self.steps).ok()?;
        let (supply_at, touch) = match mode {
            Supplied::Tank => (None, false),
            Supplied::Escort(at) => (Some(at), false),
            Supplied::Touch(at) => (Some(at), true),
        };
        let full = u16::try_from(unit.cap).ok()?;
        let key = |l: &Label| {
            (u64::from(l.touched) << 48) | (u64::from(l.cell) << 32) | u64::from(l.mask)
        };
        let joins = |cell: usize, tick: u16| {
            supply_at.is_some_and(|at| at[cell] != NONE && at[cell] <= u32::from(tick))
        };
        let mut labels: Vec<Label> = Vec::new();
        let mut front: KeyMap<Vec<u32>> = KeyMap::default();
        let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); usize::from(steps) + 1];
        // Per `(cell, touched)`: a few labels with the most spots, to drop
        // arrivals that are no earlier, no fuller and hold a strict subset.
        let mut champions: Vec<Vec<u32>> = vec![Vec::new(); self.time.len() * 2];
        let mut push = |labels: &mut Vec<Label>, buckets: &mut Vec<Vec<u32>>, new: Label| {
            let slot = usize::from(new.cell) * 2 + usize::from(new.touched);
            if champions[slot].iter().any(|&i| {
                let l = &labels[i as usize];
                l.mask != new.mask
                    && l.mask & new.mask == new.mask
                    && l.tick <= new.tick
                    && l.fuel >= new.fuel
            }) {
                return;
            }
            let entry = front.entry(key(&new)).or_default();
            if entry.iter().any(|&i| {
                let l = &labels[i as usize];
                l.tick <= new.tick && l.fuel >= new.fuel
            }) {
                return;
            }
            entry.retain(|&i| {
                let l = &mut labels[i as usize];
                let dominated = new.tick <= l.tick && new.fuel >= l.fuel;
                l.dead |= dominated;
                !dominated
            });
            let index = u32::try_from(labels.len()).expect("label index");
            entry.push(index);
            buckets[usize::from(new.tick)].push(index);
            labels.push(new);
            let list = &mut champions[slot];
            let size = |i: u32| labels[i as usize].mask.count_ones();
            if list.len() < 8 {
                list.push(index);
            } else if let Some(weakest) = (0..list.len()).min_by_key(|&k| size(list[k])) {
                if size(list[weakest]) < new.mask.count_ones() {
                    list[weakest] = index;
                }
            }
        };
        let start = u16::try_from(unit.cell).ok()?;
        let start_spot = self.spot_at[unit.cell];
        let start_mask = match start_spot {
            NONE => 0,
            spot => match spot_bit[spot as usize] {
                NONE => 0,
                bit => 1u32 << bit,
            },
        };
        push(
            &mut labels,
            &mut buckets,
            Label {
                cell: start,
                mask: start_mask,
                tick: 0,
                fuel: u16::try_from(unit.fuel.min(unit.cap)).ok()?,
                parent: NONE,
                act: WAIT,
                touched: false,
                dead: false,
            },
        );
        for tick in 0..=steps {
            let mut k = 0;
            while k < buckets[usize::from(tick)].len() {
                let index = buckets[usize::from(tick)][k];
                k += 1;
                let label = labels[index as usize];
                if label.dead {
                    continue;
                }
                if labels.len() > cap_labels
                    || (labels.len() % 8192 == 0 && deadline.is_some_and(|d| Instant::now() >= d))
                {
                    return None;
                }
                let cell = usize::from(label.cell);
                if label.fuel != ESCORTED && !label.touched {
                    // Wait here for the supply (no reflection at tick 0).
                    if let Some(at) = supply_at.map(|at| at[cell]).filter(|&at| at != NONE) {
                        let meet = at.max(1).max(u32::from(tick));
                        if meet <= self.steps && (meet > u32::from(tick) || tick == 0) {
                            push(
                                &mut labels,
                                &mut buckets,
                                Label {
                                    tick: u16::try_from(meet).ok()?,
                                    fuel: if touch { full } else { ESCORTED },
                                    parent: index,
                                    act: WAIT,
                                    touched: touch,
                                    ..label
                                },
                            );
                        }
                    }
                }
                let (dt, df) = (self.time[cell], self.fuel[cell]);
                if dt == NONE || u32::from(tick) + dt > self.steps {
                    continue;
                }
                if label.fuel != ESCORTED && u32::from(label.fuel) < df {
                    continue;
                }
                let land = tick + u16::try_from(dt).ok()?;
                for (dir, &to) in self.nbr[cell].iter().enumerate() {
                    if to == NONE {
                        continue;
                    }
                    let to = to as usize;
                    let met = !label.touched && label.fuel != ESCORTED && joins(to, land);
                    let spent = if label.fuel == ESCORTED {
                        ESCORTED
                    } else {
                        label.fuel - u16::try_from(df).ok()?
                    };
                    let spot = self.spot_at[to];
                    let mask = match spot {
                        NONE => label.mask,
                        spot => match spot_bit[spot as usize] {
                            NONE => label.mask,
                            bit => label.mask | (1u32 << bit),
                        },
                    };
                    let arrived = Label {
                        cell: u16::try_from(to).ok()?,
                        mask,
                        tick: land,
                        fuel: if met && !touch { ESCORTED } else { spent },
                        parent: index,
                        act: u8::try_from(dir).expect("dir"),
                        touched: label.touched,
                        dead: false,
                    };
                    push(&mut labels, &mut buckets, arrived);
                    if met && touch {
                        // Topped up on arrival; passing by untouched (above)
                        // stays open, since a later touch releases the supply
                        // later.
                        push(
                            &mut labels,
                            &mut buckets,
                            Label { fuel: full, touched: true, ..arrived },
                        );
                    }
                }
            }
        }
        // Per `(touched, mask)` only the end state each end-state variant
        // would pick can ever be chosen by the team step.
        let mut best: KeyMap<[u32; VARIANTS.len()]> = KeyMap::default();
        let worth = |l: &Label, variant: (i64, i64)| {
            let opt = Opt { mask: l.mask, cell: l.cell, fuel: l.fuel, label: 0 };
            (self.end_value(opt, unit.cap, last_day, variant), l.fuel, std::cmp::Reverse(l.tick))
        };
        for (i, l) in labels.iter().enumerate() {
            if l.dead {
                continue;
            }
            let slots = best
                .entry((u64::from(l.touched) << 48) | u64::from(l.mask))
                .or_insert([NONE; VARIANTS.len()]);
            for (slot, &variant) in slots.iter_mut().zip(&VARIANTS) {
                if *slot == NONE || worth(l, variant) > worth(&labels[*slot as usize], variant) {
                    *slot = u32::try_from(i).expect("label index");
                }
            }
        }
        let mut picked: Vec<u32> = best.into_values().flatten().collect();
        picked.sort_unstable();
        picked.dedup();
        let options: Vec<Opt> = picked
            .into_iter()
            .map(|i| {
                let l = &labels[i as usize];
                Opt { mask: l.mask, cell: l.cell, fuel: l.fuel, label: i }
            })
            .collect();
        Some(PatrolDay { labels, options })
    }

    /// `(new series, series, balls)` of one mask per patrol. Clustered
    /// masks hold patrol-local bits, so the visit test goes through the
    /// patrol's spot→bit table.
    fn tally(&self, masks: &[u32]) -> (i64, i64, i64) {
        let visits = |p: usize, m: u32, spot: usize| match &self.cluster {
            None => m >> spot & 1 == 1,
            Some(cl) => {
                let bit = cl.bit[p][spot];
                bit != NONE && m >> bit & 1 == 1
            }
        };
        let mut seen = vec![false; self.series_count];
        let (mut fresh, mut series, mut balls) = (0, 0, 0);
        for (spot, &stock) in self.stock.iter().enumerate() {
            let visitors = masks.iter().enumerate().filter(|&(p, &m)| visits(p, m, spot)).count();
            let got = i64::from(stock).min(i64::try_from(visitors).unwrap_or(i64::MAX));
            if got == 0 {
                continue;
            }
            balls += got;
            if !seen[self.series[spot]] {
                seen[self.series[spot]] = true;
                series += 1;
                fresh += i64::from(self.new_series[self.series[spot]]);
            }
        }
        (fresh, series, balls)
    }

    fn end_value(&self, opt: Opt, cap: u32, last_day: bool, variant: (i64, i64)) -> i64 {
        if last_day {
            return 0;
        }
        let cap = i64::from(cap.max(1));
        let fuel = if opt.fuel == ESCORTED { cap } else { i64::from(opt.fuel).min(cap) };
        let on_spot = i64::from(self.spot_at[usize::from(opt.cell)] != NONE);
        variant.0 * fuel / cap + variant.1 * on_spot
    }

    /// Best response over the patrols' options until nothing improves.
    fn team(
        &self,
        days: &[&PatrolDay],
        last_day: bool,
        variant: (i64, i64),
        order: &[usize],
        fixed: Option<(usize, Opt)>,
    ) -> Vec<Opt> {
        let stay = |day: &PatrolDay| {
            *day.options
                .iter()
                .find(|o| day.labels[o.label as usize].parent == NONE)
                .unwrap_or(&day.options[0])
        };
        let mut chosen: Vec<Opt> = days.iter().map(|d| stay(d)).collect();
        if let Some((p, opt)) = fixed {
            chosen[p] = opt;
        }
        let mut masks: Vec<u32> = chosen.iter().map(|o| o.mask).collect();
        let value = |masks: &[u32], p: usize, opt: Opt| {
            let (fresh, series, balls) = self.tally(masks);
            fresh * W_NEW
                + series * W_SERIES
                + balls * W_BALL
                + self.end_value(opt, self.patrols[p].cap, last_day, variant)
        };
        for _ in 0..8 {
            let mut changed = false;
            for &p in order {
                if fixed.is_some_and(|(q, _)| q == p) {
                    continue;
                }
                let mut best = (value(&masks, p, chosen[p]), chosen[p]);
                for &opt in &days[p].options {
                    masks[p] = opt.mask;
                    let v = value(&masks, p, opt);
                    if v > best.0 {
                        best = (v, opt);
                    }
                }
                changed |= best.1.label != chosen[p].label;
                chosen[p] = best.1;
                masks[p] = best.1.mask;
            }
            if !changed {
                break;
            }
        }
        chosen
    }
}

/// Where a patrol first meets its supply: `(cell, tick, actions before it)`.
type Meet = (usize, u32, usize);
/// A cell's `(previous cell, direction)` on a shortest-path tree.
type Tree = Vec<(u32, u8)>;

/// A patrol's day as a sequential action list: `Some(dir)` or a 1-tick wait.
fn unroll(day: &PatrolDay, opt: Opt) -> (Vec<Option<u8>>, Option<Meet>) {
    let mut chain = Vec::new();
    let mut at = opt.label;
    while at != NONE {
        chain.push(at);
        at = day.labels[at as usize].parent;
    }
    chain.reverse();
    let mut actions = Vec::new();
    // First escorted label: `(cell, tick, actions before it)`.
    let mut meet = None;
    for pair in chain.windows(2) {
        let (from, to) = (&day.labels[pair[0] as usize], &day.labels[pair[1] as usize]);
        if to.act == WAIT {
            actions.extend((from.tick..to.tick).map(|_| None));
        } else {
            actions.push(Some(to.act));
        }
        if meet.is_none() && (to.fuel == ESCORTED || to.touched) {
            meet = Some((usize::from(to.cell), u32::from(to.tick), actions.len()));
        }
    }
    (actions, meet)
}

fn action(agent: usize, dir: Option<u8>) -> Action {
    let id = u8::try_from(agent).expect("agent id");
    dir.map_or_else(|| Action::wait(id), |d| Action::step(id, Direction::ALL[usize::from(d)]))
}

/// One day replayed through the official simulator.
struct Played {
    plan: AgentActions,
    state: MatchState,
    variety: usize,
    series_today: usize,
    balls_today: u32,
    fuel: u32,
}

impl Played {
    fn today(&self) -> (usize, usize, u32, u32) {
        (self.variety, self.series_today, self.balls_today, self.fuel)
    }
}

fn roads(state: &MatchState) -> Vec<TrafficState> {
    state
        .setting
        .map
        .cells()
        .iter()
        .map(|c| match c.terrain {
            Terrain::Road(s) => s,
            _ => TrafficState::Smooth,
        })
        .collect()
}

/// Play `plan` for the day `state` is at; `None` unless it is clean.
fn play(state: &MatchState, plan: AgentActions) -> Option<Played> {
    let ctx = SolverContext::from_state(state);
    let plan = crate::adapters::walk::BudgetMode::Official.apply(&ctx, plan);
    let today = roads(state);
    let mut next = state.clone();
    let before = (next.illegal_action_count, next.rejected_response_count);
    let held: u32 = next.agents.iter().map(engine::Agent::total_inventory).sum();
    let sim = MatchSim::default();
    for block in plan.actions.chunks(ctx.agent_ids.len().max(1)) {
        sim.step(&mut next, &AgentActions::new(block.to_vec()));
    }
    if (next.illegal_action_count, next.rejected_response_count) != before {
        return None;
    }
    // Tomorrow's roads are not known; assume today's stay.
    for (cell, state) in next.setting.map.cells_mut().iter_mut().zip(today) {
        if let Terrain::Road(current) = &mut cell.terrain {
            *current = state;
        }
    }
    let total: u32 = next.agents.iter().map(engine::Agent::total_inventory).sum();
    Some(Played {
        variety: next.series_taken.len(),
        series_today: next
            .series_acquired_by_day
            .get(&state.day)
            .map_or(0, std::collections::BTreeSet::len),
        balls_today: total.saturating_sub(held),
        fuel: next.agents.iter().filter(|a| a.kind == AgentKind::Patrol).map(|a| a.fuel).sum(),
        plan,
        state: next,
    })
}

fn total(state: &MatchState) -> (usize, u32, u32) {
    (
        state.series_taken.len(),
        state.daily_varieties().iter().sum(),
        state.agents.iter().map(engine::Agent::total_inventory).sum(),
    )
}

/// The sequential action list that walks `supply` to `cell` on its tree.
fn walk(supply: usize, from: usize, cell: usize, prev: &[(u32, u8)]) -> Vec<Action> {
    let mut path = Vec::new();
    let mut at = cell;
    while at != from && prev[at].0 != NONE {
        path.push(prev[at].1);
        at = prev[at].0 as usize;
    }
    path.reverse();
    path.into_iter().map(|d| action(supply, Some(d))).collect()
}

/// Ticks a sequential action list takes when it starts on `cell`.
fn elapsed(board: &Board, mut cell: usize, line: &[Action]) -> u32 {
    let mut tick = 0;
    for a in line {
        match a.kind {
            engine::model::ActionKind::Wait => tick += 1,
            engine::model::ActionKind::Move(dir) => {
                let d = Direction::ALL.iter().position(|x| *x == dir).unwrap_or(0);
                tick += board.time[cell];
                cell = board.nbr[cell][d] as usize;
            }
        }
    }
    tick
}

impl ConvoyCandidate {
    /// Clean day plans for `state`: one per touch / escort choice and
    /// end-state variant (`variants` of them), plus `fleet`'s plan. Empty
    /// when nothing replays clean.
    #[allow(clippy::too_many_lines, reason = "one pass: searches, pairings, emission, replay")]
    fn day_plans(&self, state: &MatchState, root: bool, deadline: Option<Instant>) -> Vec<Played> {
        let variants = if root { VARIANTS.len() } else { 1 };
        let mut out: Vec<Played> = Vec::new();
        let keep = |out: &mut Vec<Played>, plan: AgentActions| {
            if let Some(played) = play(state, plan) {
                if !out.iter().any(|o| o.plan == played.plan) {
                    out.push(played);
                }
            }
        };
        let fleet = if root { &self.fleet } else { &self.ahead };
        let mut ctx = SolverContext::from_state(state);
        if let Some(d) = deadline {
            let left = d.saturating_duration_since(Instant::now()).as_millis();
            ctx.remaining_ms = u32::try_from(left).unwrap_or(u32::MAX);
        }
        keep(&mut out, fleet.decide(&ctx));
        let limits = (self.labels, deadline);
        let steps = state.day_steps();
        let k = self.cluster.then_some(self.clusters);
        let Some(board) = Board::new(state, steps, k) else { return out };
        let np = board.patrols.len();
        if np == 0 || np > 8 || state.agents.iter().enumerate().any(|(i, a)| usize::from(a.id) != i)
        {
            return out;
        }
        let late = || deadline.is_some_and(|d| Instant::now() >= d);
        let last_day = state.day + 1 >= state.setting.num_days;
        let n_agents = state.agents.len();
        // Patrol `p`'s spot→bit table: its cluster's when clustered, else
        // the identity (every spot keeps its global bit).
        let identity: Vec<u32> =
            (0..u32::try_from(board.spot_at.len()).unwrap_or(u32::MAX)).collect();
        let spot_bits =
            |p: usize| -> &[u32] { board.cluster.as_ref().map_or(&identity, |cl| &cl.bit[p]) };
        let mut tank = Vec::with_capacity(np);
        for (p, &unit) in board.patrols.iter().enumerate() {
            if late() {
                return out;
            }
            let Some(day) = board.patrol_day(unit, Supplied::Tank, last_day, limits, spot_bits(p))
            else {
                return out;
            };
            tank.push(day);
        }
        let order: Vec<usize> = (0..np).collect();
        let reversed: Vec<usize> = (0..np).rev().collect();
        let emit = |lists: Vec<Vec<Action>>| {
            let mut actions = Vec::with_capacity(n_agents * steps as usize);
            for step in 0..steps as usize {
                for (i, list) in lists.iter().enumerate() {
                    actions.push(list.get(step).copied().unwrap_or_else(|| action(i, None)));
                }
            }
            AgentActions::new(actions)
        };
        let patrol_lines = |days: &[&PatrolDay], chosen: &[Opt]| {
            let mut lists: Vec<Vec<Action>> = vec![Vec::new(); n_agents];
            for (p, &opt) in chosen.iter().enumerate() {
                let agent = board.patrols[p].agent;
                lists[agent] = unroll(days[p], opt).0.iter().map(|&a| action(agent, a)).collect();
            }
            lists
        };
        if board.supplies.is_empty() {
            let days: Vec<&PatrolDay> = tank.iter().collect();
            for &variant in VARIANTS.iter().take(variants.max(1)) {
                let chosen = board.team(&days, last_day, variant, &order, None);
                keep(&mut out, emit(patrol_lines(&days, &chosen)));
            }
            return out;
        }
        // The first supply touches at most one patrol, then escorts another;
        // any further supply escorts one patrol all day.
        let lead = board.supplies[0];
        let (lead_at, lead_prev) = board.reach(lead.cell);
        let mut escort: Vec<Option<PatrolDay>> = Vec::with_capacity(np);
        let mut touch: Vec<Option<PatrolDay>> = Vec::with_capacity(np);
        for (p, &unit) in board.patrols.iter().enumerate() {
            if late() {
                return out;
            }
            escort.push(board.patrol_day(
                unit,
                Supplied::Escort(&lead_at),
                last_day,
                limits,
                spot_bits(p),
            ));
            touch.push(board.patrol_day(
                unit,
                Supplied::Touch(&lead_at),
                last_day,
                limits,
                spot_bits(p),
            ));
        }
        let extra: Vec<(Unit, Vec<u32>, Tree)> = board.supplies[1..]
            .iter()
            .map(|&s| {
                let (at, prev) = board.reach(s.cell);
                (s, at, prev)
            })
            .collect();
        // Escort searches that start after a touch, by `(patrol, cell, tick)`.
        let mut after_touch: HashMap<(usize, usize, u32), Option<std::rc::Rc<PatrolDay>>> =
            HashMap::new();
        for b in 0..np {
            let Some(escort_b) = escort[b].as_ref() else { continue };
            // Further supplies ride with the next patrols after `b`.
            let riders: Vec<(usize, PatrolDay)> = extra
                .iter()
                .enumerate()
                .filter_map(|(k, (_, at, _))| {
                    let p = (b + 1 + k) % np;
                    (p != b && k + 1 < np)
                        .then(|| {
                            board.patrol_day(
                                board.patrols[p],
                                Supplied::Escort(at),
                                last_day,
                                limits,
                                spot_bits(p),
                            )
                        })
                        .flatten()
                        .map(|day| (p, day))
                })
                .collect();
            // `a == b` stands for "touch nobody".
            for a in 0..np {
                if late() {
                    return out;
                }
                if a != b && (touch[a].is_none() || riders.iter().any(|(p, _)| *p == a)) {
                    continue;
                }
                for &variant in VARIANTS.iter().take(variants.max(1)) {
                    for order in [&order, &reversed] {
                        let mut days: Vec<&PatrolDay> = tank.iter().collect();
                        days[b] = escort_b;
                        for (p, day) in &riders {
                            days[*p] = day;
                        }
                        if a != b {
                            days[a] = touch[a].as_ref().expect("checked above");
                        }
                        let mut chosen = board.team(&days, last_day, variant, order, None);
                        // Where the lead supply is released, and from when.
                        let touched = (a != b).then(|| unroll(days[a], chosen[a]).1).flatten();
                        let late_escort;
                        let mut from = (lead.cell, 0u32, lead_prev.clone());
                        if a != b {
                            let Some((cell, tick, _)) = touched else { continue };
                            let (dist, prev) = board.reach(cell);
                            let at: Vec<u32> = dist
                                .iter()
                                .map(|&d| if d == NONE { NONE } else { d + tick })
                                .collect();
                            let day = after_touch
                                .entry((b, cell, tick))
                                .or_insert_with(|| {
                                    board
                                        .patrol_day(
                                            board.patrols[b],
                                            Supplied::Escort(&at),
                                            last_day,
                                            limits,
                                            spot_bits(b),
                                        )
                                        .map(std::rc::Rc::new)
                                })
                                .clone();
                            let Some(day) = day else { continue };
                            late_escort = day;
                            days[b] = &*late_escort;
                            chosen =
                                board.team(&days, last_day, variant, order, Some((a, chosen[a])));
                            from = (cell, tick, prev);
                        }
                        let mut lists = patrol_lines(&days, &chosen);
                        // Lead supply: to the touch, then to the escort.
                        let mut line = Vec::new();
                        if a != b {
                            line = walk(lead.agent, lead.cell, from.0, &lead_prev);
                            let used = elapsed(&board, lead.cell, &line);
                            line.extend((used..from.1).map(|_| action(lead.agent, None)));
                        }
                        let (actions_b, meet_b) = unroll(days[b], chosen[b]);
                        let (cell, tick, after) =
                            meet_b.unwrap_or((usize::from(chosen[b].cell), NONE, actions_b.len()));
                        let leg = walk(lead.agent, from.0, cell, &from.2);
                        let arrive = from.1 + elapsed(&board, from.0, &leg);
                        line.extend(leg);
                        if tick != NONE {
                            line.extend((arrive..tick).map(|_| action(lead.agent, None)));
                            line.extend(actions_b[after..].iter().map(|&x| action(lead.agent, x)));
                        }
                        lists[lead.agent] = line;
                        for ((p, day), (unit, _, prev)) in riders.iter().zip(&extra) {
                            let (actions, meet) = unroll(day, chosen[*p]);
                            let (cell, tick, after) =
                                meet.unwrap_or((usize::from(chosen[*p].cell), NONE, actions.len()));
                            let mut line = walk(unit.agent, unit.cell, cell, prev);
                            if tick != NONE {
                                let used = elapsed(&board, unit.cell, &line);
                                line.extend((used..tick).map(|_| action(unit.agent, None)));
                                line.extend(
                                    actions[after..].iter().map(|&x| action(unit.agent, x)),
                                );
                            }
                            lists[unit.agent] = line;
                        }
                        keep(&mut out, emit(lists));
                    }
                }
            }
        }
        out
    }
}

/// A day-end state reached from the root plan `root`.
struct Node {
    root: usize,
    state: MatchState,
}

fn rank(node: &Node) -> ((usize, u32, u32), u32) {
    let fuel = node
        .state
        .agents
        .iter()
        .filter(|a| a.kind == AgentKind::Patrol)
        .map(|a| a.fuel.min(a.fuel_cap))
        .sum();
    (total(&node.state), fuel)
}

/// The pessimistic road forecast: every smooth road busy.
fn congest(state: &mut MatchState) {
    for cell in state.setting.map.cells_mut() {
        if cell.terrain == Terrain::Road(TrafficState::Smooth) {
            cell.terrain = Terrain::Road(TrafficState::Busy);
        }
    }
}

type Total = (usize, u32, u32);

impl ConvoyCandidate {
    /// Beam over day-end states from `nodes`, `depth` days ahead: the best
    /// whole-match score reached from each root (`None`: fell out of the
    /// beam), and how many days were completed before the deadline.
    fn ahead(
        &self,
        mut nodes: Vec<Node>,
        roots: usize,
        depth: u32,
        until: Option<Instant>,
    ) -> (Vec<Option<Total>>, u32) {
        let threads = std::thread::available_parallelism().map_or(1, usize::from).clamp(1, 8);
        let mut reached = 0;
        for _ in 0..depth {
            nodes.sort_by_key(|n| std::cmp::Reverse((rank(n), std::cmp::Reverse(n.root))));
            let mut seen = std::collections::BTreeSet::new();
            nodes.retain(|n| {
                seen.insert((
                    n.state
                        .agents
                        .iter()
                        .map(|a| (a.pos.col, a.pos.row, a.fuel))
                        .collect::<Vec<_>>(),
                    total(&n.state),
                ))
            });
            // The best path from `fleet`'s plan always stays in the beam, so
            // the final comparison against it is like for like.
            if let Some(k) = nodes.iter().position(|n| n.root == 0).filter(|&k| k >= self.width) {
                nodes.swap(self.width - 1, k);
            }
            nodes.truncate(self.width);
            let next = std::sync::atomic::AtomicUsize::new(0);
            let nodes_ref = &nodes;
            let mut children: Vec<(usize, Vec<Node>)> = std::thread::scope(|scope| {
                let handles: Vec<_> = (0..threads)
                    .map(|_| {
                        scope.spawn(|| {
                            let mut mine = Vec::new();
                            loop {
                                let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                if k >= nodes_ref.len() {
                                    return mine;
                                }
                                let node = &nodes_ref[k];
                                let plans = self.day_plans(&node.state, false, until);
                                mine.push((
                                    k,
                                    plans
                                        .into_iter()
                                        .map(|p| Node { root: node.root, state: p.state })
                                        .collect(),
                                ));
                            }
                        })
                    })
                    .collect();
                handles.into_iter().filter_map(|h| h.join().ok()).flatten().collect()
            });
            // Out of time: the children cover the nodes unevenly, so the
            // last complete day decides.
            if until.is_some_and(|d| Instant::now() >= d) {
                break;
            }
            children.sort_unstable_by_key(|(k, _)| *k);
            let grown: Vec<Node> = children.into_iter().flat_map(|(_, c)| c).collect();
            if grown.is_empty() {
                break;
            }
            nodes = grown;
            reached += 1;
        }
        let mut best = vec![None; roots];
        for node in &nodes {
            let score = total(&node.state);
            if best[node.root].is_none_or(|b| score > b) {
                best[node.root] = Some(score);
            }
        }
        (best, reached)
    }
}

impl Solver for ConvoyCandidate {
    fn name(&self) -> &'static str {
        "convoy"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let started = Instant::now();
        let window = (self.budget_ms > 0).then(|| {
            let cap = Duration::from_millis(u64::from(self.budget_ms));
            if ctx.remaining_ms == u32::MAX {
                cap
            } else {
                cap.min(Duration::from_millis(u64::from(ctx.remaining_ms.saturating_sub(300))))
            }
        });
        // `fleet`'s plan comes first in the pool: it is the plan to beat.
        let mut pool = self.day_plans(ctx.state, true, window.map(|w| started + w / 3));
        if pool.is_empty() {
            return self.fleet.decide(ctx);
        }
        if self.bail && pool.len() == 1 {
            tracing::info!(day = ctx.day, pool = 1, "convoy bail: fleet plan only");
            return pool.swap_remove(0).plan;
        }
        let remaining = ctx.state.setting.num_days.saturating_sub(ctx.day + 1);
        let depth = self.depth.min(remaining);
        // Each road forecast gets an equal share of what is left.
        let mut sums: Vec<Option<Total>> = vec![Some((0, 0, 0)); pool.len()];
        let mut complete = true;
        for (k, &busy) in self.scenarios.iter().enumerate() {
            let until = window.map(|w| {
                let now = Instant::now();
                let left = (started + w).saturating_duration_since(now);
                now + left / u32::try_from(self.scenarios.len() - k).unwrap_or(1)
            });
            let nodes: Vec<Node> = pool
                .iter()
                .enumerate()
                .map(|(root, p)| {
                    let mut state = p.state.clone();
                    if busy {
                        congest(&mut state);
                    }
                    Node { root, state }
                })
                .collect();
            let (best, reached) = self.ahead(nodes, pool.len(), depth, until);
            complete &= reached == depth;
            for (sum, got) in sums.iter_mut().zip(best) {
                *sum = sum.zip(got).map(|(a, b)| (a.0 + b.0, a.1 + b.1, a.2 + b.2));
            }
        }
        // Leave `fleet`'s plan only for a strictly better whole-match score,
        // and only when every forecast was played to the same depth.
        let best = (1..pool.len())
            .filter(|&k| complete && sums[k].is_some() && sums[k] > sums[0])
            .max_by_key(|&k| (sums[k], pool[k].today(), std::cmp::Reverse(k)))
            .unwrap_or(0);
        tracing::info!(
            day = ctx.day,
            pool = pool.len(),
            depth,
            complete,
            chosen = best,
            elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "convoy day plan"
        );
        pool.swap_remove(best).plan
    }
}
