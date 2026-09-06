//! 補給ロジック — 同一セル時に満タン化（domain / pure logic）。
//!
//! ## 仕様 trace
//!
//! | 規則 | 出典 |
//! |---|---|
//! | 同一セル (patrol + supply) で Patrol の燃料を上限まで回復 | 募集要項「移動と燃料」「補給」 |
//! | Patrol の燃料上限 = `day_steps * 3` | Q&A **Q14** |
//! | Supply は燃料制限なし・消費なし → 補給対象外 | 募集要項 §エージェント |
//! | 補給は 1 日 1 回実施（呼び出しは engine 層が 1 日 1 回） | Q&A **Q19** (fuel 引き継ぎ文脈) |
//! | 同位置・同行動 / 先行後追い / 複数ペア の 3 パターン | docs/contest/strategy.md §3.3 |
//!
//! ## 公開 API
//!
//! - [`apply_refuel`] — 補給ロジックを 1 回適用する純粋関数。
//! - [`RefuelState`] — 補給ロジックが要求する `MatchState` の最小ビュー。
//! - [`RefuelAgent`] — `Agent` の最小ビュー（id / kind / pos / fuel のみ）。
//! - [`RefuelReport`] — 適用結果（どの Patrol が補給されたか）。
//!
//! ## Issue #13 マージ後の置換
//!
//! `RefuelState` / `RefuelAgent` は `MatchState` / `Agent` の **最小ビュー**。
//! Issue #13 が統合されたら canonical 型への薄い newtype に置換するか、
//! canonical 型を `From` で変換して受け取る adaptor を追加する。
//! このモジュールは pure-domain logic のみを保つ（I/O / async なし）。

use serde::{Deserialize, Serialize};

// =============================================================================
// Local stubs (Issue #11 / #13 マージ後に canonical 型へ置換予定)
// =============================================================================

/// Patrol / Supply の種別。Issue #13 待ち。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentKind {
    /// 巡回車 — 燃料上限あり、玉採取可。
    Patrol,
    /// 補給車 — 燃料上限なし、消費なし、補給担当。
    Supply,
}

impl AgentKind {
    /// `true` iff this kind enforces the Q14 fuel upper bound.
    #[must_use]
    pub const fn has_fuel_limit(self) -> bool {
        matches!(self, Self::Patrol)
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

/// `(col, row)` position stub. Issue #11 (`HexCoord` / `MapSize`) 待ち。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Position {
    pub col: u8,
    pub row: u8,
}

impl Position {
    /// Construct a [`Position`] from `(col, row)` without bounds checking.
    /// Range checks against a `MapSize` will live on `MatchState::new`
    /// once #11 lands.
    #[must_use]
    pub const fn new(col: u8, row: u8) -> Self {
        Self { col, row }
    }
}

impl Default for Position {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

/// Per-agent fuel quantity stub. Issue #13 待ち。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FuelAmount(pub u32);

impl FuelAmount {
    /// The neutral zero value.
    pub const ZERO: Self = Self(0);
}

impl Default for FuelAmount {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Match day stub. Issue #13 待ち。1-indexed を満たすことの検証は呼び出し側の責務。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Day(pub u32);

/// Agent identifier stub. Issue #13 待ち。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(pub String);

// =============================================================================
// 公開型
// =============================================================================

/// 補給ロジックが要求する `Agent` の最小ビュー。
/// Issue #13 マージ後は `crate::agent::Agent` への thin adapter に置換予定。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefuelAgent {
    pub id: AgentId,
    pub kind: AgentKind,
    pub pos: Position,
    pub fuel: FuelAmount,
}

/// 補給ロジックが要求する `MatchState` の最小ビュー。
/// Issue #13 マージ後は `crate::agent::MatchState` への thin adapter に置換予定。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefuelState {
    /// 1 日のステップ数。Patrol 燃料上限 (`day_steps * 3`) の算出元。
    pub day_steps: u32,
    /// 全エージェント（順序は呼び出し側が保持、副作用なし）。
    pub agents: Vec<RefuelAgent>,
    /// 当日中に補給を受けた Patrol の累計回数。Q19 (1 日 1 回チェック) の基礎値。
    /// 1 日終了時に engine 層がリセットする想定。
    pub refueled_today: u32,
}

/// 補給ロジック 1 回適用の結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RefuelReport {
    /// 今回新たに補給を受けた Patrol の id 一覧（補給されなかった agent は含まない）。
    pub refueled: Vec<AgentId>,
}

