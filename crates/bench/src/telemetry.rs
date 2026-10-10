//! Live search telemetry for the venue run (Issue #429).
//!
//! The fleet / convoy searches already honour a wall-clock budget; this
//! module lets an operator *watch* them: iterations so far, the best model
//! score, how often it improved and when it last did — without slowing the
//! search down. The contract:
//!
//! - **Off by default.** Nothing is written until [`Telemetry::enable`]
//!   turns the global [`SEARCH`] on (the CLI does this behind its
//!   `--telemetry` flag / `PROCON_TELEMETRY` env). Disabled, every call is
//!   one relaxed `AtomicBool` load.
//! - **The hot loop only touches atomics.** [`Telemetry::tick`] runs every
//!   256 iterations (the stride the wall-clock check already used): a few
//!   relaxed stores, no locks, no allocation, no formatting.
//! - **Sampling is coarse on purpose.** Per-iteration logging would measure
//!   itself; a 256-stride sample plus per-improvement updates gives the
//!   improvement curve without perturbing a 10 s window.
//! - **The manual cutoff is one `AtomicBool`.** [`Telemetry::request_cutoff`]
//!   sets it (validated against the current day); `search()` breaks on it
//!   exactly as on the wall-clock deadline and the normal replay /
//!   trust-gate / fallback path continues unchanged. The flag is consumed by
//!   the *current* day's search only: [`Telemetry::day_begin`] clears it.
//!
//! The reader side (HTTP endpoint, dashboard) lives in the CLI
//! (`crates/cli/src/monitor.rs`); it only ever *reads* these atomics and
//! files other components wrote.
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// One static set of search counters, shared by every chain and every
/// candidate in the process. Chains race on the atomics on purpose: `best`
/// is a `fetch_max` (the best of the running chains), `iters` a sum, and a
/// torn 64-bit read cannot happen on the aligned atomics the targets use.
pub static SEARCH: Telemetry = Telemetry::new();

/// Ring-buffer capacity of the improvement curve. A 20 s window sampled
/// every 256 iterations plus one point per improvement fits comfortably;
/// beyond capacity the oldest points are overwritten.
const RING: usize = 256;

/// Labels set once when telemetry is turned on. Cheap to keep behind a
/// [`OnceLock`]: the hot loop never reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryMeta {
    /// The match-record run id (`--record-dir`), or `"(no record)"`.
    pub run_id: String,
    /// The candidate id in play (e.g. `convoy:budget=official:...`).
    pub candidate: String,
}

/// Phase of the current day, for the dashboard's one-line status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum Phase {
    /// No search running (before the first day, between days, after the
    /// match).
    Idle = 0,
    /// A `decide` is running.
    Planning = 1,
    /// The day's plan is done (awaiting the next day).
    Done = 2,
}

impl Phase {
    fn from_u64(v: u64) -> Self {
        match v {
            1 => Phase::Planning,
            2 => Phase::Done,
            _ => Phase::Idle,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Planning => "planning",
            Phase::Done => "done",
        }
    }
}

/// Why a cutoff request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutoffError {
    /// Telemetry (and with it the endpoint) is off.
    Disabled,
    /// The request named a day other than the one being planned.
    DayMismatch { current: i64 },
    /// Nothing is being planned right now.
    Idle,
}

impl std::fmt::Display for CutoffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("telemetry is off"),
            Self::Idle => f.write_str("no day is being planned right now"),
            Self::DayMismatch { current } => {
                write!(f, "that day is not being planned (current: {current})")
            }
        }
    }
}

