//! Round-one (2026-10-10, match 3) counterfactual rollouts — Issue #403.
//!
//! Plays a whole match on one `CaseSpec` through the canonical `MatchSim`
//! while the road state of every day is taken from a recorded live run
//! (`events.jsonl`, the `day_state.observed.traffics` the server published).
//! Spot stock is per team, so the published road state is the only way other
//! teams reach our result: a plan replayed under it scores what it would
//! have scored in the match, up to our own share of the traffic average.
//!
//! Usage: `round1_rollout CASE_JSON EVENTS_JSONL key=value ...`
//!
//! - `mode=submitted` replays the wire plans recorded in `events.jsonl`;
//! - `mode=solo candidate=<id>` lets one candidate answer every day;
//! - `mode=portfolio options=<id>|<id>|... cont=<id>` asks every option for
//!   today's plan, plays the remaining days of each with `cont`, and submits
//!   the plan whose whole-match outcome is lexicographically best;
//! - `mode=beam options=... width=N [cont=<id>]` is a **hindsight** search:
//!   it keeps the N best day-end states, expands each with every option and
//!   ranks them on the recorded road state of the days still to come. It
//!   answers "does a better five-day plan exist under the roads we actually
//!   met", not "what would a solver have found without knowing them".
//!
//! `traffic=observed|smooth|own` picks the road state the real days are
//! played under; `forecast=persist|smooth` picks what a portfolio rollout
//! assumes for days it has not seen yet (never the recorded future).
//! `horizon=N` caps the rollout at N days after today.
//!
//! Output is JSON Lines: one `day` record per day and one `summary`.
use bench::{CandidateRegistry, CaseSpec};
use engine::{
    MatchSim,
    model::{
        Action, ActionKind, AgentActions, Direction, MatchState, Solver, SolverContext, Terrain,
        TrafficState,
    },
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Instant;

type Roads = BTreeMap<usize, TrafficState>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Traffic {
    Observed,
    Smooth,
    Own,
}

struct Recorded {
    /// Per day: road cell index -> state, as the server published it.
    roads: Vec<Roads>,
    /// Per day: the wire plan we submitted.
    plans: Vec<Vec<Vec<i64>>>,
}

fn load_recorded(path: &str) -> Result<Recorded, Box<dyn std::error::Error>> {
    let mut roads = BTreeMap::new();
    let mut plans = BTreeMap::new();
    for line in std::fs::read_to_string(path)?.lines().filter(|l| !l.trim().is_empty()) {
        let event: Value = serde_json::from_str(line)?;
        match event["type"].as_str() {
            Some("day_state") => {
                let observed = &event["observed"];
                let day = observed["day"].as_u64().ok_or("day_state without a day")?;
                let mut map = Roads::new();
                for t in observed["traffics"].as_array().ok_or("day_state without traffics")? {
                    let pos = usize::try_from(t["pos"].as_u64().ok_or("traffic pos")?)?;
                    let state = match t["status"].as_u64() {
                        Some(1) => TrafficState::Busy,
                        Some(2) => TrafficState::Jammed,
                        _ => TrafficState::Smooth,
                    };
                    map.insert(pos, state);
                }
                roads.insert(day, map);
            }
            Some("plan") => {
                let day = event["day"].as_u64().ok_or("plan without a day")?;
                let plan: Vec<Vec<i64>> = serde_json::from_value(event["actions"].clone())?;
                plans.insert(day, plan);
            }
            _ => {}
        }
    }
    Ok(Recorded { roads: roads.into_values().collect(), plans: plans.into_values().collect() })
}

/// Overwrite every road cell's state: `roads` where given, smooth elsewhere.
fn set_roads(state: &mut MatchState, roads: Option<&Roads>) {
    let width = usize::from(state.setting.map.size.width);
    for cell in state.setting.map.cells_mut() {
        if let Terrain::Road(current) = &mut cell.terrain {
            let index = usize::try_from(cell.coord.row).unwrap_or(0) * width
                + usize::try_from(cell.coord.col).unwrap_or(0);
            *current = roads.and_then(|r| r.get(&index)).copied().unwrap_or(TrafficState::Smooth);
        }
    }
}

fn current_roads(state: &MatchState) -> Roads {
    let width = usize::from(state.setting.map.size.width);
    state
        .setting
        .map
        .cells()
        .iter()
        .filter_map(|cell| match cell.terrain {
            Terrain::Road(s) => Some((
                usize::try_from(cell.coord.row).unwrap_or(0) * width
                    + usize::try_from(cell.coord.col).unwrap_or(0),
                s,
            )),
            _ => None,
        })
        .collect()
}

/// A recorded wire plan as the step-major action list `MatchSim` takes.
fn wire_to_plan(wire: &[Vec<i64>], steps: usize) -> AgentActions {
    let lists: Vec<Vec<Action>> = wire
        .iter()
        .enumerate()
        .map(|(i, codes)| {
            let id = u8::try_from(i).expect("agent id");
            let mut out = Vec::new();
            for &code in codes {
                if code < 0 {
                    out.extend((0..-code).map(|_| Action::wait(id)));
                } else {
                    let dir = Direction::ALL[usize::try_from(code).expect("direction")];
                    out.push(Action::step(id, dir));
                }
            }
            out.resize(steps.max(out.len()), Action::wait(id));
            out
        })
        .collect();
    let mut actions = Vec::with_capacity(steps * lists.len());
    for step in 0..steps {
        for list in &lists {
            actions.push(list[step]);
        }
    }
    AgentActions::new(actions)
}

fn play_day(sim: &MatchSim, state: &mut MatchState, plan: &AgentActions) {
    let n = state.agents.len().max(1);
    for block in plan.actions.chunks(n) {
        sim.step(state, &AgentActions::new(block.to_vec()));
    }
}

fn score(state: &MatchState) -> (usize, u32, u32) {
    (
        state.series_taken.len(),
        state.daily_varieties().iter().sum(),
        state.agents.iter().map(engine::Agent::total_inventory).sum(),
    )
}

fn cell_of(state: &MatchState, agent: usize) -> usize {
    let pos = state.agents[agent].pos;
    usize::try_from(pos.row).unwrap_or(0) * usize::from(state.setting.map.size.width)
        + usize::try_from(pos.col).unwrap_or(0)
}

/// The sequential wire answer (`0..=5` moves, negative waits) a step-major
/// plan amounts to when played from `state` — the form the server takes and
/// the independent Python replay (`tools/r1sim.py`) reads.
fn plan_to_wire(state: &MatchState, plan: &AgentActions) -> Vec<Vec<i64>> {
    let n = state.agents.len().max(1);
    let limit = state.day_steps();
    (0..n)
        .map(|i| {
            let mut pos = state.agents[i].pos;
            let (mut tick, mut out) = (0u32, Vec::<i64>::new());
            let wait = |out: &mut Vec<i64>, by: u32| match out.last_mut() {
                Some(last) if *last < 0 => *last -= i64::from(by),
                _ => out.push(-i64::from(by)),
            };
            for action in plan.actions.iter().skip(i).step_by(n) {
                if tick >= limit {
                    break;
                }
                let ActionKind::Move(dir) = action.kind else {
                    wait(&mut out, 1);
                    tick += 1;
                    continue;
                };
                let cost = state
                    .setting
                    .map
                    .get(pos)
                    .and_then(|c| engine::wire_plan::terrain_step_cost(c.terrain))
                    .unwrap_or(u32::MAX);
                if cost > limit - tick {
                    break;
                }
                let code = Direction::ALL.iter().position(|d| *d == dir).unwrap_or(0);
                out.push(i64::try_from(code).unwrap_or(0));
                tick += cost;
                pos = dir.step(pos);
            }
            if tick < limit {
                wait(&mut out, limit - tick);
            }
            out
        })
        .collect()
}

/// Run `job` over `0..count` on `threads` worker threads, results in order.
fn parallel<T: Send>(count: usize, threads: usize, job: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut out: Vec<(usize, T)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads.max(1))
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if k >= count {
                            return mine;
                        }
                        mine.push((k, job(k)));
                    }
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("worker thread")).collect()
    });
    out.sort_by_key(|(k, _)| *k);
    out.into_iter().map(|(_, v)| v).collect()
}

