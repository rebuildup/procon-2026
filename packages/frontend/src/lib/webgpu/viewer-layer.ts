import { createWebGpuContext } from './device';

export interface ViewerCell {
  col: number;
  row: number;
  hue: string;
}
const shader = `
struct View { camera: vec4f, viewport: vec4f };
@group(0) @binding(0) var<uniform> view: View;
struct Out { @builtin(position) position: vec4f, @location(0) color: vec4f };
@vertex fn vs(@location(0) center: vec2f, @location(1) color: vec4f, @location(2) corner: vec2f) -> Out {
 let world = (center + corner) * view.camera.z + view.camera.xy;
 let fit = min(view.viewport.x / 370.0, view.viewport.y / 265.0);
 let offset = (view.viewport.xy - vec2f(370.0, 265.0) * fit) * 0.5;
 let pixel = (world + vec2f(24.0)) * fit + offset;
 var out: Out;
 out.position = vec4f(pixel.x / view.viewport.x * 2.0 - 1.0, 1.0 - pixel.y / view.viewport.y * 2.0, 0.0, 1.0);
 out.color = color;
 return out;
}
@fragment fn fs(in: Out) -> @location(0) vec4f { return in.color; }
`;
function color(token: string): [number, number, number, number] {
  const value = getComputedStyle(document.documentElement).getPropertyValue(token).trim();
  if (!/^#[0-9a-f]{6}([0-9a-f]{2})?$/i.test(value))
    throw new Error(`Unresolved Primer color ${token}`);
  return [
    parseInt(value.slice(1, 3), 16) / 255,
    parseInt(value.slice(3, 5), 16) / 255,
    parseInt(value.slice(5, 7), 16) / 255,
    value.length === 9 ? parseInt(value.slice(7, 9), 16) / 255 : 1,
  ];
}
export async function createViewerLayer(
  canvas: HTMLCanvasElement,
  cells: ViewerCell[],
  fail: (reason: string) => void,
) {
  const ctx = await createWebGpuContext(canvas);
  const device = ctx.device;
  device.pushErrorScope('validation');
  const module = device.createShaderModule({ code: shader });
  const pipeline = device.createRenderPipeline({
    layout: 'auto',
    vertex: {
      module,
      entryPoint: 'vs',
      buffers: [
        {
          arrayStride: 24,
          stepMode: 'instance',
          attributes: [
            { shaderLocation: 0, offset: 0, format: 'float32x2' },
            { shaderLocation: 1, offset: 8, format: 'float32x4' },
          ],
        },
        { arrayStride: 8, attributes: [{ shaderLocation: 2, offset: 0, format: 'float32x2' }] },
      ],
    },
    fragment: {
      module,
      entryPoint: 'fs',
      targets: [
        {
          format: ctx.format,
          blend: {
            color: { srcFactor: 'src-alpha', dstFactor: 'one-minus-src-alpha' },
            alpha: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha' },
          },
        },
      ],
    },
    primitive: { topology: 'triangle-list' },
  });
  const validation = await device.popErrorScope();
  if (validation) {
    device.destroy();
    throw new Error(validation.message);
  }
  const points = [
    [0, -23],
    [20, -11.5],
    [20, 11.5],
    [0, 23],
    [-20, 11.5],
    [-20, -11.5],
  ];
  const vertices = new Float32Array(
    points.flatMap((point, i) => [0, 0, ...point, ...points[(i + 1) % 6]!]),
  );
  const corners = device.createBuffer({
    size: vertices.byteLength,
    usage: GPUBufferUsage.VERTEX,
    mappedAtCreation: true,
  });
  new Float32Array(corners.getMappedRange()).set(vertices);
  corners.unmap();
  const instances = device.createBuffer({
    size: cells.length * 24,
    usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
  });
  const uniform = device.createBuffer({
    size: 32,
    usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
  });
  const group = device.createBindGroup({
    layout: pipeline.getBindGroupLayout(0),
    entries: [{ binding: 0, resource: { buffer: uniform } }],
  });
  let destroyed = false;
  void device.lost.then((info) => {
    if (!destroyed) fail(info.message || 'GPU device lost');
  });
  device.addEventListener('uncapturederror', () => fail('GPU rendering failed'));
  return {
    context: ctx,
    draw(x: number, y: number, zoom: number) {
      if (destroyed) return;
      const rect = canvas.getBoundingClientRect();
      if (!rect.width || !rect.height) return;
      canvas.width = Math.round(rect.width * devicePixelRatio);
      canvas.height = Math.round(rect.height * devicePixelRatio);
      const values = new Float32Array(
        cells.flatMap((cell) => [
          cell.col * 40 + (cell.row % 2) * 20,
          cell.row * 35,
          ...color(`--data-${cell.hue}-color-muted`),
        ]),
      );
      device.queue.writeBuffer(instances, 0, values);
      device.queue.writeBuffer(
        uniform,
        0,
        new Float32Array([x, y, zoom, 0, rect.width, rect.height, 0, 0]),
      );
      const context = canvas.getContext('webgpu')!;
      const encoder = device.createCommandEncoder();
      const bg = color('--bgColor-muted');
      const pass = encoder.beginRenderPass({
        colorAttachments: [
          {
            view: context.getCurrentTexture().createView(),
            clearValue: { r: bg[0], g: bg[1], b: bg[2], a: bg[3] },
            loadOp: 'clear',
            storeOp: 'store',
          },
        ],
      });
      pass.setPipeline(pipeline);
      pass.setBindGroup(0, group);
      pass.setVertexBuffer(0, instances);
      pass.setVertexBuffer(1, corners);
      pass.draw(18, cells.length);
      pass.end();
      device.queue.submit([encoder.finish()]);
    },
    async verifyFrame(x: number, y: number, zoom: number) {
      await device.queue.onSubmittedWorkDone();
      const rect = canvas.getBoundingClientRect();
      const fit = Math.min(rect.width / 370, rect.height / 265);
      const offsetX = (rect.width - 370 * fit) / 2,
        offsetY = (rect.height - 265 * fit) / 2;
      const sample = cells
        .map((cell) => ({
          x: ((cell.col * 40 + (cell.row % 2) * 20) * zoom + x + 24) * fit + offsetX,
          y: (cell.row * 35 * zoom + y + 24) * fit + offsetY,
        }))
        .find(
          (point) =>
            point.x > 2 && point.y > 2 && point.x < rect.width - 2 && point.y < rect.height - 2,
        );
      if (!sample) return;
      const probe = document.createElement('canvas');
      probe.width = canvas.width;
      probe.height = canvas.height;
      const context = probe.getContext('2d');
      if (!context) throw new Error('Cannot verify GPU frame');
      context.drawImage(canvas, 0, 0);
      const rgba = context.getImageData(
        Math.floor(sample.x * devicePixelRatio),
        Math.floor(sample.y * devicePixelRatio),
        1,
        1,
      ).data;
      const bg = color('--bgColor-muted');
      if (
        rgba[3] === 0 ||
        bg.slice(0, 3).every((channel, i) => Math.abs(channel * 255 - rgba[i]!) < 3)
      )
        throw new Error('blank-frame');
    },
    destroy() {
      destroyed = true;
      corners.destroy();
      instances.destroy();
      uniform.destroy();
      device.destroy();
    },
  };
}
