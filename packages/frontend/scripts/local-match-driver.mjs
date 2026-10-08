/* global fetch */
import process from 'node:process';
import { setTimeout as delay } from 'node:timers/promises';
import { writeFile } from 'node:fs/promises';
const [url, mode, output] = process.argv.slice(2);
const token = process.env.PROCON_TOKEN;
if (!token || !url || !output) throw new Error('Expected URL, move|wait, OUTPUT and PROCON_TOKEN');
const request = async (path = '', value) => {
  const response = await fetch(url + path, {
    headers: { 'Procon-Token': token, ...(value ? { 'Content-Type': 'application/json' } : {}) },
    ...(value ? { method: 'POST', body: JSON.stringify(value) } : {}),
  });
  return { ok: response.ok, body: await response.json() };
};
const settingResponse = await request('/setting');
if (!settingResponse.ok) throw new Error('Setting request failed');
const setting = settingResponse.body;
const kinds = setting.agents.map((_, index) => (mode === 'move' && index === 1 ? 1 : 0));
if (!(await request('/agent', kinds)).ok) throw new Error('Agent registration failed');
const records = [];
for (let day = 0; day < setting.daySteps.length; day++) {
  let state;
  for (;;) {
    const response = await request();
    if (response.ok && response.body.day === day) {
      state = response.body;
      break;
    }
    await delay(100);
  }
  const steps = setting.daySteps[day];
  const actions = state.agents.map((agent) => {
    if (mode === 'wait') return [-steps];
    const direction = day % 2 === 0 ? 2 : 5;
    const destination = agent.pos + (direction === 2 ? 1 : -1);
    const width = setting.map.width;
    if (
      Math.floor(destination / width) !== Math.floor(agent.pos / width) ||
      setting.map.cells[Math.floor(destination / width)][destination % width] === 3
    )
      throw new Error('Test move is not traversable');
    const terrain = setting.map.cells[Math.floor(agent.pos / width)][agent.pos % width];
    const traffic = state.traffics.find((road) => road.pos === agent.pos)?.status ?? 0;
    const time = terrain === 0 ? 2 : terrain === 2 ? 3 : [1, 2, 4][traffic];
    return [direction, -(steps - time)];
  });
  const result = await request('', actions);
  if (!result.ok || !Number.isInteger(result.body.revision) || result.body.revision < 0)
    throw new Error(`Plan rejected on day ${day}: ${JSON.stringify(result.body)}`);
  records.push({ day, actions, result: result.body });
  await writeFile(output, JSON.stringify({ mode, records }, null, 2) + '\n');
}
await delay(
  Math.max(
    0,
    setting.startsAt * 1000 + setting.daySeconds.reduce((a, b) => a + b, 0) * 1000 - Date.now(),
  ) + 500,
);
