import { setting, day } from './team-fixture';
/**
 * One recorded scenario per frontend route.
 *
 * Each test produces `target/recordings/<name>.webm` + `<name>.png` and a
 * `manifest.json` entry (see `./recording.ts`). Run with `bun run record`
 * or `just record`; never part of CI.
 */

import { expect, test } from '@playwright/test';

import { recordScenario } from './recording';

test.describe('frontend screen recording', () => {
  test('home', async ({ browser }) => {
    await recordScenario(browser, {
      name: 'home',
      route: '/',
      ready: async (page) => {
        await page.locator('[data-testid="app-header"]').waitFor();
        await expect(page.getByTestId('map-viewer')).toHaveAttribute(
          'data-renderer',
          /WebGPU|Canvas/,
          { timeout: 15000 },
        );
      },
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
          if ((await toggle.getAttribute('aria-label')) === 'Replays') {
            await toggle.click();
          }
          await page.waitForTimeout(300);
        }
      },
    });
  });

  test('solvers', async ({ browser }) => {
    await recordScenario(browser, {
      name: 'solvers',
      route: '/solvers',
      ready: async (page) => {
        await expect(page.getByTestId('map-viewer')).toHaveAttribute(
          'data-renderer',
          /WebGPU|Canvas/,
          { timeout: 15000 },
        );
      },
      steps: async (page) => {
        await page.getByTestId('map-viewer').focus();
        await page.keyboard.press('+');
        await page.keyboard.press('ArrowLeft');
        await page.waitForTimeout(1500);
        await page.getByLabel('Reset view', { exact: true }).click();
        await page.waitForTimeout(1500);
        await page.getByLabel('Example').selectOption('figure2');
        await page.waitForTimeout(1500);
        await page.getByLabel('Outer grid').selectOption('2');
        await page.waitForTimeout(1500);
      },
    });
  });

  test('live', async ({ browser }) => {
    await recordScenario(browser, {
      name: 'live',
      route: '/live',
      setup: async (page) => {
        await page.route('**/api/match/setting', (route) => route.fulfill({ json: setting }));
        await page.route('**/api/match', (route) => route.fulfill({ json: day() }));
      },
      ready: (page) =>
        expect(
          page.locator('[data-testid="live-status-badge"][data-status="running"]'),
        ).toBeVisible({ timeout: 15_000 }),
    });
  });
});
