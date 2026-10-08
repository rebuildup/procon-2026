/* global window, document, console */
import process from 'node:process';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { chromium } from '@playwright/test';
import { createServer } from 'vite';

const args = process.argv.slice(2);
const input = args[0];
const option = (name, fallback) => {
  const index = args.indexOf(name);
  return index < 0 ? fallback : args[index + 1];
};
if (!input || input.startsWith('--')) {
  throw new Error(
    'Usage: bun run record:match TRACE.json [--team 0] [--fps 10] [--out ../../target/match-video]',
  );
}
const teamIndex = Number(option('--team', '0'));
const fps = Number(option('--fps', '10'));
if (!Number.isInteger(teamIndex) || teamIndex < 0 || !Number.isFinite(fps) || fps < 1 || fps > 30) {
  throw new Error('team must be a non-negative integer; fps must be between 1 and 30');
}
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const output = resolve(option('--out', resolve(root, '../../target/match-video')));
const raw = await readFile(resolve(input));
const trace = JSON.parse(raw);
const setting = trace.problem;
const teams = trace.teams;
if (
  !setting?.map ||
  !Array.isArray(teams) ||
  !teams[teamIndex] ||
  !Array.isArray(setting.daySteps)
) {
  throw new Error(
    'Expected arena trace JSON with problem, teams[].kinds and teams[].days[].frames',
  );
}
const { width, height, cells } = setting.map;
if (
  !Number.isInteger(width) ||
  !Number.isInteger(height) ||
  width < 8 ||
  width > 32 ||
  height < 8 ||
  height > 32 ||
  !Array.isArray(cells) ||
  cells.length !== height ||
  cells.some(
    (row) =>
      !Array.isArray(row) ||
      row.length !== width ||
      row.some((cell) => ![0, 1, 2, 3].includes(cell)),
  )
) {
  throw new Error('Invalid trace map');
}
const frames = [];
for (let day = 0; day < setting.daySteps.length; day++) {
  const recorded = teams[teamIndex].days?.[day]?.frames;
  if (!Array.isArray(recorded) || recorded.length === 0)
    throw new Error(`Missing recorded frames for day ${day}`);
  for (let step = 0; step < recorded.length; step++) {
    const agentsFor = (team) => {
      const agents = team.days?.[day]?.frames?.[step]?.agents;
      if (!Array.isArray(agents) || agents.length !== team.kinds?.length)
        throw new Error(`Incomplete agents at day ${day}, step ${step}`);
      return agents.map((agent, index) => {
        const kind = team.kinds[index];
        if (
          ![0, 1].includes(kind) ||
          !Number.isInteger(agent.pos) ||
          agent.pos < 0 ||
          agent.pos >= width * height ||
          !Number.isFinite(agent.fuel)
        ) {
          throw new Error(`Invalid agent at day ${day}, step ${step}`);
        }
        return { kind, pos: agent.pos, fuel: agent.fuel };
      });
    };
    frames.push({
      day,
      step,
      state: {
        day,
        endsAt: setting.startsAt + setting.daySeconds.slice(0, day + 1).reduce((a, b) => a + b, 0),
        agents: agentsFor(teams[teamIndex]),
        others: teams
          .filter((_, index) => index !== teamIndex)
          .map((team) => ({ agents: agentsFor(team) })),
        traffics: [],
      },
    });
  }
}
if (frames.length < 2) throw new Error('At least two recorded frames are needed');
await mkdir(output, { recursive: true });
const server = await createServer({ root, server: { host: '127.0.0.1', port: 0 } });
let browser;
try {
  await server.listen();
  const address = server.httpServer.address();
  browser = await chromium.launch({
    args: [
      '--enable-unsafe-webgpu',
      '--use-angle=swiftshader',
      '--use-vulkan=swiftshader',
      '--enable-features=Vulkan',
    ],
  });
  const context = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
    recordVideo: { dir: output, size: { width: 1440, height: 1000 } },
  });
  const page = await context.newPage();
  await page.addInitScript(() => {
    const interval = window.setInterval.bind(window);
    window.setInterval = (handler, timeout, ...values) =>
      interval(handler, timeout === 1000 ? 10 : timeout, ...values);
  });
  let cursor = 0;
  let received = -1;
  await page.route('**/api/match/setting', (route) => route.fulfill({ json: setting }));
  await page.route('**/api/match', async (route) => {
    const index = cursor;
    await route.fulfill({ json: frames[index].state });
    received = index;
  });
  await page.goto(`http://127.0.0.1:${address.port}/live`);
  await page.getByTestId('map-viewer').waitFor();
  await page.waitForFunction(() =>
    ['WebGPU', 'Canvas'].includes(
      document.querySelector('[data-testid="map-viewer"]')?.getAttribute('data-renderer'),
    ),
  );
  await page.evaluate(() => {
    for (const heading of document.querySelectorAll('h2')) {
      if (heading.textContent === 'Source') {
        heading.parentElement.innerHTML =
          '<h2 class="f5 text-bold mb-3">Source</h2><p class="f6 color-fg-muted mb-0">保存済みtrace JSONの記録フレームを順に表示しています。移動の補間は行っていません。</p>';
      }
    }
    document
      .querySelector('.PageLayout-pane')
      .insertAdjacentHTML(
        'afterbegin',
        '<section class="border rounded-2 p-3 mb-3"><h2 class="f5 text-bold">Recorded match</h2><p id="recording-frame" class="text-mono mb-0"></p></section>',
      );
  });
  for (let index = 0; index < frames.length; index++) {
    const frameStartedAt = Date.now();
    cursor = index;
    const deadline = Date.now() + 5000;
    while (received !== index) {
      if (Date.now() > deadline) throw new Error(`Frame ${index} did not reach the viewer`);
      await page.waitForTimeout(10);
    }
    const { day, step } = frames[index];
    await page.evaluate(
      ({ day, step, count }) => {
        document.querySelector('#recording-frame').textContent =
          `Day ${day + 1} / ${count} · Step ${step}`;
      },
      { day, step, count: setting.daySteps.length },
    );
    await page.waitForTimeout(Math.max(0, 1000 / fps - (Date.now() - frameStartedAt)));
  }
  await page.screenshot({ path: resolve(output, 'match.png') });
  const video = page.video();
  await context.close();
  await video.saveAs(resolve(output, 'match.webm'));
  await video.delete();
  await writeFile(
    resolve(output, 'source.json'),
    JSON.stringify(
      {
        sourceSha256: createHash('sha256').update(raw).digest('hex'),
        teams: teams.length,
        focusTeamIndex: teamIndex,
        days: setting.daySteps.length,
        frames: frames.length,
        fps,
        interpolation: false,
      },
      null,
      2,
    ) + '\n',
  );
  console.log(`Recorded ${frames.length} source frames to ${output}/match.webm`);
} finally {
  await browser?.close();
  await server.close();
}
