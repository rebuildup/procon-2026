//! Evaluation cases: stable identity + provenance for one input to the
//! canonical evaluator.
//!
//! A **case** is one complete match configuration the evaluator can run a
//! candidate against. Every case carries enough identity to be reproduced
//! and re-aggregated later:
//!
//! ```text
//! case_id                  stable, human-readable, unique within a corpus
//! seed                     present for generated cases
//! generation_parameters    present for generated cases (free-form JSON)
//! source                   how the case body was obtained
//! ```
//!
//! Two sources exist today:
//!
//! - [`CaseSource::Fixture`] — a fixed, hand-authored map bundled under
//!   `crates/bench/fixtures/maps/`. These are the reproducible corpus
//!   baseline. They have no seed: they are their own source of truth.
//! - [`CaseSource::Generated`] — built by a generator from a `seed` and a
//!   `generation_parameters` blob. The generator itself is **out of scope**
//!   for this module (and for issue #178); what matters here is that the
//!   schema can carry and persist `seed` / `generation_parameters` so a
//!   generated corpus can be added later without redesigning the evaluator.
//!
//! The case body is the *official wire* [`protocol::match_config::MatchSetting`]
//! — the same shape as a bundled fixture file and as `GET /setting` on the
//! live server. [`CaseSpec::to_engine_setting`] converts it through
//! [`engine::from_wire`], the single conversion shared with the CLI.

use std::fmt::{self, Display, Formatter};
use std::path::Path;

use domain::PlayerId;
use engine::model::MatchSetting as EngineSetting;
use protocol::match_config::MatchSetting as WireSetting;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::fixture::{FixtureError, MatchFixture, MatchFixtureLoader};

/// Where a case body came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaseSource {
    /// A fixed map bundled with the crate. `name` matches the fixture file
    /// stem and the [`MatchFixture::name`] field.
    Fixture { name: String },
    /// Produced by a generator from `seed` + `generation_parameters`.
    Generated,
}

impl CaseSource {
    /// Short tag used in the `case_id` prefix (`fixture` / `generated`).
    #[must_use]
    pub const fn tag(&self) -> &'static str {
        match self {
            Self::Fixture { .. } => "fixture",
            Self::Generated => "generated",
        }
    }
}

/// One evaluation input: identity, provenance, and the case body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseSpec {
    /// Stable identity, unique within a corpus. Always `<tag>:<name>`.
    pub case_id: String,
    /// RNG seed the body was generated from, when it was generated.
    ///
    /// `None` for fixed fixtures, which are their own source of truth.
    pub seed: Option<u64>,
    /// Generator knobs the body was produced from, when it was generated.
    ///
    /// Free-form JSON so a future generator can record whatever it needs
    /// (map size, series count, terrain mix, …) without a schema change.
    pub generation_parameters: Option<serde_json::Value>,
    /// How the body was obtained.
    pub source: CaseSource,
    /// The case body, in the official wire shape.
    #[serde(flatten)]
    pub setting: WireSetting,
}

impl CaseSpec {
    /// Compose a `case_id` from a tag and a name.
    #[must_use]
    pub fn compose_id(tag: &str, name: &str) -> String {
        format!("{tag}:{name}")
    }

    /// Wrap an already-loaded fixture as a case.
    #[must_use]
    pub fn from_fixture(fixture: MatchFixture) -> Self {
        let case_id = Self::compose_id("fixture", &fixture.name);
        Self {
            case_id,
            seed: None,
            generation_parameters: None,
            source: CaseSource::Fixture { name: fixture.name },
            setting: fixture.setting,
        }
    }

    /// Load a bundled fixture by name and wrap it as a case.
    ///
    /// # Errors
    /// Returns [`CaseError::Fixture`] when the fixture is unknown, or
    /// [`CaseError::Invalid`] when it fails structural validation.
    pub fn from_bundled_fixture(name: &str) -> Result<Self, CaseError> {
        let fixture = MatchFixtureLoader::new().load_bundled(name).map_err(CaseError::Fixture)?;
        fixture.validate().map_err(|e| CaseError::Invalid(e.to_string()))?;
        Ok(Self::from_fixture(fixture))
    }

