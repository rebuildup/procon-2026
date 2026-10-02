//! CLI-level coverage for the Issue #179 reproducible-case import hook:
//! `procon arena import-case` reads the arena's same-board replay JSON and
//! emits a `CaseSpec` the Issue #178 evaluator can re-run via
//! `procon-eval --case-files`. Offline only — no network.

use bench::CaseSpec;

const SAMPLE: &str = include_str!("fixtures/arena/replay-sample.json");

fn run_import(input: &std::path::Path, output: &std::path::Path) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_procon"))
        .args([
            "arena",
            "import-case",
            "--input",
            input.to_str().expect("utf8 input"),
            "--output",
            output.to_str().expect("utf8 output"),
        ])
        .output()
        .expect("spawn procon")
}

#[test]
fn import_case_emits_a_casespec_the_evaluator_can_re_run() {
    let dir = std::env::temp_dir().join(format!("procon-arena-import-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let input = dir.join("replay.json");
    let output = dir.join("case.json");
    std::fs::write(&input, SAMPLE).expect("write sample");

    let run = run_import(&input, &output);
    assert!(run.status.success(), "stderr: {}", String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(stdout.contains("fixture:arena-sample-0001"), "{stdout}");

    // The exact shape `procon-eval --case-files` parses.
    let raw = std::fs::read_to_string(&output).expect("output written");
    let case: CaseSpec = serde_json::from_str(&raw).expect("emitted file is a CaseSpec");
    assert_eq!(case.case_id, "fixture:arena-sample-0001");
    assert_eq!(case.setting.day_steps, vec![16, 16]);
    // Evaluable: both evaluator-side validators accept it.
    case.sim_config().expect("thresholds valid");
    let team = domain::PlayerId::new("arena-import").expect("team id");
    case.to_engine_setting(&team).expect("board converts");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_case_fails_closed_on_a_non_replay_payload() {
    let dir = std::env::temp_dir().join(format!("procon-arena-import-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let input = dir.join("replay.json");
    let output = dir.join("case.json");
    // The arena's SPA shell, if someone saves it instead of the JSON.
    std::fs::write(&input, "<!DOCTYPE html><html><title>ヘキサうどん 非公式練習場</title></html>")
        .expect("write html");

    let run = run_import(&input, &output);
    assert!(!run.status.success(), "HTML must not convert into a case");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains("arena import"), "{stderr}");
    assert!(!output.exists(), "no output file on failure");

    let _ = std::fs::remove_dir_all(&dir);
}
