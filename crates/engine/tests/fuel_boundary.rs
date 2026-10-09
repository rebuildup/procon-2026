//! Fuel executability boundary (Issue #342 P0-C).
//!
//! Settled against the official simple answer server
//! (`docs/evidence/campaign-311/official-server-parity/`):
//!
//! * executability — a Move is legal iff `fuel >= fuel_cost(origin)`;
//! * charge — the deduction is `fuel_cost(origin)` (Q&A Q11).
//!
//! The decisive case is a patrol with `fuel = 1` on 平地 moving into 山地
//! (山地 fuel cost 2): the official server ACCEPTS that day (`revision = 1`)
//! and the day-boundary state shows `pos` on 山地 with `fuel = 0`. The old
//! conservative `max(origin, destination)` guard rejected it.
//!
//! These tests pin both the relaxed boundary and the cases that were already
//! legal under the conservative rule (they must stay legal).

use engine::model::{Action, Agent, Direction, HexCoord, Terrain};
use engine::rules::{fuel_cost, move_fuel};
use engine::transitions::try_move;

fn patrol_with_fuel(fuel: u32) -> Agent {
    Agent::patrol_with_fuel(0, HexCoord::new(0, 0), fuel)
}

// ---------------------------------------------------------------------------
// move_fuel: the decisive boundary
// ---------------------------------------------------------------------------

#[test]
fn plain_fuel_one_may_enter_mountain() {
    // Official case A: ACCEPTED, charged 1 (the 平地 departure cost).
    let agent = patrol_with_fuel(1);
    assert_eq!(move_fuel(&agent, Terrain::Plain, Terrain::Mountain), Some(1));
}

#[test]
fn mountain_fuel_one_may_not_enter_plain() {
    // Official case B: REJECTED — 山地 departure needs 2.
    let agent = patrol_with_fuel(1);
    assert_eq!(move_fuel(&agent, Terrain::Mountain, Terrain::Plain), None);
}

#[test]
fn plain_fuel_zero_may_not_enter_mountain() {
    // Official case A2: REJECTED — cannot pay even the 平地 cost of 1.
    // Threshold is `>=`, not `>`.
    let agent = patrol_with_fuel(0);
    assert_eq!(move_fuel(&agent, Terrain::Plain, Terrain::Mountain), None);
}

#[test]
fn charge_is_the_departure_cost_not_the_arrival_cost() {
    // Official case C: fuel=2 平地→山地 ends the day at fuel=1, so the
    // deduction is 1 (平地) and not 2 (山地).
    let agent = patrol_with_fuel(2);
    assert_eq!(move_fuel(&agent, Terrain::Plain, Terrain::Mountain), Some(1));
    assert_eq!(fuel_cost(Terrain::Plain), 1);
    assert_eq!(fuel_cost(Terrain::Mountain), 2);
}

#[test]
fn plain_fuel_one_may_enter_plain() {
    // Official case D: ordinary 平地→平地 stays legal and costs 1.
    let agent = patrol_with_fuel(1);
    assert_eq!(move_fuel(&agent, Terrain::Plain, Terrain::Plain), Some(1));
}

// ---------------------------------------------------------------------------
// Cases that were legal under the conservative rule must stay legal.
// ---------------------------------------------------------------------------

#[test]
fn conservative_legal_moves_stay_legal() {
    // Under max(origin, destination) these all required fuel >= the larger
    // bill; under the official rule they only need the origin bill. Every
    // fuel level that afforded the old guard must still afford the new one.
    let pairs = [
        (Terrain::Plain, Terrain::Plain, 1),
        (Terrain::Plain, Terrain::Mountain, 2),
        (Terrain::Plain, Terrain::Road(engine::model::TrafficState::Smooth), 2),
        (Terrain::Mountain, Terrain::Plain, 2),
        (Terrain::Mountain, Terrain::Mountain, 2),
        (Terrain::Mountain, Terrain::Road(engine::model::TrafficState::Jammed), 2),
        (Terrain::Road(engine::model::TrafficState::Smooth), Terrain::Plain, 2),
        (Terrain::Road(engine::model::TrafficState::Busy), Terrain::Mountain, 2),
        (Terrain::Road(engine::model::TrafficState::Jammed), Terrain::Road(engine::model::TrafficState::Smooth), 2),
    ];
    for (origin, destination, old_guard) in pairs {
        let fuel = old_guard.max(fuel_cost(origin)).max(fuel_cost(destination));
        let agent = patrol_with_fuel(fuel);
        assert_eq!(
            move_fuel(&agent, origin, destination),
            Some(fuel_cost(origin)),
            "{origin:?} -> {destination:?} at fuel {fuel} must stay legal and charge the origin"
        );
    }
}

