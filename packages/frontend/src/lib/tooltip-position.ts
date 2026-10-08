export interface TooltipPosition {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Prefer the side of the hex, then above/below on narrow screens. */
export function tooltipPosition(
  viewportWidth: number,
  viewportHeight: number,
  anchorX: number,
  anchorY: number,
  radius: number,
): TooltipPosition {
  const margin = 8,
    gap = 12;
  const width = Math.max(1, Math.min(320, viewportWidth - margin * 2));
  const height = Math.max(1, Math.min(224, viewportHeight - margin * 2));
  const right = anchorX + radius + gap,
    left = anchorX - radius - gap - width;
  const below = anchorY + radius + gap,
    above = anchorY - radius - gap - height;
  let x = anchorX - width / 2,
    y = anchorY - height / 2;
  if (right + width <= viewportWidth - margin) x = right;
  else if (left >= margin) x = left;
  else if (below + height <= viewportHeight - margin) y = below;
  else if (above >= margin) y = above;
  const clamp = (value: number, size: number, total: number) =>
    Math.max(margin, Math.min(value, total - size - margin));
  return { x: clamp(x, width, viewportWidth), y: clamp(y, height, viewportHeight), width, height };
}
