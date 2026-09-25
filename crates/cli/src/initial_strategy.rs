//! Live-CLI label parsing for [`InitialStrategy`] — the single parser
//! behind `procon tournament --loop-initial-strategy` and
//! `procon coordinator --loop-initial-strategy` (both flags bind it via
//! `value_parser`, so an unparsable label never reaches a subcommand).
//!
//! # Offline candidate id ↔ live flag (1:1)
//!
//! | offline `loop` parameter            | live `--loop-initial-strategy` | variant                    |
//! |-------------------------------------|--------------------------------|----------------------------|
//! | `initial=greedy` (default)          | `greedy`                       | `GreedyNearestNeighbor`    |
//! | `initial=identity`                  | `identity`                     | `Identity`                 |
//! | `initial=reverse`                   | `reverse`                      | `Reverse`                  |
//! | `initial=sweep-row`                 | `sweep-row`                    | `SweepRow`                 |
//! | `initial=sweep-col`                 | `sweep-col`                    | `SweepCol`                 |
//! | `initial=random:seed=N`             | `random:N`                     | `Random { seed: N }`       |
//!
//! Each live label resolves to exactly one offline `initial=` value and the
//! other way round, so a candidate selected in the offline evaluator
//! (`procon-eval --candidates 'loop:initial=reverse'`) replays on the wire
//! with `procon coordinator --loop-initial-strategy reverse`. The seed is
//! the only shape that differs: offline writes it as its own `seed=N`
//! parameter (so `initial` and `seed` stay independent parameters), while
//! the live side folds it into the label as `random:N` so one flag stays
//! 1:1 with one strategy.
//!
//! The legacy `PascalCase` spellings (`GreedyNearestNeighbor`, `Identity`,
//! `Reverse`, `SweepRow`, `SweepCol`) are accepted as aliases so existing
//! invocations keep working; they are not part of the table above and
//! [`initial_strategy_label`] always reports the canonical form.
//!
//! # No silent fallback
//!
//! An unknown label is an explicit error ([`InitialStrategyError::Unknown`]),
//! never a silent [`InitialStrategy::Identity`] — a typo must fail the
//! command instead of quietly running a different algorithm (Issue #170).

use solver_loop::InitialStrategy;
use thiserror::Error;

/// Canonical live labels, in offline-id order. Mirrored by the table in the
/// module docs and by [`initial_strategy_label`].
pub const ACCEPTED_LABELS: &[&str] =
    &["greedy", "identity", "reverse", "sweep-row", "sweep-col", "random:<seed>"];

/// Errors produced by [`parse_initial_strategy`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InitialStrategyError {
    /// The label is not a known strategy name. The message renders
    /// [`ACCEPTED_LABELS`], so the two can never drift apart.
    #[error(
        "unknown --loop-initial-strategy {label:?}; expected one of: {expected} (PascalCase aliases accepted)",
        expected = ACCEPTED_LABELS.join(" | ")
    )]
    Unknown {
        /// The label exactly as the operator typed it.
        label: String,
    },
    /// `random` was given without a seed.
    #[error(
        "--loop-initial-strategy {label:?} needs a seed, e.g. random:42 (offline: initial=random:seed=42)"
    )]
    MissingSeed {
        /// The label exactly as the operator typed it.
        label: String,
    },
    /// `random:<seed>` was given with a seed that is not a `u64`.
    #[error("--loop-initial-strategy {label:?}: bad random seed: {reason}")]
    InvalidSeed {
        /// The label exactly as the operator typed it.
        label: String,
        /// Why the seed could not be used.
        reason: String,
    },
}

/// Parse a live `--loop-initial-strategy` label into an [`InitialStrategy`].
///
/// This is the inverse of [`initial_strategy_label`] for every canonical
/// label. Unknown labels return [`InitialStrategyError::Unknown`] — there
/// is deliberately no fallback variant.
///
/// # Errors
///
/// See [`InitialStrategyError`]: unknown label, `random` without a seed, or
/// a seed that does not parse as `u64`.
pub fn parse_initial_strategy(label: &str) -> Result<InitialStrategy, InitialStrategyError> {
    match label {
        "greedy" | "GreedyNearestNeighbor" => return Ok(InitialStrategy::GreedyNearestNeighbor),
        "identity" | "Identity" => return Ok(InitialStrategy::Identity),
        "reverse" | "Reverse" => return Ok(InitialStrategy::Reverse),
        "sweep-row" | "SweepRow" => return Ok(InitialStrategy::SweepRow),
        "sweep-col" | "SweepCol" => return Ok(InitialStrategy::SweepCol),
        "random" => {
            return Err(InitialStrategyError::MissingSeed { label: label.to_owned() });
        }
        _ => {}
    }
    if let Some((kind, seed)) = label.split_once(':') {
        if kind == "random" {
            if seed.contains(':') {
                return Err(InitialStrategyError::InvalidSeed {
                    label: label.to_owned(),
                    reason: "expected random:<seed> with exactly one u64 value".to_owned(),
                });
            }
            return match seed.parse::<u64>() {
                Ok(seed) => Ok(InitialStrategy::Random { seed }),
                Err(err) => Err(InitialStrategyError::InvalidSeed {
                    label: label.to_owned(),
                    reason: format!("{seed:?} is not a u64 ({err})"),
                }),
            };
        }
    }
    Err(InitialStrategyError::Unknown { label: label.to_owned() })
}

