import type { Component, JSX } from 'solid-js';

import { Header } from '@/components/Header';
import { Sidebar } from '@/components/Sidebar';

export const Layout: Component<{ children?: JSX.Element }> = (props) => (
  <div class="app-shell" data-testid="app-shell">
    <Header />
    <div class="app-shell__body">
      <Sidebar />
      <main class="app-shell__main" data-testid="app-main">
        {props.children}
      </main>
    </div>
  </div>
);
