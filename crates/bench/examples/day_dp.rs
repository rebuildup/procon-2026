//! Exact per-patrol day solver under (time, fuel) — Issue #382 existence proof.
//!
//! Answers, for one day of one `CaseSpec`, "how many distinct series can the
//! patrols collect at most, and what routes do it", with an exact
//! label-setting search over `(cell, series mask)` carrying the full
//! (time, fuel) Pareto set. This is the formulation PR #386's per-leg weight
//! switching (one metric per patrol, applied to every leg) cannot express:
//! here every leg may mix fast and fuel-lean paths, which is exactly what
//! the Issue's ref-16 proof requires.
//!
//! # Two run modes
//!
//! - `mode=legs` (default): the Issue #382 enumerator, reproduced. Per-leg
//!   Pareto (time, fuel) frontiers between spot cells, a DFS over spot
//!   sequences per patrol with a `(last cell, series mask)` dominance table,
//!   then an exact assignment step maximizing the series union (patrols are
//!   independent, so per-patrol feasible masks compose by union). Fuel is
//!   charged per *departure* cell (Q11 / Issue #342 P0-C) and there is no
//!   mid-day refuel in this mode.
//! - `mode=cell`: cell-level label DP, one hex step per transition, with an
//!   optional mid-day refuel: a patrol arriving at cell `q` at tick `t` may
//!   refill to its cap when the supply vehicle's earliest possible arrival
//!   at `q` is `<= t` (`refuel=on`). This models #405's 区間途中合流 as an
//!   optimistic upper bound — the supply vehicle is unconstrained in fuel
//!   and the DP assumes it can always be where a patrol needs it, which
//!   ignores contention between patrols sharing one supply vehicle.
//!
//! # Exactness statement
//!
//! - `legs` is exact over *spot-sequence plans*: every leg uses the full
//!   (time, fuel) Pareto frontier, the dominance table keeps the
//!   non-dominated (time, fuel) set per `(last cell, series mask)`, and the
//!   assignment step maximizes the union of series masks exactly. What it
//!   abstracts away: the tick a pickup happens inside a leg (charged at the
//!   leg boundary), mid-leg waiting, and refuel. Standing on a spot cell is
//!   assumed to always convert into a pick (stock >= 1; the daily
//!   per-agent-per-spot cap is 1 — 募集要項 / Q15).
//! - `cell` without `refuel=on` is exact over one patrol's *movement plans*
//!   for the day (waits and mixed per-step weights included). With
//!   `refuel=on` it is exact for the patrol given the refuel model, and the
//!   refuel model itself is an upper bound on what a coordinated 1-supply
//!   team achieves.
//! - Neither mode schedules the supply vehicle's route; `cell refuel=on`
//!   replaces it with the earliest-arrival bound. Stock contention between
//!   patrols (two patrols picking the same spot) is checked and reported on
//!   the winning assignment.
//!
//! # Output
//!
//! Per patrol: the chosen spot sequence with (arrival tick, fuel at
//! arrival) and the series mask. Joint: the maximum distinct-series count.
//! `verify=on` replays the winning joint plan through `MatchSim` and reports
//! the day's series / D / B plus illegal / fuel-short counts, so the DP's
//! claim is checked against the official rules engine rather than trusted.
//!
//! Usage:
//! `day_dp CASE_JSON key=value ...`
//!
//! - `day=N` day index (default 0)
//! - `kinds=S,P,P,P` agent kinds (default `S` then `P` for the rest)
//! - `mode=legs|cell` (default `legs`)
//! - `refuel=on|off` (default `off`, `cell` mode only)
//! - `verify=on|off` replay the winning joint plan through `MatchSim`
//!   (default `on`)
//! - `traffic=smooth|observed` road state for the replay (default `smooth`;
//!   `observed` needs `events_jsonl=...` from a recorded run)
//! - `witness=on|off` print the winning per-patrol routes (default `on`)
//!
//! Example (the Issue's proof):
//! `day_dp /tmp/dp382/ref16.case.json day=0 kinds=S,P,P,P mode=legs`
use bench::{CaseSpec, kinds};
use engine::model::{AgentKind, Direction, HexCoord, MapGrid, MatchState, Terrain, TrafficState};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// (time, fuel) label; both minimized.
type Cost = (u32, u32);

/// Pareto frontier per cell of one single-source field.
type Frontier = Vec<Vec<Cost>>;

