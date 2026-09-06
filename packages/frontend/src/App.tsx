import { createResource, Match, Switch } from 'solid-js';

import { GpuCanvas } from '@/components/GpuCanvas';
import { detectWebGpuSupport, type WebGpuSupport } from '@/lib/webgpu/support';

export function App() {
  const [support] = createResource<WebGpuSupport>(detectWebGpuSupport);

  return (
    <main class="app">
      <header class="app__header">
        <h1>procon-2026</h1>
        <p class="app__tagline">Rust core + SolidJS + WebGPU viewer</p>
      </header>
      <Switch fallback={<p class="app__status">Detecting WebGPU support…</p>}>
        <Match when={support.state === 'errored'}>
          <p class="app__status app__status--error">WebGPU detection failed.</p>
        </Match>
        <Match when={support()?.available === false}>
          <p class="app__status app__status--error">WebGPU unavailable in this browser.</p>
        </Match>
        <Match when={support()?.available === true}>
          <GpuCanvas />
        </Match>
      </Switch>
    </main>
  );
}
