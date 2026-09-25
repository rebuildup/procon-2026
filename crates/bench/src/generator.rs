//! Deterministic, seed-fixed generated case corpus (Issue #185).
//!
//! The bundled fixtures are the reproducible baseline, but a handful of
//! fixed maps is a small sample: a solver tuned only against them can overfit their
//! terrain quirks (or get lucky on one map's series layout). This module adds
//! a **generated corpus that complements — never replaces — the fixtures**:
//!
//! ```text
//! GenerationParams { generator_version, archetype, seed, size, days }
//!         │   in-crate xorshift64* PRNG (no new Cargo dependency)
//!         ▼
//! protocol::match_config::MatchSetting
//!         │   validate_setting — every official validator, fail fast
//!         ▼
//! CaseSpec::generated(case_id, seed, generation_parameters, setting)
//! ```
//!
//! # Guarantees
//!
//! 1. **Determinism** — the same `(generator_version, params, seed)` produces a
//!    byte-identical setting. Asserted by serialising two independent
//!    generations with `serde_json`
//!    (`same_params_and_seed_regenerate_byte_identical`).
//! 2. **Traceable identity** — `case_id` embeds generator version, archetype
//!    and seed: `generated:v1:road-heavy:seed=7`, with `:size=` / `:days=`
//!    appended when they differ from the archetype default. `seed`,
//!    `generation_parameters` (which carry `generator_version`) and `case_id`
//!    all reach `RunRecord` unchanged.
//! 3. **Validator-first** — every generated setting passes
//!    [`validate_setting`], which runs the structural fixture validator, the
//!    official `daySteps` / `daySeconds` bounds, `engine::from_wire` (the one
//!    wire→engine conversion shared with the live path) and the engine's own
//!    `MatchState::new` gate. Parameter ranges are *derived from* those
//!    validators — see the table below — and re-checked by them, so a range
//!    that drifts out from under the generator fails generation instead of
//!    quietly producing a dead case.
//! 4. **Fail fast, a distinct failure class** — a bad generation surfaces as
//!    [`GenerationError`] while the corpus is being built, *before* any
//!    candidate runs. It can never be mistaken for a candidate failure
//!    (`RunRecord` with `status: invalid | timeout | panic`), because the two
//!    live in different channels: `Result<_, GenerationError>` at corpus build
//!    vs. `Ok(RunRecord)` from the evaluator.
//!
//! # Parameter ranges (single source of truth)
//!
//! | parameter | range used here | source |
//! |---|---|---|
//! | map edge | 8 / 16 / 32 | `MapSize::MIN` / `MapSize::MAX` (`crates/engine/src/model.rs`, `MapSize::new`); 16 is the `medium-16x16` fixture size |
//! | `daySteps[i]` | `(W+H) ..= (W+H)*4` | `crates/protocol/src/match_config.rs` (`MatchSetting::day_steps`), `docs/contest/api.md` |
//! | `daySeconds[i]` | `60`, inside `1..=3600` | `domain::agent::DaySeconds` (`crates/domain/src/agent.rs`) |
//! | days | `1..=`[`MAX_GENERATED_DAYS`] | design bound for the evaluation funnel; lengths and positivity are cross-checked by `engine::from_wire` |
//! | agents | `3..=8` | `MatchSetting::validate` (`crates/engine/src/model.rs`), `GameModel::AGENT_COUNT_MIN/MAX`, `MatchFixture::validate_counts` |
//! | spots | `3..=32` | `MatchFixture::validate_counts` (`crates/bench/src/fixture.rs`), `GameModel::SPOT_COUNT_MAX` (Q&A Q15) |
//! | spot `stocks` | `1..= agent count` | Q&A Q15 (`crates/domain/src/cell.rs::Spot::new`) |
//! | `fuelLimits` | `3 * daySteps[0]`, and always `>= 2 * sum(daySteps)` | Q&A Q14 multiplier `1..=3` (`crates/engine/src/rules.rs`, `docs/contest/api.md`) + the completability invariant in [`validate_setting`] |
//! | `busyThreshold` | `1..=5` | Q&A Q13 (`crates/engine/src/rules.rs::TrafficThresholds::new`) |
//! | `jammedThreshold` | `busy+1 ..= 10` | Q&A Q13 (same function) |
//! | cell type | `0..=3` | `protocol::match_config::CellType` |
//!
//! # Archetypes
//!
//! Each archetype is *materially* distinct — not a relabel — and the
//! properties asserted for it live in this module's tests:
//!
//! | archetype | terrain / layout | spots | brands | default size | default days |
//! |---|---|---|---|---|---|
//! | `open` | ≥ 90 % plain, no pond / road | 6–14 | 2–6 | medium | 1 |
//! | `mountain-heavy` | ≥ 50 % mountain with a carved plain cross | 4–10 | 2–4 | medium | 1 |
//! | `pond-obstacle` | ≥ 10 % pond blobs, cross kept clear | 6–12 | 2–6 | medium | 2 |
//! | `road-heavy` | ≥ 15 % *connected* road lattice (row / column every 8) | 8–16 | 2–6 | medium | **3** |
//! | `many-series` | plain with mountain scatter | 8–16 | ≥ 6 | medium | **3** |
//! | `few-series` | mountain wall divides the map | 6–12 | exactly 1 | medium | 1 |
//! | `sparse-spots` | few spots kept far apart | 3–6 | 2–4 | small | 1 |
//! | `dense-spots` | clustered spot blob | 18–26 | 2–6 | large | 2 |
//!
//! Multi-day defaults matter: every bundled fixture is single-day except
//! the3-day `fixture:road-heavy-v2` (Issue #187), so on the fixture set
//! `daily_variety_sum` ≡ `variety` almost everywhere and the multi-day
//! machinery (per-day budgets, end-of-day spot refill, yesterday/today
//! traffic classification, lookahead) is barely exercised. `road-heavy` and
//! `many-series` default to 3 days, `pond-obstacle` and `dense-spots` to 2,
//! so screening can observe tier-2 differences; any archetype can be forced
//! into `1..=3` days with `:days=`.
//!
//! Multi-day cases are **front-loaded** (`daySteps[0]` is the long day, later
//! days sit at the `W+H` floor): fuel is capped at `3 x daySteps[0]` but does
//! not refill across days, so a uniform `[B,B,B]` plan would let any real
//! candidate run the tank dry. See [`validate_setting`]'s fuel invariant —
//! generated cases are completable by construction, and a case that could
//! not be is a *case-generation failure*, never a candidate failure.
//!
//! # Selection grammar (`procon-eval --generated`)
//!
//! ```text
//! <spec>  := <item> ("," <item>)*
//! <item>  := <archetype> (":" <kv>)+
//! <kv>    := "seed=" <u64>            required — explicit seeds keep
//!                                      sibling comparisons on one case axis
//!          | "size=" small|medium|large
//!          | "days=" 1..=3
//! ```
//!
//! Empty input selects nothing: generated cases are **opt-in** and `--cases
//! all` keeps meaning "the bundled fixtures only".

use std::fmt::Write as _;

use domain::PlayerId;
use engine::model::{Direction, HexCoord, MapSize};
use protocol::match_config::{CellType, Map, MatchSetting as WireSetting, Spot as WireSpot};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::case::{CaseError, CaseSpec, Corpus};
use crate::fixture::{FixtureCategory, MatchFixture};

/// Version stamp of the generator implementation.
///
/// Bump this whenever generation output would change for an existing
/// `(archetype, seed, size, days)` tuple — it is part of the `case_id`
/// (`generated:v<N>:…`) and of `generation_parameters`, so old evidence can
/// never be re-read as new.
pub const GENERATOR_VERSION: u32 = 1;

/// Inclusive upper bound on the number of days a generated case may declare.
///
/// The funnel only needs 2–3 days to make tier 2 (`daily_variety_sum`) and the
/// multi-day machinery observable; more days multiply evaluation cost without
/// adding a new axis. `engine::from_wire` additionally requires
/// `daySeconds.len() == daySteps.len()`.
pub const MAX_GENERATED_DAYS: u32 = 3;

/// Per-day answer window (seconds) given to every generated day.
///
/// Inside `domain::agent::DaySeconds`'s `1..=3600` range and identical to
/// every bundled fixture, so harness deadlines stay comparable between the
/// fixture and generated halves of a mixed batch.
const DAY_SECONDS: i32 = 60;

// ---------------------------------------------------------------------------
// SoT cross-checks: fail to compile if the engine's bounds move underneath us.
// ---------------------------------------------------------------------------

const _: () = assert!(
    MapSize::MIN <= 16 && 16 <= MapSize::MAX,
    "size class `medium` must sit inside the engine's MapSize bounds"
);
const _: () = assert!(
    MapSize::MIN == 8 && MapSize::MAX == 32,
    "size classes are keyed to MapSize::MIN/MAX — update SizeClass::edge when they change"
);

// ---------------------------------------------------------------------------
// Archetype
// ---------------------------------------------------------------------------

/// What shape of map to generate. Each variant is materially distinct; see
/// the module docs for the property table and the tests for the assertions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Archetype {
    /// Open / plain-heavy field: ≥ 90 % plain, no ponds, no roads.
    Open,
    /// ≥ 50 % mountain with a carved plain cross so routes stay connected.
    MountainHeavy,
    /// Plain base with pond obstacle blobs (≥ 10 % impassable cells).
    PondObstacle,
    /// A real road lattice (row / column every 8 cells, ≥ 15 % road) whose
    /// use-or-avoid choice changes fuel burn and traffic state.
    RoadHeavy,
    /// ≥ 6 distinct brands — variety-first solvers must work.
    ManySeries,
    /// Exactly 1 brand — total-balls solvers must work.
    FewSeries,
    /// Few (3–6) spots kept far apart, so routing dominates.
    SparseSpots,
    /// 18–26 spots in tight clusters, so scheduling dominates.
    DenseSpots,
}

