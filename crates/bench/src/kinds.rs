//! `--kinds` — the evaluator's agent-kind composition knob (Issue #235).
//!
//! The wire setting carries only agent *start cells*; who patrols and who
//! supplies is decided separately (`POST /agent` on the live path,
//! `engine::from_wire`'s alternation offline). Until this module that offline
//! rule was fixed, so every candidate was compared under exactly one team:
//! slot even -> patrol, slot odd -> supply. Issue #235 asks for the
//! composition itself to become an evaluation axis.
//!
//! # Grammar
//!
//! ```text
//! --kinds <pattern>      pattern := kind ("," kind)*
//! kind                   := P | S            (case-insensitive)
//!                          | patrol | supply  (case-insensitive)
//! ```
//!
//! A pattern is a **cycle over agent slots**: slot `i` gets
//! `pattern[i % pattern.len()]`. That is what lets one pattern serve a corpus
//! whose agent counts differ (the bundled fixtures field 3, 4, 5, 6 and 8
//! agents), and whenever the pattern is at least as long as the team it is
//! simply its first `n` entries.
//!
//! The default pattern is [`DEFAULT_KINDS_PATTERN`] = `P,S`, i.e. slot even ->
//! patrol, slot odd -> supply — **byte-identical** to the alternation
//! `engine::from_wire` has always used, so pre-#235 evidence keeps its
//! meaning. `--kinds P` is the all-patrol team (5P0S on a 5-agent case),
//! `--kinds P,P,P,P,S` is 4P1S there, and the omitted flag is 3P2S there.
//!
//! # Legality (出典)
//!
//! The official `POST /agent` schema (`ProconApi_submitAgentKinds` in the
//! vendored `OpenAPI` bundle,
//! `docs/pdf/回答システムに関する情報・簡易版の回答用サーバー_1.zip`)
//! accepts an `AgentKind[]` whose element count equals the agent count and
//! whose elements are `0 | 1` — it says nothing about the patrol/supply
//! ratio. `domain` records the same reading of 募集要項 §エージェント
//! (`crates/domain/src/agent.rs`: a team "may freely choose the Patrol /
//! Supply kind mix"), and `engine::MatchSetting::validate` checks agent
//! count (3..=8), ids, start cells and patrol fuel — no kind ratio.
//!
//! So **every** composition is legal. This module rejects only what those
//! sources do make ill-formed: an empty pattern, a token that is not a kind,
//! and a pattern longer than the official maximum team size (8), which could
//! never be read in full.

use engine::model::AgentKind;

/// The pattern `--kinds` uses when the flag is omitted.
///
/// Spelled out, it is the pre-#235 alternation: slot 0 patrol, slot 1 supply,
/// slot 2 patrol, … Whatever else changes, this string must not.
pub const DEFAULT_KINDS_PATTERN: &str = "P,S";

/// Official maximum team size (`engine::MatchSetting::validate`,
/// `domain::GameModel::AGENT_COUNT_MAX`, 募集要項 §エージェント).
pub const MAX_AGENTS: usize = 8;

/// Parse a `--kinds` pattern into one [`AgentKind`] per pattern slot.
///
/// # Errors
/// An empty spec, a token that is neither a kind, or a pattern longer than
/// [`MAX_AGENTS`].
pub fn parse_kinds(spec: &str) -> Result<Vec<AgentKind>, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(format!(
            "--kinds needs a pattern such as {DEFAULT_KINDS_PATTERN:?} (P = patrol, S = supply)"
        ));
    }
    let mut out = Vec::new();
    for token in spec.split(',') {
        let token = token.trim();
        out.push(match token.to_ascii_lowercase().as_str() {
            "p" | "patrol" => AgentKind::Patrol,
            "s" | "supply" => AgentKind::Supply,
            other => {
                return Err(format!(
                    "--kinds token {other:?} is not a kind — expected P | patrol or S | supply, \
                     as in {DEFAULT_KINDS_PATTERN:?}"
                ));
            }
        });
    }
    if out.len() > MAX_AGENTS {
        return Err(format!(
            "--kinds pattern has {} slots but a team has at most {MAX_AGENTS} agents \
             (募集要項 §エージェント / MatchSetting::validate)",
            out.len()
        ));
    }
    Ok(out)
}

/// Expand a pattern to exactly `n_agents` kinds, cycling `pattern[i % len]`.
///
/// A `None` pattern means the default: [`engine::from_wire::default_agent_kinds`]
/// — the same alternation `DEFAULT_KINDS_PATTERN` expands to, reached through
/// one place so the two can never drift.
///
/// # Panics
/// Never in practice: the pattern always has at least one slot (see
/// [`parse_kinds`]).
#[must_use]
pub fn resolve_kinds(pattern: Option<&[AgentKind]>, n_agents: usize) -> Vec<AgentKind> {
    match pattern {
        None | Some([]) => engine::from_wire::default_agent_kinds(n_agents),
        Some(p) => (0..n_agents).map(|i| p[i % p.len()]).collect(),
    }
}

