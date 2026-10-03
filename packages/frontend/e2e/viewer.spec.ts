import { readFileSync } from 'node:fs';
import { expect, test } from '@playwright/test';
import { LIVE_SNAPSHOT_FIXTURE } from './recording';

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

test('live feed preserves old wire timestamps and marks failed polling as stale', async ({
  page,
}) => {
  const fixture = JSON.parse(readFileSync(LIVE_SNAPSHOT_FIXTURE, 'utf8')) as Record<
    string,
    unknown
  >;
  let phase: 'running' | 'error' | 'offline' = 'running';
  await page.route(
    (url) => url.pathname === '/match-state.json',
    (route) =>
      phase === 'offline'
        ? route.fulfill({ status: 404, body: '' })
        : route.fulfill({
            contentType: 'application/json',
            body: JSON.stringify({
              ...fixture,
              status: phase,
              error: phase === 'error' ? 'Solver failed' : null,
              lastUpdateUnix: 0,
            }),
          }),
  );
  await page.goto('/live');
  await expect(page.getByTestId('live-status-badge')).toHaveText('Running');
  await expect(page.getByText('00:00:00', { exact: true })).toBeVisible();
  phase = 'error';
  await expect(page.getByTestId('live-status-badge')).toHaveText('Error');
  await expect(page.getByRole('alert')).toHaveText('Solver failed');
  phase = 'offline';
  await expect(page.getByTestId('live-status-badge')).toHaveText('Stale', { timeout: 10000 });
  await expect(page.getByRole('alert')).toHaveText('Solver failed');
});

test('missing live feed leaves the connecting state', async ({ page }) => {
  await page.route(
    (url) => url.pathname === '/match-state.json',
    (route) => route.fulfill({ status: 404, body: '' }),
  );
  await page.goto('/live');
  await expect(page.getByTestId('live-status-badge')).toHaveText('No data', { timeout: 10000 });
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

test('GPU draws all domain symbols and advances the agent without SVG overlay', async ({
  page,
}) => {
  await page.goto('/replays');
  await expect(page.getByTestId('map-viewer')).toHaveAttribute('data-renderer', 'WebGPU', {
    timeout: 10000,
  });
  await expect(page.getByTestId('webgpu-map-canvas')).toHaveCSS('visibility', 'visible');
  await expect(
    page.getByRole('region', { name: 'Visualizer' }).locator('svg,path,polygon,button'),
  ).toHaveCount(0);
  const sample = async () => {
    const bytes = Array.from(await page.getByTestId('webgpu-map-canvas').screenshot());
    return page.getByTestId('webgpu-map-canvas').evaluate(async (el, bytes) => {
      const canvas = el as HTMLCanvasElement,
        probe = document.createElement('canvas');
      probe.width = canvas.width;
      probe.height = canvas.height;
      const context = probe.getContext('2d')!;
      const imageBitmap = await createImageBitmap(
        new Blob([new Uint8Array(bytes)], { type: 'image/png' }),
      );
      context.drawImage(imageBitmap, 0, 0);
      imageBitmap.close();
      const image = context.getImageData(0, 0, probe.width, probe.height);
      const fit = Math.min(canvas.width / 370, canvas.height / 265),
        ox = (canvas.width - 370 * fit) / 2,
        oy = (canvas.height - 265 * fit) / 2;
      const contains = (x: number, y: number, size: number, hue: string) => {
        const hex = getComputedStyle(document.documentElement)
          .getPropertyValue(`--data-${hue}-color-emphasis`)
          .trim();
        const color = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
        let count = 0;
        for (
          let py = Math.ceil((y + 24) * fit + oy);
          py < Math.floor((y + size + 24) * fit + oy);
          py++
        )
          for (
            let px = Math.ceil((x + 24) * fit + ox);
            px < Math.floor((x + size + 24) * fit + ox);
            px++
          ) {
            const i = (py * image.width + px) * 4;
            if (color.every((c, n) => Math.abs(image.data[i + n]! - c) < 4)) count++;
          }
        return count;
      };
      return {
        terrain: [
          contains(-8, -8, 16, 'gray'),
          contains(32, -8, 16, 'green'),
          contains(192, -8, 16, 'brown'),
          contains(52, 27, 16, 'blue'),
        ],
        spots: ['yellow', 'orange', 'auburn', 'purple', 'plum', 'olive'].map((h, n) =>
          contains(40 + n * 40, 27 + (n % 2) * 70, 18, h),
        ),
        patrol: [contains(40, 132, 20, 'pine'), contains(80, 132, 20, 'pine')],
        supply: contains(220, 100, 20, 'pink'),
      };
    }, bytes);
  };
  const first = await sample();
  first.terrain.forEach((count) => expect(count).toBeGreaterThan(2));
  first.spots.forEach((count) => expect(count).toBeGreaterThan(2));
  expect(first.supply).toBeGreaterThan(2);
  expect(first.patrol[0]).toBeGreaterThan(2);
  expect(first.patrol[1]).toBe(0);
  await page.getByLabel('Step forward', { exact: true }).click();
  await expect.poll(async () => (await sample()).patrol[1]).toBeGreaterThan(2);
  expect((await sample()).patrol[0]).toBe(0);
  const before = await page.getByTestId('webgpu-map-canvas').screenshot();
  await page.getByTestId('map-viewer').focus();
  await page.keyboard.press('+');
  expect(await page.getByTestId('webgpu-map-canvas').screenshot()).not.toEqual(before);
  await page.getByLabel('Reset view', { exact: true }).click();
  await expect(page.getByTestId('map-viewer')).toHaveAttribute(
    'data-camera',
    '{"x":0,"y":0,"zoom":1}',
  );
  await page.getByLabel('Toggle theme').click();
  await expect(page.getByTestId('map-viewer')).toHaveAttribute('data-renderer', 'WebGPU');
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
