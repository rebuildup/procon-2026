import { expect, test } from 'bun:test';
import { tooltipPosition } from './tooltip-position';

test('tooltip stays next to the hex and flips sides at the viewport edge', () => {
  const right = tooltipPosition(900, 600, 300, 300, 20);
  expect(right.x).toBe(332);
  const left = tooltipPosition(900, 600, 800, 300, 20);
  expect(left.x + left.width).toBe(768);
  const below = tooltipPosition(420, 800, 200, 400, 20);
  expect(below.y).toBe(432);
  const above = tooltipPosition(420, 800, 200, 650, 20);
  expect(above.y + above.height).toBe(618);
});

test('tooltip stays within the viewport with stable dimensions across anchors', () => {
  for (const [width, height] of [
    [1440, 1000],
    [420, 800],
    [320, 256],
  ]) {
    const dimensions = tooltipPosition(width!, height!, 100, 100, 24);
    for (const [x, y] of [
      [12, 12],
      [width! - 12, 12],
      [12, height! - 12],
      [width! - 12, height! - 12],
    ]) {
      const position = tooltipPosition(width!, height!, x!, y!, 24);
      expect(position.x).toBeGreaterThanOrEqual(8);
      expect(position.y).toBeGreaterThanOrEqual(8);
      expect(position.x + position.width).toBeLessThanOrEqual(width! - 8);
      expect(position.y + position.height).toBeLessThanOrEqual(height! - 8);
      expect(position.width).toBe(dimensions.width);
      expect(position.height).toBe(dimensions.height);
    }
  }
});
