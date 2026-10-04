import { afterEach, beforeEach, describe, expect, it, spyOn } from 'bun:test';
import { createFrameScheduler } from './frame-scheduler';

describe('frame scheduler', () => {
  let callbacks: Map<number, FrameRequestCallback>;
  let nextId: number;
  let request: ReturnType<typeof spyOn<typeof globalThis, 'requestAnimationFrame'>>;
  let cancel: ReturnType<typeof spyOn<typeof globalThis, 'cancelAnimationFrame'>>;

  beforeEach(() => {
    callbacks = new Map();
    nextId = 0;
    request = spyOn(globalThis, 'requestAnimationFrame').mockImplementation((callback) => {
      const id = nextId++;
      callbacks.set(id, callback);
      return id;
    });
    cancel = spyOn(globalThis, 'cancelAnimationFrame').mockImplementation((id) => {
      callbacks.delete(id);
    });
  });
  afterEach(() => {
    request.mockRestore();
    cancel.mockRestore();
  });
  const flush = () => {
    const current = [...callbacks.values()];
    callbacks.clear();
    current.forEach((callback) => callback(0));
  };

  it('draws the latest state once for a burst and remains idle afterward', () => {
    let state = 0;
    const rendered: number[] = [];
    const frames = createFrameScheduler(() => rendered.push(state));
    for (state = 0; state < 200; state++) frames.invalidate();
    expect(request).toHaveBeenCalledTimes(1);
    expect(rendered).toEqual([]);
    flush();
    expect(rendered).toEqual([200]);
    expect(callbacks.size).toBe(0);
    frames.invalidate();
    flush();
    expect(rendered).toEqual([200, 200]);
  });

  it('cancels queued work and ignores invalidation after disposal', () => {
    let draws = 0;
    const frames = createFrameScheduler(() => draws++);
    frames.invalidate();
    const stale = callbacks.get(0)!;
    frames.dispose();
    frames.dispose();
    frames.invalidate();
    stale(0);
    expect(cancel).toHaveBeenCalledTimes(1);
    expect(cancel).toHaveBeenCalledWith(0);
    expect(draws).toBe(0);
    expect(request).toHaveBeenCalledTimes(1);
  });

  it('preserves invalidation during a draw for the following frame', () => {
    let draws = 0;
    const frames = createFrameScheduler(() => {
      if (++draws === 1) frames.invalidate();
    });
    frames.invalidate();
    flush();
    expect(draws).toBe(1);
    expect(callbacks.size).toBe(1);
    flush();
    expect(draws).toBe(2);
    expect(callbacks.size).toBe(0);
  });
});
