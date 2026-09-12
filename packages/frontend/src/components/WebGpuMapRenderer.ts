/**
 * WebGPU-based instanced renderer for the procon-2026 hex map.
 *
 * Design notes:
 * - Built around the shared `WebGpuContext` from `@/lib/webgpu/device`.
 *   Callers may reuse a single device across multiple components.
 * - Hex cells, spots, and agents each render through the same shader but
 *   with their own instance buffer and pipeline, so they can be uploaded
 *   independently when only one slice of the state changes.
 * - Cells use frustum culling at the CPU side: out-of-bounds cells are
 *   omitted from the instance buffer. The GPU sees only what's actually
 *   inside the viewport.
 * - LOD is zoom-aware. When the projected hex size falls below
 *   `lodDotPixels` we collapse each cell into a tiny bounding box so the
 *   shader cost stays constant regardless of zoom level.
 * - 60fps target at 32×32 maps (the acceptance criterion in issue #48).
 *   All per-frame work is O(visible-instance-count).
 */

import { createWebGpuContext, type WebGpuContext } from '@/lib/webgpu/device';
import type { Agent, HexMap, Spot, TerrainKind } from '@/types/map';

import shaderSource from '@/shaders/hex_instanced.wgsl?raw';

const SQRT3 = Math.sqrt(3);

const TERRAIN_FILL: Readonly<Record<TerrainKind, [number, number, number, number]>> = {
  plain: [0.714, 0.78, 0.604, 1],
  mountain: [0.42, 0.42, 0.463, 1],
  water: [0.435, 0.706, 0.839, 1],
  road: [0.851, 0.796, 0.651, 1],
};

const SPOT_SERIES_FILL: ReadonlyArray<[number, number, number, number]> = [
  [0.851, 0.467, 0.024, 1], // amber
  [0.576, 0.204, 0.918, 1], // violet
  [0.863, 0.149, 0.149, 1], // red
  [0.031, 0.569, 0.698, 1], // cyan
  [0.086, 0.639, 0.29, 1], // green
  [0.859, 0.153, 0.467, 1], // pink
];

const AGENT_FILL: Readonly<Record<Agent['kind'], [number, number, number, number]>> = {
  patrol: [0.122, 0.137, 0.157, 1],
  supply: [0.035, 0.412, 0.855, 1],
};

/** Convert `(col, row)` to world-space center, honoring even-r layout. */
function hexCenter(
  coord: { col: number; row: number },
  hexSize: number,
): {
  cx: number;
  cy: number;
} {
  const dx = SQRT3 * hexSize;
  const cx = SQRT3 * hexSize * coord.col + (coord.row % 2 === 1 ? dx / 2 : 0);
  const cy = 1.5 * hexSize * coord.row;
  return { cx, cy };
}

export interface HexGpuLayerState {
  /** Instance count actually uploaded to the GPU. */
  readonly visibleInstanceCount: number;
  /** Cells in the source map (pre-culling). */
  readonly totalInstanceCount: number;
}

export interface WebGpuMapRendererOptions {
  /** Hex size in world units (center-to-vertex). Defaults to 24. */
  hexSize?: number;
  /** When the projected hex size falls below this many screen pixels the
   *  vertex shader collapses the cell to a dot. Defaults to 2. */
  lodDotPixels?: number;
  /** Translation of the camera in world units. Defaults to the map center. */
  camera?: { translateX: number; translateY: number; scale: number };
}

/**
 * Self-contained WebGPU map renderer. The class owns its pipelines, buffers,
 * and per-layer instance data. Callers drive state changes through
 * `updateMap`, `updateSpots`, `updateAgents`, and `resize`.
 */
export class WebGpuMapRenderer {
  readonly #ctx: WebGpuContext;
  readonly #canvas: HTMLCanvasElement;
  readonly #hexSize: number;
  readonly #lodDotPixels: number;

  #pipeline!: GPURenderPipeline;
  #quadBuffer!: GPUBuffer;
  #uniformBuffer!: GPUBuffer;
  #bindGroup!: GPUBindGroup;

  #cellInstanceBuffer: GPUBuffer | null = null;
  #cellInstanceCount = 0;
  #cellLayer: HexGpuLayerState = { visibleInstanceCount: 0, totalInstanceCount: 0 };

  #spotInstanceBuffer: GPUBuffer | null = null;
  #spotInstanceCount = 0;
  #spotLayer: HexGpuLayerState = { visibleInstanceCount: 0, totalInstanceCount: 0 };

  #agentInstanceBuffer: GPUBuffer | null = null;
  #agentInstanceCount = 0;
  #agentLayer: HexGpuLayerState = { visibleInstanceCount: 0, totalInstanceCount: 0 };

