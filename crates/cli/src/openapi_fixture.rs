//! Load `.reference/procon37/{openapi.json,match-config.example.json}` and
//! sanity-check that the spec covers the four endpoints the coordinator
//! drives (plus the standard auth/access-time error envelopes).
//!
//! Kept as plain filesystem reads — no async, no reqwest — so the fixture
//! loader can run inside the CLI bootstrap before any runtime is up.

use std::path::Path;

use protocol::match_config::MatchSetting;

/// `OpenAPI` spec + wire `MatchSetting` example, loaded from
/// `.reference/procon37/`.
///
/// `openapi_paths` is the parsed `paths` object of the spec; we keep it as
/// [`serde_json::Value`] so we do not bind to a schema we do not own.
///
/// `match_setting` is [`Option`] because the on-disk fixture
/// (`.reference/procon37/match-config.example.json`) is a *problem*
/// description (`{ problem: { ... }, teams: [...] }`), not the wire
/// `MatchSetting` the contest server returns. The MVP keeps both
/// representations side-by-side; the wire shape lands in a later fixture
/// (see issue #C).
#[derive(Debug, Clone)]
pub struct OpenApiFixture {
    /// Parsed `MatchSetting` example, when the file matches the wire schema.
    pub match_setting: Option<MatchSetting>,
    /// Parsed `paths` object of `openapi.json`.
    pub openapi_paths: serde_json::Value,
    /// Raw `match-config.example.json` payload (preserved even when it does
    /// not parse as `MatchSetting` — the problem schema is still useful for
    /// fixtures and CLI smoke output).
    pub match_config_raw: serde_json::Value,
}

impl OpenApiFixture {
    /// Load both files from disk and parse them.
    ///
    /// # Errors
    /// Returns `Err(String)` on I/O errors or when `openapi.json` does not
    /// parse / does not contain a `paths` object. A `match-config.example.json`
    /// parse failure is **not** fatal: the file is loaded as raw JSON so the
    /// CLI can still smoke-test the fixture path.
    pub fn load(openapi_path: &Path, match_config_path: &Path) -> Result<Self, String> {
        let openapi_raw = std::fs::read_to_string(openapi_path)
            .map_err(|e| format!("read {}: {e}", openapi_path.display()))?;
        let match_config_raw_text = std::fs::read_to_string(match_config_path)
            .map_err(|e| format!("read {}: {e}", match_config_path.display()))?;

        let openapi_value: serde_json::Value =
            serde_json::from_str(&openapi_raw).map_err(|e| format!("parse openapi: {e}"))?;
        let openapi_paths = openapi_value
            .get("paths")
            .cloned()
            .ok_or_else(|| "openapi.json has no `paths` object".to_owned())?;

        let match_config_raw: serde_json::Value = serde_json::from_str(&match_config_raw_text)
            .map_err(|e| format!("parse match-config: {e}"))?;
        let match_setting = serde_json::from_value(match_config_raw.clone()).ok();

        Ok(Self { match_setting, openapi_paths, match_config_raw })
    }

    /// Cheap structural sanity-check: every endpoint the coordinator drives
    /// exists in the spec, with the standard `200 / 401 / 403` responses.
    ///
    /// This catches cases where the spec gets slimmed down (a path deleted,
    /// a response removed) without our noticing. It does **not** validate
    /// request bodies or schemas — the JSON Schemas are too noisy and we
    /// already type-check them via `protocol::match_config`.
    ///
    /// # Errors
    /// Returns `Err(String)` listing the first missing path / verb / response.
    pub fn assert_minimal_coverage(&self) -> Result<(), String> {
        const REQUIRED: &[(&str, &str, &[&str])] = &[
            ("/", "get", &["200", "401", "403"]),
            ("/", "post", &["200", "401", "403"]),
            ("/agent", "post", &["200", "401", "403"]),
            ("/setting", "get", &["200", "401", "403"]),
        ];
        for (path, verb, codes) in REQUIRED {
            let node = self
                .openapi_paths
                .get(*path)
                .ok_or_else(|| format!("openapi spec is missing path `{path}`"))?;
            let op = node
                .get(*verb)
                .ok_or_else(|| format!("openapi `{path}` is missing verb `{verb}`"))?;
            let responses = op
                .get("responses")
                .ok_or_else(|| format!("openapi `{path}` `{verb}` has no `responses`"))?;
            for code in *codes {
                if responses.get(*code).is_none() {
                    return Err(format!("openapi `{path}` `{verb}` is missing response `{code}`"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Locate the reference dir by walking up from `CARGO_MANIFEST_DIR`.
    /// Works both in the workspace and in a worktree (where the reference
    /// dir is shared with the main checkout via git's worktree linkage).
    fn reference_dir() -> std::path::PathBuf {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        // crates/cli -> crates -> workspace root. (`CARGO_MANIFEST_DIR` is the
        // crate dir, not `crates/cli/src`, so `nth(3)` would land on the
        // *parent* of the workspace root and silently skip the guard below.)
        manifest.ancestors().nth(2).expect("workspace root").join(".reference").join("procon37")
    }

    #[test]
    fn load_real_fixture_and_check_coverage() {
        let dir = reference_dir();
        let openapi = dir.join("openapi.json");
        let match_config = dir.join("match-config.example.json");
        if !openapi.exists() || !match_config.exists() {
            // In a worktree without `.reference` (e.g. shallow clone) the
            // fixture is simply absent; the assertion is informational.
            eprintln!("skipping: reference fixtures not present at {}", dir.display());
            return;
        }
        let fixture = OpenApiFixture::load(&openapi, &match_config).expect("load");
        fixture.assert_minimal_coverage().expect("coverage");
        // The on-disk fixture is the *problem* schema; the wire-shaped
        // `MatchSetting` may not be present. Fall back to raw day count.
        let day_count = fixture
            .match_setting
            .as_ref()
            .map(|s| s.day_steps.len())
            .or_else(|| {
                fixture.match_config_raw.get("problem")?.get("daySteps")?.as_array().map(Vec::len)
            })
            .unwrap_or(0);
        assert_eq!(day_count, 4, "the example problem declares 4 days");
    }
}
