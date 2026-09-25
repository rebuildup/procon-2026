//! Canonical offline evaluation of contest solvers.
//!
//! **One evaluation path.** A candidate produces actions; [`engine::MatchSim`]
//! produces the outcome; this crate turns that outcome into the official score
//! tuple and a durable record. A solver never scores itself.
//!
//! ```text
//! candidate solver
//!     ↓   actions / decisions
//! engine::MatchSim
//!     ↓   final MatchState / RunResult
//! evaluator (this crate)
//!     ↓   official score tuple
//!        (variety, daily_variety_sum, total_balls, response_time)
//! durable result (runs.jsonl)
//! ```
//!
//! # Where things live
//!
//! | Module | Role |
//! |---|---|
//! | [`case`] | evaluation inputs: `case_id`, `seed`, `generation_parameters`, body |
//! | [`candidate`] | candidate identity (`loop:iterations=32:initial=greedy`) + registry |
//! | [`adapters`] | turn real solver crates into [`engine::model::Solver`]s |
//! | [`evaluate`] | the canonical path: candidate × case → [`evaluate::RunRecord`] |
//! | [`generator`] | seed-fixed generated cases (`--generated`), opt-in complement to the fixtures |
//! | [`aggregate`] | per-case ranking, win / draw / lose vs baseline, summaries |
//! | [`tournament`] | N cases × M candidates batch runner + Markdown rendering |
//! | [`report`] | JSONL / JSON persistence |
//! | [`fixture`] | the bundled fixed-map corpus loader |
//!
//! # Score and ranking
//!
//! The score type is [`domain::OfficialScore`] and the comparison is
//! [`domain::compare_scores`]: strictly lexicographic,
//!
//! 1. `variety` — higher is better
//! 2. `daily_variety_sum` — higher is better
//! 3. `total_balls` — higher is better
//! 4. `response_time` — **lower** is better
//!
//! The first three are deterministic given (commit, candidate config, case).
//! The fourth is measured. Wall-clock elapsed is recorded alongside it as
//! evidence and is **never** part of the ranking — environmental noise has no
//! place in a competitive score.
//!
//! # Standard workflow
//!
//! See `docs/evaluation.md` for the full walkthrough. In short:
//!
//! ```bash
//! # 1. develop a solver variant, give it a candidate id
//! # 2. evaluate it against the fixed corpus with a baseline
//! just eval
//! # 3. read target/eval/aggregate.json or the printed Markdown
//! ```

#![doc(test(attr(deny(warnings))))]

pub mod adapters;
pub mod aggregate;
pub mod candidate;
pub mod case;
pub mod evaluate;
pub mod fixture;
pub mod generator;
pub mod report;
pub mod tournament;

pub use aggregate::{
    AggregateReport, CandidateSummary, CaseRankEntry, CaseRanking, Outcome, ScoreTupleSummary,
    Stat, status_histogram,
};
pub use candidate::{
    Candidate, CandidateCatalog, CandidateError, CandidateFactory, CandidateRegistry,
    CandidateSpec, ParamSpec, parse_candidate_list,
};
pub use case::{CaseError, CaseSource, CaseSpec, Corpus};
pub use evaluate::{EvalConfig, Evaluator, RunRecord, RunStatus, source_commit_sha};
pub use fixture::{FixtureCategory, FixtureError, FixtureResult, MatchFixture, MatchFixtureLoader};
pub use generator::{
    Archetype, GENERATOR_VERSION, GenerationError, GenerationParams, MAX_GENERATED_DAYS, SizeClass,
};
pub use report::{ReportError, ReportPaths, read_runs, write_runs};
pub use tournament::{TournamentConfig, TournamentError, TournamentOutcome, TournamentRunner};

/// The official 4-tier competitive score and its lexicographic comparison.
///
/// Re-exported so callers can pin one dependency on `bench` and still compare
/// scores with the contest's own ordering. See [`domain::OfficialScore`] —
/// named `OfficialScore` rather than `Score` because `domain::Score` is an
/// unrelated turn-level points newtype.
pub use domain::{OfficialScore, compare_scores, judge};

/// The simulator every evaluation goes through.
///
/// Re-exported to make the dependency explicit: `bench` does not implement a
/// second simulator, and there is no code path that scores a run without
/// [`engine::MatchSim`].
pub use engine::MatchSim;
