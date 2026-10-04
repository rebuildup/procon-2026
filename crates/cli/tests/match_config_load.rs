//! Integration tests for `procon match-config` subcommands.
//!
//! The tests run the `procon` binary as a subprocess against temporary JSON
//! fixtures so we exercise the full CLI surface — argument parsing, JSON
//! loading, validation, and stdout/stderr formatting.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// Resolve the `procon` binary path. Cargo sets `CARGO_BIN_EXE_procon` for
/// integration tests, but fall back to a manual `target/debug/procon` lookup
/// so the file can also be exercised with `cargo run` from the repo root.
fn procon_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_procon") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("debug")
        .join("procon")
}

/// Build a minimal-but-valid match configuration JSON value.
fn valid_config() -> Value {
    json!({
        "days": 5,
        "daySteps": 200,
        "agents": [
            {"id": "agent-1", "kind": "patrol"},
            {"id": "agent-2", "kind": "patrol"},
            {"id": "agent-3", "kind": "supply"},
        ],
        "notes": "fixture",
    })
}

/// Counter so parallel test threads never collide on a fixture path.
static SEQ: AtomicU64 = AtomicU64::new(0);

/// Allocate a unique fixture directory under the OS temp dir and write a JSON
/// payload to it. Returns the directory (so the caller can clean it up) and
/// the path to the fixture file.
fn write_fixture(name: &str, value: &Value) -> (PathBuf, PathBuf) {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("procon-cli-fixtures-{nanos}-{seq}"));
    fs::create_dir_all(&dir).expect("create fixture dir");
    let path = dir.join(name);
    fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    (dir, path)
}

#[test]
fn load_pretty_prints_match_config() {
    let (_dir, path) = write_fixture("ok.json", &valid_config());

    let output = Command::new(procon_bin())
        .args(["match-config", "load"])
        .arg(&path)
        .output()
        .expect("run procon match-config load");

    assert!(
        output.status.success(),
        "exit={:?}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"days\""), "stdout was: {stdout}");
    assert!(stdout.contains("\"daySteps\""), "stdout was: {stdout}");
    assert!(stdout.contains("\"agents\""), "stdout was: {stdout}");
    assert!(stdout.contains("\"agent-1\""), "stdout was: {stdout}");
    assert!(stdout.contains("\"notes\""), "extras should be preserved, got: {stdout}");
}

#[test]
fn validate_accepts_valid_fixture() {
    let (_dir, path) = write_fixture("ok.json", &valid_config());

    let output = Command::new(procon_bin())
        .args(["match-config", "validate"])
        .arg(&path)
        .output()
        .expect("run procon match-config validate");

    assert!(
        output.status.success(),
        "exit={:?}\nstderr={}\nstdout={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"ok\": true"), "stdout was: {stdout}");
}

#[test]
fn validate_rejects_too_few_agents() {
    let bad = json!({
        "days": 5,
        "daySteps": 200,
        "agents": [
            {"id": "agent-1", "kind": "patrol"},
            {"id": "agent-2", "kind": "patrol"},
        ]
    });
    let (_dir, path) = write_fixture("bad.json", &bad);

    let output = Command::new(procon_bin())
        .args(["match-config", "validate"])
        .arg(&path)
        .output()
        .expect("run procon match-config validate");

    assert!(!output.status.success(), "expected non-zero exit, got success");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"ok\": false"), "stdout was: {stdout}");
    assert!(stdout.contains("agents count"), "stdout was: {stdout}");
}

#[test]
fn validate_rejects_zero_day_steps() {
    let bad = json!({
        "days": 5,
        "daySteps": 0,
        "agents": [
            {"id": "agent-1", "kind": "patrol"},
            {"id": "agent-2", "kind": "patrol"},
            {"id": "agent-3", "kind": "supply"},
        ]
    });
    let (_dir, path) = write_fixture("zero.json", &bad);

    let output = Command::new(procon_bin())
        .args(["match-config", "validate"])
        .arg(&path)
        .output()
        .expect("run procon match-config validate");

    assert!(!output.status.success(), "expected non-zero exit");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("`daySteps`"), "stdout was: {stdout}");
}

#[test]
fn validate_rejects_duplicate_ids() {
    let bad = json!({
        "days": 5,
        "daySteps": 200,
        "agents": [
            {"id": "dup", "kind": "patrol"},
            {"id": "dup", "kind": "supply"},
            {"id": "agent-3", "kind": "patrol"},
        ]
    });
    let (_dir, path) = write_fixture("dup.json", &bad);

    let output = Command::new(procon_bin())
        .args(["match-config", "validate"])
        .arg(&path)
        .output()
        .expect("run procon match-config validate");

    assert!(!output.status.success(), "expected non-zero exit");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("duplicate agent id"), "stdout was: {stdout}");
}

#[test]
fn help_lists_subcommands() {
    let output = Command::new(procon_bin())
        .args(["match-config", "--help"])
        .output()
        .expect("run procon match-config --help");

    assert!(output.status.success(), "stderr={}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("load"), "stdout was: {stdout}");
    assert!(stdout.contains("validate"), "stdout was: {stdout}");
}

#[test]
fn load_reports_missing_file() {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("procon-cli-missing-{nanos}-{seq}"));
    fs::create_dir_all(&dir).unwrap();
    let missing = dir.join("does-not-exist.json");

    let output = Command::new(procon_bin())
        .args(["match-config", "load"])
        .arg(&missing)
        .output()
        .expect("run procon match-config load missing");

    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does-not-exist.json") || stderr.contains("error"),
        "stderr was: {stderr}",
    );
}
