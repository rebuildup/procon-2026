//! Reproducible-case import hook: arena replay JSON → offline `CaseSpec`.
//!
//! The arena publishes a same-board replay JSON for every match (Issue #179
//! scope: reproducible-case import hook). This module converts that JSON
//! into a [`bench::CaseSpec`] so an arena loss can be re-run offline
//! through Issue #178's canonical evaluator — and re-ranked against the
//! corpus that selected the candidate in the first place.
//!
//! The converter is deliberately **thin and fail-closed**:
//!
//! - the wire `MatchSetting` is located at a known pointer (root, or one of
//!   a few documented wrappers) — no shape guessing beyond that list;
//! - the located value must deserialize into the official wire setting;
//! - the result is validated through the same conversions the evaluator
//!   uses ([`engine::from_wire::sim_config_from_wire`] +
//!   [`engine::from_wire::protocol_setting_to_engine`]), so a malformed
//!   board is rejected here rather than producing a poisoned corpus entry.
//!
//! The emitted case is **fixture-shaped**: `case_id = fixture:<name>` with
//! `CaseSource::Fixture`. It carries no account data — only the board and
//! a name derived from the replay's match identifier (sanitised).

use std::path::Path;

use bench::CaseSpec;
use protocol::match_config::MatchSetting as WireSetting;
use thiserror::Error;

/// Errors from [`case_from_replay`]. Every variant fails closed — there is
/// no "best effort" path that guesses at an unknown arena payload.
#[derive(Debug, Error)]
pub enum ImportError {
    /// The input is not JSON at all (HTML shell, truncated file, …).
    #[error("arena replay is not valid JSON: {0}")]
    NotJson(String),
    /// No match setting at any known location.
    #[error("no match setting found in the arena replay (looked at: {tried})")]
    SettingNotFound {
        /// JSON pointers that were tried, in order.
        tried: String,
    },
    /// The located value does not match the official wire setting shape.
    #[error("arena replay match setting is malformed: {0}")]
    BadSetting(String),
    /// The board fails the evaluator's own validation.
    #[error("arena replay board is not evaluable: {0}")]
    InvalidBoard(String),
    /// The derived / supplied case name is unusable.
    #[error("arena replay case name is unusable: {0}")]
    BadName(String),
    /// IO failure reading the input file.
    #[error("could not read {path}: {source}")]
    Io {
        /// Path that failed.
        path: String,
        /// Underlying IO error.
        #[source]
        source: std::io::Error,
    },
}

/// JSON pointers checked for the wire match setting, in order.
const SETTING_POINTERS: [&str; 4] =
    ["/setting", "/matchSetting", "/replay/setting", "/match/setting"];

/// Convert raw arena replay bytes into a [`CaseSpec`].
///
/// `name` overrides the case name; when `None` the replay's match
/// identifier (`matchId` / `match_id` / `id`) is used, falling back to a
/// stable generic name. The name is sanitised to `[A-Za-z0-9._-]` so it is
/// always safe as a `case_id` component and as a file stem.
///
/// # Errors
/// See [`ImportError`] — every unknown shape fails closed.
pub fn case_from_replay(raw: &[u8], name: Option<&str>) -> Result<CaseSpec, ImportError> {
    let value: serde_json::Value =
        serde_json::from_slice(raw).map_err(|e| ImportError::NotJson(e.to_string()))?;
    let setting_value = locate_setting(&value)?;
    let setting: WireSetting = serde_json::from_value(setting_value)
        .map_err(|e| ImportError::BadSetting(e.to_string()))?;

    let name = match name {
        Some(n) => sanitize_name(n)?,
        None => derive_name(&value),
    };

    let case = CaseSpec {
        case_id: CaseSpec::compose_id("fixture", &name),
        seed: None,
        generation_parameters: None,
        source: bench::CaseSource::Fixture { name: name.clone() },
        setting,
    };

    // Validate through the evaluator's own conversion path: thresholds,
    // day budgets, map shape, agent positions — all of it must hold or the
    // case would only fail later, inside a worker, as a poisoned record.
    case.sim_config().map_err(|e| ImportError::InvalidBoard(e.to_string()))?;
    let team = domain::PlayerId::new("arena-import")
        .map_err(|e| ImportError::InvalidBoard(format!("team id: {e}")))?;
    case.to_engine_setting(&team).map_err(|e| ImportError::InvalidBoard(e.to_string()))?;

    Ok(case)
}

