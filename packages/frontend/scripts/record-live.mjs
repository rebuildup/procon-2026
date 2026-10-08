/* global console, navigator, fetch, document */
import process from 'node:process';
import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, URL } from 'node:url';
import { setTimeout } from 'node:timers';
import { createHash } from 'node:crypto';
import { createServer } from 'vite';
import { chromium, expect } from '@playwright/test';

const [serverBin, configPath, outputPath] = process.argv.slice(2);
if (!outputPath)
  throw new Error('Usage: bun scripts/record-live.mjs SERVER_BIN CONFIG.json OUTPUT_DIR');
const config = JSON.parse(await readFile(resolve(configPath), 'utf8'));
if (config.teams?.length !== 2 || config.problem?.daySteps?.length !== 4)
  throw new Error('Use an isolated two-team, four-day test config');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const output = resolve(outputPath);
await mkdir(output, { recursive: true });
const port = Number(process.env.RECORD_MATCH_PORT ?? 5491);
const serverUrl = `http://127.0.0.1:${port}`;
const children = [];
const spawnChild = (bin, args, env = process.env) => {
  const child = spawn(resolve(bin), args, { env, stdio: ['ignore', 'pipe', 'pipe'] });
  let log = '';
  child.stdout.on('data', (bytes) => {
    log += bytes;
  });
  child.stderr.on('data', (bytes) => {
    log += bytes;
  });
  const done = new Promise((accept, reject) => {
    child.once('error', reject);
    child.once('exit', (code) =>
      code === 0 ? accept() : reject(new Error(`Child exited ${code}: ${log.slice(-2000)}`)),
    );
  });
  done.catch(() => {});
  children.push(child);
  return { child, done };
};
let vite, browser;
try {
  const official = spawnChild(serverBin, [
    '-config',
    resolve(configPath),
    '-addr',
    `127.0.0.1:${port}`,
    '-kind-deadline',
    '7s',
    '-match-start-delay',
    '2s',
  ]);
  for (let attempt = 0; attempt < 50; attempt++) {
    try {
      await fetch(`${serverUrl}/setting`);
      break;
    } catch {
      await new Promise((accept) => setTimeout(accept, 100));
    }
  }
  const solvers = ['move', 'wait'].map((mode, index) =>
    spawnChild(
      process.execPath,
      [
        resolve(root, 'scripts/local-match-driver.mjs'),
        serverUrl,
        mode,
        resolve(output, `team-${index}-plans.json`),
      ],
      { ...process.env, PROCON_TOKEN: config.teams[index].token },
    ),
  );
  vite = await createServer({
    root,
    logLevel: 'silent',
    server: {
      host: '127.0.0.1',
      port: 0,
      proxy: {
        '/api/match': {
          target: serverUrl,
          changeOrigin: true,
          rewrite: (path) => path.replace(/^\/api\/match/, ''),
          headers: { 'Procon-Token': config.teams[0].token },
        },
      },
    },
  });
  await vite.listen();
  process.env.RECORD_PORT = String(vite.httpServer.address().port);
  process.env.RECORD_OUT = output;
  process.env.RECORD_MS = '1000';
  const { recordScenario } = await import('../e2e/recording.ts');
  browser = await chromium.launch({
    args: [
      '--enable-unsafe-webgpu',
      '--use-angle=swiftshader',
      '--use-vulkan=swiftshader',
      '--enable-features=Vulkan',
    ],
  });
  const wireStates = new Map();
  let setting;
  let stopped;
  const finish = () =>
    (stopped ??= (async () => {
      await Promise.all(solvers.map((solver) => solver.done));
      official.child.kill();
    })());
  await Promise.all(
    ['live-desktop', 'live-canvas-dark'].map((name) =>
      recordScenario(browser, {
        name,
        route: '/live',
        viewport: { width: 1440, height: 1000 },
        setup: async (page) => {
          page.on('pageerror', (error) => {
            throw error;
          });
          if (name === 'live-canvas-dark')
            await page.addInitScript(() =>
              Object.defineProperty(navigator, 'gpu', { value: undefined, configurable: true }),
            );
          page.on('response', async (response) => {
            try {
              const path = new URL(response.url()).pathname;
              if (response.ok() && path === '/api/match/setting') setting = await response.json();
              if (response.ok() && path === '/api/match') {
                const value = await response.json();
                wireStates.set(value.day, value);
                writeFileSync(
                  resolve(output, 'match-data.json'),
                  JSON.stringify({ setting, states: [...wireStates.values()] }, null, 2),
                );
              }
            } catch {
              /* Ignore requests aborted by context teardown. */
            }
          });
        },
        ready: async (page) => {
          await expect(page.getByTestId('live-status-badge')).toHaveText('Running', {
            timeout: 20000,
          });
          await expect(page.getByTestId('map-viewer')).toHaveAttribute(
            'data-renderer',
            name === 'live-canvas-dark' ? 'Canvas' : /WebGPU|Canvas/,
            { timeout: 15000 },
          );
          if (name === 'live-canvas-dark')
            await page.getByRole('button', { name: 'Toggle theme' }).click();
        },
        steps: async (page) => {
          let firstMap;
          for (let day = 0; day < config.problem.daySteps.length; day++) {
            await expect(
              page.getByText(`${day + 1} / ${config.problem.daySteps.length}`, { exact: true }),
            ).toBeVisible({ timeout: 15000 });
            const image = await page.getByTestId('map-viewer').screenshot();
            if (day === 0) firstMap = image;
            if (day === 3) expect(image.equals(firstMap)).toBe(false);
            await page.screenshot({
              path: resolve(output, `${name}-day-${day + 1}.png`),
              fullPage: true,
            });
            if (day === 0) {
              const point = await page.evaluate(async () => {
                const [{ sceneTransform }, { matchScene }] = await Promise.all([
                  import('/src/lib/webgpu/viewer-scene.ts'),
                  import('/src/lib/team-match.ts'),
                ]);
                const setting = await (await fetch('/api/match/setting')).json();
                const state = await (await fetch('/api/match')).json();
                const scene = matchScene({ setting, state });
                const stage = document.querySelector('[data-testid="map-viewer"]');
                const rect = stage.getBoundingClientRect();
                const transform = sceneTransform(
                  scene,
                  rect.width,
                  rect.height,
                  JSON.parse(stage.dataset.camera),
                );
                const cell = scene.cells[state.agents[0].pos];
                return {
                  x: rect.left + cell.x * transform.scale + transform.x,
                  y: rect.top + cell.y * transform.scale + transform.y,
                };
              });
              await page.mouse.move(point.x, point.y);
              await expect(page.getByRole('tooltip')).toBeVisible();
              await page.waitForTimeout(1500);
              await page.mouse.move(0, 0);
              await expect(page.getByRole('tooltip')).toBeHidden();
              await page.getByRole('complementary').evaluate((element) => {
                element.scrollTop = element.scrollHeight;
              });
              await page.waitForTimeout(1500);
              await page.getByRole('complementary').evaluate((element) => {
                element.scrollTop = 0;
              });
            }
            if (day === 1 && name === 'live-canvas-dark') {
              await page.getByTestId('map-viewer').focus();
              await page.keyboard.press('+');
              await page.keyboard.press('ArrowRight');
              await page.screenshot({ path: resolve(output, 'live-camera.png'), fullPage: true });
              await page.getByRole('button', { name: 'Reset view', exact: true }).click();
            }
            if (day === 2 && name === 'live-desktop') {
              await page.setViewportSize({ width: 420, height: 900 });
              await page.screenshot({ path: resolve(output, 'live-mobile.png'), fullPage: true });
              await page.setViewportSize({ width: 1440, height: 1000 });
            }
          }
          await finish();
          await expect(page.getByTestId('live-status-badge')).toHaveText('Stale', {
            timeout: 10000,
          });
          await expect(page.getByTestId('map-viewer')).toBeVisible();
          await page.screenshot({ path: resolve(output, `${name}-stale.png`), fullPage: true });
        },
      }),
    ),
  );
  expect([...wireStates.keys()].sort()).toEqual([0, 1, 2, 3]);
  expect(wireStates.get(0).agents.map((agent) => agent.pos)).not.toEqual(
    wireStates.get(3).agents.map((agent) => agent.pos),
  );
  await writeFile(
    resolve(output, 'match-data.json'),
    JSON.stringify(
      { setting, states: [...wireStates.values()].sort((a, b) => a.day - b.day) },
      null,
      2,
    ) + '\n',
  );
  await writeFile(
    resolve(output, 'source.json'),
    JSON.stringify(
      {
        source: 'Official verification server: isolated local match',
        serverSha256: createHash('sha256')
          .update(await readFile(resolve(serverBin)))
          .digest('hex'),
        driverSha256: createHash('sha256')
          .update(await readFile(resolve(root, 'scripts/local-match-driver.mjs')))
          .digest('hex'),
        drivers: ['one horizontal move per day then wait', 'wait'],
        days: config.problem.daySteps.length,
        daySteps: config.problem.daySteps,
        daySeconds: config.problem.daySeconds,
        driversCompleted: true,
        note: 'GET /setting and GET / responses; no interpolated steps or mock positions. Official example config with daySeconds shortened for recording. Driver plans are communication checks, not a competitive solver.',
      },
      null,
      2,
    ) + '\n',
  );
  console.log(`Recorded live HTTP match to ${output}`);
} finally {
  await browser?.close();
  await vite?.close();
  for (const child of children) if (child.exitCode === null) child.kill();
}
