/**
 * Live mode wire types + pollers.
 *
 * Two wire formats coexist:
 *
 * 1. Legacy `LiveMatchState` / `LiveMatchSetting` — the original coordinator
 *    snapshot shape (per-turn `endsAt`, `day`, `agents`). Kept around for
 *    other pages that may still import these types.
 * 2. New `LiveSnapshot` — the CLI's `MatchStateSnapshot` (PR #160), written
 *    atomically by `--state-output` and polled by the Live page. This is
 *    the canonical format going forward.
 *
 * The CLI does not yet expose HTTP endpoints, so both readers fetch from
 * static fixtures Vite serves out of `public/`. The snapshot URL is
 * `/match-state.json` so the CLI can write directly into `public/` while
 * the developer is running `bun run dev`.
 */

import { createSignal, type Accessor } from 'solid-js';

/** Per-day step budget + time window for the match. */
export interface LiveMatchSetting {
  readonly startsAt: number;
  readonly daySeconds: ReadonlyArray<number>;
  readonly daySteps: ReadonlyArray<number>;
  readonly map: {
    readonly height: number;
    readonly width: number;
    /** Row-major `CellType` integers; shape mirrors `protocol::match_config::Map.cells`. */
    readonly cells: ReadonlyArray<unknown>;
    readonly spots: ReadonlyArray<unknown>;
  };
  readonly agents: ReadonlyArray<unknown>;
  readonly spots: ReadonlyArray<unknown>;
}

/** A single agent's runtime state at poll time. */
export interface LiveAgentState {
  readonly id: number;
  /** `0` → patrol, `1` → supply (mirrors `protocol::match_config::AgentKind`). */
  readonly kind: number;
  /** Hex coordinate (axial `q,r`); the page converts to `(col,row)` for the viewer. */
  readonly pos: { readonly q: number; readonly r: number };
  readonly stock: number;
}

/** Per-turn snapshot returned by the coordinator / fixture. */
export interface LiveMatchState {
  readonly endsAt: number;
  readonly day: number;
  readonly agents: ReadonlyArray<LiveAgentState>;
  readonly others: ReadonlyArray<unknown>;
  readonly traffics: ReadonlyArray<unknown>;
}

export type LiveStatus = 'waiting' | 'running' | 'ended';

/** Default fixture path served by Vite out of `packages/frontend/public/`. */
export const MATCH_STATE_URL = '/match-state.example.json';
export const MATCH_SETTING_URL = '/match-setting.example.json';

/** CLI `--state-output` snapshot, served from `packages/frontend/public/match-state.json`. */
export const LIVE_SNAPSHOT_URL = '/match-state.json';

/** Default polling cadence (1 second per spec). */
export const DEFAULT_POLL_MS = 1000;

/**
 * CLI's `MatchStateSnapshot` (PR #160). camelCase, schemaVersion=1.
 * Written atomically by `procon coordinator --state-output PATH` after each
 * phase transition.
 */
export type LiveSnapshotStatus = 'waiting' | 'running' | 'completed' | 'rejected' | 'error';

export interface LiveSnapshot {
  readonly schemaVersion: 1;
  readonly server: string;
  /** `--solver` value, e.g. `loop` / `wait`. */
  readonly solver: string;
  readonly status: LiveSnapshotStatus;
  /** Current day index (0-based). `null` until the first day has started. */
  readonly day: number | null;
  /** Total day count from `setting.daySteps.len()`. */
  readonly daysTotal: number;
  /** Last accepted revision; `null` before the first acceptance. */
  readonly revision: number | null;
  /** Per-day revision array (one entry per accepted day). */
  readonly revisions: ReadonlyArray<number>;
  /** Unix seconds at which the snapshot was written. */
  readonly lastUpdateUnix: number;
  /** Populated only when `status` is `rejected` or `error`. */
  readonly error: string | null;
}

const SNAPSHOT_STATUSES: ReadonlyArray<LiveSnapshotStatus> = [
  'waiting',
  'running',
  'completed',
  'rejected',
  'error',
];

