/**
 * Minimal WebGPU renderer used by `<GpuCanvas />` to verify that the device,
 * pipeline, and shader all wire together. Real rendering for the contest
 * problem lands in subsequent tickets.
 */

import type { WebGpuContext } from '@/lib/webgpu/device';

const SHADER_MODULE = /* wgsl */ `
struct VsOut {
  @builtin(position) pos: vec4f,
  @location(0) uv: vec2f,
};

@vertex
fn vs(@location(0) pos: vec2f, @location(1) uv: vec2f) -> VsOut {
  var out: VsOut;
  out.pos = vec4f(pos, 0.0, 1.0);
  out.uv = uv;
  return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4f {
  let c = vec3f(in.uv, 0.5 + 0.5 * sin(in.uv.x * 12.5664));
  return vec4f(c, 1.0);
}
`;

const QUAD_VERTICES = new Float32Array([-1, -1, 0, 1, 1, -1, 1, 1, -1, 1, 0, 0, 1, 1, 1, 0]);

export interface RendererStats {
  readonly framesRendered: number;
}

export class PlaceholderRenderer {
  readonly #ctx: WebGpuContext;
  readonly #canvas: HTMLCanvasElement;
  #pipeline!: GPURenderPipeline;
  #vertexBuffer!: GPUBuffer;
  #framesRendered = 0;
  #rafHandle: number | null = null;

  constructor(ctx: WebGpuContext, canvas: HTMLCanvasElement) {
    this.#ctx = ctx;
    this.#canvas = canvas;
  }

  async init(): Promise<void> {
    const module = this.#ctx.device.createShaderModule({ code: SHADER_MODULE });
    this.#pipeline = this.#ctx.device.createRenderPipeline({
      layout: 'auto',
      vertex: {
        module,
        entryPoint: 'vs',
        buffers: [
          {
            arrayStride: 16,
            attributes: [
              { shaderLocation: 0, offset: 0, format: 'float32x2' },
              { shaderLocation: 1, offset: 8, format: 'float32x2' },
            ],
          },
        ],
      },
      fragment: { module, entryPoint: 'fs', targets: [{ format: this.#ctx.format }] },
      primitive: { topology: 'triangle-strip' },
    });
    this.#vertexBuffer = this.#ctx.device.createBuffer({
      size: QUAD_VERTICES.byteLength,
      usage: GPUBufferUsage.VERTEX,
      mappedAtCreation: true,
    });
    new Float32Array(this.#vertexBuffer.getMappedRange()).set(QUAD_VERTICES);
    this.#vertexBuffer.unmap();
  }

  start(): void {
    if (this.#rafHandle !== null) return;
    const tick = (): void => {
      this.#drawFrame();
      this.#rafHandle = requestAnimationFrame(tick);
    };
    this.#rafHandle = requestAnimationFrame(tick);
  }

  stop(): void {
    if (this.#rafHandle !== null) {
      cancelAnimationFrame(this.#rafHandle);
      this.#rafHandle = null;
    }
  }

  stats(): RendererStats {
    return { framesRendered: this.#framesRendered };
  }

  #drawFrame(): void {
    const context = this.#canvas.getContext('webgpu');
    if (!context) return;
    const texture = context.getCurrentTexture();
    const view = texture.createView();
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
    pass.setVertexBuffer(0, this.#vertexBuffer);
    pass.draw(4);
    pass.end();
    this.#ctx.device.queue.submit([encoder.finish()]);
    this.#framesRendered += 1;
  }
}
