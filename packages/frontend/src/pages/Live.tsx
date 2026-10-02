import { createMemo, createResource, onCleanup, onMount, Show, type Component } from 'solid-js';

import { MapViewer } from '@/components/MapViewer';
import { WebGpuMapViewer } from '@/components/WebGpuMapViewer';
import {
  createLiveSnapshotPoller,
  fetchLiveMatchSetting,
  type LiveMatchSetting,
  type LiveSnapshot,
  type LiveSnapshotStatus,
} from '@/lib/live';
import type { HexCell, HexMap, TerrainKind } from '@/types/map';
import '@/styles/live.css';

/** Map `protocol::match_config::CellType` integers to the viewer's terrain labels. */
const CELL_TYPE_TO_TERRAIN: Readonly<Record<number, TerrainKind>> = {
  0: 'plain',
  1: 'mountain',
  2: 'water',
  3: 'road',
};

/**
 * Build the `HexMap` shape the viewer expects from the wire payload. The
 * setting's `map.cells` is row-major `CellType` integers (`0..=3`); the
 * viewer wants flat `{coord, terrain}` entries.
 *
 * Cells outside the declared bounds are skipped silently so a malformed
 * fixture cannot crash the renderer.
 */
function buildHexMap(setting: LiveMatchSetting): HexMap {
  const cells: HexCell[] = [];
  const grid = setting.map.cells as ReadonlyArray<ReadonlyArray<unknown>>;
  const height = setting.map.height;
  const width = setting.map.width;
  for (let row = 0; row < height; row += 1) {
    const rowCells = grid[row];
    if (!Array.isArray(rowCells)) continue;
    for (let col = 0; col < width; col += 1) {
      const raw = rowCells[col];
      const code = typeof raw === 'number' ? raw : 0;
      const terrain = CELL_TYPE_TO_TERRAIN[code] ?? 'plain';
      cells.push({ coord: { col, row }, terrain });
    }
  }
  return { width, height, cells };
}

/** Format the last-poll timestamp as `HH:MM:SS` for the status footer. */
function formatClock(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number): string => String(n).padStart(2, '0');
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** Pretty label per `LiveSnapshotStatus`. Used by the status badge text. */
const STATUS_LABEL: Readonly<Record<LiveSnapshotStatus, string>> = {
  waiting: 'Waiting',
  running: 'Running',
  completed: 'Completed',
  rejected: 'Rejected',
  error: 'Error',
};

/** Status rendered before the first successful poll resolves. */
const STATUS_PLACEHOLDER = '—';

/**
 * Live page — polls `/match-state.json` every second and renders the most
 * recent CLI snapshot (PR #160). The CLI writes this file atomically via
 * `--state-output <PATH>`. The setting fixture (`match-setting.example.json`)
 * still feeds the map viewer for now; the new snapshot doesn't carry
 * per-agent positions so we render an empty agent set.
 */
export const Live: Component = () => {
  const [setting] = createResource<LiveMatchSetting | null, true>(true, () =>
    fetchLiveMatchSetting(),
  );
  const poller = createLiveSnapshotPoller();

  onMount(() => poller.start());
  onCleanup(() => poller.stop());

  const map = createMemo<HexMap | null>(() => {
    const s = setting();
    return s ? buildHexMap(s) : null;
  });

  const snapshot = createMemo<LiveSnapshot | null>(() => poller.snapshot());

  const statusLabel = createMemo<string>(() => {
    const s = snapshot();
    return s ? STATUS_LABEL[s.status] : STATUS_PLACEHOLDER;
  });

  const statusAttr = createMemo<LiveSnapshotStatus | 'waiting'>(
    () => snapshot()?.status ?? 'waiting',
  );

  const revisionsText = createMemo<string>(() => {
    const revs = snapshot()?.revisions ?? [];
    return `[${revs.join(', ')}]`;
  });

  const lastUpdateText = createMemo<string>(() => {
    const s = snapshot();
    return s ? formatClock(s.lastUpdateUnix * 1000) : STATUS_PLACEHOLDER;
  });

  const dayText = createMemo<string>(() => {
    const s = snapshot();
    if (!s) return STATUS_PLACEHOLDER;
    return `${s.day ?? STATUS_PLACEHOLDER} / ${s.daysTotal}`;
  });

  return (
    <section class="page page--live" data-testid="page-live">
      <header class="live__header">
        <h1 class="live__title">Live Match</h1>
        <span class="live__badge" data-testid="live-status-badge" data-status={statusAttr()}>
          {statusLabel()}
        </span>
      </header>

      <p class="live__body">
        Polling <code>/match-state.json</code> once per second. The CLI writes this file via{' '}
        <code>--state-output &lt;PATH&gt;</code> (PR #160). Run the CLI with:
      </p>
      <pre class="live__command">
        procon coordinator --solver wait --state-output packages/frontend/public/match-state.json
      </pre>

      <div class="live__hud">
        <div class="live__hud-cell" data-testid="live-day">
          <span class="live__hud-label">Day</span>
          <span class="live__hud-value live__hud-value--large">{dayText()}</span>
        </div>
        <div class="live__hud-cell" data-testid="live-revisions">
          <span class="live__hud-label">Revisions</span>
          <span class="live__hud-value live__hud-value--mono">{revisionsText()}</span>
        </div>
        <div class="live__hud-cell" data-testid="live-solver">
          <span class="live__hud-label">Solver</span>
          <span class="live__hud-value">{snapshot()?.solver ?? STATUS_PLACEHOLDER}</span>
        </div>
        <div class="live__hud-cell" data-testid="live-server">
          <span class="live__hud-label">Server</span>
          <span class="live__hud-value live__hud-value--mono">
            {snapshot()?.server ?? STATUS_PLACEHOLDER}
          </span>
        </div>
        <div class="live__hud-cell" data-testid="live-last-poll">
          <span class="live__hud-label">Last update</span>
          <span class="live__hud-value live__hud-value--mono">{lastUpdateText()}</span>
        </div>
      </div>

      <Show when={snapshot()?.error}>
        {(err) => (
          <div class="live__error" data-testid="live-error" role="alert">
            {err()}
          </div>
        )}
      </Show>

      <div class="live__map">
        <Show
          when={map()}
          fallback={
            <MapViewer
              map={{ width: 4, height: 4, cells: [] }}
              agents={[]}
              width={800}
              height={420}
            />
          }
        >
          {(m) => <WebGpuMapViewer map={m()} agents={[]} width={800} height={420} />}
        </Show>
      </div>
    </section>
  );
};
