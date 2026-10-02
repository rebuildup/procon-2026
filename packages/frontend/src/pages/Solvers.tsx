import { createSignal, Show, type Component } from 'solid-js';

import { MapViewer } from '@/components/MapViewer';
import { WebGpuMapViewer, isWebGpuAvailable } from '@/components/WebGpuMapViewer';
import type { Agent, HexMap, Spot } from '@/types/map';
import '@/styles/map-viewer.css';
import '@/styles/webgpu-map-viewer.css';

/**
 * Tiny inline fixture used by the Solvers page preview. Mirrors the
 * crates/domain shape closely enough for the renderer to exercise all
 * four terrain kinds, multiple spot series, and both agent kinds.
 */
const FIXTURE_MAP: HexMap = {
  width: 8,
  height: 6,
  cells: (() => {
    const out = [];
    for (let row = 0; row < 6; row++) {
      for (let col = 0; col < 8; col++) {
        let terrain: HexMap['cells'][number]['terrain'];
        if (row === 0 || row === 5 || col === 0 || col === 7) {
          terrain = 'mountain';
        } else if ((row + col) % 5 === 0) {
          terrain = 'water';
        } else if ((row + col) % 4 === 0) {
          terrain = 'road';
        } else {
          terrain = 'plain';
        }
        out.push({ coord: { col, row }, terrain });
      }
    }
    return out;
  })(),
};

const FIXTURE_SPOTS: ReadonlyArray<Spot> = [
  { id: 's0', coord: { col: 1, row: 1 }, series: 0 },
  { id: 's1', coord: { col: 5, row: 1 }, series: 1 },
  { id: 's2', coord: { col: 2, row: 3 }, series: 2 },
  { id: 's3', coord: { col: 6, row: 4 }, series: 0 },
];

const FIXTURE_AGENTS: ReadonlyArray<Agent> = [
  { id: 'patrol-0', kind: 'patrol', coord: { col: 1, row: 4 } },
  { id: 'patrol-1', kind: 'patrol', coord: { col: 4, row: 2 } },
  { id: 'supply-0', kind: 'supply', coord: { col: 6, row: 2 } },
];

/**
 * Toggle for picking between the SVG-based `MapViewer` and the WebGPU-based
 * `WebGpuMapViewer`. Defaults to SVG so the page remains useful on
 * environments without a WebGPU adapter (CI, older browsers, …).
 */
type RendererMode = 'svg' | 'webgpu';

export const Solvers: Component = () => {
  const [mode, setMode] = createSignal<RendererMode>(isWebGpuAvailable() ? 'webgpu' : 'svg');

  return (
    <section class="page page--solvers">
      <h2 class="page__title">Solvers</h2>
      <p class="page__body">
        Solver list will appear here once the protocol bridge is online. The map viewer below is a
        preview seeded with an inline fixture to exercise the renderer.
      </p>
      <div
        class="page__map"
        style={{
          position: 'relative',
          width: '100%',
          height: '420px',
          'margin-top': '1rem',
        }}
      >
        <div class="page__renderer-toggle" role="group" aria-label="Renderer mode">
          <button
            type="button"
            class="page__renderer-btn"
            data-testid="solvers-renderer-svg"
            aria-pressed={mode() === 'svg'}
            onClick={() => setMode('svg')}
          >
            SVG
          </button>
          <button
            type="button"
            class="page__renderer-btn"
            data-testid="solvers-renderer-webgpu"
            aria-pressed={mode() === 'webgpu'}
            onClick={() => setMode('webgpu')}
          >
            WebGPU
          </button>
        </div>
        <Show
          when={mode() === 'svg'}
          fallback={
            <WebGpuMapViewer
              map={FIXTURE_MAP}
              spots={FIXTURE_SPOTS}
              agents={FIXTURE_AGENTS}
              width={800}
              height={420}
            />
          }
        >
          <MapViewer map={FIXTURE_MAP} spots={FIXTURE_SPOTS} agents={FIXTURE_AGENTS} />
        </Show>
      </div>
    </section>
  );
};
