import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, test } from '@playwright/test';
import { OUT_DIR } from './recording';

const before = JSON.parse(
  execFileSync(
    'git',
    ['show', 'bc8ae3b5641c271a604d770441eeea128c9fff11:packages/frontend/src/viewer-marks.json'],
    { encoding: 'utf8' },
  ),
);

test('all domain icons stay centered and unclipped at production sizes', async ({ page }) => {
  await page.goto('/');
  const metrics = await page.evaluate(async (before) => {
    const path = '/src/lib/webgpu/hex-assets.ts';
    const assets = await import(/* @vite-ignore */ path);
    await assets.loadHexAssets();
    document.body.replaceChildren();
    document.body.style.cssText =
      'background:white;color:#212428;padding:24px;font:14px sans-serif';
    const roles = [
      'plain',
      'mountain',
      'water',
      'road',
      'patrol',
      'supply',
      's0',
      's1',
      's2',
      's3',
      's4',
      's5',
    ];
    const labels = [
      '平地',
      '山地',
      '池',
      '道路',
      '巡回車',
      '補給車',
      '系列A',
      '系列B',
      '系列C',
      '系列D',
      '系列E',
      '系列F',
    ];
    const results = [];
    for (const family of ['ridge', 'peak', 'terrace']) {
      const h = document.createElement('h2');
      h.textContent = `${family} — 15 / 24 / 48px`;
      document.body.append(h);
      for (const version of ['Before', 'After']) {
        const title = document.createElement('h3');
        title.textContent = version;
        document.body.append(title);
        const row = document.createElement('div');
        row.style.cssText = 'display:grid;grid-template-columns:repeat(12,1fr);gap:8px';
        document.body.append(row);
        for (const [index, role] of roles.entries()) {
          const key = family === 'ridge' ? role : `${family}-${role}`;
          const item = document.createElement('div');
          item.style.cssText = 'display:flex;flex-direction:column;align-items:center;gap:8px';
          row.append(item);
          const label = document.createElement('span');
          label.textContent = labels[index]!;
          item.append(label);
          let image: HTMLImageElement | undefined;
          if (version === 'Before') {
            image = new Image();
            image.src =
              'data:image/svg+xml,' +
              encodeURIComponent(
                `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="256" height="256">${before[key].map(assets.svgNode).join('')}</svg>`,
              );
            await image.decode();
          }
          for (const size of [15, 24, 48]) {
            const c = document.createElement('canvas');
            c.width = c.height = size;
            item.append(c);
            const ctx = c.getContext('2d')!;
            if (image) ctx.drawImage(image, 0, 0, size, size);
            else
              ctx.drawImage(
                assets.glyphAtlas(),
                assets.markNames.indexOf(key) * assets.GLYPH_SIZE,
                0,
                assets.GLYPH_SIZE,
                assets.GLYPH_SIZE,
                0,
                0,
                size,
                size,
              );
          }
          if (version === 'After') {
            const c = document.createElement('canvas');
            c.width = c.height = 64;
            const ctx = c.getContext('2d')!;
            ctx.drawImage(
              assets.glyphAtlas(),
              assets.markNames.indexOf(key) * 64,
              0,
              64,
              64,
              0,
              0,
              64,
              64,
            );
            const pixels = ctx.getImageData(0, 0, 64, 64).data;
            let mass = 0,
              mx = 0,
              my = 0,
              minX = 64,
              minY = 64,
              maxX = 0,
              maxY = 0;
            for (let y = 0; y < 64; y++)
              for (let x = 0; x < 64; x++) {
                const a = pixels[(y * 64 + x) * 4 + 3]! / 255;
                mass += a;
                mx += (x + 0.5) * a;
                my += (y + 0.5) * a;
                if (a > 0.1) {
                  minX = Math.min(minX, x);
                  minY = Math.min(minY, y);
                  maxX = Math.max(maxX, x + 1);
                  maxY = Math.max(maxY, y + 1);
                }
              }
            results.push({
              key,
              centroid: [mx / mass / 4, my / mass / 4],
              bbox: [minX / 4, minY / 4, maxX / 4, maxY / 4],
              area: mass / 16,
            });
          }
        }
      }
    }
    return results;
  }, before);
  expect(metrics).toHaveLength(36);
  for (const mark of metrics) {
    expect(Math.abs(mark.centroid[0]! - 8), mark.key).toBeLessThan(0.2);
    expect(Math.abs(mark.centroid[1]! - 8), mark.key).toBeLessThan(0.2);
    expect(
      Math.min(mark.bbox[0]!, mark.bbox[1]!, 16 - mark.bbox[2]!, 16 - mark.bbox[3]!),
      mark.key,
    ).toBeGreaterThanOrEqual(1.25);
  }
  writeFileSync(resolve(OUT_DIR, 'icon-geometry.json'), JSON.stringify(metrics, null, 2) + '\n');
  await page.screenshot({ path: resolve(OUT_DIR, 'icons-before-after.png'), fullPage: true });
});