#[test]
fn road_departure_still_requires_two() {
    // 道路 departure bills 2 regardless of what it enters.
    let agent = patrol_with_fuel(1);
    assert_eq!(move_fuel(&agent, Terrain::Road(engine::model::TrafficState::Smooth), Terrain::Plain), None);
    let agent = patrol_with_fuel(2);
    assert_eq!(
        move_fuel(&agent, Terrain::Road(engine::model::TrafficState::Jammed), Terrain::Plain),
        Some(2)
    );
}

// ---------------------------------------------------------------------------
// Integration: the same boundary through transitions::try_move on a real map.
// ---------------------------------------------------------------------------

fn two_cell_map(left: Terrain, right: Terrain) -> engine::model::MapGrid {
    use engine::model::{Cell, MapGrid, MapSize};
    let size = MapSize::new(8, 8).expect("8x8");
    let mut cells = Vec::new();
    for row in 0..i32::from(size.height) {
        for col in 0..i32::from(size.width) {
            let terrain = if col == 0 && row == 0 {
                left
            } else if col == 1 && row == 0 {
                right
            } else {
                Terrain::Plain
            };
            cells.push(Cell { coord: HexCoord::new(col, row), terrain, spot: None });
        }
    }
    MapGrid::new(size, cells).expect("grid")
}

#[test]
fn try_move_allows_plain_one_into_mountain() {
    // Mirrors official case A through the simulator gate.
    let map = two_cell_map(Terrain::Plain, Terrain::Mountain);
    let agent = patrol_with_fuel(1);
    let mut outcomes = Vec::new();
    let mut events = Vec::new();
    let outcome = try_move(&agent, Action::step(0, Direction::Right), &map, &mut outcomes, &mut events);
    assert!(outcome.is_moved(), "fuel=1 平地→山地 must move under the official rule");
}

#[test]
fn try_move_rejects_mountain_one_into_plain() {
    // Mirrors official case B through the simulator gate.
    let map = two_cell_map(Terrain::Mountain, Terrain::Plain);
    let mut agent = patrol_with_fuel(1);
    agent.pos = HexCoord::new(0, 0);
    let mut outcomes = Vec::new();
    let mut events = Vec::new();
    let outcome = try_move(&agent, Action::step(0, Direction::Right), &map, &mut outcomes, &mut events);
    assert!(!outcome.is_moved(), "fuel=1 山地→平地 must be refused (departure costs 2)");
}

#[test]
fn move_fuel_and_try_move_share_the_boundary() {
    // The unit contract and the simulator gate must agree on both sides of
    // the boundary, for every origin/destination pair exercised above.
    let cases: [(Terrain, Terrain, u32); 6] = [
        (Terrain::Plain, Terrain::Mountain, 1),
        (Terrain::Plain, Terrain::Mountain, 0),
        (Terrain::Mountain, Terrain::Plain, 1),
        (Terrain::Mountain, Terrain::Plain, 2),
        (Terrain::Plain, Terrain::Plain, 1),
        (Terrain::Road(engine::model::TrafficState::Smooth), Terrain::Plain, 1),
    ];
    for (origin, destination, fuel) in cases {
        let map = two_cell_map(origin, destination);
        let agent = patrol_with_fuel(fuel);
        let mut outcomes = Vec::new();
        let mut events = Vec::new();
        let outcome = try_move(&agent, Action::step(0, Direction::Right), &map, &mut outcomes, &mut events);
        let contract = move_fuel(&agent, origin, destination);
        assert_eq!(
            outcome.is_moved(),
            contract.is_some(),
            "{origin:?} -> {destination:?} at fuel {fuel}: try_move vs move_fuel"
        );
    }
}
