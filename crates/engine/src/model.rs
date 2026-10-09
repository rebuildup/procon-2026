//! Game model used by the local match simulator: [`MatchSetting`],
//! [`MatchState`], [`RunResult`], the per-step [`AgentActions`] plan, and
//! [`Solver`].
//!
//! # Why two solver contracts exist (and why that is intentional)
//!
//! | | [`Solver`] (this module) | [`crate::solver::Solver`] |
//! |---|---|---|
//! | Context | rich [`MatchState`] (map, agents, stock, traffic) | thin [`protocol::MatchSnapshot`] (match id, turn, scores) |
//! | Output | one day of [`AgentActions`] | one day of [`crate::solver::SolverDecision`] |
//! | Used by | [`crate::sim::MatchSim`] — offline evaluation | the coordinator loop over the live wire |
//!
//! The live wire does not publish the map or agent positions, so
//! [`crate::solver::Solver`]'s context cannot carry them; the offline
//! simulator has them all. They are two views for two transports, not two
//! competing designs. The offline evaluation path deliberately drives
//! [`Solver`] below, because that is what [`crate::sim::MatchSim`] consumes.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use domain::{MatchId, PlayerId};
use serde::{Deserialize, Serialize};

use crate::sim::SimConfig;

// ---------------------------------------------------------------------------
// Hex coordinate system (Q&A Q1 — even-row right-offset layout).
// ---------------------------------------------------------------------------

/// Hex coordinate.
///
/// Layout: even rows (`row % 2 == 0`) shift right by half a column relative
/// to odd rows. Bounds are enforced by [`MapSize`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HexCoord {
    pub col: i32,
    pub row: i32,
}

impl HexCoord {
    #[must_use]
    pub const fn new(col: i32, row: i32) -> Self {
        Self { col, row }
    }
}

// ---------------------------------------------------------------------------
// Map size & grid.
// ---------------------------------------------------------------------------

/// Map dimensions.
///
/// Spec: 募集要項「ヘキサうどん」 — `8 <= W,H <= 32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapSize {
    pub width: u8,
    pub height: u8,
}

impl MapSize {
    /// Minimum allowed dimension (Q&A + 募集要項).
    pub const MIN: u8 = 8;
    /// Maximum allowed dimension.
    pub const MAX: u8 = 32;

    /// Construct a [`MapSize`], validating the bounds.
    ///
    /// # Errors
    /// Returns `Err` if either dimension is out of `[Self::MIN, Self::MAX]`.
    pub fn new(width: u8, height: u8) -> Result<Self, String> {
        if !(Self::MIN..=Self::MAX).contains(&width) {
            return Err(format!("width {width} out of bounds [{}, {}]", Self::MIN, Self::MAX));
        }
        if !(Self::MIN..=Self::MAX).contains(&height) {
            return Err(format!("height {height} out of bounds [{}, {}]", Self::MIN, Self::MAX));
        }
        Ok(Self { width, height })
    }

    /// `true` iff `coord` falls inside this size.
    #[must_use]
    pub fn contains(&self, coord: HexCoord) -> bool {
        coord.col >= 0
            && coord.row >= 0
            && coord.col < i32::from(self.width)
            && coord.row < i32::from(self.height)
    }
}

/// Traffic state for a road cell.
///
/// Spec: 募集要項「道路セルと交通量」 + Q&A Q11 (step count) / Q13 (thresholds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrafficState {
    Smooth,
    Busy,
    Jammed,
}

impl TrafficState {
    /// Step cost in time units for one *traversal* (not one move action).
    ///
    /// Smooth = 1 step, Busy = 2 steps, Jammed = 4 steps (Q&A Q11).
    #[must_use]
    pub const fn step_cost(self) -> u32 {
        match self {
            Self::Smooth => 1,
            Self::Busy => 2,
            Self::Jammed => 4,
        }
    }
}

