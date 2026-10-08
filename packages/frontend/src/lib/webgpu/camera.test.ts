import { expect, test } from 'bun:test';
import {
  sceneTransform,
  zoomCamera,
  hitTestCell,
  type Camera,
  type ViewerScene,
} from './viewer-scene';

const scene: ViewerScene = {
  cells: [
    { x: 0, y: 0, terrain: 'plain' },
    { x: 400, y: 250, terrain: 'road' },
  ],
  marks: [],
};
const center = (camera: Camera) => {
  const transform = sceneTransform(scene, 900, 600, camera);
  return { x: (450 - transform.x) / transform.scale, y: (300 - transform.y) / transform.scale };
};

test('zoom keeps the visible world center fixed after panning, including limits', () => {
  const camera = { x: 120, y: -80, zoom: 1.5 };
  for (const factor of [1.1, 1 / 1.1, 100, 0.001]) {
    const zoomed = zoomCamera(camera, factor);
    expect(center(zoomed).x).toBeCloseTo(center(camera).x, 10);
    expect(center(zoomed).y).toBeCloseTo(center(camera).y, 10);
    expect(zoomed.zoom).toBeGreaterThanOrEqual(0.5);
    expect(zoomed.zoom).toBeLessThanOrEqual(4);
  }
  const limited = { x: 10, y: 20, zoom: 4 };
  expect(zoomCamera(limited, 1.1)).toBe(limited);
});

test('hex hit testing follows pan and zoom, rejecting gaps and points outside the board', () => {
  for (const camera of [
    { x: 0, y: 0, zoom: 1 },
    { x: -100, y: 55, zoom: 2.2 },
  ]) {
    const transform = sceneTransform(scene, 900, 600, camera);
    const hit = (x: number, y: number) =>
      hitTestCell(
        scene,
        900,
        600,
        camera,
        x * transform.scale + transform.x,
        y * transform.scale + transform.y,
      );
    expect(hit(0, 0)).toBe(0);
    expect(hit(400, 250)).toBe(1);
    expect(hit(20, 20)).toBeNull();
    expect(hit(200, 100)).toBeNull();
    expect(hit(-2000, -2000)).toBeNull();
  }
});
