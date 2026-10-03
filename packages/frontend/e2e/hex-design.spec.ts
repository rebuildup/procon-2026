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
      await scene.loadHexAssets();
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

test('composed hex matches proposal and Canvas; camera frames reuse uploads', async ({ page }) => {
  await page.goto('/solvers');
  await expect(page.getByTestId('map-viewer')).toHaveAttribute('data-renderer', 'WebGPU', {
    timeout: 10000,
  });
  const report = await page.evaluate(async () => {
    const assetsPath = '/src/lib/webgpu/hex-assets.ts',
      scenePath = '/src/lib/webgpu/viewer-scene.ts',
      layerPath = '/src/lib/webgpu/viewer-layer.ts';
    const assets = await import(/* @vite-ignore */ assetsPath),
      scene = await import(/* @vite-ignore */ scenePath),
      renderer = await import(/* @vite-ignore */ layerPath);
    await assets.loadHexAssets();
    document.body.replaceChildren();
    const coords = [
      [52, 44],
      [132, 44],
      [212, 44],
      [292, 44],
      [92, 102],
      [252, 102],
    ];
    const names = ['s0', 's1', 's2', 's3', 's5', 's4'],
      terrain = ['plain', 'road', 'water', 'plain', 'mountain', 'plain'];
    const scale = 23 / 30;
    const fixture = {
      cells: coords.map(([x, y], i) => ({
        x: x! * scale,
        y: y! * scale,
        terrain: terrain[i],
      })),
      marks: names.map((mark, cell) => ({ cell, mark })),
    };
    const camera = { x: -24, y: -24, zoom: 1 / scale };
    const make = (name: string) => {
      const title = document.createElement('h2');
      title.textContent = name;
      document.body.append(title);
      const c = document.createElement('canvas');
      c.width = 370;
      c.height = 265;
      c.style.width = '370px';
      c.style.height = '265px';
      c.style.display = 'block';
      document.body.append(c);
      return c;
    };
    const gpu = make('GPU'),
      layer = await renderer.createViewerLayer(gpu, () => {});
    layer.draw(fixture, camera);
    await layer.context.device.queue.onSubmittedWorkDone();
    const fallback = make('Canvas');
    scene.drawCanvasScene(fallback, fixture, camera);
    const reference = make('Canonical SVG');
    const structurePath = '/src/viewer-structure.json';
    const structure = (await import(/* @vite-ignore */ structurePath)).default;
    const body = structure.reference.slice(0, 5).map(assets.svgNode).join('');
    const image = new Image();
    const url = URL.createObjectURL(
      new Blob(
        [
          `<svg xmlns="http://www.w3.org/2000/svg" width="370" height="265" viewBox="0 0 370 265">${body}</svg>`,
        ],
        { type: 'image/svg+xml' },
      ),
    );
    image.src = url;
    await image.decode();
    const ctx = reference.getContext('2d')!;
    ctx.fillStyle = scene.cssColor(scene.palette()('--bgColor-muted'));
    ctx.fillRect(0, 0, 370, 265);
    ctx.drawImage(image, 0, 0);
    URL.revokeObjectURL(url);
    const timings = [];
    for (let i = 0; i < 60; i++) {
      const start = performance.now();
      layer.draw(fixture, { ...camera, x: camera.x + i * 0.01 });
      timings.push(performance.now() - start);
    }
    await layer.context.device.queue.onSubmittedWorkDone();
    const stats = layer.stats();
    const fixture48 = scene.fixtureScene(0);
    const timings48: number[] = [];
    for (let i = 0; i < 60; i++) {
      const start = performance.now();
      layer.draw(fixture48, { x: i * 0.01, y: 0, zoom: 1 });
      timings48.push(performance.now() - start);
    }
    await layer.context.device.queue.onSubmittedWorkDone();
    const uploads48 = layer.stats().uploads - stats.uploads;
    layer.draw(fixture, camera);
    await layer.context.device.queue.onSubmittedWorkDone();
    return {
      stats,
      fixture48: { uploadsAdded: uploads48, p95SubmitMs: timings48.sort((a, b) => a - b)[57] },
      p95SubmitMs: timings.sort((a, b) => a - b)[57],
    };
  });
  expect(report.fixture48.uploadsAdded).toBe(1);
  expect(report.stats.uploads).toBe(1);
  expect(report.stats.atlasUploads).toBe(1);
  expect(report.stats.draws).toBe(61);
  const screenshots = [];
  for (const canvas of await page.locator('canvas').all())
    screenshots.push(Array.from(await canvas.screenshot()));
  const differences = await page.evaluate(async (images) => {
    const data: Uint8ClampedArray[] = [];
    for (const bytes of images) {
      const bitmap = await createImageBitmap(
        new Blob([new Uint8Array(bytes)], { type: 'image/png' }),
      );
      const c = document.createElement('canvas');
      c.width = bitmap.width;
      c.height = bitmap.height;
      const ctx = c.getContext('2d')!;
      ctx.drawImage(bitmap, 0, 0);
      bitmap.close();
      data.push(ctx.getImageData(0, 0, c.width, c.height).data);
    }
    return [1, 2].map((index) => {
      let sum = 0;
      for (let i = 0; i < data[0]!.length; i++) sum += Math.abs(data[0]![i]! - data[index]![i]!);
      return sum / data[0]!.length;
    });
  }, screenshots);
  expect(differences[0]).toBeLessThan(2);
  expect(differences[1]).toBeLessThan(3);
  await page.screenshot({
    path: resolve(OUT_DIR, 'shared-renderer-comparison.png'),
    fullPage: true,
  });
  const { writeFileSync } = await import('node:fs');
  writeFileSync(
    resolve(OUT_DIR, 'renderer-performance.json'),
    JSON.stringify(
      {
        ...report,
        meanChannelError: differences,
        scope:
          '6 canonical hexes and 48-cell demo, 60 camera submissions each; CPU submission timing, not production GPU throughput',
      },
      null,
      2,
    ),
  );
});
