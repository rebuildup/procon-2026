import { createWebGpuContext } from './device';
import {
  GLYPH_SIZE,
  HEX_POINTS,
  glyphAtlas,
  markNames,
  palette,
  terrainRgba,
  worldPoint,
  type Camera,
  type ViewerScene,
} from './viewer-scene';

const shader = `
struct View { camera: vec4f, viewport: vec4f };
@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var glyphSampler: sampler;
@group(0) @binding(2) var glyphs: texture_2d<f32>;
fn position(world:vec2f)->vec4f {
 let fit=min(view.viewport.x/370.0,view.viewport.y/265.0);
 let offset=(view.viewport.xy-vec2f(370.0,265.0)*fit)*0.5;
 let pixel=(world*view.camera.z+view.camera.xy+vec2f(24.0))*fit+offset;
 return vec4f(pixel.x/view.viewport.x*2.0-1.0,1.0-pixel.y/view.viewport.y*2.0,0.0,1.0);
}
struct Terrain { @builtin(position) position:vec4f, @location(0) fill:vec4f, @location(1) border:vec4f, @location(2) local:vec2f };
@vertex fn terrainVS(@location(0) center:vec2f,@location(1) fill:vec4f,@location(2) border:vec4f,@location(3) corner:vec2f)->Terrain {
 var out:Terrain;out.position=position(center+corner);out.fill=fill;out.border=border;out.local=corner;return out;
}
@fragment fn terrainFS(in:Terrain)->@location(0) vec4f {
 let distance=max(abs(in.local.x)-20.0,(abs(in.local.x)*0.575+abs(in.local.y)-23.0)/1.15353);
 let aa=max(fwidth(distance),0.01);
 var fill=in.fill;
 let p=in.local+vec2f(24.0);
 let kind=in.border.a;
 if(kind<1.5){let q=fract(p/5.0)*5.0-vec2f(2.5);if(length(q)<1.0){fill=vec4f(0.4353,0.7216,0.3412,1);}}
 else if(kind<2.5){if(abs(fract((p.x-p.y)/6.0)-0.5)<0.1167){fill=vec4f(0.5608,0.5608,0.5608,1);}}
 else if(kind<3.5){if(abs(fract((p.y-0.8*sin(p.x*0.849))/5.0)-0.64)<0.11){fill=vec4f(0.6078,0.7608,0.9020,1);}}
 else{let q=fract(p/8.0)*8.0;if(q.x>1.0&&q.x<4.0&&abs(q.y-4.0)<0.6){fill=vec4f(0.6078,0.7608,0.9020,1);}}
 let ink=vec4f(0.1294,0.1412,0.1569,1);
 let ring=mix(ink,vec4f(1),smoothstep(0.6-aa*0.5,0.6+aa*0.5,distance));
 return mix(fill,ring,smoothstep(-0.6-aa*0.5,-0.6+aa*0.5,distance));
}
struct Symbol { @builtin(position) position:vec4f, @location(0) color:vec4f, @location(1) uv:vec2f };
@vertex fn symbolVS(@builtin(vertex_index) index:u32,@location(0) rect:vec4f,@location(1) color:vec4f,@location(2) glyph:f32)->Symbol {
 let corners=array<vec2f,6>(vec2f(0,0),vec2f(1,0),vec2f(0,1),vec2f(0,1),vec2f(1,0),vec2f(1,1));
 let corner=corners[index];var out:Symbol;out.position=position(rect.xy+corner*rect.zw);out.color=color;out.uv=vec2f((glyph+corner.x)/${markNames.length}.0,corner.y);return out;
}
@fragment fn symbolFS(in:Symbol)->@location(0) vec4f {
 return textureSample(glyphs,glyphSampler,in.uv);
}
`;
export async function createViewerLayer(canvas: HTMLCanvasElement, fail: (reason: string) => void) {
  const ctx = await createWebGpuContext(canvas),
    device = ctx.device;
  try {
    device.pushErrorScope('validation');
    const module = device.createShaderModule({ code: shader });
    const bindings = device.createBindGroupLayout({
      entries: [
        { binding: 0, visibility: GPUShaderStage.VERTEX, buffer: { type: 'uniform' } },
        { binding: 1, visibility: GPUShaderStage.FRAGMENT, sampler: { type: 'filtering' } },
        { binding: 2, visibility: GPUShaderStage.FRAGMENT, texture: { sampleType: 'float' } },
      ],
    });
    const layout = device.createPipelineLayout({ bindGroupLayouts: [bindings] });
    const blend: GPUBlendState = {
      color: { srcFactor: 'src-alpha', dstFactor: 'one-minus-src-alpha' },
      alpha: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha' },
    };
    const terrain = device.createRenderPipeline({
      layout,
      vertex: {
        module,
        entryPoint: 'terrainVS',
        buffers: [
          {
            arrayStride: 40,
            stepMode: 'instance',
            attributes: [
              { shaderLocation: 0, offset: 0, format: 'float32x2' },
              { shaderLocation: 1, offset: 8, format: 'float32x4' },
              { shaderLocation: 2, offset: 24, format: 'float32x4' },
            ],
          },
          { arrayStride: 8, attributes: [{ shaderLocation: 3, offset: 0, format: 'float32x2' }] },
        ],
      },
      fragment: { module, entryPoint: 'terrainFS', targets: [{ format: ctx.format, blend }] },
      primitive: { topology: 'triangle-list' },
    });
    const symbols = device.createRenderPipeline({
      layout,
      vertex: {
        module,
        entryPoint: 'symbolVS',
        buffers: [
          {
            arrayStride: 36,
            stepMode: 'instance',
            attributes: [
              { shaderLocation: 0, offset: 0, format: 'float32x4' },
              { shaderLocation: 1, offset: 16, format: 'float32x4' },
              { shaderLocation: 2, offset: 32, format: 'float32' },
            ],
          },
        ],
      },
      fragment: { module, entryPoint: 'symbolFS', targets: [{ format: ctx.format, blend }] },
      primitive: { topology: 'triangle-list' },
    });
    const vertices = new Float32Array(
      HEX_POINTS.flatMap((point, i) => [
        0,
        0,
        ...point,
        ...HEX_POINTS[(i + 1) % 6]!.map((v) => v * 1.075),
      ]),
    );
    const corners = device.createBuffer({
      size: vertices.byteLength,
      usage: GPUBufferUsage.VERTEX,
      mappedAtCreation: true,
    });
    new Float32Array(corners.getMappedRange()).set(vertices);
    corners.unmap();
    const cells = device.createBuffer({
      size: 48 * 40,
      usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
    });
    const marks = device.createBuffer({
      size: 56 * 36,
      usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
    });
    const uniform = device.createBuffer({
      size: 32,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    const atlas = glyphAtlas();
    const texture = device.createTexture({
      size: [atlas.width, GLYPH_SIZE],
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
      layout: bindings,
      entries: [
        { binding: 0, resource: { buffer: uniform } },
        {
          binding: 1,
          resource: device.createSampler({ magFilter: 'linear', minFilter: 'linear' }),
        },
        { binding: 2, resource: texture.createView() },
      ],
    });
    const validation = await device.popErrorScope();
    if (validation) throw new Error(validation.message);
    let destroyed = false;
    void device.lost.then((info) => {
      if (!destroyed) fail(info.message || 'GPU device lost');
    });
    device.addEventListener('uncapturederror', () => {
      if (!destroyed) fail('GPU rendering failed');
    });
    const context = canvas.getContext('webgpu')!;
    return {
      context: ctx,
      draw(scene: ViewerScene, camera: Camera) {
        if (destroyed) return;
        if (scene.cells.length > 48 || scene.marks.length > 56)
          throw new Error('Viewer scene exceeds allocated instance capacity');
        const rect = canvas.getBoundingClientRect();
        if (!rect.width || !rect.height) return;
        const width = Math.round(rect.width * devicePixelRatio),
          height = Math.round(rect.height * devicePixelRatio);
        if (canvas.width !== width || canvas.height !== height) {
          canvas.width = width;
          canvas.height = height;
        }
        const colors = palette();
        device.queue.writeBuffer(
          cells,
          0,
          new Float32Array(
            scene.cells.flatMap((cell) => [
              cell.x,
              cell.y,
              ...terrainRgba(cell.mark),
              0,
              0,
              0,
              ['plain', 'mountain', 'water', 'road'].indexOf(cell.mark) + 1,
            ]),
          ),
        );
        device.queue.writeBuffer(
          marks,
          0,
          new Float32Array(
            scene.marks.flatMap((mark) => [
              mark.x,
              mark.y,
              mark.size,
              mark.size,
              ...colors(`--data-${mark.hue}-color-emphasis`),
              markNames.indexOf(mark.mark),
            ]),
          ),
        );
        device.queue.writeBuffer(
          uniform,
          0,
          new Float32Array([camera.x, camera.y, camera.zoom, 0, rect.width, rect.height, 0, 0]),
        );
        const bg = colors('--bgColor-muted'),
          encoder = device.createCommandEncoder();
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
        pass.setBindGroup(0, group);
        pass.setPipeline(terrain);
        pass.setVertexBuffer(0, cells);
        pass.setVertexBuffer(1, corners);
        pass.draw(18, scene.cells.length);
        pass.setPipeline(symbols);
        pass.setVertexBuffer(0, marks);
        pass.draw(6, scene.marks.length);
        pass.end();
        device.queue.submit([encoder.finish()]);
      },
      async verifyFrame(scene: ViewerScene, camera: Camera) {
        await device.queue.onSubmittedWorkDone();
        const rect = canvas.getBoundingClientRect();
        const sample = scene.cells
          .map((cell) => worldPoint(rect.width, rect.height, cell.x, cell.y, camera))
          .find((p) => p.x > 2 && p.y > 2 && p.x < rect.width - 2 && p.y < rect.height - 2);
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
          ).data,
          bg = palette()('--bgColor-muted');
        if (
          rgba[3] === 0 ||
          bg.slice(0, 3).every((channel, i) => Math.abs(channel * 255 - rgba[i]!) < 3)
        )
          throw new Error('blank-frame');
      },
      destroy() {
        destroyed = true;
        corners.destroy();
        cells.destroy();
        marks.destroy();
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
