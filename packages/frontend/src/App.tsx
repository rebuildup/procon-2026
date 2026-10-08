import { drawViewerOverlay } from '@/lib/webgpu/viewer-overlay';
// Octicons paths: @primer/octicons 19.38.0. Map marks: the ridge proposal in docs/design/proposals/visualizer.html.
import { A, Route, Router, useLocation } from '@solidjs/router';
import {
  For,
  Show,
  createEffect,
  createMemo,
  createUniqueId,
  createSignal,
  onCleanup,
  onMount,
  untrack,
  type JSX,
} from 'solid-js';
import { Dynamic } from 'solid-js/web';
import * as RadioGroup from '@kobalte/core/radio-group';
import { createReplayStore, PLAYBACK_RATES, type PlaybackRate } from '@/stores/replay';
import { getTheme, toggleTheme } from '@/stores/theme';
import { fetchMatch, matchScene, type Match } from '@/lib/team-match';
import { createFrameScheduler } from '@/lib/frame-scheduler';
import icons from '@/viewer-icons.json';
import q6 from '@/official-q6.json';
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
  zoomCamera,
  hitTestCell,
  type Sample,
  type Camera,
  type ViewerScene,
  type Mark,
} from '@/lib/webgpu/viewer-scene';
import './viewer.css';
import { DataTable, DataRow, DataCell } from '@/components/DataTable';
import { Ratio } from '@/components/Ratio';
import { tooltipPosition, type TooltipPosition } from '@/lib/tooltip-position';
import {
  tileAtlas,
  tileSource,
  tileIndex,
  TILE_SIZE,
  TILE_WORLD_SIZE,
  HALO_TILE,
  INK_TILE,
  type Terrain,
} from '@/lib/webgpu/hex-assets';
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

