import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { expect, test } from '@playwright/test';
import { OUT_DIR } from './recording';

const baseline = 'bc8ae3b5641c271a604d770441eeea128c9fff11';
const readBefore = (name: string) =>
  JSON.parse(
    execFileSync('git', ['show', `${baseline}:packages/frontend/src/${name}.json`], {
      encoding: 'utf8',
    }),
  );

test('large silhouettes retain contrast and separation on every terrain', async ({ page }) => {
  await page.goto('/');
  const report = await page.evaluate(
    async ({ beforeGlyphs, beforeStructure }) => {
      const path = '/src/lib/webgpu/hex-assets.ts';
      const assets = await import(/* @vite-ignore */ path);
      await assets.loadHexAssets();
      document.body.replaceChildren();
      document.body.style.cssText =
        'background:white;color:#212428;padding:24px;font:14px sans-serif';
      const marks = ['s0', 's1', 's2', 's3', 's4', 's5', 'patrol', 'supply'];
      const labels = ['系列A', '系列B', '系列C', '系列D', '系列E', '系列F', '巡回車', '補給車'];
      const terrainNames = ['平地', '山地', '池', '道路'];
      const terrainIndices = [0, 4, 2, 1];
      const panels = document.createElement('div');
      panels.style.cssText = 'display:flex;gap:32px';
      document.body.append(panels);
      const measurements: { terrain: string; mark: string; before: number; after: number }[] = [];
      const signatures: string[] = [];
      for (const version of ['Before', 'After']) {
        const panel = document.createElement('div');
        panels.append(panel);
        const heading = document.createElement('h2');
        heading.textContent =
          version === 'Before'
            ? 'Before — 32unit / dense texture'
            : 'After — 40unit / quiet texture';
        panel.append(heading);
        const guide = document.createElement('div');
        guide.style.cssText = 'display:grid;grid-template-columns:repeat(8,64px)';
        panel.append(guide);
        for (const label of labels) {
          const text = document.createElement('span');
          text.textContent = label;
          text.style.textAlign = 'center';
          guide.append(text);
        }
        for (const [row, terrain] of ['plain', 'mountain', 'water', 'road'].entries()) {
          const label = document.createElement('h3');
          label.textContent = terrainNames[row]!;
          panel.append(label);
          const c = document.createElement('canvas');
          c.width = 512;
          c.height = 64;
          panel.append(c);
          const ctx = c.getContext('2d')!;
          for (const [col, mark] of marks.entries()) {
            if (version === 'After') {
              for (const index of [
                assets.HALO_TILE,
                assets.tileIndex(terrain, mark),
                assets.INK_TILE,
              ]) {
                const origin = assets.tileSource(index);
                ctx.drawImage(
                  assets.tileAtlas(),
                  origin.x,
                  origin.y,
                  assets.TILE_SIZE,
                  assets.TILE_SIZE,
                  col * 64,
                  0,
                  64,
                  64,
                );
              }
            } else {
              const hex = beforeStructure.hex[terrainIndices[row]!];
              const [x, y] = hex.center;
              let body = `<g transform="translate(${-x},${-y})">${assets.svgNode(hex.halo)}${assets.svgNode(hex.fill)}</g>`;
              const series = beforeStructure.series[mark];
              if (series)
                body += `<g transform="translate(${-series.center[0]},${-series.center[1]})">${series.layers.map(assets.svgNode).join('')}</g>`;
              else
                body += `<g transform="translate(-16,-16) scale(2)">${beforeGlyphs[mark].map(assets.svgNode).join('')}</g>`;
              const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="-34 -34 68 68">${assets.svgNode(beforeStructure.defs)}<g transform="scale(.93)">${body}</g></svg>`;
              const image = new Image();
              image.src = 'data:image/svg+xml,' + encodeURIComponent(svg);
              await image.decode();
              ctx.drawImage(image, col * 64, 0);
            }
            const data = ctx.getImageData(col * 64, 0, 64, 64).data;
            let chromaticArea = 0;
            // Central colour area: isolated from terrain, edge frames and white casing.
            for (let y = 8; y < 56; y++)
              for (let x = 8; x < 56; x++) {
                const i = (y * 64 + x) * 4;
                if (
                  data[i + 3]! > 200 &&
                  Math.max(data[i]!, data[i + 1]!, data[i + 2]!) -
                    Math.min(data[i]!, data[i + 1]!, data[i + 2]!) >
                    60
                )
                  chromaticArea++;
              }
            let metric = measurements.find((m) => m.terrain === terrain && m.mark === mark);
            if (!metric) {
              metric = { terrain, mark, before: 0, after: 0 };
              measurements.push(metric);
            }
            metric[version === 'Before' ? 'before' : 'after'] = chromaticArea;
            if (version === 'After' && row === 1) signatures.push(Array.from(data).join(','));
          }
        }
      }
      const rgb = (hex: string) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
      const luminance = (hex: string) =>
        rgb(hex)
          .map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4))
          .reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i]!, 0);
      const contrast = (a: string, b: string) =>
        (Math.max(luminance(a), luminance(b)) + 0.05) /
        (Math.min(luminance(a), luminance(b)) + 0.05);
      const glyphPath = '/src/viewer-marks.json';
      const glyphs = (await import(/* @vite-ignore */ glyphPath)).default;
      const fillColours = (node: {
        attrs: Record<string, string>;
        children: unknown[];
      }): string[] =>
        [
          node.attrs.fill,
          ...node.children.flatMap((child) => fillColours(child as typeof node)),
        ].filter(
          (value): value is string => !!value && value.startsWith('#') && value !== '#212428',
        );
      const seriesColours = marks
        .slice(0, 6)
        .map((mark) => glyphs[mark].flatMap(fillColours)[0] as string);
      const structurePath = '/src/viewer-structure.json';
      const structure = (await import(/* @vite-ignore */ structurePath)).default;
      const terrainContrasts = structure.defs.children.map(
        (pattern: {
          attrs: Record<string, string>;
          children: { attrs: Record<string, string> }[];
        }) => ({
          terrain: pattern.attrs.id,
          ratio: contrast(
            pattern.children[0]!.attrs.fill!,
            pattern.children[1]!.attrs.stroke ?? pattern.children[1]!.attrs.fill!,
          ),
        }),
      );
      return {
        terrainContrasts,
        measurements,
        distinct: new Set(signatures).size,
        boundaryContrast: contrast('#212428', '#ffffff'),
        seriesToKeyline: seriesColours.map((colour) => ({
          colour,
          ratio: contrast(colour, '#212428'),
        })),
      };
    },
    { beforeGlyphs: readBefore('viewer-marks'), beforeStructure: readBefore('viewer-structure') },
  );
  expect(report.distinct).toBe(8);
  expect(report.boundaryContrast).toBeGreaterThan(3);
  for (const terrain of report.terrainContrasts) expect(terrain.ratio).toBeGreaterThanOrEqual(3);
  for (const colour of report.seriesToKeyline) expect(colour.ratio).toBeGreaterThanOrEqual(3);
  for (const item of report.measurements.filter(
    (m) => m.terrain === 'mountain' && m.mark.startsWith('s'),
  ))
    expect(item.after, item.mark).toBeGreaterThan(item.before);
  writeFileSync(resolve(OUT_DIR, 'board-contrast.json'), JSON.stringify(report, null, 2) + '\n');
  const cdp = await page.context().newCDPSession(page);
  for (const type of [
    'none',
    'achromatopsia',
    'protanopia',
    'deuteranopia',
    'tritanopia',
  ] as const) {
    await cdp.send('Emulation.setEmulatedVisionDeficiency', { type });
    await page.screenshot({ path: resolve(OUT_DIR, `board-${type}.png`), fullPage: true });
  }
});
