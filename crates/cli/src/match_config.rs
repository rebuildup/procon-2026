//! `procon match-config` — load and validate a contest match configuration JSON.
//!
//! Match configurations describe the per-match parameters used by the contest
//! platform (e.g. duration, agent counts). The schema is intentionally
//! permissive at the wire level — `protocol::match_config` is not yet merged
//! (Issue #14) — so this module parses `serde_json::Value` and enforces a
//! minimal set of invariants:
//!
//! - `days` is an integer in `1..=MAX_DAYS`.
//! - `daySteps` is an integer in `1..=MAX_DAY_STEPS`.
//! - `agents` is an array of `{id, kind}` objects with unique non-empty ids,
//!   and length in `3..=8` (per `docs/contest/strategy.md`).
//!
//! Once `protocol::match_config` ships, this module can be refactored to
//! derive `Deserialize` and delegate to it; the public function signatures
//! will not change.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

/// Inclusive lower bound on the agent count.
pub const MIN_AGENTS: usize = 3;
/// Inclusive upper bound on the agent count.
pub const MAX_AGENTS: usize = 8;
/// Inclusive lower bound on `days`.
pub const MIN_DAYS: u32 = 1;
/// Inclusive upper bound on `days`.
pub const MAX_DAYS: u32 = 30;
/// Inclusive lower bound on `daySteps`.
pub const MIN_DAY_STEPS: u32 = 1;
/// Inclusive upper bound on `daySteps`.
pub const MAX_DAY_STEPS: u32 = 1024;

/// Agent descriptor as it appears in the configuration file.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentSpec {
    /// Stable identifier (must be non-empty and unique within the match).
    pub id: String,
    /// Agent kind label (e.g. `patrol`, `supply`). Free-form until the
    /// official schema lands.
    pub kind: String,
}

/// Parsed match configuration.
///
/// Only the fields currently understood by the operator tooling are exposed.
/// Unrecognised keys are preserved in [`MatchConfig::extras`] so that `load`
/// can round-trip them.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MatchConfig {
    pub days: u32,
    pub day_steps: u32,
    pub agents: Vec<AgentSpec>,
    /// Any additional JSON fields present in the source, preserved verbatim.
    pub extras: serde_json::Map<String, Value>,
}

/// Outcome of [`validate`] — structured so callers (and tests) can format
/// either pass or fail cases uniformly.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ValidationReport {
    pub ok: bool,
    pub errors: Vec<String>,
}

impl ValidationReport {
    /// Construct a successful report.
    #[must_use]
    pub fn ok() -> Self {
        Self { ok: true, errors: Vec::new() }
    }

    /// Add an error message and flip `ok` to false.
    pub fn push(&mut self, msg: impl Into<String>) {
        self.ok = false;
        self.errors.push(msg.into());
    }
}

/// Read and parse a match-configuration file from disk.
///
/// Returns the parsed [`MatchConfig`] plus the raw [`Value`] tree (so callers
/// can re-serialise the original document verbatim when debugging).
///
/// # Errors
/// Returns an [`anyhow::Error`] if the file cannot be read or the JSON is
/// malformed; structural validation is left to [`validate`].
pub fn load(path: &Path) -> Result<(MatchConfig, Value)> {
    let raw = fs::read(path).with_context(|| format!("read match config {}", path.display()))?;
    let value: Value = serde_json::from_slice(&raw)
        .with_context(|| format!("parse match config {}", path.display()))?;
    let cfg = parse(&value).with_context(|| format!("validate match config {}", path.display()))?;
    Ok((cfg, value))
}

/// Parse an already-loaded [`Value`] into a [`MatchConfig`], enforcing the
/// minimal structural invariants.
///
/// # Errors
/// Returns an [`anyhow::Error`] listing every structural problem found. The
/// message is human-readable; programmatic checks should use [`validate`].
pub fn parse(value: &Value) -> Result<MatchConfig> {
    let report = validate(value);
    if !report.ok {
        anyhow::bail!("invalid match config: {}", report.errors.join("; "));
    }
    Ok(build(value).expect("validate passed"))
}