function ViewControls(props: { viewport: Viewport; live?: boolean }) {
  return (
    <Panel title="View">
      <Show when={!props.live}>
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
      </Show>
      <Row label="Renderer" value={props.viewport.renderer()} />
      <div class="pt-3">
        <Button label="Reset view" icon="sync" block onClick={props.viewport.reset} />
      </div>
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
function LegendPreview(props: { terrain?: Terrain; mark?: Mark; traffic?: number }) {
  let canvas!: HTMLCanvasElement;
  const appearance = untrack(() => ({
    terrain: props.terrain,
    mark: props.mark,
    traffic: props.traffic,
  }));
  onMount(() => {
    let active = true;
    onCleanup(() => {
      active = false;
    });
    void loadHexAssets().then(() => {
      if (!active) return;
      const context = canvas.getContext('2d')!;
      if (appearance.terrain) {
        for (const index of [
          HALO_TILE,
          tileIndex(appearance.terrain, undefined, appearance.traffic),
          INK_TILE,
        ]) {
          const source = tileSource(index);
          context.drawImage(tileAtlas(), source.x, source.y, TILE_SIZE, TILE_SIZE, 0, 0, 64, 64);
        }
      } else if (appearance.mark) {
        const index = markNames.indexOf(appearance.mark);
        context.drawImage(
          glyphAtlas(),
          index * GLYPH_SIZE,
          0,
          GLYPH_SIZE,
          GLYPH_SIZE,
          0,
          0,
          64,
          64,
        );
      }
    });
  });
  return (
    <canvas
      ref={canvas}
      width="64"
      height="64"
      class="legend-preview"
      data-terrain={props.terrain}
      data-mark={props.mark}
      data-traffic={props.traffic}
      aria-hidden="true"
    />
  );
}
function Legend() {
  return (
    <Panel title="Legend">
      <h3 class="f6 color-fg-muted mb-1">地形・道路状態</h3>
      <dl class="legend-grid legend-terrain mt-0 mb-2">
        <For
          each={
            [
              { terrain: 'plain', name: '平地', pattern: '点', traffic: 0 },
              { terrain: 'road', name: '道路', pattern: '順調', traffic: 0 },
              { terrain: 'road', name: '道路', pattern: '混雑', traffic: 1 },
              { terrain: 'road', name: '道路', pattern: '渋滞', traffic: 2 },
              { terrain: 'mountain', name: '山地', pattern: '横線', traffic: 0 },
              { terrain: 'water', name: '池', pattern: '波', traffic: 0 },
            ] as const
          }
        >
          {(item) => (
            <div class="d-flex flex-items-center">
              <LegendPreview terrain={item.terrain} traffic={item.traffic} />
              <div class="ml-1">
                <dt class="f6 d-inline">{item.name}</dt>
                <dd class="f6 d-inline color-fg-muted mb-0 ml-1">{item.pattern}</dd>
              </div>
            </div>
          )}
        </For>
      </dl>
      <p class="f6 color-fg-muted mb-2">車の円は燃料。店の破線円は現在在庫不明。</p>
      <div class="legend-grid mb-2">
        <For each={['patrol', 'supply'] as const}>
          {(mark) => (
            <div class="d-flex flex-items-center f6">
              <LegendPreview mark={mark} />
              <span class="ml-1">{mark === 'patrol' ? '巡回車' : '補給車'}</span>
            </div>
          )}
        </For>
      </div>
      <h3 class="f6 color-fg-muted mb-1">スポット・系列番号</h3>
      <div class="legend-series">
        <For each={[0, 1, 2, 3, 4, 5]}>
          {(brand) => (
            <div class="d-flex flex-items-center f6">
              <LegendPreview mark={`s${brand}` as Mark} />
              <span class="ml-1">{brand}</span>
            </div>
          )}
        </For>
      </div>
      <p class="f6 color-fg-muted mt-1 mb-0">系列6以降は記号を繰り返します。</p>
      <details class="mt-2">
        <summary class="f6 color-fg-muted">ルール</summary>
        <nav aria-label="Rules by topic" class="f6 mt-1">
          <For each={['map', 'agents', 'spots'] as const}>
            {(section) => (
              <div>
                <RuleLink section={section}>{RULE_SECTIONS[section]}</RuleLink>
              </div>
            )}
          </For>
        </nav>
      </details>
    </Panel>
  );
}
function HoverCard(props: { placement: TooltipPosition; children: JSX.Element }) {
  let element!: HTMLDivElement;
  onMount(() => element.showPopover());
  return (
    <div
      ref={element}
      popover="manual"
      class="hex-inspector border rounded-2 bgColor-default box-shadow-medium p-3"
      role="tooltip"
      aria-label="セル情報"
      style={{
        transform: `translate3d(${props.placement.x}px, ${props.placement.y}px, 0)`,
        width: `${props.placement.width}px`,
        height: `${props.placement.height}px`,
      }}
    >
      {props.children}
    </div>
  );
}
function MapView(props: {
  turn?: number;
  viewport: Viewport;
  scene?: ViewerScene;
  inspect?: (cell: number) => JSX.Element;
  onInspect?: (cell: number | null) => void;
  selectedCell?: number | null;
}) {
  const viewport = untrack(() => props.viewport);
  const [localSelection, setLocalSelection] = createSignal<number | null>(null);
  const selection = () =>
    props.selectedCell === undefined ? localSelection() : props.selectedCell;
  const [layer, setLayer] = createSignal<Awaited<ReturnType<typeof createViewerLayer>> | null>(
    null,
  );
  let gpu!: HTMLCanvasElement,
    fallback!: HTMLCanvasElement,
    stage!: HTMLDivElement,
    overlay!: HTMLCanvasElement;
  const [assetsReady, setAssetsReady] = createSignal(false);
  const overlays = createFrameScheduler(() => {
    if (assetsReady()) drawViewerOverlay(overlay, scene(), viewport.camera(), selection() ?? null);
  });
  onCleanup(overlays.dispose);
  createEffect(() => {
    void assetsReady();
    void props.scene;
    void props.turn;
    void selection();
    void viewport.sample();
    void viewport.padding();
    void viewport.camera();
    void getTheme().mode;
    overlays.invalidate();
  });
  const scene = () =>
    props.scene ??
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
  const frames = createFrameScheduler(draw);
  onCleanup(frames.dispose);
  createEffect(() => {
    // Track inputs here; the deferred draw reads their latest values.
    void assetsReady();
    void layer();
    void props.turn;
    void props.scene;
    void viewport.sample();
    void viewport.padding();
    void viewport.camera();
    void getTheme().mode;
    frames.invalidate();
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
    const observer = new ResizeObserver(() => {
      frames.invalidate();
      overlays.invalidate();
      inspectionFrames.invalidate();
    });
    observer.observe(stage);
    if (props.inspect) {
      const onScroll = () => {
        if (pointer) clearHover();
      };
      window.addEventListener('scroll', onScroll, { capture: true, passive: true });
      onCleanup(() => window.removeEventListener('scroll', onScroll, true));
    }
    onCleanup(() => {
      active = false;
      clearTimeout(timeout);
      observer.disconnect();
      created?.destroy();
    });
  });
  const [hovered, setHovered] = createSignal<number | null>(null);
  const [placement, setPlacement] = createSignal<TooltipPosition>(
    { x: 0, y: 0, width: 320, height: 224 },
    {
      equals: (a, b) => a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height,
    },
  );
  let pointer: { x: number; y: number } | undefined;
  const inspectionFrames = createFrameScheduler(() => {
    if (!pointer) return;
    const rect = stage.getBoundingClientRect();
    const cell = hitTestCell(
      scene(),
      rect.width,
      rect.height,
      viewport.camera(),
      pointer.x - rect.left,
      pointer.y - rect.top,
    );
    setHovered(cell);
    setLocalSelection(cell);
    props.onInspect?.(cell);
    if (cell !== null) {
      const transform = sceneTransform(scene(), rect.width, rect.height, viewport.camera());
      const center = scene().cells[cell]!;
      setPlacement(
        tooltipPosition(
          window.innerWidth,
          window.innerHeight,
          rect.left + center.x * transform.scale + transform.x,
          rect.top + center.y * transform.scale + transform.y,
          (TILE_WORLD_SIZE / 2) * transform.scale,
        ),
      );
    }
  });
  onCleanup(inspectionFrames.dispose);
  createEffect(() => {
    void props.scene;
    void viewport.camera();
    inspectionFrames.invalidate();
  });
  const clearHover = () => {
    pointer = undefined;
    setHovered(null);
    setLocalSelection(null);
    props.onInspect?.(null);
  };
  let drag: { x: number; y: number; moved: boolean } | undefined;
  return (
    <section
      class="map-frame position-relative border rounded-2 overflow-hidden bgColor-muted"
      aria-label="Visualizer"
    >
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
          viewport.setCamera((c) => zoomCamera(c, e.deltaY < 0 ? 1.1 : 1 / 1.1));
        }}
        onPointerDown={(e) => {
          clearHover();
          drag = { x: e.clientX, y: e.clientY, moved: false };
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
            drag = {
              x: e.clientX,
              y: e.clientY,
              moved: drag.moved || Math.hypot(e.clientX - drag.x, e.clientY - drag.y) > 2,
            };
          } else {
            pointer = { x: e.clientX, y: e.clientY };
            inspectionFrames.invalidate();
          }
        }}
        onPointerLeave={clearHover}
        onPointerUp={(e) => {
          const moved = drag?.moved;
          drag = undefined;
          if (moved) {
            clearHover();
            return;
          }
          pointer = { x: e.clientX, y: e.clientY };
          inspectionFrames.invalidate();
        }}
        onPointerCancel={() => {
          drag = undefined;
          clearHover();
        }}
        onKeyDown={(e) => {
          if (e.key === 'Escape') clearHover();
          if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', '+', '-', '0'].includes(e.key)) {
            e.preventDefault();
            e.stopPropagation();
            if (e.key === '0') viewport.reset();
            else if (e.key === '+' || e.key === '-')
              viewport.setCamera((c) => zoomCamera(c, e.key === '+' ? 1.1 : 1 / 1.1));
            else
              viewport.setCamera((c) => ({
                ...c,
                x: c.x + (e.key === 'ArrowLeft' ? 12 : e.key === 'ArrowRight' ? -12 : 0),
                y: c.y + (e.key === 'ArrowUp' ? 12 : e.key === 'ArrowDown' ? -12 : 0),
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
        <canvas
          ref={overlay}
          class="viewer-overlay"
          data-selected-cell={selection() ?? undefined}
          aria-hidden="true"
        />
      </div>
      <Show when={hovered() !== null && props.inspect}>
        <HoverCard placement={placement()}>{props.inspect?.(hovered()!)}</HoverCard>
      </Show>
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
    if (![' ', 'ArrowLeft', 'ArrowRight', 'Home', 'End', '1', '2', '4', '8'].includes(e.key))
      return;
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
    if (['1', '2', '4', '8'].includes(e.key)) store.setRate(Number(e.key) as PlaybackRate);
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
                <Ratio
                  value={store.state().currentStep + 1}
                  total={store.stepCount()}
                  label="Replay steps"
                />
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
            <div>
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
          </div>
          <RadioGroup.Root
            class="mb-3"
            aria-label="Playback speed"
            value={String(store.state().rate)}
            onChange={(v) => store.setRate(Number(v) as PlaybackRate)}
          >
            <RadioGroup.Label class="d-block text-bold mb-2">Playback speed</RadioGroup.Label>
            <div class="playback-rates">
              <For each={PLAYBACK_RATES}>
                {(rate) => (
                  <RadioGroup.Item
                    value={String(rate)}
                    class="btn btn-block"
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
              <kbd>Space</kbd>: Play / Pause · <kbd>←</kbd> <kbd>→</kbd>: ステップ · <kbd>Home</kbd>{' '}
              / <kbd>End</kbd> · <kbd>1</kbd> <kbd>2</kbd> <kbd>4</kbd> <kbd>8</kbd>: Playback speed
              · <kbd>Escape</kbd>: Cancel scrub
            </p>
          </details>
          <p class="f6 mt-3 mb-0">
            <RuleLink section="overview">{RULE_SECTIONS.overview}</RuleLink> ·{' '}
            <RuleLink section="time">{RULE_SECTIONS.time}</RuleLink>
          </p>
        </Panel>
        <ViewControls viewport={viewport} />

        <Panel title="Score (demo)">
          <DataTable
            label="Score (demo)"
            headers={['チーム', 'Score (demo)']}
            columns="minmax(0, 1fr) minmax(0, 1fr)"
          >
            <For each={store.currentFrame()?.scores}>
              {(entry) => (
                <DataRow>
                  <DataCell heading>{entry[0].value}</DataCell>
                  <DataCell end>{entry[1]}</DataCell>
                </DataRow>
              )}
            </For>
          </DataTable>
        </Panel>
        <Legend />
      </aside>
    </div>
  );
}

function OfficialExamplePage() {
  const [frame, setFrame] = createSignal(0);
  const [playing, setPlaying] = createSignal(false);
  const viewports = q6.agents.map(() => createViewport());
  onMount(() => {
    const timer = setInterval(() => {
      if (!playing()) return;
      if (frame() >= q6.phases.length - 1) setPlaying(false);
      else setFrame((value) => value + 1);
    }, 600);
    onCleanup(() => clearInterval(timer));
  });
  const scene = (index: number): ViewerScene => ({
    cells: [0, 1, 2, 3].map((x) => ({
      x: (x * 52 * 23) / 30,
      y: 0,
      terrain: x < 2 ? 'road' : 'plain',
    })),
    marks: [
      ...(q6.agents[index]!.positions[frame()] === 2 ? [] : [{ cell: 2, mark: 's0' as const }]),
      {
        cell: q6.agents[index]!.positions[frame()]!,
        mark: q6.agents[index]!.kind === 0 ? 'patrol' : 'supply',
      },
    ],
  });
  return (
    <div class="PageLayout-columns official-example">
      <div class="PageLayout-content">
        <For each={q6.agents}>
          {(agent, index) => (
            <div class="d-flex flex-items-center mb-2">
              <div class="mr-3" style={{ width: '140px', 'flex-shrink': 0 }}>
                <strong>{agent.name}</strong>
                <div class="f6 text-mono" data-testid={`q6-agent-${index()}`}>
                  セル {agent.positions[frame()]}
                  <Show when={agent.fuel}> · 燃料 {agent.fuel?.[frame()]}</Show>
                  <Show when={agent.udons}> · {agent.udons?.[frame()]}玉</Show>
                </div>
              </div>
              <div style={{ flex: 1, 'min-width': 0 }}>
                <MapView viewport={viewports[index()]!} scene={scene(index())} />
              </div>
            </div>
          )}
        </For>
      </div>
      <aside class="PageLayout-pane" aria-label="Official Q6 example">
        <Panel title="公式 Q6 行動例">
          <Row label="フェーズ" value={<span data-testid="q6-phase">{q6.phases[frame()]}</span>} />
          <Row
            label="Frame"
            value={<Ratio value={frame() + 1} total={q6.phases.length} label="Example frames" />}
          />
          <input
            class="width-full my-3"
            type="range"
            min="0"
            max={q6.phases.length - 1}
            value={frame()}
            aria-label="Example phase"
            onInput={(event) => {
              setPlaying(false);
              setFrame(Number(event.currentTarget.value));
            }}
          />
          <Button
            label={playing() ? 'Pause example' : 'Play example'}
            icon={playing() ? 'pause' : 'triangle-fill'}
            block
            onClick={() => {
              if (frame() === q6.phases.length - 1) setFrame(0);
              setPlaying((value) => !value);
            }}
          />
          <div class="mt-2">
            <Button
              label="Reset example"
              icon="sync"
              block
              onClick={() => {
                setPlaying(false);
                setFrame(0);
              }}
            />
          </div>
        </Panel>
        <Panel title="セル 0 → 1 → 2 → 3">
          <p class="f6">
            セル0・1は道路、セル2・3は平地。セル2に在庫4のスポットがあり、巡回車の最大燃料は3です。
          </p>
          <p class="f6 mb-0">
            同じ4セルを車両ごとに分けて表示しています。補給車の燃料欄は原表にありません。
          </p>
        </Panel>
        <Panel title="Source">
          <a href={q6.source} target="_blank" rel="noopener noreferrer">
            公式補足資料 p.4 全数値表
          </a>
          <p class="f6 color-fg-muted mt-2">
            位置・燃料・獲得玉数は表の39列をそのまま再生します。補間やシミュレーションは行いません。
          </p>
          <p class="f6 color-fg-muted mb-0">
            公式の説明例です。実試合の記録ではありません。処理順は原表の「補給 →
            獲得」を保持しています。
          </p>
        </Panel>
        <Legend />
      </aside>
    </div>
  );
}

const formatDuration = (seconds: number) =>
  seconds >= 60 ? `${Math.floor(seconds / 60)}m ${seconds % 60}s` : `${seconds}s`;
const terrainLabels = ['平地', '道路', '山', '池'];
const trafficLabels = ['順調', '混雑', '渋滞'];
const kindLabel = (kind: number) => (kind === 0 ? '巡回車' : '補給車');
const fuelLabel = (fuel: number, kind: number, limit?: number) =>
  kind === 1 ? (
    '対象外'
  ) : limit == null ? (
    fuel
  ) : (
    <Ratio value={fuel} total={limit} label="Fuel" meter />
  );
function VehicleKind(props: { kind: number }) {
  return (
    <span class="vehicle-kind d-inline-flex flex-items-center">
      <Show when={props.kind === 0 || props.kind === 1}>
        <LegendPreview mark={props.kind === 0 ? 'patrol' : 'supply'} />
      </Show>
      <span>{kindLabel(props.kind)}</span>
    </span>
  );
}
function CellDetails(props: { match: Match; cell: number; compact?: boolean }) {
  const width = () => props.match.setting.map.width;
  const spots = () => props.match.setting.spots.filter((spot) => spot.pos === props.cell);
  const traffic = () => props.match.state.traffics.find((road) => road.pos === props.cell);
  const occupants = () =>
    [
      ...props.match.state.agents.map((agent, index) => ({ agent, label: `自車 #${index + 1}` })),
      ...props.match.state.others.flatMap((team, teamIndex) =>
        team.agents.map((agent, index) => ({
          agent,
          label: `相手 ${team.id ?? teamIndex + 1} 車 #${index + 1}`,
        })),
      ),
    ].filter(({ agent }) => agent.pos === props.cell);
  return (
    <div class="cell-details">
      <h3 class="f6 text-bold mb-1">
        セル {props.cell}（{props.cell % width()}, {Math.floor(props.cell / width())}）
      </h3>
      <div class="cell-metadata f6 mb-2">
        <p class="mb-1">
          地形:{' '}
          {
            terrainLabels[
              props.match.setting.map.cells[Math.floor(props.cell / width())]?.[
                props.cell % width()
              ] ?? -1
            ]
          }
          <Show when={props.compact && traffic()}>
            {' '}
            · {trafficLabels[traffic()!.status] ?? '不明'}
          </Show>
        </p>
        <Show when={!props.compact}>
          <p class="mb-1">交通: {traffic() ? (trafficLabels[traffic()!.status] ?? '不明') : '—'}</p>
        </Show>
        <p class="mb-0">
          {spots().length
            ? spots()
                .map((spot) => `系列 ${spot.brand} · 最大在庫 ${spot.stocks}`)
                .join(' / ')
            : 'スポットなし'}
        </p>
      </div>
      <div class="cell-occupants">
        <DataTable
          label="セル内の車両"
          headers={['車両', '種類', '燃料']}
          columns="minmax(0, 1fr) 4.5rem minmax(6rem, 1fr)"
        >
          <For
            each={occupants()}
            fallback={
              <DataRow>
                <DataCell>車両なし</DataCell>
                <DataCell>—</DataCell>
                <DataCell>—</DataCell>
              </DataRow>
            }
          >
            {({ agent, label }) => (
              <DataRow>
                <DataCell heading title={label}>
                  {label}
                </DataCell>
                <DataCell>
                  <VehicleKind kind={agent.kind} />
                </DataCell>
                <DataCell end>
                  {fuelLabel(agent.fuel, agent.kind, props.match.setting.fuelLimits)}
                </DataCell>
              </DataRow>
            )}
          </For>
        </DataTable>
      </div>
    </div>
  );
}
function LivePage() {
  const viewport = createViewport();
  viewport.setRenderer('Waiting');
  const [snapshot, setSnapshot] = createSignal<Match | null>(null),
    [received, setReceived] = createSignal(0),
    [now, setNow] = createSignal(Date.now());
  const liveScene = createMemo(() => (snapshot() ? matchScene(snapshot()!) : undefined));
  const [selectedCell, setSelectedCell] = createSignal<number | null>(null);
  const controller = new AbortController();
  let active = true;
  let polling = false;
  const started = Date.now();
  onMount(() => {
    const poll = async () => {
      if (polling) return;
      polling = true;
      const value = await fetchMatch(controller.signal);
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
          : 'clock';
  const statusColor = () =>
    unavailable() || stale()
      ? 'color-fg-attention'
      : snapshot()?.status === 'running' || snapshot()?.status === 'completed'
        ? 'color-fg-success'
        : 'color-fg-muted';
  return (
    <div class="PageLayout-columns">
      <div class="PageLayout-content">
        <Show
          when={snapshot()}
          fallback={
            <section
              class="map-frame border rounded-2 overflow-hidden bgColor-muted"
              aria-label="Visualizer"
              aria-busy="true"
            >
              <div class="map-stage d-flex flex-items-center flex-justify-center">
                <p class="color-fg-muted m-3" role="status">
                  試合データを待っています。
                </p>
              </div>
            </section>
          }
        >
          {(match) => (
            <MapView
              viewport={viewport}
              scene={liveScene()!}
              inspect={(cell) => <CellDetails match={match()} cell={cell} compact />}
              onInspect={setSelectedCell}
              selectedCell={selectedCell()}
            />
          )}
        </Show>
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
            label="日"
            value={
              snapshot() ? (
                <Ratio
                  value={snapshot()!.day + 1}
                  total={snapshot()!.daysTotal}
                  label="Match days"
                />
              ) : (
                '—'
              )
            }
          />
          <Row label="ステップ数" value={snapshot()?.setting.daySteps[snapshot()!.day] ?? '—'} />
          <Row label="燃料上限" value={snapshot()?.setting.fuelLimits ?? '—'} />
          <Row
            label="回答締切"
            value={
              snapshot()
                ? new Date(snapshot()!.state.endsAt * 1000).toLocaleTimeString(undefined, {
                    hour12: false,
                  })
                : '—'
            }
          />
          <Row
            label="残り時間"
            value={
              snapshot()
                ? formatDuration(
                    Math.max(0, Math.ceil((snapshot()!.state.endsAt * 1000 - now()) / 1000)),
                  )
                : '—'
            }
          />
          <Row label="Server" value={<code>{snapshot()?.server ?? '—'}</code>} />
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
        <Show when={snapshot()}>
          {(match) => (
            <>
              <Panel title="車両">
                <p class="f6 color-fg-muted mb-2">日初めの状態 · 車両番号はAPIの配列順</p>
                <DataTable
                  label="自チームの車両"
                  headers={['車両', '燃料', 'セル']}
                  columns="minmax(7rem, 1fr) minmax(7rem, 1.2fr) 3.5rem"
                  scroll
                >
                  <For each={match().state.agents}>
                    {(agent, index) => (
                      <DataRow>
                        <DataCell heading>
                          #{index() + 1} <VehicleKind kind={agent.kind} />
                        </DataCell>
                        <DataCell end>
                          {fuelLabel(agent.fuel, agent.kind, match().setting.fuelLimits)}
                        </DataCell>
                        <DataCell end>
                          <button class="btn-link" onClick={() => setSelectedCell(agent.pos)}>
                            {agent.pos}
                          </button>
                        </DataCell>
                      </DataRow>
                    )}
                  </For>
                </DataTable>
                <details class="mt-3">
                  <summary>他チームの車両</summary>
                  <For each={match().state.others}>
                    {(team, teamIndex) => (
                      <div class="mt-2">
                        <h3 class="f6 text-bold">相手 {team.id ?? teamIndex() + 1}</h3>
                        <DataTable
                          label={`相手 ${team.id ?? teamIndex() + 1} の車両`}
                          headers={['車両', '燃料', 'セル']}
                          columns="minmax(7rem, 1fr) minmax(7rem, 1.2fr) 3.5rem"
                          scroll
                        >
                          <For each={team.agents}>
                            {(agent, index) => (
                              <DataRow>
                                <DataCell heading>
                                  #{index() + 1} <VehicleKind kind={agent.kind} />
                                </DataCell>
                                <DataCell end>
                                  {fuelLabel(agent.fuel, agent.kind, match().setting.fuelLimits)}
                                </DataCell>
                                <DataCell end>{agent.pos}</DataCell>
                              </DataRow>
                            )}
                          </For>
                        </DataTable>
                      </div>
                    )}
                  </For>
                </details>
              </Panel>
              <Panel title="セル情報">
                <label class="d-block f6 mb-2" for="inspect-cell">
                  セル番号（ホバー・タップでも選択）
                </label>
                <input
                  id="inspect-cell"
                  class="form-control width-full mb-3"
                  type="number"
                  min="0"
                  max={match().setting.map.width * match().setting.map.height - 1}
                  value={selectedCell() ?? ''}
                  onInput={(e) => {
                    const cell = e.currentTarget.valueAsNumber;
                    setSelectedCell(
                      Number.isInteger(cell) &&
                        cell >= 0 &&
                        cell < match().setting.map.width * match().setting.map.height
                        ? cell
                        : null,
                    );
                  }}
                />
                <Show
                  when={selectedCell() !== null}
                  fallback={<p class="f6 color-fg-muted mb-0">ヘキサを選ぶと詳細を表示します。</p>}
                >
                  <CellDetails match={match()} cell={selectedCell()!} />
                </Show>
                <p class="f6 color-fg-muted mt-3 mb-0">
                  最大在庫は設定値です。現在在庫・獲得玉数・得点はこのAPIでは取得できません。
                </p>
              </Panel>
            </>
          )}
        </Show>
        <ViewControls viewport={viewport} live />
        <Panel title="Source">
          <Row label="Setting" value={<code>/api/match/setting</code>} />
          <Row label="State" value={<code>/api/match</code>} />
          <Row label="Polling" value="1s" />
          <Show when={snapshot()}>
            <details class="mt-3">
              <summary>JSON</summary>
              <pre
                class="match-json text-mono f6 bgColor-muted border rounded-2 p-2 mt-2 mb-0"
                tabindex="0"
                aria-label="試合データ JSON"
              >
                <code>
                  {JSON.stringify(
                    { setting: snapshot()?.setting, state: snapshot()?.state },
                    null,
                    2,
                  )}
                </code>
              </pre>
            </details>
          </Show>
          <p class="color-fg-muted f6 mt-3 mb-0">
            試合設定と状態を取得し、地形・スポット・両チームの車両を表示します。
          </p>
        </Panel>
        <Legend />
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
    ['/examples/q6', '公式例', 'history'],
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
        <h1 class="f5 mr-3 mb-0">ヘキサうどん</h1>
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
      <Route path="/examples/q6" component={OfficialExamplePage} />
      <Route path="*" component={Home} />
    </Router>
  );
}
