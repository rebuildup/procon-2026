import type { Component } from 'solid-js';
import { createEffect, createMemo, For, onCleanup } from 'solid-js';

import { MapViewer } from '@/components/MapViewer';
import { createReplayStore, PLAYBACK_RATES, type PlaybackRate, type Replay } from '@/stores/replay';

export interface ReplayViewerProps {
  replay: Replay;
}

/**
 * `ReplayViewer` — playback UI for a serialized match.
 *
 * Composes:
 *   - scrub bar (`<input type="range">`) for jumping to a step
 *   - transport controls (play/pause + step ± buttons)
 *   - rate selector (1× / 2× / 4×)
 *   - `<MapViewer>` showing the state at the current step
 *
 * The store owns reactive transport state; this component owns the wall-clock
 * timer that auto-advances the step when playing.
 */
export const ReplayViewer: Component<ReplayViewerProps> = (props) => {
  const store = createReplayStore();

  // Load the supplied replay (and any swapped replays). Reading `props.replay`
  // here keeps the access inside a tracked scope, satisfying the
  // `solid/reactivity` lint rule.
  createEffect(() => {
    store.load(props.replay);
  });

  // Auto-advance timer. Re-arms on rate change / play toggle so the period
  // reflects the current playback speed.
  let timerId: ReturnType<typeof setInterval> | null = null;
  const periodMs = (rate: PlaybackRate): number => Math.max(50, Math.round(1000 / rate));

  createEffect(() => {
    if (timerId !== null) {
      clearInterval(timerId);
      timerId = null;
    }
    if (!store.state().isPlaying) return;
    const ms = periodMs(store.state().rate);
    timerId = setInterval(() => {
      if (store.currentFrame() === null) return;
      const last = store.stepCount() - 1;
      const next = store.state().currentStep + 1;
      if (next > last) {
        store.pause();
        return;
      }
      store.setStep(next);
    }, ms);
  });

  onCleanup(() => {
    if (timerId !== null) {
      clearInterval(timerId);
      timerId = null;
    }
  });

  const scrubValue = createMemo<number>(() => store.state().currentStep);
  const maxStep = createMemo<number>(() => Math.max(0, store.stepCount() - 1));

  return (
    <section class="replay-viewer" data-testid="replay-viewer">
      <header class="replay-viewer__header">
        <h3 class="replay-viewer__title">{props.replay.title ?? `Replay ${props.replay.id}`}</h3>
        <p class="replay-viewer__meta">
          step <span data-testid="replay-current-step">{scrubValue() + 1}</span> /{' '}
          {store.stepCount()}
        </p>
      </header>

      <div class="replay-viewer__map">
        <MapViewer state={store.currentFrame()} />
      </div>

      <div class="replay-viewer__transport" role="group" aria-label="Replay transport">
        <button
          type="button"
          class="replay-viewer__btn"
          data-testid="replay-step-back"
          aria-label="Step backward"
          disabled={scrubValue() <= 0}
          onClick={() => store.stepBackward()}
        >
          ◀◀
        </button>
        <button
          type="button"
          class="replay-viewer__btn"
          data-testid="replay-toggle"
          aria-label={store.state().isPlaying ? 'Pause' : 'Play'}
          onClick={() => (store.state().isPlaying ? store.pause() : store.play())}
        >
          {store.state().isPlaying ? 'Pause' : 'Play'}
        </button>
        <button
          type="button"
          class="replay-viewer__btn"
          data-testid="replay-step-forward"
          aria-label="Step forward"
          disabled={scrubValue() >= maxStep()}
          onClick={() => store.stepForward()}
        >
          ▶▶
        </button>

        <div class="replay-viewer__rate" role="group" aria-label="Playback rate">
          <For each={PLAYBACK_RATES}>
            {(r) => (
              <button
                type="button"
                class="replay-viewer__rate-btn"
                data-testid={`replay-rate-${r}`}
                aria-pressed={store.state().rate === r}
                onClick={() => store.setRate(r)}
              >
                {r}×
              </button>
            )}
          </For>
        </div>
      </div>

      <label class="replay-viewer__scrub-label" for="replay-scrub">
        Scrub
        <input
          id="replay-scrub"
          class="replay-viewer__scrub"
          data-testid="replay-scrub"
          type="range"
          min={0}
          max={maxStep()}
          step={1}
          value={scrubValue()}
          aria-valuemin={0}
          aria-valuemax={maxStep()}
          aria-valuenow={scrubValue()}
          onInput={(event) => {
            const next = Number((event.currentTarget as HTMLInputElement).value);
            store.setStep(next);
          }}
        />
      </label>
    </section>
  );
};