/// Terrain type of a cell.
///
/// `Road` carries the current traffic state. Spot-bearing plains are modelled
/// by [`Cell::spot`] being `Some`, not by a separate terrain variant, so the
/// acquisition logic stays a single code path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Terrain {
    Plain,
    Mountain,
    Pond,
    Road(TrafficState),
}

impl Terrain {
    /// Whether agents can enter this terrain with a `Move` action.
    #[must_use]
    pub const fn traversable(self) -> bool {
        !matches!(self, Self::Pond)
    }
}

/// Stockpile at a `Plain` cell.
///
/// Spec: Q&A Q18 — one cell can host at most one spot. `max_stock` is the
/// number of agents on the team (Q&A Q15 + 募集要項).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spot {
    /// Series id (1-based). Distinct series are the primary scoring key
    /// (募集要項「順位決定」).
    pub series: u32,
    pub pos: HexCoord,
    /// Maximum balls in stock (`1..=agent_count`, Q&A Q15).
    pub max_stock: u32,
    /// Current stock; refilled to `max_stock` at the start of each day.
    pub current_stock: u32,
}

impl Spot {
    /// Validate the spec invariants for a spot.
    ///
    /// # Errors
    /// Returns `Err` if `series == 0`, `max_stock == 0`, or stock exceeds the
    /// cap.
    pub fn validate(&self) -> Result<(), String> {
        if self.series == 0 {
            return Err("spot series must be >= 1".to_owned());
        }
        if self.max_stock == 0 {
            return Err("spot max_stock must be >= 1".to_owned());
        }
        if self.current_stock > self.max_stock {
            return Err("spot current_stock exceeds max_stock".to_owned());
        }
        Ok(())
    }
}

/// One map cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub coord: HexCoord,
    pub terrain: Terrain,
    pub spot: Option<Spot>,
}

impl Cell {
    /// Convenience: look up by coordinate via the owning [`MapGrid`].
    #[must_use]
    pub fn spot(&self) -> Option<&Spot> {
        self.spot.as_ref()
    }
}

/// Dense map representation. Cells are row-major, indexed as
/// `row * width + col`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapGrid {
    pub size: MapSize,
    cells: Vec<Cell>,
}

impl MapGrid {
    /// Build a grid from cells. Validates dimensions and bounds.
    ///
    /// # Errors
    /// Returns `Err` if the cell count does not match `size`, any cell is
    /// out of bounds, or any spot fails its own validation.
    pub fn new(size: MapSize, mut cells: Vec<Cell>) -> Result<Self, String> {
        let expected = usize::from(size.width) * usize::from(size.height);
        if cells.len() != expected {
            return Err(format!(
                "expected {expected} cells for size {size:?}, got {}",
                cells.len()
            ));
        }
        for cell in &cells {
            if !size.contains(cell.coord) {
                return Err(format!("cell {:?} outside map bounds", cell.coord));
            }
            if let Some(spot) = &cell.spot {
                spot.validate()?;
                if spot.pos != cell.coord {
                    return Err(format!(
                        "spot pos {:?} does not match cell {:?}",
                        spot.pos, cell.coord
                    ));
                }
            }
        }
        // Sort row-major by (row, col) for predictable indexing.
        cells.sort_by_key(|c| (c.coord.row, c.coord.col));
        // `get` computes its index from the coordinate (`row * width + col`),
        // so a duplicated coordinate would make some *other* cell land at
        // that index and every lookup past it silently return the wrong
        // cell. Reject duplicates instead of mis-mapping.
        if let Some(pair) = cells.windows(2).find(|w| w[0].coord == w[1].coord) {
            return Err(format!("duplicate cell coordinate {:?}", pair[0].coord));
        }
        Ok(Self { size, cells })
    }

    /// Look up a cell by coordinate.
    #[must_use]
    pub fn get(&self, coord: HexCoord) -> Option<&Cell> {
        if !self.size.contains(coord) {
            return None;
        }
        let idx = coord.row as usize * self.size.width as usize + coord.col as usize;
        self.cells.get(idx)
    }