/**
 * Type-guard for the CLI snapshot. Strict enough to reject schema drift
 * (e.g. `schemaVersion: 2`) so the page can fall back to "no data".
 */
export function isLiveSnapshot(value: unknown): value is LiveSnapshot {
  if (!value || typeof value !== 'object') return false;
  const v = value as Record<string, unknown>;
  if (v['schemaVersion'] !== 1) return false;
  if (typeof v['server'] !== 'string') return false;
  if (typeof v['solver'] !== 'string') return false;
  const status = v['status'];
  if (typeof status !== 'string') return false;
  if (!SNAPSHOT_STATUSES.includes(status as LiveSnapshotStatus)) return false;
  if (v['day'] !== null && typeof v['day'] !== 'number') return false;
  if (typeof v['daysTotal'] !== 'number') return false;
  if (v['revision'] !== null && typeof v['revision'] !== 'number') return false;
  if (!Array.isArray(v['revisions'])) return false;
  if (typeof v['lastUpdateUnix'] !== 'number') return false;
  if (v['error'] !== null && typeof v['error'] !== 'string') return false;
  return true;
}

/**
 * Fetch a single CLI snapshot. Returns `null` on transport failure or
 * shape mismatch so the caller can keep the previous value rendered
 * instead of flashing an error overlay every second.
 */
export async function fetchLiveSnapshot(signal?: AbortSignal): Promise<LiveSnapshot | null> {
  try {
    const cacheBuster = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    const url = `${LIVE_SNAPSHOT_URL}?t=${cacheBuster}`;
    const res = signal ? await fetch(url, { signal }) : await fetch(url);
    if (!res.ok) return null;
    const body: unknown = await res.json();
    return isLiveSnapshot(body) ? body : null;
  } catch {
    // AbortError and network errors both land here; treat both as "no update".
    return null;
  }
}

/**
 * Type-guard for the legacy wire payload. `unknown[]` fields are accepted
 * unchecked; the page only consumes `endsAt`, `day`, and `agents`.
 */
function isLiveMatchState(value: unknown): value is LiveMatchState {
  if (!value || typeof value !== 'object') return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v['endsAt'] === 'number' && typeof v['day'] === 'number' && Array.isArray(v['agents'])
  );
}

function isLiveMatchSetting(value: unknown): value is LiveMatchSetting {
  if (!value || typeof value !== 'object') return false;
  const v = value as Record<string, unknown>;
  const map = v['map'];
  if (!map || typeof map !== 'object') return false;
  const m = map as Record<string, unknown>;
  return (
    typeof v['startsAt'] === 'number' &&
    Array.isArray(v['daySeconds']) &&
    Array.isArray(v['daySteps']) &&
    typeof m['height'] === 'number' &&
    typeof m['width'] === 'number' &&
    Array.isArray(m['cells']) &&
    Array.isArray(m['spots'])
  );
}

/**
 * Fetch a single snapshot of the legacy match state. Returns `null` on
 * transport failure or shape mismatch.
 */
export async function fetchLiveMatchState(signal?: AbortSignal): Promise<LiveMatchState | null> {
  try {
    const cacheBuster = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    const url = `${MATCH_STATE_URL}?t=${cacheBuster}`;
    const res = signal ? await fetch(url, { signal }) : await fetch(url);
    if (!res.ok) return null;
    const body: unknown = await res.json();
    return isLiveMatchState(body) ? body : null;
  } catch {
    return null;
  }
}

/**
 * The setting is static for the duration of a match, so callers fetch it
 * once on mount and feed it into the map adapter.
 */
export async function fetchLiveMatchSetting(
  signal?: AbortSignal,
): Promise<LiveMatchSetting | null> {
  try {
    const res = signal
      ? await fetch(MATCH_SETTING_URL, { signal })
      : await fetch(MATCH_SETTING_URL);
    if (!res.ok) return null;
    const body: unknown = await res.json();
    return isLiveMatchSetting(body) ? body : null;
  } catch {
    return null;
  }
}

/**
 * Classify the current phase.
 *
 * With `daySeconds` available:
 *   - `waiting` — the day's deadline is further out than the configured window
 *     (i.e. the day has not yet started).
 *   - `running` — within the configured window.
 *   - `ended`   — `endsAt` is in the past.
 *
 * Without `daySeconds` only the running/ended split is computable; the
 * function returns `running` for any non-ended snapshot in that case.
 */
