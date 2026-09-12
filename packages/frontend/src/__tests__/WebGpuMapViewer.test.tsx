import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { jsx as hyperscriptJsx } from 'solid-js/h/jsx-runtime';

import { WebGpuMapViewer } from '@/components/WebGpuMapViewer';
import type { Agent, HexMap, Spot } from '@/types/map';

function buildMap(width: number, height: number): HexMap {
  const cells = [];
  for (let row = 0; row < height; row++) {
    for (let col = 0; col < width; col++) {
      const flat = row * width + col;
      let terrain: HexMap['cells'][number]['terrain'];
      if (row === 0 || row === height - 1 || col === 0 || col === width - 1) {
        terrain = 'mountain';
      } else if (flat % 5 === 0) {
        terrain = 'water';
      } else if (flat % 3 === 0) {
        terrain = 'road';
      } else {
        terrain = 'plain';
      }
      cells.push({ coord: { col, row }, terrain });
    }
  }
  return { width, height, cells };
}

const FIXTURE_SPOTS: ReadonlyArray<Spot> = [{ id: 's0', coord: { col: 1, row: 1 }, series: 0 }];

const FIXTURE_AGENTS: ReadonlyArray<Agent> = [
  { id: 'a0', kind: 'patrol', coord: { col: 2, row: 2 } },
];

/**
 * Capture the original `navigator.gpu` so each test can restore it on
 * cleanup. This keeps the test suite hermetic when other tests mock or
 * pollute the global.
 */
function withNavigatorGpu(value: unknown, fn: () => void): void {
  const nav = navigator as Navigator & { gpu?: unknown };
  const prev = nav.gpu;
  if (value === undefined) {
    delete (nav as { gpu?: unknown }).gpu;
  } else {
    (nav as { gpu?: unknown }).gpu = value;
  }
  try {
    fn();
  } finally {
    if (prev === undefined) {
      delete (nav as { gpu?: unknown }).gpu;
    } else {
      nav.gpu = prev;
    }
  }
}

/**
 * Minimal mock of `GPU`. Each test composes the level of support it needs.
 * `requestAdapter` returns a stub adapter that exposes `requestDevice`.
 */
function buildMockGpu(opts: {
  adapterAvailable: boolean;
  deviceAvailable?: boolean;
}): GPU & { requestAdapterCalls: number; requestDeviceCalls: number } {
  const requestAdapterCalls = { value: 0 };
  const requestDeviceCalls = { value: 0 };
  const adapter = opts.adapterAvailable
    ? {
        requestDevice: () => {
          requestDeviceCalls.value += 1;
          if (opts.deviceAvailable === false) {
            return Promise.reject(new Error('mock: no device'));
          }
          return Promise.resolve(buildMockDevice());
        },
      }
    : null;
  const gpu = {
    requestAdapterCalls: 0,
    requestDeviceCalls: 0,
    requestAdapter: () => {
      requestAdapterCalls.value += 1;
      (gpu as { requestAdapterCalls: number }).requestAdapterCalls += 1;
      return Promise.resolve(adapter);
    },
    getPreferredCanvasFormat: () => 'bgra8unorm' as GPUTextureFormat,
  } as unknown as GPU & { requestAdapterCalls: number; requestDeviceCalls: number };
  // Wrap counters on the returned object so the test can inspect them.
  Object.defineProperty(gpu, 'requestAdapterCalls', {
    get: () => requestAdapterCalls.value,
  });
  Object.defineProperty(gpu, 'requestDeviceCalls', {
    get: () => requestDeviceCalls.value,
  });
  return gpu;
}

function buildMockDevice(): GPUDevice {
  const buffer = {
    size: 0,
    usage: 0,
    destroy: () => undefined,
    unmap: () => undefined,
    getMappedRange: () => new ArrayBuffer(0),
  } as unknown as GPUBuffer;
  return {
    createShaderModule: () => ({}) as GPUShaderModule,
    createRenderPipeline: () => {
      const layout = {
        entries: [],
      };
      return {
        getBindGroupLayout: () => layout,
        layout,
      } as unknown as GPURenderPipeline;
    },
    createBuffer: () => buffer,
    createBindGroup: () => ({}) as GPUBindGroup,
    createCommandEncoder: () => ({
      beginRenderPass: () => ({
        setPipeline: () => undefined,
        setBindGroup: () => undefined,
        setVertexBuffer: () => undefined,
        draw: () => undefined,
        end: () => undefined,
      }),
      finish: () => ({}) as GPUCommandBuffer,
    }),
    queue: {
      submit: () => undefined,
      writeBuffer: () => undefined,
    },
    lost: Promise.resolve({ reason: 'mock', message: 'mock' }),
  } as unknown as GPUDevice;
}

