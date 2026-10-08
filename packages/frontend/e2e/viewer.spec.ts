import { expect, test } from '@playwright/test';
import { setting, day } from './team-fixture';

test('replay transport, scores, keyboard scrub and map navigation', async ({ page }) => {
  await page.goto('/replays');
  const step = page.getByTestId('replay-current-step');
  await expect(step).toHaveText('1 / 5');
  await page.getByLabel('Step forward', { exact: true }).click();
  await expect(step).toHaveText('2 / 5');
  await expect(page.getByRole('row', { name: 'alice 10' })).toBeVisible();
  await page.getByText('4×', { exact: true }).click();
  await expect(page.getByRole('radio').nth(2)).toBeChecked();
  await page.getByLabel('Play', { exact: true }).click();
  await expect(step).toHaveText('5 / 5');
  await expect(page.getByLabel('Play', { exact: true })).toBeVisible();
  await page.getByLabel('First step', { exact: true }).click();
  await page.getByTestId('replay-scrub').focus();
  await page.keyboard.press('ArrowRight');
  await expect(step).toHaveText('2 / 5');
  const map = page.getByTestId('map-viewer');
  await map.focus();
  await page.keyboard.press('+');
  await expect(map).toHaveAttribute('data-camera', /"zoom":1.1/);
  await page.keyboard.press('0');
  await expect(map).toHaveAttribute('data-camera', '{"x":0,"y":0,"zoom":1}');
});

test('routes fit desktop and narrow viewports; theme persists', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  for (const width of [1280, 420]) {
    await page.setViewportSize({ width, height: 800 });
    for (const route of ['/', '/replays', '/solvers', '/live']) {
      await page.goto(route);
      await expect(page.locator('h1')).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
    }
  }
  const before = await page.locator('html').getAttribute('data-color-mode');
  await page.getByLabel('Toggle theme').click();
  const after = await page.locator('html').getAttribute('data-color-mode');
  expect(after).not.toBe(before);
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-color-mode', after!);
  expect(errors).toEqual([]);
});

test('live team feed updates fuel and keeps the last map on failure', async ({ page }) => {
  let offline = false;
  let fuel = 100;
  await page.route('**/api/match/setting', (route) => route.fulfill({ json: setting }));
  await page.route('**/api/match', (route) =>
    offline
      ? route.fulfill({ status: 503 })
      : route.fulfill({ json: { ...day(), agents: [{ kind: 0, pos: 10, fuel }] } }),
  );
  await page.goto('/live');
  await expect(page.getByTestId('live-status-badge')).toHaveText('Running');
  await expect(
    page.getByRole('table', { name: '自チームの車両' }).getByText('100', { exact: true }),
  ).toBeVisible();
  await expect(page.getByTestId('map-viewer')).toBeVisible();
  fuel = 75;
  await expect(
    page.getByRole('table', { name: '自チームの車両' }).getByText('75', { exact: true }),
  ).toBeVisible();
  offline = true;
  await expect(page.getByTestId('live-status-badge')).toHaveText('Stale', { timeout: 10000 });
  await expect(
    page.getByRole('table', { name: '自チームの車両' }).getByText('75', { exact: true }),
  ).toBeVisible();
  await expect(page.getByTestId('map-viewer')).toBeVisible();
});

test('missing live feed leaves the connecting state', async ({ page }) => {
  await page.route(
    (url) => url.pathname === '/api/match/setting',
    (route) => route.fulfill({ status: 404, body: '' }),
  );
  await page.goto('/live');
  await expect(page.getByTestId('live-status-badge')).toHaveText('No data', { timeout: 10000 });
  await expect(page.getByTestId('map-viewer')).toHaveCount(0);
});