/// Read `path` and convert it; see [`case_from_replay`].
///
/// # Errors
/// [`ImportError::Io`] when the file cannot be read, else
/// [`case_from_replay`]'s errors.
pub fn case_from_replay_file(path: &Path, name: Option<&str>) -> Result<CaseSpec, ImportError> {
    let raw = std::fs::read(path)
        .map_err(|source| ImportError::Io { path: path.display().to_string(), source })?;
    case_from_replay(&raw, name)
}

/// Write a case as pretty JSON (atomic enough for an operator tool: write
/// then rename).
///
/// # Errors
/// IO / serialisation failure.
pub fn write_case(path: &Path, case: &CaseSpec) -> Result<(), ImportError> {
    let json = serde_json::to_vec_pretty(case)
        .map_err(|e| ImportError::InvalidBoard(format!("serialise case: {e}")))?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|source| ImportError::Io { path: parent.display().to_string(), source })?;
    }
    std::fs::write(path, json)
        .map_err(|source| ImportError::Io { path: path.display().to_string(), source })
}

/// Find the wire setting at one of the known locations. The root itself
/// counts when it carries a `map` (a bare setting payload).
fn locate_setting(value: &serde_json::Value) -> Result<serde_json::Value, ImportError> {
    if value.get("map").is_some() {
        return Ok(value.clone());
    }
    for pointer in SETTING_POINTERS {
        if let Some(found) = value.pointer(pointer) {
            if found.get("map").is_some() {
                return Ok(found.clone());
            }
        }
    }
    Err(ImportError::SettingNotFound { tried: SETTING_POINTERS.join(", ") + ", <root>" })
}

