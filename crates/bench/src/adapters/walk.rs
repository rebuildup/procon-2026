//! Shared helpers for turning a solver's *intent* (target spots / tours) into
//! concrete [`engine::model::AgentActions`] on the live map.
//!
//! Every bundled candidate adapter needs the same two primitives:
//!
//! - [`step_towards`] — one legal step from a cell toward a target cell.
//! - [`plan_walk`] — a whole day's step-major plan, walking each agent along
//!   its own list of targets.
//!
//! Keeping them here means a new candidate only has to decide *where* its
//! agents should go; the hex geometry and the engine's legality rules stay in
//! one place. The engine remains the final authority on what a move does:
//! [`engine::MatchSim`] still downgrades anything illegal to a `Wait`, so an
//! imperfect plan degrades instead of aborting the match.
//!
//! # Who gets a target (`assign=`)
//!
//! The same question, one level up: [`AssignMode`] decides *which* agents
//! receive a target list at all. `assign=agents` (default) is everyone, the
//! pre-#235 behaviour; `assign=patrol` withholds the tour from
//! [`AgentKind::Supply`] agents so they wait instead of walking a spot they
//! could never collect from (Issue #235); `assign=escort` hands each Supply
//! the projected day path of its paired Patrol so the two meet and refuel
//! mid-day (Issue #248); `assign=all` gives every Patrol the union of every
//! agent's share so no spot is structurally unreachable (Issue #253).
//!
//! # Walk modes (`walk=`)
//!
//! [`plan_walk`] is the `greedy` mode — the only one that existed before
//! Issue #215. The `astar` mode walks the **same** target list but in list
//! order with A*-planned legs; it lives in [`super::astar_walk`] because it
//! carries Issue #194's coordinate-conversion machinery. Candidates select
//! one or the other through [`plan_walk_with`] after parsing their `walk`
//! parameter with [`parse_walk`]. The assignment half of a tour candidate is
//! otherwise untouched by the choice — except that [`AssignMode::Escort`]
//! projects the paired Patrol's path with the *same* walker (Issue #248), so
//! the Supply is always handed the cells the Patrol will actually walk. How
//! it then consumes that list stays the walker's own semantics: per-step
//! pursuit under `greedy`, ordered-waypoint replay under `astar`.

use engine::model::{
    Action, ActionKind, AgentActions, AgentKind, Direction, HexCoord, MapGrid, SolverContext,
};

use crate::candidate::{CandidateError, CandidateSpec};

/// Even-r hex distance between two cells.
///
/// The engine grid is an **even-row right-offset** layout — `Direction::step`
/// branches on row parity — so `(col, row)` are offset coordinates, not axial
/// ones. Measuring them as axial (the previous formula) reports a real
/// neighbour as two steps away and a two-step pair as one, which breaks
/// [`step_towards`]: its strictly-decreasing test finds no qualifying
/// neighbour and returns `None`, so the walker emits `Wait` for the rest of
/// the day while the target is still reachable.
///
/// Convert offset to cube first — `q = col - (row + row % 2) / 2`, the layout
/// `engine::model::Direction::step` actually implements — and measure in cube
/// space, which reproduces the engine's neighbour graph exactly.
#[must_use]
pub fn hex_distance(a: HexCoord, b: HexCoord) -> u32 {
    fn cube(c: HexCoord) -> (i64, i64, i64) {
        let row = i64::from(c.row);
        let q = i64::from(c.col) - (row + i64::from(c.row.rem_euclid(2))) / 2;
        (q, -q - row, row)
    }
    let (ax, ay, az) = cube(a);
    let (bx, by, bz) = cube(b);
    let dx = (ax - bx).abs();
    let dy = (ay - by).abs();
    let dz = (az - bz).abs();
    u32::try_from((dx + dy + dz) / 2).unwrap_or(u32::MAX)
}

/// One legal step that gets closer to `to`, or `None` when there is nothing
/// useful to do: already there, `to` off the map, or no legal neighbour is
/// strictly closer.
///
/// This is a *greedy* single step, not a path plan — [`plan_walk`] calls it
/// once per step and lets the agent advance across steps. Terrain legality is
/// consulted for the immediate cell only; [`engine::MatchSim`] remains the
/// final authority and downgrades anything illegal to a `Wait`.
///
/// Direction candidates are scanned in [`Direction::ALL`] (wire-code order)
/// and the strictly-closest wins, with first-wins on ties. That makes the
/// choice deterministic — two runs of the same candidate on the same case pick
/// the same step.
#[must_use]
pub fn step_towards(from: HexCoord, to: HexCoord, map: &MapGrid) -> Option<Direction> {
    if from == to {
        return None;
    }
    map.get(to)?;
    let current = hex_distance(from, to);
    let mut best: Option<(Direction, u32)> = None;
    for dir in Direction::ALL {
        let next = dir.step(from);
        if !map.get(next).is_some_and(|c| c.terrain.traversable()) {
            continue;
        }
        let d = hex_distance(next, to);
        if d >= current {
            continue;
        }
        match best {
            Some((_, bd)) if bd <= d => {}
            _ => best = Some((dir, d)),
        }
    }
    best.map(|(dir, _)| dir)
}

/// Build one day's step-major plan: for every step, each controlled agent
/// takes one legal step toward the next entry of its target list.
///
/// `targets` is indexed by position in `ctx.agent_ids` — so `targets[i]`
/// belongs to `ctx.agent_ids[i]`. An empty target list makes that agent wait
/// the whole day. Targets cycle: `step % targets.len()`, so a short list is
/// repeated rather than running out.
///
/// The result is always well-shaped (`agent_count * day_steps` actions, one
/// per controlled agent per step, in `ctx.agent_ids` order), so
/// [`engine::MatchSim`] never has to reject it as malformed.
#[must_use]
pub fn plan_walk(ctx: &SolverContext<'_>, targets: &[Vec<HexCoord>]) -> AgentActions {
    let agent_count = ctx.agent_ids.len();
    let day_steps = ctx.day_steps as usize;
    let mut actions = Vec::with_capacity(agent_count.saturating_mul(day_steps));

    // Live positions, indexed like `targets`.
    let mut positions: Vec<HexCoord> = ctx
        .agent_ids
        .iter()
        .map(|id| {
            ctx.state.agents.iter().find(|a| a.id == *id).map_or(HexCoord::new(0, 0), |a| a.pos)
        })
        .collect();

    for step in 0..day_steps {
        for (i, &agent_id) in ctx.agent_ids.iter().enumerate() {
            let Some(list) = targets.get(i).filter(|l| !l.is_empty()) else {
                actions.push(Action::wait(agent_id));
                continue;
            };
            let target = list[step % list.len()];
            match step_towards(positions[i], target, &ctx.state.setting.map) {
                Some(dir) => {
                    positions[i] = dir.step(positions[i]);
                    actions.push(Action::step(agent_id, dir));
                }
                None => actions.push(Action::wait(agent_id)),
            }
        }
    }
    AgentActions::new(actions)
}

/// How a candidate walks the per-agent target list it was given — the value
/// of the `walk=` parameter shared by the tour-shaped candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WalkMode {
    /// `greedy` (the default): [`plan_walk`] byte for byte — target chosen by
    /// `step % targets.len()`, one terrain-blind [`step_towards`] step.
    #[default]
    Greedy,
    /// `astar`: visit the list **in order**, one A* leg per target, then
    /// wait — Issue #215's anti-oscillation walk (see [`super::astar_walk`]).
    Astar,
}

/// Parse the `walk` parameter into a [`WalkMode`].
///
/// Omitted means [`WalkMode::Greedy`]: every pre-existing candidate id keeps
/// its exact behaviour, and `walk=greedy` is the explicit spelling of the
/// same default (Issue #215 appends the parameter last so existing canonical
/// ids keep their shape).
///
/// # Errors
/// [`CandidateError::InvalidParameter`] for any value other than
/// `greedy` / `astar`.
pub(crate) fn parse_walk(spec: &CandidateSpec) -> Result<WalkMode, CandidateError> {
    match spec.param("walk") {
        None | Some("greedy") => Ok(WalkMode::Greedy),
        Some("astar") => Ok(WalkMode::Astar),
        Some(other) => Err(CandidateError::InvalidParameter {
            key: "walk".to_owned(),
            value: other.to_owned(),
            reason: "expected greedy | astar".to_owned(),
        }),
    }
}

/// Parse the `budget` parameter into a [`BudgetMode`].
///
/// Omitted means [`BudgetMode::Element`]: every pre-#255 id keeps its exact
/// behaviour (the parameter is appended last in each factory's `PARAMS`, so
/// existing canonical ids keep their shape).
///
/// # Errors
/// [`CandidateError::InvalidParameter`] for any value other than
/// `element` / `official`.
pub(crate) fn parse_budget(spec: &CandidateSpec) -> Result<BudgetMode, CandidateError> {
    match spec.param("budget") {
        None | Some("element") => Ok(BudgetMode::Element),
        Some("official") => Ok(BudgetMode::Official),
        Some(other) => Err(CandidateError::InvalidParameter {
            key: "budget".to_owned(),
            value: other.to_owned(),
            reason: "expected element | official".to_owned(),
        }),
    }
}

/// How Supply agents are paired to Patrols (`fuelpair=`, Issue #252).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FuelpairMode {
    /// `fuelpair=nearest` (default): each Supply escorts the nearest Patrol
    /// with a free escort slot — byte-identical to the pre-#252 pairing.
    #[default]
    Nearest,
    /// `fuelpair=need`: pair to the neediest Patrol first (lowest fuel
    /// residue; distance breaks ties only), and give every Patrol no Supply
    /// was free for a **rendezvous tail** — the end cell of the nearest
    /// Supply's projected day path, where that Supply parks — so it can
    /// still be refuelled before it strands (Issue #252 現象2, directions
    /// a + b together: with `ceil(S/P) = 1` there is always at least one
    /// unescorted Patrol once Patrols outnumber Supplies).
    Need,
}

