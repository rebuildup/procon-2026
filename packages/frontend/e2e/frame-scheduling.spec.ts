import { expect, test } from '@playwright/test';

for (const renderer of ['Canvas', 'WebGPU'] as const) {
  test(`${renderer} coalesces camera input and redraws replay, theme and resize`, async ({
    page,
  }, testInfo) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.addInitScript((renderer) => {
      const counts = { draws: 0, detached: 0 };
      Object.assign(window, { frameCounts: counts });
      if (renderer === 'Canvas') {
        Object.defineProperty(navigator, 'gpu', { value: undefined, configurable: true });
        const original = HTMLCanvasElement.prototype.getContext;
        HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, ...args) {
          if (this.dataset.testid === 'canvas-map-fallback' && args[0] === '2d') {
            counts.draws++;
            if (!this.isConnected) counts.detached++;
          }
          return Reflect.apply(original, this, args);
        } as typeof original;
      } else {
        const requestDevice = GPUAdapter.prototype.requestDevice;
        GPUAdapter.prototype.requestDevice = async function (...args) {
          const device = await Reflect.apply(requestDevice, this, args);
          Object.assign(window, { frameDevice: device });
          return device;
        };
        const original = GPUQueue.prototype.submit;
        GPUQueue.prototype.submit = function (...args) {
          counts.draws++;
          return Reflect.apply(original, this, args);
        };
      }
    }, renderer);
    await page.goto('/replays');
    const map = page.getByTestId('map-viewer');
    await expect(map).toHaveAttribute('data-renderer', renderer);
    await expect
      .poll(() =>
        page.evaluate(
          () => (window as unknown as { frameCounts: { draws: number } }).frameCounts.draws,
        ),
      )
      .toBeGreaterThan(0);
    const canvas = page.getByTestId(
      renderer === 'Canvas' ? 'canvas-map-fallback' : 'webgpu-map-canvas',
    );
    const settle = () =>
      page.evaluate(async () => {
        await new Promise(requestAnimationFrame);
        await new Promise(requestAnimationFrame);
      });
    await map.focus();
    await settle();
    const initial = await canvas.screenshot();
    const result = await page.evaluate(async () => {
      const counts = (window as unknown as { frameCounts: { draws: number } }).frameCounts;
      const map = document.querySelector('[data-testid="map-viewer"]')!;
      const before = counts.draws;
      const started = performance.now();
      for (let i = 0; i < 200; i++)
        map.dispatchEvent(
          new KeyboardEvent('keydown', {
            key: i === 199 || i % 2 === 0 ? 'ArrowRight' : 'ArrowLeft',
            bubbles: true,
          }),
        );
      const immediate = counts.draws - before;
      const handlerMs = performance.now() - started;
      await new Promise(requestAnimationFrame);
      await new Promise(requestAnimationFrame);
      const after = counts.draws;
      const settledMs = performance.now() - started;
      await new Promise(requestAnimationFrame);
      await new Promise(requestAnimationFrame);
      return {
        immediate,
        burst: after - before,
        idle: counts.draws - after,
        handlerMs,
        settledMs,
        userAgent: navigator.userAgent,
        dpr: devicePixelRatio,
      };
    });
    await testInfo.attach('frame-burst.json', {
      body: JSON.stringify({ renderer, ...result }, null, 2),
      contentType: 'application/json',
    });
    expect(result).toMatchObject({ immediate: 0, burst: 1, idle: 0 });
    await expect(map).toHaveAttribute('data-camera', '{"x":-24,"y":0,"zoom":1}');
    expect((await canvas.screenshot()).equals(initial)).toBe(false);
    await map.press('0');
    await settle();
    expect((await canvas.screenshot()).equals(initial)).toBe(true);
    await page.getByLabel('Step forward', { exact: true }).click();
    await settle();
    const replay = await canvas.screenshot();
    expect(replay.equals(initial)).toBe(false);
    await page.getByLabel('Toggle theme').click();
    await settle();
    expect((await canvas.screenshot()).equals(replay)).toBe(false);
    const themed = await canvas.screenshot();
    await page.getByLabel('Example', { exact: true }).selectOption({ index: 1 });
    await settle();
    expect((await canvas.screenshot()).equals(themed)).toBe(false);
    const sample = await canvas.screenshot();
    await page.getByLabel('Outer grid', { exact: true }).selectOption({ index: 1 });
    await settle();
    expect((await canvas.screenshot()).equals(sample)).toBe(false);
    const width = await canvas.evaluate((el) => (el as HTMLCanvasElement).width);
    await page.setViewportSize({ width: 1000, height: 720 });
    await settle();
    expect(await canvas.evaluate((el) => (el as HTMLCanvasElement).width)).toBeLessThan(width);
    await expect(map).toHaveAttribute('data-renderer', renderer);
    await page.evaluate(() => {
      document
        .querySelector('[data-testid="map-viewer"]')!
        .dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
      (document.querySelector('a[href="/solvers"]') as HTMLAnchorElement).click();
    });
    await expect(page).toHaveURL(/\/solvers$/);
    await expect(page.getByTestId('map-viewer')).toHaveAttribute('data-renderer', renderer);
    await settle();
    expect(errors).toEqual([]);
    if (renderer === 'Canvas')
      expect(
        await page.evaluate(
          () => (window as unknown as { frameCounts: { detached: number } }).frameCounts.detached,
        ),
      ).toBe(0);
    else {
      await page.evaluate(() => {
        (window as unknown as { frameDevice: GPUDevice }).frameDevice.destroy();
      });
      await expect(page.getByTestId('map-viewer')).toHaveAttribute('data-renderer', 'Canvas');
      await settle();
      const fallback = page.getByTestId('canvas-map-fallback');
      await expect(fallback).toHaveCSS('visibility', 'visible');
      expect(
        await fallback.evaluate((el) => {
          const canvas = el as HTMLCanvasElement;
          const data = canvas
            .getContext('2d')!
            .getImageData(0, 0, canvas.width, canvas.height).data;
          for (let i = 4; i < data.length; i += 4)
            if (data[i] !== data[0] || data[i + 1] !== data[1] || data[i + 2] !== data[2])
              return true;
          return false;
        }),
      ).toBe(true);
      const before = await fallback.screenshot();
      await page.getByTestId('map-viewer').press('+');
      await settle();
      expect((await fallback.screenshot()).equals(before)).toBe(false);
      expect(errors).toEqual([]);
    }
  });
}
