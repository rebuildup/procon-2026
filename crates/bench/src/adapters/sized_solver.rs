//! One candidate id for every board size (Issue #428).
//!
//! The measured best planner depends on the board's side: `convoy` gains
//! daily variety on 16x16 boards and is the same as `fleet` or worse above
//! (its rollouts do not finish there), while `fleet`'s day-end fuel term
//! (`fuel_need` / `fuel_mix`) pays on 32x32, is neutral on 24x24 and costs
//! `convoy` a little on 16x16. A coordinator is started with one candidate
//! id before the board is known, so this candidate looks at the board and
//! hands the day to the right one. It plans nothing itself.
use crate::adapters::{ConvoyFactory, FleetFactory};
use crate::candidate::{CandidateError, CandidateFactory, CandidateSpec, ParamSpec};
use engine::model::{AgentActions, Solver, SolverContext};

pub struct SizedFactory;

const PARAMS: &[ParamSpec] = &[
    ParamSpec { key: "budget", required: true, doc: "official (required)" },
    ParamSpec {
        key: "budget_ms",
        required: false,
        doc: "wall-clock cap per day plan for boards wider than 16, 1..59000 \
              (default 0: no clock, deterministic)",
    },
    ParamSpec {
        key: "convoy_ms",
        required: false,
        doc: "wall-clock cap per day plan on boards up to 16x16 (default: budget_ms, \
              at most 20000)",
    },
    ParamSpec { key: "seed", required: false, doc: "search seed (default 42)" },
    ParamSpec {
        key: "fuel_need",
        required: false,
        doc: "fleet's fuel_need on boards wider than 16, with fuel_mix=on (default 50; 0: off)",
    },
    ParamSpec {
        key: "midleg",
        required: false,
        doc: "fleet's midleg on boards wider than 16: off (default) | on",
    },
];

/// Boards up to this side go to `convoy`.
const SMALL_SIDE: u8 = 16;

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

impl CandidateFactory for SizedFactory {
    fn algorithm(&self) -> &'static str {
        "sized"
    }
    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }
    fn describe(&self) -> &'static str {
        "convoy on boards up to 16x16, fleet with the day-end fuel term above"
    }
    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn Solver>, CandidateError> {
        super::reject_unknown_params(spec, PARAMS)?;
        if spec.param("budget") != Some("official") {
            return Err(CandidateError::Build("sized requires budget=official".into()));
        }
        let budget_ms = parse(spec, "budget_ms", 0u32)?;
        let convoy_ms = parse(spec, "convoy_ms", budget_ms.min(20_000))?;
        let seed = parse(spec, "seed", 42u64)?;
        let fuel_need = parse(spec, "fuel_need", 50u32)?;
        let pair = |key: &str, value: String| (key.to_owned(), value);
        let mut small = vec![pair("budget", "official".into()), pair("seed", seed.to_string())];
        if convoy_ms > 0 {
            small.push(pair("budget_ms", convoy_ms.to_string()));
        }
        let mut large = vec![pair("budget", "official".into()), pair("seed", seed.to_string())];
        if budget_ms > 0 {
            large.push(pair("budget_ms", budget_ms.to_string()));
            large.push(pair("adapt", "off".into()));
        } else {
            // Offline: the search length and chain count of the evidence.
            large.push(pair("iters", "60000".into()));
            large.push(pair("chains", "4".into()));
        }
        if fuel_need > 0 {
            large.push(pair("fuel_need", fuel_need.to_string()));
            large.push(pair("fuel_mix", "on".into()));
        }
        if let Some(mode) = spec.param("midleg") {
            // `fleet` validates the value.
            large.push(pair("midleg", mode.to_owned()));
        }
        Ok(Box::new(SizedCandidate {
            small: ConvoyFactory.build(&CandidateSpec::new("convoy", small))?,
            large: FleetFactory.build(&CandidateSpec::new("fleet", large))?,
        }))
    }
}

pub struct SizedCandidate {
    small: Box<dyn Solver>,
    large: Box<dyn Solver>,
}

impl Solver for SizedCandidate {
    fn name(&self) -> &'static str {
        "sized"
    }
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions {
        let size = ctx.state.setting.map.size;
        if size.width.max(size.height) <= SMALL_SIDE {
            self.small.decide(ctx)
        } else {
            self.large.decide(ctx)
        }
    }
}
