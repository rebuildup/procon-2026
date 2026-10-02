//! Domain types, invariants, and pure game logic.
//!
//! `domain` is the **canonical source-of-truth** for the procon-2026
//! contest rules. Every other crate (engine, protocol, solver-*, cli)
//! consumes the types and behaviours defined here.
//!
//! # Game overview
//!
//! procon-2026 participates in the 37th National Institute of Technology
//! Programming Contest (高専プロコン), competitive division:
//! **ヘキサうどん** (Hex Udon). Each match is a multi-day competition in
//! which teams field agents on a hex grid, move them step-by-step to
//! collect udon "balls" (玉) from spots scattered across the map, and
//! accumulate the highest score under the official four-tier ranking.
//!
//! The official spec sources referenced throughout this module are:
//!
//! - 募集要項 (rules) + 競技部門「ヘキサうどん」のフォーマットについて
//!   (format spec) — both under `docs/pdf/`
//! - Q&A 第 1 弾 / 第 2 弾 / 補足資料 (Q1..Q19)
//! - `OpenAPI` 仕様: `.reference/procon37/openapi.json`
//!
//! ## Game loop (per-day mechanics)
//!
//! Each match runs for a fixed number of days. Each day the following
//! sequence occurs (Q&A **Q6**):
//!
//! 1. **Action submission** — each team submits a per-agent action plan
//!    via `POST /`. The plan is a sequence of `day_steps` actions,
//!    each entry being either a wait (`-N`) or a move direction
//!    (`0..=5`).
//! 2. **Movement phase** — actions are replayed in step order. The
//!    server resolves each agent's intended move against the map.
//!    A single invalid move (off-map, impassable terrain, or
//!    insufficient fuel) rejects the **entire** batch (Q&A **Q6**
//!    all-or-nothing).
//! 3. **Spot interaction** — Patrol agents standing on a spot may
//!    pick up one ball of that spot's lineage per day. Supply agents
//!    are not eligible (玉採取不可).
//! 4. **Traffic update** — for every road cell, the number of agent
//!    traversals today is added to the running counter. End-of-day
//!    the counter promotes to "yesterday", today's count resets, and
//!    the cell's [`TrafficState`] is re-derived from the 2-day max.
//! 5. **Refuel** — Patrol agents sharing a cell with a Supply agent
//!    are refueled to the day's fuel cap (Q&A **Q19**, [`crate::refuel`]).
//! 6. **Score accumulation** — distinct udon lineages, per-day
//!    variety, total noodles, and final-response timestamps are
//!    aggregated for end-of-match ranking
//!    ([`crate::outcome::Score::from_daily`]).
//!
//! ## Agent movement (Q&A Q1, "移動と燃料")
//!
//! Agents traverse a hex grid of dimensions `W × H` where
//! `W, H ∈ [8, 32]`. The grid uses **even-row right-offset**
//! ([`MapLayout::EvenRowRightOffset`]) — odd `row` values sit half a
//! cell to the right of the even row above. From any cell, an agent
//! may step into one of **six** directions (clockwise from
//! upper-left):
//!
//! | wire | direction (ja) | variant | en |
//! |------|----------------|---------|----|
//! | 0 | 左上 | [`Direction::UpperLeft`]  | upper-left  |
//! | 1 | 右上 | [`Direction::UpperRight`] | upper-right |
//! | 2 | 右   | [`Direction::Right`]      | right       |
//! | 3 | 右下 | [`Direction::LowerRight`] | lower-right |
//! | 4 | 左下 | [`Direction::LowerLeft`]  | lower-left  |
//! | 5 | 左   | [`Direction::Left`]       | left        |
//!
//! Wait actions (wire value `< 0`) consume no fuel and do not move.
//! Each **move step** consumes fuel and move-steps according to the
//! destination terrain ([`crate::cell::Cell`]):
//!
//! | terrain        | move-steps | fuel/step |
//! |----------------|------------|-----------|
//! | Plain          | 2          | 1         |
//! | Mountain       | 3          | 2         |
//! | Pond           | impassable (sentinel) |
//! | Road (Smooth)  | 1          | 2         |
//! | Road (Busy)    | 2          | 2         |
//! | Road (Jammed)  | 4          | 2         |
//!
//! Roads are **state-dependent**: move-step cost varies with the
//! [`TrafficState`] (Q&A **Q11**: fuel cost is constant at 2 across
//! all three states). Pond cells are impassable; [`Action::validate`]
//! and [`move_to`] reject moves that land on them.
//!
//! ## Spot types
//!
//! Each spot is associated with a [`Series`] (うどん系列, "udon
//! lineage") and yields at most `max_stock` balls per day to a single
//! team. The rules (Q&A **Q15**, **Q18**, [`docs/contest/strategy.md`]
//! §2) require:
//!
//! - **1 cell ≤ 1 spot** (Q18): a spot occupies a separate coordinate
//!   from the terrain; [`crate::cell::Cell`] and
//!   [`crate::cell::Spot`] are kept as parallel types (see
//!   `docs/adr/0004-cell-spot-flat-enum.md`).
//! - **spots ≥ agents** (Q15): the team-wide upper bound on the
//!   distinct lineages a team can collect per day is bounded by the
//!   number of agents.
//! - **`max_stock` ∈ [1, `agent_count`]** (Q15): each spot yields at most
//!   one ball per agent on the team.
//! - **Per-day, per-agent, 1 ball per spot** (募集要項「スポット」節).
//!
//! There is **no separate "refill" / "supply" spot type** in the spec.
//! The team fields dedicated **Supply agents** ([`AgentKind::Supply`])
//! that act as mobile fuel stations: when a Patrol and a Supply share
//! a cell, the Patrol's fuel is reset to the day's cap
//! ([`crate::refuel::apply_refuel`]). Supply agents consume no fuel
//! and have no upper bound (募集要項 §エージェント, Q&A **Q14**).
//!
//! ## Scoring (公式判定, 戦略 §1)
//!
//! End-of-match ranking compares four priority tiers; higher wins,
//! earlier tier breaks ties:
//!
//! 1. **distinct udon lineages** (種類の異なり数) — [`outcome::Score::variety`]
//! 2. **cumulative per-day variety** (各日の種類数の累積) —
//    [`outcome::Score::daily_variety_sum`]
//! 3. **total balls harvested** (うどんの総玉数) —
//    [`outcome::Score::total_noodles`]
//! 4. **cumulative final-response timestamp** (各日の最終回答時刻の累積)
//    — [`outcome::Score::total_response_time_ms`] (smaller is better)
//!
//! Ties on all four tiers resolve to a draw (see
//! [`outcome::Outcome::Draw`]). The comparison function is
//! [`outcome::compare_scores`]; the convenience wrapper
//! [`outcome::judge`] returns the per-team [`outcome::Outcome`].
//!
//! ## Win condition
//!
//! The match ends **after the final day** declared in
//! [`GameModel::turn_count`]. At that point each team has accumulated
//! an [`outcome::Score`] via [`outcome::Score::from_daily`]. The team
//! with the strictly highest score (per the priority above) wins.
//! Draws occur when all four tiers compare equal. There is no early
//! termination condition: matches always run for the full
//! `turn_count` days regardless of intermediate scores.
//!
//! ## Invariants (MUST hold at all times)
//!
//! - **Determinism**: every `domain::*` function must be a pure
//!   function of its inputs — same input ⇒ same output, no I/O, no
//!   clock, no async.
//! - **Layering**: `domain` must not depend on `engine`, `protocol`,
//!   or any other higher-level crate. The dependency graph is
//!   `engine → domain ← protocol`.
//! - **Hex layout**: [`MapLayout`] is single-variant
//!   ([`MapLayout::EvenRowRightOffset`]); even rows sit at their
//!   nominal column (Q&A **Q1**).
//! - **No torus**: out-of-range coords do **not** wrap (Q&A **Q2**).
//! - **All-or-nothing**: a single invalid move in a day's batch
//!   rejects the **entire** batch (Q&A **Q6**).
//! - **Fuel policies**: Patrol fuel ≤ `day_steps * 3` (Q&A **Q14**);
//!   Supply has no upper bound ([`crate::agent::Supply`]).
//! - **Traffic thresholds**: `busy ∈ [1, 5]`, `jammed ∈ [2, 10]`,
//!   and `jammed > busy` (Q&A **Q13**, **Q19**).
//! - **Map size**: `8 ≤ W, H ≤ 32` (Q&A **Q2** / 募集要項 §マップ).
//! - **Agent count**: `3 ≤ agents ≤ 8` (募集要項 §エージェント).
//!
//! ### Known cross-crate translation points
//!
//! - The protocol layer exposes the match day as a **0-indexed**
//!   integer (初日 = 0) in JSON; the canonical [`crate::agent::Day`]
//!   here is **1-indexed** to match 募集要項 §試合進行. The
//!   conversion (`wire_day + 1 == Day`) lives in `protocol`.
//! - The per-map `day_steps` upper bound in the `OpenAPI` spec is
//!   `(W + H) * 4`; the domain-side constant
//!   [`crate::agent::DaySteps::MAX`] is `1024` (loose upper bound).
//!   The protocol layer applies the tighter per-map clamp.
//!
//! ## Static configuration
//!
//! The static configuration of a single match is captured by
//! [`GameModel`]. [`GameModel::validate`] enforces every spec-bound
//! invariant (map size, turn count, day-step / day-second ranges,
//! agent / spot counts, traffic thresholds, fuel cap) up-front so
//! downstream crates can rely on the values being inside their
//! published ranges.
//!
//! # Module layout
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`action`] | [`Direction`], [`ActionKind`], [`Action`], [`move_to`], batch validation (Q&A Q1, Q6). |
//! | [`agent`]  | [`Agent`], [`AgentKind`], [`FuelPolicy`], [`MatchState`] (Q&A Q6, Q14, Q19). |
//! | [`cell`]   | Terrain cells ([`crate::cell::Cell`]), [`Spot`], [`TrafficState`] (Q&A Q11, Q13, Q15, Q18). |
//! | [`coordinate`] | [`HexCoord`], [`MapSize`], [`MapLayout`], [`Bounds`] (Q&A Q1, Q2). |
//! | [`game_model`] | [`GameModel`] — static match configuration summary. |
//! | [`id`]     | Stable identifier types. |
//! | [`outcome`] | Per-team end-of-match comparison ([`outcome::Score`], [`outcome::judge`]). |
//! | [`refuel`] | Same-cell Patrol / Supply refuel logic (Q&A Q14, Q19). |
//! | [`state`]  | Aggregate [`GameState`] placeholder (legacy). |
//! | [`traffic`] | Per-cell traffic counters + 2-day sliding max (Q&A Q6, Q13). |
//!
//! See [`GameModel`] for the canonical static configuration type.
//! See `docs/architecture.md` and `docs/adr/` for design rationale.

