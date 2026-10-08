import { readFileSync } from 'node:fs';
const samples = JSON.parse(
  readFileSync(new URL('../src/viewer-samples.json', import.meta.url), 'utf8'),
) as Record<string, { width: number; height: number; cells: number[][] }>;
import type { Setting, DayState } from '../src/lib/team-match';
export const setting: Setting = {
  startsAt: 0,
  daySeconds: [60, 60],
  daySteps: [20, 20],
  map: {
    width: samples.figure1!.width,
    height: samples.figure1!.height,
    cells: samples.figure1!.cells,
  },
  spots: [
    { pos: 22, brand: 0, stocks: 5 },
    { pos: 56, brand: 1, stocks: 5 },
    { pos: 78, brand: 2, stocks: 5 },
  ],
  agents: [0, 1],
};
export function day(pos = 0): DayState {
  return {
    endsAt: Math.floor(Date.now() / 1000) + 60,
    day: 0,
    agents: [
      { kind: 0, pos, fuel: 100 },
      { kind: 1, pos: 1, fuel: 80 },
    ],
    others: [{ agents: [{ kind: 0, pos: 8, fuel: 90 }] }],
    traffics: [],
  };
}
