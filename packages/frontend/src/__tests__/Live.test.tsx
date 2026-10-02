import { afterEach, describe, expect, mock, test } from 'bun:test';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { jsx as hyperscriptJsx } from 'solid-js/h/jsx-runtime';

import { Live } from '@/pages/Live';
import { createLiveSnapshotPoller, type LiveSnapshot } from '@/lib/live';

/** Tiny fixture used by the render tests; mirrors `public/match-state.json`. */
const FIXTURE_SNAPSHOT: LiveSnapshot = {
  schemaVersion: 1,
  server: 'http://localhost:8080',
  solver: 'wait',
  status: 'running',
  day: 2,
  daysTotal: 4,
  revision: 2,
  revisions: [1, 2],
  lastUpdateUnix: 1_789_892_000,
  error: null,
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

/** Install a `fetch` mock that returns the snapshot + setting fixtures. */
function installFetchMock(snapshot: LiveSnapshot | null, setting: unknown): () => void {
  const originalFetch = globalThis.fetch;
  const fakeFetch = mock(async (input: RequestInfo | URL): Promise<Response> => {
    const url = typeof input === 'string' ? input : input.toString();
    if (url.includes('match-state.json')) {
      if (snapshot === null) {
        return new Response('not found', { status: 404 });
      }
      return new Response(JSON.stringify(snapshot), { status: 200 });
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
    const restore = installFetchMock(FIXTURE_SNAPSHOT, FIXTURE_SETTING);
    try {
      const handle = renderLive();
      root = handle.root;
      expect(root.querySelector('[data-testid="page-live"]')).not.toBeNull();
      const badge = root.querySelector<HTMLElement>('[data-testid="live-status-badge"]');
      expect(badge).not.toBeNull();
    } finally {
      restore();
    }
  });

  test('reflects the polled snapshot in the HUD after the first fetch resolves', async () => {
    const restore = installFetchMock(FIXTURE_SNAPSHOT, FIXTURE_SETTING);
    try {
      const handle = renderLive();
      root = handle.root;
      // Let the synchronous-on-mount fetch complete + signals flush.
      await flushMicrotasks();
      const day = root.querySelector<HTMLElement>('[data-testid="live-day"]');
      const revisions = root.querySelector<HTMLElement>('[data-testid="live-revisions"]');
      const solver = root.querySelector<HTMLElement>('[data-testid="live-solver"]');
      const server = root.querySelector<HTMLElement>('[data-testid="live-server"]');
      const badge = root.querySelector<HTMLElement>('[data-testid="live-status-badge"]');
      expect(day?.textContent ?? '').toContain('2');
      expect(day?.textContent ?? '').toContain('4');
      expect(revisions?.textContent ?? '').toContain('1');
      expect(revisions?.textContent ?? '').toContain('2');
      expect(solver?.textContent ?? '').toContain('wait');
      expect(server?.textContent ?? '').toContain('http://localhost:8080');
      expect(badge?.getAttribute('data-status')).toBe('running');
    } finally {
      restore();
    }
  });

  test('renders the error banner when the snapshot has an error message', async () => {
    const restore = installFetchMock(
      { ...FIXTURE_SNAPSHOT, status: 'error', error: 'boom' },
      FIXTURE_SETTING,
    );
    try {
      const handle = renderLive();
      root = handle.root;
      await flushMicrotasks();
      const error = root.querySelector<HTMLElement>('[data-testid="live-error"]');
      expect(error?.textContent ?? '').toContain('boom');
    } finally {
      restore();
    }
  });

  test('does not crash when /match-state.json returns 404', async () => {
    const restore = installFetchMock(null, FIXTURE_SETTING);
    try {
      const handle = renderLive();
      root = handle.root;
      await flushMicrotasks();
      expect(root.querySelector('[data-testid="page-live"]')).not.toBeNull();
      const error = root.querySelector<HTMLElement>('[data-testid="live-error"]');
      expect(error).toBeNull();
    } finally {
      restore();
    }
  });

  test('createLiveSnapshotPoller stops issuing fetches after stop()', async () => {
    const originalFetch = globalThis.fetch;

    try {
      let callCount = 0;
      const wrapped = mock(async (_input: RequestInfo | URL): Promise<Response> => {
        callCount += 1;
        return new Response(JSON.stringify(FIXTURE_SNAPSHOT), { status: 200 });
      });
      globalThis.fetch = wrapped as unknown as typeof fetch;

      const poller = createLiveSnapshotPoller(50);
      poller.start();
      // First poll runs immediately on start(); wait a few intervals.
      await new Promise((resolve) => setTimeout(resolve, 200));
      const callsBeforeStop = callCount;
      expect(callsBeforeStop).toBeGreaterThan(0);
      poller.stop();
      await new Promise((resolve) => setTimeout(resolve, 200));
      expect(callCount).toBe(callsBeforeStop);
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
});