/// Predecessor link per Pareto label: `(cell, label)` it was relaxed from.
type PredField = Vec<Vec<Option<(usize, Cost)>>>;

const INF: u32 = u32::MAX / 4;

fn step_cost_of(terrain: Terrain) -> u32 {
    engine::wire_plan::terrain_step_cost(terrain).unwrap_or(INF)
}

fn index_of(map: &MapGrid, p: HexCoord) -> usize {
    usize::try_from(p.row).expect("row fits usize") * usize::from(map.size.width)
        + usize::try_from(p.col).expect("col fits usize")
}

fn coord_of(map: &MapGrid, index: usize) -> HexCoord {
    let width = usize::from(map.size.width);
    HexCoord::new(
        i32::try_from(index % width).expect("col fits i32"),
        i32::try_from(index / width).expect("row fits i32"),
    )
}

/// Terrain-cost snapshot of one board: departure-billed step/fuel per cell,
/// neighbour lists, and the spot table.
struct Board {
    step: Vec<u32>,
    fuel: Vec<u32>,
    neighbors: Vec<Vec<usize>>,
    /// Spot cell index -> 0-based series index.
    spots: BTreeMap<usize, usize>,
    /// Spot cell index -> max stock.
    stock: BTreeMap<usize, u32>,
    series_count: usize,
}

impl Board {
    fn from_state(state: &MatchState) -> Self {
        let map = &state.setting.map;
        let mut spots = BTreeMap::new();
        let mut stock = BTreeMap::new();
        let mut series = BTreeSet::new();
        for cell in map.cells() {
            if let Some(spot) = &cell.spot {
                let idx = index_of(map, cell.coord);
                spots.insert(idx, (spot.series - 1) as usize);
                stock.insert(idx, spot.max_stock);
                series.insert(spot.series - 1);
            }
        }
        let coords: Vec<HexCoord> = (0..map.cells().len()).map(|i| coord_of(map, i)).collect();
        Board {
            step: map.cells().iter().map(|c| step_cost_of(c.terrain)).collect(),
            fuel: map.cells().iter().map(|c| engine::rules::fuel_cost(c.terrain)).collect(),
            neighbors: coords
                .iter()
                .map(|&p| {
                    Direction::ALL
                        .iter()
                        .filter_map(|dir| map.get(dir.step(p)))
                        .filter(|cell| cell.terrain.traversable())
                        .map(|cell| index_of(map, cell.coord))
                        .collect()
                })
                .collect(),
            spots,
            stock,
            series_count: series.len(),
        }
    }
}

/// Insert into a Pareto list; returns `true` when the label survived.
fn pareto_insert(labels: &mut Vec<Cost>, label: Cost) -> bool {
    if labels.iter().any(|&(t, f)| t <= label.0 && f <= label.1) {
        return false;
    }
    labels.retain(|&(t, f)| !(label.0 <= t && label.1 <= f));
    labels.push(label);
    true
}

/// The (time, fuel) Pareto frontier from `src` to every reachable cell,
/// within `(max_time, max_fuel)`. The returned predecessor field
/// (`preds[cell][i]`) names the `(cell, label)` that label `i` of `cell`
/// was relaxed from, so any label can be expanded into a full path.
fn pareto_field(board: &Board, src: usize, max_time: u32, max_fuel: u32) -> (Frontier, PredField) {
    let n = board.step.len();
    let mut labels: Vec<Vec<Cost>> = vec![Vec::new(); n];
    let mut preds: PredField = vec![Vec::new(); n];
    labels[src].push((0, 0));
    preds[src].push(None);
    let mut queue = std::collections::BinaryHeap::from([std::cmp::Reverse((0u32, 0u32, src))]);
    while let Some(std::cmp::Reverse((time, fuel, cell))) = queue.pop() {
        if !labels[cell].contains(&(time, fuel)) {
            continue;
        }
        let (dt, df) = (board.step[cell], board.fuel[cell]);
        for &next in &board.neighbors[cell] {
            let (nt, nf) = (time + dt, fuel + df);
            if nt > max_time || nf > max_fuel {
                continue;
            }
            if pareto_insert(&mut labels[next], (nt, nf)) {
                preds[next].push(Some((cell, (time, fuel))));
                queue.push(std::cmp::Reverse((nt, nf, next)));
            }
        }
    }
    (labels, preds)
}

