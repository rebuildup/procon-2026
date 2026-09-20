import { afterEach, describe, expect, mock, test } from 'bun:test';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { jsx as hyperscriptJsx } from 'solid-js/h/jsx-runtime';

import { Live } from '@/pages/Live';
import { createLivePoller, type LiveMatchState } from '@/lib/live';

/** Tiny fixture used by the render tests; matches the committed JSON shape. */
const FIXTURE_STATE: LiveMatchState = {
  endsAt: Date.now() + 5_000,
  day: 1,
  agents: [
    { id: 0, kind: 0, pos: { q: 0, r: 0 }, stock: 10 },
    { id: 1, kind: 1, pos: { q: 1, r: 0 }, stock: 8 },
  ],
  others: [],
  traffics: [],
};

/** Minimal setting fixture; the page only inspects `daySeconds`. */
const FIXTURE_SETTING = {
  startsAt: 0,
  daySeconds: [10, 10, 10, 10],
  daySteps: [6, 6, 6, 6],
  map: { height: 4, width: 4, cells: [], spots: [] },
  agents: [],
  spots: [],
};

/** Install a `fetch` mock that returns the two fixtures. Returns a reset hook. */
function installFetchMock(state: LiveMatchState, setting: unknown): () => void {
  const originalFetch = globalThis.fetch;
  const fakeFetch = mock(async (input: RequestInfo | URL): Promise<Response> => {
    const url = typeof input === 'string' ? input : input.toString();
    if (url.includes('match-state.example.json')) {
      return new Response(JSON.stringify(state), { status: 200 });
    }
    if (url.includes('match-setting.example.json')) {
      return new Response(JSON.stringify(setting), { status: 200 });
    }
    return new Response('not found', { status: 404 });
  });
  globalThis.fetch = fakeFetch as unknown as typeof fetch;
  return (): void => {
    globalThis.fetch = originalFetch;
  };
}

interface RenderHandle {
  root: HTMLDivElement;
  cleanup: () => void;
}

function renderLive(): RenderHandle {
  const root = document.createElement('div');
  document.body.appendChild(root);
  const view = (): JSX.Element => hyperscriptJsx(Live, {}) as unknown as JSX.Element;
  const dispose: () => void = render(view, root);
  return {
    root,
    cleanup: () => {
      dispose();
      root.remove();
    },
  };
}

/** Yield repeatedly so async fetches + signals settle before assertions. */
async function flushMicrotasks(times = 10): Promise<void> {
  for (let i = 0; i < times; i += 1) {
    await Promise.resolve();
  }
}

describe('Live page', () => {
  let root: HTMLDivElement;
  afterEach(() => {
    root?.remove();
  });

  test('renders the page with a status badge visible', async () => {
    const restore = installFetchMock(FIXTURE_STATE, FIXTURE_SETTING);
    try {
      const handle = renderLive();
      root = handle.root;
      expect(root.querySelector('[data-testid="page-live"]')).not.toBeNull();
      const badge = root.querySelector<HTMLElement>('[data-testid="live-status-badge"]');
      expect(badge).not.toBeNull();
      // The badge text is the lower-case status string ('waiting' before the
      // first poll resolves; 'running' once `endsAt` is in the future).
      const text = (badge?.textContent ?? '').trim();
      expect(['waiting', 'running', 'ended']).toContain(text);
    } finally {
      restore();
    }
  });

  test('reflects the polled snapshot in the HUD after the first fetch resolves', async () => {
    const restore = installFetchMock(FIXTURE_STATE, FIXTURE_SETTING);
    try {
      const handle = renderLive();
      root = handle.root;
      // Let the synchronous-on-mount fetch complete + signals flush.
      await flushMicrotasks();
      const day = root.querySelector<HTMLElement>('[data-testid="live-day"]');
      const agents = root.querySelector<HTMLElement>('[data-testid="live-agent-count"]');
      const badge = root.querySelector<HTMLElement>('[data-testid="live-status-badge"]');
      // Day is rendered as the bare number from the fixture.
      expect((day?.textContent ?? '')).toContain('1');
      // Agent count matches the fixture's `agents` length.
      expect((agents?.textContent ?? '')).toContain('2');
      // Status badge should read `running` because the fixture's endsAt is in
      // the future and within the configured 10 s window.
      expect(badge?.getAttribute('data-status')).toBe('running');
    } finally {
      restore();
    }
  });

  test('createLivePoller stops issuing fetches after stop()', async () => {
    const restore = installFetchMock(FIXTURE_STATE, FIXTURE_SETTING);
    try {
      let callCount = 0;
      const originalFetch = globalThis.fetch;
      globalThis.fetch = mock(async (input: RequestInfo | URL): Promise<Response> => {
        const url = typeof input === 'string' ? input : input.toString();
        callCount += 1;
        if (url.includes('match-state.example.json')) {
          return new Response(JSON.stringify(FIXTURE_STATE), { status: 200 });
        }
        return new Response('{}', { status: 200 });
      }) as unknown as typeof fetch;

      const poller = createLivePoller(50);
      poller.start();
      // First poll runs immediately on start(); wait a few intervals.
      await new Promise((resolve) => setTimeout(resolve, 200));
      const callsBeforeStop = callCount;
      expect(callsBeforeStop).toBeGreaterThan(0);
      poller.stop();
      await new Promise((resolve) => setTimeout(resolve, 200));
      expect(callCount).toBe(callsBeforeStop);
      globalThis.fetch = originalFetch;
    } finally {
      restore();
    }
  });
});