//! Explicit ILP constraint builders for the cross-day solver.
//!
//! Reference: issue #38 — fuel capacity / consumption, spot inventory
//! (max simultaneous occupancy), and supply/refill arcs from refill spots
//! to agents.
//!
//! The current prototype uses a relaxed LP with a hand-rolled variable
//! representation (see [`crate::model::Model`]). When the `good_lp`
//! feature is enabled the same constraints will be translated to
//! `good_lp` rows; for now we store them as typed rows on the model so
//! tests and downstream code can inspect them directly. The shape of
//! each row mirrors the linear form that the real backend would emit:
//!
//! - **Fuel rows**: `Σ_step Σ_direction (cost * x[a,d,s,dir]) ≤ cap`
//!   per `(agent, day)`.
//! - **Inventory rows**: `Σ_agent Σ_step (x[a,d,s,dir] * spot_visit[a,s])
//!   ≤ max_simultaneous` per `(spot, day)`, plus a per-series aggregate.
//! - **Supply arcs**: `(agent, refill_coord) → fuel_capacity` — when the
//!   agent's decision variable lands on `refill_coord` at step `s`, the
//!   agent's fuel counter is reset to its day-cap.
//!
//! Each builder is pure: it inspects the model + parameters and appends
//! rows. It does **not** mutate the existing decision variables, so the
//! builders compose freely. The builders return [`Result`] with a string
//! error message so callers can detect parameter validation failures
//! without pulling in an extra error crate.

use serde::{Deserialize, Serialize};

use crate::model::Model;
use crate::types::{AgentId, DaySpec, HexCoord, SpotId};

/// Standardised error type for constraint builders.
///
/// We deliberately keep the message in `String` (rather than a typed enum)
/// because the only failing cases are parameter validation and the
/// callers are tests / internal pipelines.
pub type ConstraintError = String;

/// Standardised [`Result`] alias used by every public builder in this
/// module.
pub type ConstraintResult<T> = Result<T, ConstraintError>;

/// Parameters for [`add_fuel_constraints`].
///
/// Each call appends one [`FuelRow`] per `(agent, day)` pair to the
/// model. If [`fuel_cap_override`](Self::fuel_cap_override) is `Some`,
/// the per-day cap from [`DaySpec::fuel_cap_override`] is ignored and the
/// override is used instead — useful for tests that want to drive an
/// agent into infeasibility without constructing a tight context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuelParams {
    /// Per-step movement cost. Defaults to `1` (Patrol cars) per the
    /// contest fuel model. Supply cars use `0` (no fuel consumption).
    pub per_step_cost: u32,
    /// Optional override for the per-day fuel cap. `None` keeps the cap
    /// derived from [`SolveContext::fuel_cap`](crate::SolveContext::fuel_cap).
    pub fuel_cap_override: Option<u32>,
}

impl Default for FuelParams {
    fn default() -> Self {
        Self { per_step_cost: 1, fuel_cap_override: None }
    }
}

/// Parameters for [`add_spot_inventory_constraints`].
///
/// The contest rule is "1 spot × 1 udon × 1 team × 1 day" — i.e. the
/// maximum simultaneous occupancy per spot is `1`. The series-aggregate
/// limit mirrors the team-size upper bound: at most `team_size` agents
/// can collect from the same series on a single day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryParams {
    /// Maximum number of agents that may collect from a single spot on
    /// the same day. Defaults to `1` per the contest rule.
    pub max_simultaneous_per_spot: u32,
    /// Maximum total collections from one series on a single day across
    /// all agents. Defaults to `8` (the contest's team-size ceiling).
    pub max_per_series_per_day: u32,
}

impl Default for InventoryParams {
    fn default() -> Self {
        Self { max_simultaneous_per_spot: 1, max_per_series_per_day: 8 }
    }
}

/// Parameters for [`add_supply_arc_constraints`].
///
/// Each `(coord, capacity)` pair describes a refill point: when an agent
/// lands on `coord` (its decision variable is `1.0` for the direction
/// that lands on `coord`), the agent's fuel counter resets to at most
/// `capacity` for the rest of the day. Multiple refill points are
/// allowed; the agent picks the first one it reaches along its route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplyParams {
    /// Refill points: `(coord, capacity_after_refill)`.
    pub refill_points: Vec<(HexCoord, u32)>,
    /// Whether same-cell refuel counts as a no-op move (cost 0 fuel).
    /// Defaults to `true` — supply cars share a cell with a patrol car
    /// and refuel without consuming a step.
    pub same_cell_refill: bool,
}

