import { afterEach, describe, expect, test } from 'bun:test';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { jsx as hyperscriptJsx } from 'solid-js/h/jsx-runtime';

import { ReplayViewer } from '@/components/ReplayViewer';
import type { GameState, MatchId, PlayerId, Score } from '@/lib/protocol';
import type { Replay } from '@/stores/replay';

function player(value: string): PlayerId {
  return { type: 'player', value };
}
function match(value: string): MatchId {
  return { type: 'match', value };
}

function buildReplay(steps: number): Replay {
  const list: GameState[] = [];
  for (let turn = 0; turn < steps; turn += 1) {
    const scores: ReadonlyArray<readonly [PlayerId, Score]> = [
      [player('p1'), turn],
      [player('p2'), turn * 2],
    ];
    list.push({ matchId: match('m'), turn, scores });
  }
  return { id: 'fixture', title: 'Fixture replay', steps: list };
}

interface RenderHandle {
  root: HTMLDivElement;
  cleanup: () => void;
}

function renderViewer(replay: Replay): RenderHandle {
  const root = document.createElement('div');
  document.body.appendChild(root);
  const view = (): JSX.Element =>
    hyperscriptJsx(ReplayViewer, { replay }) as unknown as JSX.Element;
  const dispose: () => void = render(view, root);
  return {
    root,
    cleanup: () => {
      dispose();
      root.remove();
    },
  };
}

afterEach(() => {
  // Make sure stray intervals from a previous test don't leak.
  // `bun --conditions=browser` + happy-dom share the global window object,
  // so clearing pending intervals is mostly defensive — but we keep the hook
  // so future tests can add per-test cleanup without touching this file.
});

describe('ReplayViewer', () => {
  test('renders the scrub bar and transport controls', () => {
    const handle = renderViewer(buildReplay(4));
    try {
      const scrub = handle.root.querySelector<HTMLInputElement>('[data-testid="replay-scrub"]');
      expect(scrub).not.toBeNull();
      expect(scrub?.tagName.toLowerCase()).toBe('input');
      expect(scrub?.type).toBe('range');
      expect(Number(scrub?.max)).toBe(3);

      expect(handle.root.querySelector('[data-testid="replay-toggle"]')).not.toBeNull();
      expect(handle.root.querySelector('[data-testid="replay-step-back"]')).not.toBeNull();
      expect(handle.root.querySelector('[data-testid="replay-step-forward"]')).not.toBeNull();
      expect(handle.root.querySelector('[data-testid="replay-rate-1"]')).not.toBeNull();
      expect(handle.root.querySelector('[data-testid="replay-rate-2"]')).not.toBeNull();
      expect(handle.root.querySelector('[data-testid="replay-rate-4"]')).not.toBeNull();
    } finally {
      handle.cleanup();
    }
  });

  test('step controls update the current step signal', () => {
    const handle = renderViewer(buildReplay(5));
    try {
      const forward = handle.root.querySelector<HTMLButtonElement>(
        '[data-testid="replay-step-forward"]',
      );
      const back = handle.root.querySelector<HTMLButtonElement>('[data-testid="replay-step-back"]');
      const step = handle.root.querySelector<HTMLElement>('[data-testid="replay-current-step"]');
      expect(forward).not.toBeNull();
      expect(back).not.toBeNull();
      expect(step?.textContent?.trim()).toBe('1');

      forward?.dispatchEvent(new window.Event('click', { bubbles: true }));
      forward?.dispatchEvent(new window.Event('click', { bubbles: true }));
      expect(step?.textContent?.trim()).toBe('3');

      back?.dispatchEvent(new window.Event('click', { bubbles: true }));
      expect(step?.textContent?.trim()).toBe('2');
    } finally {
      handle.cleanup();
    }
  });

  test('play/pause toggle switches the transport label and state', () => {
    const handle = renderViewer(buildReplay(3));
    try {
      const toggle = handle.root.querySelector<HTMLButtonElement>('[data-testid="replay-toggle"]');
      expect(toggle).not.toBeNull();
      expect(toggle?.getAttribute('aria-label')).toBe('Play');
      expect(toggle?.textContent?.trim()).toBe('Play');

      toggle?.dispatchEvent(new window.Event('click', { bubbles: true }));
      expect(toggle?.getAttribute('aria-label')).toBe('Pause');
      expect(toggle?.textContent?.trim()).toBe('Pause');

      toggle?.dispatchEvent(new window.Event('click', { bubbles: true }));
      expect(toggle?.getAttribute('aria-label')).toBe('Play');
      expect(toggle?.textContent?.trim()).toBe('Play');
    } finally {
      handle.cleanup();
    }
  });

  test('rate selector marks the active rate via aria-pressed', () => {
    const handle = renderViewer(buildReplay(2));
    try {
      const r2 = handle.root.querySelector<HTMLButtonElement>('[data-testid="replay-rate-2"]');
      const r4 = handle.root.querySelector<HTMLButtonElement>('[data-testid="replay-rate-4"]');
      const r1 = handle.root.querySelector<HTMLButtonElement>('[data-testid="replay-rate-1"]');
      expect(r1?.getAttribute('aria-pressed')).toBe('true');
      expect(r2?.getAttribute('aria-pressed')).toBe('false');

      r2?.dispatchEvent(new window.Event('click', { bubbles: true }));
      expect(r2?.getAttribute('aria-pressed')).toBe('true');
      expect(r1?.getAttribute('aria-pressed')).toBe('false');

      r4?.dispatchEvent(new window.Event('click', { bubbles: true }));
      expect(r4?.getAttribute('aria-pressed')).toBe('true');
      expect(r2?.getAttribute('aria-pressed')).toBe('false');
    } finally {
      handle.cleanup();
    }
  });

  test('scrub input drives the current step', () => {
    const handle = renderViewer(buildReplay(6));
    try {
      const scrub = handle.root.querySelector<HTMLInputElement>('[data-testid="replay-scrub"]');
      const step = handle.root.querySelector<HTMLElement>('[data-testid="replay-current-step"]');
      expect(scrub).not.toBeNull();
      scrub!.value = '4';
      scrub?.dispatchEvent(new window.Event('input', { bubbles: true }));
      expect(step?.textContent?.trim()).toBe('5');
    } finally {
      handle.cleanup();
    }
  });

  test('maps the current state into MapViewer', () => {
    const handle = renderViewer(buildReplay(3));
    try {
      // Initially shows turn 0, score 0 for p1 / 0 for p2.
      const map = handle.root.querySelector('[data-testid="map-viewer"]');
      expect(map).not.toBeNull();
      expect(map?.textContent ?? '').toContain('turn 0');
      const rows = handle.root.querySelectorAll('[data-testid="map-viewer-scores"] tbody tr');
      expect(rows.length).toBe(2);
      expect(rows[0]?.textContent ?? '').toContain('0');

      const forward = handle.root.querySelector<HTMLButtonElement>(
        '[data-testid="replay-step-forward"]',
      );
      forward?.dispatchEvent(new window.Event('click', { bubbles: true }));
      forward?.dispatchEvent(new window.Event('click', { bubbles: true }));
      expect(map?.textContent ?? '').toContain('turn 2');
      const updatedRows = handle.root.querySelectorAll(
        '[data-testid="map-viewer-scores"] tbody tr',
      );
      expect(updatedRows[0]?.textContent ?? '').toContain('2');
      expect(updatedRows[1]?.textContent ?? '').toContain('4');
    } finally {
      handle.cleanup();
    }
  });
});