    /// Mutable lookup.
    pub fn get_mut(&mut self, coord: HexCoord) -> Option<&mut Cell> {
        if !self.size.contains(coord) {
            return None;
        }
        let idx = coord.row as usize * self.size.width as usize + coord.col as usize;
        self.cells.get_mut(idx)
    }

    /// All cells, row-major.
    #[must_use]
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// Mutable cell slice, row-major. Exposed so transitions that refill
    /// spots in place do not have to round-trip through [`MapGrid::get_mut`].
    pub fn cells_mut(&mut self) -> &mut [Cell] {
        &mut self.cells
    }

    /// Iterate road cells (convenience for traffic bookkeeping).
    pub fn roads(&self) -> impl Iterator<Item = &Cell> {
        self.cells.iter().filter(|c| matches!(c.terrain, Terrain::Road(_)))
    }
}

// ---------------------------------------------------------------------------
// Agents.
// ---------------------------------------------------------------------------

/// Agent kind — determines the fuel rule.
///
/// Spec: 募集要項「エージェント」 — `Supply` has no fuel cap; `Patrol` is
/// bounded by `day_steps * multiplier`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentKind {
    Patrol,
    Supply,
}

/// One controllable agent.
///
/// `fuel_cap` is pre-computed at construction time:
/// - `Supply`: `u32::MAX` (effectively unlimited)
/// - `Patrol`: `day_steps * MULTIPLIER` (default MULTIPLIER = 3, Q&A Q14)
///
/// `inventory` is `Vec<(series, count)>` so multiple series can be carried
/// in a single day if the agent visits several spots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    /// Team-local id. Stable across days so `AgentActions` can reference it.
    pub id: u8,
    pub kind: AgentKind,
    pub pos: HexCoord,
    /// Current fuel. Decremented by movement; refilled on supply contact.
    pub fuel: u32,
    /// Maximum fuel. `u32::MAX` for `Supply` agents.
    pub fuel_cap: u32,
    /// Balls currently held, grouped by series.
    pub inventory: BTreeMap<u32, u32>,
}

impl Agent {
    /// Construct a patrol agent at `pos` with `day_steps * multiplier` fuel.
    #[must_use]
    pub fn patrol(id: u8, pos: HexCoord, day_steps: u32, multiplier: u32) -> Self {
        let cap = day_steps.saturating_mul(multiplier);
        Self {
            id,
            kind: AgentKind::Patrol,
            pos,
            fuel: cap,
            fuel_cap: cap,
            inventory: BTreeMap::new(),
        }
    }

    /// Construct a patrol agent with an explicit fuel cap.
    ///
    /// Used when the fuel budget is a match property (the wire's
    /// `fuelLimits`) rather than a multiple of the day's step count.
    #[must_use]
    pub fn patrol_with_fuel(id: u8, pos: HexCoord, fuel_cap: u32) -> Self {
        Self {
            id,
            kind: AgentKind::Patrol,
            pos,
            fuel: fuel_cap,
            fuel_cap,
            inventory: BTreeMap::new(),
        }
    }

    /// Construct a supply agent at `pos`. Fuel is uncapped.
    #[must_use]
    pub fn supply(id: u8, pos: HexCoord) -> Self {
        Self {
            id,
            kind: AgentKind::Supply,
            pos,
            fuel: u32::MAX,
            fuel_cap: u32::MAX,
            inventory: BTreeMap::new(),
        }
    }

    /// Number of balls currently held (sum over all series).
    #[must_use]
    pub fn total_inventory(&self) -> u32 {
        self.inventory.values().sum()
    }

    /// Whether the agent can pay a fuel cost of `cost` right now.
    #[must_use]
    pub fn can_afford(&self, cost: u32) -> bool {
        self.fuel >= cost
    }
}

// ---------------------------------------------------------------------------
// Actions.
// ---------------------------------------------------------------------------

/// Movement direction on the hex grid.
///
/// Order matches the official answer encoding: `0` upper-left, going
/// clockwise to `5` left. Kept identical so the protocol layer can map
/// positionally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    UpperLeft,
    UpperRight,
    Right,
    LowerRight,
    LowerLeft,
    Left,
}

