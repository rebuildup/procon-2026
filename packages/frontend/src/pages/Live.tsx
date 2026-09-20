import {
  createMemo,
  createResource,
  onCleanup,
  onMount,
  Show,
  type Component,
} from 'solid-js';

import { MapViewer } from '@/components/MapViewer';
import { WebGpuMapViewer } from '@/components/WebGpuMapViewer';
import {
  classifyStatus,
  createLivePoller,
  fetchLiveMatchSetting,
  type LiveAgentState,
  type LiveMatchSetting,
  type LiveMatchState,
  type LiveStatus,
} from '@/lib/live';
import type { Agent, HexCell, HexMap, TerrainKind } from '@/types/map';
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

/** Map `protocol::match_config::AgentKind` to the viewer's `AgentKind`. */
function agentKindOf(value: number): Agent['kind'] {
  return value === 0 ? 'patrol' : 'supply';
}

/** Convert wire-format agents into the viewer's `Agent` shape. */
function buildAgents(state: LiveMatchState | null): ReadonlyArray<Agent> {
  if (!state) return [];
  return state.agents.map((a: LiveAgentState): Agent => ({
    id: String(a.id),
    kind: agentKindOf(a.kind),
    coord: { col: a.pos.q, row: a.pos.r },
  }));
}

/** Format the last-poll timestamp as `HH:MM:SS` for the status footer. */
function formatClock(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number): string => String(n).padStart(2, '0');
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/**
 * Live page — polls `match-state.example.json` every second and renders the
 * most recent snapshot via the existing map viewer (WebGPU first, SVG
 * fallback inside `WebGpuMapViewer`).
 *
 * The "live" semantics are stubbed for now: there is no HTTP endpoint on the
 * coordinator yet, so the fixture is committed under `public/`. A follow-up
 * ticket will point `fetchLiveMatchState` at the real endpoint.
 */
export const Live: Component = () => {
  const [setting] = createResource<LiveMatchSetting | null, true>(
    true,
    () => fetchLiveMatchSetting(),
  );
  const poller = createLivePoller();

  onMount(() => poller.start());
  onCleanup(() => poller.stop());

  const map = createMemo<HexMap | null>(() => {
    const s = setting();
    return s ? buildHexMap(s) : null;
  });

  const agents = createMemo<ReadonlyArray<Agent>>(() => buildAgents(poller.state()));

  const status = createMemo<LiveStatus>(() => {
    const s = poller.state();
    if (!s) return poller.status();
    const daySeconds = setting()?.daySeconds;
    return classifyStatus(s, Date.now(), daySeconds);
  });

  return (
    <section class="page page--live" data-testid="page-live">
      <header class="live__header">
        <h1 class="live__title">Live Match</h1>
        <span
          class="live__badge"
          data-testid="live-status-badge"
          data-status={status()}
        >
          {status()}
        </span>
      </header>
      <p class="live__body">
        Polling <code>match-state.example.json</code> once per second. The CLI does not yet expose
        HTTP endpoints, so the fixture is committed under <code>public/</code>; a follow-up ticket
        will wire the coordinator into this page.
      </p>
      <div class="live__hud">
        <div class="live__hud-cell" data-testid="live-day">
          <span class="live__hud-label">Day</span>
          <span class="live__hud-value live__hud-value--large">
            {poller.state()?.day ?? '—'}
          </span>
        </div>
        <div class="live__hud-cell" data-testid="live-agent-count">
          <span class="live__hud-label">Agents</span>
          <span class="live__hud-value">{poller.state()?.agents.length ?? 0}</span>
        </div>
        <div class="live__hud-cell" data-testid="live-last-poll">
          <span class="live__hud-label">Last poll</span>
          <span class="live__hud-value live__hud-value--mono">
            {poller.state() ? formatClock(Date.now()) : '—'}
          </span>
        </div>
      </div>
      <div class="live__map">
        <Show when={map()} fallback={
          <MapViewer
            map={{ width: 4, height: 4, cells: [] }}
            agents={agents()}
            width={800}
            height={420}
          />
        }>
          {(m) => (
            <WebGpuMapViewer map={m()} agents={agents()} width={800} height={420} />
          )}
        </Show>
      </div>
    </section>
  );
};