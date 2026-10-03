import { A, Route, Router, useLocation } from '@solidjs/router';
import { For, Show, createEffect, createSignal, onCleanup, onMount, type JSX } from 'solid-js';
import * as RadioGroup from '@kobalte/core/radio-group';
import { createReplayStore, type PlaybackRate } from '@/stores/replay';
import { getTheme, toggleTheme } from '@/stores/theme';
import { fetchLiveSnapshot, type LiveSnapshot } from '@/lib/live';
import icons from '@/viewer-icons.json';
import marks from '@/viewer-marks.json';
import './viewer.css';

const Icon = (props: { name: keyof typeof icons }) => (
  <svg
    width="16"
    height="16"
    viewBox="0 0 16 16"
    fill="currentColor"
    aria-hidden="true"
    innerHTML={icons[props.name]}
  />
);
const Button = (props: {
  label: string;
  icon: keyof typeof icons;
  onClick: () => void;
  disabled?: boolean;
  testid?: string;
}) => (
  <button
    class="btn"
    aria-label={props.label}
    title={props.label}
    onClick={props.onClick}
    disabled={props.disabled}
    data-testid={props.testid}
  >
    <Icon name={props.icon} />
  </button>
);
const Box = (props: { title: string; children: JSX.Element }) => (
  <section class="Box">
    <h2 class="Box-header p-3">{props.title}</h2>
    <div class="Box-body p-3">{props.children}</div>
  </section>
);
const Row = (props: { label: string; value: JSX.Element }) => (
  <div class="info-row">
    <span>{props.label}</span>
    <strong>{props.value}</strong>
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
function MapView(props: { turn?: number }) {
  const [zoom, setZoom] = createSignal(1),
    [x, setX] = createSignal(0),
    [y, setY] = createSignal(0);
  let drag: { x: number; y: number } | undefined;
  const reset = () => {
    setZoom(1);
    setX(0);
    setY(0);
  };
  return (
    <div class="map-instrument">
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
          if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', '+', '-', '0'].includes(e.key)) {
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
                    fill={`var(--data-${terrainHue[cell.terrain]}-color-muted)`}
                    stroke={`var(--data-${terrainHue[cell.terrain]}-color-emphasis)`}
                  />
                  <svg
                    x="-8"
                    y="-8"
                    width="16"
                    height="16"
                    viewBox="0 0 16 16"
                    style={{ color: `var(--data-${terrainHue[cell.terrain]}-color-emphasis)` }}
                    innerHTML={marks[cell.terrain]}
                  />
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
                innerHTML={marks[`s${n}` as keyof typeof marks]}
              />
            )}
          </For>
          <svg
            x={40 + ((props.turn ?? 0) % 6) * 40}
            y="132"
            width="20"
            height="20"
            viewBox="0 0 16 16"
            style={{ color: 'var(--data-pine-color-emphasis)' }}
            innerHTML={marks.patrol}
          />
          <svg
            x="220"
            y="100"
            width="20"
            height="20"
            viewBox="0 0 16 16"
            style={{ color: 'var(--data-pink-color-emphasis)' }}
            innerHTML={marks.supply}
          />
        </g>
      </svg>
      <div class="map-footer">
        <span>Fixture · 8 × 6 · SVG</span>
        <span>Drag to pan · wheel / + − to zoom · arrows · 0 reset</span>
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
    resume = store.state().isPlaying;
    setScrubbing(true);
    store.pause();
  };
  const end = () => {
    if (!scrubbing()) return;
    setScrubbing(false);
    if (resume && store.state().currentStep < store.stepCount() - 1) store.play();
  };
  return (
    <>
      <div class="route-heading">
        <h1>Replays</h1>
        <span class="Label">Demo match · alice / bob</span>
      </div>
      <div class="workspace">
        <div>
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
          <MapView turn={store.state().currentStep} />
          <label class="scrub-label" for="replay-position">
            Replay position
          </label>
          <input
            id="replay-position"
            class="scrub"
            data-testid="replay-scrub"
            type="range"
            min="0"
            max="4"
            value={store.state().currentStep}
            aria-valuetext={`Step ${store.state().currentStep + 1} of 5`}
            onPointerDown={begin}
            onPointerUp={end}
            onPointerCancel={end}
            onBlur={end}
            onKeyDown={begin}
            onKeyUp={end}
            onInput={(e) => store.setStep(Number(e.currentTarget.value))}
          />
        </div>
        <aside>
          <Box title="Score">
            <table class="score-table">
              <caption>Scores — demo-match</caption>
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
          </Box>
          <Box title="Replay">
            <Row label="Step" value={`${store.state().currentStep + 1} / 5`} />
            <Row label="Status" value={store.state().status} />
            <Row label="Rate" value={`${store.state().rate}×`} />
          </Box>
        </aside>
      </div>
    </>
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
  const stale = () =>
    !!snapshot() && now() - Math.min(received(), snapshot()!.lastUpdateUnix * 1000) >= 5000;
  return (
    <>
      <div class="route-heading">
        <h1>Live</h1>
        <span
          class="Label"
          role="status"
          data-testid="live-status-badge"
          data-status={snapshot()?.status ?? 'connecting'}
        >
          {unavailable() ? 'No data' : stale() ? 'Stale' : (snapshot()?.status ?? 'Connecting')}
        </span>
      </div>
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
      <div class="workspace">
        <MapView />
        <aside>
          <Box title="Match state">
            <Row
              label="Day"
              value={
                snapshot()?.day === null
                  ? '—'
                  : `${(snapshot()?.day ?? -1) + 1} / ${snapshot()?.daysTotal ?? '—'}`
              }
            />
            <Row label="Revisions" value={snapshot()?.revisions.join(', ') || '—'} />
            <Row label="Solver" value={snapshot()?.solver ?? '—'} />
            <Row label="Server" value={snapshot()?.server ?? '—'} />
            <Row
              label="Last update"
              value={
                snapshot() ? new Date(snapshot()!.lastUpdateUnix * 1000).toLocaleTimeString() : '—'
              }
            />
          </Box>
          <Box title="Map source">
            <p class="color-fg-muted">
              Fixture preview. The CLI snapshot carries match status, not map or agent positions.
            </p>
            <Row label="Poll interval" value="1 second" />
          </Box>
        </aside>
      </div>
    </>
  );
}
function SolversPage() {
  return (
    <>
      <div class="route-heading">
        <h1>Solvers</h1>
        <span class="Label">Fixture preview</span>
      </div>
      <div class="workspace">
        <MapView />
        <aside>
          <Box title="Map fixture">
            <Row label="Dimensions" value="8 × 6" />
            <Row label="Cells" value="48" />
            <Row label="Spot series" value="6" />
            <Row label="Renderer" value="SVG" />
          </Box>
          <Box title="Legend">
            <For each={['plain', 'mountain', 'water', 'road', 'patrol', 'supply'] as const}>
              {(name) => (
                <div class="legend-row">
                  <svg
                    width="16"
                    height="16"
                    viewBox="0 0 16 16"
                    fill="currentColor"
                    innerHTML={marks[name]}
                  />
                  <span>{name}</span>
                </div>
              )}
            </For>
          </Box>
        </aside>
      </div>
    </>
  );
}
function Home() {
  return (
    <>
      <div class="route-heading">
        <h1>Match viewer</h1>
        <span class="Label">procon-2026</span>
      </div>
      <div class="workspace">
        <MapView />
        <aside>
          <Box title="Workspace">
            <A class="info-row" href="/live">
              Live match
              <Icon name="broadcast" />
            </A>
            <A class="info-row" href="/replays">
              Replay analysis
              <Icon name="history" />
            </A>
            <A class="info-row" href="/solvers">
              Map &amp; solvers
              <Icon name="beaker" />
            </A>
          </Box>
          <Box title="About">
            <p>全国高専プロコン 競技部門</p>
            <p class="color-fg-muted">Inspect match state, replay steps and the hex map.</p>
          </Box>
        </aside>
      </div>
    </>
  );
}
function Shell(props: { children?: JSX.Element }) {
  const location = useLocation();
  return (
    <div class="app-shell">
      <a class="skip-link" href="#main">
        Skip to content
      </a>
      <header class="app-header" data-testid="app-header">
        <A href="/" class="brand">
          <Icon name="home" />
          <b>procon-2026</b>
        </A>
        <span class="color-fg-muted">Match viewer</span>
        <button class="btn btn-sm ml-auto" onClick={toggleTheme} aria-label="Toggle theme">
          <Icon name={getTheme().mode === 'dark' ? 'sun' : 'moon'} />
        </button>
      </header>
      <nav class="app-sidebar" aria-label="Main navigation">
        <For
          each={
            [
              ['/', 'Home', 'home'],
              ['/replays', 'Replays', 'history'],
              ['/solvers', 'Solvers', 'beaker'],
              ['/live', 'Live', 'broadcast'],
            ] as const
          }
        >
          {(item) => (
            <A
              href={item[0]}
              end={item[0] === '/'}
              class="nav-item"
              classList={{ selected: location.pathname === item[0] }}
            >
              <Icon name={item[2]} />
              {item[1]}
            </A>
          )}
        </For>
      </nav>
      <main id="main" class="app-main">
        {props.children}
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
