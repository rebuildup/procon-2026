import { createWebGpuContext } from './device';
import { loadHexAssets, tileAtlas, TILE_COLUMNS, TILE_SIZE, TILE_WORLD_SIZE } from './hex-assets';
import { hexInstances, palette, worldPoint, type Camera, type ViewerScene } from './viewer-scene';

const shader = `
struct View {camera:vec4f,viewport:vec4f};
@group(0) @binding(0) var<uniform> view:View;
@group(0) @binding(1) var atlasSampler:sampler;
@group(0) @binding(2) var atlas:texture_2d<f32>;
struct Vertex {@builtin(position) position:vec4f,@location(0) uv:vec2f};
@vertex fn vs(@builtin(vertex_index) index:u32,@location(0) rect:vec4f,@location(1) tile:f32)->Vertex {
 let corners=array<vec2f,6>(vec2f(0,0),vec2f(1,0),vec2f(0,1),vec2f(0,1),vec2f(1,0),vec2f(1,1));
 let corner=corners[index];let world=rect.xy+corner*rect.zw;
 let fit=min(view.viewport.x/370.0,view.viewport.y/265.0);
 let offset=(view.viewport.xy-vec2f(370,265)*fit)*0.5;
 let pixel=(world*view.camera.z+view.camera.xy+vec2f(24))*fit+offset;
 var out:Vertex;out.position=vec4f(pixel.x/view.viewport.x*2-1,1-pixel.y/view.viewport.y*2,0,1);
 let origin=vec2f(f32(u32(tile)%${TILE_COLUMNS}u),floor(tile/${TILE_COLUMNS}.0))*${TILE_SIZE}.0;
 out.uv=(origin+corner*${TILE_SIZE}.0)/vec2f(textureDimensions(atlas));return out;
}
@fragment fn fs(in:Vertex)->@location(0) vec4f {return textureSample(atlas,atlasSampler,in.uv);}
`;
export async function createViewerLayer(canvas: HTMLCanvasElement, fail: (reason: string) => void) {
  await loadHexAssets();
  const context = await createWebGpuContext(canvas),
    device = context.device;
  try {
    device.pushErrorScope('validation');
    const module = device.createShaderModule({ code: shader });
    const pipeline = device.createRenderPipeline({
      layout: 'auto',
      vertex: {
        module,
        entryPoint: 'vs',
        buffers: [
          {
            arrayStride: 20,
            stepMode: 'instance',
            attributes: [
              { shaderLocation: 0, offset: 0, format: 'float32x4' },
              { shaderLocation: 1, offset: 16, format: 'float32' },
            ],
          },
        ],
      },
      fragment: {
        module,
        entryPoint: 'fs',
        targets: [
          {
            format: context.format,
            blend: {
              color: { srcFactor: 'src-alpha', dstFactor: 'one-minus-src-alpha' },
              alpha: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha' },
            },
          },
        ],
      },
      primitive: { topology: 'triangle-list' },
    });
    const uniform = device.createBuffer({
      size: 32,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    let capacity = 64;
    let buffer = device.createBuffer({
      size: capacity * 20,
      usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
    });
    const atlas = tileAtlas();
    const texture = device.createTexture({
      size: [atlas.width, atlas.height],
      format: 'rgba8unorm',
      usage:
        GPUTextureUsage.TEXTURE_BINDING |
        GPUTextureUsage.COPY_DST |
        GPUTextureUsage.RENDER_ATTACHMENT,
    });
    device.queue.copyExternalImageToTexture({ source: atlas }, { texture }, [
      atlas.width,
      atlas.height,
    ]);
    const group = device.createBindGroup({
      layout: pipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: uniform } },
        {
          binding: 1,
          resource: device.createSampler({ magFilter: 'linear', minFilter: 'linear' }),
        },
        { binding: 2, resource: texture.createView() },
      ],
    });
    const error = await device.popErrorScope();
    if (error) throw new Error(error.message);
    let destroyed = false,
      lastInstances: ReturnType<typeof hexInstances> | undefined;
    let uploads = 0,
      draws = 0;
    device.addEventListener('uncapturederror', () => {
      if (!destroyed) fail('GPU rendering failed');
    });
    void device.lost.then((info) => {
      if (!destroyed) fail(info.message || 'GPU device lost');
    });
    return {
      context,
      stats: () => ({ uploads, draws, atlasUploads: 1 }),
      draw(scene: ViewerScene, camera: Camera) {
        if (destroyed) return;
        const rect = canvas.getBoundingClientRect();
        if (!rect.width || !rect.height) return;
        const width = Math.round(rect.width * devicePixelRatio),
          height = Math.round(rect.height * devicePixelRatio);
        if (canvas.width !== width || canvas.height !== height) {
          canvas.width = width;
          canvas.height = height;
        }
        const instances = hexInstances(scene);
        if (instances !== lastInstances) {
          if (instances.length > capacity) {
            buffer.destroy();
            capacity = 2 ** Math.ceil(Math.log2(instances.length));
            buffer = device.createBuffer({
              size: capacity * 20,
              usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
            });
          }
          device.queue.writeBuffer(
            buffer,
            0,
            new Float32Array(
              instances.flatMap((i) => [i.x, i.y, TILE_WORLD_SIZE, TILE_WORLD_SIZE, i.tile]),
            ),
          );
          lastInstances = instances;
          uploads++;
        }
        device.queue.writeBuffer(
          uniform,
          0,
          new Float32Array([camera.x, camera.y, camera.zoom, 0, rect.width, rect.height, 0, 0]),
        );
        const bg = palette()('--bgColor-muted'),
          encoder = device.createCommandEncoder();
        const pass = encoder.beginRenderPass({
          colorAttachments: [
            {
              view: canvas.getContext('webgpu')!.getCurrentTexture().createView(),
              clearValue: { r: bg[0], g: bg[1], b: bg[2], a: bg[3] },
              loadOp: 'clear',
              storeOp: 'store',
            },
          ],
        });
        pass.setPipeline(pipeline);
        pass.setBindGroup(0, group);
        pass.setVertexBuffer(0, buffer);
        pass.draw(6, instances.length);
        pass.end();
        device.queue.submit([encoder.finish()]);
        draws++;
      },
      async verifyFrame(scene: ViewerScene, camera: Camera) {
        await device.queue.onSubmittedWorkDone();
        const rect = canvas.getBoundingClientRect();
        const point = scene.cells
          .map((cell) => worldPoint(rect.width, rect.height, cell.x, cell.y, camera))
          .find((p) => p.x > 2 && p.y > 2 && p.x < rect.width - 2 && p.y < rect.height - 2);
        if (!point) return;
        const probe = document.createElement('canvas');
        probe.width = canvas.width;
        probe.height = canvas.height;
        const ctx = probe.getContext('2d')!;
        ctx.drawImage(canvas, 0, 0);
        const rgba = ctx.getImageData(
            Math.floor(point.x * devicePixelRatio),
            Math.floor(point.y * devicePixelRatio),
            1,
            1,
          ).data,
          bg = palette()('--bgColor-muted');
        if (rgba[3] === 0 || bg.slice(0, 3).every((v, i) => Math.abs(v * 255 - rgba[i]!) < 3))
          throw new Error('blank-frame');
      },
      destroy() {
        destroyed = true;
        buffer.destroy();
        uniform.destroy();
        texture.destroy();
        device.destroy();
      },
    };
  } catch (error) {
    device.destroy();
    throw error;
  }
}