/// Parse `fuelpair=nearest|need`; absent is [`FuelpairMode::Nearest`].
///
/// # Errors
/// [`CandidateError::InvalidParameter`] for any other value.
pub(crate) fn parse_fuelpair(spec: &CandidateSpec) -> Result<FuelpairMode, CandidateError> {
    match spec.param("fuelpair") {
        None | Some("nearest") => Ok(FuelpairMode::Nearest),
        Some("need") => Ok(FuelpairMode::Need),
        Some(other) => Err(CandidateError::InvalidParameter {
            key: "fuelpair".to_owned(),
            value: other.to_owned(),
            reason: "expected nearest | need".to_owned(),
        }),
    }
}

/// The knobs [`AssignMode::apply`] threads through: which walker projects
/// the escort paths, which clock the projection plans in (Issue #304), and
/// how Supplies pair (Issue #252).
///
/// Bundled rather than added as further positional arguments — `apply`
/// already carries `walk` and `budget`, and a third would make the call
/// sites unreadable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssignOpts {
    /// Walker used for the escort projection *and* the final walk.
    pub walk: WalkMode,
    /// Clock the projection plans in (must match the caller's post-walk
    /// `budget.apply`).
    pub budget: BudgetMode,
    /// Supply-to-Patrol pairing rule.
    pub fuelpair: FuelpairMode,
}

impl AssignOpts {
    /// Legacy defaults — the element clock and nearest pairing every
    /// pre-#252 canonical id resolves to.
    #[must_use]
    pub fn element(walk: WalkMode) -> Self {
        Self { walk, budget: BudgetMode::Element, fuelpair: FuelpairMode::Nearest }
    }
}

/// Build one day's plan with the selected walk mode.
///
/// `WalkMode::Greedy` is exactly [`plan_walk`] (the dispatcher adds nothing
/// to it — pinned by `plan_walk_with_greedy_is_exactly_plan_walk`);
/// `WalkMode::Astar` is [`super::astar_walk::plan_walk_astar`]. Either way
/// the result is a well-shaped step-major plan.
///
/// `budget` is the candidate's [`BudgetMode`] — the A* walk plans its legs
/// against it (Issue #304) and the caller applies the same value to the
/// finished plan, so planning and truncation always share one clock.
#[must_use]
pub fn plan_walk_with(
    ctx: &SolverContext<'_>,
    targets: &[Vec<HexCoord>],
    mode: WalkMode,
    budget: BudgetMode,
) -> AgentActions {
    match mode {
        WalkMode::Greedy => plan_walk(ctx, targets),
        WalkMode::Astar => super::astar_walk::plan_walk_astar(ctx, targets, budget),
    }
}

/// Which controlled agents a tour candidate hands target spots to (the
/// `assign=` parameter, Issue #235).
///
/// Every tour candidate built before #235 gave **all** of its agents a tour,
/// including the Supply tankers — which cannot collect 玉 at all (Issue
/// #234), so the tour a Supply agent walks is pure waste and, worse, it is
/// planned from an agent that could instead be following a Patrol to refuel
/// it. `assign` makes that choice explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssignMode {
    /// `assign=agents` (default): every controlled agent gets a target list —
    /// the pre-#235 behaviour, byte for byte, so existing canonical ids keep
    /// both their shape and their meaning.
    #[default]
    Agents,
    /// `assign=patrol`: [`AgentKind::Supply`] agents get an empty target list
    /// and therefore wait the whole day ([`plan_walk`]'s documented empty-list
    /// behaviour). Patrol agents are untouched.
    ///
    /// Waiting in place is the minimal "stop wasting the tour" switch (Issue
    /// #235); [`Self::Escort`] is the following variant that puts the Supply
    /// back on the road.
    Patrol,
    /// `assign=escort` (Issue #248, spot-share handover Issue #250): each
    /// [`AgentKind::Supply`] gets the **projected day path of its paired
    /// Patrol** as its target list, so the two walk the same cells and the
    /// engine's same-cell refill rule tops the Patrol up mid-day — and before
    /// that projection runs, the Supply's **own spot share** (which the solver
    /// dealt it round-robin and which a Supply can never collect, Issue #234)
    /// is **appended to the paired Patrol's target list**, so the spots are
    /// not dropped on the floor. Under `P,S,P,S` that doubles the collectors'
    /// daily coverage from half the board to all of it.
    ///
    /// Pairing is deterministic: the Patrol whose day-start cell is nearest to
    /// the Supply ([`hex_distance`]), ties broken by the smaller position in
    /// `ctx.agent_ids`. With no Patrol in the team every Supply gets an empty
    /// list and waits — escort has nobody to follow. Patrol target lists are
    /// never *rewritten*: the handover only **appends** spots the Patrol does
    /// not already hold (own tour first, then each Supply's share, duplicates
    /// skipped), so the Patrol's own visit order stays a prefix.
    ///
    /// The projection is [`day_paths`] over `walk`, i.e. exactly the path the
    /// paired Patrol will walk in the adapter's own `plan_walk_with` call
    /// (both walkers plan every agent independently, so phase 1 reproduces the
    /// Patrol's phase-2 actions). How the Supply then consumes that list is
    /// the **walker's** semantics, and the two differ — this is a property of
    /// `walk=`, not of `assign=`:
    ///
    /// - `walk=greedy`: [`plan_walk`] resolves `list[step % len]` per step
    ///   (here `len == day_steps`), so at step `s` the Supply steps toward
    ///   the Patrol's position **after step `s`** — a pursuit that gains a
    ///   cell whenever the Patrol does not step directly away.
    /// - `walk=astar`: [`super::astar_walk`] consumes the list as **ordered
    ///   waypoints**, one A* leg per entry (Issue #215's anti-oscillation
    ///   contract) — a trail replay that walks `list[0]`, `list[1]`, … at the
    ///   Supply's own pace regardless of the current step index. The two
    ///   still share cells whenever the replayed trail crosses the Patrol
    ///   (typically while it parks at the end of its tour), but *not* by
    ///   tracking the Patrol's step-`s` position.
    ///
    /// Both modes are pinned by engine-level refill tests:
    /// [`assign_escort_greedy_walk_refuels_the_patrol_mid_day`] and
    /// [`assign_escort_astar_walk_refuels_the_patrol_mid_day`]. A Supply whose
    /// paired Patrol has no tour still parks on that Patrol's start cell.
    Escort,
    /// `assign=all` (Issue #253): every **Patrol** receives the **union of
    /// every agent's target list** — its own list first (the own tour stays a
    /// prefix), then the other agents' shares **rotated to start just after
    /// the Patrol's own index**, duplicates skipped — and every Supply
    /// follows the escort procedure on top of those unioned lists
    /// (pair → project → replace, [`Self::Escort`]). The rotation keeps the
    /// Patrols' tails from piling onto the same regions in the same order,
    /// which is what the first A/B showed as a kinds drop.
    ///
    /// Why: a region partition hands each Patrol only its own slice (on a
    /// 24-spot board roughly 8 spots/day per Patrol), which caps `total_balls`
    /// far below what live r5 teams collect — match 696 saw 108 vs r5's 320
    /// with `kinds` already tied. A Patrol that finishes its own region now
    /// inherits every other region's leftovers instead of parking, and a
    /// 1-spot region can no longer trap an agent on one cell (the match-692
    /// defect). Supplies still cannot collect (Issue #234), so their shares
    /// ride along exactly as [`Self::Escort`]'s handover would deliver them —
    /// here the union delivers *all* shares to *all* Patrols at once.
    All,
}