impl Direction {
    /// Every direction, in wire-code order.
    pub const ALL: [Self; 6] = [
        Self::UpperLeft,
        Self::UpperRight,
        Self::Right,
        Self::LowerRight,
        Self::LowerLeft,
        Self::Left,
    ];

    /// Apply this direction to a hex coordinate, assuming even-row
    /// right-offset layout (Q&A Q1). Even rows are pushed right by half a
    /// column, so an even-row cell has more "right" upper/lower neighbours
    /// than "left" ones.
    #[must_use]
    pub fn step(self, from: HexCoord) -> HexCoord {
        let even_row = from.row.rem_euclid(2) == 0;
        let (d_col, d_row) = match self {
            // Even rows are shifted right; their upper-left / lower-left
            // neighbours sit at the same column.
            Self::UpperLeft => {
                if even_row {
                    (0, -1)
                } else {
                    (-1, -1)
                }
            }
            Self::UpperRight => {
                if even_row {
                    (1, -1)
                } else {
                    (0, -1)
                }
            }
            Self::Right => (1, 0),
            Self::LowerRight => {
                if even_row {
                    (1, 1)
                } else {
                    (0, 1)
                }
            }
            Self::LowerLeft => {
                if even_row {
                    (0, 1)
                } else {
                    (-1, 1)
                }
            }
            Self::Left => (-1, 0),
        };
        HexCoord::new(from.col + d_col, from.row + d_row)
    }
}

/// What a single agent does in a single step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionKind {
    /// Stay put. Always legal — the safe fallback.
    Wait,
    /// Move one cell in `direction`.
    Move(Direction),
}

/// One agent's action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub agent_id: u8,
    pub kind: ActionKind,
}

impl Action {
    #[must_use]
    pub const fn wait(agent_id: u8) -> Self {
        Self { agent_id, kind: ActionKind::Wait }
    }

    #[must_use]
    pub const fn step(agent_id: u8, direction: Direction) -> Self {
        Self { agent_id, kind: ActionKind::Move(direction) }
    }

    #[must_use]
    pub const fn is_wait(self) -> bool {
        matches!(self.kind, ActionKind::Wait)
    }
}

/// One team's answer for one day.
///
/// Layout: step-major. The first `agent_count` entries cover step 0 (one
/// action per controlled agent, in agent-id ascending order); the next
/// `agent_count` cover step 1; etc.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentActions {
    pub actions: Vec<Action>,
}

/// An action accepted at a Q6 action phase and not yet fully reflected.
///
/// Official Q6 (問い合わせと回答その1 + 補足資料): the day's answer is a
/// **sequential** action list — the next action is reserved only when the
/// previous one completes. A Move is applied after the *departure* terrain's
/// step count has elapsed (募集要項図4 / Q10), so between the order and the
/// landing the agent carries one of these in-flight records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingAction {
    /// A reserved move: lands at the reflection of `due_step`
    /// (`issued_step + terrain_step_cost(origin)`). `fuel_cost` is the
    /// departure terrain's fuel bill (Q25 — the pre-move terrain), charged
    /// in that same reflection (Q6: 燃料の消費 → 移動の反映).
    Move { to: HexCoord, due_step: u32, fuel_cost: u32 },
    /// A 1-tick wait: the agent is busy until `until_step` and the next
    /// queued action starts at that step's action phase.
    Wait { until_step: u32 },
}

impl PendingAction {
    /// The step at which this action completes (the reflection that applies
    /// it).
    #[must_use]
    pub const fn due_step(&self) -> u32 {
        match self {
            Self::Move { due_step, .. } | Self::Wait { until_step: due_step } => *due_step,
        }
    }
}