#![doc(test(attr(deny(warnings))))]

pub mod action;
pub mod agent;
pub mod cell;
pub mod coordinate;
pub mod game_model;
pub mod id;
pub mod outcome;
pub mod refuel;
pub mod state;
pub mod traffic;

pub use action::{Action, ActionKind, Direction, move_to, neighbor, validate_batch};
pub use agent::{
    AGENT_COUNT_MAX, AGENT_COUNT_MIN, Agent, AgentId, AgentKind, Day, DaySeconds, DaySteps,
    FUEL_UPPER_BOUND_MULTIPLIER, FuelAmount, FuelPolicy, MatchState, Patrol, Position, Supply,
    Traffic,
};
pub use cell::{
    BUSY_THRESHOLD_DEFAULT, BUSY_THRESHOLD_MAX, BUSY_THRESHOLD_MIN, Cell, FUEL_COSTS,
    JAMMED_THRESHOLD_DEFAULT, JAMMED_THRESHOLD_MAX, JAMMED_THRESHOLD_MIN, MOVE_STEPS, Series, Spot,
    TrafficState, validate_traffic_thresholds,
};
pub use coordinate::{Bounds, HexCoord, MapLayout, MapSize};
pub use game_model::{GameModel, GameModelError};
pub use id::{MatchId, PlayerId, TurnId};
pub use state::{GameState, Outcome, Score, Turn};
pub use traffic::{Thresholds, TrafficCell, TrafficMap};

/// Library-wide error type. Pure-domain errors only; I/O errors belong to other crates.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// The supplied move or action is not legal in the given state.
    #[error("illegal move: {0}")]
    IllegalMove(String),
    /// An invariant in the supplied state was violated.
    #[error("invariant violation: {0}")]
    InvariantViolation(String),
}

/// Library-wide result alias.
pub type CoreResult<T> = Result<T, CoreError>;
