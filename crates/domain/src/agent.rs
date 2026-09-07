//! Agent / match-progress types for procon-2026.
//!
//! This module introduces the per-day, per-agent building blocks of the match
//! state machine. The scope (Issue #13) is deliberately narrow: **type
//! definitions, invariants, and spec-trace documentation**. Behaviour such as
//! fuel consumption, path-finding, and re-planning is layered on top in the
//! `engine` crate.
//!
//! ## Specification trace
//!
//! | Type / constant | Source |
//! |---|---|
//! | [`AgentKind::Patrol`] / [`AgentKind::Supply`] | 募集要項 §エージェント |
//! | 3 ≤ `agent_count` ≤ 8 | 募集要項 §エージェント |
//! | [`FuelAmount`] upper bound = `daySteps * 3` | Q&A **Q14** (巡回車の燃料上限) |
//! | [`FuelPolicy`] for Supply has no limit | 募集要項 §エージェント (補給車) |
//! | Patrol agents carry fuel day-to-day | Q&A **Q19** (fuel 引き継ぎ) |
//! | 1 [`Day`] ≈ [`DaySteps`] move-steps | Q&A **Q6** (1 日の処理順序) |
//! | Initial positions are on plain cells with no spot | ADR-0004 (this PR) |
//!
//! ## Scope / non-scope
//!
//! - **In**: type definitions, validation entry points (`new`), spec-trace
//!   doc-comments, JSON round-trip tests, and the minimal invariant checks
//!   listed in the issue.
//! - **Out**: fuel consumption rules, spot interactions, traffic transition
//!   rules, action generation. Those arrive via separate engine-level issues.
//!
//! ## Type-level expression of the fuel policy
//!
//! "Supply has no fuel limit; Patrol has `daySteps * 3` upper bound" is
//! encoded as a sealed [`FuelPolicy`] trait implemented by two unit struct
//! markers, [`Patrol`] and [`Supply`]. This lets the compiler statically
//! verify which policies exist (via the sealed trait), and keeps the
//! per-kind validation logic in one place. See
//! [`Patrol::validate_fuel`] / [`Supply::validate_fuel`].
//!
//! ## Placeholders for #11 / #12
//!
//! The full hex coordinate / cell types are introduced in Issues #11
//! (`HexCoord`, `MapSize`, `MapLayout`, `Bounds`) and #12 (`Cell`, `Spot`,
//! `TrafficState`). Until those land on the integration line, [`Position`]
//! here is a self-contained `(col, row)` newtype and [`Traffic`] is a
//! minimal volume-per-cell container. When the integration PRs land, the
//! field types will be re-exported from the canonical modules without
//! changing the public surface of `agent`.

use serde::{Deserialize, Serialize};

use crate::{CoreError, CoreResult};

// =============================================================================
// Day / time newtypes
// =============================================================================

/// 1-indexed match day.
///
/// Days are 1-indexed because the contest rules describe the match as a
/// sequence of days starting from `Day 1` (募集要項 §試合進行). The newtype
/// rejects `0` so that "before the match" is representable only via
/// `Option<Day>` in the host crate.
///
/// Spec: 募集要項 §試合進行 / Q&A Q6 (1 日の処理順序).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Day(u32);

impl Day {
    /// The first legal day of any match.
    pub const FIRST: Self = Self(1);

    /// Minimum legal day index (inclusive).
    pub const MIN: u32 = 1;
    /// Maximum legal day index (inclusive). Set conservatively; matches
    /// shorter than this are well-defined.
    pub const MAX: u32 = 365;

    /// Construct a [`Day`] from a raw `u32`, validating
    /// `MIN <= value <= MAX`.
    pub fn new(value: u32) -> CoreResult<Self> {
        Self::validate(value)?;
        Ok(Self(value))
    }

    /// Pure boolean range check for [`Day::new`].
    pub fn validate(value: u32) -> CoreResult<()> {
        if !matches!(value, Self::MIN..=Self::MAX) {
            return Err(CoreError::InvariantViolation(format!(
                "day {value} out of range [{}, {}]",
                Self::MIN,
                Self::MAX
            )));
        }
        Ok(())
    }

    /// Inner `u32` value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Advance to the next day. Saturates at [`Day::MAX`].
    #[must_use]
    pub const fn next(self) -> Self {
        if self.0 >= Self::MAX { Self(Self::MAX) } else { Self(self.0 + 1) }
    }
}

impl Default for Day {
    fn default() -> Self {
        Self::FIRST
    }
}

/// Number of move-steps in a single day.
///
/// The contest server processes actions in **step order** within a day
/// (Q&A Q6). Each team submits a sequence of `DaySteps` actions per agent;
/// the upper bound is intentionally loose (matches are documented to fit
/// easily within `1..=1024` steps).
///
/// Spec: Q&A Q6 (処理順序) + 募集要項 §行動生成.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DaySteps(u32);

