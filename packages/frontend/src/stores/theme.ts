import { createEffect, createRoot } from 'solid-js';
import { createStore } from 'solid-js/store';

export type ThemeMode = 'light' | 'dark';

export interface ThemeState {
  mode: ThemeMode;
}

const STORAGE_KEY = 'procon-2026.theme';

function readInitialMode(): ThemeMode {
  if (typeof window === 'undefined') {
    return 'dark';
  }
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    if (stored === 'light' || stored === 'dark') {
      return stored;
    }
  } catch {
    // localStorage access may fail (e.g. privacy mode); fall through to default
  }
  if (typeof window.matchMedia === 'function') {
    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    return prefersDark ? 'dark' : 'light';
  }
  return 'dark';
}

function applyTheme(mode: ThemeMode): void {
  if (typeof document === 'undefined') {
    return;
  }
  const root = document.documentElement;
  root.dataset['theme'] = mode;
  root.dataset['colorMode'] = mode;
  root.style.colorScheme = mode;
}

const initial: ThemeState = {
  mode: readInitialMode(),
};

const [themeState, setThemeState] = createStore<ThemeState>(initial);

createRoot(() => {
  createEffect(() => {
    const mode = themeState.mode;
    applyTheme(mode);
    if (typeof window === 'undefined') return;
    try {
      window.localStorage.setItem(STORAGE_KEY, mode);
    } catch {
      // ignore persistence failures (private mode, quota, etc.)
    }
  });
});

export function getTheme(): ThemeState {
  return themeState;
}

export function toggleTheme(): ThemeMode {
  const next: ThemeMode = themeState.mode === 'dark' ? 'light' : 'dark';
  setThemeState('mode', next);
  return next;
}

export function setTheme(mode: ThemeMode): void {
  setThemeState('mode', mode);
}