impl AgentActions {
    #[must_use]
    pub fn new(actions: Vec<Action>) -> Self {
        Self { actions }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// The always-legal fallback: every controlled agent waits every step.
    #[must_use]
    pub fn wait_all(agent_ids: &[u8], day_steps: u32) -> Self {
        let mut actions = Vec::with_capacity(agent_ids.len() * day_steps as usize);
        for _ in 0..day_steps {
            actions.extend(agent_ids.iter().copied().map(Action::wait));
        }
        Self { actions }
    }

    /// Cheap shape check: every step has exactly one action per agent, in
    /// ascending agent-id order.
    #[must_use]
    pub fn is_well_shaped(&self, agent_ids: &[u8], day_steps: u32) -> bool {
        let n = agent_ids.len();
        if self.actions.len() != n * day_steps as usize {
            return false;
        }
        self.actions
            .chunks(n.max(1))
            .all(|step| step.iter().map(|a| a.agent_id).eq(agent_ids.iter().copied()))
    }
}

// ---------------------------------------------------------------------------
// Setting, state, result.
// ---------------------------------------------------------------------------

/// Initial configuration for a single-team match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchSetting {
    pub match_id: MatchId,
    pub team: PlayerId,
    pub map: MapGrid,
    pub agents: Vec<Agent>,
    /// Steps per day for day 0 (`daySteps[0]`).
    ///
    /// Prefer [`MatchSetting::day_steps_for`], which honours the per-day
    /// vector. Kept as a field so callers that only ever build single-day-
    /// variation settings stay simple.
    pub day_steps: u32,
    /// Total days in the match (`daySteps.len()`).
    pub num_days: u32,
    /// Per-day step counts, indexed by day. Empty means "every day uses
    /// [`Self::day_steps`]".
    ///
    /// The official `daySteps` is a per-day vector, so a match whose day 1
    /// allows 24 steps and day 0 allows 16 must not be flattened to one
    /// value. Validated against `num_days` in [`MatchSetting::validate`].
    pub day_steps_by_day: Vec<u32>,
}

impl MatchSetting {
    /// Steps allowed on `day`.
    #[must_use]
    pub fn day_steps_for(&self, day: u32) -> u32 {
        match self.day_steps_by_day.get(day as usize) {
            Some(&n) => n,
            None => self.day_steps,
        }
    }
}

impl MatchSetting {
    /// Validate the spec invariants for the team composition.
    ///
    /// # Errors
    /// Returns `Err` if the agent count, fuel cap, or day parameters are
    /// invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.agents.len() < 3 || self.agents.len() > 8 {
            return Err(format!("team must have 3..=8 agents, got {}", self.agents.len()));
        }
        if self.day_steps == 0 {
            return Err("day_steps must be >= 1".to_owned());
        }
        if self.num_days == 0 {
            return Err("num_days must be >= 1".to_owned());
        }
        if !self.day_steps_by_day.is_empty() {
            if self.day_steps_by_day.len() != self.num_days as usize {
                return Err(format!(
                    "day_steps_by_day has {} entries but num_days is {}",
                    self.day_steps_by_day.len(),
                    self.num_days
                ));
            }
            if self.day_steps_by_day.contains(&0) {
                return Err("every day must allow at least one step".to_owned());
            }
            if self.day_steps_by_day[0] != self.day_steps {
                return Err(format!(
                    "day_steps ({}) must equal day_steps_by_day[0] ({})",
                    self.day_steps, self.day_steps_by_day[0]
                ));
            }
        }
        let mut ids = BTreeSet::new();
        for agent in &self.agents {
            if !ids.insert(agent.id) {
                return Err(format!("duplicate agent id {}", agent.id));
            }
            if !self.map.size.contains(agent.pos) {
                return Err(format!(
                    "agent {} starts outside map bounds at {:?}",
                    agent.id, agent.pos
                ));
            }
            let cell = self
                .map
                .get(agent.pos)
                .ok_or_else(|| format!("agent {} start cell missing", agent.id))?;
            if cell.spot.is_some() {
                return Err(format!(
                    "agent {} starts on a spot cell (spec: starts on plain)",
                    agent.id
                ));
            }
            if !cell.terrain.traversable() {
                return Err(format!(
                    "agent {} starts on impassable terrain {:?}",
                    agent.id, cell.terrain
                ));
            }
            match agent.kind {
                AgentKind::Patrol if agent.fuel_cap == 0 => {
                    return Err(format!("patrol agent {} has zero fuel cap", agent.id));
                }
                AgentKind::Supply => {}
                _ => {}
            }
        }
        Ok(())
    }

    /// Ascending agent-id list. Convenience for `AgentActions` shape checks.
    #[must_use]
    pub fn agent_ids(&self) -> Vec<u8> {
        let mut ids: Vec<u8> = self.agents.iter().map(|a| a.id).collect();
        ids.sort_unstable();
        ids
    }
}

