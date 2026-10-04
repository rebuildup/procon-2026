import { defineConfig } from '@playwright/test';
import recording from './playwright.config';

const port = 5399;
const baseURL = `http://127.0.0.1:${port}`;

/** Opt-in renderer scheduling regression, including Firefox's Canvas path. */
export default defineConfig({
  testDir: './e2e',
  testMatch: 'frame-scheduling.spec.ts',
  outputDir: '../../target/frame-tests',
  workers: 1,
  retries: 0,
  timeout: 30_000,
  reporter: [['list'], ['json', { outputFile: '../../target/frame-tests/results.json' }]],
  use: { baseURL, viewport: { width: 1280, height: 720 } },
  projects: [
    {
      name: 'chromium-swiftshader-dpr1',
      use: { browserName: 'chromium', launchOptions: recording.use?.launchOptions },
    },
    {
      name: 'chromium-swiftshader-dpr2',
      use: {
        browserName: 'chromium',
        deviceScaleFactor: 2,
        launchOptions: recording.use?.launchOptions,
      },
    },
    { name: 'firefox-canvas-dpr1', grep: /Canvas coalesces/, use: { browserName: 'firefox' } },
    {
      name: 'firefox-canvas-dpr2',
      grep: /Canvas coalesces/,
      use: { browserName: 'firefox', deviceScaleFactor: 2 },
    },
  ],
  webServer: {
    command: `bun run dev -- --host 127.0.0.1 --port ${port} --strictPort`,
    url: baseURL,
    reuseExistingServer: false,
    timeout: 30_000,
  },
});