/// Default case name: the replay's match identifier, sanitised. An
/// already-emitted `CaseSpec` (which carries `case_id = fixture:<name>`)
/// round-trips by shedding the tag again.
fn derive_name(value: &serde_json::Value) -> String {
    for key in ["matchId", "match_id", "id", "caseId", "case_id"] {
        if let Some(raw) = value.get(key).and_then(serde_json::Value::as_str) {
            let raw =
                if key.starts_with("case") { raw.rsplit(':').next().unwrap_or(raw) } else { raw };
            if let Ok(name) = sanitize_name(raw) {
                return name;
            }
        }
    }
    format!("arena-{}", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"))
}

/// Keep `[A-Za-z0-9._-]`, collapse everything else to `-`, cap the length.
fn sanitize_name(name: &str) -> Result<String, ImportError> {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches(|c| c == '-' || c == '.');
    if trimmed.is_empty() {
        return Err(ImportError::BadName(name.to_owned()));
    }
    let mut out: String = trimmed.chars().take(64).collect();
    if out.is_empty() {
        return Err(ImportError::BadName(name.to_owned()));
    }
    if out.ends_with('-') || out.ends_with('.') {
        while out.ends_with('-') || out.ends_with('.') {
            out.pop();
        }
    }
    if out.is_empty() {
        return Err(ImportError::BadName(name.to_owned()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanitised sample of the arena's same-board replay JSON: a match
    /// identifier plus the official wire `MatchSetting`. Contains no
    /// account, token, or display-name data — checked in deliberately as
    /// the converter's fixture.
    const SAMPLE: &str = include_str!("../tests/fixtures/arena/replay-sample.json");

    #[test]
    fn sample_converts_to_a_fixture_shaped_case() {
        let case = case_from_replay(SAMPLE.as_bytes(), None).expect("sample converts");
        assert_eq!(case.case_id, "fixture:arena-sample-0001");
        assert_eq!(case.source, bench::CaseSource::Fixture { name: "arena-sample-0001".into() });
        assert_eq!(case.seed, None);
        assert!(case.generation_parameters.is_none());
        // The board survives verbatim — same dimensions and day budgets.
        assert_eq!(case.setting.map.width, 8);
        assert_eq!(case.setting.day_steps, vec![16, 16]);
        // And it is evaluable (both validators ran inside the converter).
        case.sim_config().expect("thresholds valid");
    }

    #[test]
    fn explicit_name_overrides_the_embedded_identifier() {
        let case = case_from_replay(SAMPLE.as_bytes(), Some("loss vs team X!")).expect("converts");
        assert_eq!(case.case_id, "fixture:loss-vs-team-X");
    }

    #[test]
    fn emitted_case_deserializes_for_procon_eval_case_files() {
        // The offline re-run half (`procon-eval --case-files`) parses the
        // emitted file with plain `serde_json::from_str::<CaseSpec>` — the
        // round trip has to survive that, including the `source` tag shape.
        let case = case_from_replay(SAMPLE.as_bytes(), None).expect("converts");
        let json = serde_json::to_string(&case).expect("serialise");
        let back: CaseSpec = serde_json::from_str(&json).expect("evaluator can parse it");
        assert_eq!(back, case);
    }

    #[test]
    fn html_shell_fails_closed() {
        let html = b"<!DOCTYPE html><html><title>arena</title></html>";
        let err = case_from_replay(html, None).expect_err("HTML must not convert");
        assert!(matches!(err, ImportError::NotJson(_)), "{err:?}");
    }

    #[test]
    fn json_without_a_setting_fails_closed() {
        let err = case_from_replay(br#"{"hello":"world"}"#, None).expect_err("no setting anywhere");
        match err {
            ImportError::SettingNotFound { tried } => {
                assert!(tried.contains("/setting"), "{tried}");
                assert!(tried.contains("<root>"), "{tried}");
            }
            other => panic!("expected SettingNotFound, got {other:?}"),
        }
    }

    #[test]
    fn wrong_shape_setting_fails_closed() {
        // `map` present but the rest is nonsense → wire decode error.
        let err = case_from_replay(br#"{"setting":{"map":"nope"}}"#, None)
            .expect_err("bad setting must fail");
        assert!(matches!(err, ImportError::BadSetting(_)), "{err:?}");
    }

    #[test]
    fn setting_that_fails_engine_validation_fails_closed() {
        // daySeconds/daySteps length mismatch is rejected by from_wire.
        let raw = br#"{
            "matchId": "m1",
            "setting": {
                "startsAt": 1,
                "daySeconds": [30],
                "daySteps": [16, 16],
                "map": {"height": 8, "width": 8,
                        "cells": [[0,0,0,0,0,0,0,0],[0,0,0,0,0,0,0,0],[0,0,0,0,0,0,0,0],
                                  [0,0,0,0,0,0,0,0],[0,0,0,0,0,0,0,0],[0,0,0,0,0,0,0,0],
                                  [0,0,0,0,0,0,0,0],[0,0,0,0,0,0,0,0]]},
                "spots": [],
                "agents": [0, 8, 16],
                "fuelLimits": 30,
                "players": 2,
                "busyThreshold": 1,
                "jammedThreshold": 2
            }
        }"#;
        let err = case_from_replay(raw, None).expect_err("coherence must be checked");
        assert!(matches!(err, ImportError::InvalidBoard(_)), "{err:?}");
    }

    #[test]
    fn names_are_sanitised() {
        assert_eq!(sanitize_name("a b/c").expect("ok"), "a-b-c");
        assert_eq!(sanitize_name("--x--").expect("ok"), "x");
        assert!(sanitize_name("!!!").is_err());
        assert!(sanitize_name("").is_err());
        let long: String = "x".repeat(200);
        assert!(sanitize_name(&long).expect("capped").len() <= 64);
    }

    #[test]
    fn writes_a_reloadable_case_file() {
        let dir = std::env::temp_dir().join(format!("procon-arena-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let out = dir.join("case.json");
        let case = case_from_replay(SAMPLE.as_bytes(), None).expect("converts");
        write_case(&out, &case).expect("writes");
        let back = case_from_replay_file(&out, None).expect("emitted file reloads");
        assert_eq!(back, case);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
