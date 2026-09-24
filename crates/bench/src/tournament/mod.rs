//! Batch evaluation: N cases × M candidates in one call.
//!
//! This is the operational entry point on top of [`crate::evaluate`]. It owns
//! no scoring logic — it wires a [`crate::candidate::CandidateRegistry`] and a
//! [`crate::case::Corpus`] to the evaluator, aggregates the durable records,
//! and renders them.
//!
//! ```text
//! registry.build_all(candidate_ids)  ─┐
//!                                     ├─► Evaluator::evaluate_all ─► Vec<RunRecord>
//! corpus (fixtures / generated)     ─┘                                   │
//!                                                                         ▼
//!                                                          AggregateReport::build
//!                                                                         │
//!                                                                         ▼
//!                                            runs.jsonl + aggregate.json + markdown
//! ```
//!
//! The whole cross product is evaluated sequentially today. Nothing in the
//! shape forbids parallelism later: each `(candidate, case)` run is already
//! independent (see [`crate::evaluate`]'s failure-isolation notes) and yields
//! one self-contained record.

pub mod render;

use std::path::{Path, PathBuf};
use std::time::Duration;

use domain::PlayerId;
use serde::{Deserialize, Serialize};

use crate::aggregate::AggregateReport;
use crate::candidate::{Candidate, CandidateError, CandidateRegistry};
use crate::case::{CaseError, CaseSpec, Corpus};
use crate::evaluate::{EvalConfig, Evaluator, RunRecord};
use crate::report::{ReportError, ReportPaths};

/// Configuration for one batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentConfig {
    /// Candidate ids to evaluate, in order.
    pub candidate_ids: Vec<String>,
    /// Candidate used as the win / draw / lose anchor. Must be one of
    /// `candidate_ids`. When `None`, comparisons are reported as
    /// "not compared" rather than being invented.
    pub baseline_candidate_id: Option<String>,
    /// Team every candidate plays as.
    pub team: String,
    /// Commit SHA stamped onto every record. Defaults to
    /// [`crate::evaluate::source_commit_sha`] (`PROCON_SOURCE_COMMIT_SHA`).
    pub source_commit_sha: String,
    /// Wall deadline for a single `(candidate, case)` run.
    #[serde(with = "duration_ms")]
    pub run_timeout: Duration,
    /// Optional directory to write `runs.jsonl` + `aggregate.json` into.
    pub output_dir: Option<PathBuf>,
}

impl TournamentConfig {
    /// Build a config for a candidate list with a baseline.
    #[must_use]
    pub fn new(
        candidate_ids: Vec<String>,
        baseline_candidate_id: Option<String>,
        team: impl Into<String>,
        run_timeout: Duration,
    ) -> Self {
        Self {
            candidate_ids,
            baseline_candidate_id,
            team: team.into(),
            source_commit_sha: crate::evaluate::source_commit_sha(),
            run_timeout,
            output_dir: None,
        }
    }

    /// Stamp an explicit commit SHA (useful in tests and pinned CI runs).
    #[must_use]
    pub fn with_source_commit_sha(mut self, sha: impl Into<String>) -> Self {
        self.source_commit_sha = sha.into();
        self
    }

    /// Attach an output directory.
    #[must_use]
    pub fn with_output_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.output_dir = Some(dir.into());
        self
    }
}

mod duration_ms {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(d)?;
        Ok(Duration::from_millis(ms))
    }
}

