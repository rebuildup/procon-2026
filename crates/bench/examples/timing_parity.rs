//! Replay an official-server probe plan through post-parity `MatchSim` and
//! emit the day-start state the simulator predicts.
//!
//! Usage: `timing_parity CONFIG_JSON PLANS_JSON`
//!
//! `CONFIG_JSON` is the exact `config.json` the official-server probe
//! (`scripts/campaign/timing_probe.py`) started the vendored simple server
//! with: `{"problem": {...flat match problem...}, "teams": [...]}`. The
//! `problem` object is converted through `engine::from_wire`, the single
//! wire→engine path the canonical evaluator uses.
//!
//! `PLANS_JSON` is `{"days": [[[agent0 codes...], ...], ...], "kinds": [0, ...]}`
//! — the same wire plans posted to `POST /` day by day (`-N` = wait run,
//! `0..=5` = move in the official wire-code order).
//!
//! The wire plan is expanded into the engine's step-major `AgentActions`
//! exactly the way a wire plan's sequential semantics (Q23/Q24) map onto the
//! post-parity tick timeline: each order is pushed at the element tick that
//! follows the previous order's entry, and the per-agent queue + in-flight
//! record start it at the weighted completion of its predecessor. The tail
//! is padded with waits. Output is JSON Lines: one `kind:"day_start"` record
//! per day boundary (the state the official `GET /` would show at the start
//! of the next day), then a final `kind:"verdict"` record with the
//! whole-response counters.

use engine::{
    MatchSim,
    from_wire::{protocol_setting_to_engine_with_kinds, sim_config_from_wire},
    model::{Action, AgentActions, AgentKind, Direction, HexCoord, MatchState},
};
use protocol::match_config::{CellType, Map, MatchSetting as WireSetting, Spot as WireSpot};
use std::path::PathBuf;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let config_path = PathBuf::from(args.next().expect("usage: timing_parity CONFIG PLANS"));
    let plans_path = PathBuf::from(args.next().expect("usage: timing_parity CONFIG PLANS"));
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let plans: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&plans_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;

    let problem = config
        .get("problem")
        .and_then(|p| p.as_object())
        .ok_or_else(|| "config has no `problem` object".to_owned())?;
    let wire = parse_wire_setting(problem)?;
    let kinds: Option<Vec<AgentKind>> = plans
        .get("kinds")
        .and_then(|k| k.as_array())
        .map(|ks| ks.iter().map(kind_for_code).collect());

    let team = domain::PlayerId::new("local-test")?;
    let mut setting = protocol_setting_to_engine_with_kinds(&wire, team, kinds.as_deref())?;
    // The engine's team-composition validator requires 3..=8 agents; the
    // simple server happily runs fewer. Pad with dummy supply agents parked
    // on a far plain cell — they wait all day, never touch a spot, and add
    // no road stay-steps, so they cannot perturb the probed agents.
    let far = HexCoord::new(wire.map.width - 1, wire.map.height - 1);
    while setting.agents.len() < 3 {
        let id = u8::try_from(setting.agents.len()).map_err(|_| "agent count over u8")?;
        setting.agents.push(engine::model::Agent::supply(id, far));
    }
    let sim_config = sim_config_from_wire(&wire)?;
    let mut state = MatchState::new(setting)?;

    let agent_ids: Vec<u8> = state.agents.iter().map(|a| a.id).collect();
    let sim = MatchSim::new(sim_config);

    let mut days: Vec<Vec<Vec<i32>>> = Vec::new();
    for d in plans["days"].as_array().ok_or("plans.days")? {
        let mut day_body: Vec<Vec<i32>> = Vec::new();
        for a in d.as_array().ok_or("day plan")? {
            let mut codes: Vec<i32> = Vec::new();
            for c in a.as_array().ok_or("agent plan")? {
                codes.push(i32_field(c, "action code")?);
            }
            day_body.push(codes);
        }
        days.push(day_body);
    }
    // Pad the plan bodies for the dummy agents added above (all-wait).
    for day_body in &mut days {
        day_body.resize(agent_ids.len(), Vec::new());
    }

    for (index, body) in days.iter().enumerate() {
        let day = u32::try_from(index).map_err(|_| "day index over u32")?;
        let day_steps = state.setting.day_steps_for(day);
        for block in expand_wire_day(&agent_ids, body, day_steps) {
            sim.step(&mut state, &block);
        }
        emit_day_start(&state, wire.map.width, day + 1);
    }

    println!(
        "{}",
        serde_json::json!({
            "kind": "verdict",
            "days": state.day,
            "illegal_action_count": state.illegal_action_count,
            "fuel_short_count": state.fuel_short_count,
            "rejected_response_count": state.rejected_response_count,
        })
    );
    Ok(())
}

/// An `i64` JSON field narrowed to `i32` without truncation.
fn i32_field(value: &serde_json::Value, name: &str) -> Result<i32, String> {
    let raw = value.as_i64().ok_or_else(|| format!("{name} is not an integer"))?;
    i32::try_from(raw).map_err(|_| format!("{name} {raw} out of i32 range"))
}

