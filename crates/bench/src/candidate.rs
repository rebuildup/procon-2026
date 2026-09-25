//! Evaluation candidates: algorithm + parameters as a stable identity.
//!
//! A **candidate** is one concrete solver configuration that can be run
//! against a case. Its identity is the whole configuration, not just the
//! algorithm name:
//!
//! ```text
//! loop:iterations=32:initial=greedy
//! loop:iterations=128:initial=greedy
//! loop:iterations=64:initial=random:seed=42
//! loop:initial=sweep-row
//! loop:initial=sweep-col
//! greedy
//! mcts:iterations=200
//! ```
//!
//! The id is `<algorithm>[:<key>=<value>]…`. Parameter order is preserved
//! exactly as written and [`CandidateSpec::id`] round-trips
//! `parse → id` unchanged, so a durable record can be matched against the
//! command that produced it without a canonicalisation step.
//!
//! # One candidate is not one crate
//!
//! Registering a new algorithm is a [`CandidateFactory`] impl plus one
//! [`CandidateRegistry::register`] call. Parameter sweeps — dozens of
//! algorithms × hundreds of parameter sets × thousands of cases — are just
//! more ids in a batch, not more crates. See [`CandidateRegistry::builtin`]
//! for the factories that ship today.
//!
//! # Solvers never score themselves
//!
//! [`Candidate::build_solver`] returns a plain [`engine::model::Solver`],
//! which produces [`engine::model::AgentActions`] and nothing else. The
//! score comes from [`engine::MatchSim`] via [`crate::evaluate`].

use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::sync::Arc;

use engine::model::Solver as ModelSolver;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors from parsing or building a candidate.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CandidateError {
    #[error("empty candidate id")]
    EmptyId,
    #[error("candidate id {0:?} has an empty segment")]
    EmptySegment(String),
    #[error("candidate id {0:?} has a parameter segment without '='")]
    MalformedParam(String),
    #[error("candidate id {0:?} has a parameter without a value")]
    MissingParamValue(String),
    #[error("candidate id {0:?} has an empty parameter key")]
    EmptyParamKey(String),
    #[error("unknown algorithm {0:?}")]
    UnknownAlgorithm(String),
    #[error("algorithm {algorithm:?} does not take parameter {key:?}")]
    UnknownParameter { algorithm: String, key: String },
    #[error("algorithm {algorithm:?} is missing required parameter {key:?}")]
    MissingParameter { algorithm: String, key: String },
    #[error("parameter {key:?} has invalid value {value:?}: {reason}")]
    InvalidParameter { key: String, value: String, reason: String },
    #[error("failed to construct solver: {0}")]
    Build(String),
}

/// Stable identity of one evaluation candidate: algorithm + parameters.
///
/// The parameter list is ordered, preserving the order written in the id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateSpec {
    /// Algorithm name — the first `:`-separated segment of the id.
    pub algorithm: String,
    /// `key = value` pairs, in the order they appeared.
    pub params: Vec<(String, String)>,
}

impl CandidateSpec {
    /// Construct from an algorithm name and an ordered parameter list.
    #[must_use]
    pub fn new(algorithm: impl Into<String>, params: Vec<(String, String)>) -> Self {
        Self { algorithm: algorithm.into(), params }
    }

    /// Construct with no parameters.
    #[must_use]
    pub fn bare(algorithm: impl Into<String>) -> Self {
        Self::new(algorithm, Vec::new())
    }