impl Default for SupplyParams {
    fn default() -> Self {
        Self { refill_points: Vec::new(), same_cell_refill: true }
    }
}

/// A single fuel-capacity constraint row.
///
/// Linear form: for each `(agent, day)` pair,
/// `Σ_step Σ_direction (cost * x[agent, day, step, direction]) ≤ cap`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuelRow {
    /// Agent this row applies to.
    pub agent: AgentId,
    /// Day this row applies to.
    pub day: u32,
    /// Per-step movement cost.
    pub cost_per_step: u32,
    /// Per-day fuel cap.
    pub cap: u32,
}

/// A single spot-inventory constraint row.
///
/// Linear form: for each `(spot, day)` pair,
/// `Σ_agent Σ_step (visit[agent, step, spot] * x[agent, day, step, dir])
///  ≤ max_simultaneous`, where `visit` is `1` iff the chosen direction
/// lands the agent on the spot at that step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryRow {
    /// Day this row applies to.
    pub day: u32,
    /// Spot this row applies to.
    pub spot: SpotId,
    /// Maximum simultaneous occupancy (typically `1`).
    pub max_simultaneous: u32,
}

/// A single series-aggregate inventory row (across all spots of one
/// series on one day).
///
/// Linear form: for each `(series, day)` pair,
/// `Σ_spot_of_series Σ_agent Σ_step (x[..] landing on the spot) ≤ max`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesInventoryRow {
    /// Day this row applies to.
    pub day: u32,
    /// Series identifier.
    pub series: crate::types::SeriesId,
    /// Maximum aggregate collections on this day across all spots of the
    /// series.
    pub max_per_day: u32,
}

/// A single supply/refill arc.
///
/// The arc says: "if agent `agent` is at `refill_coord` on day `day`
/// (its decision variable fires), its fuel counter may reset to
/// `fuel_after_refill`". The capacity is the upper bound on fuel the
/// agent can have after the refill — the actual fuel used before the
/// refill is bounded by the [`FuelRow`] for the same `(agent, day)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplyArc {
    /// Agent this arc applies to.
    pub agent: AgentId,
    /// Day this arc applies to.
    pub day: u32,
    /// Coord of the refill point.
    pub refill_coord: HexCoord,
    /// Fuel remaining after refill (upper bound).
    pub fuel_after_refill: u32,
    /// Whether same-cell refuel counts as free (cost 0 fuel).
    pub same_cell_refill: bool,
}

/// Add explicit fuel constraints to the model.
///
/// One [`FuelRow`] is appended per `(agent, day)` pair. The cap is
/// derived from `params.fuel_cap_override` when present, otherwise from
/// the per-day `DaySpec` via `Model::fuel_cap`. The per-step cost comes
/// from `params.per_step_cost`.
///
/// Returns an error if any agent's id is invalid or if any day in the
/// model is out of range — these are programmer errors, not user
/// errors, and should not happen in practice (the model is built from a
/// `SolveContext` that already enforces them).
pub fn add_fuel_constraints(model: &mut Model, params: &FuelParams) -> ConstraintResult<()> {
    if model.agents.is_empty() {
        return Err("add_fuel_constraints: model has no agents".to_owned());
    }
    if model.days.is_empty() {
        return Err("add_fuel_constraints: model has no days".to_owned());
    }
    if params.per_step_cost == 0 && params.fuel_cap_override.is_none() {
        // Supply cars: cost 0 per move, cap is meaningless. Still emit a
        // row so the LP has the canonical form; downstream solvers can
        // drop it.
    }

    for (day_index, day_spec) in model.days.iter().enumerate() {
        let day = u32::try_from(day_index)
            .map_err(|_| format!("day index {day_index} does not fit in u32"))?;
        let cap = params.fuel_cap_override.unwrap_or_else(|| day_cap(day_spec));

        for agent in &model.agents {
            model.fuel_rows.push(FuelRow {
                agent: agent.id,
                day,
                cost_per_step: params.per_step_cost,
                cap,
            });
        }
    }
    Ok(())
}

