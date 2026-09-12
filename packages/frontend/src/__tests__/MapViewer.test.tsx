import { afterEach, describe, expect, test } from 'bun:test';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { jsx as hyperscriptJsx } from 'solid-js/h/jsx-runtime';

import { MapViewer } from '@/components/MapViewer';
import type { Agent, HexMap, Spot } from '@/types/map';

function buildMap(width: number, height: number): HexMap {
  const cells = [];
  for (let row = 0; row < height; row++) {
    for (let col = 0; col < width; col++) {
      let terrain: HexMap['cells'][number]['terrain'];
      const flat = row * width + col;
      if (row === 0 || row === height - 1 || col === 0 || col === width - 1) {
        terrain = 'mountain';
      } else if (flat % 5 === 0) {
        terrain = 'water';
      } else if (flat % 3 === 0) {
        terrain = 'road';
      } else {
        terrain = 'plain';
      }
      cells.push({ coord: { col, row }, terrain });
    }
  }
  return { width, height, cells };
}

function buildSpots(): Spot[] {
  return [
    { id: 's0', coord: { col: 1, row: 1 }, series: 0 },
    { id: 's1', coord: { col: 3, row: 2 }, series: 1 },
    { id: 's2', coord: { col: 2, row: 4 }, series: 2 },
  ];
}

function buildAgents(): Agent[] {
  return [
    { id: 'a-patrol-0', kind: 'patrol', coord: { col: 0, row: 0 } },
    { id: 'a-supply-0', kind: 'supply', coord: { col: 4, row: 4 } },
  ];
}

describe('MapViewer', () => {
  let root: HTMLDivElement;
  afterEach(() => {
    root?.remove();
  });

  function mount(args: { map: HexMap; agents?: Agent[]; spots?: Spot[] }): HTMLDivElement {
    root = document.createElement('div');
    document.body.appendChild(root);
    render(
      () =>
        hyperscriptJsx(MapViewer, {
          map: args.map,
          agents: args.agents ?? [],
          spots: args.spots ?? [],
        }) as unknown as JSX.Element,
      root,
    );
    return root;
  }

  test('renders the SVG element with the right test id', () => {
    const map = buildMap(4, 4);
    const el = mount({ map });
    const svg = el.querySelector('[data-testid="map-viewer-svg"]');
    expect(svg).not.toBeNull();
    expect(svg?.tagName.toLowerCase()).toBe('svg');
  });

  test('renders one polygon per hex cell', () => {
    const map = buildMap(5, 4);
    const el = mount({ map });
    const cells = el.querySelectorAll('[data-testid="map-viewer-cell"]');
    expect(cells.length).toBe(map.width * map.height);
  });

  test('renders spot markers as circles with the right count', () => {
    const map = buildMap(5, 5);
    const spots = buildSpots();
    const el = mount({ map, spots });
    const rendered = el.querySelectorAll('[data-testid="map-viewer-spot"]');
    expect(rendered.length).toBe(spots.length);
    // Each spot group wraps a circle, even when SVG <g> is virtual.
    const circles = el.querySelectorAll('[data-testid="map-viewer-spot"] circle');
    expect(circles.length).toBe(spots.length);
  });

  test('renders agent markers preserving kind and id', () => {
    const map = buildMap(5, 5);
    const agents = buildAgents();
    const el = mount({ map, agents });
    const rendered = el.querySelectorAll('[data-testid="map-viewer-agent"]');
    expect(rendered.length).toBe(agents.length);
    for (const a of agents) {
      const node = el.querySelector(`[data-agent-id="${a.id}"]`);
      expect(node).not.toBeNull();
      expect(node?.getAttribute('data-agent-kind')).toBe(a.kind);
    }
  });

  test('applies terrain-specific fill colors', () => {
    const map = buildMap(4, 4);
    const el = mount({ map });
    const waterNodes = el.querySelectorAll('[data-terrain="water"]');
    expect(waterNodes.length).toBeGreaterThan(0);
    for (const node of waterNodes) {
      expect(node.getAttribute('fill')).not.toBeNull();
    }
    const allCells = el.querySelectorAll('[data-testid="map-viewer-cell"]');
    expect(allCells.length).toBe(map.width * map.height);
  });
});
