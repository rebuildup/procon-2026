import samples from '@/viewer-samples.json';
import {
  tileAtlas,
  tileIndex,
  tileSource,
  TILE_SIZE,
  TILE_WORLD_SIZE,
  HALO_TILE,
  INK_TILE,
  type Terrain,
} from './hex-assets';
export { markNames, GLYPH_SIZE, glyphAtlas, loadHexAssets } from './hex-assets';
import type { Mark } from './hex-assets';
export type { Mark } from './hex-assets';

export type Family = 'ridge' | 'peak' | 'terrace';

export interface Camera {
  x: number;
  y: number;
  zoom: number;
}
export interface ViewerCell {
  readonly x: number;
  readonly y: number;
  readonly terrain: Terrain;
}
export interface ViewerMark {
  readonly cell: number;
  readonly mark: Mark;
}
export interface ViewerScene {
  readonly cells: readonly ViewerCell[];
  readonly marks: readonly ViewerMark[];
}
export type Sample = keyof typeof samples;
export { samples };
const fixtures = new Map<string, ViewerScene>();
export function fixtureScene(
  turn: number,
  family: Family = 'ridge',
  sample: Sample = 'figure1',
  padding = 0,
  animate = false,
): ViewerScene {
  const key = `${family}:${sample}:${padding}:${animate ? turn % 6 : 'static'}`;
  const cached = fixtures.get(key);
  if (cached) return cached;
  const data = samples[sample],
    cells: ViewerCell[] = [],
    marks: ViewerMark[] = [];
  const index = new Map<number, number>();
  const scale = 23 / 30;
  for (let row = -padding; row < data.height + padding; row++)
    for (let col = -padding; col < data.width + padding; col++) {
      const inside = row >= 0 && row < data.height && col >= 0 && col < data.width;
      const cell = cells.length;
      if (inside) index.set(row * data.width + col, cell);
      cells.push({
        x: (col * 52 + (row % 2 === 0 ? 26 : 0)) * scale,
        y: row * 45 * scale,
        terrain: inside
          ? (['plain', 'road', 'mountain', 'water'] as const)[data.cells[row]![col]!]!
          : 'empty',
      });
    }
  for (const spot of data.spots)
    marks.push({
      cell: index.get(spot.cell)!,
      mark: (family === 'ridge' ? `s${spot.series}` : `${family}-s${spot.series}`) as Mark,
    });
  const candidates = [...index.entries()]
    .filter(
      ([original]) =>
        data.cells[Math.floor(original / data.width)]![original % data.width] === 0 &&
        !data.spots.some((spot) => spot.cell === original),
    )
    .map(([, cell]) => cell);
  if (animate)
    marks.push({
      cell: candidates[turn % candidates.length]!,
      mark: (family === 'ridge' ? 'patrol' : `${family}-patrol`) as Mark,
    });
  const result = { cells, marks };
  fixtures.set(key, result);
  return result;
}
export function sceneTransform(scene: ViewerScene, width: number, height: number, camera: Camera) {
  const half = TILE_WORLD_SIZE / 2;
  let minX = Infinity,
    minY = Infinity,
    maxX = -Infinity,
    maxY = -Infinity;
  for (const cell of scene.cells) {
    minX = Math.min(minX, cell.x - half);
    minY = Math.min(minY, cell.y - half);
    maxX = Math.max(maxX, cell.x + half);
    maxY = Math.max(maxY, cell.y + half);
  }
  const fit = Math.min(
    Math.max(1, width - 32) / (maxX - minX),
    Math.max(1, height - 32) / (maxY - minY),
  );
  return {
    fit,
    scale: fit * camera.zoom,
    x: width / 2 - ((minX + maxX) / 2) * fit * camera.zoom + camera.x * fit,
    y: height / 2 - ((minY + maxY) / 2) * fit * camera.zoom + camera.y * fit,
  };
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
export function worldPoint(
  width: number,
  height: number,
  x: number,
  y: number,
  camera: Camera,
  scene: ViewerScene = fixtureScene(0),
) {
  const transform = sceneTransform(scene, width, height, camera);
  return {
    x: x * transform.scale + transform.x,
    y: y * transform.scale + transform.y,
    fit: transform.fit,
  };
}
export interface HexInstance {
  x: number;
  y: number;
  tile: number;
}
/** One occupant per hex, composed before either rendering backend sees it. */
const compositions = new WeakMap<ViewerScene, HexInstance[]>();
export function hexInstances(scene: ViewerScene): HexInstance[] {
  const cached = compositions.get(scene);
  if (cached) return cached;
  const occupants = new Map<number, Mark>();
  for (const occupant of scene.marks) {
    if (!Number.isInteger(occupant.cell) || !scene.cells[occupant.cell])
      throw new Error('Occupant references missing hex');
    if (occupants.has(occupant.cell)) throw new Error('Duplicate hex occupant');
    occupants.set(occupant.cell, occupant.mark);
  }
  const instances = scene.cells.map((cell, index) => {
    return {
      x: cell.x - TILE_WORLD_SIZE / 2,
      y: cell.y - TILE_WORLD_SIZE / 2,
      tile: tileIndex(cell.terrain, occupants.get(index)),
    };
  });
  compositions.set(scene, instances);
  return instances;
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
  const ctx = canvas.getContext('2d')!;
  ctx.resetTransform();
  ctx.fillStyle = cssColor(palette()('--bgColor-muted'));
  ctx.fillRect(0, 0, width, height);
  const point = worldPoint(rect.width, rect.height, 0, 0, camera, scene);
  ctx.setTransform(
    devicePixelRatio * point.fit * camera.zoom,
    0,
    0,
    devicePixelRatio * point.fit * camera.zoom,
    point.x * devicePixelRatio,
    point.y * devicePixelRatio,
  );
  const instances = hexInstances(scene);
  for (const layer of [HALO_TILE, undefined, INK_TILE])
    for (const instance of instances) {
      const source = tileSource(layer ?? instance.tile);
      ctx.drawImage(
        tileAtlas(),
        source.x,
        source.y,
        TILE_SIZE,
        TILE_SIZE,
        instance.x,
        instance.y,
        TILE_WORLD_SIZE,
        TILE_WORLD_SIZE,
      );
    }
}