/// Global search telemetry. All fields are atomics; see the module docs for
/// the contract.
pub struct Telemetry {
    /// Master switch; every other write is behind this load.
    enabled: AtomicBool,
    /// Labels set at enable time.
    meta: OnceLock<TelemetryMeta>,
    /// Day currently being planned (`-1` = none).
    day: AtomicI64,
    /// Nesting depth of `decide` calls: convoy embeds fleet (`fleet.decide`
    /// inside its own `decide`), and only the outermost call owns the day
    /// boundary — a nested `day_begin` would otherwise wipe a pending
    /// cutoff request from the operator.
    depth: AtomicU64,
    /// [`Phase`] of the current day.
    phase: AtomicU64,
    /// UNIX ms when the current day's planning began.
    start_unix_ms: AtomicU64,
    /// Wall-clock budget of the current day (0 = iters mode).
    budget_ms: AtomicU64,
    /// Iteration target of the current day (0 = wall mode).
    iters_target: AtomicU64,
    /// Iterations completed so far (summed over chains, sampled per stride).
    iters: AtomicU64,
    /// Best model score seen this day (max over chains).
    best: AtomicI64,
    /// Times the global best strictly rose this day.
    improvements: AtomicU64,
    /// UNIX ms of the last strict rise.
    last_improve_unix_ms: AtomicU64,
    /// Day plans (replayed candidates) built this day.
    plans: AtomicU64,
    /// The manual cutoff request; cleared by [`Telemetry::day_begin`].
    cutoff: AtomicBool,
    /// Write cursor of [`Self::pts`] (wraps at [`RING`]).
    head: AtomicU64,
    /// Improvement-curve points, packed `(elapsed_ms: u32) << 32 |
    /// (score as f32 bits)`. Oldest overwritten after [`RING`] points.
    pts: [AtomicU64; RING],
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::new()
    }
}

