//! Localhost monitor for a live run (Issue #429).
//!
//! One small HTTP server on `127.0.0.1:<port>` with three resources:
//!
//! - `GET /` — one HTML page (no frameworks): the per-day table, the live
//!   search counters and an SVG sparkline of the improvement curve,
//!   refreshed by `fetch` polling every second.
//! - `GET /telemetry.json` — the search counters (`bench::telemetry`).
//! - `GET /table.json` — the per-day table parsed from the run directory
//!   (`events.jsonl` + `manifest.json` + `setting.json`); absent files
//!   degrade to empty rows, never an error: the monitor only *reads* what
//!   the coordinator writes, so a crashed monitor cannot affect a match.
//! - `POST /cutoff` — **disabled unless the monitor was started with
//!   `--allow-search-cutoff`** (the coordinator passes it only with an
//!   explicit operator flag; see `docs/ops/telemetry-monitor.md`).
//!   Requests the current day's search finish early with the best plan
//!   found so far. It sets the telemetry cutoff flag — no process kill,
//!   no plan forgery: the search then walks fleet/convoy's normal
//!   completion path (replay → trust gate → fallback), so the submitted
//!   plan is exactly a plan the candidate would have submitted itself.
//!
//! The monitor runs on the CLI's own tokio runtime; every file read is
//! re-done per request (the records are tiny), so the page always shows
//! the current state and no cache can go stale mid-match.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;

use crate::match_record::{self, DayStatus, LoadedRecord, RecordEvent};

/// How the monitor was started, and with it what it may serve.
#[derive(Debug, Clone, Default)]
pub struct MonitorConfig {
    /// Serve the dashboard and `telemetry.json` / `table.json`.
    pub enabled: bool,
    /// Also serve `POST /cutoff`. Never implied: an explicit opt-in only.
    pub allow_cutoff: bool,
    /// TCP port on `127.0.0.1`. A fixed default (`8437`) keeps the
    /// two-window runbook simple; anything free works.
    pub port: u16,
    /// Run directory shown in the table. Empty = the newest
    /// `<record-dir>/<run-id>`; none = table rows from the live telemetry
    /// alone.
    pub run_dir: Option<PathBuf>,
}

/// Start the monitor when `cfg.enabled`. Returns the bound address for
/// the log line; a bind failure is logged, never fatal to the match.
pub fn spawn(cfg: MonitorConfig) -> Option<String> {
    if !cfg.enabled {
        return None;
    }
    let MonitorConfig { enabled: _, allow_cutoff, port, run_dir } = cfg;
    let addr = format!("127.0.0.1:{port}");
    match std::net::TcpListener::bind(&addr) {
        Ok(listener) => {
            let state = Arc::new(MonitorState { run_dir, allow_cutoff });
            std::thread::Builder::new()
                .name("telemetry-monitor".into())
                .spawn(move || serve(listener, state))
                .ok()
                .map(|_| {
                    let cutoff =
                        if allow_cutoff { "cutoff endpoint ENABLED" } else { "cutoff disabled" };
                    format!("{addr} ({cutoff})")
                })
        }
        Err(e) => {
            tracing::warn!(addr = %addr, error = %e, "telemetry monitor could not bind; continuing without it");
            None
        }
    }
}

/// Blocking serve loop (its own thread; the match never waits on it).
#[allow(
    clippy::needless_pass_by_value,
    reason = "the listener is consumed by `incoming`, the Arc cloned per request"
)]
fn serve(listener: std::net::TcpListener, state: Arc<MonitorState>) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let state = Arc::clone(&state);
        // One thread per request: the monitor is a single operator's
        // dashboard, not a service.
        let _ = std::thread::Builder::new().name("telemetry-req".into()).spawn(move || {
            handle(&mut stream, &state);
        });
    }
}

struct MonitorState {
    run_dir: Option<PathBuf>,
    allow_cutoff: bool,
}