impl DaySteps {
    /// Inclusive lower bound. A day with zero steps would make the fuel
    /// upper bound (Q14) vacuous, which is not the intent.
    pub const MIN: u32 = 1;
    /// Inclusive upper bound. Loosely chosen; a 32×32 map with the highest
    /// per-cell move-step cost (4, jammed road) gives at most `64 * 4 = 256`
    /// steps end-to-end.
    pub const MAX: u32 = 1024;

    /// Construct a [`DaySteps`], validating the contest bounds.
    pub fn new(value: u32) -> CoreResult<Self> {
        Self::validate(value)?;
        Ok(Self(value))
    }

    /// Pure boolean range check for [`DaySteps::new`].
    pub fn validate(value: u32) -> CoreResult<()> {
        if !matches!(value, Self::MIN..=Self::MAX) {
            return Err(CoreError::InvariantViolation(format!(
                "day_steps {value} out of range [{}, {}]",
                Self::MIN,
                Self::MAX
            )));
        }
        Ok(())
    }

    /// Inner `u32` value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Wall-clock seconds available per day.
///
/// Each team must produce `DaySteps` actions per agent within
/// `DaySeconds` of wall-clock time. The bounds match the strategy notes:
/// typical matches use 60 s/day; the upper limit is loose to accommodate
/// future rule changes.
///
/// Spec: 募集要項 §行動生成 (リアルタイム要件) + 内部戦略メモ §5.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DaySeconds(u32);

impl DaySeconds {
    /// Inclusive lower bound.
    pub const MIN: u32 = 1;
    /// Inclusive upper bound (loose; one hour per day is far above any
    /// documented contest limit).
    pub const MAX: u32 = 3_600;

    /// Construct a [`DaySeconds`], validating the contest bounds.
    pub fn new(value: u32) -> CoreResult<Self> {
        Self::validate(value)?;
        Ok(Self(value))
    }

    /// Pure boolean range check for [`DaySeconds::new`].
    pub fn validate(value: u32) -> CoreResult<()> {
        if !matches!(value, Self::MIN..=Self::MAX) {
            return Err(CoreError::InvariantViolation(format!(
                "day_seconds {value} out of range [{}, {}]",
                Self::MIN,
                Self::MAX
            )));
        }
        Ok(())
    }

    /// Inner `u32` value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

// =============================================================================
// Fuel
// =============================================================================

/// Q14 upper-bound multiplier for [`Patrol`] fuel: `patrol_fuel ≤ daySteps * 3`.
///
/// The published contest answer to Q14 enumerates the upper bound as
/// `1..=3` times `daySteps`. The validation enforces the **maximum** of
/// that range (i.e. the contest-permitted upper bound); per-match
/// configuration is free to lower the cap in the engine layer.
///
/// Spec: Q&A Q14 (巡回車の燃料上限).
pub const FUEL_UPPER_BOUND_MULTIPLIER: u32 = 3;

/// Per-agent fuel quantity.
///
/// Patrol agents carry their fuel between days (Q&A Q19); Supply agents
/// consume none and carry fuel conceptually only to fit the uniform
/// payload shape.
///
/// Spec: Q&A Q11 (燃量 = 燃料消費量), Q14 (上限), Q19 (引き継ぎ).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FuelAmount(u32);

impl FuelAmount {
    /// The neutral zero value.
    pub const ZERO: Self = Self(0);

    /// Maximum representable fuel (matches `u32::MAX`). Real values are
    /// bounded by [`FUEL_UPPER_BOUND_MULTIPLIER`] * [`DaySteps::MAX`].
    pub const MAX: Self = Self(u32::MAX);

    /// Construct a [`FuelAmount`] from a raw `u32`. Validation against the
    /// per-kind upper bound lives on [`FuelPolicy`] and is invoked from
    /// [`Agent::new`].
    #[must_use]
    pub const fn new_unchecked(value: u32) -> Self {
        Self(value)
    }

    /// Inner `u32` value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl Default for FuelAmount {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Type-level fuel policy. Implemented by [`Patrol`] (bounded) and
/// [`Supply`] (unbounded).
///
/// The trait is `sealed` so the only two implementors are the kinds
/// defined in this module; this gives the same "two-variant policy" the
/// issue asks for without forcing every call site to pattern-match on an
/// enum. The dispatch in [`Agent::new`] uses the sealed markers
/// directly, so the policy choice is fully static.
///
/// Spec: 募集要項 §エージェント (補給車は燃料制限なし, 巡回車は上限あり).
pub trait FuelPolicy: Copy + Clone + sealed::Sealed {
    /// `true` iff the policy enforces an upper bound on fuel.
    const HAS_LIMIT: bool;

