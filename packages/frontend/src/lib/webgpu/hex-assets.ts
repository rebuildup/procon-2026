import glyphs from '@/viewer-marks.json';
import structure from '@/viewer-structure.json';

export type Mark = keyof typeof glyphs;
export type Terrain = 'plain' | 'mountain' | 'water' | 'road' | 'empty';
export const markNames = Object.keys(glyphs) as Mark[];
export const GLYPH_SIZE = 64;
export const TILE_SIZE = 128;
export const TILE_COLUMNS = 16;
export const TILE_VIEW_SIZE = 68;
/** Leave white space between independently outlined neighboring cells. */
export const HEX_INSET = 0.93;
export const TILE_WORLD_SIZE = (TILE_VIEW_SIZE * 23) / 30;
export const terrains: Terrain[] = ['plain', 'mountain', 'water', 'road', 'empty'];
export const HALO_TILE = terrains.length * (markNames.length + 1);
export const INK_TILE = HALO_TILE + 1;
interface Node {
  tag: string;
  attrs: Record<string, string>;
  children: Node[];
}
const escape = (value: string) =>
  value.replaceAll('&', '&amp;').replaceAll('"', '&quot;').replaceAll('<', '&lt;');
export function svgNode(node: Node): string {
  return `<${node.tag} ${Object.entries(node.attrs)
    .map(([key, value]) => `${key}="${escape(value)}"`)
    .join(' ')}>${node.children.map(svgNode).join('')}</${node.tag}>`;
}
export function referenceSvg(): string {
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${structure.viewBox}" width="420" height="168">${structure.reference.map(svgNode).join('')}</svg>`;
}
/** A quiet white separation around the same glyph, for every terrain and family. */
function markerCasing(node: Node): Node {
  return {
    ...node,
    attrs: {
      ...node.attrs,
      stroke: '#ffffff',
      'stroke-width': '2.2',
      fill: node.attrs.fill === 'none' ? 'none' : '#ffffff',
    },
    children: node.children.map(markerCasing),
  };
}
/** Full hex: original pattern, halo, keyline and one centered domain occupant. */
export function tileSvgContent(terrain: Terrain, mark?: Mark): string {
  if (terrain === 'empty') return '';
  const template = structure.hex[{ plain: 0, mountain: 4, water: 2, road: 1 }[terrain]]!;
  const [x, y] = template.center;
  let content = `<g transform="translate(${-x!},${-y!})">${svgNode({ ...template.fill, attrs: { ...template.fill.attrs, stroke: 'none' } })}</g>`;
  if (mark) {
    const pattern = template.fill.children[0]!.attrs['data-fill-pattern']!;
    const id = pattern.slice(5, -1);
    const base = structure.defs.children.find((node) => node.attrs.id === id)!.children[0]!.attrs
      .fill!;
    content += `<circle cx="0" cy="0" r="19.2" fill="${base}"/>`;
    const shape = glyphs[mark].map(svgNode).join('');
    const casing = glyphs[mark].map((node) => svgNode(markerCasing(node))).join('');
    content += `<g transform="translate(-20,-20) scale(2.5)">${casing}${shape}</g>`;
  }
  return content;
}
export function tileIndex(terrain: Terrain, mark?: Mark): number {
  return (
    terrains.indexOf(terrain) * (markNames.length + 1) + (mark ? markNames.indexOf(mark) + 1 : 0)
  );
}
export function tileSource(index: number) {
  return { x: (index % TILE_COLUMNS) * TILE_SIZE, y: Math.floor(index / TILE_COLUMNS) * TILE_SIZE };
}
let loaded: { tiles: OffscreenCanvas; glyphs: OffscreenCanvas } | undefined;
let pending: Promise<void> | undefined;
async function rasterize(width: number, height: number, body: string): Promise<OffscreenCanvas> {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">${body}</svg>`;
  const url = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml' }));
  try {
    const image = new Image();
    image.src = url;
    await image.decode();
    const canvas = new OffscreenCanvas(width, height);
    canvas.getContext('2d')!.drawImage(image, 0, 0);
    return canvas;
  } finally {
    URL.revokeObjectURL(url);
  }
}
/** Exactly two SVG rasterizations, shared across routes and both renderers. */
export function loadHexAssets(): Promise<void> {
  return (pending ??= (async () => {
    const count = INK_TILE + 1;
    let tiles = svgNode(structure.defs);
    for (const terrain of terrains)
      for (const mark of [undefined, ...markNames]) {
        const index = tileIndex(terrain, mark),
          origin = tileSource(index);
        tiles += `<g transform="translate(${origin.x + TILE_SIZE / 2},${origin.y + TILE_SIZE / 2}) scale(${TILE_SIZE / TILE_VIEW_SIZE})"><g transform="scale(${HEX_INSET})">${tileSvgContent(terrain, mark)}</g></g>`;
      }
    for (const [index, stroke, width] of [
      [HALO_TILE, '#ffffff', '4.5'],
      [INK_TILE, '#212428', '1.6'],
    ] as const) {
      const origin = tileSource(index),
        hex = structure.hex[0]!;
      const outline = {
        ...hex.fill,
        attrs: { ...hex.fill.attrs, stroke, 'stroke-width': width },
        children: hex.fill.children.map((child) => ({
          ...child,
          attrs: { ...child.attrs, fill: 'none' },
        })),
      };
      tiles += `<g transform="translate(${origin.x + TILE_SIZE / 2},${origin.y + TILE_SIZE / 2}) scale(${TILE_SIZE / TILE_VIEW_SIZE})"><g transform="scale(${HEX_INSET})"><g transform="translate(-52,-44)">${svgNode(outline)}</g></g></g>`;
    }
    const glyphBody = markNames
      .map(
        (name, index) =>
          `<g transform="translate(${index * GLYPH_SIZE},0) scale(${GLYPH_SIZE / 16})">${glyphs[name].map(svgNode).join('')}</g>`,
      )
      .join('');
    const [tileAtlas, glyphAtlas] = await Promise.all([
      rasterize(TILE_COLUMNS * TILE_SIZE, Math.ceil(count / TILE_COLUMNS) * TILE_SIZE, tiles),
      rasterize(markNames.length * GLYPH_SIZE, GLYPH_SIZE, glyphBody),
    ]);
    loaded = { tiles: tileAtlas, glyphs: glyphAtlas };
  })());
}
export function tileAtlas(): OffscreenCanvas {
  if (!loaded) throw new Error('Hex assets not loaded');
  return loaded.tiles;
}
export function glyphAtlas(): OffscreenCanvas {
  if (!loaded) throw new Error('Hex assets not loaded');
  return loaded.glyphs;
}