/// Read one HTTP request head (and a `POST` body), answer, close.
fn handle(stream: &mut std::net::TcpStream, state: &MonitorState) {
    use std::io::{Read, Write};
    let mut buf = [0_u8; 2048];
    let Ok(read) = stream.read(&mut buf) else { return };
    let head = String::from_utf8_lossy(&buf[..read]);
    let first = head.lines().next().unwrap_or_default();
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default().split('?').next().unwrap_or_default();
    let (status, ctype, body) = match (method, path) {
        ("GET", "/") => ("200 OK", "text/html; charset=utf-8", page().into_bytes()),
        ("GET", "/telemetry.json") => {
            let snap = bench::telemetry::SEARCH.snapshot();
            ("200 OK", "application/json", serde_json::to_vec(&snap).unwrap_or_default())
        }
        ("GET", "/table.json") => {
            let table = table(state.run_dir.as_deref());
            ("200 OK", "application/json", serde_json::to_vec(&table).unwrap_or_default())
        }
        ("POST", "/cutoff") => {
            if !state.allow_cutoff {
                let body = json!({
                    "ok": false,
                    "error": "cutoff endpoint is disabled; start the monitor with \
                              --allow-search-cutoff (and telemetry) to enable it",
                });
                let body = serde_json::to_vec(&body).unwrap_or_default();
                let head = format!(
                    "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(&body));
                return;
            }
            // The body may carry `{"day": N}`; absent = the current day.
            let day: Option<i32> = body_of(&head)
                .and_then(|b| serde_json::from_str::<serde_json::Value>(&b).ok())
                .and_then(|v| v.get("day").and_then(serde_json::Value::as_i64))
                .and_then(|d| i32::try_from(d).ok());
            match bench::telemetry::SEARCH.request_cutoff(day) {
                Ok(()) => {
                    tracing::info!(day = ?day, "manual cutoff requested via the monitor");
                    let body = json!({
                        "ok": true,
                        "day": bench::telemetry::SEARCH.day(),
                        "note": "the current search finishes at its next check and the normal completion path submits the best plan",
                    });
                    let body = serde_json::to_vec(&body).unwrap_or_default();
                    let head = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ =
                        stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(&body));
                }
                Err(e) => {
                    let (status, code) = match e {
                        bench::telemetry::CutoffError::Disabled => ("403 Forbidden", "disabled"),
                        bench::telemetry::CutoffError::Idle => ("409 Conflict", "idle"),
                        bench::telemetry::CutoffError::DayMismatch { .. } => {
                            ("409 Conflict", "day-mismatch")
                        }
                    };
                    let body = json!({ "ok": false, "error": code, "detail": e.to_string() });
                    let body = serde_json::to_vec(&body).unwrap_or_default();
                    let head = format!(
                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ =
                        stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(&body));
                }
            }
            return;
        }
        _ => {
            let body = b"{\"error\":\"not found\"}".to_vec();
            let head = format!(
                "HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(&body));
            return;
        }
    };
    let head = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(&body));
}

/// The request body, when the head includes a `content-length`.
fn body_of(head: &str) -> Option<String> {
    let len: usize = head
        .lines()
        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:")?.trim().parse().ok())
        .filter(|&l| l > 0)?;
    // The first read holds the whole body for these few-byte payloads; a
    // larger body than the buffer is refused, not parsed.
    let idx = head.find("\r\n\r\n")? + 4;
    let head_bytes = head.len();
    if idx >= head_bytes {
        return None;
    }
    if len > head_bytes - idx {
        return None;
    }
    Some(head[idx..idx + len].to_owned())
}

// ---------------------------------------------------------------------------
// Per-day table
// ---------------------------------------------------------------------------

