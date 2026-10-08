import { expect, test } from '@playwright/test';
import { setting, day } from './team-fixture';
import type * as ViewerSceneModule from '../src/lib/webgpu/viewer-scene';

for (const renderer of ['Canvas', 'WebGPU'] as const) {
  test(`${renderer} inspects overlapping vehicles without redrawing the map`, async ({ page }) => {
    await page.addInitScript((renderer) => {
      const counter = { draws: 0 };
      Object.assign(window, { inspectionCounter: counter });
      if (renderer === 'Canvas') {
        Object.defineProperty(navigator, 'gpu', { value: undefined, configurable: true });
        const getContext = HTMLCanvasElement.prototype.getContext;
        HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, ...args) {
          if (this.dataset.testid === 'canvas-map-fallback' && args[0] === '2d') counter.draws++;
          return Reflect.apply(getContext, this, args);
        } as typeof getContext;
      } else {
        const submit = GPUQueue.prototype.submit;
        GPUQueue.prototype.submit = function (...args) {
          counter.draws++;
          return Reflect.apply(submit, this, args);
        };
      }
    }, renderer);
    const state = {
      ...day(),
      agents: [
        { kind: 0, pos: 22, fuel: 0 },
        { kind: 1, pos: 22, fuel: 999 },
      ],
      others: [{ id: 7, agents: [{ kind: 0, pos: 22, fuel: 8 }] }],
      traffics: [{ pos: 22, status: 2 }],
    };
    await page.route('**/api/match/setting', (route) =>
      route.fulfill({ json: { ...setting, fuelLimits: 20 } }),
    );
    await page.route('**/api/match', (route) => route.fulfill({ json: state }));
    await page.goto('/live');
    const map = page.getByTestId('map-viewer');
    await expect(map).toHaveAttribute('data-renderer', renderer);
    const table = page.getByRole('table', { name: '自チームの車両' });
    await expect(table).toContainText('0 / 20');
    await expect(table).toContainText('対象外');
    // Freeze polling so draw counts measure pointer input alone.
    await page.unroute('**/api/match');
    await page.route('**/api/match', (route) => route.fulfill({ status: 503 }));
    await expect(page.getByTestId('live-status-badge')).toHaveText('Stale', { timeout: 10000 });
    const point = async (cell = 22) =>
      page.evaluate(
        async ({ setting, cell }) => {
          const path = '/src/lib/webgpu/viewer-scene.ts';
          const { sceneTransform } = (await import(path)) as typeof ViewerSceneModule;
          const scale = 23 / 30;
          const cells = setting.map.cells.flatMap((row, y) =>
            row.map((_, x) => ({
              x: (x * 52 + (y % 2 === 0 ? 26 : 0)) * scale,
              y: y * 45 * scale,
              terrain: 'plain' as const,
            })),
          );
          const stage = document.querySelector('[data-testid="map-viewer"]')!;
          const rect = stage.getBoundingClientRect();
          const transform = sceneTransform(
            { cells, marks: [] },
            rect.width,
            rect.height,
            JSON.parse(stage.getAttribute('data-camera')!),
          );
          return {
            x: rect.left + cells[cell]!.x * transform.scale + transform.x,
            y: rect.top + cells[cell]!.y * transform.scale + transform.y,
          };
        },
        { setting, cell },
      );
    const p = await point();
    const draws = () =>
      page.evaluate(
        () =>
          (window as unknown as { inspectionCounter: { draws: number } }).inspectionCounter.draws,
      );
    const before = await draws();
    await page.mouse.move(p.x, p.y);
    const tooltip = page.getByRole('tooltip', { name: 'セル情報' });
    await expect(tooltip).toContainText('セル 22');
    await expect(tooltip).toContainText('最大在庫 5');
    await expect(tooltip).toContainText('渋滞');
    await expect(tooltip.getByRole('row', { name: /自車 #1/ })).toContainText('0 / 20');
    await expect(tooltip.getByRole('row', { name: /自車 #2/ })).toContainText('対象外');
    await expect(tooltip).toContainText('相手 7 車 #1');
    for (let i = 0; i < 10; i++) await page.mouse.move(p.x + (i % 2), p.y);
    expect(await draws()).toBe(before);
    const occupiedBox = await tooltip.boundingBox();
    const empty = await point(0);
    await page.mouse.move(empty.x, empty.y);
    await expect(tooltip).toContainText('車両なし');
    const emptyBox = await tooltip.boundingBox();
    expect(emptyBox!.width).toBeCloseTo(occupiedBox!.width, 3);
    expect(emptyBox!.height).toBeCloseTo(occupiedBox!.height, 3);
    expect(emptyBox?.x !== occupiedBox?.x || emptyBox?.y !== occupiedBox?.y).toBe(true);
    const distance = (box: NonNullable<typeof occupiedBox>, anchor: { x: number; y: number }) =>
      Math.hypot(
        Math.max(box.x - anchor.x, anchor.x - box.x - box.width, 0),
        Math.max(box.y - anchor.y, anchor.y - box.y - box.height, 0),
      );
    expect(distance(occupiedBox!, p)).toBeLessThan(80);
    expect(distance(emptyBox!, empty)).toBeLessThan(80);
    expect(emptyBox!.x).toBeGreaterThanOrEqual(8);
    expect(emptyBox!.y).toBeGreaterThanOrEqual(8);
    const selected = page.getByLabel('セル番号（ホバー・タップでも選択）');
    await expect(selected).toHaveValue('0');
    await expect(page.locator('.viewer-overlay')).toHaveAttribute('data-selected-cell', '0');
    expect(await draws()).toBe(before);
    await map.focus();
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('+');
    const zoomed = await point();
    await page.mouse.move(zoomed.x, zoomed.y);
    await expect(tooltip).toContainText('セル 22');
    const stageBox = await map.boundingBox();
    await map.click({ position: { x: zoomed.x - stageBox!.x, y: zoomed.y - stageBox!.y } });
    await expect(selected).toHaveValue('22');
    await map.focus();
    await page.keyboard.press('Escape');
    await expect(tooltip).toBeHidden();
    await page.mouse.move(0, 0);
    await expect(selected).toHaveValue('');
    await expect(page.locator('.viewer-overlay')).not.toHaveAttribute('data-selected-cell');
    await page.setViewportSize({ width: 420, height: 800 });
    await page.getByLabel('セル番号（ホバー・タップでも選択）').fill('22');
    await expect(page.getByRole('heading', { name: /^セル 22/ })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
  });
}

test('controls scroll independently and selected hex has a visible overlay', async ({ page }) => {
  await page.route('**/api/match/setting', (route) =>
    route.fulfill({ json: { ...setting, fuelLimits: 20 } }),
  );
  await page.route('**/api/match', (route) => route.fulfill({ json: day() }));
  await page.goto('/live');
  const map = page.getByTestId('map-viewer'),
    pane = page.getByRole('complementary');
  await expect(map).toBeVisible();
  const before = await map.boundingBox();
  await pane.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  expect(await map.boundingBox()).toEqual(before);
  await page.getByLabel('セル番号（ホバー・タップでも選択）').fill('22');
  const overlay = page.locator('.viewer-overlay');
  await expect(overlay).toHaveAttribute('data-selected-cell', '22');
  await expect
    .poll(() =>
      overlay.evaluate((element) => {
        const canvas = element as HTMLCanvasElement;
        return canvas
          .getContext('2d')!
          .getImageData(0, 0, canvas.width, canvas.height)
          .data.some((value, index) => index % 4 === 3 && value > 0);
      }),
    )
    .toBe(true);
});
