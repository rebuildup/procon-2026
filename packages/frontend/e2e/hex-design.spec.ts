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
      const fixture = scene.fixtureScene(0, family, 'figure2');
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
    const camera = { x: 0, y: 0, zoom: 1 };
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
    const transform = scene.sceneTransform(fixture, 370, 265, camera);
    const body = `<g transform="translate(${transform.x},${transform.y}) scale(${transform.scale * scale})">${structure.reference.slice(0, 5).map(assets.svgNode).join('')}</g>`;
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
    const fixture100 = scene.fixtureScene(0);
    const timings100: number[] = [];
    for (let i = 0; i < 60; i++) {
      const start = performance.now();
      layer.draw(fixture100, { x: i * 0.01, y: 0, zoom: 1 });
      timings100.push(performance.now() - start);
    }
    await layer.context.device.queue.onSubmittedWorkDone();
    const uploads100 = layer.stats().uploads - stats.uploads;
    layer.draw(fixture, camera);
    await layer.context.device.queue.onSubmittedWorkDone();
    return {
      stats,
      fixture100: { uploadsAdded: uploads100, p95SubmitMs: timings100.sort((a, b) => a - b)[57] },
      p95SubmitMs: timings.sort((a, b) => a - b)[57],
    };
  });
  expect(report.fixture100.uploadsAdded).toBe(1);
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
          '6 canonical hexes and 100-cell official example, 60 camera submissions each; CPU submission timing, not production GPU throughput',
      },
      null,
      2,
    ),
  );
});

test('adjacent hex borders survive reverse order and map stays centered with empty margin', async ({
  page,
}) => {
  await page.goto('/');
  await expect(page.getByTestId('map-viewer')).toHaveAttribute('data-renderer', 'WebGPU', {
    timeout: 10000,
  });
  await page.getByLabel('問題文の例').selectOption('figure2');
  await page.getByLabel('マップ外の空枠').selectOption('2');
  await expect(page.getByTestId('map-viewer')).toHaveAttribute(
    'data-camera',
    '{"x":0,"y":0,"zoom":1}',
  );
  await page.screenshot({ path: resolve(OUT_DIR, 'sample-figure2-padding.png'), fullPage: true });
  const result = await page.evaluate(async () => {
    const path = '/src/lib/webgpu/viewer-scene.ts';
    const scene = await import(/* @vite-ignore */ path);
    await scene.loadHexAssets();
    const canvas = document.createElement('canvas');
    canvas.style.width = '700px';
    canvas.style.height = '600px';
    document.body.replaceChildren(canvas);
    const fixture = scene.fixtureScene(0, 'ridge', 'figure1');
    scene.drawCanvasScene(canvas, fixture, { x: 0, y: 0, zoom: 1 });
    const ctx = canvas.getContext('2d')!;
    const first = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
    const center = fixture.cells[24],
      scale = 23 / 30;
    const edges = [
      [-13, -22.5],
      [13, -22.5],
      [26, 0],
      [13, 22.5],
      [-13, 22.5],
      [-26, 0],
    ].map(([x, y]) => {
      const p = scene.worldPoint(
        700,
        600,
        center.x + x! * scale,
        center.y + y! * scale,
        { x: 0, y: 0, zoom: 1 },
        fixture,
      );
      let ink = 0;
      for (let py = Math.floor(p.y) - 3; py <= Math.floor(p.y) + 3; py++)
        for (let px = Math.floor(p.x) - 3; px <= Math.floor(p.x) + 3; px++) {
          const i = (py * canvas.width + px) * 4;
          if (first[i]! < 80 && first[i + 1]! < 80 && first[i + 2]! < 80) ink++;
        }
      return ink;
    });
    scene.drawCanvasScene(
      canvas,
      { cells: [...fixture.cells].reverse(), marks: [] },
      { x: 0, y: 0, zoom: 1 },
    );
    const second = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
    let difference = 0;
    for (let i = 0; i < first.length; i++) difference += Math.abs(first[i]! - second[i]!);
    const padded = scene.fixtureScene(0, 'ridge', 'figure2', 2);
    scene.drawCanvasScene(canvas, padded, { x: 0, y: 0, zoom: 1 });
    const image = ctx.getImageData(0, 0, canvas.width, canvas.height).data,
      bg = image.slice(0, 3);
    let left = 700,
      right = 0,
      top = 600,
      bottom = 0;
    for (let y = 0; y < 600; y++)
      for (let x = 0; x < 700; x++) {
        const i = (y * 700 + x) * 4;
        if (bg.some((v, n) => Math.abs(v - image[i + n]!) > 10)) {
          left = Math.min(left, x);
          right = Math.max(right, x);
          top = Math.min(top, y);
          bottom = Math.max(bottom, y);
        }
      }
    return {
      edges,
      meanOrderDifference: difference / first.length,
      center: [(left + right) / 2, (top + bottom) / 2],
      empty: padded.cells.filter((cell: { terrain: string }) => cell.terrain === 'empty').length,
      figure1: fixture.cells.length,
      figure2: scene
        .fixtureScene(0, 'ridge', 'figure2')
        .marks.map((m: { cell: number; mark: string }) => [m.cell, m.mark]),
      parity: fixture.cells[0].x > fixture.cells[10].x,
    };
  });
  expect(result.figure1).toBe(100);
  expect(result.figure2).toEqual([
    [5, 's0'],
    [8, 's2'],
    [15, 's1'],
    [18, 's0'],
  ]);
  expect(result.parity).toBe(true);
  result.edges.forEach((count) => expect(count).toBeGreaterThan(2));
  expect(result.meanOrderDifference).toBeLessThan(0.5);
  expect(Math.abs(result.center[0]! - 350)).toBeLessThan(2);
  expect(Math.abs(result.center[1]! - 300)).toBeLessThan(2);
  expect(result.empty).toBeGreaterThan(0);
  await page.screenshot({ path: resolve(OUT_DIR, 'empty-margin-2.png'), fullPage: true });
});
