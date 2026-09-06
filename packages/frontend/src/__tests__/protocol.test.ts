import { describe, expect, test } from 'bun:test';

import { PROTOCOL_VERSION } from '@/lib/protocol';

describe('protocol', () => {
  test('exposes a stable protocol version', () => {
    expect(PROTOCOL_VERSION.major).toBe(1);
    expect(PROTOCOL_VERSION.minor).toBe(0);
  });
});
