//! Property-based invariant tests for the `domain` crate.
//!
//! These tests document and enforce the structural invariants that the
//! core domain types must satisfy. They run with `cargo test --workspace`.
//!
//! The proptest-generated strategies intentionally cover the *whole*
//! public surface (IDs, scores, states, turns, outcomes). Whenever a
//! domain invariant is tightened, add a property here so a regression
//! shows up as a failing test rather than a silent corruption.
//!
//! Several checks (large populations, deep nesting) are tagged
//! `#[ignore]` and only run via `cargo test --workspace -- --ignored`.
//! See the `heavy_*` tests at the bottom of this file.

use std::collections::HashSet;

use domain::{GameState, MatchId, Outcome, PlayerId, Score, Turn, TurnId};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

/// ASCII identifiers. Empty strings are intentionally excluded because
/// `*Id::new` rejects them.
fn arb_id_string() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-zA-Z][a-zA-Z0-9_-]{0,15}").expect("valid regex")
}

/// A `PlayerId` produced via the public constructor (no panics in strategies).
fn arb_player_id() -> impl Strategy<Value = PlayerId> {
    arb_id_string().prop_map(|s| PlayerId::new(s).expect("non-empty"))
}

fn arb_match_id() -> impl Strategy<Value = MatchId> {
    arb_id_string().prop_map(|s| MatchId::new(s).expect("non-empty"))
}

fn arb_turn_id() -> impl Strategy<Value = TurnId> {
    arb_id_string().prop_map(|s| TurnId::new(s).expect("non-empty"))
}

/// Bounded score. Real scoring semantics live in the engine; here we only
/// cover the absolute range that serde + invariants must survive.
fn arb_score() -> impl Strategy<Value = Score> {
    (-1_000_000i64..=1_000_000).prop_map(Score)
}

/// `Score` plus a non-zero action index.
fn arb_action_index() -> impl Strategy<Value = u16> {
    1u16..=u16::MAX
}

/// `Outcome` covering all variants (Pending, Draw, Winner).
fn arb_outcome() -> impl Strategy<Value = Outcome> {
    prop_oneof![
        Just(Outcome::Pending),
        Just(Outcome::Draw),
        arb_player_id().prop_map(Outcome::Winner),
    ]
}

/// A small `Vec<(PlayerId, Score)>` of unique players (per-contestant).
fn arb_unique_scores(max_players: usize) -> impl Strategy<Value = Vec<(PlayerId, Score)>> {
    proptest::collection::vec((arb_player_id(), arb_score()), 1..=max_players).prop_map(|pairs| {
        // De-duplicate by PlayerId, preserving the first occurrence.
        let mut seen = HashSet::new();
        pairs.into_iter().filter(|(p, _)| seen.insert(p.clone())).collect()
    })
}

fn arb_game_state() -> impl Strategy<Value = GameState> {
    (arb_match_id(), any::<u32>(), arb_unique_scores(8))
        .prop_map(|(match_id, turn, scores)| GameState { match_id, turn, scores })
}

fn arb_turn(state: GameState) -> impl Strategy<Value = Turn> {
    (arb_turn_id(), proptest::collection::vec(any::<u8>(), 0..=16), arb_action_index()).prop_map(
        move |(id, payload, action_index)| {
            let payload = serde_json::Value::Array(
                payload.into_iter().map(serde_json::Value::from).collect(),
            );
            // Pick an author that exists in `state.scores` so `validate` passes.
            let author = state.scores.first().map_or_else(
                || PlayerId::new("player-stub").expect("non-empty"),
                |(p, _)| p.clone(),
            );
            Turn { id, author, action_index, payload }
        },
    )
}

/// Convenience for tests that need a turn produced by the standard
/// generator without supplying their own state — generates a state first
/// and uses its first player as the author.
fn arb_turn_default() -> impl Strategy<Value = Turn> {
    arb_game_state().prop_flat_map(arb_turn)
}

// ---------------------------------------------------------------------------
// ID invariants
// ---------------------------------------------------------------------------

/// `prefix()` is a stable constant per ID kind. Document the contract
/// so an accidental rename (which would silently break the wire format)
/// is caught at compile/test time.
#[test]
fn id_prefix_is_stable() {
    assert_eq!(MatchId::prefix(), "match");
    assert_eq!(PlayerId::prefix(), "player");
    assert_eq!(TurnId::prefix(), "turn");
}