impl Archetype {
    /// Every archetype, in `case_id` alphabetical order.
    pub const ALL: [Self; 8] = [
        Self::DenseSpots,
        Self::FewSeries,
        Self::ManySeries,
        Self::MountainHeavy,
        Self::Open,
        Self::PondObstacle,
        Self::RoadHeavy,
        Self::SparseSpots,
    ];

    /// Stable kebab-case name — the `case_id` segment and the JSON value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::MountainHeavy => "mountain-heavy",
            Self::PondObstacle => "pond-obstacle",
            Self::RoadHeavy => "road-heavy",
            Self::ManySeries => "many-series",
            Self::FewSeries => "few-series",
            Self::SparseSpots => "sparse-spots",
            Self::DenseSpots => "dense-spots",
        }
    }

    /// Parse the kebab-case name.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.as_str() == text)
    }

    /// The comma-separated list of valid names, for error messages.
    pub fn names() -> String {
        Self::ALL.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(", ")
    }

    /// Default size class when a selection omits `:size=`.
    #[must_use]
    pub const fn default_size(self) -> SizeClass {
        match self {
            Self::SparseSpots => SizeClass::Small,
            Self::DenseSpots => SizeClass::Large,
            _ => SizeClass::Medium,
        }
    }

    /// Default day count when a selection omits `:days=`.
    ///
    /// `road-heavy` and `many-series` default to 3 days and `pond-obstacle` /
    /// `dense-spots` to 2 so that multi-day behaviour (tier 2, per-day
    /// budgets, yesterday/today traffic) is exercised without extra flags.
    #[must_use]
    pub const fn default_days(self) -> u32 {
        match self {
            Self::RoadHeavy | Self::ManySeries => 3,
            Self::PondObstacle | Self::DenseSpots => 2,
            _ => 1,
        }
    }

    /// Inclusive spot-count range. Every value satisfies the fixture bounds
    /// `3..=32` (`crates/bench/src/fixture.rs::validate_counts`).
    #[must_use]
    pub const fn spot_range(self) -> (usize, usize) {
        match self {
            Self::Open => (6, 14),
            Self::MountainHeavy => (4, 10),
            Self::PondObstacle | Self::FewSeries => (6, 12),
            Self::RoadHeavy | Self::ManySeries => (8, 16),
            Self::SparseSpots => (3, 6),
            Self::DenseSpots => (18, 26),
        }
    }

    /// Inclusive brand-count range for a case with `spots` spots. The upper
    /// end never exceeds `spots`, so every brand can be represented.
    #[must_use]
    pub fn brand_range(self, spots: usize) -> (usize, usize) {
        match self {
            Self::ManySeries => (6, spots.min(12)),
            Self::FewSeries => (1, 1),
            Self::MountainHeavy => (2, spots.min(4)),
            Self::Open
            | Self::PondObstacle
            | Self::RoadHeavy
            | Self::SparseSpots
            | Self::DenseSpots => (2, spots.min(6)),
        }
    }

    /// Fixture category used for the structural validation pass. The
    /// `series_many` / `series_few` categories carry brand assertions, which
    /// doubles as a build-time materiality check for those two archetypes.
    #[must_use]
    pub const fn fixture_category(self, size: SizeClass) -> FixtureCategory {
        match self {
            Self::ManySeries => FixtureCategory::SeriesMany,
            Self::FewSeries => FixtureCategory::SeriesFew,
            _ => size.fixture_category(),
        }
    }
}

impl std::fmt::Display for Archetype {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Size class
// ---------------------------------------------------------------------------

/// Map edge length class. Square maps only: `small` = 8×8, `medium` = 16×16,
/// `large` = 32×32.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SizeClass {
    /// 8×8 — `MapSize::MIN`.
    Small,
    /// 16×16 — the `medium-16x16` fixture size.
    Medium,
    /// 32×32 — `MapSize::MAX`.
    Large,
}

impl SizeClass {
    /// Every size class, small to large.
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

    /// Edge length in cells, derived from the engine's `MapSize` bounds.
    #[must_use]
    pub fn edge(self) -> i32 {
        match self {
            Self::Small => i32::from(MapSize::MIN),
            Self::Medium => 16,
            Self::Large => i32::from(MapSize::MAX),
        }
    }

    /// Stable lowercase name — the `case_id` segment and the JSON value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    /// Parse the lowercase name.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == text)
    }

    /// Neutral fixture category (no brand assertions) for the structural
    /// validation pass.
    #[must_use]
    pub const fn fixture_category(self) -> FixtureCategory {
        match self {
            Self::Small => FixtureCategory::Small,
            Self::Medium => FixtureCategory::Medium,
            Self::Large => FixtureCategory::Large,
        }
    }
}

impl std::fmt::Display for SizeClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// Everything needed to regenerate one case byte-for-byte.
///
/// Stored verbatim into `CaseSpec::generation_parameters`, which `RunRecord`
/// copies — so a durable record carries generator version, archetype, seed,
/// size and day count without any extra schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationParams {
    /// [`GENERATOR_VERSION`] of the implementation that must reproduce this.
    pub generator_version: u32,
    /// Which map archetype to paint.
    pub archetype: Archetype,
    /// RNG seed. Sibling candidates are compared on the *same* seed set.
    pub seed: u64,
    /// Map edge class (8 / 16 / 32).
    pub size: SizeClass,
    /// Number of days (`daySteps` / `daySeconds` entries).
    pub days: u32,
}

impl GenerationParams {
    /// Parameters for `archetype` at `seed`, using the archetype's default
    /// size and day count.
    #[must_use]
    pub const fn new(archetype: Archetype, seed: u64) -> Self {
        Self {
            generator_version: GENERATOR_VERSION,
            archetype,
            seed,
            size: archetype.default_size(),
            days: archetype.default_days(),
        }
    }

    /// The `size` that does not need to be spelled out in the `case_id`.
    #[must_use]
    pub const fn default_size(&self) -> SizeClass {
        self.archetype.default_size()
    }

    /// The `days` that does not need to be spelled out in the `case_id`.
    #[must_use]
    pub const fn default_days(&self) -> u32 {
        self.archetype.default_days()
    }

    /// Compose the stable `case_id`.
    ///
    /// Grammar (reverse-parseable to generator version + archetype + seed):
    ///
    /// ```text
    /// generated:v<version>:<archetype>:seed=<seed>[:size=<class>][:days=<n>]
    /// ```
    ///
    /// `:size=` / `:days=` appear only when they differ from the archetype
    /// default, so the common screening id stays short
    /// (`generated:v1:road-heavy:seed=7`) while two specs that differ in size
    /// or day count can never collide inside one `Corpus`.
    #[must_use]
    pub fn case_id(&self) -> String {
        let mut id = format!(
            "generated:v{}:{}:seed={}",
            self.generator_version,
            self.archetype.as_str(),
            self.seed
        );
        if self.size != self.default_size() {
            let _ = write!(id, ":size={}", self.size.as_str());
        }
        if self.days != self.default_days() {
            let _ = write!(id, ":days={}", self.days);
        }
        id
    }

    /// `generation_parameters` payload — a flat JSON object carrying
    /// `generator_version`, `archetype`, `seed`, `size` and `days`.
    ///
    /// # Errors
    /// Only if `serde_json` cannot encode the struct, which cannot happen for
    /// these scalar fields; mapped into [`GenerationError::Invalid`] so the
    /// caller keeps exactly one failure channel.
    pub fn to_json(&self) -> Result<serde_json::Value, GenerationError> {
        serde_json::to_value(self)
            .map_err(|e| self.invalid(format!("parameters are not encodable: {e}")))
    }

    /// Build the generation failure for these parameters.
    #[must_use]
    pub fn invalid(&self, cause: impl Into<String>) -> GenerationError {
        GenerationError::Invalid { archetype: self.archetype, seed: self.seed, cause: cause.into() }
    }