/// Add explicit spot-inventory constraints to the model.
///
/// For every `(spot, day)` pair, one [`InventoryRow`] is appended with
/// `max_simultaneous = params.max_simultaneous_per_spot`. For every
/// `(series, day)` pair with at least one spot, one
/// [`SeriesInventoryRow`] is appended with
/// `max_per_day = params.max_per_series_per_day`.
///
/// The constraints are *advisory* in the prototype — they appear in
/// `model.inventory_rows` / `model.series_inventory_rows` so the LP
/// model is well-defined, but the relaxed-LP fallback (`greedy_decode`)
/// enforces them at decode time using the same values.
pub fn add_spot_inventory_constraints(
    model: &mut Model,
    params: &InventoryParams,
) -> ConstraintResult<()> {
    if model.spots.is_empty() {
        return Err("add_spot_inventory_constraints: model has no spots".to_owned());
    }
    if model.days.is_empty() {
        return Err("add_spot_inventory_constraints: model has no days".to_owned());
    }
    if params.max_simultaneous_per_spot == 0 {
        return Err(
            "add_spot_inventory_constraints: max_simultaneous_per_spot must be >= 1".to_owned()
        );
    }

    for (day_index, _day_spec) in model.days.iter().enumerate() {
        let day = u32::try_from(day_index)
            .map_err(|_| format!("day index {day_index} does not fit in u32"))?;

        // Per-spot rows: one per (spot, day).
        for spot in &model.spots {
            model.inventory_rows.push(InventoryRow {
                day,
                spot: spot.id,
                max_simultaneous: params.max_simultaneous_per_spot,
            });
        }

        // Per-series aggregate rows: one per (series, day). Group spots
        // by series so the LP gets a single aggregate row rather than N
        // rows that all share the same RHS.
        let mut by_series: std::collections::BTreeMap<crate::types::SeriesId, Vec<SpotId>> =
            std::collections::BTreeMap::default();
        for spot in &model.spots {
            by_series.entry(spot.series_id).or_default().push(spot.id);
        }
        for (series, members) in by_series {
            model.series_inventory_rows.push(SeriesInventoryRow {
                day,
                series,
                max_per_day: params.max_per_series_per_day,
            });
            let _ = members; // membership is implicit via spot rows.
        }
    }
    Ok(())
}

/// Add supply/refill arc constraints to the model.
///
/// For each refill point `(coord, capacity)` in
/// `params.refill_points`, one [`SupplyArc`] is appended per
/// `(agent, day)` pair. The arc says: "if this agent ever lands on
/// `coord` on day `day`, its fuel counter may be reset to `capacity`".
///
/// The constraint is satisfied automatically by the LP: any plan that
/// visits `coord` is allowed to assume the fuel reset happened, and
/// the LP picks the highest-reward option. The relaxed-LP fallback
/// (`greedy_decode`) honours the arcs by stopping fuel bookkeeping at
/// the first step the agent reaches a refill point.
///
/// An empty `refill_points` list is allowed — the model simply gets no
/// supply arcs, which matches a Patrol-only configuration.
pub fn add_supply_arc_constraints(
    model: &mut Model,
    params: &SupplyParams,
) -> ConstraintResult<()> {
    if model.agents.is_empty() {
        return Err("add_supply_arc_constraints: model has no agents".to_owned());
    }
    if model.days.is_empty() {
        return Err("add_supply_arc_constraints: model has no days".to_owned());
    }
    if params.refill_points.is_empty() {
        return Ok(()); // Patrol-only: nothing to add.
    }
    if params.refill_points.iter().any(|(_, cap)| *cap == 0) {
        return Err("add_supply_arc_constraints: refill capacity must be > 0 (or omit the point)"
            .to_owned());
    }

    for (day_index, _day_spec) in model.days.iter().enumerate() {
        let day = u32::try_from(day_index)
            .map_err(|_| format!("day index {day_index} does not fit in u32"))?;

        for agent in &model.agents {
            for (coord, capacity) in &params.refill_points {
                model.supply_arcs.push(SupplyArc {
                    agent: agent.id,
                    day,
                    refill_coord: *coord,
                    fuel_after_refill: *capacity,
                    same_cell_refill: params.same_cell_refill,
                });
            }
        }
    }
    Ok(())
}