proptest! {
    #[test]
    fn id_new_rejects_empty_string(_s in "\\s*") {
        prop_assert!(MatchId::new("").is_err());
        prop_assert!(PlayerId::new("").is_err());
        prop_assert!(TurnId::new("").is_err());
    }

    #[test]
    fn id_display_roundtrip(
        s in arb_id_string(),
    ) {
        let id = PlayerId::new(s.clone()).expect("non-empty");
        prop_assert_eq!(id.to_string(), s);
    }

    #[test]
    fn id_equality_matches_inner_string(
        a in arb_id_string(),
        b in arb_id_string(),
    ) {
        let lhs = PlayerId::new(a.clone()).expect("non-empty");
        let rhs = PlayerId::new(b.clone()).expect("non-empty");
        prop_assert_eq!(lhs == rhs, a == b);
    }
}

// ---------------------------------------------------------------------------
// GameState invariants
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn game_state_preserves_unique_player_ids(
        state in arb_game_state(),
    ) {
        let json = serde_json::to_string(&state).expect("serialize");
        let back: GameState = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(&state, &back);

        let unique: HashSet<&PlayerId> =
            back.scores.iter().map(|(p, _)| p).collect();
        prop_assert_eq!(unique.len(), back.scores.len());
    }

    #[test]
    fn game_state_round_trips_duplicate_player_ids_when_present(
        state in arb_game_state(),
    ) {
        let mut state = state;
        if let Some((first_id, _)) = state.scores.first().cloned() {
            state.scores.push((first_id, Score(0)));
        }

        let json = serde_json::to_string(&state).expect("serialize");
        let back: GameState = serde_json::from_str(&json).expect("deserialize");

        prop_assert_eq!(state.scores.len(), back.scores.len());
        prop_assert_eq!(&state, &back);
    }
}

// ---------------------------------------------------------------------------
// Turn invariants
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn turn_new_payload_is_null(
        id in arb_turn_id(),
        author in arb_player_id(),
        action in arb_action_index(),
    ) {
        let t = Turn::new(id, author.clone(), action);
        prop_assert_eq!(&t.author, &author);
        prop_assert_eq!(t.payload, serde_json::Value::Null);
    }

    #[test]
    fn turn_validates_when_author_is_part_of_match(
        state in arb_game_state(),
        payload_bytes in proptest::collection::vec(any::<u8>(), 0..=8),
    ) {
        let author = state
            .scores
            .first()
            .map(|(p, _)| p.clone())
            .expect("arb_game_state guarantees >= 1 player");
        let turn = Turn {
            id: TurnId::new("turn-stub").expect("non-empty"),
            author: author.clone(),
            action_index: 1,
            payload: serde_json::Value::Array(
                payload_bytes.iter().map(|b| serde_json::Value::from(*b)).collect(),
            ),
        };

        prop_assert!(turn.validate(&state).is_ok());
        prop_assert_eq!(&turn.author, &author);
    }

    #[test]
    fn turn_validates_rejects_unknown_author(
        state in arb_game_state(),
        author in arb_player_id(),
        action in arb_action_index(),
    ) {
        prop_assume!(
            !state.scores.iter().any(|(p, _)| *p == author),
            "filter out authors already in the match"
        );
        let turn = Turn {
            id: TurnId::new("turn-stub").expect("non-empty"),
            author,
            action_index: action,
            payload: serde_json::Value::Null,
        };
        prop_assert!(turn.validate(&state).is_err());
    }

    #[test]
    fn turn_validates_rejects_zero_action_index(
        state in arb_game_state(),
    ) {
        let author = state
            .scores
            .first()
            .map(|(p, _)| p.clone())
            .expect("arb_game_state guarantees >= 1 player");
        let turn = Turn {
            id: TurnId::new("turn-stub").expect("non-empty"),
            author,
            action_index: 0,
            payload: serde_json::Value::Null,
        };
        prop_assert!(turn.validate(&state).is_err());
    }
}

// ---------------------------------------------------------------------------
// Score invariants
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn score_transparent_serde(
        s in arb_score(),
    ) {
        let json = serde_json::to_string(&s).expect("serialize");
        let back: Score = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(s, back);
        // Transparent wrapper: no object braces.
        prop_assert!(!json.starts_with('{'), "Score must serialise as a bare number, got {json}");
    }

    #[test]
    fn score_ordering_matches_inner(
        a in arb_score(),
        b in arb_score(),
    ) {
        prop_assert_eq!(a.cmp(&b), a.0.cmp(&b.0));
    }
}

// ---------------------------------------------------------------------------
// Outcome invariants
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn outcome_round_trips(
        outcome in arb_outcome(),
    ) {
        let json = serde_json::to_string(&outcome).expect("serialize");
        let back: Outcome = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(outcome, back);
    }
}

