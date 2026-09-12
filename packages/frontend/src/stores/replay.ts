/**
 * Replay playback store.
 *
 * Holds the transport state for a single loaded `Replay` instance:
 *   - `currentStep`  : index of the step currently shown.
 *   - `isPlaying`    : whether the transport is auto-advancing.
 *   - `rate`         : playback rate multiplier (`1 | 2 | 4`).
 *
 * Pure reactive state + action helpers (`play`, `pause`, `setStep`,
 * `setRate`). The actual `setInterval` driver lives inside `ReplayViewer`
 * because it needs to be tied to the component's reactive root + cleaned up
 * via `onCleanup`. Splitting state from the timer keeps this module trivially
 * unit-testable and reusable from non-component contexts (e.g. minimap).
 */
import { createSignal } from 'solid-js';

import type { GameState } from '@/lib/protocol';

export type PlaybackRate = 1 | 2 | 4;

export const PLAYBACK_RATES: ReadonlyArray<PlaybackRate> = [1, 2, 4];

/**
 * A persisted recording of a match. Each `step` is a single `GameState`
 * snapshot at turn `n`. The viewer scrubs through `steps` to replay the match.
 */
export interface Replay {
  readonly id: string;
  readonly title?: string;
  readonly steps: ReadonlyArray<GameState>;
}

export interface PlaybackSnapshot {
  readonly currentStep: number;
  readonly isPlaying: boolean;
  readonly rate: PlaybackRate;
  readonly status: 'idle' | 'playing' | 'paused' | 'finished';
}

export interface ReplayStore {
  /** Reactive accessor for the current playback snapshot. */
  readonly state: () => PlaybackSnapshot;
  /** Reactive accessor for the current `GameState` (the step being shown). */
  readonly currentFrame: () => GameState | null;
  /** Total number of steps in the loaded replay. */
  readonly stepCount: () => number;
  /** Begin auto-advance. Rewinds if we were at the end. */
  play: () => void;
  /** Stop auto-advance. Leaves `currentStep` as is. */
  pause: () => void;
  /** Jump to an absolute step index (clamped to `[0, stepCount - 1]`). */
  setStep: (step: number) => void;
  /** Move one step forward. Clamps at the end. */
  stepForward: () => void;
  /** Move one step backward. Clamps at 0. */
  stepBackward: () => void;
  /** Update the playback rate. No-op for unsupported rates. */
  setRate: (rate: PlaybackRate) => void;
  /** Replace the loaded replay and reset transport. */
  load: (replay: Replay) => void;
  /** Reset transport without replacing the replay. */
  reset: () => void;
}

/**
 * Create a fresh store. Each call returns an isolated store so multiple
 * viewers on the same page do not share transport state. The store starts
 * empty; call `load(replay)` (typically inside a `createEffect`) to attach
 * a `Replay`.
 */
export function createReplayStore(): ReplayStore {
  const [replay, setReplay] = createSignal<Replay | null>(null);
  const [currentStep, setCurrentStep] = createSignal(0);
  const [isPlaying, setIsPlaying] = createSignal(false);
  const [rate, setRateSignal] = createSignal<PlaybackRate>(1);

  const clamp = (n: number): number => {
    const max = Math.max(0, stepCount() - 1);
    if (!Number.isFinite(n)) return 0;
    if (n < 0) return 0;
    if (n > max) return max;
    return Math.trunc(n);
  };

  const stepCount = (): number => replay()?.steps.length ?? 0;

  const currentFrame = (): GameState | null => {
    const r = replay();
    if (!r) return null;
    return r.steps[currentStep()] ?? null;
  };

  const status = (): PlaybackSnapshot['status'] => {
    if (isPlaying()) return 'playing';
    const last = stepCount() - 1;
    if (last > 0 && currentStep() >= last) return 'finished';
    if (currentStep() === 0 && !isPlaying()) return 'idle';
    return 'paused';
  };

  const state = (): PlaybackSnapshot => ({
    currentStep: currentStep(),
    isPlaying: isPlaying(),
    rate: rate(),
    status: status(),
  });

  const setStep = (step: number): void => {
    setCurrentStep(clamp(step));
  };

  const play = (): void => {
    if (stepCount() === 0) return;
    if (currentStep() >= stepCount() - 1) {
      setCurrentStep(0);
    }
    setIsPlaying(true);
  };

  const pause = (): void => {
    setIsPlaying(false);
  };

  const stepForward = (): void => {
    setStep(currentStep() + 1);
  };

  const stepBackward = (): void => {
    setStep(currentStep() - 1);
  };

  const setRateAction = (next: PlaybackRate): void => {
    if (!PLAYBACK_RATES.includes(next)) return;
    setRateSignal(next);
  };

  const load = (next: Replay): void => {
    setReplay(next);
    setCurrentStep(0);
    setIsPlaying(false);
    setRateSignal(1);
  };

  const reset = (): void => {
    setCurrentStep(0);
    setIsPlaying(false);
    setRateSignal(1);
  };

  return {
    state,
    currentFrame,
    stepCount,
    play,
    pause,
    setStep,
    stepForward,
    stepBackward,
    setRate: setRateAction,
    load,
    reset,
  };
}
