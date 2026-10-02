import { afterEach, describe, expect, mock, test } from 'bun:test';

import {
  fetchLiveSnapshot,
  isLiveSnapshot,
  LIVE_SNAPSHOT_URL,
  type LiveSnapshot,
} from '@/lib/live';

/** Committed example fixture (mirrors `packages/frontend/public/match-state.json`). */
const EXAMPLE_SNAPSHOT: LiveSnapshot = {
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

describe('isLiveSnapshot', () => {
  test('accepts the example fixture', () => {
    expect(isLiveSnapshot(EXAMPLE_SNAPSHOT)).toBe(true);
  });

  test('accepts a snapshot with nullable fields set to null', () => {
    expect(
      isLiveSnapshot({
        ...EXAMPLE_SNAPSHOT,
        day: null,
        revision: null,
        error: null,
      }),
    ).toBe(true);
  });

  test('rejects a payload with schemaVersion=2', () => {
    const drift = { ...EXAMPLE_SNAPSHOT, schemaVersion: 2 };
    expect(isLiveSnapshot(drift)).toBe(false);
  });

  test('rejects an unknown status string', () => {
    const bad = { ...EXAMPLE_SNAPSHOT, status: 'paused' };
    expect(isLiveSnapshot(bad)).toBe(false);
  });

  test('rejects a non-object payload', () => {
    expect(isLiveSnapshot(null)).toBe(false);
    expect(isLiveSnapshot('hello')).toBe(false);
    expect(isLiveSnapshot(42)).toBe(false);
  });
});

describe('fetchLiveSnapshot', () => {
  let originalFetch: typeof fetch;

  afterEach(() => {
    globalThis.fetch = originalFetch;
  });

  test('returns the parsed snapshot on 200', async () => {
    originalFetch = globalThis.fetch;
    globalThis.fetch = mock(async (_input: RequestInfo | URL): Promise<Response> => {
      return new Response(JSON.stringify(EXAMPLE_SNAPSHOT), { status: 200 });
    }) as unknown as typeof fetch;
    const result = await fetchLiveSnapshot();
    expect(result).not.toBeNull();
    expect(result?.status).toBe('running');
    expect(result?.day).toBe(2);
  });

  test('returns null on 404 (CLI not running)', async () => {
    originalFetch = globalThis.fetch;
    globalThis.fetch = mock(async (_input: RequestInfo | URL): Promise<Response> => {
      return new Response('not found', { status: 404 });
    }) as unknown as typeof fetch;
    const result = await fetchLiveSnapshot();
    expect(result).toBeNull();
  });

  test('returns null on malformed JSON body', async () => {
    originalFetch = globalThis.fetch;
    globalThis.fetch = mock(async (_input: RequestInfo | URL): Promise<Response> => {
      // SchemaVersion missing -> fails the type guard.
      return new Response(JSON.stringify({ server: 'http://x' }), { status: 200 });
    }) as unknown as typeof fetch;
    const result = await fetchLiveSnapshot();
    expect(result).toBeNull();
  });

  test('uses the canonical snapshot URL', () => {
    expect(LIVE_SNAPSHOT_URL).toBe('/match-state.json');
  });
});
