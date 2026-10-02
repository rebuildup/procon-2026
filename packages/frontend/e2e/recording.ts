/**
 * Screen-recording harness helpers.
 *
 * `recordScenario` opens a dedicated browser context with `recordVideo`
 * enabled, drives the scenario's navigation / interaction steps, holds the
 * page on screen for at least `RECORD_MS`, drops a final-frame PNG next to
 * the finished `.webm`, and upserts the scenario into `manifest.json`.
 *
 * The context is created manually instead of using Playwright's default
 * `context` fixture so the artifact names are stable: the runner's own
 * `video: 'on'` naming is a content hash, which is useless for humans
 * browsing `target/recordings/`.
 *
 * Recording is opt-in (`bun run record` / `just record`) — never CI.
 * See `docs/development.md` § 画面録画.
 */
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, renameSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

import { expect, type Browser, type Page } from '@playwright/test';

/** `packages/frontend/e2e` — everything else is resolved from here. */
const HERE = dirname(fileURLToPath(import.meta.url));
/** Repository root (`packages/frontend/e2e` → up three levels). */
const REPO_ROOT = resolve(HERE, '..', '..', '..');

/** Dedicated dev-server port: never collides with `bun run dev` (5173). */
export const PORT = 5273;
export const BASE_URL = `http://127.0.0.1:${PORT}`;
export const VIEWPORT: { width: number; height: number } = { width: 1280, height: 720 };

/**
 * How long each scenario stays on screen after its setup steps (ms).
 * Override with `RECORD_MS=<ms> bun run record`.
 */
const envMs = Number(process.env.RECORD_MS ?? '');
export const RECORD_MS: number = Number.isFinite(envMs) && envMs > 0 ? envMs : 6000;

/** Artifact directory (repo-root `target/recordings`, gitignored). */
export const OUT_DIR: string = process.env.RECORD_OUT
  ? resolve(process.env.RECORD_OUT)
  : resolve(REPO_ROOT, 'target', 'recordings');
/** Scratch directory for the raw hash-named videos Playwright writes. */
const RAW_DIR = join(OUT_DIR, '.raw');

/** Committed snapshot served in place of the gitignored `/match-state.json`. */
export const LIVE_SNAPSHOT_FIXTURE = resolve(HERE, 'fixtures', 'match-state.snapshot.json');

/** What the browser reported about WebGPU at recording time. */
export interface WebGpuProbe {
  readonly api: boolean;
  readonly adapter: boolean;
}

export interface ManifestScenario {
  readonly name: string;
  readonly route: string;
  readonly video: string;
  readonly screenshot: string;
  readonly videoBytes: number;
  readonly durationMs: number;
  readonly chromiumVersion: string;
  readonly webgpu: WebGpuProbe;
  readonly recordedAt: string;
}

export interface Manifest {
  generatedAt: string;
  commit: string | null;
  viewport: { width: number; height: number };
  recordMs: number;
  scenarios: ManifestScenario[];
}

export interface Scenario {
  /** Artifact base name (`<name>.webm` / `<name>.png`). */
  readonly name: string;
  /** Route relative to `baseURL`. */
  readonly route: string;
  /** Network fixtures to install before navigating (e.g. the live snapshot). */
  readonly setup?: (page: Page) => Promise<void>;
  /** Resolve once the page has rendered what this scenario is meant to show. */
  readonly ready: (page: Page) => Promise<void>;
  /** Quick interactions that play immediately after `ready` (play, toggle, …). */
  readonly steps?: (page: Page) => Promise<void>;
  /**
   * Fill the remaining recording time. Defaults to a plain wait; scenarios
   * that need continuous motion (the replay transport) pass their own loop.
   */
  readonly hold?: (page: Page, remainingMs: number) => Promise<void>;
}