    /// Parse a candidate id.
    ///
    /// # Errors
    /// Returns [`CandidateError`] for an empty id, an empty segment, a
    /// parameter segment without `=`, or a parameter with an empty key or
    /// value. Algorithm validity is *not* checked here — that is the
    /// registry's job, so ids can be parsed for reporting without a registry.
    pub fn parse(id: &str) -> Result<Self, CandidateError> {
        let id = id.trim();
        if id.is_empty() {
            return Err(CandidateError::EmptyId);
        }
        let mut segments = id.split(':');
        let algorithm = segments.next().unwrap_or_default();
        if algorithm.is_empty() {
            return Err(CandidateError::EmptySegment(id.to_owned()));
        }
        let mut params = Vec::new();
        for seg in segments {
            if seg.is_empty() {
                return Err(CandidateError::EmptySegment(id.to_owned()));
            }
            let Some((key, value)) = seg.split_once('=') else {
                return Err(CandidateError::MalformedParam(seg.to_owned()));
            };
            if key.is_empty() {
                return Err(CandidateError::EmptyParamKey(id.to_owned()));
            }
            if value.is_empty() {
                return Err(CandidateError::MissingParamValue(seg.to_owned()));
            }
            params.push((key.to_owned(), value.to_owned()));
        }
        Ok(Self { algorithm: algorithm.to_owned(), params })
    }

    /// The canonical id string. Round-trips [`Self::parse`] exactly.
    #[must_use]
    pub fn id(&self) -> String {
        let mut out = self.algorithm.clone();
        for (k, v) in &self.params {
            out.push(':');
            out.push_str(k);
            out.push('=');
            out.push_str(v);
        }
        out
    }

    /// Look up a parameter by key.
    #[must_use]
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// Parse a `u32` parameter.
    ///
    /// # Errors
    /// [`CandidateError::InvalidParameter`] when present but not a `u32`.
    pub fn param_u32(&self, key: &str) -> Result<Option<u32>, CandidateError> {
        match self.param(key) {
            None => Ok(None),
            Some(v) => v.parse::<u32>().map(Some).map_err(|_| CandidateError::InvalidParameter {
                key: key.to_owned(),
                value: v.to_owned(),
                reason: "expected an unsigned 32-bit integer".to_owned(),
            }),
        }
    }

    /// Parse a `u64` parameter.
    ///
    /// # Errors
    /// [`CandidateError::InvalidParameter`] when present but not a `u64`.
    pub fn param_u64(&self, key: &str) -> Result<Option<u64>, CandidateError> {
        match self.param(key) {
            None => Ok(None),
            Some(v) => v.parse::<u64>().map(Some).map_err(|_| CandidateError::InvalidParameter {
                key: key.to_owned(),
                value: v.to_owned(),
                reason: "expected an unsigned 64-bit integer".to_owned(),
            }),
        }
    }
}

impl Display for CandidateSpec {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id())
    }
}

/// One entry of an algorithm's parameter schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamSpec {
    /// Parameter key as written in the id.
    pub key: &'static str,
    /// Whether omitting it is an error.
    pub required: bool,
    /// Accepted values / format, for CLI help and docs.
    pub doc: &'static str,
}

/// Builds one concrete solver from a [`CandidateSpec`].
///
/// This is the extension point: a new algorithm is one impl of this trait,
/// registered on a [`CandidateRegistry`]. It is deliberately *not* tied to
/// crate boundaries so a single crate can expose several algorithms, and one
/// algorithm can be parameterised into hundreds of candidates.
pub trait CandidateFactory: Send + Sync {
    /// Algorithm name, as it appears in the first id segment.
    fn algorithm(&self) -> &'static str;

    /// Parameter schema, in canonical id order. Keys not listed here are
    /// rejected at build time.
    fn params(&self) -> &'static [ParamSpec];

    /// One-line description used by docs and CLI help.
    fn describe(&self) -> &'static str;

    /// Semantic validation beyond the parameter schema.
    ///
    /// The schema already rejects unknown / missing keys. This hook rejects
    /// *incoherent combinations* — e.g. `loop:initial=random` without a
    /// `seed` — so an invalid candidate id fails at
    /// [`CandidateRegistry::build`] rather than deep inside a run.
    ///
    /// # Errors
    /// [`CandidateError`] when the combination is not constructible.
    fn validate_spec(&self, _spec: &CandidateSpec) -> Result<(), CandidateError> {
        Ok(())
    }