/// Expand the label `(time, fuel)` at `dst` of a Pareto field into one full
/// cell path from the field's source: walk backwards from `dst`, at each
/// step finding a predecessor cell whose frontier contains a label that
/// produces the remaining leg cost exactly.
fn pareto_path(labels: &Frontier, board: &Board, dst: usize, label: Cost) -> Vec<usize> {
    let mut path = vec![dst];
    let mut cur = (dst, label);
    while cur.1 != (0, 0) {
        let Some((prev_cell, prev_label)) = find_pred(labels, board, cur.0, cur.1) else {
            break;
        };
        path.push(prev_cell);
        cur = (prev_cell, prev_label);
        if path.len() > 512 {
            break; // defensive: bound the walk
        }
    }
    path.reverse();
    path
}

/// One step of the backward walk: the `(cell, label)` whose departure costs
/// produce exactly `label` at `cell`.
fn find_pred(labels: &Frontier, board: &Board, cell: usize, label: Cost) -> Option<(usize, Cost)> {
    let (t, f) = label;
    board.neighbors[cell].iter().copied().find_map(|prev| {
        labels[prev]
            .iter()
            .find(|&&(lt, lf)| board.step[prev] + lt == t && board.fuel[prev] + lf == f)
            .map(|&prev_label| (prev, prev_label))
    })
}

/// Earliest tick an unconstrained agent (the supply vehicle) reaches each cell.
fn earliest_arrival(board: &Board, src: usize) -> Vec<u32> {
    let n = board.step.len();
    let mut best = vec![INF; n];
    best[src] = 0;
    let mut queue = std::collections::BinaryHeap::from([std::cmp::Reverse((0u32, src))]);
    while let Some(std::cmp::Reverse((time, cell))) = queue.pop() {
        if best[cell] < time {
            continue;
        }
        for &next in &board.neighbors[cell] {
            let nt = time + board.step[cell];
            if nt < best[next] {
                best[next] = nt;
                queue.push(std::cmp::Reverse((nt, next)));
            }
        }
    }
    best
}

/// One witness route: cells in visiting order, starting at the patrol start.
type Route = Vec<usize>;

/// `mode=legs` state table: `(last cell, series mask)` -> non-dominated
/// `((time, fuel), route)` pairs.
type LegStates = BTreeMap<(usize, u64), Vec<(Cost, Route)>>;

/// Enumerate one patrol's feasible (series mask, time, fuel) states over
/// distinct-series spot sequences.
fn patrol_leg_states(
    board: &Board,
    start: usize,
    max_time: u32,
    max_fuel: u32,
    fields: &BTreeMap<usize, Vec<Vec<Cost>>>,
) -> LegStates {
    let mut states: LegStates = BTreeMap::new();
    states.insert((start, 0), vec![((0, 0), vec![start])]);
    let mut frontier: BTreeSet<(usize, u64)> = BTreeSet::from([(start, 0)]);
    while !frontier.is_empty() {
        let mut next = BTreeSet::new();
        for &(cell, mask) in &frontier {
            let labels = states[&(cell, mask)].clone();
            for (&spot_cell, &series) in &board.spots {
                if mask & (1 << series) != 0 {
                    continue;
                }
                let Some(leg_frontier) = fields.get(&cell).and_then(|f| f.get(spot_cell)) else {
                    continue;
                };
                if leg_frontier.is_empty() {
                    continue;
                }
                let new_mask = mask | (1 << series);
                for (base, route) in &labels {
                    for &leg in leg_frontier {
                        let (nt, nf) = (base.0 + leg.0, base.1 + leg.1);
                        if nt > max_time || nf > max_fuel {
                            continue;
                        }
                        let entry = states.entry((spot_cell, new_mask)).or_default();
                        if entry.iter().any(|&((t, f), _)| t <= nt && f <= nf) {
                            continue;
                        }
                        entry.retain(|&((t, f), _)| !(nt <= t && nf <= f));
                        let mut new_route = route.clone();
                        new_route.push(spot_cell);
                        entry.push(((nt, nf), new_route));
                        next.insert((spot_cell, new_mask));
                    }
                }
            }
        }
        frontier = next;
    }
    states
}

/// Per patrol: `series mask -> best (cost, route)` (any last cell).
fn flat_masks(states: LegStates) -> BTreeMap<u64, (Cost, Route)> {
    let mut out: BTreeMap<u64, (Cost, Route)> = BTreeMap::new();
    for ((_cell, mask), labels) in states {
        for (cost, route) in labels {
            match out.entry(mask) {
                std::collections::btree_map::Entry::Vacant(e) => {
                    e.insert((cost, route));
                }
                std::collections::btree_map::Entry::Occupied(e) => {
                    if cost < e.get().0 {
                        *e.into_mut() = (cost, route);
                    }
                }
            }
        }
    }
    out
}