impl Telemetry {
    pub const fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            meta: OnceLock::new(),
            day: AtomicI64::new(-1),
            depth: AtomicU64::new(0),
            phase: AtomicU64::new(0),
            start_unix_ms: AtomicU64::new(0),
            budget_ms: AtomicU64::new(0),
            iters_target: AtomicU64::new(0),
            iters: AtomicU64::new(0),
            best: AtomicI64::new(0),
            improvements: AtomicU64::new(0),
            last_improve_unix_ms: AtomicU64::new(0),
            plans: AtomicU64::new(0),
            cutoff: AtomicBool::new(false),
            head: AtomicU64::new(0),
            pts: [const { AtomicU64::new(0) }; RING],
        }
    }

    /// Turn telemetry on for this process and label it. Idempotent; the
    /// first label wins.
    pub fn enable(&self, meta: TelemetryMeta) {
        let _ = self.meta.set(meta);
        self.enabled.store(true, Ordering::Relaxed);
    }

    /// Whether the hot loop should write anything.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// The labels set at enable time.
    #[must_use]
    pub fn meta(&self) -> Option<&TelemetryMeta> {
        self.meta.get()
    }

    /// Begin a day's planning: resets the per-day counters and **clears the
    /// cutoff flag**, so a request can only ever cut the day it was made
    /// during. Nested calls (convoy's embedded fleet) keep the outer
    /// boundary: only the outermost `decide` resets the day.
    pub fn day_begin(&self, day: u32, budget_ms: Option<u64>, iters_target: u64) {
        if !self.enabled() {
            return;
        }
        if self.depth.fetch_add(1, Ordering::Relaxed) > 0 {
            return; // nested: the outer decide owns the day boundary
        }
        self.day.store(i64::from(day), Ordering::Relaxed);
        self.phase.store(Phase::Planning as u64, Ordering::Relaxed);
        self.start_unix_ms.store(unix_ms(), Ordering::Relaxed);
        self.budget_ms.store(budget_ms.unwrap_or(0), Ordering::Relaxed);
        self.iters_target.store(iters_target, Ordering::Relaxed);
        self.iters.store(0, Ordering::Relaxed);
        self.best.store(0, Ordering::Relaxed);
        self.improvements.store(0, Ordering::Relaxed);
        self.last_improve_unix_ms.store(unix_ms(), Ordering::Relaxed);
        self.plans.store(0, Ordering::Relaxed);
        self.head.store(0, Ordering::Relaxed);
        self.cutoff.store(false, Ordering::Relaxed);
    }

    /// Mark the current day's planning finished (outermost call only).
    pub fn day_end(&self) {
        if !self.enabled() {
            return;
        }
        // MSRV 1.85: no `try_update`; a fetch-sub. Every caller is a
        // `decide` frame, so the count returns to zero in LIFO order.
        if self.depth.fetch_sub(1, Ordering::Relaxed) > 1 {
            return; // nested: the outer decide closes the day
        }
        self.depth.store(0, Ordering::Relaxed);
        self.phase.store(Phase::Done as u64, Ordering::Relaxed);
        self.cutoff.store(false, Ordering::Relaxed);
    }

    /// Day currently being planned (`-1` when none).
    #[must_use]
    pub fn day(&self) -> i64 {
        self.day.load(Ordering::Relaxed)
    }

    /// The pending manual cutoff, if any.
    #[must_use]
    pub fn cutoff_pending(&self) -> bool {
        self.cutoff.load(Ordering::Relaxed)
    }

    /// Per-stride hot-loop sample: records the iteration count and one
    /// curve point, and reports whether a manual cutoff was requested.
    ///
    /// Call sites must already be on the 256 stride (both search modes
    /// stride there); this stays one relaxed load when disabled.
    ///
    /// Returns `true` when the caller should break out of the search.
    pub fn tick(&self, _iter: u64, best_score: i64) -> bool {
        if !self.enabled() {
            return false;
        }
        self.iters.fetch_add(256, Ordering::Relaxed);
        let start = self.start_unix_ms.load(Ordering::Relaxed);
        #[allow(clippy::cast_possible_truncation, reason = "elapsed is clamped to u32::MAX")]
        let elapsed = u32::try_from(unix_ms().saturating_sub(start)).unwrap_or(u32::MAX);
        self.push_point(elapsed, best_score);
        self.cutoff.load(Ordering::Relaxed)
    }

    /// Record a strict best-so-far improvement. Called only from the
    /// `score > best` branch, so a few relaxed stores on rare events.
    pub fn improve(&self, best_score: i64) {
        if !self.enabled() {
            return;
        }
        let prev = self.best.fetch_max(best_score, Ordering::Relaxed);
        if best_score > prev {
            self.improvements.fetch_add(1, Ordering::Relaxed);
            let now = unix_ms();
            self.last_improve_unix_ms.store(now, Ordering::Relaxed);
            let start = self.start_unix_ms.load(Ordering::Relaxed);
            #[allow(clippy::cast_possible_truncation, reason = "elapsed is clamped to u32::MAX")]
            let elapsed = u32::try_from(now.saturating_sub(start)).unwrap_or(u32::MAX);
            self.push_point(elapsed, best_score);
        }
    }

    /// Count one replayed day plan (fleet / convoy `decide` success).
    pub fn plan_built(&self) {
        if self.enabled() {
            self.plans.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Operator request to finish the current day's search early and submit
    /// the best plan found (Issue #429). The only write path into the
    /// search: it sets the same flag the wall-clock deadline shares the
    /// loop with — no kill, no plan forgery; the normal completion path
    /// (replay → trust gate → fallback) runs unchanged.
    ///
    /// `day` must match the day being planned (`None` accepts the current
    /// day), so a stale button press cannot cut the wrong day's search.
    ///
    /// # Errors
    /// [`CutoffError::Disabled`] while telemetry is off,
    /// [`CutoffError::Idle`] between days, [`CutoffError::DayMismatch`]
    /// for a stale day.
    pub fn request_cutoff(&self, day: Option<i32>) -> Result<(), CutoffError> {
        if !self.enabled() {
            return Err(CutoffError::Disabled);
        }
        let current = self.day.load(Ordering::Relaxed);
        if self.phase.load(Ordering::Relaxed) != Phase::Planning as u64 {
            return Err(CutoffError::Idle);
        }
        if let Some(day) = day {
            if i64::from(day) != current {
                return Err(CutoffError::DayMismatch { current });
            }
        }
        self.cutoff.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Push one curve point (wraps around after [`RING`]).
    fn push_point(&self, elapsed_ms: u32, score: i64) {
        // f32 keeps the point one u64; the curve is a sparkline, not an
        // evidence record — the coordinator.log line stays the SoT.
        #[allow(clippy::cast_precision_loss, reason = "sparkline fidelity")]
        let bits = (score as f32).to_bits();
        let packed = (u64::from(elapsed_ms) << 32) | u64::from(bits);
        let slot = self.head.fetch_add(1, Ordering::Relaxed) % u64::try_from(RING).expect("const");
        #[allow(clippy::cast_possible_truncation, reason = "slot < RING = 256")]
        let slot = slot as usize;
        self.pts[slot].store(packed, Ordering::Relaxed);
    }

    /// Curve points so far, oldest first: `(elapsed_ms, score)`.
    #[must_use]
    pub fn points(&self) -> Vec<(u64, i64)> {
        if !self.enabled() {
            return Vec::new();
        }
        let ring = u64::try_from(RING).expect("const");
        let head = self.head.load(Ordering::Relaxed);
        let kept = head.min(ring);
        // Points written while we read may be torn by the packing race
        // (a wrapped overwrite); the second head read bounds the damage.
        let head_after = self.head.load(Ordering::Relaxed);
        let kept = if head_after >= head { kept } else { head_after.min(ring) };
        let start = head.saturating_sub(kept);
        (start..head)
            .map(|k| {
                #[allow(clippy::cast_possible_truncation, reason = "slot < RING = 256")]
                let packed = self.pts[(k % ring) as usize].load(Ordering::Relaxed);
                let elapsed = packed >> 32;
                #[allow(clippy::cast_precision_loss, reason = "sparkline fidelity")]
                #[allow(clippy::cast_possible_truncation, reason = "packed low half")]
                let score = f32::from_bits(packed as u32) as i64;
                (elapsed, score)
            })
            .collect()
    }

    /// One JSON-shaped reading of the current counters.
    #[must_use]
    pub fn snapshot(&self) -> TelemetrySnapshot {
        let elapsed = unix_ms().saturating_sub(self.start_unix_ms.load(Ordering::Relaxed));
        TelemetrySnapshot {
            enabled: self.enabled(),
            run_id: self.meta().map_or_else(|| "(no record)".to_owned(), |m| m.run_id.clone()),
            candidate: self.meta().map_or_else(String::new, |m| m.candidate.clone()),
            day: self.day(),
            phase: Phase::from_u64(self.phase.load(Ordering::Relaxed)).label().to_owned(),
            start_unix_ms: self.start_unix_ms.load(Ordering::Relaxed),
            elapsed_ms: elapsed,
            budget_ms: self.budget_ms.load(Ordering::Relaxed),
            iters_target: self.iters_target.load(Ordering::Relaxed),
            iters: self.iters.load(Ordering::Relaxed),
            best: self.best.load(Ordering::Relaxed),
            improvements: self.improvements.load(Ordering::Relaxed),
            last_improve_unix_ms: self.last_improve_unix_ms.load(Ordering::Relaxed),
            plans: self.plans.load(Ordering::Relaxed),
            cutoff: self.cutoff_pending(),
            curve: self.points(),
        }
    }
}

/// JSON reading of [`Telemetry`] (serde shape of `GET /telemetry.json`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TelemetrySnapshot {
    pub enabled: bool,
    pub run_id: String,
    pub candidate: String,
    pub day: i64,
    pub phase: String,
    pub start_unix_ms: u64,
    pub elapsed_ms: u64,
    pub budget_ms: u64,
    pub iters_target: u64,
    pub iters: u64,
    pub best: i64,
    pub improvements: u64,
    pub last_improve_unix_ms: u64,
    pub plans: u64,
    pub cutoff: bool,
    /// `(elapsed_ms, best score)` of the improvement curve, oldest first.
    pub curve: Vec<(u64, i64)>,
}

fn unix_ms() -> u64 {
    #[allow(clippy::cast_possible_truncation, reason = "UNIX ms fits u64 until 584M AD")]
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    /// `tick` is a no-op load when disabled and never reports a cutoff.
    #[test]
    fn disabled_telemetry_is_inert() {
        let tel = Telemetry::new();
        assert!(!tel.enabled());
        assert!(!tel.tick(256, 100));
        tel.improve(50);
        tel.plan_built();
        assert_eq!(tel.points().len(), 0);
        let snap = tel.snapshot();
        assert!(!snap.enabled);
        assert_eq!(snap.iters, 0);
        assert_eq!(snap.best, 0);
        assert!(matches!(tel.request_cutoff(None), Err(CutoffError::Disabled)));
        assert!(!tel.cutoff_pending());
    }

    /// `day_begin` clears a pending cutoff: a request can only cut the day
    /// it was made during.
    #[test]
    fn day_begin_clears_cutoff() {
        let tel = Telemetry::new();
        tel.enable(TelemetryMeta { run_id: "r".into(), candidate: "c".into() });
        tel.day_begin(3, Some(10_000), 0);
        assert!(tel.request_cutoff(Some(3)).is_ok());
        assert!(tel.cutoff_pending());
        tel.day_end();
        assert!(!tel.cutoff_pending(), "day_end also clears the request");
        tel.day_begin(4, None, 1_000);
        assert!(!tel.cutoff_pending(), "a new day starts with no cutoff");
        assert_eq!(tel.day(), 4);
        assert_eq!(tel.snapshot().iters_target, 1_000);
    }

    /// A cutoff request for another day is refused and leaves no flag.
    #[test]
    fn cutoff_is_day_scoped() {
        let tel = Telemetry::new();
        tel.enable(TelemetryMeta { run_id: "r".into(), candidate: "c".into() });
        tel.day_begin(0, Some(10_000), 0);
        assert_eq!(tel.request_cutoff(Some(5)), Err(CutoffError::DayMismatch { current: 0 }));
        assert!(!tel.cutoff_pending());
        assert!(tel.request_cutoff(Some(0)).is_ok());
        // Between days nothing can be cut.
        tel.day_end();
        assert_eq!(tel.request_cutoff(None), Err(CutoffError::Idle));
    }

    /// Curve points wrap at [`RING`] and decode oldest-first.
    #[test]
    fn ring_wraps_and_decodes() {
        let tel = Telemetry::new();
        tel.enable(TelemetryMeta { run_id: "r".into(), candidate: "c".into() });
        tel.day_begin(0, None, 0);
        for score in 0..i64::from(u16::try_from(RING).expect("const")) + 32 {
            tel.push_point(0, score * 100);
        }
        let pts = tel.points();
        assert_eq!(pts.len(), RING);
        // First kept point is the (RING + 32) - RING = 32nd.
        assert_eq!(pts[0].1, 3_200);
        #[allow(clippy::cast_possible_wrap, reason = "RING = 256")]
        let ring = RING as i64;
        assert_eq!(pts.last().expect("kept").1, (ring + 31) * 100);
    }

    /// `improve` counts only strict rises of the global best.
    #[test]
    fn improvements_count_strict_rises() {
        let tel = Telemetry::new();
        tel.enable(TelemetryMeta { run_id: "r".into(), candidate: "c".into() });
        tel.day_begin(0, None, 0);
        tel.improve(500);
        tel.improve(500);
        tel.improve(700);
        tel.improve(600);
        let snap = tel.snapshot();
        assert_eq!(snap.best, 700);
        assert_eq!(snap.improvements, 2);
        assert!(snap.last_improve_unix_ms > 0);
    }

    /// The packing survives the round trip for scores beyond f32's integer
    /// range (the sparkline rounds; nothing else consumes the points).
    #[test]
    fn point_packing_keeps_shape() {
        let tel = Telemetry::new();
        tel.enable(TelemetryMeta { run_id: "r".into(), candidate: "c".into() });
        tel.day_begin(0, None, 0);
        tel.push_point(12_345, 1_234_567_890);
        let pts = tel.points();
        assert_eq!(pts.len(), 1);
        assert_eq!(pts[0].0, 12_345);
        // f32 fidelity: exact to ~7 digits, which is what a sparkline needs
        // (the coordinator.log line stays the evidence record).
        #[allow(clippy::cast_precision_loss, reason = "the point is a sparkline value")]
        let want = 1_234_567_890_i64;
        let got = pts[0].1;
        assert!(
            (got - want).abs() * 1_000_000 <= want.abs(),
            "f32 round-trip lost more than 1e-6 relative: {got} vs {want}"
        );
    }

    /// Concurrent chains: `fetch_max` keeps the max, the iteration counter
    /// sums, and no torn point breaks the curve length bound.
    #[test]
    fn concurrent_chains_agree() {
        static SHARED: Telemetry = Telemetry::new();
        SHARED.enable(TelemetryMeta { run_id: "r".into(), candidate: "c".into() });
        SHARED.day_begin(0, Some(5_000), 0);
        std::thread::scope(|s| {
            for k in 0..4i64 {
                s.spawn(move || {
                    for i in 0..1000u64 {
                        let score = k * 1_000_000 + i64::try_from(i).unwrap_or(0) * 10;
                        if i % 256 == 0 {
                            SHARED.tick(i, score);
                        }
                        SHARED.improve(score);
                    }
                });
            }
        });
        let snap = SHARED.snapshot();
        assert_eq!(snap.best, 3 * 1_000_000 + 999 * 10);
        // 4 threads x up to 4 strides each; the exact sum races, but every
        // chain passes at least one stride.
        assert!(snap.iters >= 256 * 4, "iters={}", snap.iters);
        assert!(snap.curve.len() <= RING);
    }

    /// `unix_ms` is monotone enough for the curve and never zero here.
    #[test]
    fn unix_ms_is_sane() {
        static ONCE: AtomicU64 = AtomicU64::new(0);
        let a = ONCE.fetch_add(0, Ordering::Relaxed);
        let now = unix_ms();
        assert!(now > 1_700_000_000_000);
        assert!(a <= now || a == 0);
    }
}