export function classifyStatus(
  state: LiveMatchState,
  nowMs: number,
  daySeconds?: ReadonlyArray<number>,
): LiveStatus {
  const remaining = state.endsAt - nowMs;
  if (remaining <= 0) return 'ended';
  if (daySeconds && state.day >= 0 && state.day < daySeconds.length) {
    const dayWindowMs = (daySeconds[state.day] ?? 0) * 1000;
    if (dayWindowMs > 0 && remaining > dayWindowMs) return 'waiting';
  }
  return 'running';
}

export interface LivePoller {
  /** Reactive accessor for the most recent successful snapshot. */
  readonly state: Accessor<LiveMatchState | null>;
  /** Reactive accessor for the derived phase. `waiting` until the first poll resolves. */
  readonly status: Accessor<LiveStatus>;
  /** Start polling. Safe to call more than once; subsequent calls are no-ops. */
  start(): void;
  /** Stop polling and abort any in-flight fetch. */
  stop(): void;
}

/**
 * Construct a polling loop around `fetchLiveMatchState`. The first poll runs
 * synchronously on `start()`; subsequent polls fire on `intervalMs`.
 */
export function createLivePoller(intervalMs: number = DEFAULT_POLL_MS): LivePoller {
  const [state, setState] = createSignal<LiveMatchState | null>(null);
  const [status, setStatus] = createSignal<LiveStatus>('waiting');

  let controller: AbortController | null = null;
  let timer: ReturnType<typeof setInterval> | null = null;

  async function tick(signal: AbortSignal): Promise<void> {
    const next = await fetchLiveMatchState(signal);
    if (signal.aborted) return;
    if (next) {
      setState(next);
      setStatus(classifyStatus(next, Date.now()));
    }
  }

  return {
    state,
    status,
    start(): void {
      if (timer !== null) return;
      controller = new AbortController();
      const signal = controller.signal;
      // Fire the first poll immediately so the UI doesn't sit empty for a
      // full interval after mount.
      void tick(signal);
      timer = setInterval(() => {
        if (!controller) return;
        void tick(controller.signal);
      }, intervalMs);
    },
    stop(): void {
      if (timer !== null) {
        clearInterval(timer);
        timer = null;
      }
      if (controller) {
        controller.abort();
        controller = null;
      }
    },
  };
}

export interface LiveSnapshotPoller {
  /** Reactive accessor for the most recent successful snapshot. `null` until the first poll resolves. */
  readonly snapshot: Accessor<LiveSnapshot | null>;
  /** Start polling. Safe to call more than once; subsequent calls are no-ops. */
  start(): void;
  /** Stop polling and abort any in-flight fetch. */
  stop(): void;
}

/**
 * Construct a polling loop around `fetchLiveSnapshot`. The first poll runs
 * synchronously on `start()`; subsequent polls fire on `intervalMs`.
 */
export function createLiveSnapshotPoller(intervalMs: number = DEFAULT_POLL_MS): LiveSnapshotPoller {
  const [snapshot, setSnapshot] = createSignal<LiveSnapshot | null>(null);

  let controller: AbortController | null = null;
  let timer: ReturnType<typeof setInterval> | null = null;

  async function tick(signal: AbortSignal): Promise<void> {
    const next = await fetchLiveSnapshot(signal);
    if (signal.aborted) return;
    if (next) setSnapshot(next);
  }

  return {
    snapshot,
    start(): void {
      if (timer !== null) return;
      controller = new AbortController();
      const signal = controller.signal;
      // Fire the first poll immediately so the UI doesn't sit empty for a
      // full interval after mount.
      void tick(signal);
      timer = setInterval(() => {
        if (!controller) return;
        void tick(controller.signal);
      }, intervalMs);
    },
    stop(): void {
      if (timer !== null) {
        clearInterval(timer);
        timer = null;
      }
      if (controller) {
        controller.abort();
        controller = null;
      }
    },
  };
}
