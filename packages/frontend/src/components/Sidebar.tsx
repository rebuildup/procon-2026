import { For, type Component } from 'solid-js';
import { A } from '@solidjs/router';

interface SidebarLink {
  readonly href: string;
  readonly label: string;
}

const LINKS: readonly SidebarLink[] = [
  { href: '/', label: 'Home' },
  { href: '/replays', label: 'Replays' },
  { href: '/solvers', label: 'Solvers' },
  { href: '/live', label: 'Live' },
];

export const Sidebar: Component = () => (
  <nav class="app-sidebar" aria-label="Primary" data-testid="app-sidebar">
    <ul class="app-sidebar__list">
      <For each={LINKS}>
        {(link) => (
          <li class="app-sidebar__item">
            <A
              href={link.href}
              end={link.href === '/'}
              class="app-sidebar__link"
              activeClass="app-sidebar__link--active"
            >
              {link.label}
            </A>
          </li>
        )}
      </For>
    </ul>
  </nav>
);