impl AssignMode {
    /// Resolve the Supply agents' target lists for this mode, in
    /// `ctx.agent_ids` order.
    ///
    /// Reads the kind through `ctx.state.agents[].kind`, which is the same
    /// visibility path every candidate has (Issue #235 Deliverable 2). When
    /// `self` is [`Self::Agents`] the vector is returned untouched, so the
    /// default path never re-allocates or re-orders anything.
    ///
    /// `opts.walk` selects the walker used to project the paired Patrol's
    /// path for [`Self::Escort`] and [`Self::All`]; the other modes ignore
    /// it (the caller still walks the returned lists with the same walker,
    /// so the two stay consistent). `opts.budget` rides along for the same
    /// reason: the projection and the final walk must be planned against
    /// the same clock (Issue #304), or an escorted Supply would follow a
    /// path its Patrol no longer walks. `opts.fuelpair` picks the pairing
    /// rule and its rendezvous extension (Issue #252).
    #[must_use]
    pub fn apply(
        self,
        ctx: &SolverContext<'_>,
        mut targets: Vec<Vec<HexCoord>>,
        opts: AssignOpts,
    ) -> Vec<Vec<HexCoord>> {
        let AssignOpts { walk, budget, fuelpair } = opts;
        match self {
            Self::Agents => targets,
            Self::Patrol => {
                for (index, id) in ctx.agent_ids.iter().enumerate() {
                    let is_supply =
                        ctx.state.agents.iter().any(|a| a.id == *id && a.kind == AgentKind::Supply);
                    if is_supply {
                        if let Some(list) = targets.get_mut(index) {
                            list.clear();
                        }
                    }
                }
                targets
            }
            Self::Escort => {
                // 1. Pair every Supply with its nearest Patrol (day-start
                //    geometry only, capacity-capped — Issue #250, see
                //    [`pair_supplies`]). Supplies nobody can escort wait.
                let (pairs, wait) = pair_supplies(ctx, fuelpair);
                for index in wait {
                    if let Some(list) = targets.get_mut(index) {
                        list.clear();
                    }
                }

                // 2. Handover (Issue #250): the Supply's own spot share — which
                //    a Supply can never collect (Issue #234) — is appended to
                //    the paired Patrol's list instead of being dropped. Own
                //    tour stays a prefix; duplicates are skipped.
                for &(supply, patrol) in &pairs {
                    let own = targets.get(supply).cloned().unwrap_or_default();
                    if let Some(patrol_list) = targets.get_mut(patrol) {
                        for spot in own {
                            if !patrol_list.contains(&spot) {
                                patrol_list.push(spot);
                            }
                        }
                    }
                }

                // 3. Project AFTER the handover so the Supply follows the
                //    extended path — the cells the Patrol will actually walk
                //    with the handed-over spots included.
                let paths = day_paths(ctx, &targets, walk, budget);

                // 4. Replace each paired Supply's list with the projection.
                for &(supply, patrol) in &pairs {
                    if let Some(list) = targets.get_mut(supply) {
                        *list = paths.get(patrol).cloned().unwrap_or_default();
                    }
                }
                // 5. Issue #252: under `fuelpair=need`, hand the Patrols no
                //    Supply was free for a rendezvous tail (Issue #252 現象2
                //    direction b) — now that the Supply lists *are* the
                //    projections, `paths` is exactly where each Supply ends.
                if fuelpair == FuelpairMode::Need {
                    append_rendezvous(ctx, &mut targets, &pairs, &paths);
                }
                targets
            }
            Self::All => {
                // Snapshot the pre-handover shares: the tail below must be
                // built from each agent's *original* list, not from a
                // neighbour's already-extended one.
                let original = targets.clone();
                // 1. Escort first (Issue #253): pair (same geometry, same
                //    capacity cap) and hand each paired Supply's share over
                //    — this reproduces `assign=escort`'s Patrol lists
                //    *exactly*, so they become the prefix below and kinds
                //    can never regress against escort under `walk=astar`
                //    (the ordered walk visits the prefix identically, then
                //    keeps going where escort would have parked).
                let (pairs, wait) = pair_supplies(ctx, fuelpair);
                for index in wait {
                    if let Some(list) = targets.get_mut(index) {
                        list.clear();
                    }
                }
                for &(supply, patrol) in &pairs {
                    let own = targets.get(supply).cloned().unwrap_or_default();
                    if let Some(patrol_list) = targets.get_mut(patrol) {
                        for spot in own {
                            if !patrol_list.contains(&spot) {
                                patrol_list.push(spot);
                            }
                        }
                    }
                }

                // 2. Extend every Patrol with **every other agent's share** —
                //    own/handed prefix kept, then the remaining shares
                //    **rotated to start just after the Patrol's own index**
                //    (`(index + 1 + k) % n`), duplicates skipped. A Supply's
                //    share is included even though a Supply can never collect
                //    it (Issue #234): under `assign=all` it is every Patrol,
                //    not just the paired one, that can reach those spots —
                //    this is what breaks the region-partition balls ceiling
                //    (live 696: 108 vs r5's 320) and the 1-spot-region park
                //    (live 692).
                //
                //    The rotation is load-bearing: the first A/B (agent
                //    order for everyone) made every Patrol walk the same tail
                //    in the same order, so they piled onto the early regions
                //    and later ones starved — kinds fell (665: 18 -> 13) even
                //    though balls rose. Rotating diverges the tails while the
                //    escort prefix stays common.
                let n = ctx.agent_ids.len();
                for (index, id) in ctx.agent_ids.iter().enumerate() {
                    if kind_of(ctx, *id) != Some(AgentKind::Patrol) {
                        continue;
                    }
                    let Some(list) = targets.get_mut(index) else {
                        continue;
                    };
                    let mut union: Vec<HexCoord> = list.clone();
                    for k in 1..n {
                        let other = (index + k) % n;
                        let Some(share) = original.get(other) else {
                            continue;
                        };
                        for spot in share {
                            if !union.contains(spot) {
                                union.push(*spot);
                            }
                        }
                    }
                    *list = union;
                }

                // 3. Project AFTER the extension so each Supply follows the
                //    final post-union path of its paired Patrol.
                let paths = day_paths(ctx, &targets, walk, budget);
                for &(supply, patrol) in &pairs {
                    if let Some(list) = targets.get_mut(supply) {
                        *list = paths.get(patrol).cloned().unwrap_or_default();
                    }
                }
                // 4. Same rendezvous extension as `escort` (Issue #252).
                if fuelpair == FuelpairMode::Need {
                    append_rendezvous(ctx, &mut targets, &pairs, &paths);
                }
                targets
            }
        }
    }
}

/// Pair every [`AgentKind::Supply`] with an escorting [`AgentKind::Patrol`] —
/// the shared phase-1 of [`AssignMode::Escort`] and [`AssignMode::All`].
///
/// Pairing uses day-start geometry only ([`hex_distance`], ties broken by the
/// smaller position in `ctx.agent_ids`) — no target lists are involved — and
/// is subject to a **capacity cap**: each Patrol escorts at most
/// `ceil(n_supply / n_patrol)` Supplies (Issue #250). Pure nearest-neighbour
/// can pile several Supplies onto one Patrol, leaving another Patrol
/// unescorted to starve at fuel=1 mid-match — and its handed-over spot share
/// piles onto the same over-walked Patrol. With the cap,
/// `n_supplies >= n_patrol` guarantees every Patrol gets an escort and the
/// shares spread across Patrols. (With `n_supply < n_patrol` — e.g. live
/// attempt8's `P,P,P,S` — capacity is 1 and the cap binds nothing: the
/// unescorted Patrols are a kinds-composition problem, fixed by fielding at
/// least one Supply per Patrol, e.g. `P,S`.)
///
/// Returns `(pairs, wait)`: `pairs` are `(supply index, patrol index)`, and
/// `wait` holds the indices of Supplies with no Patrol to escort (the caller
/// empties their lists so they wait, exactly like `assign=patrol`). A Supply
/// whose day-start cell is unknown appears in neither — it keeps its own
/// list, as `assign=escort` has always done.
///
/// `fuelpair` picks the selection key (Issue #252): [`FuelpairMode::Nearest`]
/// keeps the geometry-only rule above; [`FuelpairMode::Need`] ranks by the
/// Patrol's remaining fuel first — the tank about to strand wins the scarce
/// escorts — with distance demoted to a tie-break.
fn pair_supplies(
    ctx: &SolverContext<'_>,
    fuelpair: FuelpairMode,
) -> (Vec<(usize, usize)>, Vec<usize>) {
    let patrols: Vec<usize> = ctx
        .agent_ids
        .iter()
        .enumerate()
        .filter(|(_, id)| kind_of(ctx, **id) == Some(AgentKind::Patrol))
        .map(|(index, _)| index)
        .collect();
    let n_supplies =
        ctx.agent_ids.iter().filter(|id| kind_of(ctx, **id) == Some(AgentKind::Supply)).count();
    let capacity = if patrols.is_empty() { 0 } else { n_supplies.div_ceil(patrols.len()) };
    let mut escorting = vec![0usize; patrols.len()];
    let mut pairs: Vec<(usize, usize)> = Vec::new(); // (supply index, patrol index)
    let mut wait: Vec<usize> = Vec::new();
    for (index, id) in ctx.agent_ids.iter().enumerate() {
        if kind_of(ctx, *id) != Some(AgentKind::Supply) {
            continue;
        }
        let Some(supply_pos) = pos_of(ctx, *id) else {
            continue;
        };
        let Some(paired) = patrols
            .iter()
            .copied()
            .enumerate()
            .filter(|(slot, _)| escorting[*slot] < capacity)
            .map(|(_, patrol)| patrol)
            .min_by_key(|&patrol| {
                let patrol_pos = ctx.agent_ids.get(patrol).and_then(|pid| pos_of(ctx, *pid));
                let distance = patrol_pos.map_or(u32::MAX, |p| hex_distance(supply_pos, p));
                // Issue #252: `need` ranks by the Patrol's remaining fuel
                // first (lowest wins the scarce escort) and demotes distance
                // to a tie-break; `nearest` keeps its old ordering — the
                // constant leading component does not reorder it.
                let fuel_rank = match fuelpair {
                    FuelpairMode::Nearest => 0,
                    FuelpairMode::Need => ctx
                        .state
                        .agents
                        .iter()
                        .find(|a| ctx.agent_ids.get(patrol) == Some(&a.id))
                        .map_or(u32::MAX, |a| a.fuel),
                };
                (fuel_rank, distance, patrol)
            })
        else {
            // No Patrol to escort: wait, exactly like `assign=patrol`.
            wait.push(index);
            continue;
        };
        if let Some(slot) = patrols.iter().position(|&p| p == paired) {
            escorting[slot] += 1;
        }
        pairs.push((index, paired));
    }
    (pairs, wait)
}

/// Issue #252 (現象2, direction b): give every Patrol that no Supply was
/// free for a **rendezvous tail** — the end cell of the nearest such
/// Supply's projected day path.
///
/// Run *after* the paired Supply lists have been replaced by `paths`, so a
/// cell picked here is where a Supply will actually be standing (it parks
/// once the projection is consumed). The tail is appended to whatever the
/// Patrol already had, so its own tour keeps priority and the rendezvous is
/// only asked for with whatever day budget is left. Patrols that already
/// have an escort are skipped; `pairs` / `paths` are indexed exactly as
/// [`AssignMode::Escort`] builds them (agent positions in `ctx.agent_ids`).
fn append_rendezvous(
    ctx: &SolverContext<'_>,
    targets: &mut [Vec<HexCoord>],
    pairs: &[(usize, usize)],
    paths: &[Vec<HexCoord>],
) {
    let escorted: Vec<usize> = pairs.iter().map(|&(_, patrol)| patrol).collect();
    for (index, id) in ctx.agent_ids.iter().enumerate() {
        if kind_of(ctx, *id) != Some(AgentKind::Patrol) || escorted.contains(&index) {
            continue;
        }
        let Some(from) = pos_of(ctx, *id) else {
            continue;
        };
        // Nearest Supply end; ties resolve to the lower pair index, so the
        // choice is deterministic.
        let mut best: Option<(HexCoord, (u32, usize))> = None;
        for (pair, &(_, patrol)) in pairs.iter().enumerate() {
            let Some(end) = paths.get(patrol).and_then(|path| path.last()).copied() else {
                continue;
            };
            let key = (hex_distance(from, end), pair);
            if best.is_none_or(|(_, best_key)| key < best_key) {
                best = Some((end, key));
            }
        }
        let Some((cell, _)) = best else {
            continue;
        };
        if let Some(list) = targets.get_mut(index) {
            if list.last() != Some(&cell) {
                list.push(cell);
            }
        }
    }
}

/// Look up a controlled agent's kind by id through `ctx.state`.
fn kind_of(ctx: &SolverContext<'_>, id: u8) -> Option<AgentKind> {
    ctx.state.agents.iter().find(|a| a.id == id).map(|a| a.kind)
}

/// Look up a controlled agent's day-start cell by id through `ctx.state`.
fn pos_of(ctx: &SolverContext<'_>, id: u8) -> Option<HexCoord> {
    ctx.state.agents.iter().find(|a| a.id == id).map(|a| a.pos)
}