    /// Validate `fuel` against this policy and the current day's step
    /// count. Returns [`CoreError::InvariantViolation`] for over-cap
    /// Patrol agents; always succeeds for [`Supply`].
    fn validate_fuel(fuel: FuelAmount, day_steps: DaySteps) -> CoreResult<()>;
}

mod sealed {
    /// Seals [`super::FuelPolicy`] so no external crate can add new fuel
    /// policies. The two canonical kinds are [`super::Patrol`] and
    /// [`super::Supply`].
    pub trait Sealed {}
}

/// Marker for 巡回車 (Patrol). Patrol agents have a fuel upper bound of
/// `day_steps * FUEL_UPPER_BOUND_MULTIPLIER` (Q14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Patrol {
    _private: (),
}

impl Patrol {
    /// Construct a `Patrol` marker. Construction is infallible — the
    /// marker carries no payload, but is a nominal type for the trait
    /// machinery.
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for Patrol {
    fn default() -> Self {
        Self::new()
    }
}

/// Marker for 補給車 (Supply). Supply agents have no fuel limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Supply {
    _private: (),
}

impl Supply {
    /// Construct a `Supply` marker. Construction is infallible.
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for Supply {
    fn default() -> Self {
        Self::new()
    }
}

impl sealed::Sealed for Patrol {}
impl sealed::Sealed for Supply {}

impl FuelPolicy for Patrol {
    const HAS_LIMIT: bool = true;

    fn validate_fuel(fuel: FuelAmount, day_steps: DaySteps) -> CoreResult<()> {
        // saturating_mul protects against pathological inputs even though
        // DaySteps is itself bounded; the cap is still strictly enforced
        // for all values inside `DaySteps::MAX`.
        let max = day_steps.get().saturating_mul(FUEL_UPPER_BOUND_MULTIPLIER);
        if fuel.get() > max {
            return Err(CoreError::InvariantViolation(format!(
                "patrol fuel {} exceeds Q14 upper bound day_steps*{} = {}",
                fuel.get(),
                FUEL_UPPER_BOUND_MULTIPLIER,
                max
            )));
        }
        Ok(())
    }
}

impl FuelPolicy for Supply {
    const HAS_LIMIT: bool = false;

    fn validate_fuel(_fuel: FuelAmount, _day_steps: DaySteps) -> CoreResult<()> {
        // Supply agents are exempt from the Q14 cap. They may carry any
        // fuel amount (including 0) without violating the invariant.
        Ok(())
    }
}

// =============================================================================
// Agent
// =============================================================================

/// Identifies a single agent within a match.
///
/// The match-time uniqueness of an `AgentId` is enforced by
/// [`MatchState::new`]; the value itself is opaque.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(String);

impl AgentId {
    /// Construct a new `AgentId` from any string-like value. The empty
    /// string is rejected so that the wire format cannot carry a blank id.
    pub fn new(value: impl AsRef<str>) -> CoreResult<Self> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(CoreError::InvariantViolation("agent id must not be empty".to_string()));
        }
        Ok(Self(value.to_owned()))
    }

    /// Borrow the inner string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// Kinds of agent, as published in the contest rules.
///
/// Storage uses this enum (so a `Vec<Agent>` is homogeneous), while the
/// per-kind fuel policy is encoded statically via the [`FuelPolicy`]
/// trait — see [`Patrol`] / [`Supply`].
///
/// Spec: 募集要項 §エージェント.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentKind {
    /// 巡回車 (Patrol). Bounded fuel, can collect 玉 (balls) from spots.
    Patrol,
    /// 補給車 (Supply). Unbounded fuel, cannot collect 玉 but can refuel
    /// Patrol agents (behaviour layered on top in `engine`).
    Supply,
}

impl AgentKind {
    /// `true` iff this kind enforces the Q14 fuel upper bound.
    #[must_use]
    pub const fn has_fuel_limit(self) -> bool {
        match self {
            Self::Patrol => true,
            Self::Supply => false,
        }
    }

    /// Human-readable Japanese label (巡回車 / 補給車).
    #[must_use]
    pub const fn label_ja(self) -> &'static str {
        match self {
            Self::Patrol => "巡回車",
            Self::Supply => "補給車",
        }
    }
}

/// Minimal `(col, row)` position placeholder.
///
/// The full hex coordinate type (`HexCoord`, `MapSize`, `Bounds`,
/// `MapLayout`) is introduced in Issue #11. Until those types land on
/// the integration line, this newtype keeps the public API of `agent`
/// stable and gives the type system a distinct noun to point at.
///
/// Spec: 募集要項 §マップ + Q&A Q1 / Q2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Position {
    /// Column index (0-indexed, contest axis).
    pub col: u8,
    /// Row index (0-indexed, contest axis).
    pub row: u8,
}

impl Position {
    /// Construct a [`Position`] from raw `(col, row)` without bounds
    /// checking. Range checks against a [`MapSize`] will be added once
    /// `#11` lands; in the meantime [`MatchState::new`] validates the
    /// row/col bounds against `MAX_COL` / `MAX_ROW`.
    #[must_use]
    pub const fn new(col: u8, row: u8) -> Self {
        Self { col, row }
    }

