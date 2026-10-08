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
//! The id is `<algorithm>[:<key>=<value>]…`, and identity has **two layers**:
//!
//! - **Syntax** — [`CandidateSpec::parse`] → [`CandidateSpec::id`] round-trips
//!   exactly as written: the parse layer needs no registry, so an id echoed
//!   back for reporting is byte-identical to the id that produced it.
//! - **Canonical identity** — [`CandidateRegistry::resolve`] (and therefore
//!   [`CandidateRegistry::build`] / [`CandidateRegistry::build_all`]) rejects
//!   repeated keys and reorders the parameters into the schema order of
//!   [`CandidateFactory::params`]. The resolved [`Candidate`] then holds that
//!   canonical spec, so [`Candidate::id`] — and with it
//!   `RunRecord::candidate_id`, the aggregate and the report — is one unique
//!   string per configuration:
//!   `loop:iterations=32:initial=greedy` and
//!   `loop:initial=greedy:iterations=32` are one candidate and are evaluated
//!   once.
//!
//! The factory schema is the single source of truth for that order; no
//! parallel key-order table exists anywhere in this crate.
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

use std::collections::{BTreeMap, BTreeSet};
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
    #[error("parameter {key:?} is repeated; a candidate may set each key only once")]
    DuplicateParameter { key: String },
    #[error("parameter {key:?} has invalid value {value:?}: {reason}")]
    InvalidParameter { key: String, value: String, reason: String },
    #[error("failed to construct solver: {0}")]
    Build(String),
}