/// Walk a route's legs choosing one label per leg so the sum equals `total`.
fn realize_legs(frontiers: &[Vec<Cost>], total: Cost) -> Option<Vec<Cost>> {
    // suffix[i] = (t, f) sums achievable by legs i.. over the remaining budget.
    let mut suffix: Vec<BTreeSet<Cost>> = vec![BTreeSet::new(); frontiers.len() + 1];
    suffix[frontiers.len()].insert((0, 0));
    for i in (0..frontiers.len()).rev() {
        let reach: BTreeSet<Cost> = suffix[i + 1].clone();
        for &(t0, f0) in &reach {
            for &(t, f) in &frontiers[i] {
                if t0 + t <= total.0 && f0 + f <= total.1 {
                    suffix[i].insert((t0 + t, f0 + f));
                }
            }
        }
    }
    if !suffix[0].contains(&total) {
        return None;
    }
    let mut out = Vec::with_capacity(frontiers.len());
    let mut rem = total;
    for (i, leg) in frontiers.iter().enumerate() {
        let reach = suffix[i + 1].clone();
        let found = leg
            .iter()
            .copied()
            .find(|&(t, f)| t <= rem.0 && f <= rem.1 && reach.contains(&(rem.0 - t, rem.1 - f)));
        let (t, f) = found?;
        out.push((t, f));
        rem = (rem.0 - t, rem.1 - f);
    }
    Some(out)
}

/// One patrol's final plan: witness route, arrival list, series mask.
#[derive(Debug, Clone)]
struct PatrolPlan {
    mask: u64,
    total: Cost,
    route: Route,
    /// `(cell, arrival tick, fuel at arrival)` per visited spot.
    arrivals: Vec<(usize, u32, u32)>,
    /// Full event list `(tick, cell, fuel)` for the wire reconstruction.
    events: Vec<(u32, usize, u32)>,
}

/// `mode=cell` per-patrol exact DP over `(cell, mask, tick) -> min fuel`.
fn patrol_cell_states(
    board: &Board,
    start: usize,
    max_time: u32,
    max_fuel: u32,
    refuel_field: Option<&[u32]>,
) -> BTreeMap<u64, PatrolPlan> {
    // State key: (cell, mask, tick) -> min fuel. Predecessor links rebuild
    // the route; `pred_key` also marks which link was a pickup.
    let mut labels: BTreeMap<(usize, u64, u32), u32> = BTreeMap::new();
    let mut pred: BTreeMap<(usize, u64, u32), (usize, u64, u32)> = BTreeMap::new();
    let mut queue = std::collections::BinaryHeap::new();
    labels.insert((start, 0, 0), 0);
    queue.push(std::cmp::Reverse((0u32, 0u32, start, 0u64)));
    let mut best: BTreeMap<u64, PatrolPlan> = BTreeMap::new();
    while let Some(std::cmp::Reverse((tick, fuel, cell, mask))) = queue.pop() {
        if labels.get(&(cell, mask, tick)) != Some(&fuel) {
            continue; // stale entry
        }
        // Pickup at the current cell: mask grows at the same (tick, fuel).
        let mut nm = mask;
        if let Some(&series) = board.spots.get(&cell) {
            if mask & (1 << series) == 0 {
                nm = mask | (1 << series);
                let key = (cell, nm, tick);
                if labels.get(&key).is_none_or(|&f| fuel < f) {
                    labels.insert(key, fuel);
                    pred.insert(key, (cell, mask, tick));
                    queue.push(std::cmp::Reverse((tick, fuel, cell, nm)));
                }
            }
        }
        // Record: is this the earliest witness for `nm`?
        let better = match best.get(&nm) {
            Some(existing) => (tick, fuel) < existing.total,
            None => true,
        };
        if better {
            // Walk predecessors back to the start. Every state key on the
            // chain contributes one event (tick, cell, fuel); pickups are
            // the states whose mask grew.
            let mut route = Vec::new();
            let mut arrivals = Vec::new();
            let mut events = Vec::new();
            let mut key = (cell, nm, tick);
            loop {
                let (kc, km, kt) = key;
                let kf = labels[&key];
                route.push(kc);
                events.push((kt, kc, kf));
                if board.spots.contains_key(&kc) && km != pred_mask(&pred, key, km) {
                    arrivals.push((kc, kt, kf));
                }
                let Some(prev) = pred.get(&key).copied() else {
                    break;
                };
                if prev == key {
                    break;
                }
                key = prev;
            }
            route.reverse();
            events.reverse();
            arrivals.reverse();
            best.insert(nm, PatrolPlan { mask: nm, total: (tick, fuel), route, arrivals, events });
        }
        if tick >= max_time {
            continue;
        }
        // Wait one tick.
        let wait_key = (cell, nm, tick + 1);
        if labels.get(&wait_key).is_none_or(|&f| fuel < f) {
            labels.insert(wait_key, fuel);
            pred.insert(wait_key, (cell, nm, tick));
            queue.push(std::cmp::Reverse((tick + 1, fuel, cell, nm)));
        }
        // One hex step; the departure cell bills step + fuel (Q11).
        for &next in &board.neighbors[cell] {
            let nt = tick + board.step[cell];
            let mut nf = fuel + board.fuel[cell];
            if let Some(supply_field) = refuel_field {
                if supply_field[next] <= nt {
                    nf = 0;
                }
            }
            if nt > max_time || nf > max_fuel {
                continue;
            }
            let key = (next, nm, nt);
            if labels.get(&key).is_none_or(|&f| nf < f) {
                labels.insert(key, nf);
                pred.insert(key, (cell, nm, tick));
                queue.push(std::cmp::Reverse((nt, nf, next, nm)));
            }
        }
    }
    best
}