    /// Inclusive upper bound for `col` (matches the largest map width
    /// documented in 募集要項 §マップ).
    pub const MAX_COL: u8 = 31;
    /// Inclusive upper bound for `row` (matches the largest map height
    /// documented in 募集要項 §マップ).
    pub const MAX_ROW: u8 = 31;
}

impl Default for Position {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

/// A single agent on the board.
///
/// Spec: 募集要項 §エージェント — each team fields 3-8 agents (validated
/// in [`MatchState::new`]) and may freely choose the Patrol / Supply
/// kind mix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    /// Stable identifier; unique per match.
    pub id: AgentId,
    /// Patrol vs Supply.
    pub kind: AgentKind,
    /// Current board position.
    pub pos: Position,
    /// Current fuel quantity. Validation against the per-kind policy is
    /// applied in [`Agent::new`] using the `day_steps` parameter passed at
    /// construction time.
    pub fuel: FuelAmount,
}

/// Inclusive lower bound for the number of agents in a match (募集要項
/// §エージェント).
pub const AGENT_COUNT_MIN: usize = 3;
/// Inclusive upper bound for the number of agents in a match (募集要項
/// §エージェント).
pub const AGENT_COUNT_MAX: usize = 8;

impl Agent {
    /// Construct a new [`Agent`], validating the fuel amount against
    /// the per-kind policy at the given `day_steps`.
    ///
    /// Spec: Q&A Q14 + Q&A Q19 — Patrol fuel is bounded by
    /// `day_steps * 3`; Supply has no cap and carries fuel without
    /// consumption.
    pub fn new(
        id: AgentId,
        kind: AgentKind,
        pos: Position,
        fuel: FuelAmount,
        day_steps: DaySteps,
    ) -> CoreResult<Self> {
        match kind {
            AgentKind::Patrol => Patrol::validate_fuel(fuel, day_steps)?,
            AgentKind::Supply => Supply::validate_fuel(fuel, day_steps)?,
        }
        Ok(Self { id, kind, pos, fuel })
    }

    /// Returns `true` iff this agent is a Patrol.
    #[must_use]
    pub fn is_patrol(&self) -> bool {
        matches!(self.kind, AgentKind::Patrol)
    }

    /// Returns `true` iff this agent is a Supply.
    #[must_use]
    pub fn is_supply(&self) -> bool {
        matches!(self.kind, AgentKind::Supply)
    }
}

// =============================================================================
// Traffic (placeholder for #12)
// =============================================================================

/// Per-cell traffic volume on the match's road network.
///
/// The full per-cell traffic / jam-state model is introduced in Issue #12
/// (`Cell`, `TrafficState`). This placeholder stores the raw `volume` per
/// cell, kept sorted by [`Position`] so the JSON output is deterministic
/// across runs. We use a `Vec<(Position, u32)>` rather than a map because
/// JSON object keys must be strings and `Position` is a struct; an array
/// of pairs round-trips losslessly without losing the `Eq` /
/// determinism guarantees.
///
/// Spec: 募集要項 表1 「道路セルと交通量」 + Q&A Q13 (混雑閾値 1-5, 渋滞
/// 閾値 2-10).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Traffic {
    /// Volume per cell, kept sorted by `(pos.row, pos.col)` for
    /// deterministic iteration / JSON output.
    volumes: Vec<(Position, u32)>,
}

impl Traffic {
    /// Inclusive upper bound for a single cell's traffic volume. The
    /// contest server publishes no explicit ceiling; the bound is set
    /// generously above the Q&A Q13 jammed threshold (max 10) plus a
    /// safety margin so that any plausible run is representable.
    pub const MAX_VOLUME: u32 = u16::MAX as u32;

    /// Construct an empty [`Traffic`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the volume at `pos`, or `0` if no entry is recorded.
    #[must_use]
    pub fn get(&self, pos: Position) -> u32 {
        self.volumes.iter().find(|(p, _)| *p == pos).map_or(0, |(_, v)| *v)
    }

    /// Set the volume at `pos`. Returns [`CoreError::InvariantViolation`]
    /// when `volume` exceeds [`Traffic::MAX_VOLUME`]. The internal
    /// storage stays sorted by `(row, col)` so iteration is deterministic.
    pub fn set(&mut self, pos: Position, volume: u32) -> CoreResult<()> {
        if volume > Self::MAX_VOLUME {
            return Err(CoreError::InvariantViolation(format!(
                "traffic volume {volume} at {pos:?} exceeds MAX_VOLUME {}",
                Self::MAX_VOLUME
            )));
        }
        if let Some(entry) = self.volumes.iter_mut().find(|(p, _)| *p == pos) {
            entry.1 = volume;
        } else {
            self.volumes.push((pos, volume));
            self.volumes.sort_by_key(|(p, _)| (p.row, p.col));
        }
        Ok(())
    }