impl RefuelReport {
    /// Number of Patrol agents newly refueled in this call.
    #[must_use]
    pub fn refueled_count(&self) -> usize {
        self.refueled.len()
    }
}

// =============================================================================
// ロジック
// =============================================================================

/// Q14: 巡回車の燃料上限乗数 (`day_steps * FUEL_UPPER_BOUND_MULTIPLIER`)。
pub const FUEL_UPPER_BOUND_MULTIPLIER: u32 = 3;

/// 指定 kind の燃料上限を返す。
/// - Patrol: `day_steps * FUEL_UPPER_BOUND_MULTIPLIER` (Q14)
/// - Supply: 上限なし (`u32::MAX`)
#[must_use]
pub fn fuel_upper_bound(kind: AgentKind, day_steps: u32) -> u32 {
    match kind {
        AgentKind::Patrol => day_steps.saturating_mul(FUEL_UPPER_BOUND_MULTIPLIER),
        AgentKind::Supply => u32::MAX,
    }
}

/// 補給ロジックを 1 回適用する。
///
/// ## 仕様
///
/// - 各 Patrol について、自分と同一セル (`pos`) に Supply がいる時、
///   Patrol の燃料を当日上限 (`day_steps * 3`) まで回復する。
/// - Supply は補給対象外（消費ゼロ・上限なしのため、玉も採取しない）。
/// - Patrol がすでに上限以上の場合はそのまま（cap で頭打ち）。
/// - `RefuelState::refueled_today` に今回の補給回数を加算する（Q19 1 日 1 回上限の基礎値）。
///
/// ## Q19: 1 日 1 回
///
/// この関数は「1 日 1 回呼ばれる」想定。複数回呼ぶと `refueled_today` が累積するが、
/// 同じ Patrol の燃料は上限以上に押し上げられない（`>= cap` で no-op）。
/// 1 日あたりの厳格な上限（例: 「1 Patrol 1 日 1 回まで」）は engine 層の責務。
/// 本モジュールは「燃料を上限まで戻すロジック」と「補給回数の集計」のみを提供する。
pub fn apply_refuel(state: &mut RefuelState, _day: Day) -> RefuelReport {
    let cap = fuel_upper_bound(AgentKind::Patrol, state.day_steps);

    // 同セルにいる Supply の位置集合。Supply 同士が同セルでも問題ない（重複エントリ無視）。
    let supply_positions: Vec<Position> = state
        .agents
        .iter()
        .filter(|a| matches!(a.kind, AgentKind::Supply))
        .map(|a| a.pos)
        .collect();

    let mut refueled = Vec::new();
    for agent in &mut state.agents {
        if !matches!(agent.kind, AgentKind::Patrol) {
            continue;
        }
        if !supply_positions.contains(&agent.pos) {
            continue;
        }
        if agent.fuel.0 >= cap {
            // 上限到達: 補給不要（no-op、燃料はそのまま）。
            continue;
        }
        agent.fuel = FuelAmount(cap);
        refueled.push(agent.id.clone());
    }

    // saturating_add ではなく try_from で厳密変換。
    // AGENT_COUNT_MAX <= 8 (Issue #13 待ち) なので truncation は発生しない想定だが、
    // pedantic lint に従い明示的に扱う。
    let added = u32::try_from(refueled.len()).unwrap_or(u32::MAX);
    state.refueled_today = state.refueled_today.saturating_add(added);
    RefuelReport { refueled }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------- Test helpers --------

    fn patrol(id: &str, pos: Position, fuel: u32) -> RefuelAgent {
        RefuelAgent {
            id: AgentId(id.to_string()),
            kind: AgentKind::Patrol,
            pos,
            fuel: FuelAmount(fuel),
        }
    }

    fn supply(id: &str, pos: Position) -> RefuelAgent {
        RefuelAgent {
            id: AgentId(id.to_string()),
            kind: AgentKind::Supply,
            pos,
            fuel: FuelAmount(0),
        }
    }

    fn state_with(agents: Vec<RefuelAgent>, day_steps: u32) -> RefuelState {
        RefuelState { day_steps, agents, refueled_today: 0 }
    }

    fn patrol_fuel(state: &RefuelState, id: &str) -> u32 {
        state
            .agents
            .iter()
            .find(|a| a.id.0 == id)
            .map_or_else(|| panic!("no patrol {id}"), |a| a.fuel.0)
    }

    // -------- Type smoke --------

    #[test]
    fn fuel_upper_bound_patrol_uses_q14_multiplier() {
        assert_eq!(fuel_upper_bound(AgentKind::Patrol, 100), 300);
        assert_eq!(fuel_upper_bound(AgentKind::Patrol, 1), 3);
        assert_eq!(fuel_upper_bound(AgentKind::Patrol, 0), 0);
    }

    #[test]
    fn fuel_upper_bound_supply_is_unbounded() {
        assert_eq!(fuel_upper_bound(AgentKind::Supply, 100), u32::MAX);
    }

    #[test]
    fn agent_kind_metadata_matches_branch_13() {
        assert!(AgentKind::Patrol.has_fuel_limit());
        assert!(!AgentKind::Supply.has_fuel_limit());
        assert_eq!(AgentKind::Patrol.label_ja(), "巡回車");
        assert_eq!(AgentKind::Supply.label_ja(), "補給車");
    }

    #[test]
    fn position_default_is_origin() {
        assert_eq!(Position::default(), Position::new(0, 0));
    }

    // -------- Acceptance: same cell → refuel --------

    #[test]
    fn same_cell_refuels_patrol_to_upper_bound() {
        let mut s = state_with(
            vec![patrol("p0", Position::new(3, 4), 50), supply("s0", Position::new(3, 4))],
            100,
        );
        let cap = fuel_upper_bound(AgentKind::Patrol, s.day_steps);
        assert_eq!(cap, 300);

        let report = apply_refuel(&mut s, Day(1));

        assert_eq!(patrol_fuel(&s, "p0"), cap);
        assert_eq!(report.refueled_count(), 1);
        assert_eq!(report.refueled, vec![AgentId("p0".to_string())]);
        assert_eq!(s.refueled_today, 1);
    }

    // -------- Acceptance: different cells → no refuel --------

    #[test]
    fn different_cells_do_not_refuel() {
        let mut s = state_with(
            vec![patrol("p0", Position::new(0, 0), 50), supply("s0", Position::new(5, 5))],
            100,
        );
        let report = apply_refuel(&mut s, Day(1));

        assert_eq!(patrol_fuel(&s, "p0"), 50, "patrol fuel must be unchanged");
        assert_eq!(report.refueled_count(), 0);
        assert!(report.refueled.is_empty());
        assert_eq!(s.refueled_today, 0);
    }

    // -------- Acceptance: already at upper bound → no change --------

    #[test]
    fn patrol_already_full_no_change_but_count_tracks() {
        let cap = 100 * 3;
        let mut s = state_with(
            vec![patrol("p0", Position::new(0, 0), cap), supply("s0", Position::new(0, 0))],
            100,
        );
        // Pre-seed refueled_today to verify it does not get falsely bumped.
        s.refueled_today = 7;

        let report = apply_refuel(&mut s, Day(2));

        assert_eq!(patrol_fuel(&s, "p0"), cap, "patrol fuel must be unchanged at cap");
        assert_eq!(report.refueled_count(), 0);
        assert!(report.refueled.is_empty());
        // No actual refuel occurred → counter must not change (cap reached = no event).
        assert_eq!(s.refueled_today, 7);
    }

    // -------- Acceptance: multiple patrols + 1 supply --------

    #[test]
    fn multiple_patrols_one_supply_all_get_refueled() {
        let mut s = state_with(
            vec![
                patrol("p0", Position::new(2, 3), 10),
                patrol("p1", Position::new(2, 3), 20),
                patrol("p2", Position::new(2, 3), 0),
                supply("s0", Position::new(2, 3)),
            ],
            50,
        );
        let cap = fuel_upper_bound(AgentKind::Patrol, s.day_steps);
        assert_eq!(cap, 150);

        let report = apply_refuel(&mut s, Day(1));

        assert_eq!(patrol_fuel(&s, "p0"), cap);
        assert_eq!(patrol_fuel(&s, "p1"), cap);
        assert_eq!(patrol_fuel(&s, "p2"), cap);
        assert_eq!(report.refueled_count(), 3);
        assert_eq!(s.refueled_today, 3);
    }

    // -------- Supply never refueled --------

    #[test]
    fn supply_with_other_supply_does_not_refuel() {
        // Two Supply at the same cell — neither is a Patrol, so no refuel.
        let mut s = state_with(
            vec![supply("s0", Position::new(0, 0)), supply("s1", Position::new(0, 0))],
            100,
        );
        let report = apply_refuel(&mut s, Day(1));
        assert_eq!(report.refueled_count(), 0);
        assert_eq!(s.refueled_today, 0);
        // Supply "fuel" field is left untouched (no-op).
        assert_eq!(s.agents[0].fuel.0, 0);
        assert_eq!(s.agents[1].fuel.0, 0);
    }

    #[test]
    fn supply_at_patrol_cell_does_not_get_refueled() {
        // Even if a Patrol is also there, the Supply is never a refuel target.
        let mut s = state_with(
            vec![patrol("p0", Position::new(0, 0), 0), supply("s0", Position::new(0, 0))],
            100,
        );
        let _report = apply_refuel(&mut s, Day(1));
        // Patrol fuel → cap.
        assert_eq!(patrol_fuel(&s, "p0"), 300);
        // Supply fuel → untouched.
        let supply = s.agents.iter().find(|a| a.id.0 == "s0").unwrap();
        assert_eq!(supply.fuel.0, 0);
    }

    // -------- Partial refuel (mixed positions) --------

    #[test]
    fn only_patrols_sharing_supply_cell_get_refueled() {
        let mut s = state_with(
            vec![
                patrol("p_near", Position::new(1, 1), 50),
                patrol("p_far", Position::new(5, 5), 50),
                supply("s0", Position::new(1, 1)),
            ],
            100,
        );
        let cap = fuel_upper_bound(AgentKind::Patrol, s.day_steps);

        let report = apply_refuel(&mut s, Day(1));

        assert_eq!(patrol_fuel(&s, "p_near"), cap);
        assert_eq!(patrol_fuel(&s, "p_far"), 50);
        assert_eq!(report.refueled_count(), 1);
        assert_eq!(s.refueled_today, 1);
    }

    // -------- Multiple supplies --------

    #[test]
    fn multiple_supplies_at_same_cell_refuel_once() {
        // Two Supply at the patrol's cell — fuel still capped at upper, refueled once
        // (the patrol entry is set once, not twice).
        let mut s = state_with(
            vec![
                patrol("p0", Position::new(0, 0), 10),
                supply("s0", Position::new(0, 0)),
                supply("s1", Position::new(0, 0)),
            ],
            100,
        );
        let cap = fuel_upper_bound(AgentKind::Patrol, s.day_steps);

        let report = apply_refuel(&mut s, Day(1));

        assert_eq!(patrol_fuel(&s, "p0"), cap);
        // Patrol id appears once in the report.
        assert_eq!(report.refueled, vec![AgentId("p0".to_string())]);
        assert_eq!(s.refueled_today, 1);
    }

    // -------- Determinism: apply_refuel is pure --------

    #[test]
    fn apply_refuel_is_deterministic() {
        let build = || {
            state_with(
                vec![
                    patrol("p0", Position::new(0, 0), 0),
                    patrol("p1", Position::new(0, 0), 0),
                    supply("s0", Position::new(0, 0)),
                ],
                10,
            )
        };
        let mut s1 = build();
        let mut s2 = build();
        let r1 = apply_refuel(&mut s1, Day(1));
        let r2 = apply_refuel(&mut s2, Day(1));
        assert_eq!(r1, r2);
        assert_eq!(s1, s2);
    }

    // -------- daily counter accumulates across calls --------

    #[test]
    fn refueled_today_accumulates_across_multiple_calls() {
        let mut s = state_with(
            vec![
                patrol("p0", Position::new(0, 0), 0),
                patrol("p1", Position::new(1, 1), 0),
                supply("s0", Position::new(0, 0)),
                supply("s1", Position::new(1, 1)),
            ],
            10,
        );

        // First call: both patrols at cap if their supply is co-located.
        let r1 = apply_refuel(&mut s, Day(1));
        assert_eq!(r1.refueled_count(), 2);
        assert_eq!(s.refueled_today, 2);

        // Patrols are now at cap; second call no-ops.
        let r2 = apply_refuel(&mut s, Day(1));
        assert_eq!(r2.refueled_count(), 0);
        assert_eq!(s.refueled_today, 2);
    }

    // -------- empty / no-supply edge cases --------

    #[test]
    fn no_supply_at_all_means_no_refuel() {
        let mut s = state_with(
            vec![patrol("p0", Position::new(0, 0), 0), patrol("p1", Position::new(1, 1), 0)],
            10,
        );
        let report = apply_refuel(&mut s, Day(1));
        assert_eq!(report.refueled_count(), 0);
        assert_eq!(s.refueled_today, 0);
        assert_eq!(patrol_fuel(&s, "p0"), 0);
        assert_eq!(patrol_fuel(&s, "p1"), 0);
    }

    #[test]
    fn empty_state_is_a_noop() {
        let mut s = state_with(vec![], 10);
        let report = apply_refuel(&mut s, Day(1));
        assert_eq!(report.refueled_count(), 0);
        assert_eq!(s.refueled_today, 0);
    }
}