/// Canonical live label for `strategy` — the inverse of
/// [`parse_initial_strategy`] on canonical labels.
///
/// Round-trip guarantee, pinned by the module tests:
/// `parse_initial_strategy(&initial_strategy_label(s)) == Ok(s)` for every
/// variant.
#[must_use]
pub fn initial_strategy_label(strategy: InitialStrategy) -> String {
    match strategy {
        InitialStrategy::GreedyNearestNeighbor => "greedy".to_owned(),
        InitialStrategy::Identity => "identity".to_owned(),
        InitialStrategy::Reverse => "reverse".to_owned(),
        InitialStrategy::SweepRow => "sweep-row".to_owned(),
        InitialStrategy::SweepCol => "sweep-col".to_owned(),
        InitialStrategy::Random { seed } => format!("random:{seed}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One entry per strategy: canonical label ↔ variant, and the offline
    /// `initial=` value it must correspond to (see the module table).
    fn canonical_cases() -> Vec<(String, InitialStrategy, &'static str)> {
        vec![
            ("greedy".to_owned(), InitialStrategy::GreedyNearestNeighbor, "greedy"),
            ("identity".to_owned(), InitialStrategy::Identity, "identity"),
            ("reverse".to_owned(), InitialStrategy::Reverse, "reverse"),
            ("sweep-row".to_owned(), InitialStrategy::SweepRow, "sweep-row"),
            ("sweep-col".to_owned(), InitialStrategy::SweepCol, "sweep-col"),
            ("random:42".to_owned(), InitialStrategy::Random { seed: 42 }, "random"),
        ]
    }

    /// Every canonical label parses, and label → strategy → label round-trips
    /// byte-exactly. The offline half is asserted by comparing the label with
    /// the offline `initial=` value from the table.
    #[test]
    fn every_canonical_label_round_trips() {
        for (label, strategy, offline) in canonical_cases() {
            assert_eq!(
                parse_initial_strategy(&label),
                Ok(strategy),
                "label {label:?} must parse to {strategy:?}"
            );
            assert_eq!(initial_strategy_label(strategy), label, "label({strategy:?})");
            assert_eq!(
                parse_initial_strategy(&initial_strategy_label(strategy)),
                Ok(strategy),
                "round-trip failed for {label:?}"
            );
            // Offline counterpart: `loop:initial=<value>` selects the same
            // variant (the seed lives in its own `seed=N` parameter).
            if offline == "random" {
                assert_eq!(label.strip_prefix("random:"), Some("42"));
            } else {
                assert_eq!(label, offline, "live label must equal the offline initial= value");
            }
        }
        // `ACCEPTED_LABELS` stays in sync with the parser.
        for accepted in ACCEPTED_LABELS {
            if accepted.starts_with("random") {
                assert!(parse_initial_strategy("random:1").is_ok());
            } else {
                assert!(
                    parse_initial_strategy(accepted).is_ok(),
                    "ACCEPTED_LABELS entry {accepted:?} does not parse"
                );
            }
        }
    }

    /// The legacy `PascalCase` spellings resolve to the same variants.
    #[test]
    fn pascal_case_aliases_resolve_to_the_same_variants() {
        for (alias, strategy) in [
            ("GreedyNearestNeighbor", InitialStrategy::GreedyNearestNeighbor),
            ("Identity", InitialStrategy::Identity),
            ("Reverse", InitialStrategy::Reverse),
            ("SweepRow", InitialStrategy::SweepRow),
            ("SweepCol", InitialStrategy::SweepCol),
        ] {
            assert_eq!(parse_initial_strategy(alias), Ok(strategy), "alias {alias}");
            // Aliases normalise to the canonical label.
            assert_ne!(initial_strategy_label(strategy), alias);
        }
    }

    /// Seed passthrough: `random:<seed>` carries the u64 through, including
    /// the bounds.
    #[test]
    fn random_label_carries_the_seed() {
        for seed in [0_u64, 1, 42, u64::MAX] {
            let label = format!("random:{seed}");
            assert_eq!(parse_initial_strategy(&label), Ok(InitialStrategy::Random { seed }));
        }
    }

    /// The core bug of Issue #170: an unknown label must be an explicit
    /// error, never a silent `Identity` fallback.
    #[test]
    fn unknown_label_is_an_error_never_identity() {
        for label in ["", "GreedyZoneCoverage", "greedy2", "reverse-1", "SWEEP-ROW", "randomx"] {
            let parsed = parse_initial_strategy(label);
            assert!(parsed.is_err(), "{label:?} must be rejected, got {parsed:?}");
            assert_ne!(parsed, Ok(InitialStrategy::Identity), "{label:?} must not fall back");
            let Err(err) = parsed else { panic!("{label:?} must be an error") };
            assert!(matches!(err, InitialStrategyError::Unknown { .. }), "{label:?}: {err}");
            // The message names the accepted labels so a typo is fixable,
            // and stays in sync with the `ACCEPTED_LABELS` constant.
            let msg = err.to_string();
            assert!(msg.contains("reverse"), "{msg}");
            assert!(msg.contains("random:<seed>"), "{msg}");
            for accepted in ACCEPTED_LABELS {
                assert!(msg.contains(accepted), "message {msg:?} must mention {accepted:?}");
            }
        }
    }

    /// `random` needs a seed; a malformed seed is rejected too.
    #[test]
    fn random_needs_a_well_formed_seed() {
        for label in ["random", "random:", "random:-1", "random:42:7", "random:abc"] {
            let err = match parse_initial_strategy(label) {
                Ok(v) => panic!("{label:?} must be rejected, got {v:?}"),
                Err(err) => err,
            };
            assert!(
                matches!(
                    err,
                    InitialStrategyError::MissingSeed { .. }
                        | InitialStrategyError::InvalidSeed { .. }
                ),
                "{label}: {err}"
            );
        }
    }
}
