//! Rule constants and threshold tables.
//!
//! Spec sources:
//! - 募集要項「地形タイプ」「道路セルと交通量」「スポット」
//! - Q&A Q11 (road step counts), Q13 (traffic thresholds), Q14 (fuel cap),
//!   Q15 (spot count), Q18 (one cell one spot).

use crate::model::{Agent, Terrain, TrafficState};

// ---------------------------------------------------------------------------
// Terrain costs.
// ---------------------------------------------------------------------------

/// Fuel cost (in fuel units) of a single move action.
///
/// Spec: 募集要項「移動と燃料」:
/// - Plain: 1
/// - Mountain: 2
/// - Road (any traffic state): 2
///
/// `Pond` is impassable and never reached.
#[must_use]
pub const fn fuel_cost(terrain: Terrain) -> u32 {
    match terrain {
        Terrain::Plain => 1,
        Terrain::Mountain => 2,
        Terrain::Road(_) => 2,
        Terrain::Pond => u32::MAX,
    }
}

/// Official move fuel contract (Issue #342 P0-C): afford the departure
/// cell's cost, charge the departure cell's cost.
///
/// Spec: Q&A Q11 (表1「燃量」= 燃料の消費量 for moving *from* each terrain
/// type) and 募集要項「移動と燃料」 (a patrol consumes the terrain's fuel
/// at the time the move command is issued — the cell it stands on).
///
/// Pinned against the official simple answer server
/// (`docs/evidence/campaign-311/official-server-parity/`): a patrol with
/// `fuel = 1` on 平地 may move into 山地 (consumes 1) and the day is
/// accepted; the same patrol on 山地 may not step onto 平地 (needs 2).
/// The pre-#342 conservative `max(origin, destination)` guard was an
/// offline hedge that the official acceptance data contradicts.
///
/// `destination` is retained for call-site stability and documentation
/// only — it takes no part in the executability check or the deduction.
#[must_use]
pub fn move_fuel(agent: &Agent, origin: Terrain, _destination: Terrain) -> Option<u32> {
    let cost = fuel_cost(origin);
    agent.can_afford(cost).then_some(cost)
}

// ---------------------------------------------------------------------------
// Traffic thresholds (Q&A Q13).
// ---------------------------------------------------------------------------

/// Per-cell traffic count thresholds for `Smooth → Busy → Jammed`.
///
/// Spec: 募集要項「道路セルと交通量」 — busy and jammed thresholds live in
/// `[1, 5]` and `[2, 10]` respectively (Q&A Q13). Defaults below sit at the
/// midpoints (`2` and `5`), which the contest sample map uses.
///
/// Per-day count `c`:
/// - `c < busy_threshold`     → `Smooth`
/// - `c < jammed_threshold`   → `Busy`
/// - `c >= jammed_threshold`  → `Jammed`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrafficThresholds {
    pub busy_threshold: u32,
    pub jammed_threshold: u32,
}

impl Default for TrafficThresholds {
    fn default() -> Self {
        Self { busy_threshold: 2, jammed_threshold: 5 }
    }
}

impl TrafficThresholds {
    /// Build thresholds, validating Q&A Q13's `[1, 5]` / `[2, 10]` ranges.
    ///
    /// # Errors
    /// Returns `Err` if either threshold is out of range, or if `busy_threshold`
    /// is not strictly less than `jammed_threshold`.
    pub fn new(busy_threshold: u32, jammed_threshold: u32) -> Result<Self, String> {
        if !(1..=5).contains(&busy_threshold) {
            return Err(format!("busy_threshold {busy_threshold} out of [1, 5] (Q&A Q13)"));
        }
        if !(2..=10).contains(&jammed_threshold) {
            return Err(format!("jammed_threshold {jammed_threshold} out of [2, 10] (Q&A Q13)"));
        }
        if busy_threshold >= jammed_threshold {
            return Err(format!(
                "busy_threshold ({busy_threshold}) must be < jammed_threshold ({jammed_threshold})"
            ));
        }
        Ok(Self { busy_threshold, jammed_threshold })
    }

    /// Map a single-day count to a traffic state.
    #[must_use]
    pub fn classify(&self, count: u32) -> TrafficState {
        if count >= self.jammed_threshold {
            TrafficState::Jammed
        } else if count >= self.busy_threshold {
            TrafficState::Busy
        } else {
            TrafficState::Smooth
        }
    }

    /// Combine `today`'s count with `yesterday`'s — Q&A Q13 says the state
    /// is determined by the agent count *over 2 days*, so we sum the two days, then classify.
    #[must_use]
    pub fn classify_pair(&self, today: u32, yesterday: u32) -> TrafficState {
        let combined = today.saturating_add(yesterday);
        self.classify(combined)
    }
}

// ---------------------------------------------------------------------------
// Fuel.
// ---------------------------------------------------------------------------

