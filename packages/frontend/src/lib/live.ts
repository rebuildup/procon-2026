/**
 * Live mode wire types + poller.
 *
 * The Live page polls `match-state.example.json` once per second and renders
 * the snapshot via the existing map viewer. The CLI's coordinator does not
 * yet expose HTTP endpoints, so this MVP reads from a static fixture that
 * Vite serves out of `public/`. When the coordinator adds an HTTP listener
 * (separate ticket), only `fetchLiveMatchState` needs to point at the real
 * endpoint.
 *
 * Fixture shape mirrors `crates/protocol::match_config::MatchState` /
 * `MatchSetting` minimally: only the fields the Live page actually consumes
 * are typed; everything else is `unknown[]` so the runtime stays loose.
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

/** Default polling cadence (1 second per spec). */
export const DEFAULT_POLL_MS = 1000;

/**
 * Type-guard for the wire payload. `unknown[]` fields are accepted
 * unchecked; the page only consumes `endsAt`, `day`, and `agents`.
 */
function isLiveMatchState(value: unknown): value is LiveMatchState {
  if (!value || typeof value !== 'object') return false;
  const v = value as Record<string, unknown>;
  return typeof v['endsAt'] === 'number' && typeof v['day'] === 'number' && Array.isArray(v['agents']);
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
 * Fetch a single snapshot of the match state. Returns `null` on transport
 * failure or shape mismatch so the caller can keep the previous value
 * rendered instead of flashing an error overlay every second.
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
    // AbortError and network errors both land here; treat both as "no update".
    return null;
  }
}

/**
 * The setting is static for the duration of a match, so callers fetch it
 * once on mount and feed it into the map adapter.
 */
export async function fetchLiveMatchSetting(signal?: AbortSignal): Promise<LiveMatchSetting | null> {
  try {
    const res = signal ? await fetch(MATCH_SETTING_URL, { signal }) : await fetch(MATCH_SETTING_URL);
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