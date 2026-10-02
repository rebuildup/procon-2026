import type { Component } from 'solid-js';

import { getTheme, toggleTheme } from '@/stores/theme';

export const Header: Component = () => (
  <header class="app-header" data-testid="app-header">
    <div class="app-header__brand">
      <span class="app-header__logo">procon-2026</span>
      <span class="app-header__tagline">viewer</span>
    </div>
    <button
      type="button"
      class="app-header__theme-toggle"
      data-testid="theme-toggle"
      aria-label="Toggle theme"
      onClick={() => toggleTheme()}
    >
      {getTheme().mode === 'dark' ? 'Light mode' : 'Dark mode'}
    </button>
  </header>
);