/// Every agent's projected day path: `paths[agent][step]` is the cell that
/// agent occupies after step `step` when walked with `walk` over `targets`.
///
/// Phase 1 of [`AssignMode::Escort`] — run the adapter's own walker once over
/// the unmodified target lists and reconstruct each agent's positions from its
/// action sequence (the same `Direction::step` fold [`plan_walk`] uses). Both
/// walkers plan every agent independently from its day-start cell, so these
/// paths equal the actions the adapter will emit for the Patrols in phase 2.
fn day_paths(
    ctx: &SolverContext<'_>,
    targets: &[Vec<HexCoord>],
    walk: WalkMode,
    budget: BudgetMode,
) -> Vec<Vec<HexCoord>> {
    let plan = plan_walk_with(ctx, targets, walk, budget);
    let per_agent = ctx.agent_ids.len();
    let day_steps = ctx.day_steps as usize;
    let mut positions: Vec<HexCoord> =
        ctx.agent_ids.iter().map(|id| pos_of(ctx, *id).unwrap_or(HexCoord::new(0, 0))).collect();
    let mut paths = vec![Vec::with_capacity(day_steps); per_agent];
    for step in 0..day_steps {
        for (index, _) in ctx.agent_ids.iter().enumerate() {
            let action = plan.actions.get(step * per_agent + index);
            if let Some(ActionKind::Move(dir)) = action.map(|a| a.kind) {
                if let Some(pos) = positions.get_mut(index) {
                    *pos = dir.step(*pos);
                }
            }
            if let Some(path) = paths.get_mut(index) {
                if let Some(pos) = positions.get(index) {
                    path.push(*pos);
                }
            }
        }
    }
    paths
}

/// Parse the `assign` parameter into an [`AssignMode`].
///
/// Omitted means [`AssignMode::Agents`]: every pre-#235 id keeps its exact
/// behaviour, and `assign=agents` is the explicit spelling of the same
/// default (the parameter is appended last in each factory's `PARAMS`, so
/// existing canonical ids keep their shape).
///
/// # Errors
/// [`CandidateError::InvalidParameter`] for any value other than
/// `agents` / `patrol` / `escort` / `all`.
pub(crate) fn parse_assign(spec: &CandidateSpec) -> Result<AssignMode, CandidateError> {
    match spec.param("assign") {
        None | Some("agents") => Ok(AssignMode::Agents),
        Some("patrol") => Ok(AssignMode::Patrol),
        Some("escort") => Ok(AssignMode::Escort),
        Some("all") => Ok(AssignMode::All),
        Some(other) => Err(CandidateError::InvalidParameter {
            key: "assign".to_owned(),
            value: other.to_owned(),
            reason: "expected agents | patrol | escort | all".to_owned(),
        }),
    }
}

/// Which clock a candidate plans its day against (the `budget=` parameter,
/// Issue #255).
///
/// The engine's day is `day_steps` **elements** (one action per step); the
/// official submission rule counts **weighted steps** (Issue #226,
/// `engine::wire_plan`): the sum of origin step costs plus wait runs must
/// equal `daySteps`. The wire boundary closes that gap today by truncating
/// the plan's tail, so what live executes is only the budget-fitting prefix
/// while offline evaluation runs the full element plan: the two disagree by
/// construction (live match 701: 98 moves submitted vs a 146-element plan;
/// offline dks 174 vs live 96).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum BudgetMode {
    /// `budget=element` (default): plan `day_steps` movement elements and
    /// hand the untouched plan to the wire — byte-identical to every
    /// pre-#255 candidate id, truncation stays a wire-boundary concern.
    #[default]
    Element,
    /// `budget=official`: truncate the finished plan to the official
    /// terrain-weighted budget **before** simulation, using the very same
    /// [`engine::wire_plan`] rule the wire applies. What `MatchSim` executes
    /// is
    /// then what the server would accept, and re-submitting it is a no-op
    /// (`normalize` keeps every move and adds no pad).
    Official,
}

impl BudgetMode {
    /// Apply this budget accounting to a finished day plan.
    ///
    /// [`Self::Element`] returns `plan` untouched (the default path never
    /// re-allocates). [`Self::Official`] re-runs the official wire
    /// normalisation over the plan's per-agent action lists and rebuilds a
    /// well-shaped step-major plan: the budget-fitting move prefix is kept,
    /// everything after the first over-budget or officially-illegal action
    /// becomes `Wait`, and the result is padded back to `day_steps` elements
    /// per agent (the trailing waits are state-neutral idle, not budget).
    ///
    /// A plan that is not well-shaped (the candidate broke the day contract)
    /// is returned unchanged so the evaluator's malformed-plan flag stays the
    /// one that reports it.
    #[must_use]
    pub fn apply(self, ctx: &SolverContext<'_>, plan: AgentActions) -> AgentActions {
        match self {
            Self::Element => plan,
            Self::Official => truncate_to_official_budget(ctx, plan),
        }
    }
}

