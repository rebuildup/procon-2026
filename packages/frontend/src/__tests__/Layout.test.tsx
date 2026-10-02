import { describe, expect, test } from 'bun:test';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { Route, Router } from '@solidjs/router';
import { jsx as hyperscriptJsx } from 'solid-js/h/jsx-runtime';

import { Layout } from '@/components/Layout';
import { Home } from '@/pages/Home';

describe('Layout', () => {
  test('renders header, sidebar, and main', () => {
    const root = document.createElement('div');
    document.body.appendChild(root);
    try {
      const view = (): JSX.Element =>
        hyperscriptJsx(Router, {
          root: Layout,
          children: [
            hyperscriptJsx(Route, {
              path: '/',
              component: () => hyperscriptJsx('span', { children: 'test-content' }),
            }),
          ],
        }) as unknown as JSX.Element;
      render(view, root);
      expect(root.querySelector('[data-testid="app-header"]')).not.toBeNull();
      expect(root.querySelector('[data-testid="app-sidebar"]')).not.toBeNull();
      expect(root.querySelector('[data-testid="app-main"]')).not.toBeNull();
    } finally {
      root.remove();
    }
  });

  test('sidebar links to home, replays, and solvers', () => {
    const root = document.createElement('div');
    document.body.appendChild(root);
    try {
      const view = (): JSX.Element =>
        hyperscriptJsx(Router, {
          root: Layout,
          children: [
            hyperscriptJsx(Route, { path: '/', component: () => hyperscriptJsx('span', {}) }),
          ],
        }) as unknown as JSX.Element;
      render(view, root);
      const sidebar = root.querySelector('[data-testid="app-sidebar"]');
      expect(sidebar).not.toBeNull();
      const links = sidebar?.querySelectorAll('a') ?? [];
      const hrefs = Array.from(links).map((a) => a.getAttribute('href') ?? '');
      expect(hrefs).toContain('/');
      expect(hrefs).toContain('/replays');
      expect(hrefs).toContain('/solvers');
    } finally {
      root.remove();
    }
  });

  test('renders children inside the main slot', () => {
    const root = document.createElement('div');
    document.body.appendChild(root);
    try {
      const view = (): JSX.Element =>
        hyperscriptJsx(Router, {
          root: Layout,
          children: [
            hyperscriptJsx(Route, { path: '/', component: () => hyperscriptJsx(Home, {}) }),
          ],
        }) as unknown as JSX.Element;
      render(view, root);
      const main = root.querySelector('[data-testid="app-main"]');
      expect(main).not.toBeNull();
      expect(main?.textContent ?? '').toContain('Home');
    } finally {
      root.remove();
    }
  });
});
