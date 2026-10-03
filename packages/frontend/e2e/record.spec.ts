/**
 * One recorded scenario per frontend route.
 *
 * Each test produces `target/recordings/<name>.webm` + `<name>.png` and a
 * `manifest.json` entry (see `./recording.ts`). Run with `bun run record`
 * or `just record`; never part of CI.
 */
import { readFileSync } from 'node:fs';

import { expect, test } from '@playwright/test';

import { LIVE_SNAPSHOT_FIXTURE, recordScenario } from './recording';

test.describe('frontend screen recording', () => {
  test('home', async ({ browser }) => {
    await recordScenario(browser, {
      name: 'home',
      route: '/',
      ready: (page) => page.locator('[data-testid="app-header"]').waitFor(),
    });
  });

  test('replays', async ({ browser }) => {
    await recordScenario(browser, {
      name: 'replays',
      route: '/replays',
      ready: async (page) => {
        await expect(page.locator('[data-testid="replay-toggle"]')).toBeVisible();
        await expect(page.locator('[data-testid="replay-current-step"]')).toHaveText('1 / 5');
      },
      steps: async (page) => {
        await page.locator('[data-testid="replay-toggle"]').click();
        // The inline fixture must actually advance before we trust the clip.
        await expect
          .poll(() => page.locator('[data-testid="replay-current-step"]').textContent(), {
            timeout: 10_000,
          })
          .not.toBe('1 / 5');
        await page.locator('[data-testid="replay-rate-2"]').click();
      },
      hold: async (page, remainingMs) => {
        const toggle = page.locator('[data-testid="replay-toggle"]');
        const deadline = Date.now() + remainingMs;
        while (Date.now() < deadline) {
          // `play()` rewinds to step 1 once the five-step fixture has run
          // out, so pressing Play again keeps the clip moving throughout.
          if ((await toggle.getAttribute('aria-label')) === 'Play') {
            await toggle.click();
          }
          await page.waitForTimeout(300);
        }
      },
    });
  });

  test('solvers', async ({ browser }) => {
    const loading = '[data-testid="webgpu-map-loading"]';
    await recordScenario(browser, {
      name: 'solvers',
      route: '/solvers',
      ready: async (page) => {
        await expect(page.locator('[data-testid="solvers-renderer-svg"]')).toBeVisible();
        // Mount settled into GPU canvas, SVG fallback, or (no adapter) straight
        // to fallback — all three end with the loading line gone.
        await expect(page.locator(loading)).toHaveCount(0, { timeout: 15_000 });
      },
      steps: async (page) => {
        await page.locator('[data-testid="solvers-renderer-svg"]').click();
        await expect(page.locator('[data-testid="map-viewer-svg"]')).toBeVisible();
        await page.waitForTimeout(1_500);
        await page.locator('[data-testid="solvers-renderer-webgpu"]').click();
        await expect(page.locator(loading)).toHaveCount(0, { timeout: 15_000 });
        await page.waitForTimeout(1_500);
        await page.locator('[data-testid="solvers-renderer-svg"]').click();
        await expect(page.locator('[data-testid="map-viewer-svg"]')).toBeVisible();
      },
    });
  });

  test('live', async ({ browser }) => {
    await recordScenario(browser, {
      name: 'live',
      route: '/live',
      setup: async (page) => {
        // `/match-state.json` is a gitignored CLI output; serve the committed
        // schemaVersion-1 fixture instead so the HUD is deterministic even on
        // a fresh clone. `lastUpdateUnix` is stamped at fulfill time so the
        // "Last update" clock reads like a live poll.
        const fixture = JSON.parse(readFileSync(LIVE_SNAPSHOT_FIXTURE, 'utf8')) as Record<
          string,
          unknown
        >;
        await page.route(
          (url) => url.pathname === '/match-state.json',
          (route) =>
            route.fulfill({
              contentType: 'application/json',
              body: JSON.stringify({ ...fixture, lastUpdateUnix: Math.floor(Date.now() / 1000) }),
            }),
        );
      },
      ready: (page) =>
        expect(
          page.locator('[data-testid="live-status-badge"][data-status="running"]'),
        ).toBeVisible({ timeout: 15_000 }),
    });
  });
});