/// Live state during a match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchState {
    pub setting: MatchSetting,
    /// Current day (0-based).
    pub day: u32,
    /// Current step inside the day (0-based; 0..`day_steps`).
    pub day_step: u32,
    pub agents: Vec<Agent>,
    /// Traffic count for road cells *this* day, accumulated as agents
    /// enter them. Reset at the start of each day.
    pub traffic_today: HashMap<HexCoord, u32>,
    /// Traffic count for the *previous* day. Used by the Q&A Q13 transition
    /// (today + yesterday → tomorrow's `TrafficState`).
    pub traffic_yesterday: HashMap<HexCoord, u32>,
    /// `(agent_id, coord) -> count` of balls picked at this coord today.
    /// Caps acquisition at 1 per agent per spot per day (募集要項).
    pub spot_picks_today: HashMap<(u8, HexCoord), u32>,
    /// Set of series ever acquired across all days. Drives the
    /// "variety" tie-breaker (募集要項「順位決定」).
    pub series_taken: BTreeSet<u32>,
    /// Per-series first-acquisition time `(day, step)`. Diagnostic evidence
    /// only — it is *not* a ranking tier (tier 4 is response time).
    pub series_first_taken_at: BTreeMap<u32, (u32, u32)>,
    /// Actions the simulator had to downgrade because they were not legal
    /// (out of bounds, impassable terrain, or out of fuel).
    ///
    /// This exists for the *evaluator*: the official answer rules reject an
    /// answer containing an invalid action outright (Q&A Q6), whereas
    /// [`crate::MatchSim`] deliberately degrades such an action to `Wait` so
    /// the match still completes. Without this counter an evaluator reading
    /// only the final score would happily rank a solver the server would
    /// have refused.
    ///
    /// Per-action downgrades only; see [`RunResult::illegal_action_count`]
    /// for how whole-plan shape violations are reported instead.
    pub illegal_action_count: u32,
    /// The subset of [`Self::illegal_action_count`] caused by running out of
    /// fuel rather than by a malformed destination. Tracked separately: it is
    /// a resource limit, not a malformed answer, and callers may want to
    /// report it without treating it as a contract violation.
    pub fuel_short_count: u32,
    /// Series acquired on each day, keyed by day. Drives
    /// `daily_variety_sum` = Σ_days |series acquired that day| (募集要項
    /// 「順位決定」tier 2, 「各日の種類数の累積」). A series re-acquired on a
    /// later day counts again for that later day.
    pub series_acquired_by_day: BTreeMap<u32, BTreeSet<u32>>,
    /// Per-agent queue of actions accepted at an action phase but not yet
    /// started (Q6: the daily answer is a sequential action list — the next
    /// action is reserved when the previous one completes).
    #[serde(default)]
    pub action_queues: BTreeMap<u8, VecDeque<Action>>,
    /// The action each agent is currently executing (accepted, not yet
    /// reflected). See [`PendingAction`].
    #[serde(default)]
    pub in_flight: BTreeMap<u8, PendingAction>,
    /// Set when the *current* day's response contains an action the official
    /// server would reject whole (Q6: 「回答全体をリジェクトします」 — any
    /// illegal / fuel-short / step-mismatched action). Flushed into
    /// [`Self::rejected_response_count`] at the day boundary.
    #[serde(default)]
    pub day_response_invalid: bool,
    /// Number of daily responses the official server would have rejected
    /// whole. The evaluator must treat a run with a non-zero count as a run
    /// the live server would not have scored (see [`RunResult::
    /// rejected_response_count`]).
    #[serde(default)]
    pub rejected_response_count: u32,
}

