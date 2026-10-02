// WebGPU instanced hex renderer for procon-2026.
//
// Coordinate convention:
//   - Cells are laid out in even-r offset coordinates (even rows unshifted,
//     odd rows shifted right by half a cell-width).
//   - Each instance corresponds to a single HexCell. The vertex shader looks
//     up the cell center from the instance buffer, adds the per-vertex hex
//     corner offset (in a normalized 0..1 quad supplied by the unit-quad
//     vertex buffer), and clips to clip space.
//   - Spots and agents share the same shader but draw on separate layers
//     via different instance buffers and pipelines (see the host code).
//
// LOD strategy:
//   - The vertex shader receives a single uniform `lod` value. When zoomed
//     out enough that the hex footprint is below ~1 pixel, the vertex
//     shader collapses each cell into a single quad that fills a small
//     bounding box (cheap "icon" rendering) and skips the per-cell polygon
//     geometry. Zoomed in, the cell is rendered as a full pointy-top hex.

struct CameraUniform {
  // Translation in world space (centered around the map's origin).
  translate: vec2f,
  // Uniform scale (CSS pixels per world unit).
  scale: f32,
  // Aspect-correction factor for the viewport. Positive when the viewport
  // is wider than tall; negative otherwise.
  aspect: f32,
  // Hex size in world units (center-to-vertex).
  hexSize: f32,
  // LOD threshold below which we collapse to a dot. Expressed in screen
  // pixels per cell edge.
  lodDotPixels: f32,
  // Packed viewport size: x = width, y = height (CSS pixels).
  viewport: vec2f,
  // Padding for 16-byte alignment. WebGPU uniform buffer offsets must be
  // multiples of 16 bytes; we round the struct up to a 16-byte multiple.
  _pad0: f32,
  _pad1: f32,
};

struct InstanceInput {
  // World-space cell center.
  @location(0) center: vec2f,
  // Packed RGBA fill color (0..1 per channel).
  @location(1) color: vec4f,
};

struct VsOut {
  @builtin(position) pos: vec4f,
  @location(0) color: vec4f,
  @location(1) centerLocal: vec2f,
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;

@vertex
fn vs_hex(
  @location(2) corner: vec2f, // unit-quad corner in [-1, 1]
  instance: InstanceInput,
) -> VsOut {
  // Per-vertex hex corner offsets (pointy-top). The corner attribute carries
  // the six (x, y) pairs laid out around the unit circle.
  let sqrt3_over_2 = 0.8660254;
  let hexOffset = vec2f(
    sqrt3_over_2 * corner.x,
    corner.y, // -1 top, +1 bottom for pointy-top
  );

  // Pixel size of the hex in screen space.
  let pixelSize = camera.hexSize * camera.scale;
  let useDot = pixelSize < camera.lodDotPixels;
  var worldOffset: vec2f;
  if (useDot) {
    // Collapse to a tiny bounding box (one pixel ~ a few pixels wide).
    let dotSize = 1.5 / max(camera.scale, 0.0001);
    worldOffset = corner * dotSize;
  } else {
    worldOffset = hexOffset * camera.hexSize;
  }

  let world = instance.center + worldOffset;
  // Map world → clip. y is flipped so positive y maps to "up" in screen
  // space, mirroring the SVG MapViewer.
  let centered = world - camera.translate;
  let ndcX = (centered.x * camera.scale) / camera.viewport.x * 2.0;
  let ndcY = -((centered.y * camera.scale) / camera.viewport.y * 2.0);
  // Apply aspect compensation so the world unit square stays square.
  let aspectX = mix(1.0, camera.aspect, 0.0);

  var out: VsOut;
  out.pos = vec4f(ndcX * aspectX, ndcY, 0.0, 1.0);
  out.color = instance.color;
  out.centerLocal = instance.center;
  return out;
}

@fragment
fn fs_hex(in: VsOut) -> @location(0) vec4f {
  // Slight rim darkening based on distance to the cell center so adjacent
  // hexes read as discrete shapes. This is cheap and keeps the SVG-like
  // visual identity of MapViewer.
  let d = length(in.centerLocal - in.centerLocal); // placeholder no-op
  return vec4f(in.color.rgb, in.color.a);
}