    /// Build a fresh solver instance.
    ///
    /// A fresh instance per run is deliberate: if one run times out and its
    /// worker thread is abandoned mid-`decide`, the next run still gets an
    /// untouched solver.
    ///
    /// # Errors
    /// [`CandidateError`] when the spec is invalid for this algorithm, or the
    /// underlying solver refuses the configuration.
    fn build(&self, spec: &CandidateSpec) -> Result<Box<dyn ModelSolver>, CandidateError>;
}

/// A resolved candidate: its spec plus the factory that can build solvers for
/// it. The solver itself is *not* held here — see
/// [`Candidate::build_solver`] — so each evaluation run gets a fresh instance.
#[derive(Clone)]
pub struct Candidate {
    spec: CandidateSpec,
    factory: Arc<dyn CandidateFactory>,
}

impl Candidate {
    /// The candidate's stable identity.
    #[must_use]
    pub fn spec(&self) -> &CandidateSpec {
        &self.spec
    }

    /// The candidate id (`loop:iterations=32:initial=greedy`).
    #[must_use]
    pub fn id(&self) -> String {
        self.spec.id()
    }

    /// The algorithm name.
    #[must_use]
    pub fn algorithm(&self) -> &'static str {
        self.factory.algorithm()
    }

    /// Build a fresh solver instance for one evaluation run.
    ///
    /// # Errors
    /// [`CandidateError`] when the spec is invalid or construction fails.
    pub fn build_solver(&self) -> Result<Box<dyn ModelSolver>, CandidateError> {
        self.factory.build(&self.spec)
    }
}

impl fmt::Debug for Candidate {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Candidate").field("id", &self.id()).finish()
    }
}

impl Display for Candidate {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id())
    }
}

/// Maps algorithm names to factories and validates candidate ids against
/// their parameter schema.
#[derive(Default)]
pub struct CandidateRegistry {
    factories: Vec<Arc<dyn CandidateFactory>>,
}

impl CandidateRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registry pre-populated with every algorithm this crate can build today.
    ///
    /// New algorithms are added here (or via [`Self::register`] from outside
    /// the crate) — never by adding a crate and hoping the evaluator notices.
    #[must_use]
    pub fn builtin() -> Self {
        let mut r = Self::new();
        r.register(Arc::new(crate::adapters::WaitFactory));
        r.register(Arc::new(crate::adapters::LoopFactory));
        r.register(Arc::new(crate::adapters::GreedyFactory));
        r.register(Arc::new(crate::adapters::NearestFactory));
        r
    }

    /// Register a factory. Later registrations with the same algorithm name
    /// shadow earlier ones.
    pub fn register(&mut self, factory: Arc<dyn CandidateFactory>) -> &mut Self {
        self.factories.retain(|f| f.algorithm() != factory.algorithm());
        self.factories.push(factory);
        self
    }

    /// Look up a factory by algorithm name.
    #[must_use]
    pub fn factory(&self, algorithm: &str) -> Option<&Arc<dyn CandidateFactory>> {
        self.factories.iter().find(|f| f.algorithm() == algorithm)
    }

    /// Registered algorithm names, in registration order.
    #[must_use]
    pub fn algorithms(&self) -> Vec<&'static str> {
        self.factories.iter().map(|f| f.algorithm()).collect()
    }

    /// Validate a spec against its algorithm's parameter schema.
    ///
    /// # Errors
    /// [`CandidateError::UnknownAlgorithm`], [`CandidateError::UnknownParameter`],
    /// [`CandidateError::MissingParameter`].
    pub fn validate(&self, spec: &CandidateSpec) -> Result<(), CandidateError> {
        let Some(factory) = self.factory(&spec.algorithm) else {
            return Err(CandidateError::UnknownAlgorithm(spec.algorithm.clone()));
        };
        let schema = factory.params();
        for (key, _) in &spec.params {
            if !schema.iter().any(|p| p.key == key.as_str()) {
                return Err(CandidateError::UnknownParameter {
                    algorithm: spec.algorithm.clone(),
                    key: key.clone(),
                });
            }
        }
        for p in schema {
            if p.required && spec.param(p.key).is_none() {
                return Err(CandidateError::MissingParameter {
                    algorithm: spec.algorithm.clone(),
                    key: p.key.to_owned(),
                });
            }
        }
        factory.validate_spec(spec)
    }

    /// Parse and resolve a candidate id into a [`Candidate`].
    ///
    /// # Errors
    /// [`CandidateError`] from parsing or schema validation.
    pub fn build(&self, id: &str) -> Result<Candidate, CandidateError> {
        self.resolve(CandidateSpec::parse(id)?)
    }

    /// Resolve an already-parsed spec into a [`Candidate`].
    ///
    /// # Errors
    /// [`CandidateError`] from schema validation.
    pub fn resolve(&self, spec: CandidateSpec) -> Result<Candidate, CandidateError> {
        self.validate(&spec)?;
        let factory =
            self.factory(&spec.algorithm).expect("validate() checked the factory exists").clone();
        Ok(Candidate { spec, factory })
    }

    /// Parse and resolve several ids, preserving order.
    ///
    /// # Errors
    /// The first [`CandidateError`] encountered.
    pub fn build_all(&self, ids: &[String]) -> Result<Vec<Candidate>, CandidateError> {
        ids.iter().map(|id| self.build(id)).collect()
    }
}

