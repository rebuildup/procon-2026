import { expect, test } from '@playwright/test';
import { recordScenario } from './recording';

test('records the official Q6 phase table', async ({ browser }) => {
  let initialMap: Buffer;
  await recordScenario(browser, {
    name: 'official-q6',
    route: '/examples/q6',
    viewport: { width: 1440, height: 1000 },
    ready: async (page) => {
      page.on('pageerror', (error) => {
        throw error;
      });
      await expect(page.getByTestId('q6-phase')).toHaveText('初期');
      await expect(page.getByTestId('map-viewer')).toHaveCount(8);
      await expect(page.getByTestId('q6-agent-7')).toBeInViewport();
      await expect(page.getByTestId('q6-agent-7')).toHaveText('セル 2');
      await expect(page.getByTestId('map-viewer').first()).toHaveAttribute(
        'data-renderer',
        /WebGPU|Canvas/,
      );
      initialMap = await page.getByTestId('map-viewer').first().screenshot();
    },
    steps: async (page) => {
      await page.getByRole('button', { name: 'Play example', exact: true }).click();
      await expect(page.getByTestId('q6-phase')).toHaveText('最終', { timeout: 35000 });
      await expect(page.getByTestId('q6-agent-0')).toHaveText('セル 3 · 燃料 0 · 1玉');
      await expect(page.getByTestId('q6-agent-2')).toHaveText('セル 2 · 燃料 1 · 0玉');
      await expect(page.getByTestId('q6-agent-7')).toHaveText('セル 0');
      expect(await page.getByTestId('map-viewer').first().screenshot()).not.toEqual(initialMap);
    },
  });
});