/// Run consistency checks against an already-parsed [`Value`] tree.
///
/// The report aggregates all detected issues so operators can fix them in a
/// single pass instead of chasing one error at a time.
#[must_use]
pub fn validate(value: &Value) -> ValidationReport {
    let mut report = ValidationReport::ok();
    let Some(obj) = value.as_object() else {
        report.push("top-level value must be a JSON object");
        return report;
    };

    if let Some(v) = obj.get("days") {
        parse_u32_in_range(v, MIN_DAYS, MAX_DAYS, "days", &mut report);
    } else {
        report.push("missing required field `days`");
    }

    if let Some(v) = obj.get("daySteps") {
        parse_u32_in_range(v, MIN_DAY_STEPS, MAX_DAY_STEPS, "daySteps", &mut report);
    } else {
        report.push("missing required field `daySteps`");
    }

    match obj.get("agents") {
        Some(Value::Array(items)) => {
            validate_agents(items, &mut report);
            // Cross-field check: warn-free, but agent-count range still enforced
            // even when individual entries are invalid, to surface the count
            // independently.
            if items.len() < MIN_AGENTS || items.len() > MAX_AGENTS {
                report.push(format!(
                    "agents count {} is outside the allowed range {}-{}",
                    items.len(),
                    MIN_AGENTS,
                    MAX_AGENTS,
                ));
            }
        }
        Some(_) => report.push("`agents` must be a JSON array"),
        None => report.push("missing required field `agents`"),
    }

    report
}

fn parse_u32_in_range(
    value: &Value,
    min: u32,
    max: u32,
    field: &str,
    report: &mut ValidationReport,
) -> Option<u32> {
    let Some(n) = value.as_number() else {
        report.push(format!("`{field}` must be an integer"));
        return None;
    };
    let Some(raw) = n.as_u64() else {
        report.push(format!("`{field}` must be a non-negative integer"));
        return None;
    };
    let v = u32::try_from(raw).unwrap_or_else(|_| max.saturating_add(1));
    if !(min..=max).contains(&v) {
        report.push(format!("`{field}` must be in {min}..={max}, got {v}"));
        return None;
    }
    Some(v)
}

fn validate_agents(items: &[Value], report: &mut ValidationReport) {
    let mut seen_ids: Vec<&str> = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            report.push(format!("agents[{idx}] must be a JSON object"));
            continue;
        };
        let id = match obj.get("id") {
            Some(Value::String(s)) => {
                if s.is_empty() {
                    report.push(format!("agents[{idx}].id must be non-empty"));
                    continue;
                }
                s.as_str()
            }
            Some(_) => {
                report.push(format!("agents[{idx}].id must be a string"));
                continue;
            }
            None => {
                report.push(format!("agents[{idx}] missing required field `id`"));
                continue;
            }
        };
        if seen_ids.contains(&id) {
            report.push(format!("duplicate agent id: {id}"));
            continue;
        }
        seen_ids.push(id);
        match obj.get("kind") {
            Some(Value::String(s)) => {
                if s.is_empty() {
                    report.push(format!("agents[{idx}].kind must be non-empty"));
                }
            }
            Some(_) => report.push(format!("agents[{idx}].kind must be a string")),
            None => report.push(format!("agents[{idx}] missing required field `kind`")),
        }
    }
}

fn build(value: &Value) -> Option<MatchConfig> {
    let obj = value.as_object()?;
    let days = obj.get("days")?.as_u64().and_then(|v| u32::try_from(v).ok())?;
    let day_steps = obj.get("daySteps")?.as_u64().and_then(|v| u32::try_from(v).ok())?;
    let agents_v = obj.get("agents")?.as_array()?;
    let mut agents = Vec::with_capacity(agents_v.len());
    for item in agents_v {
        let o = item.as_object()?;
        let id = o.get("id")?.as_str()?.to_owned();
        let kind = o.get("kind")?.as_str()?.to_owned();
        agents.push(AgentSpec { id, kind });
    }
    let mut extras = obj.clone();
    extras.remove("days");
    extras.remove("daySteps");
    extras.remove("agents");
    Some(MatchConfig { days, day_steps, agents, extras })
}