/// Derive the per-day cap from a [`DaySpec`]. Mirrors
/// [`crate::types::SolveContext::fuel_cap`] but reads the field directly
/// so callers don't need to round-trip through a `SolveContext`.
fn day_cap(day_spec: &DaySpec) -> u32 {
    day_spec.fuel_cap_override.unwrap_or_else(|| day_spec.steps.saturating_mul(3))
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    fn model_with_one_agent_one_day() -> Model {
        let mut m = Model::default();
        m.days = vec![DaySpec::new(10)];
        m.agents = vec![crate::types::Agent {
            id: crate::types::AgentId(0),
            coord: HexCoord::new(0, 0),
            fuel: 30,
        }];
        m
    }

    #[test]
    fn add_fuel_constraints_appends_one_row_per_agent_day() {
        let mut m = model_with_one_agent_one_day();
        add_fuel_constraints(&mut m, &FuelParams::default()).unwrap();
        assert_eq!(m.fuel_rows.len(), 1);
        assert_eq!(m.fuel_rows[0].agent, crate::types::AgentId(0));
        assert_eq!(m.fuel_rows[0].day, 0);
        assert_eq!(m.fuel_rows[0].cost_per_step, 1);
        // Default cap is `steps * 3 = 30`.
        assert_eq!(m.fuel_rows[0].cap, 30);
    }

    #[test]
    fn add_fuel_constraints_with_override_uses_override() {
        let mut m = model_with_one_agent_one_day();
        let params = FuelParams { per_step_cost: 1, fuel_cap_override: Some(7) };
        add_fuel_constraints(&mut m, &params).unwrap();
        assert_eq!(m.fuel_rows[0].cap, 7, "override must win over day_cap");
    }

    #[test]
    fn add_fuel_constraints_rejects_empty_agent_set() {
        let mut m = Model::default();
        m.days = vec![DaySpec::new(10)];
        assert!(add_fuel_constraints(&mut m, &FuelParams::default()).is_err());
    }

    #[test]
    fn add_fuel_constraints_rejects_empty_day_set() {
        let mut m = Model::default();
        m.agents = vec![crate::types::Agent {
            id: crate::types::AgentId(0),
            coord: HexCoord::new(0, 0),
            fuel: 30,
        }];
        assert!(add_fuel_constraints(&mut m, &FuelParams::default()).is_err());
    }

    #[test]
    fn add_inventory_constraints_appends_rows_per_spot_and_series() {
        let mut m = Model::default();
        m.days = vec![DaySpec::new(10)];
        m.spots = vec![
            crate::types::Spot {
                id: SpotId(0),
                coord: HexCoord::new(1, 0),
                series_id: crate::types::SeriesId(0),
                initial_stock: 1,
            },
            crate::types::Spot {
                id: SpotId(1),
                coord: HexCoord::new(2, 0),
                series_id: crate::types::SeriesId(0),
                initial_stock: 1,
            },
            crate::types::Spot {
                id: SpotId(2),
                coord: HexCoord::new(0, 1),
                series_id: crate::types::SeriesId(1),
                initial_stock: 1,
            },
        ];
        add_spot_inventory_constraints(&mut m, &InventoryParams::default()).unwrap();
        assert_eq!(m.inventory_rows.len(), 3, "one row per spot per day");
        assert_eq!(
            m.series_inventory_rows.len(),
            2,
            "one aggregate row per distinct series per day"
        );
    }

    #[test]
    fn add_inventory_constraints_rejects_zero_max_simultaneous() {
        let mut m = Model::default();
        m.days = vec![DaySpec::new(10)];
        m.spots = vec![crate::types::Spot {
            id: SpotId(0),
            coord: HexCoord::new(0, 0),
            series_id: crate::types::SeriesId(0),
            initial_stock: 1,
        }];
        let bad = InventoryParams { max_simultaneous_per_spot: 0, max_per_series_per_day: 1 };
        assert!(add_spot_inventory_constraints(&mut m, &bad).is_err());
    }

    #[test]
    fn add_supply_arc_constraints_appends_one_arc_per_agent_day_point() {
        let mut m = model_with_one_agent_one_day();
        let params =
            SupplyParams { refill_points: vec![(HexCoord::new(0, 0), 30)], same_cell_refill: true };
        add_supply_arc_constraints(&mut m, &params).unwrap();
        assert_eq!(m.supply_arcs.len(), 1);
        assert_eq!(m.supply_arcs[0].refill_coord, HexCoord::new(0, 0));
        assert_eq!(m.supply_arcs[0].fuel_after_refill, 30);
    }

    #[test]
    fn add_supply_arc_constraints_empty_points_is_noop() {
        let mut m = model_with_one_agent_one_day();
        add_supply_arc_constraints(&mut m, &SupplyParams::default()).unwrap();
        assert_eq!(m.supply_arcs.len(), 0, "no refill points → no arcs");
    }

    #[test]
    fn add_supply_arc_constraints_rejects_zero_capacity() {
        let mut m = model_with_one_agent_one_day();
        let bad =
            SupplyParams { refill_points: vec![(HexCoord::new(0, 0), 0)], same_cell_refill: true };
        assert!(add_supply_arc_constraints(&mut m, &bad).is_err());
    }
}