    /// Wrap a fixture loaded from an arbitrary JSON file as a case.
    ///
    /// # Errors
    /// Returns [`CaseError::Fixture`] / [`CaseError::Invalid`] as above, or
    /// [`CaseError::Io`] when the file cannot be read.
    pub fn from_fixture_path(path: &Path) -> Result<Self, CaseError> {
        let fixture = MatchFixtureLoader::new().load_path(path).map_err(CaseError::Fixture)?;
        fixture.validate().map_err(|e| CaseError::Invalid(e.to_string()))?;
        Ok(Self::from_fixture(fixture))
    }

    /// Build a generated case. This is the extension point a future case
    /// generator plugs into — the generator produces the `setting` body and
    /// this constructor records the `seed` / `generation_parameters` needed
    /// to reproduce it.
    ///
    /// # Errors
    /// Returns [`CaseError::Invalid`] when `case_id` is empty or does not
    /// start with the `generated:` tag, so a generated case cannot be
    /// mistaken for a fixed fixture in a durable record.
    pub fn generated(
        case_id: impl Into<String>,
        seed: u64,
        generation_parameters: serde_json::Value,
        setting: WireSetting,
    ) -> Result<Self, CaseError> {
        let case_id = case_id.into();
        if case_id.is_empty() {
            return Err(CaseError::Invalid("case_id must not be empty".to_owned()));
        }
        if !case_id.starts_with("generated:") {
            return Err(CaseError::Invalid(format!(
                "generated case_id must start with 'generated:', got {case_id:?}"
            )));
        }
        Ok(Self {
            case_id,
            seed: Some(seed),
            generation_parameters: Some(generation_parameters),
            source: CaseSource::Generated,
            setting,
        })
    }

    /// The [`engine::SimConfig`] this case specifies — its own
    /// `busyThreshold` / `jammedThreshold`.
    ///
    /// Traffic thresholds are part of the match, not a simulator default: the
    /// bundled fixtures all declare `1/2` while
    /// [`engine::SimConfig::default`] is `2/5`. Evaluating a case with the
    /// default promotes and jams road cells on different counts, so movement
    /// costs — and therefore scores — describe a different game.
    ///
    /// # Errors
    /// Returns the Q&A Q13 range validation error when the case's thresholds
    /// are out of range.
    pub fn sim_config(&self) -> Result<engine::SimConfig, CaseError> {
        engine::from_wire::sim_config_from_wire(&self.setting).map_err(CaseError::Invalid)
    }

    /// Per-day answer budgets in seconds (`daySeconds`), as declared.
    ///
    /// Retained on the case rather than the engine setting: the simulator has
    /// no clock, so it cannot enforce a wall-clock budget. The evaluator
    /// records response time instead, and this is what that time is compared
    /// against when judging whether a plan fits its budget.
    #[must_use]
    pub fn day_seconds(&self) -> Vec<u32> {
        self.setting.day_seconds.iter().filter_map(|v| u32::try_from(*v).ok()).collect()
    }

    /// Per-day step counts (`daySteps`), as declared.
    #[must_use]
    pub fn day_steps(&self) -> Vec<u32> {
        self.setting.day_steps.iter().filter_map(|v| u32::try_from(*v).ok()).collect()
    }

    /// A stand-in case for a run whose case body could not be read (a worker
    /// handed an unreadable `--case-json`). It carries no body, so it can
    /// never be evaluated — only used to give a failure record somewhere to
    /// put its `case_id`.
    #[must_use]
    pub fn unreadable(case_id: &str) -> Self {
        Self {
            case_id: case_id.to_owned(),
            seed: None,
            generation_parameters: None,
            source: CaseSource::Generated,
            setting: protocol::match_config::MatchSetting {
                starts_at: 0,
                day_seconds: Vec::new(),
                day_steps: Vec::new(),
                map: protocol::match_config::Map {
                    height: 8,
                    width: 8,
                    cells: vec![vec![protocol::match_config::CellType::TYPE_0; 8]; 8],
                },
                spots: Vec::new(),
                agents: Vec::new(),
                fuel_limits: 1,
                players: 1,
                busy_threshold: 1,
                jammed_threshold: 2,
            },
        }
    }

