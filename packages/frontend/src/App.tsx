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
import { rulesLink, RULE_SECTIONS } from '@/lib/rules';
import {
  loadHexAssets,
  glyphAtlas,
  drawCanvasScene,
  fixtureScene,
  GLYPH_SIZE,
  markNames,
  samples,
  sceneTransform,
  type Sample,
  type Camera,
  type Mark,
} from '@/lib/webgpu/viewer-scene';
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
  text?: string;
  grouped?: boolean;
  block?: boolean;
  primary?: boolean;
}) => (
  <button
    class={`btn${props.grouped ? ' BtnGroup-item' : ''}${props.block ? ' btn-block' : ''}${props.primary ? ' btn-primary' : ''}`}
    aria-label={props.label}
    title={props.label}
    onClick={() => props.onClick()}
    disabled={props.disabled}
    data-testid={props.testid}
  >
    <Icon name={props.icon} /> {props.text ?? props.label}
  </button>
);
const Panel = (props: { title: string; children: JSX.Element }) => (
  <section class="border rounded-2 p-3 mb-3">
    <h2 class="f5 text-bold mb-3">{props.title}</h2>
    {props.children}
  </section>
);
const Row = (props: { label: string; value: JSX.Element }) => (
  <div class="d-flex flex-justify-between flex-items-center border-bottom py-2">
    <span class="color-fg-muted mr-2">{props.label}</span>
    <span class="text-right text-mono wb-break-word">{props.value}</span>
  </div>
);
function createViewport() {
  const [camera, setCamera] = createSignal<Camera>({ x: 0, y: 0, zoom: 1 });
  const [sample, setSample] = createSignal<Sample>('figure1');
  const [padding, setPadding] = createSignal(0);
  const [renderer, setRenderer] = createSignal('Initializing');
  const [error, setError] = createSignal('');
  return {
    camera,
    setCamera,
    sample,
    setSample,
    padding,
    setPadding,
    renderer,
    setRenderer,
    error,
    setError,
    reset: () => setCamera({ x: 0, y: 0, zoom: 1 }),
  };
}
type Viewport = ReturnType<typeof createViewport>;
function RuleLink(props: { section: string; children: JSX.Element }) {
  return (
    <a href={rulesLink(props.section)} target="_blank" rel="noopener noreferrer">
      {props.children}
    </a>
  );
}