    /// Fail fast on parameters no implementation could honour.
    fn check(&self) -> Result<(), GenerationError> {
        if self.generator_version != GENERATOR_VERSION {
            return Err(self.invalid(format!(
                "unsupported generator version {} (this build implements v{GENERATOR_VERSION}); \
                 a version this build cannot reproduce must never reach evaluation",
                self.generator_version
            )));
        }
        if self.days == 0 || self.days > MAX_GENERATED_DAYS {
            return Err(
                self.invalid(format!("days {} outside 1..={MAX_GENERATED_DAYS}", self.days))
            );
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A **case generation** failure: the corpus could not be built.
///
/// Deliberately a different type, returned from a different channel, than a
/// candidate failure. Candidate failures are `RunRecord`s with
/// `status: invalid | timeout | panic` produced by the evaluator *after* the
/// corpus exists; a [`GenerationError`] means no `CaseSpec` was produced at
/// all, so no record can exist for it. Every message starts with
/// `"case generation failure"` so logs cannot blur the two.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GenerationError {
    /// A generation invariant or an official validator rejected the output.
    #[error("case generation failure: archetype={archetype} seed={seed}: {cause}")]
    Invalid {
        /// Archetype being generated.
        archetype: Archetype,
        /// Seed being generated.
        seed: u64,
        /// What went wrong (validator message or invariant name).
        cause: String,
    },
    /// The `--generated` selection string could not be parsed.
    #[error("case generation failure: cannot parse selection {spec:?}: {reason}")]
    Selection {
        /// The offending selection text (one item, or the whole spec).
        spec: String,
        /// Why it was rejected.
        reason: String,
    },
    /// The selection parsed and generated, but the resulting corpus was
    /// unusable (duplicate `case_id`).
    #[error("case generation failure: {0}")]
    Corpus(String),
}

// ---------------------------------------------------------------------------
// PRNG — xorshift64*, same precedent as `loop:initial=random`
// ---------------------------------------------------------------------------

/// Small deterministic PRNG.
///
/// Mirrors `crates/solver-loop`'s `seeded_shuffle`: a fixed xorshift64\*
/// sequence instead of an external RNG crate, so `seed → setting` stays
/// stable across compiler and dependency versions — which is what makes a
/// generated case re-generable from its durable record.
struct Rng {
    state: u64,
}

impl Rng {
    /// Seed the stream with `seed`, mixed with the archetype name and the
    /// generator version so two archetypes at the same seed do not walk the
    /// same sequence.
    fn new(seed: u64, archetype: Archetype, version: u32) -> Self {
        let mut state = seed.wrapping_add(0x9E37_79B9_7F4A_7C15).max(1);
        for byte in archetype.as_str().bytes() {
            state = xorshift64star(state ^ u64::from(byte));
        }
        state = xorshift64star(state ^ u64::from(version));
        Self { state }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = xorshift64star(self.state);
        self.state
    }

    /// Value in `0..bound` (`bound` is clamped to at least 1).
    fn below(&mut self, bound: usize) -> usize {
        let b = u64::try_from(bound.max(1)).unwrap_or(1);
        usize::try_from(self.next_u64() % b).unwrap_or(0)
    }

    /// Inclusive value in `lo..=hi`. Callers keep `lo <= hi`.
    fn inclusive(&mut self, lo: usize, hi: usize) -> usize {
        lo.saturating_add(self.below(hi.saturating_sub(lo).saturating_add(1)))
    }

    /// In-place Fisher–Yates shuffle driven by this stream.
    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.inclusive(0, i);
            items.swap(i, j);
        }
    }

    /// `count` distinct indices from `0..total`, in random order.
    fn sample_distinct(&mut self, total: usize, count: usize) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..total).collect();
        self.shuffle(&mut idx);
        idx.into_iter().take(count.min(total)).collect()
    }
}

/// One xorshift64* step. Non-zero for any non-zero input.
fn xorshift64star(mut x: u64) -> u64 {
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

// ---------------------------------------------------------------------------
// Generation entry points
// ---------------------------------------------------------------------------

/// Generate one case from explicit parameters — the deterministic generator
/// API.
///
/// # Errors
/// [`GenerationError::Invalid`] naming archetype + seed + cause when the
/// parameters are unsupported, when an internal invariant cannot be met, or
/// when the generated setting is rejected by [`validate_setting`].
pub fn generate(params: &GenerationParams) -> Result<CaseSpec, GenerationError> {
    params.check()?;
    let setting = build_setting(params)?;
    validate_setting(params, &setting)?;
    let case_id = params.case_id();
    CaseSpec::generated(case_id, params.seed, params.to_json()?, setting)
        .map_err(|e| params.invalid(format!("case wrapper rejected the generated case: {e}")))
}

/// Build a corpus from explicit parameters, failing fast on the first bad
/// one.
///
/// This is the fail-fast seam: a generation failure stops the batch *before*
/// any candidate runs, so it can never surface as a `RunRecord`.
///
/// # Errors
/// The first [`GenerationError`] encountered (generation or duplicate id).
pub fn build_corpus(params: &[GenerationParams]) -> Result<Corpus, GenerationError> {
    let mut cases = Vec::with_capacity(params.len());
    for p in params {
        cases.push(generate(p)?);
    }
    Corpus::new(cases).map_err(|e| match e {
        CaseError::Invalid(msg) => GenerationError::Corpus(msg),
        other => GenerationError::Corpus(other.to_string()),
    })
}

/// Parse a `--generated` selection into parameters, in written order.
///
/// An empty (or whitespace-only) selection selects nothing — generated cases
/// are opt-in.
///
/// # Errors
/// [`GenerationError::Selection`] naming the offending item and reason.
pub fn parse_selection(spec: &str) -> Result<Vec<GenerationParams>, GenerationError> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    trimmed.split(',').map(|item| parse_item(item.trim())).collect()
}

/// Parse one `<archetype>:key=value…` item.
fn parse_item(item: &str) -> Result<GenerationParams, GenerationError> {
    let reject = |reason: String| GenerationError::Selection { spec: item.to_owned(), reason };
    let mut parts = item.split(':');
    let name = parts.next().unwrap_or_default();
    let Some(archetype) = Archetype::parse(name) else {
        return Err(reject(format!(
            "unknown archetype {name:?}; expected one of: {}",
            Archetype::names()
        )));
    };
    let mut params = GenerationParams::new(archetype, 0);
    let (mut seed_seen, mut size_seen, mut days_seen) = (false, false, false);
    for kv in parts {
        let Some((key, value)) = kv.split_once('=') else {
            return Err(reject(format!("expected key=value, got {kv:?}")));
        };
        match key {
            "seed" => {
                if seed_seen {
                    return Err(reject("duplicate key `seed`".to_owned()));
                }
                params.seed = value
                    .parse::<u64>()
                    .map_err(|_| reject(format!("seed {value:?} is not a u64")))?;
                seed_seen = true;
            }
            "size" => {
                if size_seen {
                    return Err(reject("duplicate key `size`".to_owned()));
                }
                params.size = SizeClass::parse(value).ok_or_else(|| {
                    reject(format!("size {value:?}; expected small | medium | large"))
                })?;
                size_seen = true;
            }
            "days" => {
                if days_seen {
                    return Err(reject("duplicate key `days`".to_owned()));
                }
                params.days = value
                    .parse::<u32>()
                    .map_err(|_| reject(format!("days {value:?} is not an integer")))?;
                days_seen = true;
            }
            other => {
                return Err(reject(format!("unknown key {other:?}; expected seed | size | days")));
            }
        }
    }
    if !seed_seen {
        return Err(reject(
            "seed= is required — explicit seeds keep sibling comparisons on one case axis"
                .to_owned(),
        ));
    }
    Ok(params)
}

// ---------------------------------------------------------------------------
// The validation gate
// ---------------------------------------------------------------------------

/// Run every official validator against a generated setting.
///
/// Called by [`generate`] for *every* case, so a setting that would die at
/// evaluation time (appearing as a candidate `invalid` record) instead fails
/// at corpus-build time as a [`GenerationError`].
///
/// Layers, outermost first:
///
/// 1. [`MatchFixture::validate`] — the same structural gate fixtures pass at
///    load (dimensions, agent / spot counts, cell shape, positions, no agent
///    on a spot or on `Pond`, series-category brand assertions);
/// 2. day-structure bounds (`daySteps` per-day official range, `daySeconds`
///    range, equal lengths, expected day count);
/// 3. [`check_fuel_budget`] — Q&A Q14's `1..=3 x daySteps[0]` cap *and* the
///    completability invariant `fuel >= 2 x sum(daySteps)`, which is what
///    makes a fuel-short (`invalid`) run structurally impossible rather than
///    merely unlikely;
/// 4. spot `stocks <= agent count` (Q&A Q15, which no wire validator
///    enforces on this path);
/// 5. `engine::from_wire::sim_config_from_wire` — Q&A Q13 thresholds;
/// 6. `engine::from_wire::protocol_setting_to_engine` — the single wire→engine
///    conversion;
/// 7. `engine::model::MatchState::new` — the engine's own
///    `MatchSetting::validate` gate, exactly what `MatchSim::run` runs.
///
/// # Errors
/// [`GenerationError::Invalid`] naming archetype, seed and the failing check.
pub fn validate_setting(
    params: &GenerationParams,
    setting: &WireSetting,
) -> Result<(), GenerationError> {
    // Structural shape first: it owns the map dimensions, so the day-range
    // check below reads dimensions that are already known to be legal.
    let fixture = MatchFixture::new(
        format!("generated-{}", params.archetype.as_str()),
        "generated case under validation",
        params.archetype.fixture_category(params.size),
        setting.clone(),
    );
    fixture.validate().map_err(|e| params.invalid(format!("structural validation: {e}")))?;

    check_day_structure(params, setting)?;
    check_fuel_budget(params, setting)?;

    let agent_count = setting.agents.len();
    for (i, spot) in setting.spots.iter().enumerate() {
        let Ok(stocks) = usize::try_from(spot.stocks) else {
            return Err(params.invalid(format!(
                "spots[{i}].stocks {} is not a usable positive value",
                spot.stocks
            )));
        };
        if stocks == 0 || stocks > agent_count {
            return Err(params.invalid(format!(
                "spots[{i}].stocks {stocks} outside 1..={agent_count} (Q&A Q15)"
            )));
        }
    }

    engine::from_wire::sim_config_from_wire(setting)
        .map_err(|e| params.invalid(format!("traffic thresholds (Q&A Q13): {e}")))?;

    let team = PlayerId::new("player-a")
        .map_err(|e| params.invalid(format!("validation team id is unusable: {e}")))?;
    let engine_setting = engine::from_wire::protocol_setting_to_engine(setting, team)
        .map_err(|e| params.invalid(format!("engine::from_wire rejected the setting: {e}")))?;
    engine::model::MatchState::new(engine_setting)
        .map_err(|e| params.invalid(format!("engine setting validator: {e}")))?;
    Ok(())
}