describe('WebGpuMapViewer', () => {
  let root: HTMLDivElement;

  beforeEach(() => {
    root = document.createElement('div');
    document.body.appendChild(root);
  });

  afterEach(() => {
    root?.remove();
  });

  function mount(args: {
    map: HexMap;
    spots?: ReadonlyArray<Spot>;
    agents?: ReadonlyArray<Agent>;
    forceWebGpu?: boolean;
  }): HTMLDivElement {
    render(
      () =>
        hyperscriptJsx(WebGpuMapViewer, {
          map: args.map,
          spots: args.spots ?? [],
          agents: args.agents ?? [],
          forceWebGpu: args.forceWebGpu,
        }) as unknown as JSX.Element,
      root,
    );
    return root;
  }

  test('falls back to MapViewer when navigator.gpu is undefined', () => {
    withNavigatorGpu(undefined, () => {
      const map = buildMap(5, 4);
      const el = mount({ map, spots: FIXTURE_SPOTS, agents: FIXTURE_AGENTS });
      const fallback = el.querySelector('[data-testid="webgpu-map-fallback"]');
      expect(fallback).not.toBeNull();
      expect(fallback?.getAttribute('data-fallback-reason')).toBe('webgpu-unavailable');
      // The SVG viewer should be embedded inside the fallback.
      const svg = el.querySelector('[data-testid="map-viewer-svg"]');
      expect(svg).not.toBeNull();
      // The canvas wrap should be hidden (display: none) so the fallback is
      // the visible surface. The element itself stays in the DOM tree so
      // onMount can find a stable ref.
      const wrap = el.querySelector('[data-testid="webgpu-map-canvas-wrap"]') as HTMLElement | null;
      expect(wrap).not.toBeNull();
      expect(wrap?.style.display).toBe('none');
    });
  });

  test('falls back to MapViewer when navigator.gpu is present but adapter is null', async () => {
    const gpu = buildMockGpu({ adapterAvailable: false });
    await withNavigatorGpuAsync(gpu, async () => {
      const map = buildMap(4, 4);
      const el = mount({ map });
      // Allow the async init microtasks to run.
      await flushMicrotasks();
      const fallback = el.querySelector('[data-testid="webgpu-map-fallback"]');
      expect(fallback).not.toBeNull();
      expect(el.querySelector('[data-testid="map-viewer-svg"]')).not.toBeNull();
      // The canvas wrap stays hidden while init fails.
      const wrap = el.querySelector('[data-testid="webgpu-map-canvas-wrap"]') as HTMLElement | null;
      expect(wrap).not.toBeNull();
      expect(wrap?.style.display).toBe('none');
    });
  });

  test('renders the WebGPU canvas when navigator.gpu is mocked and adapter resolves', async () => {
    const gpu = buildMockGpu({ adapterAvailable: true, deviceAvailable: true });
    await withNavigatorGpuAsync(gpu, async () => {
      // Patch HTMLCanvasElement.getContext so the renderer sees a context.
      const originalGetContext = HTMLCanvasElement.prototype.getContext;
      const fakeContext = {
        configure: () => undefined,
        getCurrentTexture: () => ({
          createView: () => ({}) as GPUTextureView,
        }),
      };
      const patchedGetContext = function (this: HTMLCanvasElement, type: string): unknown {
        if (type === 'webgpu') return fakeContext;
        return (originalGetContext as unknown as (t: string) => unknown).call(this, type);
      };
      HTMLCanvasElement.prototype.getContext =
        patchedGetContext as unknown as typeof HTMLCanvasElement.prototype.getContext;
      try {
        const map = buildMap(4, 4);
        const el = mount({ map, spots: FIXTURE_SPOTS, agents: FIXTURE_AGENTS });
        // Yield so the async onMount → init() chain completes.
        await flushMicrotasks();
        // The WebGPU canvas should now be in the DOM.
        const canvas = el.querySelector('[data-testid="webgpu-map-canvas"]');
        expect(canvas).not.toBeNull();
        expect(canvas?.tagName.toLowerCase()).toBe('canvas');
      } finally {
        HTMLCanvasElement.prototype.getContext = originalGetContext;
      }
    });
  });

  test('reports the SVG viewer test id when forceWebGpu is true but device request fails', async () => {
    const gpu = buildMockGpu({ adapterAvailable: true, deviceAvailable: false });
    await withNavigatorGpuAsync(gpu, async () => {
      const originalGetContext = HTMLCanvasElement.prototype.getContext;
      const patchedGetContext = function (): unknown {
        return null;
      };
      HTMLCanvasElement.prototype.getContext =
        patchedGetContext as unknown as typeof HTMLCanvasElement.prototype.getContext;
      try {
        const map = buildMap(3, 3);
        const el = mount({ map, forceWebGpu: true });
        await flushMicrotasks();
        const fallback = el.querySelector('[data-testid="webgpu-map-fallback"]');
        expect(fallback).not.toBeNull();
        expect(el.querySelector('[data-testid="map-viewer-svg"]')).not.toBeNull();
      } finally {
        HTMLCanvasElement.prototype.getContext = originalGetContext;
      }
    });
  });
});

/** Async variant of `withNavigatorGpu`. */
async function withNavigatorGpuAsync(value: unknown, fn: () => Promise<void>): Promise<void> {
  const nav = navigator as Navigator & { gpu?: unknown };
  const prev = nav.gpu;
  if (value === undefined) {
    delete (nav as { gpu?: unknown }).gpu;
  } else {
    (nav as { gpu?: unknown }).gpu = value;
  }
  try {
    await fn();
  } finally {
    if (prev === undefined) {
      delete (nav as { gpu?: unknown }).gpu;
    } else {
      nav.gpu = prev;
    }
  }
}

/**
 * Drain the microtask queue a few times. WebGPU init is an async chain
 * (requestAdapter → requestDevice → shader compile). One await is rarely
 * enough.
 */
async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 5; i += 1) {
    await Promise.resolve();
  }
}
