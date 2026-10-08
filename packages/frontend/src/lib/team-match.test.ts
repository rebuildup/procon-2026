import { afterEach, expect, mock, test } from 'bun:test';
import { fetchMatch, matchScene, type Setting } from './team-match';
const originalFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = originalFetch;
});
const setting: Setting = {
  startsAt: 0,
  daySeconds: [60],
  daySteps: [10],
  map: { width: 8, height: 8, cells: Array.from({ length: 8 }, () => Array<number>(8).fill(0)) },
  agents: [0],
  spots: [{ pos: 9, brand: 2, stocks: 4 }],
};
test('reads the team wire format, seconds and row-major positions', async () => {
  globalThis.fetch = mock(
    async (url) =>
      new Response(
        JSON.stringify(
          String(url).endsWith('/setting')
            ? setting
            : {
                endsAt: Date.now() / 1000 + 60,
                day: 0,
                agents: [{ kind: 0, pos: 8, fuel: 100 }],
                others: [{ agents: [{ kind: 1, pos: 63, fuel: 50 }] }],
                traffics: [],
              },
        ),
      ),
  ) as unknown as typeof fetch;
  const match = await fetchMatch();
  expect(match?.status).toBe('running');
  const scene = matchScene(match!);
  expect(scene.cells).toHaveLength(64);
  expect(scene.cells[0]!.x).toBe((26 * 23) / 30);
  expect(scene.cells[8]!.x).toBe(0);
  expect(scene.marks).toEqual([
    { cell: 9, mark: 's2', progress: { value: null, max: 4 } },
    { cell: 63, mark: 'supply', progress: undefined },
    { cell: 8, mark: 'patrol', progress: undefined },
  ]);
});
test('rejects malformed maps rather than drawing a sample map', async () => {
  globalThis.fetch = mock(
    async () =>
      new Response(JSON.stringify({ ...setting, map: { ...setting.map, cells: [[99]] } })),
  ) as unknown as typeof fetch;
  expect(await fetchMatch()).toBeNull();
});
test('returns no update on authentication failure', async () => {
  globalThis.fetch = mock(
    async () => new Response(null, { status: 401 }),
  ) as unknown as typeof fetch;
  expect(await fetchMatch()).toBeNull();
});

test('shared cells keep the focused agent instead of failing renderer validation', () => {
  const scene = matchScene({
    setting,
    state: {
      endsAt: 1,
      day: 0,
      agents: [{ kind: 0, pos: 9, fuel: 3 }],
      others: [{ agents: [{ kind: 1, pos: 9, fuel: 2 }] }],
      traffics: [],
    },
    day: 0,
    daysTotal: 1,
    status: 'completed',
    server: '',
    lastUpdateUnix: 0,
    error: null,
  });
  expect(scene.marks).toEqual([{ cell: 9, mark: 'patrol' }]);
});

test('rejects missing or invalid traffic data before cell inspection', async () => {
  for (const traffics of [null, [{ pos: 64, status: 0 }], [{ pos: 0, status: 3 }]]) {
    globalThis.fetch = mock(
      async (url: string | URL | Request) =>
        new Response(
          JSON.stringify(
            String(url).endsWith('/setting')
              ? setting
              : {
                  day: 0,
                  endsAt: Date.now() / 1000 + 60,
                  agents: [],
                  others: [],
                  traffics,
                },
          ),
        ),
    ) as unknown as typeof fetch;
    expect(await fetchMatch()).toBeNull();
  }
});

test('preserves traffic and fuel ratios without treating maximum stock as current stock', () => {
  const scene = matchScene({
    setting: { ...setting, fuelLimits: 20 },
    day: 0,
    daysTotal: 1,
    server: '',
    lastUpdateUnix: 0,
    error: null,
    status: 'running',
    state: {
      endsAt: 100,
      day: 0,
      agents: [{ kind: 0, pos: 8, fuel: 7 }],
      others: [],
      traffics: [{ pos: 1, status: 2 }],
    },
  });
  expect(scene.cells[1]!.traffic).toBe(2);
  expect(scene.marks.find((mark) => mark.cell === 8)?.progress).toEqual({ value: 7, max: 20 });
  expect(scene.marks.find((mark) => mark.cell === 9)?.progress).toEqual({ value: null, max: 4 });
});