test('unsupported GPU automatically uses a complete Canvas scene without SVG DOM', async ({
  page,
}) => {
  await page.addInitScript(() =>
    Object.defineProperty(navigator, 'gpu', { value: undefined, configurable: true }),
  );
  await page.goto('/replays');
  await expect(page.getByTestId('webgpu-map-fallback')).toBeVisible();
  await expect(page.getByTestId('map-viewer')).toHaveAttribute('data-renderer', 'Canvas');
  await expect(page.getByTestId('canvas-map-fallback')).toHaveCSS('visibility', 'visible');
  await expect(
    page.getByRole('region', { name: 'Visualizer' }).locator('svg,path,polygon'),
  ).toHaveCount(0);
  const pixels = await page.getByTestId('canvas-map-fallback').evaluate((el) => {
    const canvas = el as HTMLCanvasElement;
    return Array.from(
      canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data,
    ).filter((v, i) => i % 4 !== 3 && v < 128).length;
  });
  expect(pixels).toBeGreaterThan(100);
});

test('GPU replay changes composed occupants and camera without SVG overlay', async ({ page }) => {
  await page.goto('/replays');
  await expect(page.getByTestId('map-viewer')).toHaveAttribute('data-renderer', 'WebGPU', {
    timeout: 10000,
  });
  await expect(
    page.getByRole('region', { name: 'Visualizer' }).locator('svg,path,polygon,button'),
  ).toHaveCount(0);
  const before = await page.getByTestId('webgpu-map-canvas').screenshot();
  await page.getByLabel('Step forward', { exact: true }).click();
  expect(await page.getByTestId('webgpu-map-canvas').screenshot()).not.toEqual(before);
  await page.getByTestId('map-viewer').focus();
  await page.keyboard.press('+');
  await page.getByLabel('Reset view', { exact: true }).click();
  await expect(page.getByTestId('map-viewer')).toHaveAttribute(
    'data-camera',
    '{"x":0,"y":0,"zoom":1}',
  );
});

test('a blank GPU frame falls back to Canvas instead of recreating SVG DOM', async ({ page }) => {
  await page.addInitScript(() => {
    const original = CanvasRenderingContext2D.prototype.drawImage;
    CanvasRenderingContext2D.prototype.drawImage = function (
      image: CanvasImageSource,
      ...coordinates: number[]
    ) {
      if (image instanceof HTMLCanvasElement && image.dataset.testid === 'webgpu-map-canvas') {
        this.fillStyle = getComputedStyle(document.documentElement)
          .getPropertyValue('--bgColor-muted')
          .trim();
        this.fillRect(0, 0, this.canvas.width, this.canvas.height);
      } else Reflect.apply(original, this, [image, ...coordinates]);
    };
  });
  await page.goto('/solvers');
  await expect(page.getByTestId('webgpu-map-fallback')).toContainText('blank-frame');
  await expect(page.getByTestId('webgpu-map-canvas')).toHaveCSS('visibility', 'hidden');
  await expect(page.getByTestId('canvas-map-fallback')).toHaveCSS('visibility', 'visible');
  await expect(page.getByRole('region', { name: 'Visualizer' }).locator('svg')).toHaveCount(0);
});

test('replay shortcuts respect native controls and Escape cancels a held scrub', async ({
  page,
}) => {
  await page.goto('/replays');
  const step = page.getByTestId('replay-current-step');
  await page.getByTestId('replay-viewer').focus();
  await page.keyboard.press('ArrowRight');
  await expect(step).toHaveText('2 / 5');
  await page.keyboard.press('Home');
  await expect(step).toHaveText('1 / 5');
  await page.getByTestId('replay-scrub').focus();
  await page.keyboard.down('ArrowRight');
  await expect(step).toHaveText('2 / 5');
  await page.keyboard.press('Escape');
  await expect(step).toHaveText('1 / 5');
  await page.keyboard.up('ArrowRight');
  await page.getByTestId('replay-viewer').focus();
  await page.keyboard.press('4');
  await expect(page.getByRole('radio').nth(2)).toBeChecked();
  await page.keyboard.press('Space');
  await expect(step).toHaveText('5 / 5');
});

