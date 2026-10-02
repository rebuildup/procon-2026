import { afterEach, beforeEach, describe, expect, test } from 'bun:test';

import { getTheme, setTheme, toggleTheme } from '@/stores/theme';

describe('theme store', () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
  });

  test('toggleTheme flips between light and dark', () => {
    setTheme('dark');
    expect(getTheme().mode).toBe('dark');
    const next = toggleTheme();
    expect(next).toBe('light');
    expect(getTheme().mode).toBe('light');
    const next2 = toggleTheme();
    expect(next2).toBe('dark');
    expect(getTheme().mode).toBe('dark');
  });

  test('setTheme updates the active mode', () => {
    setTheme('light');
    expect(getTheme().mode).toBe('light');
    setTheme('dark');
    expect(getTheme().mode).toBe('dark');
  });

  test('persists mode to localStorage', () => {
    setTheme('light');
    expect(window.localStorage.getItem('procon-2026.theme')).toBe('light');
    setTheme('dark');
    expect(window.localStorage.getItem('procon-2026.theme')).toBe('dark');
  });
});