    /// Convert the case body into the engine setting [`engine::MatchSim`]
    /// consumes. Goes through [`engine::from_wire`], the same conversion the
    /// live-server path uses.
    ///
    /// # Errors
    /// Returns [`CaseError::Invalid`] when the wire body is malformed in a
    /// way the engine cannot represent.
    pub fn to_engine_setting(&self, team: &PlayerId) -> Result<EngineSetting, CaseError> {
        engine::from_wire::protocol_setting_to_engine(&self.setting, team.clone())
            .map_err(CaseError::Invalid)
    }
}

impl Display for CaseSpec {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.case_id)?;
        if let Some(seed) = self.seed {
            write!(f, " seed={seed}")?;
        }
        Ok(())
    }
}

/// Errors from building or converting a case.
#[derive(Debug, Error)]
pub enum CaseError {
    #[error("fixture error: {0}")]
    Fixture(#[from] FixtureError),
    #[error("invalid case: {0}")]
    Invalid(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// A set of cases evaluated together.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Corpus {
    pub cases: Vec<CaseSpec>,
}

impl Corpus {
    /// Construct a corpus from cases, rejecting duplicate `case_id`s so a
    /// durable record can always be joined back to exactly one input.
    ///
    /// # Errors
    /// Returns [`CaseError::Invalid`] naming the first duplicate.
    pub fn new(cases: Vec<CaseSpec>) -> Result<Self, CaseError> {
        let mut seen = std::collections::BTreeSet::new();
        for c in &cases {
            if !seen.insert(c.case_id.clone()) {
                return Err(CaseError::Invalid(format!("duplicate case_id {:?}", c.case_id)));
            }
        }
        Ok(Self { cases })
    }

    /// The bundled fixed fixture corpus — every map under
    /// `crates/bench/fixtures/maps/`.
    ///
    /// # Errors
    /// Returns [`CaseError::Fixture`] if a bundled fixture fails to load, or
    /// [`CaseError::Invalid`] if one fails validation.
    pub fn bundled() -> Result<Self, CaseError> {
        let mut cases = Vec::new();
        for name in MatchFixtureLoader::bundled_fixtures() {
            cases.push(CaseSpec::from_bundled_fixture(name)?);
        }
        Self::new(cases)
    }

    /// A named subset of the bundled corpus, in the order given.
    ///
    /// # Errors
    /// As [`Self::bundled`], plus [`CaseError::Invalid`] for an unknown name.
    pub fn bundled_subset(names: &[&str]) -> Result<Self, CaseError> {
        let mut cases = Vec::new();
        for name in names {
            cases.push(CaseSpec::from_bundled_fixture(name)?);
        }
        Self::new(cases)
    }

    /// Number of cases.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cases.len()
    }

    /// True when the corpus is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cases.is_empty()
    }