/// Default patrol fuel cap multiplier (Q&A Q14 — `day_steps * 1..3`).
///
/// Use [`PatrolFuelMultiplier::validate`] to enforce the spec's range.
pub const DEFAULT_PATROL_FUEL_MULTIPLIER: u32 = 3;

/// Spec bounds for the patrol fuel cap multiplier.
pub const MIN_PATROL_FUEL_MULTIPLIER: u32 = 1;
pub const MAX_PATROL_FUEL_MULTIPLIER: u32 = 3;

/// Validate that a multiplier lies in the spec range.
pub fn validate_patrol_fuel_multiplier(m: u32) -> Result<(), String> {
    if !(MIN_PATROL_FUEL_MULTIPLIER..=MAX_PATROL_FUEL_MULTIPLIER).contains(&m) {
        return Err(format!(
            "patrol fuel multiplier {m} out of [{MIN_PATROL_FUEL_MULTIPLIER}, {MAX_PATROL_FUEL_MULTIPLIER}] (Q&A Q14)"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Acquisition.
// ---------------------------------------------------------------------------

/// Maximum balls one agent can take from a single spot in a single day.
///
/// Spec: 募集要項「スポット」 — 1 day, 1 spot, 1 ball per agent.
pub const SPOT_DAILY_PICK_PER_AGENT: u32 = 1;

/// True if `agent` has reached the per-day pickup limit at `coord`.
#[must_use]
pub fn agent_at_daily_pick_cap(
    agent: &Agent,
    coord: crate::model::HexCoord,
    picks_today: &std::collections::HashMap<(u8, crate::model::HexCoord), u32>,
) -> bool {
    picks_today.get(&(agent.id, coord)).copied().unwrap_or(0) >= SPOT_DAILY_PICK_PER_AGENT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HexCoord;

    #[test]
    fn fuel_cost_table_matches_spec() {
        assert_eq!(fuel_cost(Terrain::Plain), 1);
        assert_eq!(fuel_cost(Terrain::Mountain), 2);
        assert_eq!(fuel_cost(Terrain::Road(TrafficState::Smooth)), 2);
        assert_eq!(fuel_cost(Terrain::Road(TrafficState::Busy)), 2);
        assert_eq!(fuel_cost(Terrain::Road(TrafficState::Jammed)), 2);
    }

    #[test]
    fn traffic_thresholds_default_sits_at_midpoints() {
        let t = TrafficThresholds::default();
        assert_eq!(t.busy_threshold, 2);
        assert_eq!(t.jammed_threshold, 5);
        assert_eq!(t.classify(0), TrafficState::Smooth);
        assert_eq!(t.classify(1), TrafficState::Smooth);
        assert_eq!(t.classify(2), TrafficState::Busy);
        assert_eq!(t.classify(4), TrafficState::Busy);
        assert_eq!(t.classify(5), TrafficState::Jammed);
        assert_eq!(t.classify(99), TrafficState::Jammed);
    }

    #[test]
    fn traffic_thresholds_classify_pair_sums_two_days() {
        let t = TrafficThresholds::default();
        // Both days contribute to the rolling window.
        assert_eq!(t.classify_pair(0, 5), TrafficState::Jammed);
        assert_eq!(t.classify_pair(5, 0), TrafficState::Jammed);
        assert_eq!(t.classify_pair(0, 0), TrafficState::Smooth);
        assert_eq!(t.classify_pair(1, 1), TrafficState::Busy);
        assert_eq!(t.classify_pair(u32::MAX, 1), TrafficState::Jammed);
        assert_eq!(t.classify_pair(2, 0), TrafficState::Busy);
    }

    #[test]
    fn traffic_thresholds_rejects_out_of_range() {
        assert!(TrafficThresholds::new(0, 5).is_err());
        assert!(TrafficThresholds::new(6, 5).is_err());
        assert!(TrafficThresholds::new(2, 1).is_err());
        assert!(TrafficThresholds::new(2, 11).is_err());
        // busy == jammed is also rejected.
        assert!(TrafficThresholds::new(5, 5).is_err());
        assert!(TrafficThresholds::new(1, 10).is_ok());
    }

    #[test]
    fn patrol_fuel_multiplier_validates_range() {
        assert!(validate_patrol_fuel_multiplier(0).is_err());
        assert!(validate_patrol_fuel_multiplier(4).is_err());
        assert!(validate_patrol_fuel_multiplier(1).is_ok());
        assert!(validate_patrol_fuel_multiplier(3).is_ok());
    }

    #[test]
    fn agent_at_daily_pick_cap_uses_count() {
        use crate::model::Agent;
        let a = Agent::patrol(0, HexCoord::new(0, 0), 10, 3);
        let empty = std::collections::HashMap::new();
        assert!(!agent_at_daily_pick_cap(&a, HexCoord::new(0, 0), &empty));
        let mut one = std::collections::HashMap::new();
        one.insert((0, HexCoord::new(0, 0)), 1);
        assert!(agent_at_daily_pick_cap(&a, HexCoord::new(0, 0), &one));
    }
}