/// Re-encode a step-major plan as per-agent wire bodies, run
/// [`engine::wire_plan::normalize_day_plan`], and expand the result back into
/// a well-shaped plan (single source of truth for the budget rule: Issue
/// #255).
fn truncate_to_official_budget(ctx: &SolverContext<'_>, plan: AgentActions) -> AgentActions {
    let agent_count = ctx.agent_ids.len();
    let day_steps = ctx.day_steps as usize;
    if agent_count == 0 || day_steps == 0 || plan.actions.len() != agent_count * day_steps {
        return plan; // malformed: the evaluator's shape flag reports it
    }

    // Step-major -> per-agent wire bodies (run-length encoded waits), the
    // exact encoding `cli::action_adapter::decision_to_actions` builds.
    let mut body: Vec<Vec<i32>> = Vec::with_capacity(agent_count);
    for (index, _) in ctx.agent_ids.iter().enumerate() {
        let mut codes: Vec<i32> = Vec::with_capacity(day_steps);
        let mut wait_run = 0i32;
        for step in 0..day_steps {
            match plan.actions[step * agent_count + index].kind {
                ActionKind::Wait => wait_run += 1,
                ActionKind::Move(dir) => {
                    if wait_run > 0 {
                        codes.push(-wait_run);
                        wait_run = 0;
                    }
                    codes.push(dir as i32);
                }
            }
        }
        if wait_run > 0 {
            codes.push(-wait_run);
        }
        body.push(codes);
    }

    // The official rule: keep the prefix that fits the weighted budget and is
    // legal, drop the rest (Issue #226). Failure here means the input was not
    // a submittable list at all — fall back to the plan as written.
    let Ok(normalized) = engine::wire_plan::normalize_day_plan(
        &ctx.state.setting.map,
        &ctx.state.agents,
        &ctx.agent_ids,
        ctx.day_steps,
        &body,
    ) else {
        return plan;
    };

    // Wire bodies -> per-agent element sequences, padded with Wait to the
    // engine's day contract (one action per agent per step).
    let mut per_agent: Vec<Vec<ActionKind>> = Vec::with_capacity(agent_count);
    for codes in &normalized {
        let mut flat: Vec<ActionKind> = Vec::with_capacity(day_steps);
        for &code in codes {
            if code <= -1 {
                for _ in 0..code.unsigned_abs() {
                    flat.push(ActionKind::Wait);
                }
            } else if let Some(dir) =
                Direction::ALL.get(usize::try_from(code).unwrap_or(usize::MAX))
            {
                flat.push(ActionKind::Move(*dir));
            } else {
                break; // unknown code: normalize never emits one; stop short
            }
            if flat.len() >= day_steps {
                break;
            }
        }
        while flat.len() < day_steps {
            flat.push(ActionKind::Wait);
        }
        per_agent.push(flat);
    }

    // Back to step-major.
    let mut out = Vec::with_capacity(agent_count * day_steps);
    for step in 0..day_steps {
        for (id, kinds) in ctx.agent_ids.iter().zip(&per_agent) {
            out.push(Action { agent_id: *id, kind: kinds[step] });
        }
    }
    AgentActions::new(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{MatchId, PlayerId};
    use engine::model::{Agent, MapSize, MatchSetting, MatchState, Spot, Terrain};

    fn map_with_spots() -> MapGrid {
        let size = MapSize::new(8, 8).unwrap();
        let mut cells = Vec::new();
        for row in 0..8 {
            for col in 0..8 {
                let spot = if (col, row) == (2, 0) {
                    Some(Spot {
                        series: 1,
                        pos: HexCoord::new(2, 0),
                        max_stock: 3,
                        current_stock: 3,
                    })
                } else {
                    None
                };
                cells.push(engine::model::Cell {
                    coord: HexCoord::new(col, row),
                    terrain: Terrain::Plain,
                    spot,
                });
            }
        }
        MapGrid::new(size, cells).unwrap()
    }

    fn state() -> MatchState {
        let map = map_with_spots();
        let mut agents = vec![Agent::patrol(0, HexCoord::new(0, 0), 4, 3)];
        while agents.len() < 3 {
            let id = u8::try_from(agents.len()).unwrap_or(u8::MAX);
            agents.push(Agent::supply(id, HexCoord::new(7, 7)));
        }
        MatchState::new(MatchSetting {
            match_id: MatchId::new("walk-test").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents,
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        })
        .unwrap()
    }

    #[test]
    fn step_towards_picks_the_legal_adjacent_direction() {
        let map = map_with_spots();
        // Adjacent target: one step reaches it.
        assert_eq!(
            step_towards(HexCoord::new(0, 0), HexCoord::new(1, 0), &map),
            Some(Direction::Right)
        );
        // Distant target: the step that gets closer, repeated across steps.
        assert_eq!(
            step_towards(HexCoord::new(0, 0), HexCoord::new(2, 0), &map),
            Some(Direction::Right)
        );
        // Backwards: moving away from the target is never chosen.
        assert_eq!(step_towards(HexCoord::new(2, 0), HexCoord::new(2, 0), &map), None);
    }

    #[test]
    fn step_towards_is_none_when_already_there() {
        let map = map_with_spots();
        assert_eq!(step_towards(HexCoord::new(2, 0), HexCoord::new(2, 0), &map), None);
    }

    #[test]
    fn hex_distance_matches_the_engine_neighbour_graph() {
        // Every engine step is exactly one cell away, whatever the row
        // parity — this is what the old axial formula got wrong.
        for row in 0..8 {
            for col in 0..8 {
                let from = HexCoord::new(col, row);
                assert_eq!(hex_distance(from, from), 0, "self distance at {from:?}");
                for dir in Direction::ALL {
                    assert_eq!(
                        hex_distance(from, dir.step(from)),
                        1,
                        "{dir:?} out of {from:?} must be one cell"
                    );
                }
            }
        }
        // A direct neighbour the axial formula reported as two steps.
        assert_eq!(hex_distance(HexCoord::new(0, 0), HexCoord::new(1, 1)), 1);
        // A two-step pair the axial formula reported as one.
        assert_eq!(hex_distance(HexCoord::new(1, 0), HexCoord::new(0, 1)), 2);
    }

    #[test]
    fn step_towards_reaches_a_target_two_rows_away() {
        let map = map_with_spots();
        // Under the axial metric no neighbour of (1, 0) scored strictly below
        // (0, 1), so this returned None and plan_walk emitted Wait all day.
        assert_eq!(
            step_towards(HexCoord::new(1, 0), HexCoord::new(0, 1), &map),
            Some(Direction::LowerLeft)
        );
        assert_eq!(
            step_towards(HexCoord::new(0, 0), HexCoord::new(1, 1), &map),
            Some(Direction::LowerRight)
        );
    }

    #[test]
    fn plan_walk_is_well_shaped_and_moves_the_agent() {
        let st = state();
        let ctx = SolverContext::from_state(&st);
        // Agent 0 starts at (0, 0) and heads for the spot at (2, 0): two
        // Right steps get it there, then it holds position.
        let targets = vec![vec![HexCoord::new(2, 0)], vec![], vec![]];
        let plan = plan_walk(&ctx, &targets);
        assert_eq!(plan.actions.len(), 3 * 4, "one action per agent per step");
        assert!(
            plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps),
            "plan must be acceptable to MatchSim"
        );

        let agent0: Vec<_> = plan.actions.iter().step_by(3).map(|a| a.kind).collect();
        assert_eq!(agent0.len(), 4);
        assert!(
            matches!(agent0[0], engine::model::ActionKind::Move(Direction::Right))
                && matches!(agent0[1], engine::model::ActionKind::Move(Direction::Right)),
            "first two steps walk toward the target, got {agent0:?}"
        );
        assert!(
            matches!(agent0[2], engine::model::ActionKind::Wait)
                && matches!(agent0[3], engine::model::ActionKind::Wait),
            "once arrived the agent holds position, got {agent0:?}"
        );
    }

    /// Issue #215 — `walk` is a schema-gated value: omitted and `greedy`
    /// select today's [`plan_walk`], `astar` selects the ordered-tour walk,
    /// anything else is an `InvalidParameter` at parse / build time.
    #[test]
    fn walk_param_parses_greedy_and_astar_and_rejects_anything_else() {
        let parse = |id: &str| parse_walk(&CandidateSpec::parse(id).expect("id parses"));
        assert_eq!(parse("loop"), Ok(WalkMode::Greedy));
        assert_eq!(parse("loop:walk=greedy"), Ok(WalkMode::Greedy));
        assert_eq!(parse("loop:walk=astar"), Ok(WalkMode::Astar));
        assert_eq!(parse("multiloop:kmeans=4:seed=42:walk=astar"), Ok(WalkMode::Astar));
        assert_eq!(parse("loop"), Ok(WalkMode::default()), "the default mode is greedy");
        for id in ["loop:walk=Astar", "loop:walk=Greedy", "loop:walk=smart", "loop:walk=1"] {
            assert!(
                matches!(parse(id), Err(CandidateError::InvalidParameter { key, .. }) if key == "walk"),
                "{id} must be rejected as an invalid walk value"
            );
        }
    }

    /// `assign` is a schema-gated value: omitted and `agents` select the
    /// all-agents assignment, `patrol` withholds the tour from Supply agents
    /// (Issue #235), `escort` walks each Supply along its paired Patrol's
    /// projected path (Issue #248), `all` unions every agent's share into
    /// every Patrol (Issue #253), anything else is an `InvalidParameter`.
    #[test]
    fn assign_param_parses_agents_patrol_escort_and_rejects_anything_else() {
        let parse = |id: &str| parse_assign(&CandidateSpec::parse(id).expect("id parses"));
        assert_eq!(parse("loop"), Ok(AssignMode::Agents));
        assert_eq!(parse("loop:assign=agents"), Ok(AssignMode::Agents));
        assert_eq!(parse("loop:assign=patrol"), Ok(AssignMode::Patrol));
        assert_eq!(parse("loop:assign=escort"), Ok(AssignMode::Escort));
        assert_eq!(parse("loop:assign=all"), Ok(AssignMode::All));
        assert_eq!(parse("multiloop:kmeans=4:seed=42:walk=astar:assign=all"), Ok(AssignMode::All));
        assert_eq!(
            parse("multiloop:kmeans=4:seed=42:walk=astar:assign=patrol"),
            Ok(AssignMode::Patrol)
        );
        assert_eq!(
            parse("multiloop:kmeans=4:seed=42:walk=astar:assign=escort"),
            Ok(AssignMode::Escort)
        );
        assert_eq!(parse("loop"), Ok(AssignMode::default()), "the default is agents");
        for id in ["loop:assign=Patrol", "loop:assign=Escort", "loop:assign=1", "loop:assign=tour"]
        {
            assert!(
                matches!(parse(id), Err(CandidateError::InvalidParameter { key, .. }) if key == "assign"),
                "{id} must be rejected as an invalid assign value"
            );
        }
    }

    /// Issue #235 AC — `assign=patrol` is the "Supply gets no spot tour"
    /// switch: it reads the kind through `SolverContext` and empties exactly
    /// those agents' target lists. `assign=agents` must not touch anything.
    #[test]
    fn assign_patrol_withholds_tours_from_supply_agents_only() {
        let st = state();
        let ctx = SolverContext::from_state(&st);
        // The fixture state is [Patrol, Supply, Supply].
        assert_eq!(ctx.state.agents[0].kind, AgentKind::Patrol);
        assert_eq!(ctx.state.agents[1].kind, AgentKind::Supply);
        assert_eq!(ctx.state.agents[2].kind, AgentKind::Supply);

        let targets =
            vec![vec![HexCoord::new(2, 0)], vec![HexCoord::new(3, 3)], vec![HexCoord::new(4, 4)]];

        let untouched =
            AssignMode::Agents.apply(&ctx, targets.clone(), AssignOpts::element(WalkMode::Greedy));
        assert_eq!(untouched, targets, "assign=agents must be a no-op");

        let filtered =
            AssignMode::Patrol.apply(&ctx, targets, AssignOpts::element(WalkMode::Greedy));
        assert_eq!(
            filtered,
            vec![vec![HexCoord::new(2, 0)], Vec::new(), Vec::new()],
            "the patrol keeps its tour; both supply agents wait"
        );
    }

    /// Issue #248 AC — pairing: each Supply receives the projected day path
    /// of the Patrol whose day-start cell is nearest to it (ties by smaller
    /// `agent_ids` index), while the Patrols' own lists stay untouched.
    #[test]
    fn assign_escort_pairs_each_supply_with_its_nearest_patrol() {
        let map = map_with_spots();
        let st = MatchState::new(MatchSetting {
            match_id: MatchId::new("escort-pair").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents: vec![
                Agent::patrol(0, HexCoord::new(0, 0), 4, 3),
                Agent::patrol(1, HexCoord::new(0, 7), 4, 3),
                Agent::supply(2, HexCoord::new(1, 0)),
                Agent::supply(3, HexCoord::new(1, 6)),
            ],
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        })
        .unwrap();
        let ctx = SolverContext::from_state(&st);
        let targets = vec![
            vec![HexCoord::new(2, 0)],
            vec![HexCoord::new(5, 7)],
            vec![HexCoord::new(4, 4)],
            vec![HexCoord::new(3, 3)],
        ];
        // The pairing keys off hex distance: (1, 0) is next to patrol0 and
        // (1, 6) is closest to patrol1 — the own-tour targets must not decide
        // it. Distances are even-r steps, so the cross-row pairs are two and
        // six cells apart rather than the axial formula's one and seven.
        assert_eq!(hex_distance(HexCoord::new(1, 0), HexCoord::new(0, 0)), 1);
        assert_eq!(hex_distance(HexCoord::new(1, 0), HexCoord::new(0, 7)), 7);
        assert_eq!(hex_distance(HexCoord::new(1, 6), HexCoord::new(0, 7)), 2);
        assert_eq!(hex_distance(HexCoord::new(1, 6), HexCoord::new(0, 0)), 6);

        let paths = day_paths(&ctx, &targets, WalkMode::Greedy, BudgetMode::Element);
        assert_ne!(paths[0], paths[1], "the two patrols must walk different paths");

        let escorted =
            AssignMode::Escort.apply(&ctx, targets.clone(), AssignOpts::element(WalkMode::Greedy));
        // Handover (Issue #250): each paired Supply's own share is appended to
        // the Patrol list — the own tour stays a prefix, nothing is dropped.
        assert_eq!(
            escorted[0],
            vec![HexCoord::new(2, 0), HexCoord::new(4, 4)],
            "patrol0 keeps its own tour and gains supply2's handed-over share"
        );
        assert_eq!(
            escorted[1],
            vec![HexCoord::new(5, 7), HexCoord::new(3, 3)],
            "patrol1 keeps its own tour and gains supply3's handed-over share"
        );
        // The projection runs AFTER the handover, so the supplies follow the
        // extended paths, not the pre-handover ones.
        let extended = day_paths(&ctx, &escorted, WalkMode::Greedy, BudgetMode::Element);
        assert_eq!(
            escorted[2], extended[0],
            "supply2 pairs with the nearest patrol (patrol0) and walks its extended path"
        );
        assert_eq!(
            escorted[3], extended[1],
            "supply3 pairs with the nearest patrol (patrol1) and walks its extended path"
        );
        assert_ne!(
            escorted[2], paths[0],
            "the Supply follows the POST-handover projection, not the pre-handover one"
        );
        assert_ne!(escorted[2], targets[2], "the escort replaces the Supply's own tour");
    }

    /// Issue #248 — with no Patrol in the team there is nobody to escort, so
    /// every Supply waits: the same empty-list contract as `assign=patrol`.
    #[test]
    fn assign_escort_without_a_patrol_makes_supplies_wait() {
        let map = map_with_spots();
        let st = MatchState::new(MatchSetting {
            match_id: MatchId::new("escort-solo").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents: vec![
                Agent::supply(0, HexCoord::new(0, 0)),
                Agent::supply(1, HexCoord::new(3, 3)),
                Agent::supply(2, HexCoord::new(7, 7)),
            ],
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        })
        .unwrap();
        let ctx = SolverContext::from_state(&st);
        let targets = vec![vec![HexCoord::new(2, 0)], vec![HexCoord::new(5, 5)], vec![]];
        let escorted =
            AssignMode::Escort.apply(&ctx, targets, AssignOpts::element(WalkMode::Greedy));
        assert!(
            escorted.iter().all(Vec::is_empty),
            "no Patrol to follow, so every Supply waits, got {escorted:?}"
        );
    }

    /// Issue #250 — capacity-capped pairing: with `n_supply >= n_patrol` every
    /// Patrol must get an escort even when all Supplies are nearest to the
    /// same Patrol (pure nearest-neighbour would pile them onto patrol0 and
    /// leave patrol1 unescorted to starve at fuel=1).
    #[test]
    fn assign_escort_spreads_supplies_across_patrols_when_nearest_piles_up() {
        let map = map_with_spots();
        let st = MatchState::new(MatchSetting {
            match_id: MatchId::new("escort-capacity").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents: vec![
                Agent::patrol(0, HexCoord::new(0, 0), 4, 3),
                Agent::patrol(1, HexCoord::new(0, 7), 4, 3),
                // Both Supplies sit next to patrol0 — nearest-neighbour alone
                // would pair both with it.
                Agent::supply(2, HexCoord::new(1, 0)),
                Agent::supply(3, HexCoord::new(1, 1)),
            ],
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        })
        .unwrap();
        let ctx = SolverContext::from_state(&st);
        // Both Supplies are geometrically nearest to patrol0 …
        assert_eq!(hex_distance(HexCoord::new(1, 0), HexCoord::new(0, 0)), 1);
        assert_eq!(hex_distance(HexCoord::new(1, 1), HexCoord::new(0, 0)), 1);
        assert!(
            hex_distance(HexCoord::new(1, 1), HexCoord::new(0, 0))
                < hex_distance(HexCoord::new(1, 1), HexCoord::new(0, 7)),
            "supply3 must prefer patrol0 for the capacity cap to matter"
        );
        let targets = vec![
            vec![HexCoord::new(2, 0)],
            vec![HexCoord::new(5, 7)],
            vec![HexCoord::new(4, 4)],
            vec![HexCoord::new(3, 3)],
        ];
        let escorted =
            AssignMode::Escort.apply(&ctx, targets, AssignOpts::element(WalkMode::Greedy));

        // capacity = ceil(2 supplies / 2 patrols) = 1: the first Supply takes
        // patrol0's only slot, so the second is forced onto patrol1.
        assert_eq!(
            escorted[0],
            vec![HexCoord::new(2, 0), HexCoord::new(4, 4)],
            "patrol0 gains supply2's share (its own tour stays a prefix)"
        );
        assert_eq!(
            escorted[1],
            vec![HexCoord::new(5, 7), HexCoord::new(3, 3)],
            "patrol1 gains supply3's share — forced by the capacity cap, not by distance"
        );
        let extended = day_paths(&ctx, &escorted, WalkMode::Greedy, BudgetMode::Element);
        assert_eq!(
            escorted[3], extended[1],
            "supply3 escorts patrol1 despite patrol0 being nearer"
        );
        assert_ne!(escorted[3], extended[0], "capacity cap must beat pure nearest-neighbour here");
    }

    /// Issue #252 (現象2, directions a+b): `fuelpair=need` ranks pairing by
    /// the Patrol's remaining fuel first — the tank closest to stranding
    /// wins the scarce escort even when another Patrol is nearer — and hands
    /// every unescorted Patrol a rendezvous tail. The default `nearest`
    /// keeps the old geometry-only rule and adds no tail (byte-identical).
    #[test]
    fn fuelpair_need_pairs_by_fuel_and_leaves_a_rendezvous_tail() {
        let map = map_with_spots();
        let st = MatchState::new(MatchSetting {
            match_id: MatchId::new("fuelpair-need").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents: vec![
                // Nearest to both Supplies, but a full tank …
                Agent::patrol_with_fuel(0, HexCoord::new(0, 0), 100),
                // … while these two are far away and running dry.
                Agent::patrol_with_fuel(1, HexCoord::new(0, 7), 50),
                Agent::patrol_with_fuel(2, HexCoord::new(7, 7), 10),
                Agent::supply(3, HexCoord::new(1, 0)),
                Agent::supply(4, HexCoord::new(1, 1)),
            ],
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        })
        .unwrap();
        let ctx = SolverContext::from_state(&st);
        let targets = vec![
            vec![HexCoord::new(2, 0)],
            vec![HexCoord::new(5, 7)],
            vec![HexCoord::new(4, 4)],
            vec![HexCoord::new(4, 4)],
            vec![HexCoord::new(3, 3)],
        ];

        // Default: geometry only — the nearest Supply takes patrol0's slot.
        let (nearest, _) = pair_supplies(&ctx, FuelpairMode::Nearest);
        assert_eq!(nearest[0], (3, 0), "nearest keeps supply3 -> patrol0");

        // `need`: fuel ascending — patrol2 (10) first, patrol1 (50) second;
        // the full-tank patrol0, nearest of all, gets nothing.
        let (pairs, wait) = pair_supplies(&ctx, FuelpairMode::Need);
        assert!(wait.is_empty(), "every Supply still pairs: {wait:?}");
        let paired_patrols: Vec<usize> = pairs.iter().map(|&(_, patrol)| patrol).collect();
        assert_eq!(paired_patrols, vec![2, 1], "lowest-fuel Patrols first: {pairs:?}");

        // … so the unescorted patrol0 gains exactly one rendezvous cell
        // under `need`, appended after its own tour.
        let need = AssignOpts {
            walk: WalkMode::Greedy,
            budget: BudgetMode::Element,
            fuelpair: FuelpairMode::Need,
        };
        let tailed = AssignMode::Escort.apply(&ctx, targets.clone(), need);
        assert_eq!(&tailed[0][..targets[0].len()], targets[0], "own tour stays a prefix");
        assert_eq!(
            tailed[0].len(),
            targets[0].len() + 1,
            "exactly one rendezvous cell, got {:?}",
            tailed[0]
        );

        // Default: patrol0 *is* the nearest pairing, so it gains supply3's
        // share by handover — and nothing else: no rendezvous tail.
        let plain =
            AssignMode::Escort.apply(&ctx, targets.clone(), AssignOpts::element(WalkMode::Greedy));
        assert_eq!(
            plain[0],
            vec![HexCoord::new(2, 0), HexCoord::new(4, 4)],
            "`nearest` adds only the handover, never a rendezvous tail"
        );
        assert!(
            plain[1].len() <= targets[1].len() + targets[4].len(),
            "escorted lists only ever gain their handover share: {:?}",
            plain[1]
        );
    }

    /// Issue #248 AC — the engine end of the contract under **`walk=greedy`**
    /// (the default): `assign=escort` puts the Supply on the paired Patrol's
    /// cell mid-day and the official same-cell refill rule fires (`Picked`
    /// with the `u32::MAX` sentinel, tank back to cap). The `assign=patrol`
    /// control never meets the Patrol, so it never refuels.
    ///
    /// `walk=astar` has its own proof in
    /// [`assign_escort_astar_walk_refuels_the_patrol_mid_day`] — the two
    /// walks consume the projected path differently, so neither test
    /// substitutes for the other.
    #[test]
    fn assign_escort_greedy_walk_refuels_the_patrol_mid_day() {
        let map = map_with_spots();
        let setting = MatchSetting {
            match_id: MatchId::new("escort-refuel").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents: vec![
                // Patrol walks (0,0) -> (1,0) -> (2,0) and parks on the spot.
                Agent::patrol(0, HexCoord::new(0, 0), 4, 3),
                // …and the Supply starts one cell below the first step, so
                // escorting it onto the projected path meets the Patrol at
                // step 0 — while its own tour would wander elsewhere.
                Agent::supply(1, HexCoord::new(1, 1)),
                // A second Supply too far away to arrive within the day: it
                // proves pairing is per-agent, not a global overwrite.
                Agent::supply(2, HexCoord::new(7, 7)),
            ],
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        };
        let state = MatchState::new(setting.clone()).unwrap();
        let ctx = SolverContext::from_state(&state);
        let targets =
            vec![vec![HexCoord::new(2, 0)], vec![HexCoord::new(4, 4)], vec![HexCoord::new(5, 5)]];

        let is_refuel = |e: &engine::transitions::StepEvent| matches!(e, engine::transitions::StepEvent::Picked { series, .. } if *series == u32::MAX);
        let run = |mode: AssignMode| -> Option<u32> {
            let plan = plan_walk(
                &ctx,
                &mode.apply(&ctx, targets.clone(), AssignOpts::element(WalkMode::Greedy)),
            );
            assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps));
            let mut run_state = MatchState::new(setting.clone()).unwrap();
            let sim = engine::MatchSim::default();
            let mut fuel_at_refuel = None;
            for block in plan.actions.chunks(ctx.agent_ids.len()) {
                let events = sim.step(&mut run_state, &AgentActions::new(block.to_vec()));
                if fuel_at_refuel.is_none() && events.iter().any(is_refuel) {
                    let patrol =
                        run_state.agents.iter().find(|a| a.id == 0).expect("patrol exists");
                    fuel_at_refuel = Some(patrol.fuel);
                }
            }
            fuel_at_refuel
        };

        let fuel_at_refuel = run(AssignMode::Escort);
        let fuel = fuel_at_refuel.expect("escort must produce a same-cell refill event");
        // `Agent::patrol(_, _, 4, 3)` = day_steps × multiplier = 12: the refill
        // tops the tank to cap *at the moment it fires*. The Patrol then keeps
        // walking its (handed-over, Issue #250) tour, so day-end fuel may be
        // below cap — that is the engine rule, not a weaker escort.
        assert_eq!(fuel, 12, "the refill must top the tank to cap when it fires");

        let waiting = run(AssignMode::Patrol);
        assert!(waiting.is_none(), "assign=patrol must never meet the patrol, so no refill");

        // Issue #253 — `assign=all` runs the same escort procedure (pair →
        // project → replace) on top of the unioned lists, so the rendezvous
        // and its refill must survive the switch.
        let all = run(AssignMode::All)
            .expect("assign=all keeps the escort projection, so the refill must still fire");
        assert_eq!(all, 12, "assign=all must refill exactly as escort does");
    }

    /// PR #249 review AC — the live-favoured `walk=astar` needs its own
    /// engine-level proof: escort under A* still puts the Supply on the
    /// Patrol's cell and the official same-cell refill fires.
    ///
    /// The same test pins what A* escort *means*, because the cell sequence
    /// rules out time-indexed pursuit: `plan_walk_astar` consumes the
    /// projected path as an **ordered waypoint list** (Issue #215), so the
    /// Supply walks leg 0 from (4,0) straight to `list[0]=(1,0)` — *passing
    /// through* the Patrol's park cell (2,0) at step 1 (the meeting), then
    /// continuing to (1,0) and walking back. A chaser aiming at `list[step]`
    /// would never leave the parked Patrol, so `[.., (1,0), ..]` at step 2 is
    /// the observable difference between trail replay and pursuit.
    #[test]
    fn assign_escort_astar_walk_refuels_the_patrol_mid_day() {
        let map = map_with_spots();
        let setting = MatchSetting {
            match_id: MatchId::new("escort-refuel-astar").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents: vec![
                // Patrol walks (0,0) -> (1,0) -> (2,0) and parks on the spot.
                Agent::patrol(0, HexCoord::new(0, 0), 4, 3),
                // The Supply starts *beyond* the park cell, so its first A*
                // leg (4,0) -> (1,0) necessarily passes through (2,0) — the
                // rendezvous — and then overshoots to the waypoint itself.
                Agent::supply(1, HexCoord::new(4, 0)),
                // Too far to arrive within the day: pairing stays per-agent.
                Agent::supply(2, HexCoord::new(7, 7)),
            ],
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        };
        let state = MatchState::new(setting.clone()).unwrap();
        let ctx = SolverContext::from_state(&state);
        let targets =
            vec![vec![HexCoord::new(2, 0)], vec![HexCoord::new(4, 4)], vec![HexCoord::new(5, 5)]];

        // Walk supply1's cells under each mode: fold its Move actions.
        let supply1_cells = |walk: WalkMode| -> Vec<HexCoord> {
            let plan = plan_walk_with(
                &ctx,
                &AssignMode::Escort.apply(&ctx, targets.clone(), AssignOpts::element(walk)),
                walk,
                BudgetMode::Element,
            );
            assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps));
            let mut pos = HexCoord::new(4, 0);
            let mut cells = vec![pos];
            for block in plan.actions.chunks(ctx.agent_ids.len()) {
                if let ActionKind::Move(dir) = block[1].kind {
                    pos = dir.step(pos);
                }
                cells.push(pos);
            }
            cells
        };

        // The semantics pin: A* replays the list in order (overshoots the
        // park cell to reach waypoint 0), greedy pursues time-indexed
        // positions. Deterministic: the map is plain, so both A* legs are
        // straight lines along row 0. (The patrol's tour now also carries the
        // handed-over shares, Issue #250 — hence the projection is longer than
        // the pre-handover one, but the overshoot-to-list[0] signature stands.)
        assert_eq!(
            supply1_cells(WalkMode::Astar),
            vec![
                HexCoord::new(4, 0),
                HexCoord::new(3, 0),
                HexCoord::new(2, 0),
                HexCoord::new(1, 0),
                HexCoord::new(2, 0)
            ],
            "walk=astar must consume the projected path as ordered waypoints: pass the park \
             cell on leg 0, reach list[0], then walk back"
        );
        assert_eq!(
            supply1_cells(WalkMode::Greedy),
            vec![
                HexCoord::new(4, 0),
                HexCoord::new(3, 0),
                HexCoord::new(2, 0),
                HexCoord::new(3, 1),
                HexCoord::new(2, 0)
            ],
            "walk=greedy aims at list[step] instead: it never overshoots to list[0]=(1,0)"
        );

        // The engine proof: step the A* plan through MatchSim and watch the
        // official refill sentinel; the patrol-assign control never meets.
        let is_refuel = |e: &engine::transitions::StepEvent| matches!(e, engine::transitions::StepEvent::Picked { series, .. } if *series == u32::MAX);
        let run = |mode: AssignMode| -> Option<u32> {
            let plan = plan_walk_with(
                &ctx,
                &mode.apply(&ctx, targets.clone(), AssignOpts::element(WalkMode::Astar)),
                WalkMode::Astar,
                BudgetMode::Element,
            );
            assert!(plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps));
            let mut run_state = MatchState::new(setting.clone()).unwrap();
            let sim = engine::MatchSim::default();
            let mut fuel_at_refuel = None;
            for block in plan.actions.chunks(ctx.agent_ids.len()) {
                let events = sim.step(&mut run_state, &AgentActions::new(block.to_vec()));
                if fuel_at_refuel.is_none() && events.iter().any(is_refuel) {
                    let patrol =
                        run_state.agents.iter().find(|a| a.id == 0).expect("patrol exists");
                    fuel_at_refuel = Some(patrol.fuel);
                }
            }
            fuel_at_refuel
        };

        let fuel = run(AssignMode::Escort)
            .expect("walk=astar escort must produce a same-cell refill event");
        // `Agent::patrol(_, _, 4, 3)` = day_steps × multiplier = 12: the refill
        // tops the tank to cap *at the moment it fires*; the Patrol keeps
        // walking its handed-over tour afterwards (Issue #250), so day-end fuel
        // is not asserted here.
        assert_eq!(fuel, 12, "the refill must top the tank to cap when it fires");

        let waiting = run(AssignMode::Patrol);
        assert!(
            waiting.is_none(),
            "assign=patrol must never meet the patrol under walk=astar either"
        );
    }

    /// Issue #253 AC — under `assign=all` every Patrol's target list is the
    /// **union of every agent's share**: its own list first (the own tour
    /// stays a prefix), then the other agents in `ctx.agent_ids` order,
    /// duplicates skipped. Checked on the live kinds compositions (4a / 5a /
    /// 7a), because those are the shapes the arena fields.
    ///
    /// Supplies are not unioned — they still follow the escort projection of
    /// their paired Patrol (asserted here through the shared [`pair_supplies`]
    /// so the test pins *this* mode's pairing, not a re-implementation).
    #[test]
    fn assign_all_unions_every_patrol_list_for_live_kinds() {
        for pattern in ["P,S,P,S", "P,S,S,P,S", "P,S,P,S,P,S,P"] {
            let kinds = crate::kinds::parse_kinds(pattern).expect("pattern parses");
            let map = map_with_spots();
            let agents = kinds
                .iter()
                .enumerate()
                .map(|(i, kind)| {
                    let id = u8::try_from(i).unwrap_or(u8::MAX);
                    // Spread along row 1 so pairing distances stay distinct
                    // and nobody starts on the (2, 0) spot cell.
                    let pos = HexCoord::new(i32::try_from(i).unwrap_or(0), 1);
                    match kind {
                        AgentKind::Patrol => Agent::patrol(id, pos, 4, 3),
                        AgentKind::Supply => Agent::supply(id, pos),
                    }
                })
                .collect();
            let st = MatchState::new(MatchSetting {
                match_id: MatchId::new("assign-all").unwrap(),
                team: PlayerId::new("player-a").unwrap(),
                map,
                agents,
                day_steps: 4,
                num_days: 1,
                day_steps_by_day: Vec::new(),
            })
            .unwrap();
            let ctx = SolverContext::from_state(&st);
            // Each agent owns one distinct spot; every list also carries the
            // shared spot (2, 0) to pin dedup.
            let shared = HexCoord::new(2, 0);
            let targets: Vec<Vec<HexCoord>> = (0..kinds.len())
                .map(|i| vec![HexCoord::new(i32::try_from(i).unwrap_or(0), 7), shared])
                .collect();
            let all_spots: Vec<HexCoord> = (0..kinds.len())
                .map(|i| HexCoord::new(i32::try_from(i).unwrap_or(0), 7))
                .chain([shared])
                .collect();

            let applied =
                AssignMode::All.apply(&ctx, targets.clone(), AssignOpts::element(WalkMode::Greedy));
            // The escort lists are `assign=all`'s prefix — that is the
            // non-regression contract against kinds.
            let escorted = AssignMode::Escort.apply(
                &ctx,
                targets.clone(),
                AssignOpts::element(WalkMode::Greedy),
            );

            for (index, kind) in kinds.iter().enumerate() {
                if *kind != AgentKind::Patrol {
                    continue;
                }
                let list = &applied[index];
                assert_eq!(
                    &list[..escorted[index].len()],
                    escorted[index].as_slice(),
                    "{pattern} patrol {index} must keep the escort list as its prefix"
                );
                // …then every other agent's share, rotated to start just
                // after this Patrol's index, deduped.
                let mut expected = escorted[index].clone();
                for k in 1..kinds.len() {
                    let other = (index + k) % kinds.len();
                    for spot in &targets[other] {
                        if !expected.contains(spot) {
                            expected.push(*spot);
                        }
                    }
                }
                assert_eq!(list, &expected, "{pattern} patrol {index} must hold the full union");
                for spot in &all_spots {
                    assert!(list.contains(spot), "{pattern} patrol {index} misses {spot:?}");
                }
            }

            // Supplies: escort projection over the unioned Patrol lists.
            let (pairs, wait) = pair_supplies(&ctx, FuelpairMode::Nearest);
            assert!(wait.is_empty(), "{pattern}: capacity covers every Supply here");
            let paths = day_paths(&ctx, &applied, WalkMode::Greedy, BudgetMode::Element);
            for (supply, patrol) in pairs {
                assert_eq!(
                    applied[supply], paths[patrol],
                    "{pattern} supply {supply} must walk patrol {patrol}'s projected path"
                );
                assert_ne!(
                    applied[supply], targets[supply],
                    "{pattern} supply {supply} must not keep its own tour"
                );
            }

            // The mode is a real switch, not a synonym for the default.
            let default = AssignMode::Agents.apply(
                &ctx,
                targets.clone(),
                AssignOpts::element(WalkMode::Greedy),
            );
            assert_ne!(applied, default, "{pattern}: assign=all must change the lists");
        }
    }

    /// Issue #253 — with no Patrol in the team there is nobody to union into
    /// and nobody to escort: every Supply waits (the `assign=patrol` contract).
    #[test]
    fn assign_all_without_a_patrol_makes_supplies_wait() {
        let map = map_with_spots();
        let st = MatchState::new(MatchSetting {
            match_id: MatchId::new("assign-all-solo").unwrap(),
            team: PlayerId::new("player-a").unwrap(),
            map,
            agents: vec![
                Agent::supply(0, HexCoord::new(0, 0)),
                Agent::supply(1, HexCoord::new(3, 3)),
                Agent::supply(2, HexCoord::new(7, 7)),
            ],
            day_steps: 4,
            num_days: 1,
            day_steps_by_day: Vec::new(),
        })
        .unwrap();
        let ctx = SolverContext::from_state(&st);
        let targets =
            vec![vec![HexCoord::new(2, 0)], vec![HexCoord::new(5, 5)], vec![HexCoord::new(6, 6)]];
        let applied = AssignMode::All.apply(&ctx, targets, AssignOpts::element(WalkMode::Greedy));
        assert!(
            applied.iter().all(Vec::is_empty),
            "nobody to walk, so everyone waits: {applied:?}"
        );
    }

    /// Issue #255 — the `budget` parameter is schema-gated: omitted and
    /// `element` select the historical element clock (byte-identical),
    /// `official` truncates to the terrain-weighted budget, anything else is
    /// an `InvalidParameter`.
    #[test]
    fn budget_param_parses_element_official_and_rejects_anything_else() {
        let parse = |id: &str| parse_budget(&CandidateSpec::parse(id).expect("id parses"));
        assert_eq!(parse("loop"), Ok(BudgetMode::Element));
        assert_eq!(parse("loop:budget=element"), Ok(BudgetMode::Element));
        assert_eq!(parse("loop:budget=official"), Ok(BudgetMode::Official));
        assert_eq!(
            parse("multiloop:grid=2x2:walk=astar:assign=all:budget=official"),
            Ok(BudgetMode::Official)
        );
        assert_eq!(parse("loop"), Ok(BudgetMode::default()), "element is default");
        for id in ["loop:budget=Official", "loop:budget=wire", "loop:budget=2"] {
            assert!(
                matches!(parse(id), Err(CandidateError::InvalidParameter { key, .. }) if key == "budget"),
                "{id} must be rejected as an invalid budget value"
            );
        }
    }

    /// Issue #255 byte-identical contract: `budget=element` (the default)
    /// returns the plan untouched — the element clock is what every pre-#255
    /// id already runs.
    #[test]
    fn budget_element_returns_the_plan_untouched() {
        let st = state();
        let ctx = SolverContext::from_state(&st);
        let targets = vec![vec![HexCoord::new(2, 0)], vec![], vec![]];
        let plan = plan_walk(&ctx, &targets);
        assert_eq!(BudgetMode::Element.apply(&ctx, plan.clone()), plan, "element must be a no-op");
        assert_eq!(BudgetMode::default(), BudgetMode::Element, "the default is element");
    }

    /// Issue #255 AC1 — `budget=official` plans only what the official
    /// weighted budget can pay for. Replaying the emitted plan per agent:
    ///
    /// * every Move fits the remaining budget (Σ origin step cost never
    ///   exceeds `daySteps`) and is legal on the live map;
    /// * once the budget is exhausted, every remaining action is a `Wait`
    ///   (nothing the official rule would reject is ever emitted);
    /// * the budget-consuming prefix sums to **exactly** `daySteps` — the
    ///   same equality the wire validates (`engine::wire_plan`), so the
    ///   pad-append branch there never fires (AC2's `pad == 0`).
    #[test]
    fn budget_official_plan_fits_the_official_weighted_budget() {
        let st = state(); // 8x8 plain map, day_steps = 4, 3 agents
        let ctx = SolverContext::from_state(&st);
        // A greedy tour towards far cells: element mode would emit 4 moves
        // each, which on 平地 (step cost 2) is 8 weighted units — double the
        // official budget of 4.
        let targets =
            vec![vec![HexCoord::new(7, 7)], vec![HexCoord::new(7, 7)], vec![HexCoord::new(6, 6)]];
        let raw = plan_walk(&ctx, &targets);
        let plan = BudgetMode::Official.apply(&ctx, raw.clone());
        assert_ne!(plan, raw, "the element plan must not already fit the budget");
        assert!(
            plan.is_well_shaped(&ctx.agent_ids, ctx.day_steps),
            "truncation must preserve the engine day contract"
        );

        let agent_count = ctx.agent_ids.len();
        for (index, id) in ctx.agent_ids.iter().enumerate() {
            let agent =
                ctx.state.agents.iter().find(|a| a.id == *id).expect("agent exists in state");
            let mut pos = agent.pos;
            let mut used: u32 = 0;
            let mut exhausted = false;
            let mut budget_units: u32 = 0;
            for step in 0..ctx.day_steps as usize {
                let action = &plan.actions[step * agent_count + index];
                match action.kind {
                    ActionKind::Move(dir) => {
                        assert!(
                            !exhausted,
                            "agent {id} moved after the official budget was exhausted"
                        );
                        let origin = ctx.state.setting.map.get(pos).expect("agent is on the map");
                        let cost = engine::wire_plan::terrain_step_cost(origin.terrain)
                            .expect("agent never stands on impassable terrain");
                        assert!(
                            used + cost <= ctx.day_steps,
                            "agent {id}: move would cost {used} + {cost} > {}",
                            ctx.day_steps
                        );
                        used += cost;
                        budget_units += cost;
                        let to = dir.step(pos);
                        assert!(
                            ctx.state.setting.map.get(to).is_some_and(|c| c.terrain.traversable()),
                            "agent {id} walked off the map or into a pond"
                        );
                        pos = to;
                    }
                    ActionKind::Wait => {
                        if used < ctx.day_steps {
                            used += 1;
                            budget_units += 1;
                        } else {
                            exhausted = true;
                        }
                    }
                }
                if used >= ctx.day_steps {
                    exhausted = true;
                }
            }
            assert_eq!(
                budget_units, ctx.day_steps,
                "agent {id}: the budget-consuming prefix must spend the official day \
                 exactly (this is the `pad == 0` property: normalize never appends)"
            );
        }
    }

    /// Issue #255 AC2 — re-submitting a `budget=official` plan is a no-op at
    /// the wire: `normalize_day_plan` keeps every move, appends no pad, and
    /// the result still validates under the official rule. The candidate's
    /// tail is never lost because the plan already stops at the budget.
    #[test]
    fn budget_official_plan_survives_wire_normalize_unchanged() {
        let st = state();
        let ctx = SolverContext::from_state(&st);
        let targets = vec![vec![HexCoord::new(7, 7)], Vec::new(), vec![HexCoord::new(6, 6)]];
        let plan = BudgetMode::Official.apply(&ctx, plan_walk(&ctx, &targets));

        // Element plan -> per-agent wire codes, the encoding
        // `cli::action_adapter::decision_to_actions` produces.
        let agent_count = ctx.agent_ids.len();
        let mut body: Vec<Vec<i32>> = Vec::with_capacity(agent_count);
        for index in 0..agent_count {
            let mut codes: Vec<i32> = Vec::new();
            let mut wait_run = 0i32;
            for step in 0..ctx.day_steps as usize {
                match plan.actions[step * agent_count + index].kind {
                    ActionKind::Wait => wait_run += 1,
                    ActionKind::Move(dir) => {
                        if wait_run > 0 {
                            codes.push(-wait_run);
                            wait_run = 0;
                        }
                        codes.push(i32::try_from(usize::from(dir as u8)).expect("dir fits"));
                    }
                }
            }
            if wait_run > 0 {
                codes.push(-wait_run);
            }
            body.push(codes);
        }

        let normalized = engine::wire_plan::normalize_day_plan(
            &ctx.state.setting.map,
            &ctx.state.agents,
            &ctx.agent_ids,
            ctx.day_steps,
            &body,
        )
        .expect("a budget=official plan is submittable");

        let moves = |body: &[Vec<i32>]| -> Vec<Vec<i32>> {
            body.iter().map(|codes| codes.iter().copied().filter(|c| *c >= 0).collect()).collect()
        };
        assert_eq!(
            moves(&normalized),
            moves(&body),
            "normalize must keep every move: no tail is lost"
        );
        for (index, id) in ctx.agent_ids.iter().enumerate() {
            let agent =
                ctx.state.agents.iter().find(|a| a.id == *id).expect("agent exists in state");
            // `Ok(sum)` where sum == day_steps is exactly the `pad == 0`
            // signal: normalize appended nothing, the plan already filled
            // the official budget.
            let sum = engine::wire_plan::validate_agent_plan(
                agent,
                &ctx.state.setting.map,
                ctx.day_steps,
                &normalized[index],
            )
            .unwrap_or_else(|e| panic!("agent {id} must be submittable: {e}"));
            assert_eq!(sum, ctx.day_steps, "agent {id}: official equality, no pad appended");
        }
    }

    /// Issue #215 AC — default equivalence: selecting `greedy` explicitly is
    /// *exactly* [`plan_walk`], action for action, on a fixed input.
    #[test]
    fn plan_walk_with_greedy_is_exactly_plan_walk() {
        let st = state();
        let ctx = SolverContext::from_state(&st);
        let targets = vec![vec![HexCoord::new(2, 0)], vec![], vec![]];
        assert_eq!(
            plan_walk_with(&ctx, &targets, WalkMode::Greedy, BudgetMode::Element),
            plan_walk(&ctx, &targets),
            "walk=greedy must be byte-for-byte the pre-#215 walk"
        );
        // And so is the omitted parameter, resolved through the mode default.
        assert_eq!(
            plan_walk_with(&ctx, &targets, WalkMode::default(), BudgetMode::Element),
            plan_walk(&ctx, &targets)
        );
    }
}