    /// Iterate over `(pos, volume)` pairs in deterministic order.
    pub fn iter(&self) -> impl Iterator<Item = (Position, u32)> + '_ {
        self.volumes.iter().copied()
    }

    /// Number of cells with a recorded volume entry.
    #[must_use]
    pub fn len(&self) -> usize {
        self.volumes.len()
    }

    /// Whether no volume entries are recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.volumes.is_empty()
    }
}

// =============================================================================
// MatchState
// =============================================================================

/// Snapshot of a match at the end of one day.
///
/// Spec: 募集要項 §試合進行 + Q&A Q6 (処理順序). Behaviour such as
/// transitioning between days lives in the `engine` crate; this type
/// only carries the data and its invariants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchState {
    /// Current day index (1-based).
    pub day: Day,
    /// Step budget for the **next** day; Patrol fuel cap is derived from
    /// this field (Q14).
    pub day_steps: DaySteps,
    /// Wall-clock seconds available per day.
    pub day_seconds: DaySeconds,
    /// All agents currently on the board. Must satisfy
    /// `AGENT_COUNT_MIN <= len <= AGENT_COUNT_MAX` and have unique ids.
    pub agents: Vec<Agent>,
    /// Current traffic volumes per cell.
    pub traffic: Traffic,
}

impl MatchState {
    /// Construct a [`MatchState`], enforcing:
    ///
    /// - `day` is a legal index ([`Day::MIN`]..=[`Day::MAX`]);
    /// - `day_steps` is in [`DaySteps::MIN`]..=[`DaySteps::MAX`];
    /// - `day_seconds` is in [`DaySeconds::MIN`]..=[`DaySeconds::MAX`];
    /// - `agents.len()` lies in `AGENT_COUNT_MIN..=AGENT_COUNT_MAX`;
    /// - all `AgentId`s are unique;
    /// - every agent's `pos.col` is in `0..=Position::MAX_COL` and
    ///   `pos.row` is in `0..=Position::MAX_ROW`.
    /// - every Patrol agent's `fuel` is at most `day_steps * 3` (Q14);
    /// - every Supply agent has no fuel-cap violation (always succeeds).
    pub fn new(
        day: Day,
        day_steps: DaySteps,
        day_seconds: DaySeconds,
        mut agents: Vec<Agent>,
        traffic: Traffic,
    ) -> CoreResult<Self> {
        // -- Cardinality --
        let count = agents.len();
        if !(AGENT_COUNT_MIN..=AGENT_COUNT_MAX).contains(&count) {
            return Err(CoreError::InvariantViolation(format!(
                "agent count {count} out of range [{AGENT_COUNT_MIN}, {AGENT_COUNT_MAX}]"
            )));
        }

        // -- Unique ids --
        let mut seen_ids: Vec<&AgentId> = Vec::with_capacity(count);
        for agent in &agents {
            if seen_ids.contains(&&agent.id) {
                return Err(CoreError::InvariantViolation(format!(
                    "duplicate agent id {} in match state",
                    agent.id
                )));
            }
            seen_ids.push(&agent.id);
        }

        // -- Per-agent positions & fuel --
        for agent in &mut agents {
            if agent.pos.col > Position::MAX_COL || agent.pos.row > Position::MAX_ROW {
                return Err(CoreError::InvariantViolation(format!(
                    "agent {} position {:?} out of effective map bounds [0..={}, 0..={}]",
                    agent.id,
                    agent.pos,
                    Position::MAX_COL,
                    Position::MAX_ROW
                )));
            }
            // Re-validate fuel against the supplied day_steps so that
            // downstream mutations cannot smuggle in over-cap values.
            match agent.kind {
                AgentKind::Patrol => Patrol::validate_fuel(agent.fuel, day_steps)?,
                AgentKind::Supply => Supply::validate_fuel(agent.fuel, day_steps)?,
            }
        }

        Ok(Self { day, day_steps, day_seconds, agents, traffic })
    }

    /// Number of Patrol agents in this state.
    #[must_use]
    pub fn patrol_count(&self) -> usize {
        self.agents.iter().filter(|a| a.is_patrol()).count()
    }

    /// Number of Supply agents in this state.
    #[must_use]
    pub fn supply_count(&self) -> usize {
        self.agents.iter().filter(|a| a.is_supply()).count()
    }

