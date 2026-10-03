import marks from '@/viewer-marks.json';

export type Family = 'ridge' | 'peak' | 'terrace';
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
export function fixtureScene(turn: number, family: Family = 'ridge'): ViewerScene {
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
        mark: (family === 'ridge' ? cell.mark : `${family}-${cell.mark}`) as Mark,
      })),
      ...hues.map((hue, n) => ({
        x: 40 + n * 40,
        y: 27 + (n % 2) * 70,
        size: 18,
        hue,
        mark: (family === 'ridge' ? `s${n}` : `${family}-s${n}`) as Mark,
      })),
      {
        x: 40 + (turn % 6) * 40,
        y: 132,
        size: 20,
        hue: 'pine',
        mark: (family === 'ridge' ? 'patrol' : `${family}-patrol`) as Mark,
      },
      {
        x: 220,
        y: 100,
        size: 20,
        hue: 'pink',
        mark: (family === 'ridge' ? 'supply' : `${family}-supply`) as Mark,
      },
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
  const paint = (node: GlyphNode, inherited: Record<string, string>, halo = false) => {
    const attrs = { ...inherited, ...node.attrs };
    if (node.tag === 'g') {
      for (const child of node.children) paint(child, attrs, halo);
      return;
    }
    const n = (name: string) => Number(attrs[name] ?? 0);
    const path = new Path2D();
    if (node.tag === 'path') path.addPath(new Path2D(attrs.d));
    else if (node.tag === 'rect') path.roundRect(n('x'), n('y'), n('width'), n('height'), n('rx'));
    else if (node.tag === 'circle') path.arc(n('cx'), n('cy'), n('r'), 0, Math.PI * 2);
    else throw new Error(`Unsupported glyph shape: ${node.tag}`);
    if (halo) {
      ctx.strokeStyle = '#ffffff';
      ctx.lineWidth = Number(attrs['stroke-width'] ?? 1.1) + 2.3;
      ctx.lineJoin = 'round';
      ctx.lineCap = 'round';
      ctx.stroke(path);
      return;
    }
    ctx.globalAlpha = Number(attrs.opacity ?? 1);
    if ((attrs.fill ?? 'black') !== 'none') {
      ctx.fillStyle = attrs.fill ?? 'black';
      ctx.fill(path);
    }
    if (attrs.stroke && attrs.stroke !== 'none') {
      ctx.strokeStyle = attrs.stroke;
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
    for (const node of marks[name]) paint(node, {}, true);
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
/** Canonical glyphs already carry the proposal colors and keylines. */
export function coloredAtlas(_color: string): OffscreenCanvas {
  return glyphAtlas();
}
export const terrainColors = {
  plain: '#8ed973',
  mountain: '#bfbfbf',
  water: '#46b1e1',
  road: '#156082',
} as const;
export function terrainRgba(mark: Mark): number[] {
  const hex = terrainColors[mark as keyof typeof terrainColors];
  return [1, 3, 5].map((offset) => parseInt(hex.slice(offset, offset + 2), 16) / 255).concat(1);
}
function terrainPattern(mark: Mark): CanvasPattern {
  const tile = new OffscreenCanvas(10, 10);
  const ctx = tile.getContext('2d')!;
  ctx.fillStyle = terrainColors[mark as keyof typeof terrainColors];
  ctx.fillRect(0, 0, 10, 10);
  if (mark === 'plain') {
    ctx.fillStyle = '#6fb857';
    for (const x of [2.5, 7.5])
      for (const y of [2.5, 7.5]) {
        ctx.beginPath();
        ctx.arc(x, y, 1, 0, Math.PI * 2);
        ctx.fill();
      }
  } else {
    ctx.strokeStyle = mark === 'mountain' ? '#8f8f8f' : '#9bc2e6';
    ctx.lineWidth = mark === 'mountain' ? 1.4 : 1.1;
    ctx.beginPath();
    if (mark === 'mountain') {
      for (const x of [-10, -4, 2, 8, 14]) {
        ctx.moveTo(x, 0);
        ctx.lineTo(x + 10, 10);
      }
    } else if (mark === 'road') {
      ctx.moveTo(1, 4);
      ctx.lineTo(4, 4);
    } else {
      for (const y of [3.2, 8.2]) {
        ctx.moveTo(0, y);
        ctx.bezierCurveTo(1.2, y - 1.6, 2.5, y - 1.6, 3.7, y);
        ctx.bezierCurveTo(4.9, y + 1.6, 6.2, y + 1.6, 7.4, y);
      }
    }
    ctx.stroke();
  }
  return ctx.createPattern(tile, 'repeat')!;
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
    ctx.strokeStyle = '#ffffff';
    ctx.lineJoin = 'round';
    ctx.lineWidth = 3.5;
    ctx.stroke(path);
    ctx.fillStyle = terrainPattern(cell.mark);
    ctx.fill(path);
    ctx.save();
    ctx.strokeStyle = '#212428';
    ctx.lineWidth = 1.2;
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
