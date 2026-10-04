import { expect, test } from '@playwright/test';

test('failed canvas configuration releases every GPU device and keeps Canvas usable', async ({
  page,
}) => {
  await page.addInitScript(() => {
    const counts = { created: 0, destroyed: 0 };
    Object.assign(window, { gpuOwnershipCounts: counts });
    const requestDevice = GPUAdapter.prototype.requestDevice;
    GPUAdapter.prototype.requestDevice = async function (...args) {
      const device = await Reflect.apply(requestDevice, this, args);
      counts.created++;
      const destroy = device.destroy.bind(device);
      device.destroy = () => {
        counts.destroyed++;
        destroy();
      };
      return device;
    };
    GPUCanvasContext.prototype.configure = () => {
      throw new Error('injected-canvas-configuration-failure');
    };
  });
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/replays');
  const check = async (expected: number) => {
    await expect(page.getByTestId('webgpu-map-fallback')).toContainText(
      'injected-canvas-configuration-failure',
    );
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as unknown as { gpuOwnershipCounts: { created: number; destroyed: number } })
              .gpuOwnershipCounts,
        ),
      )
      .toEqual({ created: expected, destroyed: expected });
    await expect(page.getByTestId('map-viewer')).toHaveAttribute('data-renderer', 'Canvas');
    const canvas = page.getByTestId('canvas-map-fallback');
    await expect(canvas).toHaveCSS('visibility', 'visible');
    await expect
      .poll(() =>
        canvas.evaluate((el) => {
          const canvas = el as HTMLCanvasElement;
          const data = canvas
            .getContext('2d')!
            .getImageData(0, 0, canvas.width, canvas.height).data;
          for (let i = 4; i < data.length; i += 4)
            if (data[i] !== data[0] || data[i + 1] !== data[1] || data[i + 2] !== data[2])
              return true;
          return false;
        }),
      )
      .toBe(true);
  };
  await check(1);
  await page.getByLabel('Step forward', { exact: true }).click();
  await expect(page.getByTestId('replay-current-step')).toHaveText('2 / 5');
  await page.getByTestId('map-viewer').press('ArrowRight');
  await expect(page.getByTestId('map-viewer')).toHaveAttribute(
    'data-camera',
    '{"x":-12,"y":0,"zoom":1}',
  );
  await page.locator('a[href="/solvers"]').click();
  await check(2);
  expect(errors).toEqual([]);
});