    /// Look up an agent by id.
    #[must_use]
    pub fn agent(&self, id: &AgentId) -> Option<&Agent> {
        self.agents.iter().find(|a| &a.id == id)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------- Day --------

    #[test]
    fn day_first_is_one() {
        assert_eq!(Day::FIRST.get(), 1);
        assert_eq!(Day::default(), Day::FIRST);
    }

    #[test]
    fn day_new_accepts_boundaries() {
        assert!(Day::new(Day::MIN).is_ok());
        assert!(Day::new(Day::MAX).is_ok());
    }

    #[test]
    fn day_new_rejects_zero() {
        let err = Day::new(0).expect_err("0 is below MIN");
        assert!(matches!(err, CoreError::InvariantViolation(_)));
    }

    #[test]
    fn day_next_increments_and_saturates() {
        assert_eq!(Day::FIRST.next().get(), 2);
        assert_eq!(Day::new(Day::MAX).unwrap().next().get(), Day::MAX);
    }

    #[test]
    fn day_serde_round_trip() {
        let d = Day::new(7).unwrap();
        let s = serde_json::to_string(&d).unwrap();
        assert_eq!(s, "7");
        let back: Day = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
    }

    // -------- DaySteps / DaySeconds --------

    #[test]
    fn day_steps_accepts_min_and_rejects_zero() {
        assert!(DaySteps::new(DaySteps::MIN).is_ok());
        assert!(DaySteps::new(0).is_err());
        assert!(DaySteps::new(DaySteps::MAX).is_ok());
        assert!(DaySteps::new(DaySteps::MAX + 1).is_err());
    }

    #[test]
    fn day_seconds_accepts_min_and_rejects_zero() {
        assert!(DaySeconds::new(DaySeconds::MIN).is_ok());
        assert!(DaySeconds::new(0).is_err());
        assert!(DaySeconds::new(DaySeconds::MAX).is_ok());
        assert!(DaySeconds::new(DaySeconds::MAX + 1).is_err());
    }

    #[test]
    fn day_steps_serde_round_trip() {
        let v = DaySteps::new(200).unwrap();
        let s = serde_json::to_string(&v).unwrap();
        let back: DaySteps = serde_json::from_str(&s).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn day_seconds_serde_round_trip() {
        let v = DaySeconds::new(60).unwrap();
        let s = serde_json::to_string(&v).unwrap();
        let back: DaySeconds = serde_json::from_str(&s).unwrap();
        assert_eq!(v, back);
    }

    // -------- FuelAmount --------

    #[test]
    fn fuel_amount_zero_and_default() {
        assert_eq!(FuelAmount::ZERO.get(), 0);
        assert_eq!(FuelAmount::default().get(), 0);
    }

    #[test]
    fn fuel_amount_serde_round_trip() {
        let v = FuelAmount::new_unchecked(42);
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(s, "42");
        let back: FuelAmount = serde_json::from_str(&s).unwrap();
        assert_eq!(v, back);
    }

    // -------- FuelPolicy --------

    #[test]
    fn patrol_policy_rejects_fuel_above_q14_upper_bound() {
        // Q14: fuel <= day_steps * 3.
        let steps = DaySteps::new(100).unwrap();
        let cap_plus_one = FuelAmount::new_unchecked(steps.get() * 3 + 1);
        let err = Patrol::validate_fuel(cap_plus_one, steps).expect_err("over cap");
        assert!(matches!(err, CoreError::InvariantViolation(_)));

        // exactly at the cap must succeed.
        let cap = FuelAmount::new_unchecked(steps.get() * 3);
        assert!(Patrol::validate_fuel(cap, steps).is_ok());
    }

    #[test]
    fn patrol_policy_accepts_zero_fuel() {
        // A Patrol agent at day start can be at 0 fuel (e.g. after running
        // out); the policy is an upper bound, not a positivity requirement.
        let steps = DaySteps::new(10).unwrap();
        assert!(Patrol::validate_fuel(FuelAmount::ZERO, steps).is_ok());
    }

    #[test]
    fn supply_policy_never_rejects() {
        let steps = DaySteps::new(100).unwrap();
        // Even arbitrary fuel amounts are accepted by Supply.
        let huge = FuelAmount::new_unchecked(u32::MAX);
        assert!(Supply::validate_fuel(huge, steps).is_ok());
        assert!(Supply::validate_fuel(FuelAmount::ZERO, steps).is_ok());
    }

    #[test]
    fn fuel_policy_has_limit_is_static() {
        // Type-level: HAS_LIMIT differs between the two markers.
        const { assert!(Patrol::HAS_LIMIT) };
        const { assert!(!Supply::HAS_LIMIT) };
    }

    #[test]
    fn fuel_policy_is_sealed() {
        // Compile-time seal check: the trait can only be implemented
        // for the two markers defined in this module. This test simply
        // exercises the public surface; the seal is enforced by the
        // `mod sealed` block.
        fn accepts<T: FuelPolicy>() {}
        accepts::<Patrol>();
        accepts::<Supply>();
    }

    // -------- AgentKind --------

    #[test]
    fn agent_kind_labels() {
        assert_eq!(AgentKind::Patrol.label_ja(), "巡回車");
        assert_eq!(AgentKind::Supply.label_ja(), "補給車");
        assert!(AgentKind::Patrol.has_fuel_limit());
        assert!(!AgentKind::Supply.has_fuel_limit());
    }

    #[test]
    fn agent_kind_serde_round_trip() {
        for k in [AgentKind::Patrol, AgentKind::Supply] {
            let s = serde_json::to_string(&k).unwrap();
            let back: AgentKind = serde_json::from_str(&s).unwrap();
            assert_eq!(k, back);
        }
    }

    // -------- Agent --------

    fn sample_steps() -> DaySteps {
        DaySteps::new(100).unwrap()
    }

    fn patrol_agent(fuel: u32) -> Agent {
        patrol_agent_with_id("p0", fuel)
    }

    fn patrol_agent_with_id(id: &str, fuel: u32) -> Agent {
        Agent::new(
            AgentId::new(id).unwrap(),
            AgentKind::Patrol,
            Position::new(5, 5),
            FuelAmount::new_unchecked(fuel),
            sample_steps(),
        )
        .expect("agent should validate")
    }

    fn supply_agent(fuel: u32) -> Agent {
        supply_agent_with_id("s0", fuel)
    }

    fn supply_agent_with_id(id: &str, fuel: u32) -> Agent {
        Agent::new(
            AgentId::new(id).unwrap(),
            AgentKind::Supply,
            Position::new(0, 0),
            FuelAmount::new_unchecked(fuel),
            sample_steps(),
        )
        .expect("agent should validate")
    }

    #[test]
    fn agent_new_accepts_patrol_within_q14_cap() {
        // Exactly at the cap.
        let a = patrol_agent(100 * 3);
        assert!(a.is_patrol());
        assert!(!a.is_supply());
    }

    #[test]
    fn agent_new_rejects_patrol_over_q14_cap() {
        let err = Agent::new(
            AgentId::new("p1").unwrap(),
            AgentKind::Patrol,
            Position::new(0, 0),
            FuelAmount::new_unchecked(100 * 3 + 1),
            sample_steps(),
        )
        .expect_err("patrol over cap must fail");
        assert!(matches!(err, CoreError::InvariantViolation(_)));
    }

    #[test]
    fn agent_new_accepts_supply_with_arbitrary_fuel() {
        let a = supply_agent(u32::MAX);
        assert!(a.is_supply());
        assert!(!a.is_patrol());
    }

    #[test]
    fn agent_id_rejects_empty() {
        assert!(AgentId::new("").is_err());
        assert!(AgentId::new("ok").is_ok());
    }

    #[test]
    fn agent_serde_round_trip() {
        let a = patrol_agent(50);
        let s = serde_json::to_string(&a).unwrap();
        let back: Agent = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }

    // -------- Position --------

    #[test]
    fn position_default_is_origin() {
        assert_eq!(Position::default(), Position::new(0, 0));
    }

    #[test]
    fn position_serde_round_trip() {
        let p = Position::new(3, 7);
        let s = serde_json::to_string(&p).unwrap();
        let back: Position = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }

    // -------- Traffic --------

    #[test]
    fn traffic_get_returns_zero_for_missing_entry() {
        let t = Traffic::new();
        assert_eq!(t.get(Position::new(0, 0)), 0);
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn traffic_set_stores_and_iterates() {
        let mut t = Traffic::new();
        t.set(Position::new(1, 2), 7).unwrap();
        t.set(Position::new(0, 0), 0).unwrap();
        assert_eq!(t.get(Position::new(1, 2)), 7);
        assert_eq!(t.len(), 2);
        let pairs: Vec<(Position, u32)> = t.iter().collect();
        // Sorted by (row, col): (0,0) before (1,2).
        assert_eq!(pairs, vec![(Position::new(0, 0), 0), (Position::new(1, 2), 7)]);
    }

    #[test]
    fn traffic_set_rejects_over_max_volume() {
        let mut t = Traffic::new();
        let err = t.set(Position::new(0, 0), Traffic::MAX_VOLUME + 1).unwrap_err();
        assert!(matches!(err, CoreError::InvariantViolation(_)));
    }

    #[test]
    fn traffic_serde_round_trip() {
        let mut t = Traffic::new();
        t.set(Position::new(2, 4), 9).unwrap();
        let s = serde_json::to_string(&t).unwrap();
        let back: Traffic = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }

    // -------- MatchState --------

    fn sample_match_state(agents: Vec<Agent>) -> MatchState {
        MatchState::new(
            Day::FIRST,
            sample_steps(),
            DaySeconds::new(60).unwrap(),
            agents,
            Traffic::new(),
        )
        .expect("sample state should validate")
    }

    fn n_patrols(n: usize) -> Vec<Agent> {
        (0..n)
            .map(|i| {
                Agent::new(
                    AgentId::new(format!("p{i}")).unwrap(),
                    AgentKind::Patrol,
                    Position::new(0, 0),
                    FuelAmount::new_unchecked(0),
                    sample_steps(),
                )
                .unwrap()
            })
            .collect()
    }

    // -- Agent count boundaries --

    #[test]
    fn match_state_accepts_min_agent_count() {
        let agents = n_patrols(AGENT_COUNT_MIN);
        let s = sample_match_state(agents);
        assert_eq!(s.agents.len(), AGENT_COUNT_MIN);
        assert_eq!(s.patrol_count(), AGENT_COUNT_MIN);
        assert_eq!(s.supply_count(), 0);
    }

    #[test]
    fn match_state_accepts_max_agent_count() {
        let agents = n_patrols(AGENT_COUNT_MAX);
        let s = sample_match_state(agents);
        assert_eq!(s.agents.len(), AGENT_COUNT_MAX);
    }

    #[test]
    fn match_state_rejects_below_min_agents() {
        let agents = n_patrols(AGENT_COUNT_MIN - 1);
        let err = MatchState::new(
            Day::FIRST,
            sample_steps(),
            DaySeconds::new(60).unwrap(),
            agents,
            Traffic::new(),
        )
        .expect_err("2 agents is below the 3-agent minimum");
        assert!(matches!(err, CoreError::InvariantViolation(_)));
    }

    #[test]
    fn match_state_rejects_above_max_agents() {
        let agents = n_patrols(AGENT_COUNT_MAX + 1);
        let err = MatchState::new(
            Day::FIRST,
            sample_steps(),
            DaySeconds::new(60).unwrap(),
            agents,
            Traffic::new(),
        )
        .expect_err("9 agents is above the 8-agent maximum");
        assert!(matches!(err, CoreError::InvariantViolation(_)));
    }

    #[test]
    fn match_state_rejects_duplicate_agent_ids() {
        let mut agents = n_patrols(3);
        agents[2].id = agents[0].id.clone();
        let err = MatchState::new(
            Day::FIRST,
            sample_steps(),
            DaySeconds::new(60).unwrap(),
            agents,
            Traffic::new(),
        )
        .expect_err("duplicate ids must fail");
        assert!(matches!(err, CoreError::InvariantViolation(_)));
    }

    #[test]
    fn match_state_rejects_position_out_of_effective_bounds() {
        let mut agents = n_patrols(3);
        agents[0].pos = Position::new(Position::MAX_COL + 1, 0);
        let err = MatchState::new(
            Day::FIRST,
            sample_steps(),
            DaySeconds::new(60).unwrap(),
            agents,
            Traffic::new(),
        )
        .expect_err("position past MAX_COL must fail");
        assert!(matches!(err, CoreError::InvariantViolation(_)));
    }

    #[test]
    fn match_state_rejects_patrol_over_q14_cap() {
        // Build a Patrol with a legal `fuel` for `day_steps = 100`,
        // then construct the state with `day_steps = 1` so the same fuel
        // is suddenly over the cap. The `MatchState::new` re-validation
        // is what catches this.
        let agents = vec![patrol_agent(50)]; // 50 <= 1*3 ? no -> rejected
        let err = MatchState::new(
            Day::FIRST,
            DaySteps::new(1).unwrap(),
            DaySeconds::new(60).unwrap(),
            agents,
            Traffic::new(),
        )
        .expect_err("patrol fuel 50 violates cap at day_steps=1");
        assert!(matches!(err, CoreError::InvariantViolation(_)));
    }

    #[test]
    fn match_state_patrol_supply_mix_counted_correctly() {
        let agents = vec![
            patrol_agent(10),
            supply_agent_with_id("s1", 0),
            supply_agent_with_id("s2", 0),
            patrol_agent_with_id("p3", 20),
        ];
        let s = sample_match_state(agents);
        assert_eq!(s.patrol_count(), 2);
        assert_eq!(s.supply_count(), 2);
    }

    #[test]
    fn match_state_agent_lookup_by_id() {
        let agents = vec![
            patrol_agent_with_id("p0", 10),
            supply_agent_with_id("s0", 0),
            patrol_agent_with_id("p1", 20),
        ];
        let s = sample_match_state(agents);
        assert!(s.agent(&AgentId::new("p0").unwrap()).is_some());
        assert!(s.agent(&AgentId::new("missing").unwrap()).is_none());
    }

    #[test]
    fn match_state_serde_round_trip() {
        let agents = vec![
            patrol_agent_with_id("p0", 30),
            supply_agent_with_id("s0", 0),
            patrol_agent_with_id("p1", 40),
        ];
        let s = sample_match_state(agents);
        let json = serde_json::to_string(&s).unwrap();
        let back: MatchState = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    // -------- Patrol vs Supply kind semantics --------

    #[test]
    fn supply_kind_carries_fuel_without_cap_check() {
        // Supply: same kind, same position, but u32::MAX fuel is OK.
        let a = supply_agent(u32::MAX);
        assert_eq!(a.fuel.get(), u32::MAX);
    }

    #[test]
    fn patrol_kind_fuel_is_bounded_by_day_steps_times_three() {
        let steps = sample_steps(); // 100
        // day_steps * 3 = 300
        assert_eq!(patrol_agent(300).fuel.get(), 300);
        let err = Agent::new(
            AgentId::new("p_bad").unwrap(),
            AgentKind::Patrol,
            Position::new(0, 0),
            FuelAmount::new_unchecked(301),
            steps,
        )
        .expect_err("301 must exceed cap of 300");
        assert!(matches!(err, CoreError::InvariantViolation(_)));
    }
}
