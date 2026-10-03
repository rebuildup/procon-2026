import {
  tileAtlas,
  tileIndex,
  tileSource,
  TILE_SIZE,
  TILE_WORLD_SIZE,
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
const fixtures = new Map<string, ViewerScene>();
export function fixtureScene(turn: number, family: Family = 'ridge'): ViewerScene {
  const key = `${family}:${turn % 6}`;
  const cached = fixtures.get(key);
  if (cached) return cached;
  const cells: ViewerCell[] = Array.from({ length: 48 }, (_, i) => {
    const mark = i % 11 === 0 ? 'mountain' : i % 9 === 0 ? 'water' : i % 5 === 0 ? 'road' : 'plain';
    return {
      x: (i % 8) * 40 + (Math.floor(i / 8) % 2) * 20,
      y: Math.floor(i / 8) * 35,
      terrain: mark,
    };
  });

  const result: ViewerScene = {
    cells,
    marks: [
      ...Array.from({ length: 6 }, (_, n) => ({
        cell: 9 + n,
        mark: (family === 'ridge' ? `s${n}` : `${family}-s${n}`) as Mark,
      })),
      { cell: 33 + (turn % 6), mark: (family === 'ridge' ? 'patrol' : `${family}-patrol`) as Mark },
      { cell: 30, mark: (family === 'ridge' ? 'supply' : `${family}-supply`) as Mark },
    ],
  };
  fixtures.set(key, result);
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
  const point = worldPoint(rect.width, rect.height, 0, 0, camera);
  ctx.setTransform(
    devicePixelRatio * point.fit * camera.zoom,
    0,
    0,
    devicePixelRatio * point.fit * camera.zoom,
    point.x * devicePixelRatio,
    point.y * devicePixelRatio,
  );
  for (const instance of hexInstances(scene)) {
    const source = tileSource(instance.tile);
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
