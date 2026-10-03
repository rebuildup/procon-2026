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

test('live feed displays status, errors and source timestamp staleness', async ({ page }) => {
  const fixture = JSON.parse(readFileSync(LIVE_SNAPSHOT_FIXTURE, 'utf8')) as Record<
    string,
    unknown
  >;
  let stale = false;
  await page.route(
    (url) => url.pathname === '/match-state.json',
    (route) =>
      route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({
          ...fixture,
          status: stale ? 'error' : 'running',
          error: stale ? 'Solver failed' : null,
          lastUpdateUnix: Math.floor(Date.now() / 1000) - (stale ? 10 : 0),
        }),
      }),
  );
  await page.goto('/live');
  await expect(page.getByTestId('live-status-badge')).toHaveText('running');
  stale = true;
  await expect(page.getByTestId('live-status-badge')).toHaveText('Stale');
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
