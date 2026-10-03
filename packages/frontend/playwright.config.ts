import process from 'node:process';

import { defineConfig } from '@playwright/test';

import { BASE_URL, PORT, VIEWPORT } from './e2e/recording';

/**
 * Playwright config for the screen-recording harness (`bun run record`).
 *
 * - Recording is opt-in and local-only; CI never starts a browser.
 * - The dev server runs on its own port (5273) so it cannot collide with a
 *   developer's `bun run dev` (5173) or `vite preview` (4173), and
 *   `--strictPort` fails loudly instead of silently moving.
 * - WebGPU launch flags make headless runs exercise the real GPU renderer;
 *   every page still falls back to Canvas when no adapter is available.
 */
export default defineConfig({
  testDir: './e2e',
  // Runner scratch space (traces, temp files) — gitignored via `/target`.
  outputDir: '../../target/playwright',
  globalSetup: './e2e/global-setup.ts',
  // Serial by design: parallel workers would fight for CPU (dropped frames)
  // and race the manifest read-modify-write.
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 120_000,
  reporter: [['list']],
  use: {
    baseURL: BASE_URL,
    viewport: VIEWPORT,
    launchOptions: {
      args: [
        '--enable-unsafe-webgpu',
        '--use-angle=swiftshader',
        '--use-vulkan=swiftshader',
        '--enable-features=Vulkan',
      ],
    },
  },
  webServer: {
    command: `bun run dev -- --port ${PORT} --strictPort`,
    url: BASE_URL,
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
  },
});