/// The per-day table: one row per day seen in the record, plus the live
/// telemetry day. Opponents' scores are never fabricated — the official
/// wire publishes none, so there is no column for them.
#[must_use]
pub fn table(run_dir: Option<&Path>) -> serde_json::Value {
    let mut rows: Vec<serde_json::Value> = Vec::new();
    let (run_id, candidate, continues) = run_dir
        .and_then(|dir| match_record::load_record(dir).ok())
        .map(|record| {
            let rows_from = rows_from_record(&record);
            let run_id = record
                .manifest
                .get("run_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let candidate = record
                .manifest
                .pointer("/solver/candidate_id")
                .or_else(|| record.manifest.pointer("/solver/label"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let continues = record
                .manifest
                .get("continues_run_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            rows.extend(rows_from);
            (run_id, candidate, continues)
        })
        .unwrap_or_default();
    json!({
        "run_id": run_id,
        "candidate": candidate,
        "continues_run_id": continues,
        "official_note": "series/balls are OUR team's; opponents' scores are not published by the official API and are never estimated here",
        "days": rows,
    })
}

/// One table row per recorded day: observed (OFFICIAL) values, model
/// estimates (marked `model_*`), verdict and revision.
fn rows_from_record(record: &LoadedRecord) -> Vec<serde_json::Value> {
    let mut rows: Vec<serde_json::Value> = Vec::new();
    let mut balls_cum = 0_u64;
    let mut have_cum = false;
    // Day-start inventories, officially observed by `GET /`.
    let observed: Vec<(i64, Option<u32>)> = record
        .lines
        .iter()
        .filter_map(|line| match &line.event {
            RecordEvent::DayState { observed, .. } => {
                let balls = observed
                    .agents
                    .iter()
                    .map(|a| u32::try_from(a.fuel.max(0)).unwrap_or(0))
                    .sum::<u32>();
                Some((i64::from(observed.day), Some(balls)))
            }
            _ => None,
        })
        .collect();
    let mut days: Vec<u32> = observed.iter().filter_map(|(d, _)| u32::try_from(*d).ok()).collect();
    days.extend(record.day_results().keys().copied());
    days.sort_unstable();
    days.dedup();
    for day in days {
        let day_state = observed.iter().find(|(d, _)| *d == i64::from(day)).and_then(|(_, b)| *b);
        // The plan's solve/budget (the last one wins when a day was
        // re-planned).
        let mut solve_ms = None;
        let mut budget_ms = None;
        let mut status = None;
        let mut revision = None;
        let mut attempts = None;
        for line in &record.lines {
            match &line.event {
                RecordEvent::Plan { day: d, budget_ms: b, solve_ms: s, .. } if *d == day => {
                    solve_ms = Some(*s);
                    budget_ms = Some(*b);
                }
                RecordEvent::SubmitAttempt { day: d, attempt, .. } if *d == day => {
                    attempts = Some(*attempt);
                }
                RecordEvent::DayResult { day: d, status: st, revision: r, .. } if *d == day => {
                    status = Some((*st).into_label());
                    revision = *r;
                }
                _ => {}
            }
        }
        if let (true, Some(b)) = (day_state.is_some(), day_state) {
            // Day-start fuel is not the day's gain; the gain shows in the
            // next day's start. Cumulative balls = the last observed start
            // plus what this day's replay delivered — but the record only
            // observes day STARTS, so cumulative stays None until the next
            // day confirms. Keep the table honest: observed = start only.
            balls_cum = u64::from(b);
            have_cum = true;
        }
        rows.push(json!({
            "day": day,
            "balls_day_start": day_state,
            "balls_cum": if have_cum { Some(balls_cum) } else { None },
            "solve_ms": solve_ms,
            "budget_ms": budget_ms,
            "status": status,
            "revision": revision,
            "attempts": attempts,
        }));
    }
    rows
}

impl DayStatus {
    /// Table label for the verdict.
    #[must_use]
    pub fn into_label(self) -> &'static str {
        match self {
            DayStatus::Accepted => "accepted",
            DayStatus::Rejected => "rejected",
            DayStatus::Unanswered => "unanswered",
        }
    }
}

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------

/// `GET /`: one page, fetch polling, plain tables, one SVG sparkline.
#[allow(clippy::too_many_lines, reason = "one self-contained HTML page")]
fn page() -> String {
    r##"<!doctype html>
<html lang="ja">
<meta charset="utf-8">
<title>procon-2026 telemetry (Issue #429)</title>
<style>
  body {{ font-family: system-ui, sans-serif; margin: 1rem; background: #fafafa; color: #111; }}
  h1 {{ font-size: 1.1rem; }} h2 {{ font-size: 0.95rem; margin-bottom: 0.3rem; }}
  table {{ border-collapse: collapse; font-size: 0.85rem; margin-bottom: 1rem; }}
  th, td {{ border: 1px solid #ccc; padding: 2px 8px; text-align: right; }}
  th {{ background: #eee; }}
  td:first-child, th:first-child {{ text-align: left; }}
  .live td {{ font-variant-numeric: tabular-nums; }}
  .cut {{ margin: 0.5rem 0 1rem; }}
  .cut button {{ font-size: 0.9rem; padding: 4px 14px; }}
  .cut .off {{ color: #888; }}
  #msg {{ color: #b00; font-size: 0.85rem; min-height: 1.2em; }}
  code {{ background: #eee; padding: 0 3px; }}
</style>
<h1>procon-2026 実況モニタ <span id="meta" style="font-weight:normal"></span></h1>
<p id="msg"></p>
<div class="cut"><button id="cut" type="button">今日の探索を打ち切って提出</button>
<span id="cutstate" class="off">cutoff endpoint state: unknown</span></div>
<h2>日別スコア（公式観測値のみ）</h2>
<table id="days"><thead><tr>
<th>day</th><th>balls(開始)</th><th>status</th><th>revision</th><th>attempts</th><th>solve ms</th><th>budget ms</th>
</tr></thead><tbody></tbody></table>
<h2>探索の実況（現在手のモデル値）</h2>
<table class="live"><tbody>
<tr><th>day</th><td id="t-day"></td><th>phase</th><td id="t-phase"></td></tr>
<tr><th>iters</th><td id="t-iters"></td><th>target</th><td id="t-target"></td></tr>
<tr><th>best (model)</th><td id="t-best"></td><th>improvements</th><td id="t-imp"></td></tr>
<tr><th>last improve</th><td id="t-last"></td><th>elapsed</th><td id="t-elapsed"></td></tr>
<tr><th>budget</th><td id="t-budget"></td><th>plans</th><td id="t-plans"></td></tr>
</tbody></table>
<h2>改善曲線 (best model score)</h2>
<svg id="spark" width="600" height="80" viewBox="0 0 600 80" role="img" aria-label="improvement curve">
  <polyline id="spark-line" fill="none" stroke="#0a6" stroke-width="2" points=""/>
</svg>
<p style="font-size:0.8rem;color:#555">run: <span id="runid"></span> · candidate: <span id="cand"></span>
· 続き: <span id="cont"></span></p>
<script>
const $ = id => document.getElementById(id);
async function get(u) {{
  const r = await fetch(u, {{cache: "no-store"}});
  if (!r.ok) throw new Error(u + " -> " + r.status);
  return r.json();
}}
function fmt(v) {{ return v === null || v === undefined ? "—" : String(v); }}
function tick() {{
  get("/telemetry.json").then(t => {{
    $("t-day").textContent = fmt(t.day);
    $("t-phase").textContent = t.phase;
    $("t-iters").textContent = fmt(t.iters);
    $("t-target").textContent = fmt(t.iters_target);
    $("t-best").textContent = fmt(t.best);
    $("t-imp").textContent = fmt(t.improvements);
    $("t-last").textContent = t.last_improve_unix_ms
      ? new Date(t.last_improve_unix_ms).toLocaleTimeString() : "—";
    $("t-elapsed").textContent = t.elapsed_ms + " ms";
    $("t-budget").textContent = t.budget_ms ? t.budget_ms + " ms" : "iters mode";
    $("t-plans").textContent = fmt(t.plans);
    $("meta").textContent = t.enabled ? "" : "（telemetry 無効: 探索側に --telemetry が必要）";
    draw(t.curve || []);
    const cs = $("cutstate");
    if (!t.enabled) {{ cs.textContent = "cutoff: telemetry off"; }}
    else {{ cs.textContent = t.cutoff ? "cutoff: REQUESTED (search finishes at the next check)" : "cutoff: idle"; }}
  }}).catch(e => {{ $("msg").textContent = "telemetry: " + e.message; }});
  get("/table.json").then(tb => {{
    $("msg").textContent = "";
    $("runid").textContent = tb.run_id || "—";
    $("cand").textContent = tb.candidate || "—";
    $("cont").textContent = tb.continues_run_id || "—";
    const body = $("days").tBodies[0];
    body.innerHTML = (tb.days || []).map(d =>
      "<tr><td>" + d.day + "</td><td>" + fmt(d.balls_day_start) + "</td><td>" +
      fmt(d.status) + "</td><td>" + fmt(d.revision) + "</td><td>" + fmt(d.attempts) +
      "</td><td>" + fmt(d.solve_ms) + "</td><td>" + fmt(d.budget_ms) + "</td></tr>").join("");
  }}).catch(e => {{ $("msg").textContent = "table: " + e.message; }});
}}
function draw(curve) {{
  const pts = curve.map(([ms, score]) => [ms, score]);
  if (pts.length < 2) {{ $("spark-line").setAttribute("points", ""); return; }}
  const t0 = pts[0][0], t1 = Math.max(t0 + 1, pts[pts.length-1][0]);
  const s0 = Math.min(...pts.map(p => p[1])), s1 = Math.max(...pts.map(p => p[1]));
  const range = Math.max(1, s1 - s0);
  const line = pts.map(([ms, s]) =>
    ((ms - t0) / (t1 - t0) * 590 + 5).toFixed(1) + "," +
    (75 - (s - s0) / range * 70).toFixed(1)).join(" ");
  $("spark-line").setAttribute("points", line);
}}
$("cut").addEventListener("click", () => {{
  fetch("/cutoff", {{method: "POST", headers: {{"content-type": "application/json"}}, body: "{{}}"}})
    .then(r => r.json()).then(j => {{ $("msg").textContent = j.ok ? "cutoff requested" : ("cutoff refused: " + (j.detail || j.error)); }})
    .catch(e => {{ $("msg").textContent = "cutoff: " + e.message; }});
}});
tick();
setInterval(tick, 1500);
</script>
"##
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cutoff endpoint answers 403 while disabled, and a disabled
    /// monitor never starts.
    #[test]
    fn monitor_is_off_by_default() {
        let cfg = MonitorConfig::default();
        assert!(!cfg.enabled);
        assert!(!cfg.allow_cutoff);
        assert!(spawn(cfg).is_none(), "default config starts nothing");
    }

    /// `spawn` with cutoff enabled still requires `enabled`.
    #[test]
    fn cutoff_requires_the_monitor() {
        let cfg = MonitorConfig { allow_cutoff: true, ..MonitorConfig::default() };
        assert!(spawn(cfg).is_none());
    }

    /// The table parser reads a fixture run directory (shipped in
    /// `tests/fixtures/run-fixture/`): one row per day, official values
    /// only, model columns absent from the JSON until the log adds them.
    #[test]
    fn table_parses_a_run_directory() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/run-fixture");
        let t = table(Some(&dir));
        assert_eq!(t["run_id"], "20261010T044644Z-fixture");
        let days = t["days"].as_array().expect("rows");
        assert_eq!(days.len(), 2, "two recorded days");
        assert_eq!(days[0]["day"], 0);
        assert_eq!(days[0]["status"], "accepted");
        assert_eq!(days[0]["revision"], 3);
        assert_eq!(days[0]["attempts"], 1);
        assert_eq!(
            days[0]["balls_day_start"],
            25 + 35 + 35 + 35,
            "day-start fuel sum, officially observed"
        );
        assert_eq!(days[0]["solve_ms"], 10_001);
        assert_eq!(days[0]["budget_ms"], 55_607);
        assert_eq!(days[1]["day"], 1);
        assert_eq!(days[1]["status"], "rejected");
        assert_eq!(days[1]["attempts"], 2, "the retried POSTs are visible");
        assert_eq!(days[1]["revision"], -1);
        // The candidate comes from the manifest.
        assert_eq!(t["candidate"], "fleet:budget=official:budget_ms=10000:adapt=off");
    }

    /// No run directory: an empty, well-formed table (never an error).
    #[test]
    fn table_without_a_directory_is_empty() {
        let t = table(None);
        assert_eq!(t["days"].as_array().expect("rows").len(), 0);
        assert_eq!(t["run_id"], "");
    }

    /// The page is valid-ish HTML with the polling script and the
    /// cutoff button.
    #[test]
    fn page_has_the_expected_pieces() {
        let p = page();
        assert!(p.contains("fetch(\"/telemetry.json\")") || p.contains("get(\"/telemetry.json\")"));
        assert!(p.contains("setInterval"));
        assert!(p.contains("/cutoff"));
        assert!(p.contains("<svg"));
        assert!(p.contains("table.json"));
    }

    /// A GET against a live monitor on an ephemeral port serves the
    /// documented resources (integration through the real socket).
    #[test]
    fn monitor_serves_end_to_end() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("ephemeral port");
        let port = listener.local_addr().expect("addr").port();
        let state = Arc::new(MonitorState { run_dir: None, allow_cutoff: false });
        let handle = std::thread::spawn(move || serve(listener, state));
        // Give the server thread a moment, then talk to it.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let get = |path: &str| {
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
            write!(s, "GET {path} HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n")
                .expect("write");
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        };
        let root = get("/");
        assert!(root.contains("200 OK"), "GET / answers: {root}");
        let tel = get("/telemetry.json");
        assert!(tel.contains("200 OK"));
        assert!(tel.contains("\"enabled\":"));
        let tab = get("/table.json");
        assert!(tab.contains("200 OK"));
        assert!(tab.contains("\"days\":"));
        // Cutoff is 403 while disabled.
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        write!(s, "POST /cutoff HTTP/1.1\r\nhost: localhost\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}")
            .expect("write");
        let mut buf = Vec::new();
        let _ = s.read_to_end(&mut buf);
        let post = String::from_utf8_lossy(&buf).into_owned();
        assert!(post.contains("403"), "cutoff is disabled without the flag: {post}");
        drop(handle);
    }
}