/// Day-structure checks: the axis every bundled fixture is missing.
fn check_day_structure(
    params: &GenerationParams,
    setting: &WireSetting,
) -> Result<(), GenerationError> {
    let days = setting.day_steps.len();
    if days != setting.day_seconds.len() {
        return Err(params.invalid(format!(
            "daySteps has {days} entries but daySeconds has {}",
            setting.day_seconds.len()
        )));
    }
    if u32::try_from(days).is_ok_and(|d| d != params.days) {
        return Err(
            params.invalid(format!("declared {} days but the setting carries {days}", params.days))
        );
    }
    let span = setting.map.width.saturating_add(setting.map.height);
    let (lo, hi) = (span, span.saturating_mul(4));
    for (i, steps) in setting.day_steps.iter().enumerate() {
        if !(lo..=hi).contains(steps) {
            return Err(params.invalid(format!(
                "daySteps[{i}] = {steps} outside the official [{lo}, {hi}] (W+H ..= (W+H)*4)"
            )));
        }
    }
    for (i, secs) in setting.day_seconds.iter().enumerate() {
        if !(*secs > 0 && *secs <= 3_600) {
            return Err(params.invalid(format!(
                "daySeconds[{i}] = {secs} outside 1..=3600 (domain::agent::DaySeconds)"
            )));
        }
    }
    Ok(())
}

