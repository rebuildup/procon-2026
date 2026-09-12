import { createMemo, createSignal, For, type JSX, onCleanup, onMount, Show } from 'solid-js';

import {
  AGENT_COLORS,
  type Agent,
  type HexCoord,
  type HexMap,
  type Spot,
  TERRAIN_COLORS,
} from '@/types/map';

/**
 * Basic map viewer for the procon-2026 hex grid.
 *
 * Spec trace:
 * - Q&A Q1: even rows are non-shifted, odd rows shift right by half a
 *   cell-width. This viewer uses *pointy-top* hexes with even-r offset.
 * - Q&A Q2: the map is not a torus; out-of-range coordinates are simply
 *   skipped during render.
 *
 * Features:
 * - Renders one `<polygon>` per `HexCell`.
 * - Renders spots as colored circles with the series label (centered text).
 * - Renders agents as triangles (point-up) for `patrol`, diamonds for `supply`.
 * - Wheel: zoom around the cursor (clamped between MIN_ZOOM and MAX_ZOOM).
 * - Drag (mouse / pointer): pan the camera.
 *
 * Camera state is intentionally simple (no inertia, no animations) so the
 * component stays testable. The test suite mounts the component under
 * happy-dom and verifies the SVG skeleton + marker counts.
 */
export interface MapViewerProps {
  map: HexMap;
  agents?: ReadonlyArray<Agent>;
  spots?: ReadonlyArray<Spot>;
  /** Optional override for the rendered SVG width. Defaults to the map pixel width. */
  width?: number;
  /** Optional override for the rendered SVG height. Defaults to the map pixel height. */
  height?: number;
}

const HEX_SIZE = 24; // center-to-vertex radius in SVG user units.
const SQRT3 = Math.sqrt(3);
const MIN_ZOOM = 0.4;
const MAX_ZOOM = 4;

const ZOOM_FACTOR = 1.1;

/** SVG `points` attribute string for a pointy-top hex centered at `(cx, cy)`. */
function hexPoints(cx: number, cy: number, size = HEX_SIZE): string {
  const half = size / 2;
  const dx = (SQRT3 / 2) * size;
  return [
    `${cx},${cy - size}`,
    `${cx + dx},${cy - half}`,
    `${cx + dx},${cy + half}`,
    `${cx},${cy + size}`,
    `${cx - dx},${cy + half}`,
    `${cx - dx},${cy - half}`,
  ].join(' ');
}

/** Convert `(col, row)` offset coords to pixel center, honoring even-r layout. */
function hexCenter(coord: HexCoord): { cx: number; cy: number } {
  const cx = SQRT3 * HEX_SIZE * coord.col + (coord.row % 2 === 1 ? (SQRT3 / 2) * HEX_SIZE : 0);
  const cy = 1.5 * HEX_SIZE * coord.row;
  return { cx, cy };
}

/** Total SVG pixel size required to render a `HexMap` of the given dimensions. */
export function mapPixelSize(map: HexMap): { width: number; height: number } {
  const dx = SQRT3 * HEX_SIZE;
  const width = SQRT3 * HEX_SIZE * map.width + dx / 2 + dx / 2; // include top + bottom overlap
  const height = 1.5 * HEX_SIZE * map.height + HEX_SIZE + HEX_SIZE; // include top + bottom padding
  return { width, height };
}

/** Color of a spot series, wrapping cyclically through the brand palette. */
function spotFill(series: number): string {
  const palette = ['#d97706', '#9333ea', '#dc2626', '#0891b2', '#16a34a', '#db2777'];
  const idx = ((series % palette.length) + palette.length) % palette.length;
  return palette[idx] ?? '#d97706';
}