/// Errors from running a batch. All of these are *setup* failures — a
/// candidate that fails mid-run is recorded, not raised.
#[derive(Debug, thiserror::Error)]
pub enum TournamentError {
    #[error("candidate error: {0}")]
    Candidate(#[from] CandidateError),
    #[error("case error: {0}")]
    Case(#[from] CaseError),
    #[error("report error: {0}")]
    Report(#[from] ReportError),
    #[error("no candidates supplied")]
    NoCandidates,
    #[error("no cases supplied")]
    NoCases,
    #[error("baseline candidate {0:?} is not in the candidate list")]
    UnknownBaseline(String),
}

/// The result of one batch: every record plus their aggregation.
#[derive(Debug, Clone)]
pub struct TournamentOutcome {
    /// Every `(candidate, case)` record, in evaluation order.
    pub records: Vec<RunRecord>,
    /// Aggregation of `records`.
    pub report: AggregateReport,
    /// Where the artifacts were written, when an output dir was configured.
    pub paths: Option<ReportPaths>,
}

impl TournamentOutcome {
    /// Render the report as a Markdown table set.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        render::render_markdown(&self.report)
    }
}

/// N cases × M candidates batch runner.
#[derive(Debug, Clone)]
pub struct TournamentRunner {
    config: TournamentConfig,
}

impl TournamentRunner {
    /// Construct a runner.
    #[must_use]
    pub const fn new(config: TournamentConfig) -> Self {
        Self { config }
    }

    /// Borrow the configuration.
    #[must_use]
    pub fn config(&self) -> &TournamentConfig {
        &self.config
    }

    /// Resolve the configured candidate ids against a registry.
    ///
    /// # Errors
    /// [`TournamentError::NoCandidates`], [`TournamentError::UnknownBaseline`],
    /// or [`TournamentError::Candidate`] for an unknown algorithm / parameter.
    pub fn resolve_candidates(
        &self,
        registry: &CandidateRegistry,
    ) -> Result<Vec<Candidate>, TournamentError> {
        if self.config.candidate_ids.is_empty() {
            return Err(TournamentError::NoCandidates);
        }
        let candidates = registry.build_all(&self.config.candidate_ids)?;
        if let Some(baseline) = &self.config.baseline_candidate_id {
            if !self.config.candidate_ids.iter().any(|id| id == baseline) {
                return Err(TournamentError::UnknownBaseline(baseline.clone()));
            }
        }
        Ok(candidates)
    }

    /// Run the full cross product against a corpus.
    ///
    /// # Errors
    /// Setup failures only — see [`TournamentError`]. Individual candidate
    /// failures are recorded in [`TournamentOutcome::records`] and never
    /// abort the batch.
    pub fn run(
        &self,
        registry: &CandidateRegistry,
        corpus: &Corpus,
    ) -> Result<TournamentOutcome, TournamentError> {
        self.run_cases(registry, &corpus.cases)
    }

    /// Same as [`Self::run`] against an explicit case list.
    ///
    /// # Errors
    /// Setup failures only — see [`TournamentError`].
    pub fn run_cases(
        &self,
        registry: &CandidateRegistry,
        cases: &[CaseSpec],
    ) -> Result<TournamentOutcome, TournamentError> {
        if cases.is_empty() {
            return Err(TournamentError::NoCases);
        }
        let candidates = self.resolve_candidates(registry)?;
        let team = PlayerId::new(self.config.team.as_str())
            .map_err(|e| TournamentError::Case(CaseError::Invalid(e)))?;
        let eval_config = EvalConfig {
            source_commit_sha: self.config.source_commit_sha.clone(),
            team,
            run_timeout: self.config.run_timeout,
            sim: engine::SimConfig::default(),
        };

        let records = Evaluator::new(eval_config).evaluate_all(&candidates, cases);
        let report =
            AggregateReport::build(records.clone(), self.config.baseline_candidate_id.as_deref());

        let paths = match &self.config.output_dir {
            Some(dir) => {
                let paths = ReportPaths::in_dir(dir);
                paths.write(&records, &report)?;
                Some(paths)
            }
            None => None,
        };

        Ok(TournamentOutcome { records, report, paths })
    }
}

/// Convenience: the bundled fixture corpus with a chosen subset of names.
///
/// # Errors
/// [`TournamentError::Case`] when a name is unknown or a fixture is invalid.
pub fn fixture_corpus(names: &[&str]) -> Result<Corpus, TournamentError> {
    Ok(Corpus::bundled_subset(names)?)
}

/// Convenience: every bundled fixture.
///
/// # Errors
/// [`TournamentError::Case`] when a fixture fails to load or validate.
pub fn full_fixture_corpus() -> Result<Corpus, TournamentError> {
    Ok(Corpus::bundled()?)
}

/// Path helper kept for callers that want the standard layout.
#[must_use]
pub fn report_paths(dir: impl AsRef<Path>) -> ReportPaths {
    ReportPaths::in_dir(dir)
}
