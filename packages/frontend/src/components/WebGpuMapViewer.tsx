import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from 'solid-js';

import { MapViewer } from '@/components/MapViewer';
import { WebGpuMapRenderer } from '@/components/WebGpuMapRenderer';
import type { Agent, HexMap, Spot } from '@/types/map';
import '@/styles/webgpu-map-viewer.css';

/**
 * `WebGpuMapViewer` — Solid wrapper around `WebGpuMapRenderer` with a
 * graceful fallback to `MapViewer` when WebGPU is unavailable.
 *
 * Feature detection runs synchronously on mount (`navigator.gpu`) so the
 * fallback path can render the SVG viewer immediately. If WebGPU is
 * available we asynchronously swap the canvas in once the renderer is
 * initialized; failures during init (e.g. adapter request rejected) drop
 * back to the SVG renderer too.
 *
 * Acceptance criteria for issue #48:
 *   - Instanced rendering for hex cells.
 *   - Frustum culling + LOD in the WebGPU path.
 *   - 100% fallback to the existing SVG viewer when WebGPU is missing.
 */
export interface WebGpuMapViewerProps {
  map: HexMap;
  agents?: ReadonlyArray<Agent>;
  spots?: ReadonlyArray<Spot>;
  /** Optional override for the rendered canvas / SVG width. */
  width?: number;
  /** Optional override for the rendered canvas / SVG height. */
  height?: number;
  /**
   * Forces the WebGPU renderer to attempt init even if `navigator.gpu` is
   * missing. Tests use this to drive a mocked WebGPU path.
   */
  forceWebGpu?: boolean;
}

/** Synchronous probe for WebGPU support. Returns `true` when the API is exposed. */
export function isWebGpuAvailable(): boolean {
  if (typeof navigator === 'undefined') return false;
  if (!('gpu' in navigator)) return false;
  const gpu = (navigator as Navigator & { gpu?: GPU }).gpu;
  return Boolean(gpu);
}

/** Stable probe that doesn't observe props (used for the initial mount). */
function detectWebGpu(force: boolean): boolean {
  if (force) return true;
  return isWebGpuAvailable();
}

export function WebGpuMapViewer(props: WebGpuMapViewerProps): JSX.Element {
  // Initial value: use the runtime probe so the very first render already
  // shows the correct branch (fallback when WebGPU is missing). The effect
  // below re-reads `forceWebGpu` for prop changes.
  const [hasWebGpu, setHasWebGpu] = createSignal<boolean>(isWebGpuAvailable());
  const [renderer, setRenderer] = createSignal<WebGpuMapRenderer | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  let canvasRef: HTMLCanvasElement | undefined;

  // React to `forceWebGpu` prop changes. The setter must be invoked from
  // a tracked scope so Solid can rerun dependent effects.
  createEffect(() => {
    setHasWebGpu(detectWebGpu(Boolean(props.forceWebGpu)));
  });

  // Always render the canvas so its ref is available for init. The fallback
  // branch hides the canvas visually but keeps it in the DOM tree. Without
  // this, `onMount` would race with `<Show>` and find a null ref.
  onMount(() => {
    let cancelled = false;
    let created: WebGpuMapRenderer | null = null;

    void (async (): Promise<void> => {
      if (!hasWebGpu()) return;
      // Wait for the canvas to be inserted into the DOM. Solid renders
      // synchronously, but `<Show>` evaluation can defer when a signal
      // hasn't propagated. Yield once so the ref is wired up.
      await Promise.resolve();
      const canvas = canvasRef;
      if (!canvas || cancelled) return;
      try {
        const { createWebGpuContext } = await import('@/lib/webgpu/device');
        const ctx = await createWebGpuContext(canvas);
        created = new WebGpuMapRenderer(ctx, canvas, {
          hexSize: 24,
          lodDotPixels: 2,
        });
        await created.init();
        if (cancelled) {
          created.destroy();
          return;
        }
        created.resize({
          width: canvas.clientWidth || props.width || 800,
          height: canvas.clientHeight || props.height || 420,
        });
        created.setCamera({ translateX: 0, translateY: 0, scale: 8 });
        created.updateMap(props.map);
        created.updateSpots(props.spots ?? []);
        created.updateAgents(props.agents ?? []);
        created.start();
        if (!cancelled) {
          setRenderer(created);
        } else {
          created.destroy();
          created = null;
        }
      } catch (err) {
        if (created) created.destroy();
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
        }
      }
    })();

    onCleanup(() => {
      cancelled = true;
      const r = created;
      if (r) r.destroy();
    });
  });

  // Reactive: push prop updates into the live renderer.
  createEffect(() => {
    const r = renderer();
    if (!r) return;
    // Reading these inside the effect registers them as dependencies.
    const m = props.map;
    const s = props.spots;
    const a = props.agents;
    r.updateMap(m);
    r.updateSpots(s ?? []);
    r.updateAgents(a ?? []);
  });

  const showWebGpu = (): boolean => hasWebGpu() && renderer() !== null && error() === null;
  const showFallback = (): boolean => !hasWebGpu() || renderer() === null || error() !== null;
  const showLoading = (): boolean => hasWebGpu() && renderer() === null && error() === null;

  // Build the MapViewerProps without spreading undefined values; the
  // MapViewer type uses `exactOptionalPropertyTypes`, which rejects
  // explicit `undefined` for optional fields.
  const fallbackProps = (): {
    map: HexMap;
    agents?: ReadonlyArray<Agent>;
    spots?: ReadonlyArray<Spot>;
    width?: number;
    height?: number;
  } => {
    const out: {
      map: HexMap;
      agents?: ReadonlyArray<Agent>;
      spots?: ReadonlyArray<Spot>;
      width?: number;
      height?: number;
    } = { map: props.map };
    if (props.agents !== undefined) out.agents = props.agents;
    if (props.spots !== undefined) out.spots = props.spots;
    if (props.width !== undefined) out.width = props.width;
    if (props.height !== undefined) out.height = props.height;
    return out;
  };

  const fallbackReason = (): string => {
    if (!hasWebGpu()) return 'webgpu-unavailable';
    const e = error();
    return e ?? 'init-failed';
  };

  return (
    <div class="webgpu-map-viewer" data-testid="webgpu-map-viewer">
      {/*
        Always render the canvas so its ref is available to onMount. The
        canvas is hidden (via the wrapper's visibility) when not active.
      */}
      <div
        class="webgpu-map-viewer__canvas-wrap"
        data-testid="webgpu-map-canvas-wrap"
        style={{ display: showWebGpu() ? 'block' : 'none' }}
      >
        <canvas
          class="webgpu-map-viewer__canvas"
          data-testid="webgpu-map-canvas"
          width={props.width ?? 800}
          height={props.height ?? 420}
          ref={(el) => {
            canvasRef = el;
          }}
        />
      </div>
      <Show when={showFallback()}>
        <div
          class="webgpu-map-viewer__fallback"
          data-testid="webgpu-map-fallback"
          data-fallback-reason={fallbackReason()}
        >
          <MapViewer {...fallbackProps()} />
        </div>
      </Show>
      <Show when={showLoading()}>
        <p class="webgpu-map-viewer__loading" data-testid="webgpu-map-loading">
          Loading WebGPU renderer…
        </p>
      </Show>
    </div>
  );
}
