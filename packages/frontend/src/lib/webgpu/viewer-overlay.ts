import { glyphAtlas, markNames, GLYPH_SIZE, HEX_INSET } from './hex-assets';
import { cssColor, palette, sceneTransform, type Camera, type ViewerScene } from './viewer-scene';

/** One transparent layer for both backends; selection never redraws the terrain. */
export function drawViewerOverlay(
  canvas: HTMLCanvasElement,
  scene: ViewerScene,
  camera: Camera,
  selected: number | null,
) {
  const rect = canvas.getBoundingClientRect();
  if (!rect.width || !rect.height) return;
  const width = Math.round(rect.width * devicePixelRatio),
    height = Math.round(rect.height * devicePixelRatio);
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) canvas.height = height;
  const ctx = canvas.getContext('2d')!;
  ctx.resetTransform();
  ctx.clearRect(0, 0, width, height);
  const transform = sceneTransform(scene, rect.width, rect.height, camera),
    colors = palette();
  ctx.setTransform(
    transform.scale * devicePixelRatio,
    0,
    0,
    transform.scale * devicePixelRatio,
    transform.x * devicePixelRatio,
    transform.y * devicePixelRatio,
  );
  for (const mark of scene.marks) {
    const progress = mark.progress;
    if (!progress) continue;
    const cell = scene.cells[mark.cell]!;
    const arc = (amount: number) => {
      ctx.beginPath();
      ctx.arc(cell.x, cell.y, 13.8, -Math.PI / 2, -Math.PI / 2 + amount * Math.PI * 2);
      ctx.stroke();
    };
    if (progress.value === null) {
      ctx.strokeStyle = cssColor(colors('--fgColor-muted'));
      ctx.globalAlpha = 0.6;
      ctx.lineWidth = 1.8;
      ctx.setLineDash([2, 3]);
      arc(1);
      ctx.setLineDash([]);
      ctx.globalAlpha = 1;
    } else {
      ctx.strokeStyle = '#ffffff';
      ctx.lineWidth = 4.2;
      arc(1);
      ctx.strokeStyle = cssColor(colors('--borderColor-default'));
      ctx.lineWidth = 2.8;
      arc(1);
      const ratio = Math.max(0, Math.min(1, progress.value / progress.max));
      ctx.strokeStyle = cssColor(
        colors(
          ratio <= 0.2
            ? '--fgColor-danger'
            : ratio <= 0.5
              ? '--fgColor-attention'
              : '--fgColor-accent',
        ),
      );
      arc(ratio);
    }
  }
  // Draw the original cased icon over its gauge, using the same atlas and world size.
  const iconSize = (44 * HEX_INSET * 23) / 30;
  for (const mark of scene.marks) {
    if (!mark.progress) continue;
    const cell = scene.cells[mark.cell]!;
    ctx.drawImage(
      glyphAtlas(),
      markNames.indexOf(mark.mark) * GLYPH_SIZE,
      GLYPH_SIZE,
      GLYPH_SIZE,
      GLYPH_SIZE,
      cell.x - iconSize / 2,
      cell.y - iconSize / 2,
      iconSize,
      iconSize,
    );
  }
  const cell = selected === null ? undefined : scene.cells[selected];
  if (cell) {
    const scale = 23 / 30;
    ctx.beginPath();
    [
      [0, -30],
      [26, -15],
      [26, 15],
      [0, 30],
      [-26, 15],
      [-26, -15],
    ].forEach(([x, y], index) => {
      const px = cell.x + x! * scale,
        py = cell.y + y! * scale;
      if (index === 0) ctx.moveTo(px, py);
      else ctx.lineTo(px, py);
    });
    ctx.closePath();
    // Fill the gap up to the outer edge of the inset 1.6-unit black keyline.
    // A small overlap covers the antialiasing fringe of the rasterized atlas.
    ctx.lineWidth = 2 * (26 * (1 - HEX_INSET) - 0.8 * HEX_INSET) * scale + 0.2;
    ctx.lineJoin = 'miter';
    ctx.strokeStyle = cssColor(colors('--fgColor-accent'));
    ctx.stroke();
  }
}
