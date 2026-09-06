import { createSignal, onCleanup, onMount, Show } from 'solid-js';

import { createWebGpuContext } from '@/lib/webgpu/device';
import { PlaceholderRenderer } from '@/lib/webgpu/renderer';

export function GpuCanvas() {
  const [renderer, setRenderer] = createSignal<PlaceholderRenderer | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [frames, setFrames] = createSignal(0);

  let canvasRef!: HTMLCanvasElement;

  onMount(async () => {
    try {
      const ctx = await createWebGpuContext(canvasRef);
      const r = new PlaceholderRenderer(ctx, canvasRef);
      await r.init();
      r.start();
      setRenderer(r);
      const id = window.setInterval(() => setFrames(r.stats().framesRendered), 250);
      onCleanup(() => {
        window.clearInterval(id);
        r.stop();
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  });

  return (
    <div class="gpu-canvas">
      <canvas class="gpu-canvas__canvas" ref={canvasRef!} />
      <Show when={error()}>
        {(msg) => <p class="gpu-canvas__error">WebGPU init failed: {msg()}</p>}
      </Show>
      <Show when={renderer()}>
        <p class="gpu-canvas__hud">frames rendered: {frames()}</p>
      </Show>
    </div>
  );
}