test('all routes keep the visualizer primary and information in the Primer pane', async ({
  page,
}) => {
  await page.route('**/api/match/setting', (route) => route.fulfill({ json: setting }));
  await page.route('**/api/match', (route) => route.fulfill({ json: day() }));
  for (const route of ['/', '/replays', '/solvers', '/live']) {
    await page.goto(route);
    const content = page.locator('.PageLayout-content');
    const pane = page.locator('.PageLayout-pane');
    await expect(content.getByTestId('map-viewer')).toBeVisible();
    await expect(content.getByRole('region', { name: 'Visualizer' })).toBeVisible();
    await expect(content.locator('button,table,[role="status"],h2,svg')).toHaveCount(0);
    await expect(pane.getByRole('button', { name: 'Reset view' })).toBeVisible();
    await expect(pane).toBeVisible();
    const geometry = await page.evaluate(() => {
      const content = document.querySelector('.PageLayout-content')!.getBoundingClientRect();
      const pane = document.querySelector('.PageLayout-pane')!.getBoundingClientRect();
      const columns = getComputedStyle(document.querySelector('.PageLayout-columns')!);
      return {
        left: content.left,
        right: content.right,
        paneLeft: pane.left,
        width: content.width,
        paneWidth: pane.width,
        areas: columns.gridTemplateAreas,
      };
    });
    expect(geometry.width).toBeGreaterThan(geometry.paneWidth);
    expect(geometry.right).toBeLessThan(geometry.paneLeft);
    expect(geometry.areas).toBe('"content pane"');
    await expect(page.getByText('rebuildup /', { exact: true })).toHaveCount(0);
  }
  await page.setViewportSize({ width: 420, height: 800 });
  await page.goto('/replays');
  expect(
    await page
      .locator('.PageLayout-columns')
      .evaluate((el) => getComputedStyle(el).gridTemplateAreas),
  ).toBe('"content" "pane"');
  await page.reload();
  await expect(
    page.getByRole('navigation').getByRole('link', { name: 'Replays', exact: true }),
  ).toHaveAttribute('aria-current', 'page');
  await expect(page.getByTestId('replay-toggle')).toBeVisible();
});

