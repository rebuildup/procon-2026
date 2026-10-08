import type { ViewerScene, ViewerMark } from './webgpu/viewer-scene';

export interface Setting {
  startsAt: number;
  daySeconds: number[];
  daySteps: number[];
  map: { width: number; height: number; cells: number[][] };
  spots: { pos: number; brand: number; stocks: number }[];
  agents: number[];
  fuelLimits?: number;
}
export interface AgentState {
  kind: number;
  pos: number;
  fuel: number;
}
export interface DayState {
  endsAt: number;
  day: number;
  agents: AgentState[];
  others: { id?: number; agents: AgentState[] }[];
  traffics: { pos: number; status: number }[];
}
export interface Match {
  setting: Setting;
  state: DayState;
  day: number;
  daysTotal: number;
  status: 'waiting' | 'running' | 'completed';
  server: string;
  lastUpdateUnix: number;
  error: null;
}

function validSetting(value: Setting): boolean {
  const { map } = value;
  return (
    !!map &&
    Number.isInteger(map.width) &&
    Number.isInteger(map.height) &&
    map.width >= 8 &&
    map.width <= 32 &&
    map.height >= 8 &&
    map.height <= 32 &&
    Array.isArray(map.cells) &&
    map.cells.length === map.height &&
    map.cells.every(
      (row) =>
        Array.isArray(row) &&
        row.length === map.width &&
        row.every((cell) => Number.isInteger(cell) && cell >= 0 && cell <= 3),
    ) &&
    Array.isArray(value.spots) &&
    value.spots.every(
      (spot) =>
        Number.isInteger(spot.brand) &&
        spot.brand >= 0 &&
        spot.brand <= 255 &&
        Number.isInteger(spot.stocks) &&
        spot.stocks >= 0 &&
        validPos(spot.pos, value),
    ) &&
    (value.fuelLimits === undefined ||
      (Number.isInteger(value.fuelLimits) && value.fuelLimits > 0)) &&
    Array.isArray(value.daySteps) &&
    Array.isArray(value.daySeconds) &&
    Number.isFinite(value.startsAt)
  );
}
function validPos(pos: number, setting: Setting): boolean {
  return Number.isInteger(pos) && pos >= 0 && pos < setting.map.width * setting.map.height;
}
function validAgents(agents: AgentState[], setting: Setting): boolean {
  return (
    Array.isArray(agents) &&
    agents.every(
      (agent) =>
        (agent.kind === 0 || agent.kind === 1) &&
        validPos(agent.pos, setting) &&
        Number.isFinite(agent.fuel),
    )
  );
}

export async function fetchMatch(signal?: AbortSignal): Promise<Match | null> {
  try {
    const options = { cache: 'no-store' as const, ...(signal ? { signal } : {}) };
    const settingResponse = await fetch('/api/match/setting', options);
    if (!settingResponse.ok) return null;
    const setting = (await settingResponse.json()) as Setting;
    if (!validSetting(setting)) return null;
    const stateResponse = await fetch('/api/match', options);
    if (!stateResponse.ok) return null;
    const state = (await stateResponse.json()) as DayState;
    if (
      !Number.isInteger(state.day) ||
      state.day < 0 ||
      state.day >= setting.daySteps.length ||
      !Number.isFinite(state.endsAt) ||
      !validAgents(state.agents, setting) ||
      !Array.isArray(state.others) ||
      !state.others.every((team) => validAgents(team.agents, setting)) ||
      !Array.isArray(state.traffics) ||
      !state.traffics.every(
        (road) =>
          validPos(road.pos, setting) &&
          Number.isInteger(road.status) &&
          road.status >= 0 &&
          road.status <= 2,
      )
    )
      return null;
    const now = Date.now() / 1000;
    return {
      setting,
      state,
      day: state.day,
      daysTotal: setting.daySteps.length,
      status:
        now < setting.startsAt
          ? 'waiting'
          : state.day === setting.daySteps.length - 1 && now >= state.endsAt
            ? 'completed'
            : 'running',
      server: '/api/match',
      lastUpdateUnix: Math.floor(now),
      error: null,
    };
  } catch {
    return null;
  }
}

export function matchScene({ setting, state }: Match): ViewerScene {
  const scale = 23 / 30;
  const traffic = new Map(state.traffics.map((item) => [item.pos, item.status]));
  const cells = setting.map.cells.flatMap((row, y) =>
    row.map((terrain, x) => ({
      x: (x * 52 + (y % 2 === 0 ? 26 : 0)) * scale,
      y: y * 45 * scale,
      terrain: (['plain', 'road', 'mountain', 'water'] as const)[terrain]!,
      traffic: traffic.get(y * setting.map.width + x),
    })),
  );
  const marks: ViewerMark[] = setting.spots.map((spot) => ({
    cell: spot.pos,
    mark: `s${spot.brand % 6}` as ViewerMark['mark'],
    progress: { value: null, max: spot.stocks },
  }));
  for (const team of [...state.others.map((team) => team.agents), state.agents]) {
    for (const agent of team)
      marks.push({
        cell: agent.pos,
        mark: agent.kind === 0 ? 'patrol' : 'supply',
        progress:
          agent.kind === 0 && setting.fuelLimits
            ? { value: agent.fuel, max: setting.fuelLimits }
            : undefined,
      });
  }
  // The atlas accepts one occupant per cell; focused-team agents take priority.
  return { cells, marks: [...new Map(marks.map((mark) => [mark.cell, mark])).values()] };
}