impl MatchState {
    /// Build the initial state from a validated setting. Spot stock is
    /// refilled to `max_stock`.
    ///
    /// # Errors
    /// Propagates setting validation errors.
    pub fn new(setting: MatchSetting) -> Result<Self, String> {
        setting.validate()?;
        let mut map = setting.map.clone();
        // Refill every spot to its max.
        for cell in map.cells.iter_mut() {
            if let Some(spot) = &mut cell.spot {
                spot.current_stock = spot.max_stock;
            }
        }
        Ok(Self {
            day: 0,
            day_step: 0,
            agents: setting.agents.clone(),
            traffic_today: HashMap::new(),
            traffic_yesterday: HashMap::new(),
            spot_picks_today: HashMap::new(),
            series_taken: BTreeSet::new(),
            series_first_taken_at: BTreeMap::new(),
            series_acquired_by_day: BTreeMap::new(),
            illegal_action_count: 0,
            fuel_short_count: 0,
            action_queues: BTreeMap::new(),
            in_flight: BTreeMap::new(),
            day_response_invalid: false,
            rejected_response_count: 0,
            setting: MatchSetting { map, ..setting.clone() },
        })
    }

    /// Steps allowed on the *current* day. Honours the per-day `daySteps`
    /// vector, so a setting with `daySteps = [16, 24]` really does give day 1
    /// twenty-four steps.
    #[must_use]
    pub fn day_steps(&self) -> u32 {
        self.setting.day_steps_for(self.day)
    }

    /// Convenience accessor.
    #[must_use]
    pub fn num_days(&self) -> u32 {
        self.setting.num_days
    }

    /// Convenience accessor.
    #[must_use]
    pub fn team(&self) -> &PlayerId {
        &self.setting.team
    }

    /// `true` once the match has finished.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.day >= self.num_days()
    }

    /// Distinct series acquired on each day, indexed by day, length
    /// `num_days`. Days with no acquisition contribute `0`.
    ///
    /// This is the per-day input to the official tier-2 metric
    /// (`daily_variety_sum` = the sum of this vector). See
    /// [`Self::series_acquired_by_day`].
    #[must_use]
    pub fn daily_varieties(&self) -> Vec<u32> {
        (0..self.num_days())
            .map(|d| self.series_acquired_by_day.get(&d).map_or(0, |series| series.len() as u32))
            .collect()
    }
}

/// Summary returned by [`super::MatchSim::run`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunResult {
    pub final_state: MatchState,
    /// Number of distinct series ever acquired.
    pub variety: u32,
    /// Sum over days of distinct series acquired on that day.
    pub daily_variety_sum: u32,
    /// Total balls acquired across the whole match.
    pub total_balls: u32,
    /// `(day, step)` of the *latest first-acquisition* — i.e. the last
    /// timestamp at which a previously-unseen series was first taken.
    ///
    /// Diagnostic evidence only. It is **not** a ranking tier: 募集要項
    /// 「順位決定」tier 4 is the cumulative per-day response time, which
    /// [`domain::Score::total_response_time_ms`] carries and the evaluator
    /// measures outside the simulator (the simulator has no clock).
    pub last_acquisition_at: Option<(u32, u32)>,
    /// Non-legal actions the simulator downgraded to `Wait`. Non-zero means
    /// the official rules would have rejected this answer, so the evaluator
    /// must not treat the run as a clean `success`.
    ///
    /// Scope: **per-action** downgrades only (out of bounds, impassable
    /// terrain, out of fuel). See [`Self::rejected_response_count`] for the
    /// official whole-response verdict (Q&A Q6 rejects the *entire* day's
    /// answer when any action is invalid / fuel-short / step-mismatched).
    pub illegal_action_count: u32,
    /// Subset of [`Self::illegal_action_count`] caused by fuel exhaustion.
    pub fuel_short_count: u32,
    /// Daily responses the official server would have rejected **whole**
    /// (Q&A Q6: 「回答全体をリジェクトします」). The simulator still
    /// demotes the offending actions to `Wait` so the match completes and a
    /// score can be produced, but a non-zero count means the live server
    /// would have refused the answer — the evaluator must surface that as an
    /// invalid run rather than ranking the demoted score.
    pub rejected_response_count: u32,
}