test('viewer actions stay English; domain terms and rule sources connect to official sections', async ({
  page,
}) => {
  await page.goto('/');
  await expect(page.getByRole('link', { name: 'Rules', exact: true })).toHaveAttribute(
    'href',
    'https://rebuildup.dev/share/procon2026#map',
  );
  await expect(page.getByTestId('sample-source')).toContainText(
    '令和8年度香川（高松）大会募集要項.pdf',
  );
  await expect(page.getByTestId('sample-source').getByRole('link')).toHaveAttribute(
    'href',
    'https://rebuildup.dev/share/procon2026#s1-p8',
  );
  await page.getByLabel('Example').selectOption('figure2');
  await expect(page.getByTestId('sample-source')).toContainText('p.10・図2');
  await expect(page.getByTestId('sample-source').getByRole('link')).toHaveAttribute(
    'href',
    'https://rebuildup.dev/share/procon2026#s1-p10',
  );
  await page.getByText('ルール', { exact: true }).click();
  const topics = page.getByRole('navigation', { name: 'Rules by topic' }).getByRole('link');
  await expect(topics).toHaveCount(3);
  for (const [index, section] of ['map', 'agents', 'spots'].entries())
    await expect(topics.nth(index)).toHaveAttribute(
      'href',
      `https://rebuildup.dev/share/procon2026#${section}`,
    );
  await expect(page.getByText('平地', { exact: true })).toBeVisible();
  await expect(page.getByText('池', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Reset view', exact: true })).toBeVisible();
  await page.goto('/replays');
  await expect(page.getByTestId('replay-toggle')).toHaveText('Play');
  await expect(page.getByRole('link', { name: 'Rules', exact: true })).toHaveAttribute(
    'href',
    'https://rebuildup.dev/share/procon2026#time',
  );
});

test('live reserves the map frame while waiting and exposes received JSON', async ({ page }) => {
  let ready = false;
  await page.route('**/api/match/setting', (route) =>
    ready ? route.fulfill({ json: setting }) : route.fulfill({ status: 503 }),
  );
  await page.route('**/api/match', (route) => route.fulfill({ json: day() }));
  await page.goto('/live');
  const frame = page.getByRole('region', { name: 'Visualizer', exact: true });
  await expect(frame).toHaveAttribute('aria-busy', 'true');
  const before = await frame.boundingBox();
  ready = true;
  await expect(page.getByTestId('map-viewer')).toBeVisible();
  expect(await frame.boundingBox()).toEqual(before);
  await page.getByText('JSON', { exact: true }).click();
  const json = page.getByLabel('試合データ JSON');
  await expect(json).toBeVisible();
  expect(JSON.parse((await json.textContent())!).setting).toEqual(setting);
  await page.setViewportSize({ width: 420, height: 800 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('keyboard and wheel zoom preserve the visible center after panning', async ({ page }) => {
  await page.goto('/');
  const map = page.getByTestId('map-viewer');
  await map.focus();
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('ArrowDown');
  const camera = async () =>
    JSON.parse((await map.getAttribute('data-camera'))!) as { x: number; y: number; zoom: number };
  const before = await camera();
  await page.keyboard.press('+');
  const keyboard = await camera();
  expect(keyboard.zoom).toBeCloseTo(before.zoom * 1.1);
  expect(keyboard.x / keyboard.zoom).toBeCloseTo(before.x / before.zoom);
  expect(keyboard.y / keyboard.zoom).toBeCloseTo(before.y / before.zoom);
  await map.dispatchEvent('wheel', { deltaY: 100 });
  const wheel = await camera();
  expect(wheel.zoom).toBeCloseTo(before.zoom);
  expect(wheel.x).toBeCloseTo(before.x);
  expect(wheel.y).toBeCloseTo(before.y);
});

test('all pages show a legend and replay transport aligns at high speeds', async ({ page }) => {
  for (const route of ['/', '/replays', '/solvers', '/live', '/examples/q6']) {
    await page.goto(route);
    await expect(page.getByRole('heading', { name: 'Legend', exact: true })).toBeVisible();
  }
  await page.goto('/replays');
  const previous = await page.getByLabel('Step backward', { exact: true }).boundingBox();
  const first = await page.getByLabel('First step', { exact: true }).boundingBox();
  expect(first?.x).toBe(previous?.x);
  expect(first?.width).toBe(previous?.width);
  await expect(page.getByRole('progressbar', { name: 'Replay steps' })).toHaveAttribute(
    'aria-valuenow',
    '1',
  );
  await page.getByText('32×', { exact: true }).click();
  await expect(page.getByRole('radio', { name: '32×' })).toBeChecked();
  await page.getByLabel('Play', { exact: true }).click();
  await expect(page.getByTestId('replay-current-step')).toHaveText('5 / 5');
  await expect(page.getByRole('progressbar', { name: 'Replay steps' })).toHaveAttribute(
    'aria-valuenow',
    '5',
  );
  await expect(page.getByRole('table', { name: 'Score (demo)' })).toBeVisible();
});

test('legend shows distinct map terrain tiles and object icons in a compact grid', async ({
  page,
}) => {
  await page.goto('/');
  const legend = page
    .locator('section')
    .filter({ has: page.getByRole('heading', { name: 'Legend', exact: true }) });
  const tiles = legend.locator('canvas[data-terrain]');
  await expect(tiles).toHaveCount(6);
  await expect(legend.locator('canvas[data-mark]')).toHaveCount(8);
  await expect
    .poll(() =>
      tiles.evaluateAll((canvases) =>
        canvases.every((node) => {
          const canvas = node as HTMLCanvasElement;
          return canvas
            .getContext('2d')!
            .getImageData(0, 0, 64, 64)
            .data.some((value, index) => index % 4 === 3 && value > 0);
        }),
      ),
    )
    .toBe(true);
  const signatures = await tiles.evaluateAll((canvases) =>
    canvases.map((node) => (node as HTMLCanvasElement).toDataURL()),
  );
  expect(new Set(signatures).size).toBe(6);
  expect((await legend.boundingBox())!.height).toBeLessThan(430);
  await expect(legend.getByText('混雑', { exact: true })).toBeVisible();
  await expect(legend.getByText('波', { exact: true })).toBeVisible();
  await page.setViewportSize({ width: 420, height: 800 });
  expect((await legend.boundingBox())!.height).toBeLessThan(430);
});