    /// Case ids, in corpus order.
    #[must_use]
    pub fn case_ids(&self) -> Vec<&str> {
        self.cases.iter().map(|c| c.case_id.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn team() -> PlayerId {
        PlayerId::new("player-a").unwrap()
    }

    #[test]
    fn bundled_fixtures_become_cases_with_stable_ids() {
        let corpus = Corpus::bundled().expect("bundled corpus loads");
        assert!(corpus.len() >= 3, "need at least 3 cases for the e2e gate, got {}", corpus.len());
        for case in &corpus.cases {
            assert!(
                case.case_id.starts_with("fixture:"),
                "fixture case_id must be prefixed 'fixture:', got {:?}",
                case.case_id
            );
            assert_eq!(case.seed, None, "fixed fixtures carry no seed");
            assert!(matches!(case.source, CaseSource::Fixture { .. }));
        }
    }

    #[test]
    fn bundled_subset_is_ordered_and_non_empty() {
        let corpus = Corpus::bundled_subset(&["small-8x8", "medium-16x16"]).expect("subset loads");
        assert_eq!(corpus.case_ids(), vec!["fixture:small-8x8", "fixture:medium-16x16"]);
    }

    #[test]
    fn corpus_rejects_duplicate_case_ids() {
        let a = CaseSpec::from_bundled_fixture("small-8x8").unwrap();
        let b = CaseSpec::from_bundled_fixture("small-8x8").unwrap();
        assert!(matches!(Corpus::new(vec![a, b]), Err(CaseError::Invalid(_))));
    }

    #[test]
    fn generated_case_records_seed_and_parameters() {
        let fixture = MatchFixtureLoader::new().load_bundled("small-8x8").unwrap();
        let case = CaseSpec::generated(
            "generated:test:seed=42",
            42,
            serde_json::json!({"width": 8, "height": 8}),
            fixture.setting,
        )
        .unwrap();
        assert_eq!(case.case_id, "generated:test:seed=42");
        assert_eq!(case.seed, Some(42));
        assert_eq!(case.generation_parameters.unwrap()["width"], 8);
        assert!(matches!(case.source, CaseSource::Generated));
    }

    #[test]
    fn generated_case_requires_generated_prefix() {
        let fixture = MatchFixtureLoader::new().load_bundled("small-8x8").unwrap();
        assert!(
            CaseSpec::generated("fixture:spoof", 1, serde_json::json!({}), fixture.setting)
                .is_err()
        );
    }

    #[test]
    fn case_converts_to_engine_setting() {
        let case = CaseSpec::from_bundled_fixture("small-8x8").unwrap();
        let team = PlayerId::new("player-a").unwrap();
        let engine = case.to_engine_setting(&team).expect("converts");
        assert_eq!(engine.team, team);
        assert!(engine.num_days >= 1);
    }

    #[test]
    fn sim_config_carries_the_cases_own_traffic_thresholds() {
        let case = CaseSpec::from_bundled_fixture("small-8x8").unwrap();
        let sim = case.sim_config().unwrap();
        // The fixture declares 1/2; the simulator default is 2/5.
        assert_eq!(sim.traffic_thresholds.busy_threshold, 1);
        assert_eq!(sim.traffic_thresholds.jammed_threshold, 2);
        assert_ne!(
            sim.traffic_thresholds,
            engine::SimConfig::default().traffic_thresholds,
            "case thresholds must differ from the default, or this test proves nothing"
        );
    }

    #[test]
    fn fuel_limits_reach_the_engine_setting() {
        let case = CaseSpec::from_bundled_fixture("small-8x8").unwrap();
        assert_eq!(case.setting.fuel_limits, 24);
        let engine = case.to_engine_setting(&team()).unwrap();
        // Patrol agents (even indices) must carry the declared 24 fuel —
        // not `daySteps[0] * 3 == 48`.
        for agent in &engine.agents {
            if matches!(agent.kind, engine::model::AgentKind::Patrol) {
                assert_eq!(agent.fuel_cap, 24, "patrol must use fuelLimits");
            }
        }
    }

    #[test]
    fn per_day_steps_are_preserved() {
        let case = CaseSpec::from_bundled_fixture("small-8x8").unwrap();
        let engine = case.to_engine_setting(&team()).unwrap();
        assert_eq!(case.day_steps(), vec![16]);
        assert_eq!(engine.day_steps_by_day, vec![16]);
        assert_eq!(engine.day_steps_for(0), 16);
    }

    #[test]
    fn case_id_and_source_round_trip_json() {
        let case = CaseSpec::from_bundled_fixture("series-many").unwrap();
        let json = serde_json::to_string(&case).unwrap();
        let back: CaseSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(case, back);
    }
}