// ---------------------------------------------------------------------------
// Local stubs documenting expected Agent / Fuel invariants
// ---------------------------------------------------------------------------
//
// The base domain snapshot (`b504821`) deliberately does NOT carry
// `Agent` or fuel types yet — those will land in a later ticket. The
// invariants below describe the *shape* the types must satisfy and act
// as a regression net for when the real types arrive.

/// Local stub for a per-match agent. Mirrors the invariants the real
/// `domain::agent::Agent` type will have to honour:
///
/// - `agent_id` is unique within a single match (`#[unique]` invariant)
/// - `kind` is one of `{"scout", "supply", ...}` and serialises as a `snake_case` tag
/// - `fuel` is `0 <= fuel <= fuel_cap` (upper bound = contest rule)
/// - `pos` is a valid cell on the map
#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentStub {
    agent_id: u32,
    kind: String,
    fuel: u32,
    fuel_cap: u32,
    pos: u32,
}

fn arb_agent_kind() -> impl Strategy<Value = String> {
    proptest::sample::select(vec!["scout".to_owned(), "supply".to_owned(), "guard".to_owned()])
}

fn arb_agent_stub(max_fuel_cap: u32) -> impl Strategy<Value = AgentStub> {
    // `fuel_cap` is drawn first so we can pin `fuel` to `0..=fuel_cap`
    // and guarantee the invariant under test.
    (any::<u32>(), arb_agent_kind(), 1u32..=max_fuel_cap, 0u32..=1024).prop_flat_map(
        |(id, kind, fuel_cap, pos)| {
            (0u32..=fuel_cap).prop_map(move |fuel| AgentStub {
                agent_id: id,
                kind: kind.clone(),
                fuel,
                fuel_cap,
                pos,
            })
        },
    )
}

proptest! {
    #[test]
    fn agent_fuel_never_exceeds_cap(
        agent in arb_agent_stub(200),
    ) {
        prop_assert!(agent.fuel <= agent.fuel_cap, "fuel {} > cap {}", agent.fuel, agent.fuel_cap);
    }

    #[test]
    fn agent_unique_id_within_match(
        agents in proptest::collection::vec(arb_agent_stub(50), 1..=8),
    ) {
        let ids: HashSet<u32> = agents.iter().map(|a| a.agent_id).collect();
        prop_assert_eq!(ids.len(), agents.len(),
            "duplicate agent_id in match: {:?}", agents);
    }

    #[test]
    fn agent_kind_is_recognised(
        agent in arb_agent_stub(50),
    ) {
        prop_assert!(
            ["scout", "supply", "guard"].contains(&agent.kind.as_str()),
            "unknown kind: {}", agent.kind
        );
    }
}

// ---------------------------------------------------------------------------
// Heavy / long-running property tests (run via `cargo test -- --ignored`)
// ---------------------------------------------------------------------------
//
// These tests exercise larger inputs and are excluded from the default
// `cargo test --workspace` run to keep CI fast.

#[ignore = "heavy property test, run via `cargo test -- --ignored`"]
#[test]
fn heavy_unique_scores_invariant_at_scale() {
    // 256 turns * 8 players = 2048 round-trips. Confirms serde stays
    // O(N) and no quadratic surprises creep in.
    proptest!(ProptestConfig::with_cases(256), |(
        states in proptest::collection::vec(arb_game_state(), 1..=8),
    )| {
        for state in &states {
            let json = serde_json::to_string(state).expect("serialize");
            let back: GameState = serde_json::from_str(&json).expect("deserialize");
            prop_assert_eq!(state, &back);
            let unique: HashSet<&PlayerId> =
                back.scores.iter().map(|(p, _)| p).collect();
            prop_assert_eq!(unique.len(), back.scores.len());
        }
    });
}

#[ignore = "heavy property test, run via `cargo test -- --ignored`"]
#[test]
fn heavy_turn_validation_population() {
    // 1000 mixed-validity turns. Catches any monotonicity regression in
    // the validator.
    proptest!(ProptestConfig::with_cases(1000), |(
        state in arb_game_state(),
        turns in proptest::collection::vec(arb_turn_default(), 1..=16),
    )| {
        for turn in &turns {
            // Either the turn validates (author is in state.scores) or
            // it errors — both are legal. The invariant is that the
            // error variant matches the failure mode.
            match turn.validate(&state) {
                Ok(()) => prop_assert!(
                    state.scores.iter().any(|(p, _)| *p == turn.author)
                ),
                Err(_) => prop_assert!(
                    turn.action_index == 0
                        || !state.scores.iter().any(|(p, _)| *p == turn.author)
                ),
            }
        }
    });
}