struct Node {
    state: MatchState,
    plans: Vec<AgentActions>,
    rank: (usize, u32, u32),
}

/// Hindsight beam over day-end states; returns the best path's day plans.
fn beam(
    sim: &MatchSim,
    start: &MatchState,
    recorded: &Recorded,
    options: &[Box<dyn Solver>],
    cont: Option<&dyn Solver>,
    width: usize,
    threads: usize,
) -> Vec<AgentActions> {
    let mut nodes = vec![Node { state: start.clone(), plans: Vec::new(), rank: (0, 0, 0) }];
    while !nodes[0].state.is_finished() {
        let day = nodes[0].state.day as usize;
        for node in &mut nodes {
            set_roads(&mut node.state, recorded.roads.get(day));
        }
        let jobs: Vec<(usize, usize)> =
            (0..nodes.len()).flat_map(|n| (0..options.len()).map(move |o| (n, o))).collect();
        let mut next: Vec<Node> = parallel(jobs.len(), threads, |k| {
            let (n, o) = jobs[k];
            let node = &nodes[n];
            let plan = options[o].decide(&SolverContext::from_state(&node.state));
            let mut state = node.state.clone();
            let before = (state.illegal_action_count, state.rejected_response_count);
            play_day(sim, &mut state, &plan);
            let clean = (state.illegal_action_count, state.rejected_response_count) == before;
            let mut future = state.clone();
            if let Some(cont) = cont {
                while !future.is_finished() {
                    let ahead = future.day as usize;
                    set_roads(&mut future, recorded.roads.get(ahead));
                    let step = cont.decide(&SolverContext::from_state(&future));
                    play_day(sim, &mut future, &step);
                }
            }
            let rank = if clean { score(&future) } else { (0, 0, 0) };
            let mut plans = node.plans.clone();
            plans.push(plan);
            Node { state, plans, rank }
        });
        next.sort_by(|a, b| b.rank.cmp(&a.rank).then(score(&b.state).cmp(&score(&a.state))));
        let mut seen = std::collections::BTreeSet::new();
        next.retain(|node| {
            seen.insert((
                node.state
                    .agents
                    .iter()
                    .map(|a| (a.pos.col, a.pos.row, a.fuel))
                    .collect::<Vec<_>>(),
                score(&node.state),
            ))
        });
        next.truncate(width.max(1));
        eprintln!(
            "beam day {day}: {} states, best rank {:?}, best so far {:?}",
            next.len(),
            next[0].rank,
            next.iter().map(|n| score(&n.state)).max()
        );
        nodes = next;
    }
    nodes.sort_by_key(|a| std::cmp::Reverse(score(&a.state)));
    nodes.swap_remove(0).plans
}