impl fmt::Debug for CandidateRegistry {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("CandidateRegistry").field("algorithms", &self.algorithms()).finish()
    }
}

/// Human-readable catalog of a registry, for docs and CLI `--list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateCatalog {
    pub algorithms: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub algorithm: String,
    pub describe: String,
    pub params: Vec<CatalogParam>,
    /// Ready-to-copy example ids.
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogParam {
    pub key: String,
    pub required: bool,
    pub doc: String,
}

impl CandidateRegistry {
    /// Describe every registered algorithm.
    #[must_use]
    pub fn catalog(&self) -> CandidateCatalog {
        let algorithms = self
            .factories
            .iter()
            .map(|f| CatalogEntry {
                algorithm: f.algorithm().to_owned(),
                describe: f.describe().to_owned(),
                params: f
                    .params()
                    .iter()
                    .map(|p| CatalogParam {
                        key: p.key.to_owned(),
                        required: p.required,
                        doc: p.doc.to_owned(),
                    })
                    .collect(),
                examples: f
                    .params()
                    .iter()
                    .map(|p| format!("{}:{}=…", f.algorithm(), p.key))
                    .collect(),
            })
            .collect();
        CandidateCatalog { algorithms }
    }
}

/// Parse a comma/newline-separated list of candidate ids into a batch,
/// preserving order and rejecting duplicates.
///
/// # Errors
/// [`CandidateError::InvalidParameter`] on a duplicate id; other errors come
/// from [`CandidateSpec::parse`].
pub fn parse_candidate_list(text: &str) -> Result<Vec<CandidateSpec>, CandidateError> {
    let mut out: Vec<CandidateSpec> = Vec::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for line in text
        .lines()
        .flat_map(|l| l.split(','))
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let spec = CandidateSpec::parse(line)?;
        let id = spec.id();
        if let Some(first) = seen.insert(id.clone(), out.len()) {
            return Err(CandidateError::InvalidParameter {
                key: "candidate".to_owned(),
                value: id,
                reason: format!("duplicate of entry #{first}"),
            });
        }
        out.push(spec);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_id_round_trips_exactly() {
        for id in [
            "loop",
            "loop:iterations=32:initial=greedy",
            "loop:iterations=128:initial=greedy",
            "loop:iterations=64:initial=random:seed=42",
            "loop:initial=sweep-row",
            "loop:initial=sweep-col",
            "greedy",
            "nearest",
            "mcts:iterations=200",
        ] {
            let spec = CandidateSpec::parse(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(spec.id(), id, "id must round-trip unchanged");
        }
    }

    #[test]
    fn parse_extracts_algorithm_and_ordered_params() {
        let spec = CandidateSpec::parse("loop:iterations=64:initial=random:seed=42").unwrap();
        assert_eq!(spec.algorithm, "loop");
        assert_eq!(
            spec.params,
            vec![
                ("iterations".to_owned(), "64".to_owned()),
                ("initial".to_owned(), "random".to_owned()),
                ("seed".to_owned(), "42".to_owned()),
            ]
        );
        assert_eq!(spec.param("initial"), Some("random"));
        assert_eq!(spec.param("missing"), None);
        assert_eq!(spec.param_u64("seed"), Ok(Some(42)));
        assert_eq!(spec.param_u32("iterations"), Ok(Some(64)));
    }

    #[test]
    fn parse_rejects_malformed_ids() {
        assert!(matches!(CandidateSpec::parse(""), Err(CandidateError::EmptyId)));
        assert!(matches!(CandidateSpec::parse("loop:"), Err(CandidateError::EmptySegment(_))));
        assert!(matches!(
            CandidateSpec::parse("loop:iterations"),
            Err(CandidateError::MalformedParam(_))
        ));
        assert!(matches!(
            CandidateSpec::parse("loop:iterations="),
            Err(CandidateError::MissingParamValue(_))
        ));
        assert!(matches!(CandidateSpec::parse("loop:=32"), Err(CandidateError::EmptyParamKey(_))));
    }

    #[test]
    fn builtin_registry_knows_every_named_algorithm() {
        let reg = CandidateRegistry::builtin();
        for algo in ["wait", "loop", "greedy", "nearest"] {
            assert!(reg.factory(algo).is_some(), "missing algorithm {algo}");
        }
    }

    /// A registered algorithm must be usable as a bare canonical id: parse →
    /// build → id must round-trip without touching the parameter grammar.
    #[test]
    fn builtin_bare_ids_round_trip_and_build() {
        let reg = CandidateRegistry::builtin();
        for id in ["wait", "loop", "greedy", "nearest"] {
            let spec = CandidateSpec::parse(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(spec.id(), id, "bare id must round-trip unchanged");
            let built = reg.build(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(built.id(), id);
            assert!(built.build_solver().is_ok(), "{id} must build a solver");
        }
    }

    #[test]
    fn registry_rejects_unknown_algorithm_and_parameter() {
        let reg = CandidateRegistry::builtin();
        assert!(matches!(reg.build("nope:iterations=1"), Err(CandidateError::UnknownAlgorithm(_))));
        assert!(matches!(reg.build("loop:bogus=1"), Err(CandidateError::UnknownParameter { .. })));
    }

    #[test]
    fn registry_builds_loop_candidates_with_distinct_ids() {
        let reg = CandidateRegistry::builtin();
        let ids = [
            "loop:iterations=32:initial=greedy",
            "loop:iterations=128:initial=greedy",
            "loop:initial=sweep-row",
            "loop:initial=sweep-col",
            "loop:iterations=64:initial=random:seed=42",
        ];
        let mut built = Vec::new();
        for id in ids {
            let c = reg.build(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(c.id(), id);
            // Each build yields a fresh solver instance.
            assert!(c.build_solver().is_ok());
            built.push(c.id());
        }
        let unique: std::collections::BTreeSet<_> = built.iter().collect();
        assert_eq!(unique.len(), ids.len(), "candidate ids must be distinct");
    }

    #[test]
    fn catalog_lists_params_and_is_serialisable() {
        let cat = CandidateRegistry::builtin().catalog();
        let loop_entry =
            cat.algorithms.iter().find(|a| a.algorithm == "loop").expect("loop in catalog");
        let keys: Vec<_> = loop_entry.params.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, vec!["iterations", "initial", "seed"]);
        let json = serde_json::to_string_pretty(&cat).unwrap();
        assert!(json.contains("sweep-row"));
    }

    #[test]
    fn parse_candidate_list_skips_comments_and_rejects_duplicates() {
        let specs = parse_candidate_list(
            "# sweep\nloop:initial=sweep-row\nloop:initial=sweep-col\ngreedy\n",
        )
        .unwrap();
        assert_eq!(specs.len(), 3);
        assert!(parse_candidate_list("greedy\ngreedy\n").is_err());
    }
}