export function MapViewer(props: MapViewerProps): JSX.Element {
  const agents = (): ReadonlyArray<Agent> => props.agents ?? [];
  const spots = (): ReadonlyArray<Spot> => props.spots ?? [];
  const dims = createMemo(() => mapPixelSize(props.map));
  const svgW = (): number => props.width ?? dims().width;
  const svgH = (): number => props.height ?? dims().height;

  const [scale, setScale] = createSignal(1);
  const [tx, setTx] = createSignal(0);
  const [ty, setTy] = createSignal(0);
  const [dragging, setDragging] = createSignal(false);

  let svgRef: SVGSVGElement | undefined;
  let dragState: { startX: number; startY: number; tx0: number; ty0: number } | null = null;
  let activePointer: number | null = null;

  function clampZoom(z: number): number {
    if (z < MIN_ZOOM) return MIN_ZOOM;
    if (z > MAX_ZOOM) return MAX_ZOOM;
    return z;
  }

  function onWheel(e: WheelEvent): void {
    if (!svgRef) return;
    e.preventDefault();
    const rect = svgRef.getBoundingClientRect();
    const cx = e.clientX - rect.left;
    const cy = e.clientY - rect.top;
    const direction = e.deltaY > 0 ? 1 / ZOOM_FACTOR : ZOOM_FACTOR;
    const oldScale = scale();
    const newScale = clampZoom(oldScale * direction);
    if (newScale === oldScale) return;
    // Keep the world point under the cursor fixed across the zoom step.
    const worldX = (cx - tx()) / oldScale;
    const worldY = (cy - ty()) / oldScale;
    setScale(newScale);
    setTx(() => cx - worldX * newScale);
    setTy(() => cy - worldY * newScale);
  }

  function onPointerDown(e: PointerEvent): void {
    if (!svgRef) return;
    if (e.button !== 0 && e.pointerType === 'mouse') return;
    const target = e.currentTarget as SVGSVGElement;
    target.setPointerCapture(e.pointerId);
    activePointer = e.pointerId;
    dragState = { startX: e.clientX, startY: e.clientY, tx0: tx(), ty0: ty() };
    setDragging(true);
  }

  function onPointerMove(e: PointerEvent): void {
    if (!dragState || activePointer !== e.pointerId) return;
    const dx = e.clientX - dragState.startX;
    const dy = e.clientY - dragState.startY;
    setTx(() => dragState!.tx0 + dx);
    setTy(() => dragState!.ty0 + dy);
  }

  function onPointerUp(e: PointerEvent): void {
    if (activePointer !== null) {
      const target = svgRef as SVGSVGElement | undefined;
      try {
        target?.releasePointerCapture(e.pointerId);
      } catch {
        // The pointer may not be captured (e.g. it was released during a
        // synthetic test). Failing to release is harmless here.
      }
    }
    dragState = null;
    activePointer = null;
    setDragging(false);
  }

  // Reset the camera whenever the underlying map changes (e.g. fixture swap).
  const mapKey = createMemo(() => `${props.map.width}x${props.map.height}`);
  let lastMapKey: string | undefined;
  function resetIfMapChanged(): void {
    const next = mapKey();
    if (next !== lastMapKey) {
      setScale(1);
      setTx(0);
      setTy(0);
      lastMapKey = next;
    }
  }

  onMount(() => {
    resetIfMapChanged();
  });
  onCleanup(() => {
    dragState = null;
    activePointer = null;
  });

  const transform = (): string => `translate(${tx()} ${ty()}) scale(${scale()})`;

  const canvasClass = (): string =>
    dragging() ? 'map-viewer__canvas map-viewer__canvas--dragging' : 'map-viewer__canvas';

  return (
    <div class="map-viewer" data-testid="map-viewer">
      <Show
        when={props.map.cells.length > 0}
        fallback={<p class="map-viewer__empty">map is empty</p>}
      >
        <svg
          ref={(el) => {
            svgRef = el;
          }}
          class={canvasClass()}
          data-testid="map-viewer-svg"
          width={svgW()}
          height={svgH()}
          viewBox={`0 0 ${svgW()} ${svgH()}`}
          onWheel={onWheel}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        >
          <g transform={transform()}>
            <For each={props.map.cells}>
              {(cell) => {
                const { cx, cy } = hexCenter(cell.coord);
                const fill = TERRAIN_COLORS[cell.terrain];
                return (
                  <polygon
                    class="map-viewer__cell"
                    data-testid="map-viewer-cell"
                    data-coord={`${cell.coord.col},${cell.coord.row}`}
                    data-terrain={cell.terrain}
                    points={hexPoints(cx, cy)}
                    fill={fill}
                  />
                );
              }}
            </For>
            <For each={spots()}>
              {(spot) => {
                const { cx, cy } = hexCenter(spot.coord);
                const fill = spotFill(spot.series);
                return (
                  <g data-testid="map-viewer-spot" data-spot-id={spot.id}>
                    <circle
                      class="map-viewer__spot"
                      cx={cx}
                      cy={cy}
                      r={HEX_SIZE * 0.32}
                      fill={fill}
                    />
                    <text class="map-viewer__spot-label" x={cx} y={cy}>
                      {String(spot.series)}
                    </text>
                  </g>
                );
              }}
            </For>
            <For each={agents()}>
              {(agent) => {
                const { cx, cy } = hexCenter(agent.coord);
                const fill = AGENT_COLORS[agent.kind];
                const r = HEX_SIZE * 0.42;
                // Triangle for patrol; diamond for supply (different shapes so
                // the viewer conveys agent role even in monochrome printing).
                const points =
                  agent.kind === 'patrol'
                    ? `${cx},${cy - r} ${cx + r},${cy + (r * 2) / 3} ${cx - r},${cy + (r * 2) / 3}`
                    : `${cx},${cy - r} ${cx + r},${cy} ${cx},${cy + r} ${cx - r},${cy}`;
                return (
                  <polygon
                    class="map-viewer__agent"
                    data-testid="map-viewer-agent"
                    data-agent-id={agent.id}
                    data-agent-kind={agent.kind}
                    points={points}
                    fill={fill}
                  />
                );
              }}
            </For>
          </g>
        </svg>
      </Show>
      <p class="map-viewer__hint">drag to pan, wheel to zoom</p>
    </div>
  );
}