  #camera = { translateX: 0, translateY: 0, scale: 8 };
  #viewport = { width: 1, height: 1 };

  #rafHandle: number | null = null;
  #framesRendered = 0;
  #destroyed = false;

  constructor(
    ctx: WebGpuContext,
    canvas: HTMLCanvasElement,
    options: WebGpuMapRendererOptions = {},
  ) {
    this.#ctx = ctx;
    this.#canvas = canvas;
    this.#hexSize = options.hexSize ?? 24;
    this.#lodDotPixels = options.lodDotPixels ?? 2;
    if (options.camera) this.#camera = { ...options.camera };
  }

  /**
   * Initialize the pipeline, buffers, and bind group. Must be called once
   * before `start()`. Returns when the renderer is ready to draw.
   */
  async init(): Promise<void> {
    const module = this.#ctx.device.createShaderModule({ code: shaderSource });

    this.#pipeline = this.#ctx.device.createRenderPipeline({
      layout: 'auto',
      vertex: {
        module,
        entryPoint: 'vs_hex',
        buffers: [
          // Slot 0: per-instance (center + color).
          {
            arrayStride: 24,
            stepMode: 'instance',
            attributes: [
              { shaderLocation: 0, offset: 0, format: 'float32x2' },
              { shaderLocation: 1, offset: 8, format: 'float32x4' },
            ],
          },
          // Slot 1: per-vertex corner (unit quad covering a single hex).
          {
            arrayStride: 8,
            stepMode: 'vertex',
            attributes: [{ shaderLocation: 2, offset: 0, format: 'float32x2' }],
          },
        ],
      },
      fragment: {
        module,
        entryPoint: 'fs_hex',
        targets: [{ format: this.#ctx.format }],
      },
      primitive: { topology: 'triangle-list' },
    });

    // Unit quad corners. Six triangles for the hex shape, but since the
    // shader treats corners as pointy-top offsets we use a triangle fan
    // with 6 vertices that forms a triangle list (6 verts → 4 triangles).
    // Using a quad keeps the geometry buffer tiny and lets the shader
    // ignore the geometry-specific layout via `corner`.
    const corners = new Float32Array([-1, -1, 1, -1, 1, 1, -1, -1, 1, 1, -1, 1]);
    this.#quadBuffer = this.#ctx.device.createBuffer({
      size: corners.byteLength,
      usage: GPUBufferUsage.VERTEX,
      mappedAtCreation: true,
    });
    new Float32Array(this.#quadBuffer.getMappedRange()).set(corners);
    this.#quadBuffer.unmap();

    this.#uniformBuffer = this.#ctx.device.createBuffer({
      size: 64, // matches the aligned CameraUniform struct (8 × 4 = 32 floats)
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    this.#bindGroup = this.#ctx.device.createBindGroup({
      layout: this.#pipeline.getBindGroupLayout(0),
      entries: [{ binding: 0, resource: { buffer: this.#uniformBuffer } }],
    });

    this.#resizeCanvas();
  }

  /** Replace the cell map. Re-uploads the instance buffer if anything changed. */
  updateMap(map: HexMap): HexGpuLayerState {
    if (this.#destroyed) return this.#cellLayer;
    const total = map.cells.length;
    const data = new Float32Array(total * 6); // 2 floats center + 4 floats color
    let visible = 0;
    for (const cell of map.cells) {
      const { cx, cy } = hexCenter(cell.coord, this.#hexSize);
      if (!this.#isInFrustum(cx, cy)) continue;
      const fill = TERRAIN_FILL[cell.terrain];
      const off = visible * 6;
      data[off + 0] = cx;
      data[off + 1] = cy;
      data[off + 2] = fill[0];
      data[off + 3] = fill[1];
      data[off + 4] = fill[2];
      data[off + 5] = fill[3];
      visible += 1;
    }
    if (visible === 0) {
      // No cells visible: clear the buffer (still allocate at least one slot).
      this.#cellInstanceBuffer = null;
      this.#cellInstanceCount = 0;
    } else {
      this.#cellInstanceBuffer = this.#uploadInstanceBuffer(
        this.#cellInstanceBuffer,
        data,
        visible,
      );
      this.#cellInstanceCount = visible;
    }
    this.#cellLayer = { visibleInstanceCount: visible, totalInstanceCount: total };
    return this.#cellLayer;
  }

  /** Replace the spot layer. */
  updateSpots(spots: ReadonlyArray<Spot>): HexGpuLayerState {
    if (this.#destroyed) return this.#spotLayer;
    const total = spots.length;
    const data = new Float32Array(Math.max(total, 1) * 6);
    let visible = 0;
    for (const spot of spots) {
      const { cx, cy } = hexCenter(spot.coord, this.#hexSize);
      if (!this.#isInFrustum(cx, cy)) continue;
      const fill = SPOT_SERIES_FILL[spot.series % SPOT_SERIES_FILL.length] ?? SPOT_SERIES_FILL[0]!;
      const off = visible * 6;
      data[off + 0] = cx;
      data[off + 1] = cy;
      data[off + 2] = fill[0];
      data[off + 3] = fill[1];
      data[off + 4] = fill[2];
      data[off + 5] = fill[3];
      visible += 1;
    }
    if (visible === 0) {
      this.#spotInstanceBuffer = null;
      this.#spotInstanceCount = 0;
    } else {
      this.#spotInstanceBuffer = this.#uploadInstanceBuffer(
        this.#spotInstanceBuffer,
        data,
        visible,
      );
      this.#spotInstanceCount = visible;
    }
    this.#spotLayer = { visibleInstanceCount: visible, totalInstanceCount: total };
    return this.#spotLayer;
  }

  /** Replace the agent layer. */
  updateAgents(agents: ReadonlyArray<Agent>): HexGpuLayerState {
    if (this.#destroyed) return this.#agentLayer;
    const total = agents.length;
    const data = new Float32Array(Math.max(total, 1) * 6);
    let visible = 0;
    for (const agent of agents) {
      const { cx, cy } = hexCenter(agent.coord, this.#hexSize);
      if (!this.#isInFrustum(cx, cy)) continue;
      const fill = AGENT_FILL[agent.kind];
      const off = visible * 6;
      data[off + 0] = cx;
      data[off + 1] = cy;
      data[off + 2] = fill[0];
      data[off + 3] = fill[1];
      data[off + 4] = fill[2];
      data[off + 5] = fill[3];
      visible += 1;
    }
    if (visible === 0) {
      this.#agentInstanceBuffer = null;
      this.#agentInstanceCount = 0;
    } else {
      this.#agentInstanceBuffer = this.#uploadInstanceBuffer(
        this.#agentInstanceBuffer,
        data,
        visible,
      );
      this.#agentInstanceCount = visible;
    }
    this.#agentLayer = { visibleInstanceCount: visible, totalInstanceCount: total };
    return this.#agentLayer;
  }

  /**
   * Update the viewport. Call this from a `ResizeObserver` so the renderer
   * can keep its drawable size in sync with the canvas's CSS size.
   */
  resize(viewport: { width: number; height: number }): void {
    if (this.#destroyed) return;
    this.#viewport = { width: viewport.width, height: viewport.height };
    this.#resizeCanvas();
  }

  /**
   * Update the camera. Translation is in world units; `scale` is CSS pixels
   * per world unit.
   */
  setCamera(camera: { translateX: number; translateY: number; scale: number }): void {
    this.#camera = { ...camera };
  }

  /** Begin the render loop. Idempotent. */
  start(): void {
    if (this.#rafHandle !== null || this.#destroyed) return;
    const tick = (): void => {
      this.#drawFrame();
      this.#rafHandle = requestAnimationFrame(tick);
    };
    this.#rafHandle = requestAnimationFrame(tick);
  }

  /** Stop the render loop. Idempotent. */
  stop(): void {
    if (this.#rafHandle !== null) {
      cancelAnimationFrame(this.#rafHandle);
      this.#rafHandle = null;
    }
  }

  /** Free GPU resources. Safe to call multiple times. */
  destroy(): void {
    if (this.#destroyed) return;
    this.stop();
    this.#cellInstanceBuffer?.destroy();
    this.#spotInstanceBuffer?.destroy();
    this.#agentInstanceBuffer?.destroy();
    this.#cellInstanceBuffer = null;
    this.#spotInstanceBuffer = null;
    this.#agentInstanceBuffer = null;
    this.#destroyed = true;
  }

  /** Render statistics: how many frames have been drawn since construction. */
  stats(): {
    framesRendered: number;
    cellLayer: HexGpuLayerState;
    spotLayer: HexGpuLayerState;
    agentLayer: HexGpuLayerState;
  } {
    return {
      framesRendered: this.#framesRendered,
      cellLayer: this.#cellLayer,
      spotLayer: this.#spotLayer,
      agentLayer: this.#agentLayer,
    };
  }

  #drawFrame(): void {
    if (this.#destroyed) return;
    const context = this.#canvas.getContext('webgpu');
    if (!context) return;
    const texture = context.getCurrentTexture();
    const view = texture.createView();

    // Upload camera + viewport. UniformBufferData must match the WGSL
    // struct exactly; padded to 16-byte alignment by adding two f32s.
    const aspect = this.#viewport.width / Math.max(this.#viewport.height, 1);
    const data = new Float32Array([
      this.#camera.translateX,
      this.#camera.translateY,
      this.#camera.scale,
      aspect,
      this.#hexSize,
      this.#lodDotPixels,
      this.#viewport.width,
      this.#viewport.height,
      0,
      0,
    ]);
    this.#ctx.device.queue.writeBuffer(this.#uniformBuffer, 0, data);

    const encoder = this.#ctx.device.createCommandEncoder();
    const pass = encoder.beginRenderPass({
      colorAttachments: [
        {
          view,
          clearValue: { r: 0.05, g: 0.05, b: 0.08, a: 1 },
          loadOp: 'clear',
          storeOp: 'store',
        },
      ],
    });

    pass.setPipeline(this.#pipeline);
    pass.setBindGroup(0, this.#bindGroup);
    pass.setVertexBuffer(1, this.#quadBuffer);

    if (this.#cellInstanceBuffer && this.#cellInstanceCount > 0) {
      pass.setVertexBuffer(0, this.#cellInstanceBuffer);
      pass.draw(6, this.#cellInstanceCount);
    }
    if (this.#spotInstanceBuffer && this.#spotInstanceCount > 0) {
      pass.setVertexBuffer(0, this.#spotInstanceBuffer);
      pass.draw(6, this.#spotInstanceCount);
    }
    if (this.#agentInstanceBuffer && this.#agentInstanceCount > 0) {
      pass.setVertexBuffer(0, this.#agentInstanceBuffer);
      pass.draw(6, this.#agentInstanceCount);
    }
    pass.end();
    this.#ctx.device.queue.submit([encoder.finish()]);
    this.#framesRendered += 1;
  }

  #uploadInstanceBuffer(
    prev: GPUBuffer | null,
    data: Float32Array,
    instanceCount: number,
  ): GPUBuffer {
    // Reuse the previous buffer if it has enough capacity. This avoids
    // a fresh allocation on every state change which would otherwise
    // cost a ~1ms stall on some drivers.
    const required = instanceCount * 24;
    if (prev && prev.size >= required) {
      this.#ctx.device.queue.writeBuffer(prev, 0, data.buffer, data.byteOffset, required);
      return prev;
    }
    prev?.destroy();
    const buffer = this.#ctx.device.createBuffer({
      size: Math.max(required, 24),
      usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
      mappedAtCreation: true,
    });
    new Float32Array(buffer.getMappedRange()).set(data.subarray(0, instanceCount * 6));
    buffer.unmap();
    return buffer;
  }

  #resizeCanvas(): void {
    const dpr = typeof window !== 'undefined' ? window.devicePixelRatio || 1 : 1;
    const w = Math.max(1, Math.floor(this.#viewport.width * dpr));
    const h = Math.max(1, Math.floor(this.#viewport.height * dpr));
    this.#canvas.width = w;
    this.#canvas.height = h;
  }

  /**
   * Cheap AABB frustum culling. The viewport is computed from the current
   * camera state; cells outside the screen-aligned rectangle are dropped
   * before upload. A small margin keeps the edges from popping at the
   * viewport boundary.
   */
  #isInFrustum(cx: number, cy: number): boolean {
    const halfW = this.#viewport.width / Math.max(this.#camera.scale, 0.001) / 2;
    const halfH = this.#viewport.height / Math.max(this.#camera.scale, 0.001) / 2;
    const margin = this.#hexSize * 2;
    return (
      cx >= this.#camera.translateX - halfW - margin &&
      cx <= this.#camera.translateX + halfW + margin &&
      cy >= this.#camera.translateY - halfH - margin &&
      cy <= this.#camera.translateY + halfH + margin
    );
  }
}

/**
 * Convenience helper that wraps the typical lifecycle: create a context,
 * initialize the renderer, and push the initial map. The returned promise
 * resolves to a ready-to-use renderer or rejects if WebGPU is not available
 * (the wrapper component is expected to fall back in that case).
 */
export async function createWebGpuMapRenderer(
  canvas: HTMLCanvasElement,
  map: HexMap,
  options?: WebGpuMapRendererOptions,
): Promise<WebGpuMapRenderer> {
  const ctx = await createWebGpuContext(canvas);
  const renderer = new WebGpuMapRenderer(ctx, canvas, options);
  await renderer.init();
  renderer.resize({ width: canvas.clientWidth || 800, height: canvas.clientHeight || 420 });
  // Center the camera on the map.
  const { width, height } = map;
  const cx = (SQRT3 * (options?.hexSize ?? 24) * width) / 2;
  const cy = (1.5 * (options?.hexSize ?? 24) * height) / 2;
  renderer.setCamera({ translateX: cx, translateY: cy, scale: 8 });
  renderer.updateMap(map);
  return renderer;
}