/// The fuel invariant: the official cap and the completability bound together.
///
/// Two facts collide here:
///
/// - **The cap.** `fuelLimits` must sit in `1..=3 x daySteps[0]` (Q&A Q14;
///   `docs/contest/api.md`, `docs/contest/openapi.md` — the official sample
///   declares `daySteps [64,64,64]` → `fuelLimits 192` = 3x).
/// - **The burn does not reset.** Fuel is a per-agent tank that persists
///   across the whole match: only a supply-agent contact refills it. So burn
///   scales with `sum(daySteps)` over *all* days while the cap scales only
///   with `daySteps[0]`.
///
/// Worst-case burn for one agent is `2 x sum(daySteps)` — the maximum terrain
/// fuel cost is 2 (`engine::rules::fuel_cost`: plain 1, mountain / road 2) and
/// an agent acts at most once per step — so the generator guarantees
///
/// ```text
/// 3 x daySteps[0] >= fuelLimits >= 2 x sum(daySteps)
/// ```
///
/// Without the right-hand inequality a uniform multi-day case such as
/// `[24,24,24]` with `fuelLimits 72` is legal on paper yet every real
/// candidate runs the tank dry and the whole corpus column comes back
/// `invalid` — invalid-by-construction. This check turns that generator bug
/// into an explicit case-generation failure.
fn check_fuel_budget(
    params: &GenerationParams,
    setting: &WireSetting,
) -> Result<(), GenerationError> {
    let Some(&day0) = setting.day_steps.first() else {
        return Err(params.invalid("setting declares no day steps"));
    };
    let fuel = i64::from(setting.fuel_limits);
    let cap = i64::from(day0).saturating_mul(3);
    let floor = i64::from(day0);
    if !(floor..=cap).contains(&fuel) {
        return Err(params.invalid(format!(
            "fuelLimits {fuel} outside {floor}..={cap} — Q&A Q14 allows 1..=3 x daySteps[0] \
             (docs/contest/api.md)"
        )));
    }
    let total: i64 = setting.day_steps.iter().map(|s| i64::from(*s)).sum();
    let worst_case_burn = total.saturating_mul(2);
    if fuel < worst_case_burn {
        return Err(params.invalid(format!(
            "fuelLimits {fuel} cannot cover the worst-case burn {worst_case_burn} \
             (2 fuel x {total} total steps across {} days); fuel does not refill at day \
             boundaries while Q&A Q14 caps fuelLimits at 3 x daySteps[0], so this case would \
             be invalid-by-construction",
            setting.day_steps.len()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Setting construction
// ---------------------------------------------------------------------------

/// Paint a full wire setting for `params`.
fn build_setting(params: &GenerationParams) -> Result<WireSetting, GenerationError> {
    let edge = usize::try_from(params.size.edge())
        .map_err(|_| params.invalid("size class does not fit a usable edge length"))?;
    let mut rng = Rng::new(params.seed, params.archetype, params.generator_version);

    // Counts first: painting then guarantees capacity for exactly these.
    let (spot_lo, spot_hi) = params.archetype.spot_range();
    let spot_count = rng.inclusive(spot_lo, spot_hi);
    // Q&A Q15: a team can never field more agents than there are spots.
    let agent_count = rng.inclusive(3, spot_count.min(8));

    let mut cells = paint(params, &mut rng, edge);
    ensure_plain_capacity(params, &mut cells, spot_count + agent_count, &mut rng)?;

    let spots_idx = place_spots(params, &mut rng, &cells, edge, spot_count)?;
    let agents_idx = place_agents(params, &mut rng, &cells, edge, &spots_idx, agent_count)?;

    // Day structure — front-loaded, inside the official W+H ..= 4(W+H).
    let base_steps = params.size.edge().saturating_add(params.size.edge());
    let day_steps = plan_day_steps(params, &mut rng, base_steps)?;
    let day_seconds = vec![DAY_SECONDS; days_len(params)];
    // Q&A Q14: fuel cap = 1..=3 x day-1 steps. The generator always picks the
    // top of the range (12x the floor of a single `W+H` day) *and* keeps the
    // completability invariant `fuel >= 2 x sum(daySteps)`, so no legal plan
    // can run the tank dry — see `check_fuel_budget`.
    let fuel_limits = day_steps.first().copied().unwrap_or(base_steps).saturating_mul(3);

    // Q&A Q13 thresholds, inside the ranges `TrafficThresholds::new` checks.
    let busy_idx = rng.inclusive(1, 5);
    let busy =
        i32::try_from(busy_idx).map_err(|_| params.invalid("busy threshold out of range"))?;
    let jammed = i32::try_from(rng.inclusive(busy_idx.saturating_add(1), 10))
        .map_err(|_| params.invalid("jammed threshold out of range"))?;

    // Spots: brands + stocks (both inside their documented ranges).
    let (brand_lo, brand_hi) = params.archetype.brand_range(spot_count);
    let brands = rng.inclusive(brand_lo, brand_hi);
    let mut spots = Vec::with_capacity(spots_idx.len());
    for (i, &pos) in spots_idx.iter().enumerate() {
        let brand = i % brands;
        let stocks = rng.inclusive(1, agent_count);
        spots.push(WireSpot {
            brand: i32::try_from(brand)
                .map_err(|_| params.invalid("brand index does not fit the wire field"))?,
            pos: i32::try_from(pos)
                .map_err(|_| params.invalid("spot position does not fit the wire field"))?,
            stocks: i32::try_from(stocks)
                .map_err(|_| params.invalid("spot stock does not fit the wire field"))?,
        });
    }
    let mut agents = Vec::with_capacity(agents_idx.len());
    for &pos in &agents_idx {
        agents.push(
            i32::try_from(pos)
                .map_err(|_| params.invalid("agent position does not fit the wire field"))?,
        );
    }

    let height = i32::try_from(edge)
        .map_err(|_| params.invalid("map edge does not fit the wire map fields"))?;
    Ok(WireSetting {
        starts_at: 0,
        day_seconds,
        day_steps,
        map: Map { height, width: height, cells },
        spots,
        agents,
        fuel_limits,
        players: 1,
        busy_threshold: busy,
        jammed_threshold: jammed,
    })
}

/// Number of day entries actually built for `params`.
fn days_len(params: &GenerationParams) -> usize {
    usize::try_from(params.days).unwrap_or(1)
}

/// Per-day step counts, **front-loaded** so the Q&A Q14 fuel cap can cover
/// the whole match (see [`check_fuel_budget`]).
///
/// With `base = W + H` (the official per-day floor, so every entry stays in
/// `base ..= 4*base`):
///
/// | days | plan | why |
/// |---|---|---|
/// | 1 | `m * base`, `m in 1..=4` | burn `2*base <= fuel 3*m*base` holds for every `m` |
/// | 2 | `m*base` with `m in 3..=4`, then `base ..= day0/2` | `2*(day0+day1) <= 3*day0` keeps the tank alive into day 2 |
/// | 3 | `[4*base, base, base]` — forced | `day1,day2 >= base` and `day0 <= 4*base` leave exactly one solution |
///
/// The uniform alternative (`[B,B,B]` with `3B` fuel) is *unreachable*: the
/// worst-case burn is `2*3B = 6B > 3B`, so every candidate would run dry and
/// the case would be invalid-by-construction. Non-uniform days are also the
/// axis the fixture corpus never exercises.
fn plan_day_steps(
    params: &GenerationParams,
    rng: &mut Rng,
    base: i32,
) -> Result<Vec<i32>, GenerationError> {
    let mult = |m: usize| -> Result<i32, GenerationError> {
        let m = i32::try_from(m).map_err(|_| params.invalid("day-step multiplier out of range"))?;
        Ok(base.saturating_mul(m))
    };
    match params.days {
        1 => Ok(vec![mult(rng.inclusive(1, 4))?]),
        2 => {
            let day0 = mult(rng.inclusive(3, 4))?;
            // day1 in [base, day0 / 2]: day0 >= 3*base keeps that range
            // non-empty, and the upper bound is exactly what
            // `2 * (day0 + day1) <= 3 * day0` needs.
            let headroom = usize::try_from(day0 / 2)
                .unwrap_or(0)
                .saturating_sub(usize::try_from(base).unwrap_or(0));
            let offset = i32::try_from(rng.inclusive(0, headroom))
                .map_err(|_| params.invalid("day-step offset out of range"))?;
            Ok(vec![day0, base.saturating_add(offset)])
        }
        _ => Ok(vec![mult(4)?, base, base]),
    }
}

// ---------------------------------------------------------------------------
// Terrain painting
// ---------------------------------------------------------------------------

/// Paint the archetype's terrain into a fresh `edge x edge` grid.
fn paint(params: &GenerationParams, rng: &mut Rng, edge: usize) -> Vec<Vec<CellType>> {
    let mut cells = vec![vec![CellType::TYPE_0; edge]; edge];
    match params.archetype {
        Archetype::Open => paint_open(&mut cells, rng),
        Archetype::MountainHeavy => paint_mountain_heavy(&mut cells, rng, edge),
        Archetype::PondObstacle => paint_pond_obstacle(&mut cells, rng, edge),
        Archetype::RoadHeavy => paint_road_heavy(&mut cells, rng),
        Archetype::ManySeries => paint_many_series(&mut cells, rng, edge),
        Archetype::FewSeries => paint_few_series(&mut cells, edge),
        Archetype::SparseSpots => paint_scatter(&mut cells, rng, 8),
        Archetype::DenseSpots => paint_scatter(&mut cells, rng, 16),
    }
    cells
}

/// Open / plain-heavy (at least 90 % plain): a sprinkle of singletons, never
/// a pond, never a road.
fn paint_open(cells: &mut [Vec<CellType>], rng: &mut Rng) {
    let total = cells.len() * cells.len();
    let mountains = (total / 64).max(1);
    for idx in rng.sample_distinct(total, mountains) {
        set_cell(cells, idx, CellType::TYPE_1);
    }
}

/// Mountain-heavy (at least 50 % mountain): everything is mountain except a
/// carved plain cross plus a grown fringe, so agents and spots always have a
/// connected plain spine.
fn paint_mountain_heavy(cells: &mut [Vec<CellType>], rng: &mut Rng, edge: usize) {
    for row in cells.iter_mut() {
        for cell in row.iter_mut() {
            *cell = CellType::TYPE_1;
        }
    }
    let cross = carve_cross(cells, edge);
    let target = cross.len().saturating_add(edge * edge / 16);
    grow_region(cells, &cross, target, CellType::TYPE_0, &[], rng);
}

/// Plain base with pond blobs (>= 10 % of cells), never on the cross.
fn paint_pond_obstacle(cells: &mut [Vec<CellType>], rng: &mut Rng, edge: usize) {
    let total = edge * edge;
    let target_pond = (total / 8).max(4);
    let skip = cross_indices(edge);
    let want_centers = (target_pond / 7).max(1);
    let mut centers = Vec::new();
    for idx in rng.sample_distinct(total, total) {
        if skip.contains(&idx) {
            continue;
        }
        centers.push(idx);
        if centers.len() == want_centers {
            break;
        }
    }
    for &idx in &centers {
        set_cell(cells, idx, CellType::TYPE_2);
    }
    grow_region(cells, &centers, target_pond, CellType::TYPE_2, &skip, rng);
    // A little terrain texture so routing is not a pure obstacle course.
    paint_scatter(cells, rng, 64);
}

/// A real road lattice: whole rows and columns every 8 cells are road
/// (>= 15 % of cells, connected by construction), plus scattered mountains so
/// the plain detour exists but is not free.
fn paint_road_heavy(cells: &mut [Vec<CellType>], rng: &mut Rng) {
    for (r, row) in cells.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            if r % 8 == 0 || c % 8 == 0 {
                *cell = CellType::TYPE_3;
            }
        }
    }
    paint_scatter(cells, rng, 64);
}

/// Plain with a mountain scatter (~20 %) — the terrain is neutral here; the
/// material property is the brand count.
fn paint_many_series(cells: &mut [Vec<CellType>], rng: &mut Rng, edge: usize) {
    paint_scatter_on(cells, rng, edge * edge / 5, &cross_indices(edge));
}

/// A mountain wall (two full rows and two full columns) divides the plain —
/// the material property is the single brand.
fn paint_few_series(cells: &mut [Vec<CellType>], edge: usize) {
    let wall_row = edge / 3;
    let wall_col = edge * 2 / 3;
    for offset in 0..2 {
        if let Some(row) = cells.get_mut(wall_row + offset) {
            for cell in row.iter_mut() {
                *cell = CellType::TYPE_1;
            }
        }
        for row in cells.iter_mut() {
            if let Some(cell) = row.get_mut(wall_col + offset) {
                *cell = CellType::TYPE_1;
            }
        }
    }
}

/// Sprinkle `total / divisor` mountains over non-cross cells. Sparsity and
/// density come from placement, not from terrain.
fn paint_scatter(cells: &mut [Vec<CellType>], rng: &mut Rng, divisor: usize) {
    let edge = cells.len();
    let total = edge * edge;
    let mountains = (total / divisor.max(1)).max(1);
    paint_scatter_on(cells, rng, mountains, &cross_indices(edge));
}

/// Turn `count` eligible cells into mountains: plain cells only, never on the
/// cross — so scattering texture can never eat a road lattice, a pond field,
/// or the carved corridor.
fn paint_scatter_on(cells: &mut [Vec<CellType>], rng: &mut Rng, count: usize, skip: &[usize]) {
    let total = cells.len() * cells.len();
    let mut pool: Vec<usize> = (0..total)
        .filter(|i| !skip.contains(i) && get_cell(cells, *i) == Some(CellType::TYPE_0))
        .collect();
    rng.shuffle(&mut pool);
    for idx in pool.into_iter().take(count) {
        set_cell(cells, idx, CellType::TYPE_1);
    }
}

// ---------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------

/// Pick `count` plain cells for spots, honouring the archetype's spatial
/// policy (generic spread, minimum separation for `sparse-spots`, clustering
/// for `dense-spots`).
fn place_spots(
    params: &GenerationParams,
    rng: &mut Rng,
    cells: &[Vec<CellType>],
    edge: usize,
    count: usize,
) -> Result<Vec<usize>, GenerationError> {
    match params.archetype {
        Archetype::SparseSpots => place_sparse_spots(params, rng, cells, edge, count),
        Archetype::DenseSpots => place_dense_spots(params, rng, cells, edge, count),
        _ => sample_ranked(rng, spot_buckets(cells, edge), count)
            .ok_or_else(|| params.invalid("not enough plain cells for the requested spot count")),
    }
}

/// Ranked buckets of spot-capable cells: the carved cross first (guaranteed
/// connected plain), then the rest.
fn spot_buckets(cells: &[Vec<CellType>], edge: usize) -> Vec<Vec<usize>> {
    let cross = cross_indices(edge);
    let mut preferred = Vec::new();
    let mut rest = Vec::new();
    for (idx, cell) in cells.iter().flatten().enumerate() {
        if *cell != CellType::TYPE_0 {
            continue;
        }
        if cross.contains(&idx) {
            preferred.push(idx);
        } else {
            rest.push(idx);
        }
    }
    vec![preferred, rest]
}

/// `sparse-spots`: greedily accept plain cells that keep a minimum grid
/// distance from every spot already placed.
fn place_sparse_spots(
    params: &GenerationParams,
    rng: &mut Rng,
    cells: &[Vec<CellType>],
    edge: usize,
    count: usize,
) -> Result<Vec<usize>, GenerationError> {
    let min_gap = (edge / 3).max(2);
    let mut buckets = spot_buckets(cells, edge);
    for b in &mut buckets {
        rng.shuffle(b);
    }
    let mut chosen: Vec<usize> = Vec::with_capacity(count);
    'spots: for idx in buckets.into_iter().flatten() {
        let (r, c) = (idx / edge, idx % edge);
        for &other in &chosen {
            let (or_, oc) = (other / edge, other % edge);
            if r.abs_diff(or_).max(c.abs_diff(oc)) < min_gap {
                continue 'spots;
            }
        }
        chosen.push(idx);
        if chosen.len() == count {
            return Ok(chosen);
        }
    }
    Err(params
        .invalid(format!("could not place {count} spots with a minimum gap of {min_gap} cells")))
}

/// `dense-spots`: rank every plain cell by squared distance to the nearest of
/// a few cluster centres (seed-dependent tie-break) and take the closest —
/// the spots are a tight blob by construction.
fn place_dense_spots(
    params: &GenerationParams,
    rng: &mut Rng,
    cells: &[Vec<CellType>],
    edge: usize,
    count: usize,
) -> Result<Vec<usize>, GenerationError> {
    let mut candidates: Vec<usize> = spot_buckets(cells, edge).into_iter().flatten().collect();
    if candidates.len() < count {
        return Err(params.invalid("not enough plain cells for the dense spot cluster"));
    }
    rng.shuffle(&mut candidates);
    let center_count = if edge >= 32 { 3 } else { 2 };
    let centers: Vec<usize> = candidates.iter().copied().take(center_count).collect();

    let mut keyed: Vec<(u64, u64, usize)> = Vec::with_capacity(candidates.len());
    for &idx in &candidates {
        let jitter = rng.next_u64();
        keyed.push((min_dist2(idx, &centers, edge), jitter, idx));
    }
    keyed.sort_unstable();
    Ok(keyed.into_iter().take(count).map(|(_, _, idx)| idx).collect())
}

/// Squared grid distance from `idx` to the nearest of `centers`.
fn min_dist2(idx: usize, centers: &[usize], edge: usize) -> u64 {
    let (r, c) = (idx / edge, idx % edge);
    centers
        .iter()
        .map(|&other| {
            let dr = u64::try_from(r.abs_diff(other / edge)).unwrap_or(0);
            let dc = u64::try_from(c.abs_diff(other % edge)).unwrap_or(0);
            dr * dr + dc * dc
        })
        .min()
        .unwrap_or(u64::MAX)
}

/// Pick `count` agent start cells: passable, not a spot, preferring the
/// carved cross then plain cells.
fn place_agents(
    params: &GenerationParams,
    rng: &mut Rng,
    cells: &[Vec<CellType>],
    edge: usize,
    occupied: &[usize],
    count: usize,
) -> Result<Vec<usize>, GenerationError> {
    let cross = cross_indices(edge);
    let mut preferred = Vec::new();
    let mut plain = Vec::new();
    let mut other = Vec::new();
    for (idx, cell) in cells.iter().flatten().enumerate() {
        if *cell == CellType::TYPE_2 || occupied.contains(&idx) {
            continue;
        }
        if cross.contains(&idx) {
            preferred.push(idx);
        } else if *cell == CellType::TYPE_0 {
            plain.push(idx);
        } else {
            other.push(idx);
        }
    }
    sample_ranked(rng, vec![preferred, plain, other], count)
        .ok_or_else(|| params.invalid("not enough passable cells for the requested agent count"))
}

/// Take `count` cells from ranked buckets, preferring earlier buckets.
fn sample_ranked(rng: &mut Rng, buckets: Vec<Vec<usize>>, count: usize) -> Option<Vec<usize>> {
    let available: usize = buckets.iter().map(Vec::len).sum();
    if available < count {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    let mut remaining = count;
    for mut bucket in buckets {
        if remaining == 0 {
            break;
        }
        rng.shuffle(&mut bucket);
        let take = remaining.min(bucket.len());
        out.extend(bucket.into_iter().take(take));
        remaining -= take;
    }
    Some(out)
}

/// Guarantee at least `needed` plain cells by growing the plain region from
/// where it already is (so the growth stays connected to the corridors).
fn ensure_plain_capacity(
    params: &GenerationParams,
    cells: &mut [Vec<CellType>],
    needed: usize,
    rng: &mut Rng,
) -> Result<(), GenerationError> {
    if count_cell(cells, CellType::TYPE_0) >= needed {
        return Ok(());
    }
    let seeds = count_indices(cells, CellType::TYPE_0);
    grow_region(cells, &seeds, needed, CellType::TYPE_0, &[], rng);
    let plain = count_cell(cells, CellType::TYPE_0);
    if plain < needed {
        return Err(
            params.invalid(format!("map has {plain} plain cells but spots + agents need {needed}"))
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Grid helpers (row-major linear indices, even-row right-offset hex)
// ---------------------------------------------------------------------------

/// Set the cell at linear index `idx`.
fn set_cell(cells: &mut [Vec<CellType>], idx: usize, cell: CellType) {
    let edge = cells.len();
    if edge == 0 {
        return;
    }
    if let Some(row) = cells.get_mut(idx / edge) {
        if let Some(slot) = row.get_mut(idx % edge) {
            *slot = cell;
        }
    }
}

/// Read the cell at linear index `idx`.
fn get_cell(cells: &[Vec<CellType>], idx: usize) -> Option<CellType> {
    let edge = cells.len();
    if edge == 0 {
        return None;
    }
    cells.get(idx / edge).and_then(|row| row.get(idx % edge)).copied()
}

fn count_cell(cells: &[Vec<CellType>], want: CellType) -> usize {
    cells.iter().flatten().filter(|c| **c == want).count()
}

fn count_indices(cells: &[Vec<CellType>], want: CellType) -> Vec<usize> {
    cells.iter().flatten().enumerate().filter(|(_, c)| **c == want).map(|(i, _)| i).collect()
}

/// Linear indices of the carved cross (middle row + middle column).
fn cross_indices(edge: usize) -> Vec<usize> {
    let mid = edge / 2;
    let mut out = Vec::new();
    for r in 0..edge {
        for c in 0..edge {
            if r == mid || c == mid {
                out.push(r * edge + c);
            }
        }
    }
    out
}

/// Force the cross to plain terrain and return its indices.
fn carve_cross(cells: &mut [Vec<CellType>], edge: usize) -> Vec<usize> {
    let cross = cross_indices(edge);
    for &idx in &cross {
        set_cell(cells, idx, CellType::TYPE_0);
    }
    cross
}

/// In-bounds hex neighbours of a linear index (even-row right-offset layout,
/// Q&A Q1 — same rule as `engine::model::Direction::step`).
fn hex_neighbours(idx: usize, edge: usize) -> Vec<(usize, usize)> {
    let (r, c) = (idx / edge, idx % edge);
    let (Ok(row), Ok(col)) = (i32::try_from(r), i32::try_from(c)) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(6);
    for dir in Direction::ALL {
        let n = dir.step(HexCoord::new(col, row));
        if let (Ok(nr), Ok(nc)) = (usize::try_from(n.row), usize::try_from(n.col)) {
            if nr < edge && nc < edge {
                out.push((nr, nc));
            }
        }
    }
    out
}

/// Grow a region of `fill` cells outward from `seeds` (which must already be
/// `fill`) until it covers `target` cells, never converting `skip`.
///
/// Growth follows hex adjacency, so the grown region is connected — agents
/// walking the corridor really can reach everything planted on it.
fn grow_region(
    cells: &mut [Vec<CellType>],
    seeds: &[usize],
    target: usize,
    fill: CellType,
    skip: &[usize],
    rng: &mut Rng,
) {
    if seeds.is_empty() {
        return;
    }
    let edge = cells.len();
    let mut count = count_cell(cells, fill);
    if count >= target {
        return;
    }
    let mut frontier: Vec<usize> = seeds.to_vec();
    while count < target && !frontier.is_empty() {
        let pick = rng.inclusive(0, frontier.len() - 1);
        let idx = frontier.swap_remove(pick);
        for (nr, nc) in hex_neighbours(idx, edge) {
            let nidx = nr * edge + nc;
            if skip.contains(&nidx) || get_cell(cells, nidx) == Some(fill) {
                continue;
            }
            set_cell(cells, nidx, fill);
            frontier.push(nidx);
            count += 1;
            if count >= target {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use engine::model::MatchState;

    const SEEDS: [u64; 3] = [1, 7, 42];

    fn team() -> PlayerId {
        PlayerId::new("player-a").expect("valid team id")
    }

    fn generate_ok(params: &GenerationParams) -> CaseSpec {
        generate(params).unwrap_or_else(|e| {
            panic!("{} seed={} must generate: {e}", params.archetype.as_str(), params.seed)
        })
    }

    fn terrain_count(case: &CaseSpec, want: CellType) -> usize {
        case.setting.map.cells.iter().flatten().filter(|c| **c == want).count()
    }

    fn distinct_brands(case: &CaseSpec) -> usize {
        let mut brands = std::collections::BTreeSet::new();
        for spot in &case.setting.spots {
            brands.insert(spot.brand);
        }
        brands.len()
    }

    /// Size of the connected component of `want` cells containing `start`.
    fn component_size(cells: &[Vec<CellType>], start: usize, want: CellType) -> usize {
        let edge = cells.len();
        let mut seen = vec![false; edge * edge];
        let mut stack = vec![start];
        seen[start] = true;
        let mut size = 0;
        while let Some(idx) = stack.pop() {
            size += 1;
            for (nr, nc) in hex_neighbours(idx, edge) {
                let nidx = nr * edge + nc;
                if !seen[nidx] && get_cell(cells, nidx) == Some(want) {
                    seen[nidx] = true;
                    stack.push(nidx);
                }
            }
        }
        size
    }

    fn spot_positions(case: &CaseSpec) -> Vec<(usize, usize)> {
        let edge = usize::try_from(case.setting.map.width).expect("width fits usize");
        case.setting
            .spots
            .iter()
            .map(|s| {
                let pos = usize::try_from(s.pos).expect("position fits usize");
                (pos / edge, pos % edge)
            })
            .collect()
    }

    fn mean_pairwise_gap(positions: &[(usize, usize)]) -> f64 {
        let mut total = 0_u64;
        let mut pairs = 0_u64;
        for (i, a) in positions.iter().enumerate() {
            for b in positions.iter().skip(i + 1) {
                let gap = a.0.abs_diff(b.0).max(a.1.abs_diff(b.1));
                total += u64::try_from(gap).unwrap_or(0);
                pairs += 1;
            }
        }
        if pairs == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let total = total as f64;
        #[allow(clippy::cast_precision_loss)]
        let pairs = pairs as f64;
        total / pairs
    }

    // -- determinism --------------------------------------------------------

    #[test]
    fn same_params_and_seed_regenerate_byte_identical() {
        for archetype in Archetype::ALL {
            for seed in SEEDS {
                let first = generate_ok(&GenerationParams::new(archetype, seed));
                let second = generate_ok(&GenerationParams::new(archetype, seed));

                assert_eq!(first.case_id, second.case_id, "case_id must reproduce");
                assert_eq!(first.seed, Some(seed), "seed must be recorded");
                assert_eq!(
                    first.generation_parameters, second.generation_parameters,
                    "generation_parameters must reproduce"
                );
                let a = serde_json::to_string(&first.setting).expect("setting encodes");
                let b = serde_json::to_string(&second.setting).expect("setting encodes");
                assert_eq!(a, b, "{archetype} seed={seed} must regenerate byte-identically");
                // And the provenance slot is the generated one, not a fixture.
                assert!(matches!(first.source, crate::case::CaseSource::Generated));
            }
        }
    }

    #[test]
    fn generator_version_reaches_the_generation_parameters() {
        let case = generate_ok(&GenerationParams::new(Archetype::Open, 3));
        let params = case.generation_parameters.as_ref().expect("generated case carries params");
        assert_eq!(params["generator_version"], GENERATOR_VERSION);
        assert_eq!(params["archetype"], "open");
        assert_eq!(params["seed"], 3);
        assert_eq!(params["size"], "medium");
        assert_eq!(params["days"], 1);
        assert!(case.case_id.starts_with("generated:v1:"));
    }

    // -- identity -----------------------------------------------------------

    #[test]
    fn case_ids_are_traceable_to_version_archetype_and_seed() {
        let base = GenerationParams::new(Archetype::RoadHeavy, 7);
        assert_eq!(base.case_id(), "generated:v1:road-heavy:seed=7");

        let mut spaced = GenerationParams::new(Archetype::RoadHeavy, 7);
        spaced.size = SizeClass::Small;
        spaced.days = 1;
        assert_eq!(spaced.case_id(), "generated:v1:road-heavy:seed=7:size=small:days=1");

        // Defaults are implied, so an explicit-but-default spec collides with
        // the short form — the two really are the same case.
        let mut explicit = GenerationParams::new(Archetype::RoadHeavy, 7);
        explicit.size = SizeClass::Medium;
        explicit.days = 3;
        assert_eq!(explicit.case_id(), base.case_id());

        // Different seeds (and different overrides) are distinct ids. The
        // override is deliberately off-default for *every* archetype —
        // `dense-spots` already defaults to large/2 — so a collision here
        // would mean the id grammar cannot distinguish two real cases.
        let mut ids = std::collections::BTreeSet::new();
        for archetype in Archetype::ALL {
            for seed in [0, 1, 7] {
                ids.insert(GenerationParams::new(archetype, seed).case_id());
            }
            let mut other = GenerationParams::new(archetype, 1);
            other.size = SizeClass::Small;
            other.days = 3;
            ids.insert(other.case_id());
        }
        assert_eq!(ids.len(), Archetype::ALL.len() * 4, "no id may collide");
    }

    // -- validation ---------------------------------------------------------

    #[test]
    fn every_archetype_and_size_passes_the_real_validation_path() {
        for archetype in Archetype::ALL {
            for seed in SEEDS {
                for size in SizeClass::ALL {
                    let mut params = GenerationParams::new(archetype, seed);
                    params.size = size;
                    let case = generate_ok(&params);

                    // The exact path an evaluation run takes.
                    let engine_setting = case.to_engine_setting(&team()).expect("from_wire");
                    let _sim_config = case.sim_config().expect("Q&A Q13 thresholds");
                    engine_setting.validate().expect("engine setting validator");
                    MatchState::new(engine_setting).expect("MatchState::new — MatchSim's gate");
                    case.day_budget_us().expect("per-day budgets are coherent");
                }
            }
        }
    }

    #[test]
    fn day_structure_is_an_explicit_axis() {
        for archetype in Archetype::ALL {
            let params = GenerationParams::new(archetype, 3);
            let case = generate_ok(&params);
            let days = usize::try_from(params.days).expect("days fit");
            assert_eq!(case.setting.day_steps.len(), days, "{archetype:?}");
            assert_eq!(case.setting.day_seconds.len(), days, "{archetype:?}");

            let engine_setting = case.to_engine_setting(&team()).expect("from_wire");
            assert_eq!(engine_setting.num_days, params.days);
            assert_eq!(engine_setting.day_steps_by_day.len(), days);

            // Per-day steps inside the official W+H ..= 4(W+H) range, and
            // Q&A Q14's 1..=3x fuel multiplier (the generator picks 3x).
            let span = case.setting.map.width + case.setting.map.height;
            for steps in &case.setting.day_steps {
                assert!((span..=span * 4).contains(steps), "{archetype:?}: daySteps {steps}");
            }
            assert_eq!(case.setting.fuel_limits, case.setting.day_steps[0] * 3);

            // The completability invariant: the Q14 cap covers the whole
            // match's worst-case burn (fuel does not refill across days), so
            // no candidate can be fuel-short on a generated case by
            // construction. Plus multi-day cases must be front-loaded.
            let total: i64 = case.setting.day_steps.iter().map(|s| i64::from(*s)).sum();
            assert!(
                i64::from(case.setting.fuel_limits) >= 2 * total,
                "{archetype:?}: fuel {} cannot cover worst-case burn {}",
                case.setting.fuel_limits,
                2 * total
            );
            if days > 1 {
                assert!(
                    case.setting.day_steps[0] > case.setting.day_steps[1],
                    "{archetype:?}: multi-day cases must be front-loaded, got {:?}",
                    case.setting.day_steps
                );
            }

            // Q&A Q13 ranges, re-read from the generated body.
            assert!((1..=5).contains(&case.setting.busy_threshold));
            assert!(
                (case.setting.busy_threshold + 1..=10).contains(&case.setting.jammed_threshold),
                "{archetype:?}: jammed {} must exceed busy {}",
                case.setting.jammed_threshold,
                case.setting.busy_threshold
            );
        }

        // Until #187 no bundled fixture was multi-day; today only
        // `road-heavy-v2` is. The generated corpus must carry multi-day
        // coverage itself.
        assert!(Archetype::RoadHeavy.default_days() >= 2, "road-heavy drives multi-day traffic");
        assert!(Archetype::ManySeries.default_days() >= 2, "many-series drives tier 2");
        assert!(Archetype::PondObstacle.default_days() >= 2);
        assert!(Archetype::DenseSpots.default_days() >= 2);
        assert_eq!(MAX_GENERATED_DAYS, 3);
    }

    #[test]
    fn invalid_parameters_surface_a_generation_failure_naming_archetype_and_seed() {
        for days in [0, MAX_GENERATED_DAYS + 1] {
            let mut params = GenerationParams::new(Archetype::RoadHeavy, 7);
            params.days = days;
            let err = generate(&params).expect_err("out-of-range days must fail generation");
            let msg = err.to_string();
            assert!(msg.starts_with("case generation failure"), "{msg}");
            assert!(msg.contains("archetype=road-heavy"), "{msg}");
            assert!(msg.contains("seed=7"), "{msg}");
            assert!(msg.contains("days"), "{msg}");
        }

        let mut params = GenerationParams::new(Archetype::Open, 1);
        params.generator_version = GENERATOR_VERSION + 1;
        let err = generate(&params).expect_err("an unknown version must fail generation");
        let msg = err.to_string();
        assert!(msg.starts_with("case generation failure"), "{msg}");
        assert!(msg.contains("unsupported generator version"), "{msg}");
        assert!(msg.contains("seed=1"), "{msg}");
    }

    #[test]
    fn validator_rejects_broken_generated_output() {
        let params = GenerationParams::new(Archetype::RoadHeavy, 7);
        let good = generate_ok(&params);

        let assert_rejected = |broken: WireSetting, needle: &str| {
            let err = validate_setting(&params, &broken)
                .expect_err("broken output must be rejected at corpus-build time");
            let msg = err.to_string();
            assert!(msg.starts_with("case generation failure"), "{msg}");
            assert!(msg.contains("archetype=road-heavy") && msg.contains("seed=7"), "{msg}");
            assert!(msg.contains(needle), "expected {needle:?} in: {msg}");
        };

        // (a) daySeconds shorter than daySteps — the from_wire coherence rule.
        let mut broken = good.setting.clone();
        broken.day_seconds.pop();
        assert_rejected(broken, "daySteps has 3 entries but daySeconds has 2");

        // (b) a threshold outside Q&A Q13's range.
        let mut broken = good.setting.clone();
        broken.busy_threshold = 42;
        assert_rejected(broken, "Q&A Q13");

        // (c) a map outside the engine's MapSize bounds.
        let mut broken = good.setting.clone();
        broken.map.width = 7;
        assert_rejected(broken, "structural validation");

        // (d) daySteps outside the official W+H ..= 4(W+H) window.
        let mut broken = good.setting.clone();
        broken.day_steps[0] = 1;
        assert_rejected(broken, "daySteps[0]");
    }

    #[test]
    fn uniform_multi_day_with_a_q14_legal_fuel_cap_is_rejected() {
        // The exact trap: uniform days look right and pass the wire shape,
        // but fuel does not refill across days while the cap only scales with
        // daySteps[0] — every candidate would run the tank dry mid-match and
        // the whole corpus column would come back `invalid`.
        let params = GenerationParams::new(Archetype::ManySeries, 4);
        let good = generate_ok(&params);
        assert_eq!(good.setting.day_steps.len(), 3, "many-series defaults to 3 days");

        let mut broken = good.setting.clone();
        broken.day_steps = vec![40, 40, 40]; // inside the official [32, 128] window
        broken.fuel_limits = 120; // 3 x 40 — Q14-legal
        let err = validate_setting(&params, &broken)
            .expect_err("an undersupplied multi-day case must fail generation");
        let msg = err.to_string();
        assert!(msg.starts_with("case generation failure"), "{msg}");
        assert!(msg.contains("worst-case burn"), "{msg}");
        assert!(msg.contains("invalid-by-construction"), "{msg}");

        // And the cap itself is enforced in the other direction too: fuel
        // above 3 x daySteps[0] is simply not a legal setting.
        let mut over = good.setting.clone();
        over.fuel_limits = over.day_steps[0] * 3 + 1;
        let err = validate_setting(&params, &over).expect_err("over-cap fuel must fail");
        let msg = err.to_string();
        assert!(msg.contains("Q&A Q14"), "{msg}");
    }

    #[test]
    fn validator_rejects_an_agent_planted_on_a_pond() {
        let params = GenerationParams::new(Archetype::PondObstacle, 5);
        let good = generate_ok(&params);
        let pond = good
            .setting
            .map
            .cells
            .iter()
            .flatten()
            .enumerate()
            .find(|(_, c)| **c == CellType::TYPE_2)
            .map(|(i, _)| i)
            .expect("pond-obstacle has a pond");
        let pond = i32::try_from(pond).expect("index fits the wire field");
        assert!(
            !good.setting.agents.contains(&pond),
            "this pond must be free, or the test proves nothing"
        );

        let mut broken = good.setting.clone();
        broken.agents[0] = pond;
        let err = validate_setting(&params, &broken)
            .expect_err("an agent on impassable terrain must fail generation");
        let msg = err.to_string();
        assert!(msg.starts_with("case generation failure"), "{msg}");
        assert!(msg.contains("impassable terrain Pond"), "{msg}");
        assert!(msg.contains("archetype=pond-obstacle") && msg.contains("seed=5"), "{msg}");
    }

    // -- materiality --------------------------------------------------------

    #[test]
    fn open_is_plain_heavy_and_has_no_obstacles() {
        let case = generate_ok(&GenerationParams::new(Archetype::Open, 11));
        let total = case.setting.map.cells.len() * case.setting.map.cells[0].len();
        let plain = terrain_count(&case, CellType::TYPE_0);
        assert!(plain * 10 >= total * 9, "open must be >= 90% plain, got {plain}/{total}");
        assert_eq!(terrain_count(&case, CellType::TYPE_2), 0, "open has no ponds");
        assert_eq!(terrain_count(&case, CellType::TYPE_3), 0, "open has no roads");
    }

    #[test]
    fn mountain_heavy_is_half_mountains_with_a_usable_plain_spine() {
        for size in SizeClass::ALL {
            let mut params = GenerationParams::new(Archetype::MountainHeavy, 11);
            params.size = size;
            let case = generate_ok(&params);
            let total = case.setting.map.cells.len() * case.setting.map.cells[0].len();
            let mountains = terrain_count(&case, CellType::TYPE_1);
            assert!(
                mountains * 2 >= total,
                "{size:?}: mountain-heavy must be >= 50% mountain, got {mountains}/{total}"
            );
        }
    }

    #[test]
    fn pond_obstacle_is_a_real_obstacle_course() {
        let case = generate_ok(&GenerationParams::new(Archetype::PondObstacle, 11));
        let total = case.setting.map.cells.len() * case.setting.map.cells[0].len();
        let ponds = terrain_count(&case, CellType::TYPE_2);
        assert!(ponds * 10 >= total, "pond-obstacle must be >= 10% pond, got {ponds}/{total}");
    }

    #[test]
    fn road_heavy_has_a_connected_road_lattice() {
        for size in SizeClass::ALL {
            let mut params = GenerationParams::new(Archetype::RoadHeavy, 11);
            params.size = size;
            let case = generate_ok(&params);
            let edge = usize::try_from(case.setting.map.width).expect("width fits usize");
            let total = edge * edge;
            let roads = terrain_count(&case, CellType::TYPE_3);
            assert!(
                roads * 100 >= total * 15,
                "{size:?}: road-heavy must be >= 15% road, got {roads}/{total}"
            );

            // One dominant connected component — a lattice, not scattered
            // stubs — so routing really can choose road vs plain.
            let biggest = (0..edge * edge)
                .filter(|&i| get_cell(&case.setting.map.cells, i) == Some(CellType::TYPE_3))
                .map(|i| component_size(&case.setting.map.cells, i, CellType::TYPE_3))
                .max()
                .unwrap_or(0);
            assert!(
                biggest * 10 >= roads * 9,
                "{size:?}: road cells must form one lattice ({biggest} of {roads} connected)"
            );
        }

        // The use/avoid choice: road and plain cost different fuel
        // (`crates/engine/src/rules.rs::fuel_cost`), so crossing the lattice
        // is a decision, not a free ride.
        assert_eq!(engine::rules::fuel_cost(engine::model::Terrain::Plain), 1);
        assert_eq!(
            engine::rules::fuel_cost(engine::model::Terrain::Road(
                engine::model::TrafficState::Smooth
            )),
            2
        );
    }

    #[test]
    fn series_archetypes_have_their_brand_cardinality() {
        let many = generate_ok(&GenerationParams::new(Archetype::ManySeries, 11));
        let many_brands = distinct_brands(&many);
        assert!(many_brands >= 6, "many-series must carry >= 6 brands, got {many_brands}");

        let few = generate_ok(&GenerationParams::new(Archetype::FewSeries, 11));
        assert_eq!(distinct_brands(&few), 1, "few-series must carry exactly 1 brand");
    }

    #[test]
    fn sparse_and_dense_spots_differ_spatially() {
        let mut sparse_params = GenerationParams::new(Archetype::SparseSpots, 5);
        sparse_params.size = SizeClass::Small;
        let sparse = generate_ok(&sparse_params);
        assert!(
            sparse.setting.spots.len() <= 6,
            "sparse-spots caps at 6, got {}",
            sparse.setting.spots.len()
        );
        let edge = usize::try_from(sparse.setting.map.width).expect("width fits usize");
        let positions = spot_positions(&sparse);
        let gap = positions
            .iter()
            .flat_map(|a| positions.iter().map(move |b| (a, b)))
            .map(|(a, b)| a.0.abs_diff(b.0).max(a.1.abs_diff(b.1)))
            .filter(|d| *d > 0)
            .min()
            .unwrap_or(0);
        let required = (edge / 3).max(2);
        assert!(gap >= required, "sparse spots must keep a gap of >= {required} cells, got {gap}");

        let mut dense_params = GenerationParams::new(Archetype::DenseSpots, 5);
        dense_params.size = SizeClass::Small;
        let dense = generate_ok(&dense_params);
        assert!(
            dense.setting.spots.len() >= 18,
            "dense-spots starts at 18, got {}",
            dense.setting.spots.len()
        );
        let dense_gap = mean_pairwise_gap(&spot_positions(&dense));
        let sparse_gap = mean_pairwise_gap(&spot_positions(&sparse));
        assert!(
            dense_gap < sparse_gap,
            "dense spots must cluster tighter than sparse ones: {dense_gap} vs {sparse_gap}"
        );
    }

    // -- selection grammar --------------------------------------------------

    #[test]
    fn selection_grammar_parses_explicit_seeds_and_overrides() {
        let sel = parse_selection("open:seed=1,road-heavy:seed=7:size=small:days=3")
            .expect("valid selection parses");
        assert_eq!(sel.len(), 2);
        assert_eq!(sel[0], GenerationParams::new(Archetype::Open, 1));
        assert_eq!(sel[0].case_id(), "generated:v1:open:seed=1");
        assert_eq!(sel[1].archetype, Archetype::RoadHeavy);
        assert_eq!(sel[1].size, SizeClass::Small);
        // `days=3` is road-heavy's default, so the id stays short — the
        // default is implied by generator version + archetype.
        assert_eq!(sel[1].days, 3);
        assert_eq!(sel[1].case_id(), "generated:v1:road-heavy:seed=7:size=small");

        // An empty selection selects nothing (generated cases are opt-in).
        assert!(parse_selection("").expect("empty parses").is_empty());
        assert!(parse_selection("   ").expect("blank parses").is_empty());
    }

    #[test]
    fn selection_grammar_rejects_ambiguous_or_incomplete_items() {
        let bad = [
            "open",                  // no seed
            "bogus:seed=1",          // unknown archetype
            "open:size=huge:seed=1", // unknown size
            "open:seed=1:days=abc",  // unparseable days
            "open:seed=1:weight=2",  // unknown key
            "open:seed=1:seed=2",    // duplicate key
            "open:seed=1,",          // empty item
        ];
        for spec in bad {
            match parse_selection(spec) {
                Ok(sel) => panic!("{spec:?} must be rejected, got {sel:?}"),
                Err(err) => {
                    let msg = err.to_string();
                    assert!(msg.starts_with("case generation failure"), "{msg}");
                }
            }
        }
        // The seedless item is specifically the "required axis" complaint.
        let msg = parse_selection("open").expect_err("seed is required").to_string();
        assert!(msg.contains("seed= is required"), "{msg}");
    }

    #[test]
    fn corpus_build_fails_fast_and_stays_a_generation_failure() {
        let params = parse_selection("open:seed=1,open:seed=1").expect("parses");
        let err = build_corpus(&params).expect_err("duplicate ids must fail the build");
        let msg = err.to_string();
        assert!(msg.starts_with("case generation failure"), "{msg}");
        assert!(msg.contains("duplicate"), "{msg}");

        // A poisoned parameter stops the build before the first case exists —
        // there is no partial corpus that could be evaluated as `invalid`.
        let mut poisoned = GenerationParams::new(Archetype::RoadHeavy, 9);
        poisoned.days = 99;
        let err = build_corpus(&[GenerationParams::new(Archetype::Open, 1), poisoned])
            .expect_err("the poisoned item must fail the whole build");
        assert!(err.to_string().contains("days 99"), "{err}");
    }

    #[test]
    fn generated_cases_compose_with_the_bundled_corpus() {
        let fixtures = crate::fixture::MatchFixtureLoader::bundled_fixtures().len();
        let mut all = Corpus::bundled().expect("bundled loads").cases;
        let generated =
            build_corpus(&parse_selection("open:seed=1,road-heavy:seed=7").unwrap()[..])
                .expect("generated loads");
        all.extend(generated.cases);
        let merged = Corpus::new(all).expect("fixtures and generated ids never collide");
        assert_eq!(merged.len(), fixtures + 2);
        assert!(merged.case_ids().contains(&"generated:v1:open:seed=1"));
        assert!(merged.case_ids().contains(&"fixture:small-8x8"));
    }
}