/// Convenience: pretty-print a JSON value with a trailing newline.
pub fn pretty_print(value: &Value) -> Result<String> {
    serde_json::to_string_pretty(value).context("format match config as JSON")
}

/// Resolve `path` to an absolute form for user-facing error messages.
pub fn display_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(path),
            Err(_) => path.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_ok() -> Value {
        json!({
            "days": 5,
            "daySteps": 200,
            "agents": [
                {"id": "a", "kind": "patrol"},
                {"id": "b", "kind": "patrol"},
                {"id": "c", "kind": "supply"},
            ],
            "notes": "fixture",
        })
    }

    #[test]
    fn validate_accepts_sample() {
        let report = validate(&sample_ok());
        assert!(report.ok, "errors: {:?}", report.errors);
    }

    #[test]
    fn parse_returns_config() {
        let value = sample_ok();
        let cfg = parse(&value).unwrap();
        assert_eq!(cfg.days, 5);
        assert_eq!(cfg.day_steps, 200);
        assert_eq!(cfg.agents.len(), 3);
        assert_eq!(cfg.extras.get("notes"), Some(&Value::String("fixture".into())));
    }

    #[test]
    fn validate_rejects_too_few_agents() {
        let value = json!({
            "days": 5,
            "daySteps": 200,
            "agents": [
                {"id": "a", "kind": "patrol"},
                {"id": "b", "kind": "patrol"},
            ]
        });
        let report = validate(&value);
        assert!(!report.ok);
        assert!(report.errors.iter().any(|e| e.contains("agents count")));
    }

    #[test]
    fn validate_rejects_too_many_agents() {
        let mut agents = Vec::new();
        for i in 0..10 {
            agents.push(json!({"id": format!("a{i}"), "kind": "patrol"}));
        }
        let value = json!({"days": 1, "daySteps": 1, "agents": agents});
        let report = validate(&value);
        assert!(!report.ok);
        assert!(report.errors.iter().any(|e| e.contains("agents count")));
    }

    #[test]
    fn validate_rejects_zero_days() {
        let value = json!({
            "days": 0,
            "daySteps": 200,
            "agents": sample_ok()["agents"].clone()
        });
        let report = validate(&value);
        assert!(!report.ok);
        assert!(report.errors.iter().any(|e| e.contains("`days`")));
    }

    #[test]
    fn validate_rejects_duplicate_ids() {
        let value = json!({
            "days": 5,
            "daySteps": 200,
            "agents": [
                {"id": "a", "kind": "patrol"},
                {"id": "a", "kind": "supply"},
                {"id": "b", "kind": "patrol"},
            ]
        });
        let report = validate(&value);
        assert!(!report.ok);
        assert!(report.errors.iter().any(|e| e.contains("duplicate agent id")));
    }

    #[test]
    fn validate_rejects_missing_fields() {
        let value = json!({"days": 5});
        let report = validate(&value);
        assert!(!report.ok);
        assert!(report.errors.iter().any(|e| e.contains("`daySteps`")));
        assert!(report.errors.iter().any(|e| e.contains("`agents`")));
    }

    #[test]
    fn validate_rejects_non_object_root() {
        let value = json!([1, 2, 3]);
        let report = validate(&value);
        assert!(!report.ok);
        assert!(report.errors.iter().any(|e| e.contains("top-level value")));
    }

    #[test]
    fn pretty_print_round_trips() {
        let value = sample_ok();
        let out = pretty_print(&value).unwrap();
        assert!(out.contains("\"days\""));
        assert!(out.contains("\"agents\""));
    }
}
