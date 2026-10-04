/** Coalesce invalidations into one frame without running an idle render loop. */
export function createFrameScheduler(draw: () => void) {
  let pending: number | undefined;
  let disposed = false;
  return {
    invalidate() {
      if (disposed || pending !== undefined) return;
      pending = requestAnimationFrame(() => {
        pending = undefined;
        if (!disposed) draw();
      });
    },
    dispose() {
      disposed = true;
      if (pending !== undefined) cancelAnimationFrame(pending);
      pending = undefined;
    },
  };
}
