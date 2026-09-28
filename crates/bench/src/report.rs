//! Durable, machine-readable result persistence.
//!
//! Two artifacts, both plain JSON on disk so any tooling can re-read them:
//!
//! - **`runs.jsonl`** — one [`RunRecord`] per line, appended in evaluation
//!   order. This is the durable evidence: it is append-only, so a crashed or
//!   partial batch keeps the runs that did complete.
//! - **`aggregate.json`** — one [`AggregateReport`], written after the batch.
//!   Derived entirely from the records, so it can always be regenerated.
//!
//! JSONL (rather than one big JSON array) for the runs on purpose: it streams,
//! it appends safely, and a single corrupt line cannot invalidate a whole
//! batch.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::aggregate::AggregateReport;
use crate::evaluate::RunRecord;

/// Errors from reading or writing reports.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("line {line} of {path} is not a valid RunRecord: {source}")]
    BadLine {
        path: String,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
}

/// Append one record to a JSONL file, creating it if needed.
///
/// # Errors
/// [`ReportError::Io`] / [`ReportError::Json`].
pub fn append_run(path: &Path, record: &RunRecord) -> Result<(), ReportError> {
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(record)?)?;
    Ok(())
}

/// Append many records to a JSONL file in order.
///
/// # Errors
/// [`ReportError::Io`] / [`ReportError::Json`].
pub fn append_runs(path: &Path, records: &[RunRecord]) -> Result<(), ReportError> {
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    for record in records {
        writeln!(file, "{}", serde_json::to_string(record)?)?;
    }
    Ok(())
}

/// Write a complete JSONL file from scratch, replacing any existing content.
///
/// # Errors
/// [`ReportError::Io`] / [`ReportError::Json`].
pub fn write_runs(path: &Path, records: &[RunRecord]) -> Result<(), ReportError> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let mut file = std::fs::File::create(path)?;
    for record in records {
        writeln!(file, "{}", serde_json::to_string(record)?)?;
    }
    Ok(())
}

/// Read a JSONL file back into records.
///
/// Blank lines are skipped. A malformed line is an error naming its line
/// number — silently dropping it would corrupt re-aggregation.
///
/// # Errors
/// [`ReportError::Io`] / [`ReportError::BadLine`].
pub fn read_runs(path: &Path) -> Result<Vec<RunRecord>, ReportError> {
    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record: RunRecord = serde_json::from_str(&line).map_err(|source| {
            ReportError::BadLine { path: path.display().to_string(), line: idx + 1, source }
        })?;
        out.push(record);
    }
    Ok(out)
}

/// Write the aggregate report as pretty JSON.
///
/// # Errors
/// [`ReportError::Io`] / [`ReportError::Json`].
pub fn write_aggregate(path: &Path, report: &AggregateReport) -> Result<(), ReportError> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let mut file = std::fs::File::create(path)?;
    writeln!(file, "{}", serde_json::to_string_pretty(report)?)?;
    Ok(())
}

/// Read an aggregate report back.
///
/// # Errors
/// [`ReportError::Io`] / [`ReportError::Json`].
pub fn read_aggregate(path: &Path) -> Result<AggregateReport, ReportError> {
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

/// A pair of output paths for one batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportPaths {
    pub runs: std::path::PathBuf,
    pub aggregate: std::path::PathBuf,
}

impl ReportPaths {
    /// Standard layout inside `dir`: `runs.jsonl` + `aggregate.json`.
    #[must_use]
    pub fn in_dir(dir: impl AsRef<Path>) -> Self {
        let dir = dir.as_ref();
        Self { runs: dir.join("runs.jsonl"), aggregate: dir.join("aggregate.json") }
    }

    /// Write both artifacts.
    ///
    /// # Errors
    /// [`ReportError`] from either write.
    pub fn write(
        &self,
        records: &[RunRecord],
        report: &AggregateReport,
    ) -> Result<(), ReportError> {
        write_runs(&self.runs, records)?;
        write_aggregate(&self.aggregate, report)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluate::{RunRecord, RunStatus};

    fn record(id: &str) -> RunRecord {
        RunRecord {
            candidate_id: id.to_owned(),
            case_id: "fixture:small-8x8".to_owned(),
            seed: Some(7),
            generation_parameters: Some(serde_json::json!({"k": 1})),
            source_commit_sha: "cafebabe".to_owned(),
            team: "player-a".to_owned(),
            variety: 3,
            daily_variety_sum: 6,
            total_balls: 9,
            response_time_us: 1234,
            elapsed_us: 5678,
            timeout: false,
            status: RunStatus::Success,
            scored: true,
            status_detail: None,
            last_acquisition_at: Some((1, 2)),
            illegal_action_count: 0,
            fuel_short_count: 0,
            day_budget_exceeded_days: Vec::new(),
            daily_response_us: vec![600, 634],
            daily_varieties: vec![2, 3],
            agent_kinds: None,
            agent_ball_counts: None,
            oracle_refill: false,
        }
    }

    #[test]
    fn runs_round_trip_through_jsonl() {
        let dir = std::env::temp_dir().join("procon-bench-report-round-trip");
        let _ = std::fs::remove_dir_all(&dir);
        let paths = ReportPaths::in_dir(&dir);
        let records = vec![record("a"), record("b")];
        paths
            .write(&records, &AggregateReport::build(records.clone(), Some("a")).unwrap())
            .unwrap();

        let back = read_runs(&paths.runs).unwrap();
        assert_eq!(back, records);

        let agg = read_aggregate(&paths.aggregate).unwrap();
        assert_eq!(agg.runs.len(), 2);
        assert_eq!(agg.baseline_candidate_id.as_deref(), Some("a"));
    }

    #[test]
    fn append_keeps_existing_runs() {
        let dir = std::env::temp_dir().join("procon-bench-report-append");
        let _ = std::fs::remove_dir_all(&dir);
        let paths = ReportPaths::in_dir(&dir);
        write_runs(&paths.runs, &[record("first")]).unwrap();
        append_run(&paths.runs, &record("second")).unwrap();
        let back = read_runs(&paths.runs).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].candidate_id, "first");
        assert_eq!(back[1].candidate_id, "second");
    }

    #[test]
    fn malformed_line_reports_its_line_number() {
        let dir = std::env::temp_dir().join("procon-bench-report-bad-line");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("runs.jsonl");
        let good = serde_json::to_string(&record("ok")).unwrap();
        std::fs::write(&path, format!("{good}\nnot json\n")).unwrap();
        match read_runs(&path) {
            Err(ReportError::BadLine { line, .. }) => assert_eq!(line, 2),
            other => panic!("expected BadLine, got {other:?}"),
        }
    }

    #[test]
    fn blank_lines_are_skipped() {
        let dir = std::env::temp_dir().join("procon-bench-report-blank");
        let _ = std::fs::remove_dir_all(&dir);
        let paths = ReportPaths::in_dir(&dir);
        write_runs(&paths.runs, &[record("a")]).unwrap();
        let mut f = std::fs::OpenOptions::new().append(true).open(&paths.runs).unwrap();
        writeln!(f).unwrap();
        let back = read_runs(&paths.runs).unwrap();
        assert_eq!(back.len(), 1);
    }
}
