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
  const map = page.getByTestId('map-viewer-svg');
  await map.focus();
  await page.keyboard.press('+');
  await expect(map.locator(':scope > g')).toHaveAttribute('transform', /scale\(1.1\)/);
  await page.keyboard.press('0');
  await expect(map.locator(':scope > g')).toHaveAttribute('transform', 'translate(0 0) scale(1)');
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

test('renderer selection returns to SVG and has a readable fallback', async ({ page }) => {
  await page.addInitScript(() =>
    Object.defineProperty(navigator, 'gpu', { value: undefined, configurable: true }),
  );
  await page.goto('/solvers');
  await page.getByTestId('solvers-renderer-webgpu').click();
  await expect(page.getByTestId('webgpu-map-fallback')).toBeVisible();
  await expect(page.getByTestId('map-viewer-svg')).toBeVisible();
  await expect(page.getByTestId('webgpu-map-loading')).toHaveCount(0);
  await page.getByTestId('solvers-renderer-svg').click();
  await expect(page.getByTestId('webgpu-map-fallback')).toHaveCount(0);
});

test('WebGPU renders colored cells when a headless adapter is available', async ({ page }) => {
  await page.goto('/solvers');
  await page.getByTestId('solvers-renderer-webgpu').click();
  await expect(page.getByTestId('webgpu-map-loading')).toHaveCount(0, { timeout: 10000 });
  await expect(page.getByTestId('webgpu-map-fallback')).toHaveCount(0);
  await expect(page.getByTestId('webgpu-map-canvas')).toHaveCSS('visibility', 'visible');
  const canvas = page.getByTestId('webgpu-map-canvas');
  await canvas.screenshot({ path: '../../target/playwright/ui-271-webgpu.png' });
  await page.getByTestId('map-viewer-svg').focus();
  await page.keyboard.press('+');
  await page.getByLabel('Toggle theme').click();
  await expect(canvas).toHaveCSS('visibility', 'visible');
});

test('a blank GPU frame retains the SVG map', async ({ page }) => {
  await page.addInitScript(() => {
    CanvasRenderingContext2D.prototype.drawImage = function () {
      this.fillStyle = getComputedStyle(document.documentElement).getPropertyValue(
        '--bgColor-muted',
      );
      this.fillRect(0, 0, this.canvas.width, this.canvas.height);
    };
  });
  await page.goto('/solvers');
  await page.getByTestId('solvers-renderer-webgpu').click();
  await expect(page.getByTestId('webgpu-map-fallback')).toContainText('blank-frame');
  await expect(page.getByTestId('webgpu-map-canvas')).toHaveCSS('visibility', 'hidden');
  await expect(page.getByTestId('map-viewer-svg').locator('polygon').first()).not.toHaveAttribute(
    'fill',
    'transparent',
  );
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

test('overview links to details and reload preserves the navigation context', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Overview', exact: true })).toBeVisible();
  await expect(page.getByTestId('map-viewer-svg')).toHaveCount(0);
  await page
    .getByRole('region', { name: 'Workspace contents' })
    .getByRole('link', { name: 'Replays', exact: true })
    .click();
  await expect(page).toHaveURL(/\/replays$/);
  await page.reload();
  await expect(page.getByTestId('replay-toggle')).toBeVisible();
  await expect(
    page.getByRole('navigation').getByRole('link', { name: 'Replays', exact: true }),
  ).toHaveAttribute('aria-current', 'page');
});