fn kind_for_code(k: &serde_json::Value) -> AgentKind {
    match k.as_i64().unwrap_or_default() {
        0 => AgentKind::Patrol,
        _ => AgentKind::Supply,
    }
}

/// Build the wire setting from the probe's flat `problem` object.
fn parse_wire_setting(
    problem: &serde_json::Map<String, serde_json::Value>,
) -> Result<WireSetting, String> {
    let field = |key: &str| problem.get(key).ok_or_else(|| format!("problem has no {key}"));
    let width = i32_field(field("width")?, "width")?;
    let height = i32_field(field("height")?, "height")?;
    let cells_raw = field("cells")?
        .as_array()
        .ok_or("cells is not an array")?
        .iter()
        .map(|row| {
            row.as_array()
                .ok_or("cells row is not an array")?
                .iter()
                .map(|c| match c.as_i64().ok_or("cell code is not an integer")? {
                    0 => Ok(CellType::TYPE_0),
                    1 => Ok(CellType::TYPE_1),
                    2 => Ok(CellType::TYPE_2),
                    3 => Ok(CellType::TYPE_3),
                    _ => Err("cell code out of 0..=3".to_owned()),
                })
                .collect::<Result<Vec<CellType>, String>>()
        })
        .collect::<Result<Vec<Vec<CellType>>, String>>()?;
    let spots = problem["spots"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|s| {
            Ok(WireSpot {
                brand: i32_field(&s["brand"], "spot brand")?,
                pos: i32_field(&s["pos"], "spot pos")?,
                stocks: i32_field(&s["stocks"], "spot stocks")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let vector = |key: &str| -> Result<Vec<i32>, String> {
        let values = field(key)?.as_array().ok_or_else(|| format!("{key} is not an array"))?;
        values.iter().map(|v| i32_field(v, key)).collect()
    };
    Ok(WireSetting {
        starts_at: problem["startsAt"].as_i64().unwrap_or_default(),
        day_seconds: vector("daySeconds")?,
        day_steps: vector("daySteps")?,
        map: Map { height, width, cells: cells_raw },
        spots,
        agents: vector("agentStarts")?,
        fuel_limits: i32_field(field("fuelLimits")?, "fuelLimits")?,
        players: i32_field(field("players")?, "players")?,
        busy_threshold: i32_field(field("busyThreshold")?, "busyThreshold")?,
        jammed_threshold: i32_field(field("jammedThreshold")?, "jammedThreshold")?,
    })
}

/// Emit the state the official `GET /` shows at the start of `next_day`.
fn emit_day_start(state: &MatchState, width: i32, next_day: u32) {
    let agents: Vec<serde_json::Value> = state
        .agents
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "kind": match a.kind {
                    AgentKind::Patrol => 0,
                    AgentKind::Supply => 1,
                },
                "pos": a.pos.row * width + a.pos.col,
                "col": a.pos.col,
                "row": a.pos.row,
                "fuel": a.fuel,
                "fuelCap": a.fuel_cap,
            })
        })
        .collect();
    println!("{}", serde_json::json!({ "kind": "day_start", "day": next_day, "agents": agents }));
}

/// Expand one day's wire plan (per-agent weighted code list) into the
/// step-major blocks `MatchSim::step` consumes.
///
/// Sequential wire semantics (Q23/Q24) map onto the tick timeline by pushing
/// each order at the element tick that follows its predecessor's entry; the
/// per-agent queue then starts it exactly at the predecessor's weighted
/// completion (the reflection that lands a move frees the agent in the same
/// tick's action phase). The tail is padded with waits so every tick carries
/// one action per agent (`is_well_shaped`).
fn expand_wire_day(agent_ids: &[u8], body: &[Vec<i32>], day_steps: u32) -> Vec<AgentActions> {
    let day_steps = day_steps as usize;
    let mut streams: Vec<Vec<Action>> = Vec::with_capacity(agent_ids.len());
    for (id, codes) in agent_ids.iter().zip(body.iter()) {
        let mut flat: Vec<Action> = Vec::with_capacity(day_steps);
        for &code in codes {
            if code <= -1 {
                for _ in 0..code.unsigned_abs() {
                    flat.push(Action::wait(*id));
                }
            } else if let Some(dir) = wire_dir(code) {
                flat.push(Action::step(*id, dir));
            }
        }
        flat.resize(day_steps, Action::wait(*id));
        flat.truncate(day_steps);
        streams.push(flat);
    }
    (0..day_steps)
        .map(|step| AgentActions::new(streams.iter().map(|s| s[step]).collect()))
        .collect()
}

/// Wire direction code (`0..=5`, official order) -> engine direction.
fn wire_dir(code: i32) -> Option<Direction> {
    Direction::ALL.get(usize::try_from(code).ok()?).copied()
}
