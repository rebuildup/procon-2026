import marks from '@/viewer-marks.json';

export type Mark = keyof typeof marks;
export interface Camera {
  x: number;
  y: number;
  zoom: number;
}
export interface ViewerCell {
  x: number;
  y: number;
  hue: string;
  mark: Mark;
}
export interface ViewerMark {
  x: number;
  y: number;
  size: number;
  hue: string;
  mark: Mark;
}
export interface ViewerScene {
  cells: ViewerCell[];
  marks: ViewerMark[];
}
export const markNames = Object.keys(marks) as Mark[];
export const GLYPH_SIZE = 64;
export const HEX_POINTS = [
  [0, -23],
  [20, -11.5],
  [20, 11.5],
  [0, 23],
  [-20, 11.5],
  [-20, -11.5],
] as const;
const terrainHue = { plain: 'green', mountain: 'gray', water: 'blue', road: 'brown' };
export function fixtureScene(turn: number): ViewerScene {
  const cells: ViewerCell[] = Array.from({ length: 48 }, (_, i) => {
    const mark = i % 11 === 0 ? 'mountain' : i % 9 === 0 ? 'water' : i % 5 === 0 ? 'road' : 'plain';
    return {
      x: (i % 8) * 40 + (Math.floor(i / 8) % 2) * 20,
      y: Math.floor(i / 8) * 35,
      hue: terrainHue[mark],
      mark,
    };
  });
  const hues = ['yellow', 'orange', 'auburn', 'purple', 'plum', 'olive'];
  return {
    cells,
    marks: [
      ...cells.map((cell) => ({
        x: cell.x - 8,
        y: cell.y - 8,
        size: 16,
        hue: cell.hue,
        mark: cell.mark,
      })),
      ...hues.map((hue, n) => ({
        x: 40 + n * 40,
        y: 27 + (n % 2) * 70,
        size: 18,
        hue,
        mark: `s${n}` as Mark,
      })),
      { x: 40 + (turn % 6) * 40, y: 132, size: 20, hue: 'pine', mark: 'patrol' },
      { x: 220, y: 100, size: 20, hue: 'pink', mark: 'supply' },
    ],
  };
}
interface GlyphNode {
  tag: string;
  attrs: Record<string, string>;
  children: GlyphNode[];
}
let atlas: OffscreenCanvas | undefined;
export function glyphAtlas(): OffscreenCanvas {
  if (atlas) return atlas;
  const result = new OffscreenCanvas(GLYPH_SIZE * markNames.length, GLYPH_SIZE);
  const ctx = result.getContext('2d');
  if (!ctx) throw new Error('No glyph atlas context');
  const paint = (node: GlyphNode, inherited: Record<string, string>) => {
    const attrs = { ...inherited, ...node.attrs };
    if (node.tag === 'g') {
      for (const child of node.children) paint(child, attrs);
      return;
    }
    const n = (name: string) => Number(attrs[name] ?? 0);
    const path = new Path2D();
    if (node.tag === 'path') path.addPath(new Path2D(attrs.d));
    else if (node.tag === 'rect') path.roundRect(n('x'), n('y'), n('width'), n('height'), n('rx'));
    else if (node.tag === 'circle') path.arc(n('cx'), n('cy'), n('r'), 0, Math.PI * 2);
    else throw new Error(`Unsupported glyph shape: ${node.tag}`);
    if (attrs.fill !== 'none') {
      ctx.fillStyle = 'white';
      ctx.fill(path);
    }
    if (attrs.stroke && attrs.stroke !== 'none') {
      ctx.strokeStyle = 'white';
      ctx.lineWidth = Number(attrs['stroke-width'] ?? 1);
      ctx.lineCap = (attrs['stroke-linecap'] ?? 'butt') as CanvasLineCap;
      ctx.lineJoin = (attrs['stroke-linejoin'] ?? 'miter') as CanvasLineJoin;
      ctx.stroke(path);
    }
  };
  markNames.forEach((name, index) => {
    ctx.save();
    ctx.translate(index * GLYPH_SIZE, 0);
    ctx.scale(GLYPH_SIZE / 16, GLYPH_SIZE / 16);
    for (const node of marks[name]) paint(node, {});
    ctx.restore();
  });
  atlas = result;
  return result;
}
export function palette(): (token: string) => [number, number, number, number] {
  const style = getComputedStyle(document.documentElement);
  const cache = new Map<string, [number, number, number, number]>();
  return (token) => {
    const cached = cache.get(token);
    if (cached) return cached;
    const value = style.getPropertyValue(token).trim();
    if (!/^#[0-9a-f]{6}([0-9a-f]{2})?$/i.test(value))
      throw new Error(`Unresolved Primer color ${token}`);
    const rgba: [number, number, number, number] = [
      parseInt(value.slice(1, 3), 16) / 255,
      parseInt(value.slice(3, 5), 16) / 255,
      parseInt(value.slice(5, 7), 16) / 255,
      value.length === 9 ? parseInt(value.slice(7, 9), 16) / 255 : 1,
    ];
    cache.set(token, rgba);
    return rgba;
  };
}
export function cssColor(rgba: readonly number[]): string {
  return `rgba(${rgba[0]! * 255},${rgba[1]! * 255},${rgba[2]! * 255},${rgba[3]})`;
}
export function worldPoint(width: number, height: number, x: number, y: number, camera: Camera) {
  const fit = Math.min(width / 370, height / 265);
  return {
    x: (x * camera.zoom + camera.x + 24) * fit + (width - 370 * fit) / 2,
    y: (y * camera.zoom + camera.y + 24) * fit + (height - 265 * fit) / 2,
    fit,
  };
}
const tinted = new Map<string, OffscreenCanvas>();
export function coloredAtlas(color: string): OffscreenCanvas {
  const cached = tinted.get(color);
  if (cached) return cached;
  const source = glyphAtlas();
  const result = new OffscreenCanvas(source.width, source.height);
  const ctx = result.getContext('2d')!;
  ctx.drawImage(source, 0, 0);
  ctx.globalCompositeOperation = 'source-in';
  ctx.fillStyle = color;
  ctx.fillRect(0, 0, result.width, result.height);
  if (tinted.size >= 24) tinted.clear();
  tinted.set(color, result);
  return result;
}
export function drawCanvasScene(canvas: HTMLCanvasElement, scene: ViewerScene, camera: Camera) {
  const rect = canvas.getBoundingClientRect();
  if (!rect.width || !rect.height) return;
  const width = Math.round(rect.width * devicePixelRatio),
    height = Math.round(rect.height * devicePixelRatio);
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('No Canvas fallback context');
  const colors = palette();
  ctx.resetTransform();
  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = cssColor(colors('--bgColor-muted'));
  ctx.fillRect(0, 0, width, height);
  const point = worldPoint(rect.width, rect.height, 0, 0, camera);
  ctx.setTransform(
    devicePixelRatio * point.fit * camera.zoom,
    0,
    0,
    devicePixelRatio * point.fit * camera.zoom,
    point.x * devicePixelRatio,
    point.y * devicePixelRatio,
  );
  for (const cell of scene.cells) {
    const path = new Path2D();
    HEX_POINTS.forEach(([x, y], i) => {
      if (i === 0) path.moveTo(cell.x + x, cell.y + y);
      else path.lineTo(cell.x + x, cell.y + y);
    });
    path.closePath();
    ctx.fillStyle = cssColor(colors(`--data-${cell.hue}-color-muted`));
    ctx.fill(path);
    ctx.save();
    ctx.clip(path);
    ctx.strokeStyle = cssColor(colors(`--data-${cell.hue}-color-emphasis`));
    ctx.lineWidth = 2;
    ctx.stroke(path);
    ctx.restore();
  }
  for (const mark of scene.marks) {
    const source = coloredAtlas(cssColor(colors(`--data-${mark.hue}-color-emphasis`)));
    ctx.drawImage(
      source,
      markNames.indexOf(mark.mark) * GLYPH_SIZE,
      0,
      GLYPH_SIZE,
      GLYPH_SIZE,
      mark.x,
      mark.y,
      mark.size,
      mark.size,
    );
  }
}