/** Probe `navigator.gpu` + adapter availability in the recorded page. */
async function probeInPage(): Promise<WebGpuProbe> {
  const gpu = (navigator as Navigator & { gpu?: GPU }).gpu;
  if (!gpu) return { api: false, adapter: false };
  try {
    const adapter = await gpu.requestAdapter();
    return { api: true, adapter: adapter !== null };
  } catch {
    return { api: true, adapter: false };
  }
}

export async function probeWebGpu(page: Page): Promise<WebGpuProbe> {
  return page.evaluate(probeInPage);
}

function gitCommit(): string | null {
  try {
    return execFileSync('git', ['rev-parse', 'HEAD'], { cwd: REPO_ROOT, encoding: 'utf8' }).trim();
  } catch {
    return null;
  }
}

/** Replace (or append) this scenario's entry, keeping the rest of the run. */
function upsertScenario(entry: ManifestScenario): void {
  const manifestPath = join(OUT_DIR, 'manifest.json');
  let manifest: Manifest = {
    generatedAt: new Date().toISOString(),
    commit: gitCommit(),
    viewport: { width: VIEWPORT.width, height: VIEWPORT.height },
    recordMs: RECORD_MS,
    scenarios: [],
  };
  if (existsSync(manifestPath)) {
    try {
      manifest = JSON.parse(readFileSync(manifestPath, 'utf8')) as Manifest;
    } catch {
      // A truncated manifest from an interrupted run: rebuild from scratch.
    }
  }
  const others = manifest.scenarios.filter((s) => s.name !== entry.name);
  manifest.scenarios = [...others, entry].sort((a, b) => a.name.localeCompare(b.name));
  manifest.generatedAt = new Date().toISOString();
  manifest.commit = gitCommit();
  manifest.viewport = { width: VIEWPORT.width, height: VIEWPORT.height };
  manifest.recordMs = RECORD_MS;
  writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
}

/**
 * Record one scenario end-to-end. Throws (failing the Playwright test) when
 * the scenario cannot render or the resulting video is empty.
 */
export async function recordScenario(browser: Browser, scenario: Scenario): Promise<void> {
  mkdirSync(RAW_DIR, { recursive: true });
  const startedAt = Date.now();
  const context = await browser.newContext({
    baseURL: BASE_URL,
    viewport: VIEWPORT,
    recordVideo: { dir: RAW_DIR, size: VIEWPORT },
  });

  try {
    const page = await context.newPage();
    if (scenario.setup) await scenario.setup(page);
    await page.goto(scenario.route, { waitUntil: 'load' });
    await scenario.ready(page);
    const webgpu = await probeWebGpu(page);

    // The hold window starts once the page is ready, so every recording has
    // at least `RECORD_MS` of the screen a reviewer wants to look at.
    const deadline = Date.now() + RECORD_MS;
    if (scenario.steps) await scenario.steps(page);
    const remainingMs = Math.max(0, deadline - Date.now());
    if (scenario.hold) {
      await scenario.hold(page, remainingMs);
    } else {
      await page.waitForTimeout(remainingMs);
    }
    await page.screenshot({ path: join(OUT_DIR, `${scenario.name}.png`) });

    // Playwright only finalizes the `.webm` when the context closes.
    const video = page.video();
    await context.close();
    if (!video) throw new Error(`[${scenario.name}] context recorded no video`);
    const rawPath = await video.path();
    const webmPath = join(OUT_DIR, `${scenario.name}.webm`);
    renameSync(rawPath, webmPath);
    const videoBytes = statSync(webmPath).size;
    expect(
      videoBytes,
      `[${scenario.name}] recording is suspiciously small (${videoBytes} bytes)`,
    ).toBeGreaterThan(10_000);

    upsertScenario({
      name: scenario.name,
      route: scenario.route,
      video: `${scenario.name}.webm`,
      screenshot: `${scenario.name}.png`,
      videoBytes,
      durationMs: Date.now() - startedAt,
      chromiumVersion: browser.version(),
      webgpu,
      recordedAt: new Date().toISOString(),
    });
  } finally {
    // No-op when the happy path already closed it; guarantees teardown on error.
    await context.close().catch(() => undefined);
  }
}
