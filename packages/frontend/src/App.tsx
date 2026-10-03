// Octicons paths: @primer/octicons 19.38.0. Map marks: the ridge proposal in docs/design/proposals/visualizer.html.
import { A, Route, Router, useLocation } from '@solidjs/router';
import {
  For,
  Show,
  createEffect,
  createUniqueId,
  createSignal,
  onCleanup,
  onMount,
  untrack,
  type JSX,
} from 'solid-js';
import { Dynamic } from 'solid-js/web';
import * as RadioGroup from '@kobalte/core/radio-group';
import { createReplayStore, type PlaybackRate } from '@/stores/replay';
import { getTheme, toggleTheme } from '@/stores/theme';
import { fetchLiveSnapshot, type LiveSnapshot } from '@/lib/live';
import icons from '@/viewer-icons.json';
import marks from '@/viewer-marks.json';
import './viewer.css';
import { createViewerLayer } from '@/lib/webgpu/viewer-layer';

interface SvgNode {
  tag: string;
  attrs: Record<string, string>;
  children: SvgNode[];
}
const SvgNodes = (props: { nodes: SvgNode[] }) => (
  <For each={props.nodes}>
    {(node) => (
      <Dynamic component={node.tag} {...node.attrs}>
        <SvgNodes nodes={node.children} />
      </Dynamic>
    )}
  </For>
);
const Icon = (props: { name: keyof typeof icons }) => (
  <svg
    class="octicon"
    width="16"
    height="16"
    viewBox="0 0 16 16"
    fill="currentColor"
    aria-hidden="true"
  >
    <SvgNodes nodes={icons[props.name]} />
  </svg>
);
const Button = (props: {
  label: string;
  icon: keyof typeof icons;
  onClick: () => void;
  disabled?: boolean;
  testid?: string;
}) => (
  <button
    class="btn btn-sm"
    aria-label={props.label}
    title={props.label}
    onClick={() => props.onClick()}
    disabled={props.disabled}
    data-testid={props.testid}
  >
    <Icon name={props.icon} />
  </button>
);
const Panel = (props: { title: string; children: JSX.Element }) => (
  <section class="border rounded-2 p-3 mb-3">
    <h2 class="f5 text-bold mb-3">{props.title}</h2>
    {props.children}
  </section>
);
const Row = (props: { label: string; value: JSX.Element }) => (
  <div
    class="d-flex flex-justify-between flex-items-center border-bottom py-2"
    style={{ gap: 'var(--base-size-8)' }}
  >
    <span class="color-fg-muted">{props.label}</span>
    <span
      class="text-right"
      style={{ 'font-variant-numeric': 'tabular-nums', 'overflow-wrap': 'anywhere' }}
    >
      {props.value}
    </span>
  </div>
);
const cells = Array.from({ length: 48 }, (_, i) => ({
  col: i % 8,
  row: Math.floor(i / 8),
  terrain: (i % 11 === 0 ? 'mountain' : i % 9 === 0 ? 'water' : i % 5 === 0 ? 'road' : 'plain') as
    'plain' | 'mountain' | 'water' | 'road',
}));
const terrainHue = { plain: 'green', mountain: 'gray', water: 'blue', road: 'brown' };
const seriesHue = ['yellow', 'orange', 'auburn', 'purple', 'plum', 'olive'];
function MapView(props: { turn?: number; gpu?: boolean }) {
  const [zoom, setZoom] = createSignal(1),
    [x, setX] = createSignal(0),
    [y, setY] = createSignal(0);
  const [gpuReady, setGpuReady] = createSignal(false);
  const [gpuError, setGpuError] = createSignal('');
  const [layer, setLayer] = createSignal<Awaited<ReturnType<typeof createViewerLayer>> | null>(
    null,
  );
  let canvas!: HTMLCanvasElement;
  createEffect(() => {
    if (!props.gpu) return;
    let active = true;
    let failed = false;
    let created: Awaited<ReturnType<typeof createViewerLayer>> | undefined;
    setGpuError('');
    const fallback = (reason: string) => {
      failed = true;
      if (active) {
        setGpuError(reason);
        setGpuReady(false);
      }
    };
    const timeout = setTimeout(() => fallback('GPU initialization timed out'), 5000);
    void createViewerLayer(
      canvas,
      cells.map((cell) => ({ ...cell, hue: terrainHue[cell.terrain] })),
      fallback,
    )
      .then(async (value) => {
        created = value;
        if (!active || failed) {
          value.destroy();
          return;
        }
        try {
          const camera = untrack(() => [x(), y(), zoom()] as const);
          value.draw(...camera);
          await value.verifyFrame(...camera);
          clearTimeout(timeout);
          if (!active || failed) {
            value.destroy();
            return;
          }
          setLayer(value);
          setGpuReady(true);
        } catch (error) {
          fallback(String(error));
        }
      })
      .catch((error) => {
        clearTimeout(timeout);
        fallback(String(error));
      });
    onCleanup(() => {
      active = false;
      clearTimeout(timeout);
      created?.destroy();
      setLayer(null);
      setGpuReady(false);
    });
  });
  createEffect(() => {
    void getTheme().mode;
    const current = layer();
    const px = x(),
      py = y(),
      z = zoom();
    if (current) {
      try {
        current.draw(px, py, z);
      } catch (error) {
        setGpuReady(false);
        setGpuError(String(error));
      }
    }
  });
  onMount(() => {
    const observer = new ResizeObserver(() => {
      try {
        layer()?.draw(x(), y(), zoom());
      } catch (error) {
        setGpuReady(false);
        setGpuError(String(error));
      }
    });
    observer.observe(canvas);
    onCleanup(() => observer.disconnect());
  });
  let drag: { x: number; y: number } | undefined;
  const reset = () => {
    setZoom(1);
    setX(0);
    setY(0);
  };
  return (
    <div class="map-instrument">
      <div class="map-stage">
        <canvas
          ref={canvas}
          data-testid="webgpu-map-canvas"
          aria-hidden="true"
          style={{ visibility: gpuReady() ? 'visible' : 'hidden' }}
        />
        <svg
          data-testid="map-viewer-svg"
          viewBox="-24 -24 370 265"
          role="img"
          aria-label={`Match map, turn ${props.turn ?? 0}`}
          tabindex="0"
          onWheel={(e) => {
            e.preventDefault();
            setZoom((z) => Math.min(4, Math.max(0.5, z * (e.deltaY < 0 ? 1.1 : 1 / 1.1))));
          }}
          onPointerDown={(e) => {
            drag = { x: e.clientX, y: e.clientY };
            e.currentTarget.setPointerCapture(e.pointerId);
          }}
          onPointerMove={(e) => {
            if (drag) {
              const factor = 370 / e.currentTarget.getBoundingClientRect().width;
              setX((v) => v + (e.clientX - drag!.x) * factor);
              setY((v) => v + (e.clientY - drag!.y) * factor);
              drag = { x: e.clientX, y: e.clientY };
            }
          }}
          onPointerUp={() => {
            drag = undefined;
          }}
          onPointerCancel={() => {
            drag = undefined;
          }}
          onKeyDown={(e) => {
            if (
              ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', '+', '-', '0'].includes(e.key)
            ) {
              e.preventDefault();
              if (e.key === '0') reset();
              if (e.key === '+') setZoom((v) => Math.min(4, v * 1.1));
              if (e.key === '-') setZoom((v) => Math.max(0.5, v / 1.1));
              if (e.key === 'ArrowLeft') setX((v) => v + 12);
              if (e.key === 'ArrowRight') setX((v) => v - 12);
              if (e.key === 'ArrowUp') setY((v) => v + 12);
              if (e.key === 'ArrowDown') setY((v) => v - 12);
            }
          }}
        >
          <title>Fixture map — eight columns, six rows</title>
          <g transform={`translate(${x()} ${y()}) scale(${zoom()})`}>
            <For each={cells}>
              {(cell) => {
                const cx = cell.col * 40 + (cell.row % 2) * 20,
                  cy = cell.row * 35;
                return (
                  <g transform={`translate(${cx} ${cy})`}>
                    <polygon
                      points="0,-23 20,-11.5 20,11.5 0,23 -20,11.5 -20,-11.5"
                      fill={
                        gpuReady()
                          ? 'transparent'
                          : `var(--data-${terrainHue[cell.terrain]}-color-muted)`
                      }
                      stroke={`var(--data-${terrainHue[cell.terrain]}-color-emphasis)`}
                    />
                    <svg
                      x="-8"
                      y="-8"
                      width="16"
                      height="16"
                      viewBox="0 0 16 16"
                      style={{ color: `var(--data-${terrainHue[cell.terrain]}-color-emphasis)` }}
                    >
                      <SvgNodes nodes={marks[cell.terrain]} />
                    </svg>
                  </g>
                );
              }}
            </For>
            <For each={[0, 1, 2, 3, 4, 5]}>
              {(n) => (
                <svg
                  x={40 + n * 40}
                  y={27 + (n % 2) * 70}
                  width="18"
                  height="18"
                  viewBox="0 0 16 16"
                  style={{ color: `var(--data-${seriesHue[n]}-color-emphasis)` }}
                >
                  <SvgNodes nodes={marks[`s${n}` as keyof typeof marks]} />
                </svg>
              )}
            </For>
            <svg
              x={40 + ((props.turn ?? 0) % 6) * 40}
              y="132"
              width="20"
              height="20"
              viewBox="0 0 16 16"
              style={{ color: 'var(--data-pine-color-emphasis)' }}
            >
              <SvgNodes nodes={marks.patrol} />
            </svg>
            <svg
              x="220"
              y="100"
              width="20"
              height="20"
              viewBox="0 0 16 16"
              style={{ color: 'var(--data-pink-color-emphasis)' }}
            >
              <SvgNodes nodes={marks.supply} />
            </svg>
          </g>
        </svg>
      </div>
      <Show when={props.gpu && !gpuReady()}>
        <p
          class="px-3 color-fg-muted"
          data-testid={gpuError() ? 'webgpu-map-fallback' : 'webgpu-map-loading'}
        >
          {gpuError() ? `SVG fallback: ${gpuError()}` : 'Loading WebGPU renderer…'}
        </p>
      </Show>
      <div class="map-footer">
        <button class="btn btn-sm" onClick={reset}>
          Reset view
        </button>
      </div>
    </div>
  );
}
function ReplayPage() {
  const store = createReplayStore();
  store.load({
    id: 'demo',
    title: 'Demo match',
    steps: Array.from({ length: 5 }, (_, turn) => ({
      matchId: { type: 'match' as const, value: 'demo-match' },
      turn,
      scores: [
        [{ type: 'player' as const, value: 'alice' }, turn * 10] as const,
        [{ type: 'player' as const, value: 'bob' }, turn * 10 + 1] as const,
      ],
    })),
  });
  const scrubId = createUniqueId();
  let committedStep = 0;
  let resume = false;
  const [scrubbing, setScrubbing] = createSignal(false);
  createEffect(() => {
    const s = store.state();
    if (!s.isPlaying || scrubbing()) return;
    const id = setInterval(() => {
      if (store.state().currentStep >= store.stepCount() - 1) store.pause();
      else {
        store.stepForward();
        if (store.state().currentStep >= store.stepCount() - 1) store.pause();
      }
    }, 1000 / s.rate);
    onCleanup(() => clearInterval(id));
  });
  const begin = () => {
    if (scrubbing()) return;
    committedStep = store.state().currentStep;
    resume = store.state().isPlaying;
    setScrubbing(true);
    store.pause();
  };
  const end = () => {
    if (!scrubbing()) return;
    setScrubbing(false);
    if (resume && store.state().currentStep < store.stepCount() - 1) store.play();
  };
  const cancel = () => {
    if (!scrubbing()) return;
    store.setStep(committedStep);
    end();
  };
  const shortcut = (e: KeyboardEvent) => {
    if (e.repeat || e.altKey || e.ctrlKey || e.metaKey) return;
    if (
      (e.target as Element).closest(
        'button,input,select,textarea,a,svg,[role="radiogroup"],[contenteditable]',
      )
    )
      return;
    if (![' ', 'ArrowLeft', 'ArrowRight', 'Home', 'End', '1', '2', '4'].includes(e.key)) return;
    e.preventDefault();
    if (e.key === ' ') {
      if (store.state().isPlaying) store.pause();
      else store.play();
    }
    if (e.key === 'ArrowLeft') store.stepBackward();
    if (e.key === 'ArrowRight') store.stepForward();
    if (e.key === 'Home') store.setStep(0);
    if (e.key === 'End') {
      store.setStep(store.stepCount() - 1);
      store.pause();
    }
    if (['1', '2', '4'].includes(e.key)) store.setRate(Number(e.key) as PlaybackRate);
  };
  return (
    <div
      class="PageLayout-columns"
      role="region"
      aria-label="Replay viewer"
      tabindex="0"
      data-testid="replay-viewer"
      onKeyDown={shortcut}
    >
      <div class="PageLayout-content">
        <MapView turn={store.state().currentStep} />
      </div>
      <aside class="PageLayout-pane" aria-label="Replay information and controls">
        <Panel title="Replay">
          <p class="color-fg-muted f6">Demo match · alice / bob</p>
          <div class="toolbar">
            <Button
              label="First step"
              icon="move-to-start"
              onClick={() => store.setStep(0)}
              disabled={store.state().currentStep === 0}
            />
            <Button
              label="Step backward"
              icon="chevron-left"
              testid="replay-step-back"
              onClick={store.stepBackward}
              disabled={store.state().currentStep === 0}
            />
            <Button
              label={store.state().isPlaying ? 'Pause' : 'Play'}
              icon={store.state().isPlaying ? 'pause' : 'triangle-fill'}
              testid="replay-toggle"
              onClick={() => (store.state().isPlaying ? store.pause() : store.play())}
            />
            <Button
              label="Step forward"
              icon="chevron-right"
              testid="replay-step-forward"
              onClick={store.stepForward}
              disabled={store.state().currentStep === 4}
            />
            <Button
              label="Last step"
              icon="move-to-end"
              onClick={() => {
                store.setStep(4);
                store.pause();
              }}
              disabled={store.state().currentStep === 4}
            />
            <RadioGroup.Root
              class="rates"
              aria-label="Playback rate"
              value={String(store.state().rate)}
              onChange={(v) => store.setRate(Number(v) as PlaybackRate)}
            >
              <For each={[1, 2, 4]}>
                {(rate) => (
                  <RadioGroup.Item
                    value={String(rate)}
                    class="btn btn-sm"
                    classList={{ selected: store.state().rate === rate }}
                    data-testid={`replay-rate-${rate}`}
                  >
                    <RadioGroup.ItemInput />
                    <RadioGroup.ItemLabel>{rate}×</RadioGroup.ItemLabel>
                  </RadioGroup.Item>
                )}
              </For>
            </RadioGroup.Root>
            <span class="ml-auto" data-testid="replay-current-step">
              {store.state().currentStep + 1} / {store.stepCount()}
            </span>
          </div>
          <label class="scrub-label" for={scrubId}>
            Replay position
          </label>
          <input
            id={scrubId}
            class="scrub"
            data-testid="replay-scrub"
            type="range"
            min="0"
            max="4"
            value={store.state().currentStep}
            aria-valuetext={`Step ${store.state().currentStep + 1} of 5`}
            onPointerDown={(e) => {
              begin();
              e.currentTarget.setPointerCapture(e.pointerId);
            }}
            onPointerUp={end}
            onPointerCancel={cancel}
            onBlur={end}
            onKeyDown={(e) => {
              if (e.key === 'Escape') {
                e.preventDefault();
                cancel();
              } else if (
                ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(e.key)
              )
                begin();
            }}
            onKeyUp={end}
            onInput={(e) => store.setStep(Number(e.currentTarget.value))}
          />
          <details class="mt-3">
            <summary class="color-fg-muted f6">Keyboard shortcuts</summary>
            <p class="f6 mt-2 mb-0">
              Space play/pause · ← → step · Home / End · 1 2 4 rate · Escape cancels scrub
            </p>
          </details>
        </Panel>
        <Panel title="Score">
          <table class="score-table" aria-label="Scores">
            <thead>
              <tr>
                <th scope="col">Player</th>
                <th scope="col">Score</th>
              </tr>
            </thead>
            <tbody>
              <For each={store.currentFrame()?.scores}>
                {(entry) => (
                  <tr>
                    <th scope="row">{entry[0].value}</th>
                    <td>{entry[1]}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </Panel>
      </aside>
    </div>
  );
}

function LivePage() {
  const [snapshot, setSnapshot] = createSignal<LiveSnapshot | null>(null),
    [received, setReceived] = createSignal(0),
    [now, setNow] = createSignal(Date.now());
  const controller = new AbortController();
  let active = true;
  let polling = false;
  const started = Date.now();
  onMount(() => {
    const poll = async () => {
      if (polling) return;
      polling = true;
      const value = await fetchLiveSnapshot(controller.signal);
      polling = false;
      if (active && value) {
        setSnapshot(value);
        setReceived(Date.now());
      }
    };
    void poll();
    const timer = setInterval(() => {
      setNow(Date.now());
      void poll();
    }, 1000);
    onCleanup(() => {
      active = false;
      controller.abort();
      clearInterval(timer);
    });
  });
  const unavailable = () => !snapshot() && now() - started >= 5000;
  const stale = () => received() > 0 && now() - received() >= 5000;
  const statusLabel = {
    waiting: 'Idle',
    running: 'Running',
    completed: 'Completed',
    rejected: 'Error',
    error: 'Error',
  };
  const label = () =>
    unavailable()
      ? 'No data'
      : stale()
        ? 'Stale'
        : snapshot()
          ? statusLabel[snapshot()!.status]
          : 'Connecting';
  const statusIcon = () =>
    unavailable() || stale()
      ? 'alert'
      : snapshot()?.status === 'running'
        ? 'broadcast'
        : snapshot()?.status === 'completed'
          ? 'check-circle'
          : snapshot()?.status === 'error' || snapshot()?.status === 'rejected'
            ? 'x-circle'
            : 'clock';
  const statusColor = () =>
    unavailable() || stale()
      ? 'color-fg-attention'
      : snapshot()?.status === 'running' || snapshot()?.status === 'completed'
        ? 'color-fg-success'
        : snapshot()?.status === 'error' || snapshot()?.status === 'rejected'
          ? 'color-fg-danger'
          : 'color-fg-muted';
  return (
    <div class="PageLayout-columns">
      <div class="PageLayout-content">
        <MapView />
      </div>
      <aside class="PageLayout-pane" aria-label="Live match information">
        <Panel title="Match">
          <Row
            label="Status"
            value={
              <span
                role="status"
                class="d-inline-flex flex-items-center"
                data-testid="live-status-badge"
                data-status={snapshot()?.status ?? 'connecting'}
              >
                <span class={statusColor() + ' mr-2'}>
                  <Icon name={statusIcon()} />
                </span>
                {label()}
              </span>
            }
          />
          <Row
            label="Day"
            value={
              snapshot()?.day == null
                ? '—'
                : `${(snapshot()?.day ?? -1) + 1} / ${snapshot()?.daysTotal ?? '—'}`
            }
          />
          <Row
            label="Revisions"
            value={snapshot() ? `[${snapshot()!.revisions.join(', ')}]` : '—'}
          />
          <Row label="Solver" value={snapshot()?.solver ?? '—'} />
          <Row label="Server" value={snapshot()?.server ?? '—'} />
          <Row
            label="Last update"
            value={
              snapshot()
                ? snapshot()!.lastUpdateUnix === 0
                  ? '00:00:00'
                  : new Date(snapshot()!.lastUpdateUnix * 1000).toLocaleTimeString(undefined, {
                      hour12: false,
                    })
                : '—'
            }
          />
          <Show when={unavailable()}>
            <div class="flash" role="status">
              No match snapshot available. Start the CLI with --state-output pointing to
              public/match-state.json.
            </div>
          </Show>
          <Show when={stale()}>
            <div class="flash flash-warn" role="status">
              Feed is stale. Values are last-known; check the CLI state output.
            </div>
          </Show>
          <Show when={snapshot()?.error}>
            <div class="flash flash-error" role="alert">
              {snapshot()?.error}
            </div>
          </Show>
        </Panel>
        <Panel title="Source">
          <p class="color-fg-muted f6">
            Status: local match-state snapshot. Map: demo fixture; the snapshot contains no map or
            agent positions.
          </p>
          <Row label="Polling" value="1 second" />
        </Panel>
      </aside>
    </div>
  );
}

function SolversPage() {
  const [renderer, setRenderer] = createSignal('svg');
  return (
    <div class="PageLayout-columns">
      <div class="PageLayout-content">
        <MapView gpu={renderer() === 'webgpu'} />
      </div>
      <aside class="PageLayout-pane" aria-label="Renderer information and controls">
        <Panel title="Renderer">
          <div class="BtnGroup mb-3">
            <button
              class="btn btn-sm BtnGroup-item"
              data-testid="solvers-renderer-svg"
              aria-pressed={renderer() === 'svg'}
              classList={{ selected: renderer() === 'svg' }}
              onClick={() => setRenderer('svg')}
            >
              SVG
            </button>
            <button
              class="btn btn-sm BtnGroup-item"
              data-testid="solvers-renderer-webgpu"
              aria-pressed={renderer() === 'webgpu'}
              classList={{ selected: renderer() === 'webgpu' }}
              onClick={() => setRenderer('webgpu')}
            >
              WebGPU
            </button>
          </div>
          <p class="color-fg-muted f6 mb-0">
            Demo map · 8 × 6 · 48 cells. WebGPU draws terrain; symbols remain SVG.
          </p>
        </Panel>
        <Panel title="Legend">
          <For each={['plain', 'mountain', 'water', 'road', 'patrol', 'supply'] as const}>
            {(name) => (
              <div class="legend-row">
                <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
                  <SvgNodes nodes={marks[name]} />
                </svg>
                <span>{name}</span>
              </div>
            )}
          </For>
        </Panel>
      </aside>
    </div>
  );
}

function Home() {
  return (
    <div class="PageLayout-columns">
      <div class="PageLayout-content">
        <MapView />
      </div>
      <aside class="PageLayout-pane" aria-label="Map information">
        <Panel title="Map">
          <Row label="Source" value="Demo fixture" />
          <Row label="Dimensions" value="8 × 6" />
          <Row label="Cells" value="48" />
          <Row label="Spot series" value="6" />
        </Panel>
        <Panel title="Legend">
          <For each={['plain', 'mountain', 'water', 'road', 'patrol', 'supply'] as const}>
            {(name) => (
              <div class="legend-row">
                <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
                  <SvgNodes nodes={marks[name]} />
                </svg>
                <span>{name}</span>
              </div>
            )}
          </For>
        </Panel>
      </aside>
    </div>
  );
}
function Shell(props: { children?: JSX.Element }) {
  const location = useLocation();
  const destinations = [
    ['/', 'Map', 'home'],
    ['/replays', 'Replays', 'history'],
    ['/solvers', 'Solvers', 'beaker'],
    ['/live', 'Live', 'broadcast'],
  ] as const;
  return (
    <div class="app-shell bgColor-default">
      <a class="skip-link" href="#main">
        Skip to content
      </a>
      <header
        class="d-flex flex-items-center flex-wrap border-bottom px-3 py-2"
        data-testid="app-header"
      >
        <h1 class="f5 mr-3 mb-0">Match viewer</h1>
        <nav aria-label="Main navigation" class="d-flex flex-items-center flex-wrap">
          <For each={destinations}>
            {(item) => (
              <A
                href={item[0]}
                end={item[0] === '/'}
                class="btn btn-sm btn-invisible color-fg-default mr-1"
                aria-current={location.pathname === item[0] ? 'page' : undefined}
                classList={{
                  'bgColor-muted': location.pathname === item[0],
                  'text-bold': location.pathname === item[0],
                }}
              >
                <Icon name={item[2]} /> <span>{item[1]}</span>
              </A>
            )}
          </For>
        </nav>
        <button class="btn btn-sm ml-auto" onClick={toggleTheme} aria-label="Toggle theme">
          <Icon name={getTheme().mode === 'dark' ? 'sun' : 'moon'} />
        </button>
      </header>
      <main id="main" class="app-main">
        <div class="PageLayout PageLayout--panePos-end PageLayout--responsive-stackRegions PageLayout--outerSpacing-normal PageLayout--columnGap-normal PageLayout--rowGap-normal">
          {props.children}
        </div>
      </main>
    </div>
  );
}
export function App() {
  return (
    <Router root={Shell}>
      <Route path="/" component={Home} />
      <Route path="/replays" component={ReplayPage} />
      <Route path="/solvers" component={SolversPage} />
      <Route path="/live" component={LivePage} />
      <Route path="*" component={Home} />
    </Router>
  );
}
