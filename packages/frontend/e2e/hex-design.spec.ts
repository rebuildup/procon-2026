import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';

import { expect, test } from '@playwright/test';

import { OUT_DIR } from './recording';

test('all 36 glyphs preserve the canonical proposal definition', () => {
  execFileSync('python3', [resolve('../../scripts/extract-viewer-marks.py'), '--check']);
});

test('three families render in GPU and six adjacent series remain distinct at 15px', async ({
  page,
}) => {
  await page.goto('/solvers');
  await expect(page.getByTestId('map-viewer')).toHaveAttribute('data-renderer', 'WebGPU', {
    timeout: 10000,
  });
  const result = await page.evaluate(async () => {
    const scenePath = '/src/lib/webgpu/viewer-scene.ts';
    const layerPath = '/src/lib/webgpu/viewer-layer.ts';
    const scene = await import(/* @vite-ignore */ scenePath);
    const { createViewerLayer } = await import(/* @vite-ignore */ layerPath);
    const failures: string[] = [];
    document.body.replaceChildren();
    document.body.style.background = 'white';
    const previews: { family: string; distinct: number; ink: number }[] = [];
    for (const family of ['ridge', 'peak', 'terrace']) {
      const title = document.createElement('h2');
      title.textContent = family;
      document.body.append(title);
      const canvas = document.createElement('canvas');
      canvas.style.width = '640px';
      canvas.style.height = '320px';
      document.body.append(canvas);
      const layer = await createViewerLayer(canvas, (reason: string) => failures.push(reason));
      const fixture = scene.fixtureScene(0, family);
      layer.draw(fixture, { x: 0, y: 0, zoom: 1 });
      await layer.context.device.queue.onSubmittedWorkDone();
      const row = document.createElement('canvas');
      row.width = 90;
      row.height = 15;
      row.style.display = 'block';
      document.body.append(row);
      const ctx = row.getContext('2d')!;
      const signatures = [];
      for (let n = 0; n < 6; n++) {
        const key = family === 'ridge' ? `s${n}` : `${family}-s${n}`;
        ctx.drawImage(
          scene.glyphAtlas(),
          scene.markNames.indexOf(key) * scene.GLYPH_SIZE,
          0,
          scene.GLYPH_SIZE,
          scene.GLYPH_SIZE,
          n * 15,
          0,
          15,
          15,
        );
        signatures.push(Array.from(ctx.getImageData(n * 15, 0, 15, 15).data).join(','));
      }
      const data = ctx.getImageData(0, 0, 90, 15).data;
      let ink = 0;
      for (let i = 0; i < data.length; i += 4)
        if (data[i]! < 80 && data[i + 1]! < 80 && data[i + 2]! < 80 && data[i + 3]! > 200) ink++;
      previews.push({ family, distinct: new Set(signatures).size, ink });
    }
    return { failures, previews };
  });
  expect(result.failures).toEqual([]);
  for (const preview of result.previews) {
    expect(preview.distinct).toBe(6);
    expect(preview.ink).toBeGreaterThan(10);
  }
  for (const index of [0, 2, 4]) {
    const bytes = Array.from(await page.locator('canvas').nth(index).screenshot());
    const ink = await page.evaluate(async (bytes) => {
      const bitmap = await createImageBitmap(
        new Blob([new Uint8Array(bytes)], { type: 'image/png' }),
      );
      const c = document.createElement('canvas');
      c.width = bitmap.width;
      c.height = bitmap.height;
      const ctx = c.getContext('2d')!;
      ctx.drawImage(bitmap, 0, 0);
      bitmap.close();
      const data = ctx.getImageData(0, 0, c.width, c.height).data;
      let count = 0;
      for (let i = 0; i < data.length; i += 4)
        if (data[i]! < 80 && data[i + 1]! < 80 && data[i + 2]! < 80) count++;
      return count;
    }, bytes);
    expect(ink).toBeGreaterThan(100);
  }
  await page.screenshot({ path: resolve(OUT_DIR, 'families-15px.png'), fullPage: true });
});
