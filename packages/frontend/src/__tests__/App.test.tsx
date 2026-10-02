import { describe, expect, test } from 'bun:test';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { jsx as hyperscriptJsx } from 'solid-js/h/jsx-runtime';

import { App } from '@/App';

describe('App', () => {
  test('renders without crashing', () => {
    const root = document.createElement('div');
    document.body.appendChild(root);
    try {
      const view = (): JSX.Element => hyperscriptJsx(App, {}) as unknown as JSX.Element;
      expect(() => render(view, root)).not.toThrow();
      expect(root.querySelector('[data-testid="app-shell"]')).not.toBeNull();
    } finally {
      root.remove();
    }
  });

  test('exposes the layout shell on first paint', () => {
    const root = document.createElement('div');
    document.body.appendChild(root);
    try {
      const view = (): JSX.Element => hyperscriptJsx(App, {}) as unknown as JSX.Element;
      render(view, root);
      expect(root.querySelector('[data-testid="app-header"]')).not.toBeNull();
      expect(root.querySelector('[data-testid="app-sidebar"]')).not.toBeNull();
      expect(root.querySelector('[data-testid="app-main"]')).not.toBeNull();
    } finally {
      root.remove();
    }
  });
});