/// The predecessor's mask at `key`: the mask the patrol carried before the
/// state's own (possible) pickup. Equal to `mask` when the state is not a
/// pickup state.
fn pred_mask(
    pred: &BTreeMap<(usize, u64, u32), (usize, u64, u32)>,
    key: (usize, u64, u32),
    mask: u64,
) -> u64 {
    pred.get(&key).map_or(mask, |&(_, pm, _)| pm)
}

/// BFS shortest (fewest cells) path between two cells over passable cells.
fn shortest_path(board: &Board, src: usize, dst: usize) -> Option<Vec<usize>> {
    if src == dst {
        return Some(vec![src]);
    }
    let mut prev: BTreeMap<usize, usize> = BTreeMap::new();
    let mut seen: BTreeSet<usize> = BTreeSet::from([src]);
    let mut queue = VecDeque::from([src]);
    while let Some(cell) = queue.pop_front() {
        for &next in &board.neighbors[cell] {
            if seen.insert(next) {
                prev.insert(next, cell);
                if next == dst {
                    let mut path = vec![dst];
                    let mut cur = dst;
                    while let Some(&p) = prev.get(&cur) {
                        path.push(p);
                        cur = p;
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back(next);
            }
        }
    }
    None
}

/// Wire code (official order) of the move `a -> b`.
fn direction_between(a: HexCoord, b: HexCoord) -> Option<usize> {
    Direction::ALL.iter().position(|&dir| dir.step(a) == b)
}

/// Expand a witness event list into per-agent wire codes: moves between
/// consecutive event cells along BFS paths, waits elsewhere.
fn plan_to_wire(
    board: &Board,
    map: &MapGrid,
    events: &[(u32, usize, u32)],
    start: usize,
    day_steps: u32,
) -> Vec<i64> {
    let coords: Vec<HexCoord> = (0..board.step.len()).map(|i| coord_of(map, i)).collect();
    let mut codes: Vec<i64> = Vec::new();
    let mut tick = 0u32;
    let mut pos = start;
    for &(event_tick, cell, _fuel) in events {
        if cell == pos {
            continue;
        }
        let Some(path) = shortest_path(board, pos, cell) else { break };
        for (a, b) in path.windows(2).map(|w| (w[0], w[1])) {
            let step = board.step[a];
            if tick + step > event_tick.min(day_steps) {
                break;
            }
            let Some(dir) = direction_between(coords[a], coords[b]) else { break };
            codes.push(i64::try_from(dir).expect("direction fits i64"));
            tick += step;
            pos = b;
        }
    }
    if tick < day_steps {
        codes.push(-i64::from(day_steps - tick));
    }
    codes
}

struct Recorded {
    roads: Vec<BTreeMap<usize, TrafficState>>,
}

fn load_recorded(path: &str) -> Result<Recorded, Box<dyn std::error::Error>> {
    let mut roads: Vec<BTreeMap<usize, TrafficState>> = Vec::new();
    for line in std::fs::read_to_string(path)?.lines().filter(|l| !l.trim().is_empty()) {
        let event: serde_json::Value = serde_json::from_str(line)?;
        if event["type"].as_str() != Some("day_state") {
            continue;
        }
        let observed = &event["observed"];
        let day = usize::try_from(observed["day"].as_u64().ok_or("day_state without day")?)?;
        let mut map = BTreeMap::new();
        for t in observed["traffics"].as_array().ok_or("day_state without traffics")? {
            let pos = usize::try_from(t["pos"].as_u64().ok_or("traffic pos")?)?;
            let state = match t["status"].as_u64() {
                Some(1) => TrafficState::Busy,
                Some(2) => TrafficState::Jammed,
                _ => TrafficState::Smooth,
            };
            map.insert(pos, state);
        }
        if roads.len() <= day {
            roads.resize(day + 1, BTreeMap::new());
        }
        roads[day] = map;
    }
    Ok(Recorded { roads })
}

/// Overwrite every road cell's state: `roads` where given, smooth elsewhere.
fn set_roads(state: &mut MatchState, roads: Option<&BTreeMap<usize, TrafficState>>) {
    let width = usize::from(state.setting.map.size.width);
    for cell in state.setting.map.cells_mut() {
        if let Terrain::Road(current) = &mut cell.terrain {
            let index = usize::try_from(cell.coord.row).unwrap_or(0) * width
                + usize::try_from(cell.coord.col).unwrap_or(0);
            *current = roads.and_then(|r| r.get(&index)).copied().unwrap_or(TrafficState::Smooth);
        }
    }
}

/// Play one day of a step-major plan through `MatchSim`.
fn play_day(sim: &engine::MatchSim, state: &mut MatchState, plan: &engine::model::AgentActions) {
    let n = state.agents.len().max(1);
    for block in plan.actions.chunks(n) {
        sim.step(state, &engine::model::AgentActions::new(block.to_vec()));
    }
}

#[allow(clippy::too_many_lines, reason = "one linear driver: parse, both DP modes, replay")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 2 {
        return Err("expected CASE_JSON key=value ...".into());
    }
    let values: BTreeMap<String, String> = argv[2..]
        .iter()
        .filter_map(|a| a.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())))
        .collect();
    // Key=value options; defaults are compile-time constants, so `default`
    // is always `'static` and the closure owns no borrow of `values`.
    let get = |key: &str, default: &'static str| -> String {
        values.get(key).map_or_else(|| default.to_owned(), String::from)
    };
    let case: CaseSpec = if let Some(name) = argv[1].strip_prefix("fixture:") {
        CaseSpec::from_bundled_fixture(name)?
    } else {
        serde_json::from_str(&std::fs::read_to_string(&argv[1])?)?
    };
    let pattern = kinds::parse_kinds(&get("kinds", "S,P,P,P"))?;
    let kinds = kinds::resolve_kinds(Some(&pattern), case.setting.agents.len());
    let day = usize::try_from(get("day", "0").parse::<u64>()?)?;
    let max_time = u32::try_from(*case.setting.day_steps.get(day).ok_or("day out of range")?)?;
    let max_fuel = u32::try_from(case.setting.fuel_limits)?;
    let mode: String = get("mode", "legs");
    let refuel = get("refuel", "off") == "on";
    let verify = get("verify", "on") == "on";
    let witness = get("witness", "on") == "on";
    let team = domain::PlayerId::new("player-a")?;
    let state = MatchState::new(case.to_engine_setting_with_kinds(&team, Some(&kinds))?)?;
    let board = Board::from_state(&state);
    let map = &state.setting.map;
    let patrol_starts: Vec<usize> = state
        .agents
        .iter()
        .filter(|a| a.kind == AgentKind::Patrol)
        .map(|a| index_of(map, a.pos))
        .collect();
    let supply_start: Option<usize> =
        state.agents.iter().find(|a| a.kind == AgentKind::Supply).map(|a| index_of(map, a.pos));
    println!(
        "board: {} cells, {} spots, {} series; day {} steps {max_time} fuel {max_fuel}; patrols {patrol_starts:?} supply {supply_start:?}",
        board.step.len(),
        board.spots.len(),
        board.series_count,
        day
    );
    let started = std::time::Instant::now();
    let plans: Vec<Option<PatrolPlan>> = match mode.as_str() {
        "legs" => {
            let sources: BTreeSet<usize> =
                patrol_starts.iter().copied().chain(board.spots.keys().copied()).collect();
            let mut fields: BTreeMap<usize, Vec<Vec<Cost>>> = BTreeMap::new();
            for src in sources {
                let (labels, _preds) = pareto_field(&board, src, max_time, max_fuel);
                fields.insert(src, labels);
            }
            let flat: Vec<BTreeMap<u64, (Cost, Route)>> = patrol_starts
                .iter()
                .map(|&start| {
                    flat_masks(patrol_leg_states(&board, start, max_time, max_fuel, &fields))
                })
                .collect();
            // Exact assignment: DP over patrols, state = union mask.
            let mut dp: BTreeMap<u64, Vec<Option<u64>>> = BTreeMap::new();
            dp.insert(0, vec![None; patrol_starts.len()]);
            for (p, masks) in flat.iter().enumerate() {
                let mut next: BTreeMap<u64, Vec<Option<u64>>> = BTreeMap::new();
                for (&union, picks) in &dp {
                    for &mask in masks.keys() {
                        let new_union = union | mask;
                        let mut new_picks = picks.clone();
                        new_picks[p] = Some(mask);
                        next.entry(new_union).or_insert(new_picks);
                    }
                }
                dp = next;
            }
            let best_union = dp.keys().copied().max_by_key(|m| m.count_ones()).unwrap_or(0);
            let picks = dp.get(&best_union).cloned().unwrap_or_default();
            picks
                .into_iter()
                .enumerate()
                .map(|(p, opt)| {
                    opt.map(|mask| {
                        let (total, route) = flat[p][&mask].clone();
                        let frontiers: Vec<Vec<Cost>> =
                            route.windows(2).map(|w| fields[&w[0]][w[1]].clone()).collect();
                        let legs = realize_legs(&frontiers, total);
                        let mut arrivals = Vec::new();
                        let mut events = Vec::new();
                        if let Some(legs) = legs {
                            let (mut t, mut f) = (0u32, 0u32);
                            for (i, &(lt, lf)) in legs.iter().enumerate() {
                                // Expand the chosen leg label into its full
                                // cell path (pred chain in the Pareto field).
                                let src_cell = route[i];
                                let dst_cell = route[i + 1];
                                let path =
                                    pareto_path(&fields[&src_cell], &board, dst_cell, (lt, lf));
                                for &step_cell in path.iter().skip(1) {
                                    t += board.step[
                                        // path step departs the previous cell
                                        events.last().map_or(src_cell, |e: &(u32, usize, u32)| e.1)
                                    ];
                                    f += board.fuel[events
                                        .last()
                                        .map_or(src_cell, |e: &(u32, usize, u32)| e.1)];
                                    events.push((t, step_cell, f));
                                }
                                arrivals.push((dst_cell, t, f));
                            }
                        }
                        PatrolPlan { mask, total, route, arrivals, events }
                    })
                })
                .collect()
        }
        "cell" => {
            let refuel_field = refuel.then(|| supply_start.map(|s| earliest_arrival(&board, s)));
            let per_patrol: Vec<BTreeMap<u64, PatrolPlan>> = patrol_starts
                .iter()
                .map(|&start| {
                    let field = refuel_field.as_ref().and_then(|f| f.as_deref());
                    patrol_cell_states(&board, start, max_time, max_fuel, field)
                })
                .collect();
            // Exact assignment over masks.
            let mut dp: BTreeMap<u64, Vec<Option<u64>>> = BTreeMap::new();
            dp.insert(0, vec![None; patrol_starts.len()]);
            for (p, masks) in per_patrol.iter().enumerate() {
                let mut next: BTreeMap<u64, Vec<Option<u64>>> = BTreeMap::new();
                for (&union, picks) in &dp {
                    for &mask in masks.keys() {
                        let new_union = union | mask;
                        let mut new_picks = picks.clone();
                        new_picks[p] = Some(mask);
                        next.entry(new_union).or_insert(new_picks);
                    }
                }
                dp = next;
            }
            let best_union = dp.keys().copied().max_by_key(|m| m.count_ones()).unwrap_or(0);
            let picks = dp.get(&best_union).cloned().unwrap_or_default();
            picks
                .into_iter()
                .enumerate()
                .map(|(p, opt)| opt.map(|mask| per_patrol[p][&mask].clone()))
                .collect()
        }
        other => return Err(format!("unknown mode {other:?}").into()),
    };
    let best_count: usize = plans
        .iter()
        .fold(
            {
                let mut u = 0u64;
                for plan in plans.iter().flatten() {
                    u |= plan.mask;
                }
                u
            },
            |acc, _| acc,
        )
        .count_ones() as usize;
    println!(
        "joint best: {best_count}/{} distinct series (mode={mode} refuel={refuel}, {:.1?})",
        board.series_count,
        started.elapsed()
    );
    if witness {
        for (p, plan) in plans.iter().enumerate() {
            let Some(plan) = plan else {
                println!("patrol {p} from {}: no plan", patrol_starts[p]);
                continue;
            };
            let series: Vec<usize> =
                (0..board.series_count).filter(|&s| plan.mask & (1 << s) != 0).collect();
            println!(
                "patrol {p} from {}: {} series {series:?} total (t,f)=({},{})",
                patrol_starts[p],
                series.len(),
                plan.total.0,
                plan.total.1
            );
            println!("  route (cells): {:?}", plan.route);
            println!("  arrivals: {:?}", plan.arrivals);
        }
    }
    if verify && best_count > 0 {
        let recorded = match get("traffic", "smooth").as_str() {
            "observed" => {
                let path = get("events_jsonl", "");
                if path.is_empty() {
                    return Err("traffic=observed needs events_jsonl=...".into());
                }
                Some(load_recorded(&path)?)
            }
            _ => None,
        };
        let sim = engine::MatchSim::new(case.sim_config()?);
        let mut sim_state = state.clone();
        set_roads(&mut sim_state, recorded.as_ref().and_then(|r| r.roads.get(day)));
        let steps = usize::try_from(max_time)?;
        // Per-agent sequential wire codes. `plans` is indexed by patrol
        // (agents in kind order), while the wire list is indexed by agent id —
        // assign by kind: the supply agent and any planless patrol wait all day.
        let mut patrol_iter = plans.iter();
        let lists: Vec<Vec<i64>> = sim_state
            .agents
            .iter()
            .map(|agent| {
                if agent.kind == AgentKind::Patrol {
                    match patrol_iter.next() {
                        Some(Some(plan)) => plan_to_wire(
                            &board,
                            map,
                            &plan.events,
                            index_of(map, agent.pos),
                            max_time,
                        ),
                        _ => vec![-i64::from(max_time)],
                    }
                } else {
                    vec![-i64::from(max_time)]
                }
            })
            .collect();
        // Step-major action list (`play_day` chunks by agent count).
        let step_major: Vec<engine::model::Action> = (0..steps)
            .flat_map(|step| {
                lists.iter().enumerate().map(move |(i, list)| {
                    let id = u8::try_from(i).expect("agent id");
                    let code = list.get(step).copied().unwrap_or(-1);
                    if code < 0 {
                        engine::model::Action::wait(id)
                    } else {
                        engine::model::Action::step(
                            id,
                            Direction::ALL[usize::try_from(code).expect("direction")],
                        )
                    }
                })
            })
            .collect();
        let before = (sim_state.illegal_action_count, sim_state.fuel_short_count);
        play_day(&sim, &mut sim_state, &engine::model::AgentActions::new(step_major));
        let after = (sim_state.illegal_action_count, sim_state.fuel_short_count);
        // Stock contention on the winning assignment.
        let mut per_spot: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (p, plan) in plans.iter().enumerate() {
            if let Some(plan) = plan {
                for &(cell, _, _) in &plan.arrivals {
                    per_spot.entry(cell).or_default().push(p);
                }
            }
        }
        for (cell, patrols) in &per_spot {
            if patrols.len() > 1 {
                println!(
                    "stock contention: spot cell {cell} picked by patrols {patrols:?} (max stock {})",
                    board.stock.get(cell).copied().unwrap_or(0)
                );
            }
        }
        println!(
            "replay: day {} series={:?} D={:?} B={} illegal Δ={:?} fuel_short Δ={:?} rejected={}",
            sim_state.day,
            sim_state.series_taken.iter().collect::<Vec<_>>(),
            sim_state.daily_varieties(),
            sim_state.agents.iter().map(engine::Agent::total_inventory).sum::<u32>(),
            (after.0.saturating_sub(before.0), after.1.saturating_sub(before.1)),
            after.1.saturating_sub(before.1),
            sim_state.rejected_response_count
        );
        println!(
            "replay agents: {:?}",
            sim_state
                .agents
                .iter()
                .map(|a| (a.kind, a.pos.col, a.pos.row, a.fuel))
                .collect::<Vec<_>>()
        );
    }
    Ok(())
}