// ---------------------------------------------------------------------------
// Solver trait — the contract `MatchSim` consumes.
// ---------------------------------------------------------------------------

/// Solver contract for the offline simulation.
///
/// `decide` returns one day's worth of [`AgentActions`] and **nothing else**:
/// no score, no ranking. Scoring is [`crate::sim::RunResult`]'s job, derived
/// from the final [`MatchState`], which is what keeps the evaluation path
/// one-directional (see `docs/evaluation.md`).
///
/// Distinct from [`crate::solver::Solver`] (the wire-facing contract) — see
/// the module docs for the comparison.
pub trait Solver: Send + Sync {
    /// Stable, human-readable identifier. Used in logs and metadata.
    fn name(&self) -> &'static str;

    /// Plan one day for the calling team.
    ///
    /// Must be a pure function of `ctx`. Taking `&self` keeps it callable
    /// through `dyn Solver` from multiple threads.
    fn decide(&self, ctx: &SolverContext<'_>) -> AgentActions;
}

/// Everything a solver is allowed to look at when planning a day.
///
/// Local stand-in — the canonical [`super::super::solver::SolverContext`]
/// carries the same fields once #57 lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolverContext<'a> {
    pub state: &'a MatchState,
    pub team: &'a PlayerId,
    pub day: u32,
    pub day_steps: u32,
    pub agent_ids: Vec<u8>,
    /// Wall-clock budget left for answering this day, in milliseconds.
    ///
    /// The live bridge fills this from the coordinator's answer window
    /// (Q&A Q3: `daySeconds[day]`, measured against the server deadline);
    /// [`Self::from_state`] has no clock, so it reports `u32::MAX` and a
    /// time-capped search degenerates to its explicit `time_budget_ms`.
    pub remaining_ms: u32,
    /// Rules used by the simulator that is driving this solver. This lets
    /// offline planners replay future state with the same case-specific
    /// traffic thresholds as the evaluator.
    pub sim_config: SimConfig,
}

impl<'a> SolverContext<'a> {
    #[must_use]
    pub fn from_state(state: &'a MatchState) -> Self {
        Self::from_state_with_config(state, SimConfig::default())
    }

    #[must_use]
    pub fn from_state_with_config(state: &'a MatchState, sim_config: SimConfig) -> Self {
        let agent_ids = state.setting.agent_ids();
        Self {
            team: state.team(),
            state,
            day: state.day,
            day_steps: state.day_steps(),
            agent_ids,
            remaining_ms: u32::MAX,
            sim_config,
        }
    }
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// `MapGrid::get` derives its index from the coordinate, so a duplicated
    /// coordinate would silently shift every later lookup onto the wrong cell.
    #[test]
    fn duplicate_cell_coordinates_are_rejected() {
        let size = MapSize::new(8, 8).unwrap();
        let mut cells: Vec<Cell> = (0..8)
            .flat_map(|row| {
                (0..8).map(move |col| Cell {
                    coord: HexCoord::new(col, row),
                    terrain: Terrain::Plain,
                    spot: None,
                })
            })
            .collect();
        // Swap one coordinate for a repeat of its neighbour: the list is
        // still in bounds and still the right length, so only the uniqueness
        // check can catch it.
        cells[1].coord = cells[0].coord;
        let err = MapGrid::new(size, cells).expect_err("duplicate coordinate must be rejected");
        assert!(err.contains("duplicate cell coordinate"), "got: {err}");
    }
}