struct Args {
    values: BTreeMap<String, String>,
}

impl Args {
    fn get<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.values.get(key).map_or(default, String::as_str)
    }
}

#[allow(clippy::too_many_lines, reason = "one linear driver: parse, per-day loop, summary")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 3 {
        return Err("expected CASE_JSON EVENTS_JSONL key=value ...".into());
    }
    let args = Args {
        values: argv[3..]
            .iter()
            .filter_map(|a| a.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())))
            .collect(),
    };
    let case: CaseSpec = serde_json::from_str(&std::fs::read_to_string(&argv[1])?)?;
    let recorded = load_recorded(&argv[2])?;
    let pattern = bench::kinds::parse_kinds(args.get("kinds", "S,P,P,P"))?;
    let kinds = bench::kinds::resolve_kinds(Some(&pattern), case.setting.agents.len());
    let team = domain::PlayerId::new("player-a")?;
    let mut state = MatchState::new(case.to_engine_setting_with_kinds(&team, Some(&kinds))?)?;
    let sim = MatchSim::new(case.sim_config()?);
    let traffic = match args.get("traffic", "observed") {
        "observed" => Traffic::Observed,
        "smooth" => Traffic::Smooth,
        "own" => Traffic::Own,
        other => return Err(format!("unknown traffic {other:?}").into()),
    };
    let persist = args.get("forecast", "persist") == "persist";
    let horizon: u32 = args.get("horizon", "99").parse()?;
    let mode = args.get("mode", "solo").to_owned();
    let registry = CandidateRegistry::builtin();
    let build = |id: &str| -> Result<Box<dyn Solver>, Box<dyn std::error::Error>> {
        Ok(registry.build(id)?.build_solver()?)
    };
    let option_ids: Vec<String> = match mode.as_str() {
        "submitted" => Vec::new(),
        "solo" => vec![args.get("candidate", "").to_owned()],
        "portfolio" | "beam" => args.get("options", "").split('|').map(str::to_owned).collect(),
        other => return Err(format!("unknown mode {other:?}").into()),
    };
    let options: Vec<Box<dyn Solver>> =
        option_ids.iter().map(|id| build(id)).collect::<Result<_, _>>()?;
    let cont: Option<Box<dyn Solver>> = match args.values.get("cont") {
        Some(id) => Some(build(id)?),
        None => None,
    };
    let total_days = state.num_days();
    let scripted: Option<Vec<AgentActions>> = (mode == "beam").then(|| {
        beam(
            &sim,
            &state,
            &recorded,
            &options,
            cont.as_deref(),
            args.get("width", "8").parse().unwrap_or(8),
            args.get("threads", "8").parse().unwrap_or(8),
        )
    });
    let mut day_ms = Vec::new();
    let mut daily = Vec::new();

    while !state.is_finished() {
        let day = state.day;
        let d = day as usize;
        match traffic {
            Traffic::Observed => set_roads(&mut state, recorded.roads.get(d)),
            Traffic::Smooth => set_roads(&mut state, None),
            Traffic::Own => {}
        }
        let today = current_roads(&state);
        let steps = state.day_steps() as usize;
        let before = state.clone();
        let started = Instant::now();
        let mut option_log = Vec::new();
        let plan = if let Some(plans) = &scripted {
            plans[d].clone()
        } else if let (0, Some(path)) = (d, args.values.get("first")) {
            // `first=<json>`: a hand-built wire plan for day 0 (existence
            // checks); the candidate answers from day 1 on.
            let wire: Vec<Vec<i64>> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
            wire_to_plan(&wire, steps)
        } else if mode == "submitted" {
            wire_to_plan(recorded.plans.get(d).ok_or("no recorded plan for the day")?, steps)
        } else if options.len() == 1 && cont.is_none() {
            options[0].decide(&SolverContext::from_state(&state))
        } else {
            let cont = cont.as_deref().ok_or("mode=portfolio needs cont=<id>")?;
            // Every option plans today, then `cont` plays the days after it
            // under the forecast road state; threads stand in for the cores a
            // live coordinator would spread the options over.
            let outcomes: Vec<(AgentActions, (usize, u32, u32), bool)> =
                std::thread::scope(|scope| {
                    let handles: Vec<_> = options
                        .iter()
                        .map(|option| {
                            let (state, sim, today) = (&state, &sim, &today);
                            scope.spawn(move || {
                                let plan = option.decide(&SolverContext::from_state(state));
                                let mut future = state.clone();
                                let before =
                                    (future.illegal_action_count, future.rejected_response_count);
                                play_day(sim, &mut future, &plan);
                                let clean =
                                    (future.illegal_action_count, future.rejected_response_count)
                                        == before;
                                let mut ahead = 0;
                                while !future.is_finished() && ahead < horizon {
                                    set_roads(&mut future, persist.then_some(today));
                                    let next = cont.decide(&SolverContext::from_state(&future));
                                    play_day(sim, &mut future, &next);
                                    ahead += 1;
                                }
                                (plan, score(&future), clean)
                            })
                        })
                        .collect();
                    handles.into_iter().map(|h| h.join().expect("option thread")).collect()
                });
            let best = (0..outcomes.len())
                .rev()
                .max_by_key(|&k| (outcomes[k].2, outcomes[k].1))
                .ok_or("no options")?;
            for (k, (_, s, clean)) in outcomes.iter().enumerate() {
                option_log.push(json!({"option": option_ids[k], "rollout": [s.0, s.1, s.2],
                    "clean": clean, "chosen": k == best}));
            }
            outcomes.into_iter().nth(best).expect("best option").0
        };
        let elapsed = started.elapsed().as_millis();
        day_ms.push(elapsed);
        let wire = plan_to_wire(&state, &plan);
        play_day(&sim, &mut state, &plan);
        let acquired: Vec<u32> = state
            .series_acquired_by_day
            .get(&day)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        let balls: u32 = state.agents.iter().map(engine::Agent::total_inventory).sum::<u32>()
            - before.agents.iter().map(engine::Agent::total_inventory).sum::<u32>();
        let agents: Vec<Value> = (0..state.agents.len())
            .map(|i| {
                let moves = plan
                    .actions
                    .iter()
                    .skip(i)
                    .step_by(state.agents.len())
                    .filter(|a| matches!(a.kind, ActionKind::Move(_)))
                    .count();
                json!({"kind": format!("{:?}", state.agents[i].kind),
                    "start": cell_of(&before, i), "end": cell_of(&state, i),
                    "fuel_start": before.agents[i].fuel, "fuel_end": state.agents[i].fuel,
                    "moves": moves,
                    "balls": state.agents[i].total_inventory() - before.agents[i].total_inventory()})
            })
            .collect();
        daily.push((acquired.len(), balls));
        println!(
            "{}",
            json!({"kind": "day", "day": day, "series": acquired, "D": acquired.len(), "B": balls,
                "agents": agents, "decide_ms": u64::try_from(elapsed).unwrap_or(u64::MAX),
                "options": option_log, "wire": wire,
                "busy_roads": today.values().filter(|s| **s == TrafficState::Busy).count(),
                "jammed_roads": today.values().filter(|s| **s == TrafficState::Jammed).count()})
        );
    }
    let (v, dsum, b) = score(&state);
    println!(
        "{}",
        json!({"kind": "summary", "source_commit_sha": bench::source_commit_sha(),
            "case_id": case.case_id, "mode": mode, "label": args.get("label", ""),
            "kinds": bench::kinds::render_kinds(&kinds),
            "traffic": args.get("traffic", "observed"), "forecast": args.get("forecast", "persist"),
            "horizon": horizon.min(total_days), "options": option_ids, "cont": args.values.get("cont"),
            "V": v, "D": dsum, "B": b,
            "daily_D": daily.iter().map(|d| d.0).collect::<Vec<_>>(),
            "daily_B": daily.iter().map(|d| d.1).collect::<Vec<_>>(),
            "decide_ms": day_ms.iter().map(|m| u64::try_from(*m).unwrap_or(u64::MAX)).collect::<Vec<_>>(),
            "illegal_action_count": state.illegal_action_count,
            "fuel_short_count": state.fuel_short_count,
            "rejected_response_count": state.rejected_response_count})
    );
    Ok(())
}