function ViewControls(props: { viewport: Viewport }) {
  return (
    <Panel title="View">
      <label class="d-block text-bold mb-2" for="map-sample">
        Example
      </label>
      <select
        id="map-sample"
        class="form-select width-full mb-3"
        value={props.viewport.sample()}
        onChange={(e) => {
          props.viewport.setSample(e.currentTarget.value as Sample);
          props.viewport.reset();
        }}
      >
        <For each={Object.entries(samples)}>
          {([id, data]) => <option value={id}>{data.label}</option>}
        </For>
      </select>
      <p class="f6 color-fg-muted mb-3" data-testid="sample-source">
        Source:{' '}
        <RuleLink section={samples[props.viewport.sample()].source.anchor}>
          {samples[props.viewport.sample()].source.file}（p.
          {samples[props.viewport.sample()].source.page}・
          {samples[props.viewport.sample()].source.figure}）
        </RuleLink>
      </p>
      <label class="d-block text-bold mb-2" for="map-padding">
        Outer grid
      </label>
      <select
        id="map-padding"
        class="form-select width-full mb-3"
        value={props.viewport.padding()}
        onChange={(e) => {
          props.viewport.setPadding(Number(e.currentTarget.value));
          props.viewport.reset();
        }}
      >
        <option value="0">Off</option>
        <option value="1">1セル分</option>
        <option value="2">2セル分</option>
        <option value="3">3セル分</option>
      </select>
      <Row label="Renderer" value={props.viewport.renderer()} />
      <Button label="Reset view" icon="sync" block onClick={props.viewport.reset} />
      <Show when={props.viewport.error()}>
        <p class="f6 color-fg-muted mt-2 mb-0" data-testid="webgpu-map-fallback">
          Canvas fallback: {props.viewport.error()}
        </p>
      </Show>
      <Show when={props.viewport.renderer() === 'Initializing'}>
        <p class="f6 color-fg-muted mt-2 mb-0" data-testid="webgpu-map-loading">
          Initializing GPU…
        </p>
      </Show>
    </Panel>
  );
}
function GlyphPreview(props: { mark: Mark; hue: string }) {
  let canvas!: HTMLCanvasElement;
  createEffect(() => {
    void getTheme().mode;
    const index = markNames.indexOf(props.mark);
    void loadHexAssets().then(() => {
      const source = glyphAtlas();
      const context = canvas.getContext('2d')!;
      context.clearRect(0, 0, 16, 16);
      context.drawImage(source, index * GLYPH_SIZE, 0, GLYPH_SIZE, GLYPH_SIZE, 0, 0, 16, 16);
    });
  });
  return <canvas ref={canvas} width="16" height="16" aria-hidden="true" />;
}
function Legend() {
  return (
    <Panel title="Legend">
      <For
        each={
          [
            { mark: 'plain', hue: 'green' },
            { mark: 'mountain', hue: 'gray' },
            { mark: 'water', hue: 'blue' },
            { mark: 'road', hue: 'brown' },
            { mark: 'patrol', hue: 'pine' },
            { mark: 'supply', hue: 'pink' },
          ] as const
        }
      >
        {(item) => (
          <div class="d-flex flex-items-center py-1">
            <GlyphPreview {...item} />
            <span class="ml-2">
              {
                {
                  plain: '平地',
                  mountain: '山地',
                  water: '池',
                  road: '道路',
                  patrol: '巡回車',
                  supply: '補給車',
                }[item.mark]
              }
            </span>
          </div>
        )}
      </For>
      <nav aria-label="Rules by topic" class="border-top pt-2 mt-3">
        <For each={['map', 'agents', 'spots'] as const}>
          {(section) => (
            <div class="f6 py-1">
              <RuleLink section={section}>{RULE_SECTIONS[section]}</RuleLink>
            </div>
          )}
        </For>
      </nav>
    </Panel>
  );
}
function MapView(props: { turn?: number; viewport: Viewport }) {
  const viewport = untrack(() => props.viewport);
  const [layer, setLayer] = createSignal<Awaited<ReturnType<typeof createViewerLayer>> | null>(
    null,
  );
  let gpu!: HTMLCanvasElement, fallback!: HTMLCanvasElement, stage!: HTMLDivElement;
  const [assetsReady, setAssetsReady] = createSignal(false);
  const scene = () =>
    fixtureScene(
      props.turn ?? 0,
      'ridge',
      viewport.sample(),
      viewport.padding(),
      props.turn !== undefined,
    );
  const fail = (reason: string) => {
    viewport.setError(reason);
    viewport.setRenderer('Canvas');
    const current = untrack(layer);
    setLayer(null);
    current?.destroy();
  };
  const draw = () => {
    if (!assetsReady()) return;
    const current = layer();
    if (current) {
      try {
        current.draw(scene(), viewport.camera());
      } catch (error) {
        fail(String(error));
      }
    }
    if (!layer()) drawCanvasScene(fallback, scene(), viewport.camera());
  };
  createEffect(() => {
    void getTheme().mode;
    draw();
  });
  onMount(() => {
    void loadHexAssets().then(() => setAssetsReady(true));
    let active = true,
      failed = false;
    let created: Awaited<ReturnType<typeof createViewerLayer>> | undefined;
    const reject = (reason: string) => {
      if (active) {
        failed = true;
        fail(reason);
      }
    };
    const timeout = setTimeout(() => reject('GPU initialization timed out'), 5000);
    void createViewerLayer(gpu, reject)
      .then(async (value) => {
        created = value;
        if (!active || failed) {
          value.destroy();
          return;
        }
        try {
          const firstScene = untrack(() => scene()),
            camera = untrack(viewport.camera);
          value.draw(firstScene, camera);
          await value.verifyFrame(firstScene, camera);
          clearTimeout(timeout);
          if (!active || failed) {
            value.destroy();
            return;
          }
          setLayer(value);
          viewport.setRenderer('WebGPU');
        } catch (error) {
          value.destroy();
          reject(String(error));
        }
      })
      .catch((error) => {
        clearTimeout(timeout);
        reject(String(error));
      });
    const observer = new ResizeObserver(() => untrack(() => draw()));
    observer.observe(stage);
    onCleanup(() => {
      active = false;
      clearTimeout(timeout);
      observer.disconnect();
      created?.destroy();
    });
  });
  let drag: { x: number; y: number } | undefined;
  return (
    <section class="border rounded-2 overflow-hidden bgColor-muted" aria-label="Visualizer">
      <div
        class="map-stage"
        ref={stage}
        role="img"
        tabindex="0"
        data-testid="map-viewer"
        aria-label={`マップ、ステップ ${props.turn ?? 0}. Arrow keys to pan, plus/minus to zoom, 0 to reset view.`}
        data-camera={JSON.stringify(viewport.camera())}
        data-renderer={viewport.renderer()}
        onWheel={(e) => {
          e.preventDefault();
          viewport.setCamera((c) => ({
            ...c,
            zoom: Math.min(4, Math.max(0.5, c.zoom * (e.deltaY < 0 ? 1.1 : 1 / 1.1))),
          }));
        }}
        onPointerDown={(e) => {
          drag = { x: e.clientX, y: e.clientY };
          e.currentTarget.setPointerCapture(e.pointerId);
        }}
        onPointerMove={(e) => {
          if (drag) {
            const rect = e.currentTarget.getBoundingClientRect(),
              fit = sceneTransform(scene(), rect.width, rect.height, viewport.camera()).fit;
            viewport.setCamera((c) => ({
              ...c,
              x: c.x + (e.clientX - drag!.x) / fit,
              y: c.y + (e.clientY - drag!.y) / fit,
            }));
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
          if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', '+', '-', '0'].includes(e.key)) {
            e.preventDefault();
            e.stopPropagation();
            if (e.key === '0') viewport.reset();
            else
              viewport.setCamera((c) => ({
                ...c,
                x: c.x + (e.key === 'ArrowLeft' ? 12 : e.key === 'ArrowRight' ? -12 : 0),
                y: c.y + (e.key === 'ArrowUp' ? 12 : e.key === 'ArrowDown' ? -12 : 0),
                zoom:
                  e.key === '+'
                    ? Math.min(4, c.zoom * 1.1)
                    : e.key === '-'
                      ? Math.max(0.5, c.zoom / 1.1)
                      : c.zoom,
              }));
          }
        }}
      >
        <canvas
          ref={gpu}
          data-testid="webgpu-map-canvas"
          aria-hidden="true"
          style={{ visibility: layer() ? 'visible' : 'hidden' }}
        />
        <canvas
          ref={fallback}
          data-testid="canvas-map-fallback"
          aria-hidden="true"
          style={{ visibility: layer() ? 'hidden' : 'visible' }}
        />
      </div>
    </section>
  );
}
function ReplayPage() {
  const viewport = createViewport();
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
        'button,input,select,textarea,a,[role="img"],[role="radiogroup"],[contenteditable]',
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
        <MapView turn={store.state().currentStep} viewport={viewport} />
      </div>
      <aside class="PageLayout-pane" aria-label="Replay information and controls">
        <Panel title="Replays">
          <p class="color-fg-muted f6">Demo match · alice / bob</p>
          <div class="mb-3">
            <Button
              label={store.state().isPlaying ? 'Pause' : 'Play'}
              icon={store.state().isPlaying ? 'pause' : 'triangle-fill'}
              block
              primary={!store.state().isPlaying}
              testid="replay-toggle"
              onClick={() => (store.state().isPlaying ? store.pause() : store.play())}
            />
          </div>
          <div class="mb-3">
            <div class="d-flex flex-justify-between flex-items-center mb-2">
              <span class="text-bold">ステップ</span>
              <span class="text-mono" data-testid="replay-current-step">
                {store.state().currentStep + 1} / {store.stepCount()}
              </span>
            </div>
            <div class="BtnGroup d-flex mb-2" role="group" aria-label="Step navigation">
              <Button
                label="Step backward"
                text="Previous"
                icon="chevron-left"
                grouped
                block
                testid="replay-step-back"
                onClick={store.stepBackward}
                disabled={store.state().currentStep === 0}
              />
              <Button
                label="Step forward"
                text="Next"
                icon="chevron-right"
                grouped
                block
                testid="replay-step-forward"
                onClick={store.stepForward}
                disabled={store.state().currentStep === 4}
              />
            </div>
            <div class="BtnGroup d-flex" role="group" aria-label="Replay boundaries">
              <Button
                label="First step"
                text="First"
                icon="move-to-start"
                grouped
                block
                onClick={() => store.setStep(0)}
                disabled={store.state().currentStep === 0}
              />
              <Button
                label="Last step"
                text="Last"
                icon="move-to-end"
                grouped
                block
                onClick={() => {
                  store.setStep(4);
                  store.pause();
                }}
                disabled={store.state().currentStep === 4}
              />
            </div>
          </div>
          <RadioGroup.Root
            class="mb-3"
            aria-label="Playback speed"
            value={String(store.state().rate)}
            onChange={(v) => store.setRate(Number(v) as PlaybackRate)}
          >
            <RadioGroup.Label class="d-block text-bold mb-2">Playback speed</RadioGroup.Label>
            <div class="BtnGroup d-flex">
              <For each={[1, 2, 4]}>
                {(rate) => (
                  <RadioGroup.Item
                    value={String(rate)}
                    class="btn BtnGroup-item btn-block"
                    classList={{ selected: store.state().rate === rate }}
                    data-testid={`replay-rate-${rate}`}
                  >
                    <RadioGroup.ItemInput />
                    <RadioGroup.ItemLabel>{rate}×</RadioGroup.ItemLabel>
                  </RadioGroup.Item>
                )}
              </For>
            </div>
          </RadioGroup.Root>
          <label class="d-block text-bold mb-2" for={scrubId}>
            Replay position
          </label>
          <input
            id={scrubId}
            class="width-full"
            data-testid="replay-scrub"
            type="range"
            min="0"
            max="4"
            value={store.state().currentStep}
            aria-valuetext={`ステップ ${store.state().currentStep + 1} / 5`}
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
              Space: Play / Pause · ← →: ステップ · Home / End · 1 2 4: Playback speed · Escape:
              Cancel scrub
            </p>
          </details>
        </Panel>
        <ViewControls viewport={viewport} />
        <p class="f6">
          <RuleLink section="overview">{RULE_SECTIONS.overview}</RuleLink> ·{' '}
          <RuleLink section="time">{RULE_SECTIONS.time}</RuleLink>
        </p>
        <Panel title="Score (demo)">
          <table class="width-full" aria-label="Score (demo)">
            <thead>
              <tr>
                <th class="text-left color-fg-muted border-bottom pb-2" scope="col">
                  チーム
                </th>
                <th class="text-left color-fg-muted border-bottom pb-2" scope="col">
                  Score (demo)
                </th>
              </tr>
            </thead>
            <tbody>
              <For each={store.currentFrame()?.scores}>
                {(entry) => (
                  <tr>
                    <th class="text-left border-bottom py-2" scope="row">
                      {entry[0].value}
                    </th>
                    <td class="text-right text-mono border-bottom py-2">{entry[1]}</td>
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
  const viewport = createViewport();
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
        <MapView viewport={viewport} />
      </div>
      <aside class="PageLayout-pane" aria-label="Live match information">
        <ViewControls viewport={viewport} />
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
            label="日"
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
              試合情報を受信していません。
            </div>
          </Show>
          <Show when={stale()}>
            <div class="flash flash-warn" role="status">
              更新が停止しています。最後に受信した試合情報を表示しています。
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
            試合情報は接続先から取得しています。マップは問題文の例です。受信データにマップやエージェントの位置は含まれていません。
          </p>
          <Row label="Polling" value="1秒" />
        </Panel>
      </aside>
    </div>
  );
}

function SolversPage() {
  const viewport = createViewport();
  return (
    <div class="PageLayout-columns">
      <div class="PageLayout-content">
        <MapView viewport={viewport} />
      </div>
      <aside class="PageLayout-pane" aria-label="Renderer information and controls">
        <ViewControls viewport={viewport} />
        <Panel title="マップ">
          <Row label="Source" value={samples[viewport.sample()].label} />
          <Row
            label="セル数"
            value={samples[viewport.sample()].width * samples[viewport.sample()].height}
          />
          <Row label="Symbols" value="稜線・波・帯" />
        </Panel>
        <Legend />
      </aside>
    </div>
  );
}
function Home() {
  const viewport = createViewport();
  return (
    <div class="PageLayout-columns">
      <div class="PageLayout-content">
        <MapView viewport={viewport} />
      </div>
      <aside class="PageLayout-pane" aria-label="Map information">
        <ViewControls viewport={viewport} />
        <Panel title="マップ">
          <Row label="Source" value={samples[viewport.sample()].label} />
          <Row
            label="マップの大きさ"
            value={`${samples[viewport.sample()].width} × ${samples[viewport.sample()].height}`}
          />
          <Row
            label="セル数"
            value={samples[viewport.sample()].width * samples[viewport.sample()].height}
          />
          <Row
            label="系列数"
            value={new Set(samples[viewport.sample()].spots.map((spot) => spot.series)).size}
          />
        </Panel>
        <Legend />
      </aside>
    </div>
  );
}
function Shell(props: { children?: JSX.Element }) {
  const location = useLocation();
  const destinations = [
    ['/', 'マップ', 'home'],
    ['/replays', 'Replays', 'history'],
    ['/solvers', 'Solvers', 'beaker'],
    ['/live', 'Live', 'broadcast'],
  ] as const;
  return (
    <div class="AppFrame app-shell bgColor-default">
      <div class="AppFrame-a11yNav">
        <a class="AppFrame-a11yLink" href="#main">
          Skip to content
        </a>
      </div>
      <header
        class="d-flex flex-items-center flex-wrap border-bottom px-3 py-2"
        data-testid="app-header"
      >
        <h1 class="f5 mr-3 mb-0">ヘキサうどん_ ビューア</h1>
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
        <a
          class="btn btn-sm btn-invisible ml-auto mr-2"
          href={rulesLink(
            location.pathname === '/replays'
              ? 'time'
              : location.pathname === '/live'
                ? 'agents'
                : 'map',
          )}
          target="_blank"
          rel="noopener noreferrer"
        >
          Rules
        </a>
        <button class="btn btn-sm" onClick={toggleTheme} aria-label="Toggle theme">
          <Icon name={getTheme().mode === 'dark' ? 'sun' : 'moon'} /> Theme
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