/// Render kinds back to the `P,S,…` spelling the CLI accepts, so an evidence
/// line and a runnable flag use the same words.
#[must_use]
pub fn render_kinds(kinds: &[AgentKind]) -> String {
    kinds
        .iter()
        .map(|k| match k {
            AgentKind::Patrol => "P",
            AgentKind::Supply => "S",
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// `Some` when the spec names a pattern, `None` when the flag was omitted or
/// left empty — the "keep today's default" reading.
///
/// # Errors
/// Propagates [`parse_kinds`].
pub fn parse_kinds_flag(spec: &str) -> Result<Option<Vec<AgentKind>>, String> {
    if spec.trim().is_empty() {
        return Ok(None);
    }
    parse_kinds(spec).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_pattern_is_the_historical_alternation() {
        // This is the AC: the omitted flag must produce exactly what
        // from_wire always produced, for every team size in the official
        // 3..=8 range.
        for n in 3..=8 {
            let pattern = parse_kinds(DEFAULT_KINDS_PATTERN).expect("the default parses");
            let resolved = resolve_kinds(Some(&pattern), n);
            assert_eq!(
                resolved,
                engine::from_wire::default_agent_kinds(n),
                "n={n}: the default pattern must equal from_wire's alternation"
            );
            let implicit = resolve_kinds(None, n);
            assert_eq!(resolved, implicit, "n={n}: omitted flag == explicit default");
        }
        assert_eq!(DEFAULT_KINDS_PATTERN, "P,S");
    }

    #[test]
    fn patterns_cycle_over_slots() {
        let pattern = parse_kinds("P,P,S").unwrap();
        assert_eq!(
            resolve_kinds(Some(&pattern), 5),
            vec![
                AgentKind::Patrol,
                AgentKind::Patrol,
                AgentKind::Supply,
                AgentKind::Patrol,
                AgentKind::Patrol
            ]
        );
        // A pattern at least as long as the team is its own prefix.
        let long = parse_kinds("P,S,P,S,P").unwrap();
        assert_eq!(
            resolve_kinds(Some(&long), 5),
            vec![
                AgentKind::Patrol,
                AgentKind::Supply,
                AgentKind::Patrol,
                AgentKind::Supply,
                AgentKind::Patrol
            ],
            "the Issue #235 example must be 3P2S on a 5-agent team"
        );
        assert_eq!(
            resolve_kinds(Some(&long), 3),
            vec![AgentKind::Patrol, AgentKind::Supply, AgentKind::Patrol]
        );
    }

    #[test]
    fn single_kind_patterns_express_the_extremes() {
        let all_patrol = parse_kinds("P").unwrap();
        assert!(resolve_kinds(Some(&all_patrol), 5).iter().all(|k| *k == AgentKind::Patrol));
        let all_supply = parse_kinds("supply").unwrap();
        assert!(resolve_kinds(Some(&all_supply), 8).iter().all(|k| *k == AgentKind::Supply));
    }

    #[test]
    fn tokens_are_case_insensitive_and_accept_full_words() {
        assert_eq!(
            parse_kinds("p, S ,Patrol,supply"),
            Ok(vec![AgentKind::Patrol, AgentKind::Supply, AgentKind::Patrol, AgentKind::Supply])
        );
    }

    #[test]
    fn malformed_patterns_are_rejected() {
        for spec in ["", "   ", "3p2s", "P,S,", ",P", "X", "patroll"] {
            match parse_kinds(spec) {
                Ok(ok) => panic!("{spec:?} must be rejected, got {ok:?}"),
                Err(err) => assert!(err.contains("--kinds"), "{spec:?} → {err}"),
            }
        }
        let long = "P,S,P,S,P,S,P,S,P";
        let err = parse_kinds(long).expect_err("9 slots exceed the official maximum team");
        assert!(err.contains("at most 8"), "{err}");
    }

    #[test]
    fn an_empty_flag_means_the_default() {
        assert_eq!(parse_kinds_flag("").unwrap(), None);
        assert_eq!(parse_kinds_flag("  ").unwrap(), None);
        assert_eq!(parse_kinds_flag("P,S").unwrap(), Some(parse_kinds("P,S").unwrap()));
    }

    #[test]
    fn rendering_round_trips_to_the_cli_spelling() {
        let pattern = parse_kinds(DEFAULT_KINDS_PATTERN).unwrap();
        assert_eq!(render_kinds(&pattern), "P,S");
        let kinds = resolve_kinds(Some(&parse_kinds("P,P,P,P,S").unwrap()), 5);
        assert_eq!(render_kinds(&kinds), "P,P,P,P,S");
        assert_eq!(render_kinds(&resolve_kinds(Some(&parse_kinds("P").unwrap()), 5)), "P,P,P,P,P");
    }
}