/// Stable identity of one evaluation candidate: algorithm + parameters.
///
/// The parameter list keeps the order it was parsed or built with — this is
/// the **syntax** layer. Canonical *schema* order is applied only when the
/// spec goes through [`CandidateRegistry::resolve`]; see the module docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateSpec {
    /// Algorithm name — the first `:`-separated segment of the id.
    pub algorithm: String,
    /// `key = value` pairs, in the order they appeared (unique keys —
    /// repeated keys are rejected by [`Self::parse`] and
    /// [`CandidateRegistry::validate`]).
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
    /// This is the registry-independent **syntax** layer: parameter order is
    /// taken as written, and [`Self::id`] renders it back unchanged. The
    /// canonical schema-ordered identity comes from
    /// [`CandidateRegistry::resolve`].
    ///
    /// # Errors
    /// Returns [`CandidateError`] for an empty id, an empty segment, a
    /// parameter segment without `=`, a parameter with an empty key or
    /// value, or a repeated parameter key. A repeated key is structurally
    /// ambiguous — `loop:iterations=32:iterations=128` has no single meaning
    /// — so it is rejected here even without a registry. Algorithm validity
    /// is *not* checked here: that is the registry's job, so ids can be
    /// parsed for reporting without one.
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
        let mut params: Vec<(String, String)> = Vec::new();
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
            if params.iter().any(|(k, _)| k == key) {
                return Err(CandidateError::DuplicateParameter { key: key.to_owned() });
            }
            params.push((key.to_owned(), value.to_owned()));
        }
        Ok(Self { algorithm: algorithm.to_owned(), params })
    }

    /// The id string for *this* parameter order.
    ///
    /// Round-trips [`Self::parse`] exactly (the syntax layer). For the
    /// canonical schema-ordered identity use [`Candidate::id`] on a
    /// [`CandidateRegistry::resolve`]d candidate.
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

    /// The **canonical** candidate id (`loop:iterations=32:initial=greedy`).
    ///
    /// A `Candidate` can only come from [`CandidateRegistry::resolve`], which
    /// reorders the parameters into [`CandidateFactory::params`] schema order,
    /// so this id — and every durable record stamped with it — is unique per
    /// configuration regardless of the order the id was written in.
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
        r.register(Arc::new(crate::adapters::MultiLoopFactory));
        r.register(Arc::new(crate::adapters::GreedyFactory));
        r.register(Arc::new(crate::adapters::NearestFactory));
        r.register(Arc::new(crate::adapters::AstarFactory));
        r.register(Arc::new(crate::adapters::CoverageFactory));
        r.register(Arc::new(crate::adapters::SearchFactory));
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
    /// Rejects unknown keys, **repeated keys** (a spec built with
    /// [`CandidateSpec::new`] bypasses [`CandidateSpec::parse`], so the
    /// ambiguity is caught here too) and missing required keys, then runs the
    /// factory's own [`CandidateFactory::validate_spec`].
    ///
    /// # Errors
    /// [`CandidateError::UnknownAlgorithm`], [`CandidateError::UnknownParameter`],
    /// [`CandidateError::DuplicateParameter`], [`CandidateError::MissingParameter`],
    /// or any error from [`CandidateFactory::validate_spec`].
    pub fn validate(&self, spec: &CandidateSpec) -> Result<(), CandidateError> {
        let Some(factory) = self.factory(&spec.algorithm) else {
            return Err(CandidateError::UnknownAlgorithm(spec.algorithm.clone()));
        };
        let schema = factory.params();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for (key, _) in &spec.params {
            if !seen.insert(key.as_str()) {
                return Err(CandidateError::DuplicateParameter { key: key.clone() });
            }
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
    /// The result holds the canonical spec (see [`Self::resolve`]), so
    /// [`Candidate::id`] is schema-ordered.
    ///
    /// # Errors
    /// [`CandidateError`] from parsing (including a repeated key) or schema
    /// validation.
    pub fn build(&self, id: &str) -> Result<Candidate, CandidateError> {
        self.resolve(CandidateSpec::parse(id)?)
    }

    /// Resolve an already-parsed spec into a [`Candidate`].
    ///
    /// This is where **canonicalisation** happens: after validation, the
    /// parameters are reordered into the order of the factory's
    /// [`CandidateFactory::params`] — the single source of truth for canonical
    /// id order. Validation runs first, so every remaining key is known and
    /// unique, and the reorder is total: nothing is dropped, nothing is
    /// invented. The returned [`Candidate`] therefore renders its
    /// [`Candidate::id`] in schema order whatever order the spec was written
    /// in.
    ///
    /// # Errors
    /// [`CandidateError`] from schema validation (including a repeated key).
    pub fn resolve(&self, spec: CandidateSpec) -> Result<Candidate, CandidateError> {
        self.validate(&spec)?;
        let factory =
            self.factory(&spec.algorithm).expect("validate() checked the factory exists").clone();
        Ok(Candidate { spec: canonicalise(spec, factory.params()), factory })
    }

    /// Parse and resolve several ids, preserving order.
    ///
    /// Ids are canonicalised as they resolve, so two order-permutations of one
    /// configuration collapse to the same [`Candidate::id`] and are rejected
    /// as a duplicate rather than evaluated twice.
    ///
    /// # Errors
    /// The first [`CandidateError`] encountered, or
    /// [`CandidateError::InvalidParameter`] when two ids name the same
    /// canonical candidate.
    pub fn build_all(&self, ids: &[String]) -> Result<Vec<Candidate>, CandidateError> {
        let mut out: Vec<Candidate> = Vec::with_capacity(ids.len());
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        for (index, id) in ids.iter().enumerate() {
            let candidate = self.build(id)?;
            let canonical = candidate.id();
            if let Some(first) = seen.insert(canonical.clone(), index) {
                return Err(CandidateError::InvalidParameter {
                    key: "candidate".to_owned(),
                    value: canonical,
                    reason: format!("duplicate of entry #{first} — same algorithm and parameters"),
                });
            }
            out.push(candidate);
        }
        Ok(out)
    }
}

/// Reorder a validated spec's parameters into schema order.
///
/// Called only after [`CandidateRegistry::validate`], which guarantees every
/// present key exists in `schema` exactly once — hence the `expect`. The
/// factory schema stays the only authority on order; nothing else in the
/// crate holds a key-order table.
fn canonicalise(mut spec: CandidateSpec, schema: &'static [ParamSpec]) -> CandidateSpec {
    let mut keyed: Vec<(usize, (String, String))> = spec
        .params
        .into_iter()
        .map(|kv| {
            let position = schema
                .iter()
                .position(|p| p.key == kv.0)
                .expect("validate() rejected unknown keys before canonicalisation");
            (position, kv)
        })
        .collect();
    keyed.sort_by_key(|(position, _)| *position);
    spec.params = keyed.into_iter().map(|(_, kv)| kv).collect();
    spec
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

/// Semantic identity of a spec: algorithm + the parameter **set**, order
/// ignored.
///
/// Two ids that differ only in parameter order share one identity, so a batch
/// cannot list the same configuration twice. Equality of parameter sets needs
/// no schema — [`CandidateSpec::parse`] has already rejected repeated keys,
/// so sorting the `(key, value)` pairs yields a canonical form for any parsed
/// spec. Exact string duplicates are a subset of this check.
fn semantic_identity(spec: &CandidateSpec) -> String {
    let mut params: Vec<(&str, &str)> =
        spec.params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    params.sort_unstable();
    let mut out = spec.algorithm.clone();
    for (key, value) in params {
        out.push(':');
        out.push_str(key);
        out.push('=');
        out.push_str(value);
    }
    out
}

/// Parse a comma/newline-separated list of candidate ids into a batch,
/// preserving order and rejecting semantic duplicates.
///
/// A duplicate means the same algorithm and parameter values, whatever their
/// order: `loop:iterations=32:initial=greedy` and
/// `loop:initial=greedy:iterations=32` are one entry, not two candidates to
/// evaluate side by side.
///
/// # Errors
/// [`CandidateError::InvalidParameter`] on a semantic duplicate; other errors
/// come from [`CandidateSpec::parse`].
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
        let identity = semantic_identity(&spec);
        if let Some(first) = seen.insert(identity.clone(), out.len()) {
            return Err(CandidateError::InvalidParameter {
                key: "candidate".to_owned(),
                value: identity,
                reason: format!("duplicate of entry #{first} — same algorithm and parameters"),
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
        for algo in ["wait", "loop", "multiloop", "greedy", "nearest", "astar"] {
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
    fn multiloop_candidate_ids_round_trip_exactly() {
        for id in [
            "multiloop:grid=2x2",
            "multiloop:grid=3x3",
            "multiloop:kmeans=4:seed=42",
            "multiloop:grid=2x2:refine=false",
        ] {
            let spec = CandidateSpec::parse(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(spec.id(), id, "id must round-trip unchanged");
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
            "loop:initial=reverse",
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
        // Canonical id order: `operators` (#191), `lookahead` (#193), `walk`
        // (#215), `assign` (#235), then `budget` (#255) appended last —
        // older ids keep their exact shape.
        assert_eq!(
            keys,
            vec![
                "iterations",
                "initial",
                "seed",
                "operators",
                "lookahead",
                "walk",
                "assign",
                "budget"
            ]
        );
        let json = serde_json::to_string_pretty(&cat).unwrap();
        assert!(json.contains("sweep-row"));
        // `reverse` ships in the `initial` value list (Issue #170).
        assert!(json.contains("reverse"));
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

    /// The whole point of canonicalisation: two order-permutations of one
    /// configuration are one candidate, so a sweep cannot pay for the same
    /// setting twice.
    #[test]
    fn resolve_canonicalises_parameter_order_to_one_id() {
        let reg = CandidateRegistry::builtin();
        let ids = ["loop:iterations=32:initial=greedy", "loop:initial=greedy:iterations=32"];
        let resolved: Vec<String> = ids
            .iter()
            .map(|id| reg.build(id).unwrap_or_else(|e| panic!("{id}: {e}")).id())
            .collect();
        assert_eq!(resolved[0], "loop:iterations=32:initial=greedy");
        assert_eq!(
            resolved[0], resolved[1],
            "input order must not survive resolution: {ids:?} → {resolved:?}"
        );

        // A three-key permutation lands on the schema order too:
        // iterations → initial → seed.
        let canonical = "loop:iterations=64:initial=random:seed=42";
        for shuffled in [
            "loop:seed=42:initial=random:iterations=64",
            "loop:initial=random:seed=42:iterations=64",
            "loop:iterations=64:seed=42:initial=random",
        ] {
            assert_eq!(
                reg.build(shuffled).unwrap().id(),
                canonical,
                "{shuffled} must canonicalise to {canonical}"
            );
        }
    }

    /// A repeated key is ambiguous on its own terms, so `parse` rejects it
    /// (no registry needed), and `validate` rejects it for a spec built with
    /// [`CandidateSpec::new`], which never went through `parse`.
    #[test]
    fn duplicate_parameter_keys_are_rejected_by_parse_and_validate() {
        assert!(matches!(
            CandidateSpec::parse("loop:iterations=32:iterations=128:initial=greedy"),
            Err(CandidateError::DuplicateParameter { key }) if key == "iterations"
        ));

        let spec = CandidateSpec::new(
            "loop",
            vec![
                ("iterations".to_owned(), "32".to_owned()),
                ("initial".to_owned(), "greedy".to_owned()),
                ("iterations".to_owned(), "128".to_owned()),
            ],
        );
        let reg = CandidateRegistry::builtin();
        assert!(
            matches!(reg.validate(&spec), Err(CandidateError::DuplicateParameter { key }) if key == "iterations"),
            "validate must catch a programmatic duplicate key, got {:?}",
            reg.validate(&spec)
        );
        assert!(matches!(reg.resolve(spec), Err(CandidateError::DuplicateParameter { .. })));
    }

    /// Semantic (order-insensitive) duplicate detection lives even where no
    /// schema is available, because a sorted `(key, value)` set needs none.
    #[test]
    fn parse_candidate_list_rejects_order_permutations_as_one_duplicate() {
        let err = parse_candidate_list(
            "loop:iterations=32:initial=greedy,loop:initial=greedy:iterations=32",
        )
        .expect_err("two spellings of one configuration are not two candidates");
        assert!(
            err.to_string().contains("duplicate of entry #0"),
            "the error must point at the entry it duplicates: {err}"
        );

        // Genuinely distinct parameter sets still pass.
        let specs = parse_candidate_list(
            "loop:iterations=32:initial=greedy,loop:iterations=64:initial=greedy,wait",
        )
        .expect("distinct configurations are one valid batch");
        assert_eq!(specs.len(), 3);
    }

    /// Post-resolution duplicates: `build_all` canonicalises first, so the
    /// permutation pair is caught even if `parse_candidate_list` was bypassed.
    #[test]
    fn build_all_rejects_canonical_duplicates() {
        let reg = CandidateRegistry::builtin();
        let err = reg
            .build_all(&[
                "loop:iterations=32:initial=greedy".to_owned(),
                "loop:initial=greedy:iterations=32".to_owned(),
            ])
            .expect_err("one configuration must not be built twice");
        assert!(
            matches!(&err, CandidateError::InvalidParameter { value, reason, .. }
                if value == "loop:iterations=32:initial=greedy" && reason.contains("entry #0")),
            "unexpected error: {err:?}"
        );

        let built = reg
            .build_all(&["wait".to_owned(), "loop:iterations=32:initial=greedy".to_owned()])
            .expect("distinct ids still build");
        assert_eq!(built.len(), 2);
    }

    /// Canonical ids are *stable*: an id that is already in schema order comes
    /// back byte-identical, so previously persisted records keep matching.
    #[test]
    fn canonical_ids_are_stable_for_every_documented_example() {
        let reg = CandidateRegistry::builtin();
        for id in [
            "loop",
            "greedy",
            "wait",
            "loop:iterations=32:initial=greedy",
            "loop:iterations=128:initial=greedy",
            "loop:iterations=64:initial=random:seed=42",
            "loop:initial=sweep-row",
            "loop:initial=sweep-col",
            "mcts:iterations=200",
        ] {
            // `resolve(parse(id)).id() == id` — the syntax layer round-trips
            // and the canonical layer does not move an already-canonical id.
            let parsed = CandidateSpec::parse(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(parsed.id(), id, "syntax round-trip: {id}");
            match reg.resolve(parsed) {
                Ok(candidate) => assert_eq!(candidate.id(), id, "canonical stability: {id}"),
                // Not registered (e.g. `mcts`) — validation rejects it before
                // canonicalisation, so its id is never silently rewritten.
                Err(err) => assert!(
                    matches!(err, CandidateError::UnknownAlgorithm(_)),
                    "{id} should fail only as unknown: {err}"
                ),
            }
        }
    }
}
